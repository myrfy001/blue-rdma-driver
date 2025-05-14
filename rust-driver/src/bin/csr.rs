use std::{
    fs, io,
    path::{Path, PathBuf},
};

use pci_info::PciInfo;

const VENDER_ID: u16 = 0x10ee;
const DEVICE_ID: u16 = 0x903f;
const PCI_SYSFS_BUS_PATH: &str = "/sys/bus/pci/devices";

pub(crate) struct PciHwDevice {
    sysfs_path: PathBuf,
}

impl PciHwDevice {
    pub(crate) fn open_default() -> io::Result<Self> {
        let build_err = || io::Error::new(io::ErrorKind::Other, "Failed to open device");
        let info = PciInfo::enumerate_pci().map_err(|_err| build_err())?;
        let device = info
            .iter()
            .flatten()
            .find(|d| d.vendor_id() == VENDER_ID && d.device_id() == DEVICE_ID)
            .ok_or_else(build_err)?;
        let location = device.location().map_err(|_err| build_err())?;
        let sysfs_path = PathBuf::from(PCI_SYSFS_BUS_PATH).join(location.to_string());

        Ok(Self { sysfs_path })
    }
}

pub(crate) struct DebugInfoFetcher {
    bar: memmap2::MmapMut,
}

#[allow(unsafe_code, clippy::cast_ptr_alignment)]
impl DebugInfoFetcher {
    const RQ_FIFO: usize = 0x4000;
    const INPUT_PACKET_CLASSIFIER_FIFO: usize = 0x4400;
    const INPUT_PACKET_CLASSIFIER_1: usize = 0x4404;
    const RDMA_HEADER_EXTRACTOR_FIFO: usize = 0x4480;
    const PAYLODGEN_FIFO: usize = 0x4800;
    const AUTOACKGEN_FIFO: usize = 0x5200;
    const DMA_ENGINE_FIFO: usize = 0x8000;

    pub(crate) fn new(sysfs_path: impl AsRef<Path>) -> io::Result<Self> {
        let bar_path = sysfs_path.as_ref().join(format!("resource0"));
        let file = fs::OpenOptions::new()
            .read(true)
            .write(true)
            .open(&bar_path)?;
        let mmap = unsafe { memmap2::MmapOptions::new().map_mut(&file)? };

        Ok(Self { bar: mmap })
    }

    pub(crate) fn get_rq_fifo_status(&self) -> u32 {
        unsafe {
            self.bar
                .as_ptr()
                .add(Self::RQ_FIFO)
                .cast::<u32>()
                .read_volatile()
        }
    }
    pub(crate) fn get_input_packet_classifier_fifo_status(&self) -> u32 {
        unsafe {
            self.bar
                .as_ptr()
                .add(Self::INPUT_PACKET_CLASSIFIER_FIFO)
                .cast::<u32>()
                .read_volatile()
        }
    }

    pub(crate) fn get_input_packet_classifier_1_status(&self) -> u32 {
        unsafe {
            self.bar
                .as_ptr()
                .add(Self::INPUT_PACKET_CLASSIFIER_1)
                .cast::<u32>()
                .read_volatile()
        }
    }

    pub(crate) fn get_rdma_header_extractor_fifo_status(&self) -> u32 {
        unsafe {
            self.bar
                .as_ptr()
                .add(Self::RDMA_HEADER_EXTRACTOR_FIFO)
                .cast::<u32>()
                .read_volatile()
        }
    }

    pub(crate) fn get_payloadgen_fifo_status(&self) -> u32 {
        unsafe {
            self.bar
                .as_ptr()
                .add(Self::PAYLODGEN_FIFO)
                .cast::<u32>()
                .read_volatile()
        }
    }

    pub(crate) fn get_autoackgen_fifo_status(&self) -> u32 {
        unsafe {
            self.bar
                .as_ptr()
                .add(Self::AUTOACKGEN_FIFO)
                .cast::<u32>()
                .read_volatile()
        }
    }

    pub(crate) fn get_dma_engine_fifo_status(&self) -> u32 {
        unsafe {
            self.bar
                .as_ptr()
                .add(Self::DMA_ENGINE_FIFO)
                .cast::<u32>()
                .read_volatile()
        }
    }
}

struct InfoPrinter(DebugInfoFetcher);

impl InfoPrinter {
    fn print_binary(&self) {
        println!("FIFO Status Values (Binary):");
        println!("--------------------------");

        let rq_status = self.0.get_rq_fifo_status();
        println!("RQ FIFO:                    {:#034b}", rq_status);

        let ipc_status = self.0.get_input_packet_classifier_fifo_status();
        println!("Input Packet Classifier:    {:#034b}", ipc_status);

        let ipc1_status = self.0.get_input_packet_classifier_1_status();
        println!("Input Packet Classifier 1:  {:#034b}", ipc1_status);

        let rdma_status = self.0.get_rdma_header_extractor_fifo_status();
        println!("RDMA Header Extractor:      {:#034b}", rdma_status);

        let payload_status = self.0.get_payloadgen_fifo_status();
        println!("Payload Generator:          {:#034b}", payload_status);

        let autoack_status = self.0.get_autoackgen_fifo_status();
        println!("Auto ACK Generator:         {:#034b}", autoack_status);

        let dma_status = self.0.get_dma_engine_fifo_status();
        println!("DMA Engine:                 {:#034b}", dma_status);

        println!("--------------------------");
    }
}

fn main() {
    let dev = PciHwDevice::open_default().unwrap();
    let fetcher = DebugInfoFetcher::new(dev.sysfs_path).unwrap();
    let printer = InfoPrinter(fetcher);
    printer.print_binary();
}
