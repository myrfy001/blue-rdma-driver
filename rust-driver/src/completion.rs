use std::{collections::VecDeque, iter, ops::ControlFlow, sync::Arc};

use bitvec::vec::BitVec;
use log::trace;
use parking_lot::Mutex;

use crate::{
    ack_responder::AckResponse,
    ack_timeout::AckTimeoutTask,
    constants::MAX_CQ_CNT,
    qp::{QpAttr, QpTable, QpTableShared},
    spawner::{SingleThreadTaskWorker, TaskTx},
    utils::{Msn, Psn},
};

struct EventRegister {
    message_id: MessageIdentifier,
    event: Event,
}

struct Message {
    id: MessageIdentifier,
    meta: MessageMeta,
}

struct MessageIdentifier {
    qpn: u32,
    msn: u16,
    is_send: bool,
}

struct CompletionQueueRegistry {
    inner: VecDeque<EventRegister>,
}

impl CompletionQueueRegistry {
    fn push(&mut self, register: EventRegister) {
        self.inner.push_back(register);
    }

    fn pop() -> Option<EventRegister> {
        None
    }
}

#[derive(Debug)]
#[allow(variant_size_differences)]
pub(crate) enum CompletionTask {
    Register { qpn: u32, event: Event },
    AckSend { qpn: u32, base_psn: Psn },
    AckRecv { qpn: u32, base_psn: Psn },
}

pub(crate) struct CompletionWorker {
    tracker_table: QpTable<QueuePairMessageTracker>,
    cq_table: CompletionQueueTable,
    qp_table: QpTableShared<QpAttr>,
    ack_resp_tx: TaskTx<AckResponse>,
    ack_timeout_tx: TaskTx<AckTimeoutTask>,
}

impl SingleThreadTaskWorker for CompletionWorker {
    type Task = CompletionTask;

    fn process(&mut self, task: Self::Task) {
        let qpn = match task {
            CompletionTask::Register { qpn, .. }
            | CompletionTask::AckSend { qpn, .. }
            | CompletionTask::AckRecv { qpn, .. } => qpn,
        };
        let Some(tracker) = self.tracker_table.get_qp_mut(qpn) else {
            return;
        };
        let Some(qp_attr) = self.qp_table.get_qp(qpn) else {
            return;
        };
        match task {
            CompletionTask::Register { event, .. } => {
                tracker.append(event);
            }
            CompletionTask::AckSend { base_psn, .. } => {
                if let Some(send_cq) = qp_attr.send_cq.and_then(|h| self.cq_table.get_cq(h)) {
                    tracker.ack_send(Some(base_psn), send_cq, &self.ack_timeout_tx);
                }
            }
            CompletionTask::AckRecv { base_psn, .. } => {
                let send_cq = qp_attr.send_cq.and_then(|h| self.cq_table.get_cq(h));
                if let Some(recv_cq) = qp_attr.recv_cq.and_then(|h| self.cq_table.get_cq(h)) {
                    tracker.ack_recv(
                        base_psn,
                        recv_cq,
                        send_cq,
                        qpn,
                        &self.ack_resp_tx,
                        &self.ack_timeout_tx,
                    );
                }
            }
        }
    }
}

impl CompletionWorker {
    pub(crate) fn new(
        cq_table: CompletionQueueTable,
        qp_table: QpTableShared<QpAttr>,
        ack_resp_tx: TaskTx<AckResponse>,
        ack_timeout_tx: TaskTx<AckTimeoutTask>,
    ) -> Self {
        Self {
            tracker_table: QpTable::new(),
            cq_table,
            qp_table,
            ack_resp_tx,
            ack_timeout_tx,
        }
    }
}

pub(crate) struct EventWithQpn {
    qpn: u32,
    event: Event,
}

impl EventWithQpn {
    pub(crate) fn new(qpn: u32, event: Event) -> Self {
        Self { qpn, event }
    }
}

#[derive(Default)]
struct QueuePairMessageTracker {
    send: MessageTracker<SendEvent>,
    recv: MessageTracker<RecvEvent>,
    read_resp_queue: VecDeque<RecvEvent>,
    post_recv_queue: VecDeque<PostRecvEvent>,
}

impl QueuePairMessageTracker {
    fn new(
        send: MessageTracker<SendEvent>,
        recv: MessageTracker<RecvEvent>,
        read_resp_queue: VecDeque<RecvEvent>,
        post_recv_queue: VecDeque<PostRecvEvent>,
    ) -> Self {
        Self {
            send,
            recv,
            read_resp_queue,
            post_recv_queue,
        }
    }

    fn append(&mut self, event: Event) {
        match event {
            Event::Send(x) => self.send.append(x),
            Event::Recv(x) => self.recv.append(x),
            Event::PostRecv(x) => {
                self.post_recv_queue.push_back(x);
            }
        }
    }

    fn ack_send(
        &mut self,
        psn: Option<Psn>,
        send_cq: &CompletionQueue,
        ack_timeout_tx: &TaskTx<AckTimeoutTask>,
    ) {
        if let Some(psn) = psn {
            self.send.ack(psn);
        }
        while let Some(event) = self.send.peek() {
            trace!("ack send event: {event:?}");
            match event.op {
                SendEventOp::WriteSignaled | SendEventOp::SendSignaled => {
                    let x = self.send.pop().unwrap_or_else(|| unreachable!());
                    let completion = match x.op {
                        SendEventOp::WriteSignaled => Completion::RdmaWrite { wr_id: x.wr_id },
                        SendEventOp::SendSignaled => Completion::Send { wr_id: x.wr_id },
                        SendEventOp::ReadSignaled => unreachable!(),
                    };
                    ack_timeout_tx.send(AckTimeoutTask::ack(x.qpn));
                    send_cq.push_back(completion);
                }
                SendEventOp::ReadSignaled => {
                    if let Some(recv_event) = self.read_resp_queue.pop_front() {
                        let x = self.send.pop().unwrap_or_else(|| unreachable!());
                        let completion = Completion::RdmaRead { wr_id: x.wr_id };
                        send_cq.push_back(completion);
                    } else {
                        break;
                    }
                }
            }
        }
    }

    fn ack_recv(
        &mut self,
        psn: Psn,
        recv_cq: &CompletionQueue,
        send_cq: Option<&CompletionQueue>,
        qpn: u32,
        ack_resp_tx: &TaskTx<AckResponse>,
        ack_timeout_tx: &TaskTx<AckTimeoutTask>,
    ) {
        self.recv.ack(psn);
        while let Some(event) = self.recv.pop() {
            match event.op {
                RecvEventOp::WriteWithImm { imm } => {
                    let completion = Completion::RecvRdmaWithImm { imm };
                    recv_cq.push_back(completion);
                }
                RecvEventOp::Recv => {
                    let x = self
                        .post_recv_queue
                        .pop_back()
                        .unwrap_or_else(|| unreachable!("no posted recv wr"));
                    let completion = Completion::Recv {
                        wr_id: x.wr_id,
                        imm: None,
                    };
                    recv_cq.push_back(completion);
                }
                RecvEventOp::RecvWithImm { imm } => {
                    let x = self
                        .post_recv_queue
                        .pop_back()
                        .unwrap_or_else(|| unreachable!("no posted recv wr"));
                    let completion = Completion::Recv {
                        wr_id: x.wr_id,
                        imm: Some(imm),
                    };
                    recv_cq.push_back(completion);
                }
                RecvEventOp::ReadResp => {
                    self.read_resp_queue.push_back(event);
                    // check if the read  completion could be updated
                    if let Some(cq) = send_cq {
                        self.ack_send(None, cq, ack_timeout_tx);
                    }
                }
                RecvEventOp::RecvRead | RecvEventOp::Write => {}
            }
            if event.ack_req {
                ack_resp_tx.send(AckResponse::Ack {
                    qpn,
                    msn: event.meta().msn,
                    last_psn: event.meta().end_psn,
                });
            }
        }
    }
}

#[derive(Debug)]
struct MessageTracker<E> {
    inner: VecDeque<E>,
    base_psn: Psn,
}

impl<E> Default for MessageTracker<E> {
    fn default() -> Self {
        Self {
            inner: VecDeque::default(),
            base_psn: Psn::default(),
        }
    }
}

impl<E: EventMeta> MessageTracker<E> {
    fn append(&mut self, event: E) {
        let pos = self
            .inner
            .iter()
            .rev()
            .position(|e| Msn(e.meta().msn) < Msn(event.meta().msn))
            .unwrap_or(self.inner.len());
        let index = self.inner.len() - pos;
        if self
            .inner
            .get(index)
            .is_none_or(|e| e.meta().msn != event.meta().msn)
        {
            self.inner.insert(index, event);
        }
    }

    fn ack(&mut self, base_psn: Psn) {
        self.base_psn = base_psn;
    }

    fn peek(&self) -> Option<&E> {
        let front = self.inner.front()?;
        (front.meta().end_psn <= self.base_psn).then_some(front)
    }

    fn pop(&mut self) -> Option<E> {
        let front = self.inner.front()?;
        if front.meta().end_psn <= self.base_psn {
            self.inner.pop_front()
        } else {
            None
        }
    }
}

trait EventMeta {
    fn meta(&self) -> MessageMeta;
}

#[derive(Debug, Clone, Copy)]
pub(crate) enum Event {
    Send(SendEvent),
    Recv(RecvEvent),
    PostRecv(PostRecvEvent),
}

#[derive(Debug, Clone, Copy)]
pub(crate) struct SendEvent {
    qpn: u32,
    op: SendEventOp,
    meta: MessageMeta,
    wr_id: u64,
}

impl SendEvent {
    pub(crate) fn new(qpn: u32, op: SendEventOp, meta: MessageMeta, wr_id: u64) -> Self {
        Self {
            qpn,
            op,
            meta,
            wr_id,
        }
    }
}

impl EventMeta for SendEvent {
    fn meta(&self) -> MessageMeta {
        self.meta
    }
}

#[allow(clippy::enum_variant_names)]
#[derive(Debug, Clone, Copy)]
pub(crate) enum SendEventOp {
    WriteSignaled,
    SendSignaled,
    ReadSignaled,
}

#[derive(Debug, Clone, Copy)]
pub(crate) struct RecvEvent {
    qpn: u32,
    op: RecvEventOp,
    meta: MessageMeta,
    ack_req: bool,
}

impl RecvEvent {
    pub(crate) fn new(qpn: u32, op: RecvEventOp, meta: MessageMeta, ack_req: bool) -> Self {
        Self {
            qpn,
            op,
            meta,
            ack_req,
        }
    }
}

impl EventMeta for RecvEvent {
    fn meta(&self) -> MessageMeta {
        self.meta
    }
}

#[derive(Debug, Clone, Copy)]
pub(crate) enum RecvEventOp {
    Write,
    WriteWithImm { imm: u32 },
    Recv,
    RecvWithImm { imm: u32 },
    ReadResp,
    RecvRead,
}

#[derive(Debug, Clone, Copy)]
pub(crate) struct PostRecvEvent {
    qpn: u32,
    wr_id: u64,
}

impl PostRecvEvent {
    pub(crate) fn new(qpn: u32, wr_id: u64) -> Self {
        Self { qpn, wr_id }
    }
}

#[derive(Default, Debug, Clone, Copy)]
pub(crate) struct MessageMeta {
    pub(crate) msn: u16,
    pub(crate) end_psn: Psn,
}

impl MessageMeta {
    pub(crate) fn new(msn: u16, end_psn: Psn) -> Self {
        Self { msn, end_psn }
    }
}

pub(crate) struct CompletionQueueTable {
    inner: Arc<[CompletionQueue]>,
}

impl CompletionQueueTable {
    pub(crate) fn new() -> Self {
        Self {
            inner: iter::repeat_with(CompletionQueue::default)
                .take(MAX_CQ_CNT)
                .collect(),
        }
    }

    pub(crate) fn clone_arc(&self) -> Self {
        Self {
            inner: Arc::clone(&self.inner),
        }
    }

    pub(crate) fn get_cq(&self, handle: u32) -> Option<&CompletionQueue> {
        self.inner.get(handle as usize)
    }
}

#[derive(Default)]
pub(crate) struct CompletionQueue1 {
    inner: Mutex<VecDeque<Completion>>,
}

impl CompletionQueue1 {
    pub(crate) fn push_back(&self, event: Completion) {
        let mut queue = self.inner.lock();
        queue.push_back(event);
    }

    pub(crate) fn pop_front(&self) -> Option<Completion> {
        let mut queue = self.inner.lock();
        queue.pop_front()
    }

    pub(crate) fn front(&self) -> Option<Completion> {
        let queue = self.inner.lock();
        queue.front().copied()
    }
}

#[derive(Default)]
pub(crate) struct CompletionQueue {
    inner: Mutex<VecDeque<Completion>>,
}

impl CompletionQueue {
    pub(crate) fn push_back(&self, event: Completion) {
        let mut queue = self.inner.lock();
        queue.push_back(event);
    }

    pub(crate) fn pop_front(&self) -> Option<Completion> {
        let mut queue = self.inner.lock();
        queue.pop_front()
    }

    pub(crate) fn front(&self) -> Option<Completion> {
        let queue = self.inner.lock();
        queue.front().copied()
    }
}

#[derive(Debug, Clone, Copy)]
pub(crate) enum Completion {
    Send { wr_id: u64 },
    RdmaWrite { wr_id: u64 },
    RdmaRead { wr_id: u64 },
    Recv { wr_id: u64, imm: Option<u32> },
    RecvRdmaWithImm { imm: u32 },
}

impl Completion {
    pub(crate) fn opcode(&self) -> u32 {
        match *self {
            Completion::Send { .. } => ibverbs_sys::ibv_wc_opcode::IBV_WC_SEND,
            Completion::RdmaWrite { .. } => ibverbs_sys::ibv_wc_opcode::IBV_WC_RDMA_WRITE,
            Completion::RdmaRead { .. } => ibverbs_sys::ibv_wc_opcode::IBV_WC_RDMA_READ,
            Completion::Recv { .. } => ibverbs_sys::ibv_wc_opcode::IBV_WC_RECV,
            Completion::RecvRdmaWithImm { .. } => {
                ibverbs_sys::ibv_wc_opcode::IBV_WC_RECV_RDMA_WITH_IMM
            }
        }
    }
}

/// Manages CQs
pub(crate) struct CqManager {
    /// Bitmap tracking allocated CQ handles
    bitmap: BitVec,
}

#[allow(clippy::as_conversions, clippy::indexing_slicing)]
impl CqManager {
    /// Creates a new `CqManager`
    pub(crate) fn new() -> Self {
        let mut bitmap = BitVec::with_capacity(MAX_CQ_CNT);
        bitmap.resize(MAX_CQ_CNT, false);
        Self { bitmap }
    }

    /// Allocates a new cq and returns its cqN
    #[allow(clippy::cast_possible_truncation)] // no larger than u32
    pub(crate) fn create_cq(&mut self) -> Option<u32> {
        let handle = self.bitmap.first_zero()? as u32;
        self.bitmap.set(handle as usize, true);
        Some(handle)
    }

    /// Removes and returns the cq associated with the given cqN
    pub(crate) fn destroy_cq(&mut self, handle: u32) {
        if handle as usize >= MAX_CQ_CNT {
            return;
        }
        self.bitmap.set(handle as usize, false);
    }
}
