use std::{io, iter, sync::Arc};

use types::{WrInjector, WrWorker};
use worker::{SendQueueSync, SendWorker};

use crate::{
    csr::{mode::Mode, proxy::build_send_queue_proxies, CsrBaseAddrAdaptor, DeviceAdaptor},
    mem::DmaBuf,
    ringbuf::DescRingBuffer,
    workers::spawner::{AbortSignal, SingleThreadPollingWorker},
};

mod types;
mod worker;

pub(crate) use types::*;
pub(crate) use worker::SendHandle;

pub(crate) fn spawn<Dev>(
    dev: &Dev,
    bufs: Vec<DmaBuf>,
    mode: Mode,
    abort: &AbortSignal,
) -> io::Result<SendHandle>
where
    Dev: DeviceAdaptor + Clone + Send + 'static,
{
    let injector = Arc::new(WrInjector::new());
    let handle = SendHandle::new(Arc::clone(&injector));
    let mut sq_proxies = build_send_queue_proxies(dev.clone(), mode);
    for (proxy, buf) in sq_proxies.iter_mut().zip(bufs.iter()) {
        proxy.write_base_addr(buf.phys_addr)?;
    }
    let send_queues: Vec<_> = bufs
        .into_iter()
        .map(|p| SendQueue::new(DescRingBuffer::new(p.buf)))
        .collect();
    let workers: Vec<_> = iter::repeat_with(WrWorker::new_fifo)
        .take(send_queues.len())
        .collect();
    let stealers: Vec<_> = workers.iter().map(WrWorker::stealer).collect();
    let sqs = send_queues
        .into_iter()
        .zip(sq_proxies)
        .map(|(sq, proxy)| SendQueueSync::new(sq, proxy));
    for (id, (local, sq)) in workers.into_iter().zip(sqs).enumerate() {
        let worker = SendWorker::new(
            id,
            local,
            Arc::clone(&injector),
            stealers
                .clone()
                .into_iter()
                .enumerate()
                .filter_map(|(i, x)| (i != id).then_some(x))
                .collect(),
            sq,
        );
        let name = format!("SendWorker{id}");
        worker.spawn(&name, abort.clone());
    }

    Ok(handle)
}
