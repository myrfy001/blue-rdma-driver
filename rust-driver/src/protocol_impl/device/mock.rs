use std::{
    collections::{HashMap, VecDeque},
    io,
};

use bitvec::store::BitStore;
use log::info;

use crate::{
    completion::Completion,
    mem::{
        page::MmapMut, virt_to_phy::AddressResolver, DmaBuf, DmaBufAllocator, MemoryPinner,
        UmemHandler,
    },
    recv::RecvWr,
    send::SendWr,
};

use super::{
    ops_impl::{DeviceOps, HwDevice},
    DeviceAdaptor,
};

#[derive(Debug, Clone, Copy)]
pub(crate) struct MockDeviceAdaptor;

impl DeviceAdaptor for MockDeviceAdaptor {
    fn read_csr(&self, addr: usize) -> io::Result<u32> {
        Ok(0)
    }

    fn write_csr(&self, addr: usize, data: u32) -> io::Result<()> {
        Ok(())
    }
}

#[derive(Debug, Clone, Copy)]
pub(crate) struct MockDmaBufAllocator;

impl DmaBufAllocator for MockDmaBufAllocator {
    fn alloc(&mut self, len: usize) -> io::Result<DmaBuf> {
        const LEN: usize = 4096 * 32;
        #[allow(unsafe_code)]
        let ptr = unsafe {
            libc::mmap(
                std::ptr::null_mut(),
                LEN,
                libc::PROT_READ | libc::PROT_WRITE,
                libc::MAP_SHARED | libc::MAP_ANON,
                -1,
                0,
            )
        };

        let mmap = MmapMut::new(ptr, usize::MAX);
        Ok(DmaBuf::new(mmap, 0))
    }
}

#[derive(Debug, Clone, Copy)]
pub(crate) struct MockUmemHandler;

impl MemoryPinner for MockUmemHandler {
    fn pin_pages(&self, addr: u64, length: usize) -> io::Result<()> {
        Ok(())
    }

    fn unpin_pages(&self, addr: u64, length: usize) -> io::Result<()> {
        Ok(())
    }
}

impl AddressResolver for MockUmemHandler {
    fn virt_to_phys(&self, virt_addr: u64) -> io::Result<Option<u64>> {
        Ok(Some(0))
    }
}

impl UmemHandler for MockUmemHandler {}

#[derive(Debug)]
pub(crate) struct MockHwDevice;

impl HwDevice for MockHwDevice {
    type Adaptor = MockDeviceAdaptor;

    type DmaBufAllocator = MockDmaBufAllocator;

    type UmemHandler = MockUmemHandler;

    fn new_adaptor(&self) -> io::Result<Self::Adaptor> {
        Ok(MockDeviceAdaptor)
    }

    fn new_dma_buf_allocator(&self) -> io::Result<Self::DmaBufAllocator> {
        Ok(MockDmaBufAllocator)
    }

    fn new_umem_handler(&self) -> Self::UmemHandler {
        MockUmemHandler
    }
}

#[derive(Default)]
pub(crate) struct MockDeviceCtx {
    mr_key: u32,
    qpn: u32,
    cq_handle: u32,
    cq_table: HashMap<u32, VecDeque<Completion>>,
    send_qp_cq_map: HashMap<u32, u32>,
    recv_qp_cq_map: HashMap<u32, u32>,
}

impl DeviceOps for MockDeviceCtx {
    fn reg_mr(&mut self, addr: u64, length: usize, pd_handle: u32, access: u8) -> io::Result<u32> {
        self.mr_key += 1;
        info!("mock reg mr");
        Ok(self.mr_key)
    }

    fn dereg_mr(&mut self, mr_key: u32) -> io::Result<()> {
        info!("mock dereg mr");
        Ok(())
    }

    fn create_qp(&mut self, attr: super::ops_impl::qp_attr::IbvQpInitAttr) -> io::Result<u32> {
        self.qpn += 1;
        if let Some(h) = attr.send_cq() {
            let _ignore = self.send_qp_cq_map.insert(self.qpn, h);
        }
        if let Some(h) = attr.send_cq() {
            let _ignore = self.recv_qp_cq_map.insert(self.qpn, h);
        }
        info!("mock create qp: {}", self.qpn);

        Ok(self.qpn)
    }

    fn update_qp(&mut self, qpn: u32, attr: super::ops_impl::qp_attr::IbvQpAttr) -> io::Result<()> {
        info!("mock update qp");
        Ok(())
    }

    fn destroy_qp(&mut self, qpn: u32) {
        info!("mock destroy qp");
    }

    fn create_cq(&mut self) -> Option<u32> {
        self.cq_handle += 1;
        info!("mock create cq, handle: {}", self.cq_handle);
        Some(self.cq_handle)
    }

    fn destroy_cq(&mut self, handle: u32) {
        info!("mock destroy cq, handle: {handle}");
    }

    fn poll_cq(&mut self, handle: u32, max_num_entries: usize) -> Vec<Completion> {
        let completions = if let Some(cq) = self.cq_table.get_mut(&handle) {
            cq.pop_front().into_iter().collect()
        } else {
            vec![]
        };
        info!("completions: {completions:?}");
        completions
    }

    fn post_send(&mut self, qpn: u32, wr: SendWr) -> io::Result<()> {
        if wr.send_flags() & ibverbs_sys::ibv_send_flags::IBV_SEND_SIGNALED.0 != 0 {
            let completion = match wr {
                SendWr::Rdma(_) => Completion::RdmaWrite { wr_id: wr.wr_id() },
                SendWr::Send(_) => Completion::Send { wr_id: wr.wr_id() },
            };
            if let Some(cq) = self
                .send_qp_cq_map
                .get_mut(&qpn)
                .and_then(|h| self.cq_table.get_mut(h))
            {
                cq.push_back(completion);
            }
        }
        info!("post send wr: {wr:?}");

        Ok(())
    }

    fn post_recv(&mut self, qpn: u32, wr: RecvWr) -> io::Result<()> {
        let completion = Completion::Recv {
            wr_id: wr.wr_id,
            imm: None,
        };
        if let Some(cq) = self
            .send_qp_cq_map
            .get_mut(&qpn)
            .and_then(|h| self.cq_table.get_mut(h))
        {
            cq.push_back(completion);
        }
        info!("post recv wr: {wr:?}");

        Ok(())
    }
}
