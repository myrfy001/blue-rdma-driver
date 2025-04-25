use std::{
    io,
    net::{IpAddr, Ipv4Addr},
};

use ipnetwork::Ipv4Network;

use crate::{
    device_protocol::DeviceCommand,
    mem::{sim_alloc, DmaBufAllocator, PageWithPhysAddr},
    net::config::{MacAddress, NetworkConfig},
    protocol_impl::device::CsrWriterAdaptor,
};

use super::{
    desc::CmdQueueReqDescSetNetworkParam,
    device::{
        ffi_impl::EmulatedHwDevice,
        hardware::{DmaEngineConfigurator, PciHwDevice},
        ops_impl::HwDevice,
    },
    queue::{alloc::DescRingBufAllocator, cmd_queue::CmdQueueDesc},
    CommandController,
};

#[allow(
    clippy::unwrap_used,
    missing_docs,
    clippy::missing_errors_doc,
    clippy::missing_panics_doc
)]
#[inline]
pub fn run_test_rb() {
    env_logger::try_init().unwrap();
    let device = PciHwDevice::open_default().unwrap();
    device.reset().unwrap();
    device.init_dma_engine().unwrap();
    let adaptor = device.new_adaptor().unwrap();
    let mut allocator = device.new_dma_buf_allocator().unwrap();
    let mut rb_allocator = DescRingBufAllocator::new(allocator);
    let cmd_controller = CommandController::init_v2(
        &adaptor,
        rb_allocator.alloc().unwrap(),
        rb_allocator.alloc().unwrap(),
    )
    .unwrap();
    let network_config = NetworkConfig {
        ip: Ipv4Network::new("10.0.0.2".parse().unwrap(), 24).unwrap(),
        gateway: "10.0.0.1".parse().unwrap(),
        mac: MacAddress([0; 6]),
    };
    let mut cnt = 0;
    loop {
        cnt += 1;
        log::info!("cnt: {cnt}");
        cmd_controller.set_network(network_config).unwrap();
        let network = network_config.ip;
        let IpAddr::V4(gateway) = network_config.gateway else {
            unreachable!("IPv6 unsupported")
        };
        let desc = CmdQueueReqDescSetNetworkParam::new(
            0,
            gateway.to_bits(),
            network.mask().to_bits(),
            network.ip().to_bits(),
            network_config.mac.into(),
        );
        let mut qp = cmd_controller.cmd_qp.lock();
        let mut update = qp.update();
        update.push(CmdQueueDesc::SetNetworkParam(desc));
        update.flush(&cmd_controller.req_csr_proxy);
        cmd_controller.print_resp_info();
        // cmd_controller.req_csr_proxy.read_tail()
    }
}
