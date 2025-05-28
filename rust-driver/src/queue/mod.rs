/// Command queue implementation
pub(crate) mod cmd_queue;

/// Simple NIC tx queue implementation
pub(crate) mod simple_nic;

/// Queue allocator implementation
pub(crate) mod alloc;

pub(crate) use alloc::DescRingBuffer;
