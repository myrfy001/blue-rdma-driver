mod types;
mod worker;

use std::{
    io,
    sync::{atomic::AtomicBool, Arc},
};

use types::{MetaReportQueue, MetaReportQueueCtx, MetaReportQueueHandler};
use worker::{MetaHandler, MetaWorker};

use crate::{
    ack_responder::AckResponse,
    ack_timeout::AckTimeoutTask,
    completion::CompletionTask,
    device::{
        mode::Mode, proxy::build_meta_report_queue_proxies, CsrBaseAddrAdaptor, DeviceAdaptor,
    },
    mem::DmaBuf,
    packet_retransmit::PacketRetransmitTask,
    queue::DescRingBuffer,
    rdma_write_worker::RdmaWriteTask,
};

#[allow(clippy::too_many_arguments)]
pub(crate) fn spawn<Dev>(
    dev: &Dev,
    pages: Vec<DmaBuf>,
    mode: Mode,
    ack_tx: flume::Sender<AckResponse>,
    retransmit_tx: flume::Sender<AckTimeoutTask>,
    packet_retransmit_tx: flume::Sender<PacketRetransmitTask>,
    completion_tx: flume::Sender<CompletionTask>,
    rdma_write_tx: flume::Sender<RdmaWriteTask>,
    is_shutdown: Arc<AtomicBool>,
) -> io::Result<()>
where
    Dev: Clone + DeviceAdaptor + Send + 'static,
{
    let mut mrq_proxies = build_meta_report_queue_proxies(dev.clone(), mode);
    for (proxy, page) in mrq_proxies.iter_mut().zip(pages.iter()) {
        proxy.write_base_addr(page.phys_addr)?;
    }
    let ctxs: Vec<_> = pages
        .into_iter()
        .map(|p| MetaReportQueue::new(DescRingBuffer::new(p.buf)))
        .zip(mrq_proxies)
        .map(|(q, p)| MetaReportQueueCtx::new(q, p))
        .collect();

    let handler = MetaHandler::new(
        ack_tx,
        retransmit_tx,
        packet_retransmit_tx,
        completion_tx,
        rdma_write_tx,
    );
    MetaWorker::new(MetaReportQueueHandler::new(ctxs), handler).spawn(is_shutdown);

    Ok(())
}
