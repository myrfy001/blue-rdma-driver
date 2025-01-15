use core::ptr::NonNull;

pub struct Rxe {
    ibv_context: NonNull<ffi::ibv_context>,
}

impl Rxe {
    pub fn new(ibv_context: NonNull<ffi::ibv_context>) -> Rxe {
        Rxe { ibv_context }
    }

    fn as_ibv_context(&self) -> *mut ffi::ibv_context {
        self.ibv_context.as_ptr()
    }
}

impl Drop for Rxe {
    fn drop(&mut self) {
        // #Safety: `ibv_close_device` is called with a valid `ibv_context` pointer.
        unsafe {
            ffi::ibv_close_device(self.as_ibv_context());
        }
    }
}

#[repr(C)]
struct BlueRdmaDevice {
    pad: [u8; 712],
    rxe: *mut core::ffi::c_void,
    abi_version: core::ffi::c_int,
}

pub unsafe fn to_rxe_context(context: *mut ffi::ibv_context) -> *mut ffi::ibv_context {
    let dev_ptr = unsafe { *context }.device.cast::<BlueRdmaDevice>();
    let rex_ptr = unsafe { dev_ptr.as_ref() }.unwrap().rxe.cast::<Rxe>();
    unsafe { rex_ptr.as_ref().unwrap() }.as_ibv_context()
}
