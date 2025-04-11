#![allow(unsafe_code)]
#![allow(unsafe_op_in_unsafe_fn)]
#![allow(missing_docs)]
#![allow(clippy::all)]
#![allow(clippy::pedantic)]
#![allow(clippy::panic)]
#![allow(clippy::indexing_slicing)]

use std::{
    alloc::{GlobalAlloc, Layout},
    ffi::c_void,
};

use buddy_system_allocator::LockedHeap;

pub(crate) use ctor;

const ORDER: usize = 32;
const SHM_PATHS: [&str; 2] = ["/bluesim1\0", "/bluesim2\0"];
const SHM_BLOCK_SIZE: usize = 1024 * 1024 * 256;
//pub(crate) static mut SHM_START_ADDR: usize = 0;
const SHM_START_ADDR: usize = 0x7f7e_8e60_0000;

/// Memory Layout:
/// ```text
/// +----------------------+ SHM_START_ADDR + 256MB
/// |      Heap Space      | (192MB - 256MB)
/// +----------------------+ SHM_START_ADDR + 192MB
/// |      Page Space      | (128MB - 192MB)
/// +----------------------+ SHM_START_ADDR + 128MB
/// |      Reserved        | (0MB - 128MB)
/// +----------------------+ SHM_START_ADDR
/// ```
/// Offset of the address space used by the allocator
///
/// Range SHM_START_ADDR..SHM_START_ADDR + HEAP_START_ADDR_OFFSET is reserved for mmap allocation
const HEAP_START_ADDR_OFFSET: usize = 1024 * 1024 * 192;
/// Offset of the address space used by the page allocator
const PAGE_START_ADDR_OFFSET: usize = 1024 * 1024 * 128;

/// Handle to the allocator
///
/// This type implements the `GlobalAlloc` trait, allowing usage a global allocator.
pub(crate) struct Simalloc(LockedHeap<ORDER>);

impl Simalloc {
    pub(crate) const fn new() -> Self {
        Self(LockedHeap::new())
    }
}

impl Default for Simalloc {
    fn default() -> Self {
        Self::new()
    }
}

unsafe impl GlobalAlloc for Simalloc {
    #[inline]
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        self.0.alloc(layout)
    }

    #[inline]
    unsafe fn alloc_zeroed(&self, layout: Layout) -> *mut u8 {
        self.0.alloc_zeroed(layout)
    }

    #[inline]
    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        self.0.dealloc(ptr, layout)
    }

    #[inline]
    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        self.0.realloc(ptr, layout, new_size)
    }
}

#[macro_export]
macro_rules! setup_allocator {
    ($index:expr) => {
        use $crate::ctor;

        #[global_allocator]
        static HEAP_ALLOCATOR: $crate::Simalloc = $crate::Simalloc::new();

        #[ctor::ctor]
        fn init_global_allocator() {
            $crate::init_global_allocator($index, &HEAP_ALLOCATOR);
        }
    };
}

pub(crate) fn shm_start_addr() -> usize {
    SHM_START_ADDR
}

pub(crate) fn page_start_addr() -> usize {
    SHM_START_ADDR + PAGE_START_ADDR_OFFSET
}

pub(crate) fn heap_start_addr() -> usize {
    SHM_START_ADDR + HEAP_START_ADDR_OFFSET
}

pub(crate) fn init_global_allocator(index: usize, allocator: &Simalloc) {
    unsafe {
        let shm_fd = libc::shm_open(
            SHM_PATHS[index].as_ptr() as *const libc::c_char,
            libc::O_RDWR,
            0o600,
        );
        if shm_fd == -1 {
            panic!("failed to open shared memory");
        }
        assert!(shm_fd != -1, "shm_open failed");

        let shm = libc::mmap(
            0x7f7e_8e60_0000 as *mut c_void,
            SHM_BLOCK_SIZE,
            libc::PROT_READ | libc::PROT_WRITE,
            libc::MAP_SHARED,
            shm_fd,
            0,
        );

        if shm.is_null() {
            panic!("failed to open shared memory");
        }

        //SHM_START_ADDR = shm as usize;
        let heap_size = SHM_BLOCK_SIZE - HEAP_START_ADDR_OFFSET;

        allocator.0.lock().init(heap_start_addr(), heap_size);
    }
}
