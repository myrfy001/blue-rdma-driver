use std::io;

use crate::{
    completion::Completion,
    mem::{
        page::MmapMut, virt_to_phy::AddressResolver, DmaBuf, DmaBufAllocator, MemoryPinner,
        UmemHandler,
    },
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

pub(crate) struct MockDeviceCtx;

impl DeviceOps for MockDeviceCtx {
    fn reg_mr(&mut self, addr: u64, length: usize, pd_handle: u32, access: u8) -> io::Result<u32> {
        Ok(0)
    }

    fn dereg_mr(&mut self, mr_key: u32) -> io::Result<()> {
        Ok(())
    }

    fn create_qp(&mut self, attr: super::ops_impl::qp_attr::IbvQpInitAttr) -> io::Result<u32> {
        Ok(0)
    }

    fn update_qp(&mut self, qpn: u32, attr: super::ops_impl::qp_attr::IbvQpAttr) -> io::Result<()> {
        Ok(())
    }

    fn destroy_qp(&mut self, qpn: u32) {}

    fn create_cq(&mut self) -> Option<u32> {
        Some(0)
    }

    fn destroy_cq(&mut self, handle: u32) {}

    fn poll_cq(&mut self, handle: u32, max_num_entries: usize) -> Vec<Completion> {
        let comp = Completion::Send { wr_id: 0 };
        vec![comp]
    }

    fn post_send(&mut self, qpn: u32, wr: crate::send::SendWr) -> io::Result<()> {
        Ok(())
    }

    fn post_recv(&mut self, qpn: u32, wr: crate::recv::RecvWr) -> io::Result<()> {
        Ok(())
    }
}
