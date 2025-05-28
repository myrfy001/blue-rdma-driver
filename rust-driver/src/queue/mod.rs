/// Command queue implementation
pub(crate) mod cmd_queue;

/// Queue allocator implementation
pub(crate) mod alloc;

pub(crate) use alloc::DescRingBuffer;
