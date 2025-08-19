use std::io;

use log::{error, debug};

use crate::{
    constants::PSN_MASK,
    csr::{proxy::MetaReportQueueProxy, CsrReaderAdaptor},
    rdma_utils::psn::Psn,
    ringbuf::DescRingBuffer,
};

use crate::{
    csr::{mode::Mode, proxy::build_meta_report_queue_proxies, CsrBaseAddrAdaptor, DeviceAdaptor},
    descriptors::{
        MetaReportQueueAckDesc, MetaReportQueueAckExtraDesc, MetaReportQueueDescFirst,
        MetaReportQueueDescNext, MetaReportQueuePacketBasicInfoDesc,
        MetaReportQueueReadReqExtendInfoDesc,
    },
    mem::DmaBuf,
    workers::{
        ack_responder::AckResponse, completion::CompletionTask, qp_timeout::AckTimeoutTask,
        rdma::RdmaWriteTask, retransmit::PacketRetransmitTask,
    },
};

pub(crate) struct MetaReportQueueCtx<Dev> {
    queue: MetaReportQueue,
    proxy: MetaReportQueueProxy<Dev>,
}

impl<Dev> MetaReportQueueCtx<Dev> {
    pub(crate) fn new(queue: MetaReportQueue, proxy: MetaReportQueueProxy<Dev>) -> Self {
        Self { queue, proxy }
    }
}

/// Handler for meta report queues
pub(crate) struct MetaReportQueueHandler<Dev> {
    /// All four meta report queues
    inner: Vec<MetaReportQueueCtx<Dev>>,
    /// Current position, used for round robin polling
    pos: usize,
}

impl<Dev> MetaReportQueueHandler<Dev> {
    pub(crate) fn new(inner: Vec<MetaReportQueueCtx<Dev>>) -> Self {
        Self { inner, pos: 0 }
    }

    fn remap_psn(psn: Psn) -> Psn {
        // 128 (window size) - 16 (first stride)
        const OFFSET: u32 = 112;
        psn - OFFSET
    }
}

impl<Dev: DeviceAdaptor> MetaReportQueueHandler<Dev> {
    #[allow(clippy::arithmetic_side_effects, clippy::indexing_slicing)] // should never overflow
    pub(crate) fn try_recv_meta(&mut self) -> Option<ReportMeta> {
        let num_queues = self.inner.len();
        for i in 0..num_queues {
            let idx = (self.pos + i) % num_queues;
            let ctx = &mut self.inner[idx];
            let Some(desc) = ctx.queue.pop() else {
                continue;
            };
            let _ignore = ctx.proxy.write_tail(ctx.queue.tail());
            if let Ok(head_ptr) = ctx.proxy.read_head() {
                ctx.queue.set_head(head_ptr);
            }

            self.pos = (idx + 1) % num_queues;
            let meta = match desc {
                MetaReportQueueDesc::WritePacketInfo(d) => {
                    ReportMeta::HeaderWrite(HeaderWriteMeta {
                        pos: d.packet_pos(),
                        msn: d.msn(),
                        psn: d.psn().into(),
                        solicited: d.solicited(),
                        ack_req: d.ack_req(),
                        is_retry: d.is_retry(),
                        dqpn: d.dqpn(),
                        total_len: d.total_len(),
                        raddr: d.raddr(),
                        rkey: d.rkey(),
                        imm: d.imm_data(),
                        header_type: d.header_type(),
                    })
                }
                MetaReportQueueDesc::ReadPacketInfo((f, n)) => {
                    ReportMeta::HeaderRead(HeaderReadMeta {
                        dqpn: f.dqpn(),
                        raddr: f.raddr(),
                        rkey: f.rkey(),
                        total_len: n.total_len(),
                        laddr: n.laddr(),
                        lkey: n.lkey(),
                        ack_req: f.ack_req(),
                        msn: f.msn(),
                        psn: f.psn().into(),
                    })
                }
                MetaReportQueueDesc::CnpPacketInfo(d) => ReportMeta::Cnp(CnpMeta { qpn: d.dqpn() }),
                MetaReportQueueDesc::Ack(d) => {
                    match (d.is_send_by_driver(), d.is_send_by_local_hw()) {
                        (true, false) => ReportMeta::AckRemoteDriver(AckMetaRemoteDriver {
                            qpn: d.qpn(),
                            psn_now: d.psn_now().into(),
                        }),
                        (false, true) => ReportMeta::AckLocalHw(AckMetaLocalHw {
                            qpn: d.qpn(),
                            psn_now: Self::remap_psn(d.psn_now().into()),
                            now_bitmap: d.now_bitmap(),
                        }),
                        (false, false) | (true, true) => unreachable!("invalid ack branch"),
                    }
                }
                MetaReportQueueDesc::Nak((f, n)) => {
                    match (f.is_send_by_driver(), f.is_send_by_local_hw()) {
                        (true, false) => ReportMeta::NakRemoteDriver(NakMetaRemoteDriver {
                            qpn: f.qpn(),
                            psn_now: f.psn_now().into(),
                            psn_pre: f.psn_before_slide().into(),
                        }),
                        (false, true) => ReportMeta::NakLocalHw(NakMetaLocalHw {
                            qpn: f.qpn(),
                            msn: f.msn(),
                            psn_now: Self::remap_psn(f.psn_now().into()),
                            now_bitmap: f.now_bitmap(),
                            psn_pre: Self::remap_psn(f.psn_before_slide().into()),
                            pre_bitmap: n.pre_bitmap(),
                        }),
                        (false, false) => ReportMeta::NakRemoteHw(NakMetaRemoteHw {
                            qpn: f.qpn(),
                            msn: f.msn(),
                            psn_now: Self::remap_psn(f.psn_now().into()),
                            now_bitmap: f.now_bitmap(),
                            psn_pre: Self::remap_psn(f.psn_before_slide().into()),
                            pre_bitmap: n.pre_bitmap(),
                        }),
                        (true, true) => unreachable!("invalid nak branch"),
                    }
                }
            };
            debug!("meta report queue {i} got new desc: {meta:?}");
            return Some(meta);
        }
        None
    }
}

/// Meta report queue descriptors
pub(crate) enum MetaReportQueueDesc {
    /// Packet info for write operations
    WritePacketInfo(MetaReportQueuePacketBasicInfoDesc),
    /// Packet info for read operations
    ReadPacketInfo(
        (
            MetaReportQueuePacketBasicInfoDesc,
            MetaReportQueueReadReqExtendInfoDesc,
        ),
    ),
    /// Packet info for congestion event
    CnpPacketInfo(MetaReportQueuePacketBasicInfoDesc),
    /// Ack
    Ack(MetaReportQueueAckDesc),
    /// Nak
    Nak((MetaReportQueueAckDesc, MetaReportQueueAckExtraDesc)),
}

/// A transmit queue for the simple NIC device.
pub(crate) struct MetaReportQueue {
    /// Inner ring buffer
    inner: DescRingBuffer,
}

impl MetaReportQueue {
    pub(crate) fn new(inner: DescRingBuffer) -> Self {
        Self { inner }
    }

    /// Tries to poll next valid entry from the queue
    pub(crate) fn pop(&mut self) -> Option<MetaReportQueueDesc> {
        let (first, next) = self.inner.pop_two();
        #[allow(clippy::wildcard_enum_match_arm)] // too verbose
        match (first, next) {
            (None, None) => None,
            (Some(MetaReportQueueDescFirst::PacketInfo(d)), None) if d.ecn_marked() => {
                Some(MetaReportQueueDesc::CnpPacketInfo(d))
            }
            (Some(MetaReportQueueDescFirst::PacketInfo(d)), None) => {
                Some(MetaReportQueueDesc::WritePacketInfo(d))
            }
            (Some(MetaReportQueueDescFirst::Ack(d)), None) => Some(MetaReportQueueDesc::Ack(d)),
            (
                Some(MetaReportQueueDescFirst::PacketInfo(f)),
                Some(MetaReportQueueDescNext::ReadInfo(n)),
            ) => Some(MetaReportQueueDesc::ReadPacketInfo((f, n))),
            (
                Some(MetaReportQueueDescFirst::Ack(f)),
                Some(MetaReportQueueDescNext::AckExtra(n)),
            ) => Some(MetaReportQueueDesc::Nak((f, n))),
            _ => {
                unreachable!("invalid descriptor format")
            }
        }
    }

    pub(crate) fn tail(&self) -> u32 {
        self.inner.tail() as u32
    }

    pub(crate) fn set_head(&mut self, head: u32) {
        self.inner.set_head(head);
    }

    pub(crate) fn remaining(&self) -> usize {
        self.inner.remaining()
    }
}

/// The position of a packet
#[derive(Debug, Clone, Copy)]
pub(crate) enum PacketPos {
    /// First packet
    First,
    /// Middle packet
    Middle,
    /// Last packet
    Last,
    /// Only packet
    Only,
}

/// Metadata from meta report queue
#[derive(Debug, Clone, Copy)]
pub(crate) enum ReportMeta {
    /// Write operation header
    HeaderWrite(HeaderWriteMeta),
    /// Read operation header
    HeaderRead(HeaderReadMeta),
    /// ACK generated by the local hardware
    AckLocalHw(AckMetaLocalHw),
    /// ACK generated by the remote driver
    AckRemoteDriver(AckMetaRemoteDriver),
    /// NAK generated by the local hardware
    NakLocalHw(NakMetaLocalHw),
    /// NAK generated by the remote hardware
    NakRemoteHw(NakMetaRemoteHw),
    /// NAK generated by the remote driver
    NakRemoteDriver(NakMetaRemoteDriver),
    /// Congestion Notification Packet
    Cnp(CnpMeta),
}

impl ReportMeta {
    pub(crate) fn qpn(&self) -> u32 {
        match *self {
            ReportMeta::HeaderWrite(x) => x.dqpn,
            ReportMeta::HeaderRead(x) => x.dqpn,
            ReportMeta::AckLocalHw(x) => x.qpn,
            ReportMeta::AckRemoteDriver(x) => x.qpn,
            ReportMeta::NakLocalHw(x) => x.qpn,
            ReportMeta::NakRemoteHw(x) => x.qpn,
            ReportMeta::NakRemoteDriver(x) => x.qpn,
            ReportMeta::Cnp(x) => x.qpn,
        }
    }
}

#[derive(Debug, Clone, Copy)]
pub(crate) struct HeaderWriteMeta {
    pub(crate) pos: PacketPos,
    pub(crate) msn: u16,
    pub(crate) psn: Psn,
    pub(crate) solicited: bool,
    pub(crate) ack_req: bool,
    pub(crate) is_retry: bool,
    pub(crate) dqpn: u32,
    pub(crate) total_len: u32,
    pub(crate) raddr: u64,
    pub(crate) rkey: u32,
    pub(crate) imm: u32,
    pub(crate) header_type: HeaderType,
}

#[derive(Debug, Clone, Copy)]
pub(crate) struct HeaderReadMeta {
    pub(crate) msn: u16,
    pub(crate) psn: Psn,
    pub(crate) dqpn: u32,
    pub(crate) raddr: u64,
    pub(crate) rkey: u32,
    pub(crate) total_len: u32,
    pub(crate) laddr: u64,
    pub(crate) lkey: u32,
    pub(crate) ack_req: bool,
}

#[derive(Debug, Clone, Copy)]
pub(crate) struct CnpMeta {
    /// The initiator's QP number
    pub(crate) qpn: u32,
}

#[derive(Debug, Clone, Copy)]
pub(crate) struct AckMetaLocalHw {
    pub(crate) qpn: u32,
    pub(crate) psn_now: Psn,
    pub(crate) now_bitmap: u128,
}

#[derive(Debug, Clone, Copy)]
pub(crate) struct AckMetaRemoteDriver {
    pub(crate) qpn: u32,
    pub(crate) psn_now: Psn,
}

#[derive(Debug, Clone, Copy)]
pub(crate) struct NakMetaLocalHw {
    pub(crate) qpn: u32,
    pub(crate) msn: u16,
    pub(crate) psn_now: Psn,
    pub(crate) now_bitmap: u128,
    pub(crate) psn_pre: Psn,
    pub(crate) pre_bitmap: u128,
}

#[derive(Debug, Clone, Copy)]
pub(crate) struct NakMetaRemoteHw {
    pub(crate) qpn: u32,
    pub(crate) msn: u16,
    pub(crate) psn_now: Psn,
    pub(crate) now_bitmap: u128,
    pub(crate) psn_pre: Psn,
    pub(crate) pre_bitmap: u128,
}

#[derive(Debug, Clone, Copy)]
pub(crate) struct NakMetaRemoteDriver {
    pub(crate) qpn: u32,
    pub(crate) psn_now: Psn,
    pub(crate) psn_pre: Psn,
}

#[derive(Debug, PartialEq, Eq, Clone, Copy)]
pub(crate) enum HeaderType {
    Write,
    WriteWithImm,
    Send,
    SendWithImm,
    ReadResp,
}
