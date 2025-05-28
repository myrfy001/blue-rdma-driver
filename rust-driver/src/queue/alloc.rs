use std::{
    io,
    ops::{Deref, DerefMut},
};

use crate::{
    descriptors::{RingBufDescUntyped, DESC_SIZE},
    mem::{
        page::{ContiguousPages, HostPageAllocator, MmapMut, PageAllocator},
        DmaBuf, DmaBufAllocator,
    },
    ringbuf::{DmaRingBuf, RING_BUF_LEN},
};

/// Ring buffer storing RDMA descriptors
pub(crate) struct DescRingBuffer(DmaRingBuf<RingBufDescUntyped>);

impl DescRingBuffer {
    pub(crate) fn new(buf: MmapMut) -> Self {
        let rb = DmaRingBuf::new(buf);
        Self(rb)
    }

    pub(crate) fn remaining(&self) -> usize {
        self.0.remaining()
    }

    pub(crate) fn pop(&mut self) -> Option<RingBufDescUntyped> {
        self.0.pop(RingBufDescUntyped::is_valid)
    }

    pub(crate) fn pop_two(&mut self) -> (Option<RingBufDescUntyped>, Option<RingBufDescUntyped>) {
        self.0
            .pop_two(RingBufDescUntyped::is_valid, RingBufDescUntyped::has_next)
    }
}

impl Deref for DescRingBuffer {
    type Target = DmaRingBuf<RingBufDescUntyped>;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl DerefMut for DescRingBuffer {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.0
    }
}

pub(crate) struct DescRingBufAllocator<A> {
    dma_buf_allocator: A,
}

impl<A: DmaBufAllocator> DescRingBufAllocator<A> {
    pub(crate) fn new(dma_buf_allocator: A) -> Self {
        Self { dma_buf_allocator }
    }

    pub(crate) fn alloc(&mut self) -> io::Result<DmaBuf> {
        self.dma_buf_allocator.alloc(RING_BUF_LEN * DESC_SIZE)
    }
}
