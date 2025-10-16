use std::io;

use log::debug;
use parking_lot::Mutex;

use crate::{
    constants::PSN_MASK,
    rdma_utils::{
        fragmenter::{WrChunkFragmenter, WrPacketFragmenter},
        psn::Psn,
        qp::{num_psn, QpTable, QpTableShared, SendQueueContext, qpn_to_index},
        types::{QpAttr, SendWrRdma},
    },
    workers::{
        completion::{Completion, CompletionTask, Event, MessageMeta, SendEvent, SendEventOp},
        qp_timeout::AckTimeoutTask,
        retransmit::{PacketRetransmitTask, SendQueueElem},
        send::{ChunkPos, QpParams, SendHandle, WorkReqOpCode, WrChunkBuilder},
        spawner::{SingleThreadTaskWorker, TaskTx},
    },
};

#[derive(Debug, PartialEq, Eq)]
pub(crate) enum RdmaWriteTask {
    Write { qpn: u32, wr: SendWrRdma },
    Ack { qpn: u32, base_psn: Psn },
    NewComplete { qpn: u32, msn: u16 },
}

impl RdmaWriteTask {
    pub(crate) fn new_write(qpn: u32, wr: SendWrRdma) -> Self {
        Self::Write { qpn, wr }
    }

    pub(crate) fn new_ack(qpn: u32, base_psn: Psn) -> Self {
        Self::Ack { qpn, base_psn }
    }

    pub(crate) fn new_complete(qpn: u32, msn: u16) -> Self {
        Self::NewComplete { qpn, msn }
    }
}

pub(crate) struct RdmaWriteWorker {
    sq_ctx_table: QpTable<SendQueueContext>,
    qp_attr_table: QpTableShared<QpAttr>,
    send_handle: SendHandle,
    timeout_tx: TaskTx<AckTimeoutTask>,
    retransmit_tx: TaskTx<PacketRetransmitTask>,
    completion_tx: TaskTx<CompletionTask>,
}

impl SingleThreadTaskWorker for RdmaWriteWorker {
    type Task = RdmaWriteTask;

    fn process(&mut self, task: Self::Task) {
        match task {
            RdmaWriteTask::Write { qpn, wr } => {
                #[allow(clippy::wildcard_enum_match_arm)]
                let _resp = match wr.opcode() {
                    WorkReqOpCode::RdmaWrite
                    | WorkReqOpCode::RdmaWriteWithImm
                    | WorkReqOpCode::Send
                    | WorkReqOpCode::SendWithImm
                    | WorkReqOpCode::RdmaReadResp => self.write(qpn, wr),
                    WorkReqOpCode::RdmaRead => self.rdma_read(qpn, wr),
                    _ => unreachable!("opcode unsupported"),
                };
            }
            RdmaWriteTask::Ack { qpn, base_psn } => {
                let ctx = self.sq_ctx_table.get_qp_mut(qpn).expect("invalid qpn");
                ctx.update_psn_acked(base_psn);
            }
            RdmaWriteTask::NewComplete { qpn, msn } => {
                let ctx = self.sq_ctx_table.get_qp_mut(qpn).expect("invalid qpn");
                ctx.update_msn_acked(msn);
            }
        }
    }

    fn maintainance(&mut self) {}
}

impl RdmaWriteWorker {
    pub(crate) fn new(
        qp_attr_table: QpTableShared<QpAttr>,
        send_handle: SendHandle,
        timeout_tx: TaskTx<AckTimeoutTask>,
        retransmit_tx: TaskTx<PacketRetransmitTask>,
        completion_tx: TaskTx<CompletionTask>,
    ) -> Self {
        Self {
            sq_ctx_table: QpTable::new(),
            qp_attr_table,
            send_handle,
            timeout_tx,
            retransmit_tx,
            completion_tx,
        }
    }

    fn rdma_read(&mut self, qpn: u32, wr: SendWrRdma) -> io::Result<()> {
        let qp = self
            .qp_attr_table
            .get_qp(qpn)
            .ok_or(io::Error::from(io::ErrorKind::InvalidInput))?;

        let addr = wr.raddr();
        let length = wr.length();
        let num_psn = 1;
        let (msn, psn) = self
            .sq_ctx_table
            .get_qp_mut(qpn)
            .and_then(|ctx| ctx.next_wr(num_psn))
            .ok_or(io::Error::from(io::ErrorKind::InvalidInput))?;
        let end_psn = psn + num_psn;
        let qp_params = QpParams::new(
            msn,
            qp.qp_type,
            qp.qpn,
            qp.mac_addr,
            qp.dqpn,
            qp.dqp_ip,
            qp.pmtu,
        );
        let opcode = WorkReqOpCode::RdmaRead;
        let chunk = WrChunkBuilder::new_with_opcode(opcode)
            .set_qp_params(qp_params)
            .set_ibv_params(
                wr.send_flags() as u8,
                wr.rkey(),
                wr.length(),
                wr.lkey(),
                wr.imm(),
            )
            .set_chunk_meta(psn, wr.laddr(), wr.raddr(), wr.length(), ChunkPos::Only)
            .build();
        let flags = wr.send_flags();
        let mut ack_req = false;
        if flags & ibverbs_sys::ibv_send_flags::IBV_SEND_SIGNALED.0 != 0 {
            ack_req = true;
            let wr_id = wr.wr_id();
            let send_cq_handle = qp
                .send_cq
                .ok_or(io::Error::from(io::ErrorKind::InvalidInput))?;
            let event = Event::Send(SendEvent::new(
                qpn,
                SendEventOp::ReadSignaled,
                MessageMeta::new(msn, end_psn),
                wr_id,
            ));
            self.completion_tx
                .send(CompletionTask::Register { qpn, event });
        }

        if ack_req {
            self.timeout_tx.send(AckTimeoutTask::new_ack_req(qpn));
        }

        self.retransmit_tx.send(PacketRetransmitTask::NewWr {
            qpn,
            wr: SendQueueElem::new(wr, psn, qp_params),
        });

        self.send_handle.send(chunk);

        Ok(())
    }

    fn write(&mut self, qpn: u32, wr: SendWrRdma) -> io::Result<()> {
        
        let qp = self
            .qp_attr_table
            .get_qp(qpn)
            .ok_or(io::Error::from(io::ErrorKind::InvalidInput))?;

        debug!("write called with sqpn={:?}, dqpn={:?}, wr={:?}", qpn, qp.dqpn, wr);

        let addr = wr.raddr();
        let length = wr.length();
        let num_psn =
            num_psn(qp.pmtu, addr, length).ok_or(io::Error::from(io::ErrorKind::InvalidInput))?;
        let (msn, psn) = self
            .sq_ctx_table
            .get_qp_mut(qpn)
            .and_then(|ctx| ctx.next_wr(num_psn))
            .ok_or(io::Error::from(io::ErrorKind::InvalidInput))?;
        let end_psn = psn + num_psn;
        let flags = wr.send_flags();
        let mut ack_req = false;
        if flags & ibverbs_sys::ibv_send_flags::IBV_SEND_SIGNALED.0 != 0 {
            ack_req = true;
            let wr_id = wr.wr_id();
            let send_cq_handle = qp
                .send_cq
                .ok_or(io::Error::from(io::ErrorKind::InvalidInput))?;
            #[allow(clippy::wildcard_enum_match_arm)]
            let op = match wr.opcode() {
                WorkReqOpCode::RdmaWrite | WorkReqOpCode::RdmaWriteWithImm => {
                    SendEventOp::WriteSignaled
                }
                WorkReqOpCode::Send | WorkReqOpCode::SendWithImm => SendEventOp::SendSignaled,
                WorkReqOpCode::RdmaRead => SendEventOp::ReadSignaled,
                _ => return Err(io::ErrorKind::Unsupported.into()),
            };
            let event = Event::Send(SendEvent::new(
                qpn,
                op,
                MessageMeta::new(msn, end_psn),
                wr_id,
            ));
            self.completion_tx
                .send(CompletionTask::Register { qpn, event });
        }
        let qp_params = QpParams::new(
            msn,
            qp.qp_type,
            qp.qpn,
            qp.mac_addr,
            qp.dqpn,
            qp.dqp_ip,
            qp.pmtu,
        );

        if ack_req {
            let fragmenter = WrPacketFragmenter::new(wr, qp_params, psn);
            let Some(last_packet_chunk) = fragmenter.into_iter().last() else {
                debug!("RdmaWriteWorker handle write early return");
                return Ok(());
            };
            self.timeout_tx.send(AckTimeoutTask::new_ack_req(qpn));
        }

        self.retransmit_tx.send(PacketRetransmitTask::NewWr {
            qpn,
            wr: SendQueueElem::new(wr, psn, qp_params),
        });

        let fragmenter = WrChunkFragmenter::new(wr, qp_params, psn);
        for chunk in fragmenter {
            self.send_handle.send(chunk);
        }


        debug!("RdmaWriteWorker handle write done");
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use crossbeam_deque::Injector;

    use super::*;
    use crate::{
        net::config::MacAddress,
        rdma_utils::types::SendWrBase,
        workers::spawner::{task_channel, TaskRx},
    };
    use std::{net::Ipv4Addr, str::FromStr, sync::Arc};

    #[allow(clippy::struct_field_names)]
    struct Rxs {
        timeout_rx: TaskRx<AckTimeoutTask>,
        retransmit_rx: TaskRx<PacketRetransmitTask>,
        completion_rx: TaskRx<CompletionTask>,
    }

    #[allow(clippy::needless_pass_by_value)]
    impl Rxs {
        fn assert_timeout(&self, task: AckTimeoutTask) {
            let recv = self.timeout_rx.recv().unwrap();
            assert_eq!(task, recv);
        }

        fn assert_retransmit(&self, task: PacketRetransmitTask) {
            let recv = self.retransmit_rx.recv().unwrap();
            assert_eq!(task, recv);
        }

        fn assert_completion(&self, task: CompletionTask) {
            let recv = self.completion_rx.recv().unwrap();
            assert_eq!(task, recv);
        }

        fn assert_no_timeout(&self) {
            assert!(self.timeout_rx.try_recv().is_none());
        }

        fn assert_no_retransmit(&self) {
            assert!(self.retransmit_rx.try_recv().is_none());
        }

        fn assert_no_completion(&self) {
            assert!(self.completion_rx.try_recv().is_none());
        }
    }

    fn create_test_send_wr_rdma(opcode: WorkReqOpCode) -> SendWrRdma {
        let base = SendWrBase {
            wr_id: 123,
            send_flags: 0,
            laddr: 0x1000,
            length: 1024,
            lkey: 0x456,
            imm_data: 0,
            opcode,
        };
        SendWrRdma {
            base,
            raddr: 0x2000,
            rkey: 0x789,
        }
    }

    fn create_test_qp_attr() -> QpAttr {
        QpAttr {
            qp_type: ibverbs_sys::ibv_qp_type::IBV_QPT_RC as u8,
            qpn: 1,
            mac_addr: 0xAABB_CCDD_EE0A,
            dqpn: 2,
            ip: 0x13,
            dqp_ip: 0x14,
            pmtu: 1,
            send_cq: Some(1),
            recv_cq: Some(2),
            access_flags: 4,
        }
    }

    fn create_test_qp_param(msn: u16) -> QpParams {
        QpParams {
            msn,
            qp_type: ibverbs_sys::ibv_qp_type::IBV_QPT_RC as u8,
            sqpn: 1,
            mac_addr: 0xAABB_CCDD_EE0A,
            dqpn: 2,
            dqp_ip: 0x14,
            pmtu: 1,
        }
    }

    fn init_worker() -> (RdmaWriteWorker, Rxs) {
        let qp_attr_table = QpTableShared::new();
        let qp_attr = create_test_qp_attr();
        qp_attr_table.map_qp_mut(1, |attr| *attr = qp_attr).unwrap();

        let injector = Arc::new(Injector::new());
        let send_handle = SendHandle::new(injector);
        let (timeout_tx, timeout_rx) = task_channel();
        let (retransmit_tx, retransmit_rx) = task_channel();
        let (completion_tx, completion_rx) = task_channel();

        let worker = RdmaWriteWorker::new(
            qp_attr_table,
            send_handle,
            timeout_tx,
            retransmit_tx,
            completion_tx,
        );

        let rxs = Rxs {
            timeout_rx,
            retransmit_rx,
            completion_rx,
        };

        (worker, rxs)
    }

    #[test]
    fn test_process_write_task() {
        let (mut worker, rxs) = init_worker();

        let wr = create_test_send_wr_rdma(WorkReqOpCode::RdmaWrite);
        let task = RdmaWriteTask::new_write(1, wr);

        worker.process(task);

        rxs.assert_retransmit(PacketRetransmitTask::NewWr {
            qpn: 1,
            wr: SendQueueElem::new(wr, Psn(0), create_test_qp_param(0)),
        });
        rxs.assert_no_timeout();
        rxs.assert_no_completion();
    }

    #[test]
    fn test_process_rdma_read_task() {
        let (mut worker, rxs) = init_worker();

        let wr = create_test_send_wr_rdma(WorkReqOpCode::RdmaRead);
        let task = RdmaWriteTask::new_write(1, wr);

        worker.process(task);

        rxs.assert_retransmit(PacketRetransmitTask::NewWr {
            qpn: 1,
            wr: SendQueueElem::new(wr, Psn(0), create_test_qp_param(0)),
        });
        rxs.assert_no_timeout();
        rxs.assert_no_completion();
    }

    #[test]
    fn test_process_send_task() {
        let (mut worker, rxs) = init_worker();

        let wr = create_test_send_wr_rdma(WorkReqOpCode::Send);
        let task = RdmaWriteTask::new_write(1, wr);

        worker.process(task);

        rxs.assert_retransmit(PacketRetransmitTask::NewWr {
            qpn: 1,
            wr: SendQueueElem::new(wr, Psn(0), create_test_qp_param(0)),
        });
        rxs.assert_no_timeout();
        rxs.assert_no_completion();
    }

    #[test]
    fn test_process_signaled_write() {
        let (mut worker, rxs) = init_worker();

        let mut wr = create_test_send_wr_rdma(WorkReqOpCode::RdmaWrite);
        wr.base.send_flags = ibverbs_sys::ibv_send_flags::IBV_SEND_SIGNALED.0;
        let task = RdmaWriteTask::new_write(1, wr);

        worker.process(task);

        rxs.assert_retransmit(PacketRetransmitTask::NewWr {
            qpn: 1,
            wr: SendQueueElem::new(wr, Psn(0), create_test_qp_param(0)),
        });
        rxs.assert_completion(CompletionTask::Register {
            qpn: 1,
            event: Event::Send(SendEvent::new(
                1,
                SendEventOp::WriteSignaled,
                MessageMeta::new(0, Psn(4)),
                123,
            )),
        });

        rxs.assert_timeout(AckTimeoutTask::NewAckReq { qpn: 1 });
    }

    #[test]
    fn test_process_signaled_read() {
        let (mut worker, rxs) = init_worker();

        let mut wr = create_test_send_wr_rdma(WorkReqOpCode::RdmaRead);
        wr.base.send_flags = ibverbs_sys::ibv_send_flags::IBV_SEND_SIGNALED.0;
        let task = RdmaWriteTask::new_write(1, wr);

        worker.process(task);

        rxs.assert_retransmit(PacketRetransmitTask::NewWr {
            qpn: 1,
            wr: SendQueueElem::new(wr, Psn(0), create_test_qp_param(0)),
        });
        rxs.assert_completion(CompletionTask::Register {
            qpn: 1,
            event: Event::Send(SendEvent::new(
                1,
                SendEventOp::ReadSignaled,
                MessageMeta::new(0, Psn(1)),
                123,
            )),
        });

        rxs.assert_timeout(AckTimeoutTask::NewAckReq { qpn: 1 });
    }

    #[test]
    fn test_process_ack_task() {
        let (mut worker, rxs) = init_worker();

        let ctx = worker.sq_ctx_table.get_qp_mut(1).unwrap();
        let initial_psn_acked = ctx.psn_acked;

        let psn = Psn(100);
        let task = RdmaWriteTask::new_ack(1, psn);

        worker.process(task);

        // Verify PSN was updated
        let ctx = worker.sq_ctx_table.get_qp(1).unwrap();
        assert_ne!(ctx.psn_acked, initial_psn_acked);

        rxs.assert_no_timeout();
        rxs.assert_no_retransmit();
        rxs.assert_no_completion();
    }

    #[test]
    fn test_process_complete_task() {
        let (mut worker, rxs) = init_worker();

        let ctx = worker.sq_ctx_table.get_qp_mut(1).unwrap();
        let initial_msn_acked = ctx.msn_acked;

        let task = RdmaWriteTask::new_complete(1, 50);

        worker.process(task);

        // Verify MSN was updated
        let ctx = worker.sq_ctx_table.get_qp(1).unwrap();
        assert_ne!(ctx.msn_acked, initial_msn_acked);

        rxs.assert_no_timeout();
        rxs.assert_no_retransmit();
        rxs.assert_no_completion();
    }

    #[test]
    fn test_multiple_tasks_processing() {
        let (mut worker, rxs) = init_worker();

        let wr1 = create_test_send_wr_rdma(WorkReqOpCode::RdmaWrite);
        let task1 = RdmaWriteTask::new_write(1, wr1);
        worker.process(task1);

        let wr2 = create_test_send_wr_rdma(WorkReqOpCode::Send);
        let task2 = RdmaWriteTask::new_write(1, wr2);
        worker.process(task2);

        let task3 = RdmaWriteTask::new_ack(1, Psn(200));
        worker.process(task3);

        // Verify multiple retransmit tasks were sent
        assert!(rxs.retransmit_rx.try_recv().is_some());
        assert!(rxs.retransmit_rx.try_recv().is_some());

        rxs.assert_no_timeout();
        rxs.assert_no_completion();
    }

    #[test]
    fn test_signaled_operations_generate_completion_and_timeout() {
        let (mut worker, rxs) = init_worker();

        let opcodes = [
            WorkReqOpCode::RdmaWrite,
            WorkReqOpCode::RdmaWriteWithImm,
            WorkReqOpCode::Send,
            WorkReqOpCode::SendWithImm,
            WorkReqOpCode::RdmaRead,
        ];

        for opcode in opcodes {
            let mut wr = create_test_send_wr_rdma(opcode);
            wr.base.send_flags = ibverbs_sys::ibv_send_flags::IBV_SEND_SIGNALED.0;
            let task = RdmaWriteTask::new_write(1, wr);
            worker.process(task);

            assert!(rxs.retransmit_rx.try_recv().is_some());
            assert!(rxs.completion_rx.try_recv().is_some());
            assert!(rxs.timeout_rx.try_recv().is_some());
        }
    }
}
