use core::ffi::{CStr, c_char, c_void};
use core::ptr::{self, NonNull};

use ffi::{ibv_free_device_list, ibv_get_device_list, ibv_open_device};

use super::imp::{Rxe, to_rxe_context};

#[unsafe(export_name = "bluerdma_init")]
pub unsafe extern "C" fn init() {
    let _ = env_logger::try_init();
}

/// Safety: caller must ensure `sysfs_name` is a valid pointer.
#[unsafe(export_name = "bluerdma_new")]
pub unsafe extern "C" fn new(sysfs_name: *const c_char) -> *mut c_void {
    // Safety: caller must ensure `sysfs_name` is a valid pointer.
    let sysfs_name = unsafe { CStr::from_ptr(sysfs_name) }.to_str().unwrap();

    log::info!("Creating new RDMA device with sysfs name: {sysfs_name}");

    let mut num_devices = 0;

    // Safety: `num_devices` is a valid pointer.
    let list = unsafe { ibv_get_device_list(&raw mut num_devices) };

    // Safety: `list` is at least `num_devices` long.
    let device_list = unsafe { std::slice::from_raw_parts(list, num_devices.try_into().unwrap()) };

    // TODO: remove hardcode index
    let rxe_device = device_list[0];

    let rxe = unsafe { ibv_open_device(rxe_device) };

    unsafe { ibv_free_device_list(list) };

    Box::into_raw(Box::new(Rxe::new(NonNull::new(rxe).unwrap()))).cast()
}

#[unsafe(export_name = "bluerdma_free")]
pub extern "C" fn free(driver_data: *const c_void) {
    // Safety: caller must ensure `driver_data` is a valid pointer.
    let rxe_ptr = driver_data.cast::<Rxe>().cast_mut();
    let _ = unsafe { Box::from_raw(rxe_ptr) };
}

#[unsafe(export_name = "bluerdma_alloc_pd")]
pub unsafe extern "C" fn alloc_pd(blue_context: *mut ffi::ibv_context) -> *mut ffi::ibv_pd {
    log::info!("Allocating protection domain");

    let rxe_context = unsafe { to_rxe_context(blue_context) };

    unsafe { ffi::ibv_alloc_pd(rxe_context) }
}

#[unsafe(export_name = "bluerdma_dealloc_pd")]
pub unsafe extern "C" fn dealloc_pd(pd: *mut ffi::ibv_pd) -> ::std::os::raw::c_int {
    log::info!("Deallocating protection domain");

    let blue_context = unsafe { pd.as_ref() }.unwrap().context;
    let rxe_context = unsafe { to_rxe_context(blue_context) };

    let pd_mut = unsafe { pd.as_mut() }.unwrap();

    pd_mut.context = rxe_context;
    let rc = unsafe { ffi::ibv_dealloc_pd(pd) };

    rc
}

#[unsafe(export_name = "bluerdma_query_device_ex")]
pub unsafe extern "C" fn query_device_ex(
    context: *mut ffi::ibv_context,
    _input: *const ffi::ibv_query_device_ex_input,
    device_attr: *mut ffi::ibv_device_attr,
    _attr_size: usize,
) -> ::std::os::raw::c_int {
    log::info!("Querying device attributes");

    let context = unsafe { to_rxe_context(context) };
    let ctx = unsafe { context.as_ref() }.unwrap();

    unsafe { ctx.ops._compat_query_device.unwrap()(context, device_attr) }
}

#[unsafe(export_name = "bluerdma_query_port")]
pub unsafe extern "C" fn query_port(
    context: *mut ffi::ibv_context,
    port_num: u8,
    port_attr: *mut ffi::ibv_port_attr,
) -> ::std::os::raw::c_int {
    log::info!("Querying port attributes");

    let context = unsafe { to_rxe_context(context) };
    let ctx = unsafe { context.as_ref() }.unwrap();

    unsafe { ctx.ops._compat_query_port.unwrap()(context, port_num, port_attr.cast()) }
}

#[unsafe(export_name = "bluerdma_create_cq")]
pub unsafe extern "C" fn create_cq(
    blue_context: *mut ffi::ibv_context,
    cqe: core::ffi::c_int,
    channel: *mut ffi::ibv_comp_channel,
    comp_vector: core::ffi::c_int,
) -> *mut ffi::ibv_cq {
    log::info!("Creating completion queue");

    let rxe_context = unsafe { to_rxe_context(blue_context) };

    unsafe { ffi::ibv_create_cq(rxe_context, cqe, ptr::null_mut(), channel, comp_vector) }
}

#[unsafe(export_name = "bluerdma_destroy_cq")]
pub unsafe extern "C" fn destroy_cq(cq: *mut ffi::ibv_cq) -> ::std::os::raw::c_int {
    log::info!("Destroying completion queue");

    let blue_context = unsafe { cq.as_ref() }.unwrap().context;
    let rxe_context = unsafe { to_rxe_context(blue_context) };

    let cq_mut = unsafe { cq.as_mut() }.unwrap();

    cq_mut.context = rxe_context;
    let rc = unsafe { ffi::ibv_destroy_cq(cq) };

    rc
}

#[unsafe(export_name = "bluerdma_create_qp")]
pub unsafe extern "C" fn create_qp(pd: *mut ffi::ibv_pd, init_attr: *mut ffi::ibv_qp_init_attr) -> *mut ffi::ibv_qp {
    log::info!("Creating queue pair");

    let blue_context = unsafe { pd.as_ref() }.unwrap().context;
    let rxe_context = unsafe { to_rxe_context(blue_context) };

    let pd_mut = unsafe { pd.as_mut() }.unwrap();

    pd_mut.context = rxe_context;
    let qp = unsafe { ffi::ibv_create_qp(pd, init_attr) };
    pd_mut.context = blue_context;

    let qp_mut = unsafe { qp.as_mut() }.unwrap();
    qp_mut.context = blue_context;

    qp
}

#[unsafe(export_name = "bluerdma_destroy_qp")]
pub unsafe extern "C" fn destroy_qp(qp: *mut ffi::ibv_qp) -> ::std::os::raw::c_int {
    log::info!("Destroying queue pair");

    let blue_context = unsafe { qp.as_ref() }.unwrap().context;
    let rxe_context = unsafe { to_rxe_context(blue_context) };

    let qp_mut = unsafe { qp.as_mut() }.unwrap();

    qp_mut.context = rxe_context;
    let rc = unsafe { ffi::ibv_destroy_qp(qp) };

    rc
}

#[unsafe(export_name = "bluerdma_modify_qp")]
pub unsafe extern "C" fn modify_qp(
    qp: *mut ffi::ibv_qp,
    attr: *mut ffi::ibv_qp_attr,
    attr_mask: core::ffi::c_int,
) -> ::std::os::raw::c_int {
    log::info!("Modifying queue pair");

    let blue_context = unsafe { qp.as_ref() }.unwrap().context;
    let rxe_context = unsafe { to_rxe_context(blue_context) };

    let qp_mut = unsafe { qp.as_mut() }.unwrap();

    qp_mut.context = rxe_context;
    let rc = unsafe { ffi::ibv_modify_qp(qp, attr, attr_mask) };
    qp_mut.context = blue_context;

    rc
}

#[unsafe(export_name = "bluerdma_query_qp")]
pub unsafe extern "C" fn query_qp(
    qp: *mut ffi::ibv_qp,
    attr: *mut ffi::ibv_qp_attr,
    attr_mask: core::ffi::c_int,
    init_attr: *mut ffi::ibv_qp_init_attr,
) -> ::std::os::raw::c_int {
    log::info!("Querying queue pair");

    let blue_context = unsafe { qp.as_ref() }.unwrap().context;
    let rxe_context = unsafe { to_rxe_context(blue_context) };

    let qp_mut = unsafe { qp.as_mut() }.unwrap();

    qp_mut.context = rxe_context;
    let rc = unsafe { ffi::ibv_query_qp(qp, attr, attr_mask, init_attr) };
    qp_mut.context = blue_context;

    rc
}

#[unsafe(export_name = "bluerdma_reg_mr")]
pub unsafe extern "C" fn reg_mr(
    pd: *mut ffi::ibv_pd,
    addr: *mut ::std::os::raw::c_void,
    length: usize,
    _hca_va: u64,
    access: core::ffi::c_int,
) -> *mut ffi::ibv_mr {
    log::info!("Registering memory region");

    let blue_context = unsafe { pd.as_ref() }.unwrap().context;
    let rxe_context = unsafe { to_rxe_context(blue_context) };

    let pd_mut = unsafe { pd.as_mut() }.unwrap();

    pd_mut.context = rxe_context;
    let mr = unsafe { ffi::ibv_reg_mr(pd, addr, length, access) };
    pd_mut.context = blue_context;

    let mr_mut = unsafe { mr.as_mut() }.unwrap();
    mr_mut.context = blue_context;

    mr
}

#[unsafe(export_name = "bluerdma_dereg_mr")]
pub unsafe extern "C" fn dereg_mr(mr: *mut ffi::ibv_mr) -> ::std::os::raw::c_int {
    log::info!("Deregistering memory region");

    let blue_context = unsafe { mr.as_ref() }.unwrap().context;
    let rxe_context = unsafe { to_rxe_context(blue_context) };

    let mr_mut = unsafe { mr.as_mut() }.unwrap();

    mr_mut.context = rxe_context;
    let rc = unsafe { ffi::ibv_dereg_mr(mr) };

    rc
}

#[unsafe(export_name = "bluerdma_post_send")]
pub unsafe extern "C" fn post_send(
    qp: *mut ffi::ibv_qp,
    wr: *mut ffi::ibv_send_wr,
    bad_wr: *mut *mut ffi::ibv_send_wr,
) -> ::std::os::raw::c_int {
    log::info!("Posting send work request");

    let blue_context = unsafe { qp.as_ref() }.unwrap().context;
    let rxe_context = unsafe { to_rxe_context(blue_context) };
    let ctx = unsafe { rxe_context.as_ref() }.unwrap();

    let qp_mut = unsafe { qp.as_mut() }.unwrap();

    qp_mut.context = rxe_context;
    let rc = unsafe { ctx.ops.post_send.unwrap()(qp, wr, bad_wr) };
    qp_mut.context = blue_context;

    rc
}

#[unsafe(export_name = "bluerdma_post_recv")]
pub unsafe extern "C" fn post_recv(
    qp: *mut ffi::ibv_qp,
    wr: *mut ffi::ibv_recv_wr,
    bad_wr: *mut *mut ffi::ibv_recv_wr,
) -> ::std::os::raw::c_int {
    log::info!("Posting receive work request");

    let blue_context = unsafe { qp.as_ref() }.unwrap().context;
    let rxe_context = unsafe { to_rxe_context(blue_context) };
    let ctx = unsafe { rxe_context.as_ref() }.unwrap();

    let qp_mut = unsafe { qp.as_mut() }.unwrap();

    qp_mut.context = rxe_context;
    let rc = unsafe { ctx.ops.post_recv.unwrap()(qp, wr, bad_wr) };
    qp_mut.context = blue_context;

    rc
}

#[unsafe(export_name = "bluerdma_poll_cq")]
pub unsafe extern "C" fn poll_cq(cq: *mut ffi::ibv_cq, num_entries: i32, wc: *mut ffi::ibv_wc) -> i32 {
    log::info!("Polling completion queue");

    let blue_context = unsafe { cq.as_ref() }.unwrap().context;
    let rxe_context = unsafe { to_rxe_context(blue_context) };
    let ctx = unsafe { rxe_context.as_ref() }.unwrap();

    let cq_mut = unsafe { cq.as_mut() }.unwrap();

    cq_mut.context = rxe_context;
    let rc = unsafe { ctx.ops.poll_cq.unwrap()(cq, num_entries, wc) };
    cq_mut.context = blue_context;

    rc
}
