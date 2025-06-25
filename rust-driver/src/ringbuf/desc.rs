use std::{
    io,
    ops::{Deref, DerefMut},
};

use crate::{
    descriptors::DESC_SIZE,
    mem::{
        page::{ContiguousPages, HostPageAllocator, MmapMut, PageAllocator},
        DmaBuf, DmaBufAllocator,
    },
    ringbuf::dma_rb::{DmaRingBuf, RING_BUF_LEN},
};

pub(crate) trait DescSerialize {
    fn serialize(&self) -> [u8; 32];
}

pub(crate) trait DescDeserialize {
    fn deserialize(d: [u8; 32]) -> Self;
}

pub(crate) struct DescRingBuffer(DmaRingBuf<[u8; 32]>);

impl DescRingBuffer {
    pub(crate) fn new(buf: MmapMut) -> Self {
        let rb = DmaRingBuf::new(buf);
        Self(rb)
    }

    pub(crate) fn push<T: DescSerialize>(&mut self, value: &T) -> bool {
        self.0.push(value.serialize())
    }

    pub(crate) fn pop<T: DescDeserialize>(&mut self) -> Option<T> {
        self.0.pop(Self::is_valid).map(DescDeserialize::deserialize)
    }

    pub(crate) fn pop_two<A: DescDeserialize, B: DescDeserialize>(
        &mut self,
    ) -> (Option<A>, Option<B>) {
        let (a, b) = self.0.pop_two(Self::is_valid, Self::has_next);
        (
            a.map(DescDeserialize::deserialize),
            b.map(DescDeserialize::deserialize),
        )
    }

    pub(crate) fn remaining(&self) -> usize {
        self.0.remaining()
    }

    pub(crate) fn set_tail(&mut self, tail: u32) {
        self.0.set_tail(tail);
    }

    pub(crate) fn set_head(&mut self, head: u32) {
        self.0.set_head(head);
    }

    /// Returns the current head index in the ring buffer
    pub(crate) fn head(&self) -> usize {
        self.0.head()
    }

    /// Returns the current tail index in the ring buffer
    pub(crate) fn tail(&self) -> usize {
        self.0.tail()
    }

    fn is_valid(desc: &[u8; 32]) -> bool {
        // highest bit is the valid bit
        desc[31] >> 7 == 1
    }

    fn has_next(desc: &[u8; 32]) -> bool {
        (desc[31] >> 6) & 1 == 1
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
