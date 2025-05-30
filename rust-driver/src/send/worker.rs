use std::{io, iter, sync::Arc, time::Duration};

use crossbeam_deque::{Injector, Steal, Stealer, Worker};
use log::error;

use crate::{
    descriptors::{SendQueueReqDescSeg0, SendQueueReqDescSeg1},
    device::{proxy::SendQueueProxy, CsrWriterAdaptor, DeviceAdaptor},
    spawner::{SingleThreadPollingWorker, SingleThreadTaskWorker},
};

use super::{
    types::{SendQueue, SendQueueDesc, WrInjector, WrStealer, WrWorker},
    WrChunk,
};

#[derive(Clone)]
pub(crate) struct SendHandle {
    pub(super) injector: Arc<WrInjector>,
}

impl SendHandle {
    pub(crate) fn new(injector: Arc<WrInjector>) -> Self {
        Self { injector }
    }

    pub(crate) fn send(&self, wr: WrChunk) {
        self.injector.push(wr);
    }
}

pub(crate) struct SendQueueSync<Dev> {
    /// Queue for submitting send requests to the NIC
    send_queue: SendQueue,
    /// Csr proxy
    csr_adaptor: SendQueueProxy<Dev>,
}

impl<Dev: DeviceAdaptor> SendQueueSync<Dev> {
    pub(crate) fn new(send_queue: SendQueue, csr_adaptor: SendQueueProxy<Dev>) -> Self {
        Self {
            send_queue,
            csr_adaptor,
        }
    }

    fn send(&mut self, descs: Vec<SendQueueDesc>) -> bool {
        if self.send_queue.remaining() < descs.len() {
            self.sync_tail();
        }
        if self.send_queue.remaining() < descs.len() {
            return false;
        }
        for desc in descs {
            assert!(self.send_queue.push(desc), "full send queue");
        }
        self.sync_head();
        true
    }

    fn sync_head(&self) {
        self.csr_adaptor
            .write_head(self.send_queue.head())
            .expect("failed to write head csr");
    }

    fn sync_tail(&mut self) {
        let tail_ptr = self
            .csr_adaptor
            .read_tail()
            .expect("failed to read tail csr");
        self.send_queue.set_tail(tail_ptr);
    }
}

/// Worker thread for processing send work requests
pub(crate) struct SendWorker<Dev> {
    /// id of the worker
    id: usize,
    /// Local work request queue for this worker
    local: WrWorker,
    /// Global work request injector shared across workers
    global: Arc<WrInjector>,
    /// Work stealers for taking work from other workers
    remotes: Box<[WrStealer]>,
    sq: SendQueueSync<Dev>,
}

impl<Dev> SendWorker<Dev> {
    pub(crate) fn new(
        id: usize,
        local: WrWorker,
        global: Arc<WrInjector>,
        remotes: Box<[WrStealer]>,
        sq: SendQueueSync<Dev>,
    ) -> Self {
        Self {
            id,
            local,
            global,
            remotes,
            sq,
        }
    }
}

impl<Dev: DeviceAdaptor + Send + 'static> SingleThreadPollingWorker for SendWorker<Dev> {
    type Task = WrChunk;

    fn poll(&mut self) -> Option<Self::Task> {
        // Pop a task from the local queue, if not empty.
        self.local.pop().or_else(|| {
            // Otherwise, we need to look for a task elsewhere.
            iter::repeat_with(|| {
                // Try stealing a batch of tasks from the global queue.
                self.global
                    .steal_batch_and_pop(&self.local)
                    // Or try stealing a task from one of the other threads.
                    .or_else(|| self.remotes.iter().map(Stealer::steal).collect())
            })
            // Loop while no task was stolen and any steal operation needs to be retried.
            .find(|s| !s.is_retry())
            // Extract the stolen task, if there is one.
            .and_then(Steal::success)
        })
    }

    fn process(&mut self, wr: Self::Task) {
        let fst = SendQueueReqDescSeg0::new(
            wr.opcode,
            wr.msn,
            wr.psn.into_inner(),
            wr.qp_type,
            wr.dqpn,
            wr.flags,
            wr.dqp_ip,
            wr.raddr,
            wr.rkey,
            wr.total_len,
        );
        let snd = SendQueueReqDescSeg1::new(
            wr.opcode,
            wr.pmtu,
            wr.is_first,
            wr.is_last,
            wr.is_retry,
            wr.enable_ecn,
            wr.sqpn,
            wr.imm,
            wr.mac_addr,
            wr.lkey,
            wr.len,
            wr.laddr,
        );
        let descs = vec![SendQueueDesc::Seg0(fst), SendQueueDesc::Seg1(snd)];
        if !self.sq.send(descs) {
            self.local.push(wr);
        }
    }
}
