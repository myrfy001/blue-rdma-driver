mod verbs;

/// Get list of IB devices currently available
///
/// @num_devices: optional. if non-NULL, set to the number of devices returned in the array.
///
/// Return a NULL-terminated array of IB devices.
/// The array can be released with ibv_free_device_list().
#[unsafe(export_name = "ibv_get_device_list")]
pub unsafe extern "C" fn ibv_get_device_list(num_devices: *mut ::std::os::raw::c_int) -> *mut *mut ffi::ibv_device {
    eprintln!("Rust here!");
    let ibv_get_device_list: unsafe fn(*mut ::std::os::raw::c_int) -> *mut *mut ffi::ibv_device =
        unsafe { core::mem::transmute(libc::dlsym(libc::RTLD_NEXT, c"ibv_get_device_list".as_ptr())) };
    let dev_list = unsafe { ibv_get_device_list(num_devices) };
    dev_list
}
