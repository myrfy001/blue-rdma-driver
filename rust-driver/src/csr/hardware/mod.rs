use memmap2::{MmapMut, MmapOptions};
use parking_lot::Mutex;
use pci_driver::{
    backends::vfio::VfioPciDevice,
    device::PciDevice,
    regions::{MappedOwningPciRegion, OwningPciRegion, PciRegion, Permissions},
};
use pci_info::PciInfo;
use std::{
    fs::{self, File, OpenOptions},
    io,
    path::{Path, PathBuf},
    sync::Arc,
};

use crate::mem::{
    dmabuf::DmaBufAllocator, page::HostPageAllocator, u_dma_buf::UDmaBufAllocator,
    virt_to_phy::PhysAddrResolverLinuxX86, HostUmemHandler,
};

use super::DeviceAdaptor;

const BAR_INDEX: usize = 1;
const BAR_MAP_RANGE_END: u64 = 4096;

#[derive(Clone, Debug)]
pub(crate) struct VfioPciCsrAdaptor {
    bar: Arc<MappedOwningPciRegion>,
}

impl VfioPciCsrAdaptor {
    fn new(sysfs_path: impl AsRef<Path>) -> io::Result<Self> {
        let path = sysfs_path.as_ref();
        let device = VfioPciDevice::open(path).map_err(|err| {
            io::Error::new(
                io::ErrorKind::Other,
                format!("Failed to open sysfs_path: {err}"),
            )
        })?;
        let bar = device.bar(BAR_INDEX).ok_or_else(|| {
            io::Error::new(io::ErrorKind::NotFound, "Expected device to have BAR")
        })?;
        let mapped_bar = bar.map(..BAR_MAP_RANGE_END, Permissions::ReadWrite)?;
        Ok(Self {
            bar: Arc::new(mapped_bar),
        })
    }
}

// TODO: use u64 instead of usize
impl DeviceAdaptor for VfioPciCsrAdaptor {
    fn read_csr(&self, addr: usize) -> io::Result<u32> {
        self.bar.read_le_u32(addr as u64)
    }

    fn write_csr(&self, addr: usize, data: u32) -> io::Result<()> {
        self.bar.write_le_u32(addr as u64, data)
    }
}

#[derive(Clone, Debug)]
pub(crate) struct SysfsPciCsrAdaptor {
    bar: Arc<Mutex<MmapMut>>,
}

#[allow(unsafe_code)]
impl SysfsPciCsrAdaptor {
    pub(crate) fn new(sysfs_path: impl AsRef<Path>) -> io::Result<Self> {
        let bar_path = sysfs_path.as_ref().join(format!("resource{BAR_INDEX}"));
        let file = OpenOptions::new().read(true).write(true).open(&bar_path)?;
        let mmap = unsafe { MmapOptions::new().map_mut(&file)? };

        Ok(Self {
            bar: Arc::new(Mutex::new(mmap)),
        })
    }
}

#[allow(unsafe_code, clippy::cast_ptr_alignment)]
impl DeviceAdaptor for SysfsPciCsrAdaptor {
    fn read_csr(&self, addr: usize) -> io::Result<u32> {
        if addr % 4 != 0 {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "unaligned access",
            ));
        }

        let bar = self.bar.lock();
        unsafe {
            let ptr = bar.as_ptr().add(addr);
            Ok(ptr.cast::<u32>().read_volatile())
        }
    }

    fn write_csr(&self, addr: usize, data: u32) -> io::Result<()> {
        if addr % 4 != 0 {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "unaligned access",
            ));
        }

        let mut bar = self.bar.lock();
        unsafe {
            let ptr = bar.as_mut_ptr().add(addr);
            ptr.cast::<u32>().write_volatile(data);
        }

        Ok(())
    }
}

pub(crate) struct CustomCsrConfigurator {
    bar: MmapMut,
}

#[allow(unsafe_code, clippy::cast_ptr_alignment)]
impl CustomCsrConfigurator {
    pub(crate) fn new(sysfs_path: impl AsRef<Path>) -> io::Result<Self> {
        let bar_path = sysfs_path.as_ref().join(format!("resource{BAR_INDEX}"));
        let file = OpenOptions::new().read(true).write(true).open(&bar_path)?;
        let mmap = unsafe { MmapOptions::new().map_mut(&file)? };

        Ok(Self { bar: mmap })
    }

    pub(crate) fn set_loopback(&mut self) {
        const ADDR: usize = 0x180;
        unsafe {
            self.bar
                .as_mut_ptr()
                .add(ADDR)
                .cast::<u32>()
                .write_volatile(1);
        }
    }

    pub(crate) fn set_seed(&mut self, seed: u32) {
        const ADDR: usize = 0x184;
        unsafe {
            self.bar
                .as_mut_ptr()
                .add(ADDR)
                .cast::<u32>()
                .write_volatile(seed);
        }
    }

    pub(crate) fn set_drop_thresh(&mut self, rate: u8) {
        const ADDR: usize = 0x188;
        unsafe {
            self.bar
                .as_mut_ptr()
                .add(ADDR)
                .cast::<u32>()
                .write_volatile(u32::from(rate));
        }
    }
}
