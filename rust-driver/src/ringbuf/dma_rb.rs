use std::sync::atomic::{fence, Ordering};

use crate::mem::page::MmapMut;

/// Number of bits used to represent the length of the ring buffer.
const RING_BUF_LEN_BITS: u8 = 12;
/// Highest bit of the ring buffer
pub(crate) const RING_BUF_LEN: usize = 1 << RING_BUF_LEN_BITS;
/// Mask used to calculate the length of the ring buffer
const RING_BUF_LEN_MASK: usize = (1 << RING_BUF_LEN_BITS) - 1;
/// Mask used to wrap indices around the ring buffer length.
/// Allows the highest bit to overflow for convenient wraparound.
const RING_BUF_LEN_WRAP_MASK: usize = (1 << (RING_BUF_LEN_BITS + 1)) - 1;

pub(crate) struct DmaRingBuf<T> {
    ptr: *mut T,
    head: usize,
    tail: usize,

    _mmap: MmapMut,
}

#[allow(unsafe_code)]
impl<T: Copy> DmaRingBuf<T> {
    pub(crate) fn new(mmap: MmapMut) -> Self {
        assert!(mmap.len >= RING_BUF_LEN * size_of::<T>(), "invalid length");
        Self {
            ptr: mmap.ptr.cast(),
            head: 0,
            tail: 0,
            _mmap: mmap,
        }
    }

    pub(crate) fn push(&mut self, value: T) -> bool {
        if self.len() == RING_BUF_LEN {
            return false;
        }
        unsafe {
            self.ptr.add(self.head_idx()).write_volatile(value);
        }

        self.inc_head();

        true
    }

    pub(crate) fn pop<F>(&mut self, cond: F) -> Option<T>
    where
        F: FnOnce(&T) -> bool,
    {
        let value = self.read_index(self.tail_idx());
        if cond(&value) {
            // Ensures that the value is read atomically from memory
            fence(Ordering::Acquire);
            let value = self.read_and_advance(self.tail_idx());
            return Some(value);
        }

        None
    }

    pub(crate) fn pop_two<F, R>(
        &mut self,
        mut cond: F,
        mut require_next: R,
    ) -> (Option<T>, Option<T>)
    where
        F: FnMut(&T) -> bool,
        R: FnMut(&T) -> bool,
    {
        let idx_first = self.tail_idx();
        let idx_next = idx_first.wrapping_add(1) & RING_BUF_LEN_MASK;
        let value_first = self.read_index(idx_first);
        let value_next = self.read_index(idx_next);

        match (
            cond(&value_first),
            cond(&value_next),
            require_next(&value_first),
        ) {
            (true, true, true) => {
                fence(Ordering::Acquire);
                let value_first = self.read_and_advance(idx_first);
                let value_next = self.read_and_advance(idx_next);
                (Some(value_first), Some(value_next))
            }
            (true, _, false) => {
                fence(Ordering::Acquire);
                let value_first = self.read_and_advance(idx_first);
                (Some(value_first), None)
            }
            (true, false, true) | (false, _, _) => (None, None),
        }
    }

    fn read_index(&self, index: usize) -> T {
        unsafe { self.ptr.add(index).read_volatile() }
    }

    fn zero_index(&mut self, index: usize) {
        unsafe {
            self.ptr.add(index).write_volatile(std::mem::zeroed());
        }
    }

    fn read_and_advance(&mut self, index: usize) -> T {
        let value = self.read_index(index);
        self.zero_index(index);
        self.inc_tail();
        value
    }

    /// Returns the current head index in the ring buffer
    pub(crate) fn head(&self) -> usize {
        self.head
    }

    /// Returns the current tail index in the ring buffer
    pub(crate) fn tail(&self) -> usize {
        self.tail
    }

    /// Returns the current length of data in the ring buffer
    pub(crate) fn len(&self) -> usize {
        let dlt = self.head.wrapping_sub(self.tail);
        dlt & RING_BUF_LEN_WRAP_MASK
    }

    pub(crate) fn remaining(&self) -> usize {
        RING_BUF_LEN - self.len()
    }

    /// Returns true if the ring buffer is empty
    pub(crate) fn is_empty(&self) -> bool {
        self.head == self.tail
    }

    /// Returns true if the ring buffer is full
    pub(crate) fn is_full(&self) -> bool {
        self.head ^ self.tail == RING_BUF_LEN
    }

    /// Increments the head pointer of the ring buffer
    pub(crate) fn inc_head(&mut self) {
        self.head = self.head.wrapping_add(1) & RING_BUF_LEN_WRAP_MASK;
    }

    /// Increments the tail pointer of the ring buffer
    pub(crate) fn inc_tail(&mut self) {
        self.tail = self.tail.wrapping_add(1) & RING_BUF_LEN_WRAP_MASK;
    }

    pub(crate) fn set_tail(&mut self, tail: u32) {
        self.tail = tail as usize;
    }

    pub(crate) fn set_head(&mut self, head: u32) {
        self.head = head as usize;
    }

    fn head_idx(&self) -> usize {
        self.head & RING_BUF_LEN_MASK
    }

    fn tail_idx(&self) -> usize {
        self.tail & RING_BUF_LEN_MASK
    }
}

#[allow(unsafe_code)]
unsafe impl<T> Send for DmaRingBuf<T> {}

#[cfg(test)]
mod tests {
    use super::*;

    #[allow(unsafe_code)]
    fn create_test_mmap() -> MmapMut {
        let len = 0x10000;
        let ptr = unsafe {
            libc::mmap(
                std::ptr::null_mut(),
                len,
                libc::PROT_READ | libc::PROT_WRITE,
                libc::MAP_SHARED | libc::MAP_ANON,
                -1,
                0,
            )
        };

        MmapMut { ptr, len }
    }

    #[test]
    fn test_dma_ring_buf_push_pop() {
        let mmap = create_test_mmap();
        let mut rb = DmaRingBuf::<u32>::new(mmap);

        assert!(rb.push(42));
        assert_eq!(rb.len(), 1);
        assert_eq!(rb.remaining(), RING_BUF_LEN - 1);
        assert!(!rb.is_empty());
        assert!(!rb.is_full());

        let popped = rb.pop(|_| true);
        assert_eq!(popped, Some(42));
        assert_eq!(rb.len(), 0);
        assert_eq!(rb.remaining(), RING_BUF_LEN);
        assert!(rb.is_empty());
        assert!(!rb.is_full());
    }

    #[test]
    fn test_dma_ring_buf_conditional_pop() {
        let mmap = create_test_mmap();
        let mut rb = DmaRingBuf::<u32>::new(mmap);

        assert!(rb.push(42));

        let popped = rb.pop(|_| false);
        assert_eq!(popped, None);
        assert_eq!(rb.len(), 1);

        let popped = rb.pop(|&x| x == 42);
        assert_eq!(popped, Some(42));
        assert_eq!(rb.len(), 0);
    }

    #[test]
    fn test_dma_ring_buf_fill_and_overflow() {
        let mmap = create_test_mmap();
        let mut rb = DmaRingBuf::<u32>::new(mmap);

        for i in 0..RING_BUF_LEN {
            assert!(rb.push(i as u32));
        }

        assert_eq!(rb.len(), RING_BUF_LEN);
        assert_eq!(rb.remaining(), 0);
        assert!(!rb.is_empty());
        assert!(rb.is_full());

        assert!(!rb.push(9999));

        for i in 0..RING_BUF_LEN {
            let popped = rb.pop(|_| true);
            assert_eq!(popped, Some(i as u32));
        }

        assert!(rb.is_empty());
        assert!(!rb.is_full());
    }

    #[test]
    fn test_dma_ring_buf_wraparound() {
        let mmap = create_test_mmap();
        let mut rb = DmaRingBuf::<u32>::new(mmap);

        for i in 0..RING_BUF_LEN / 2 {
            assert!(rb.push(i as u32));
        }

        for i in 0..RING_BUF_LEN / 2 {
            let popped = rb.pop(|_| true);
            assert_eq!(popped, Some(i as u32));
        }

        for i in 0..RING_BUF_LEN {
            assert!(rb.push((i + 1000) as u32));
        }

        assert!(rb.is_full());

        for i in 0..RING_BUF_LEN {
            let popped = rb.pop(|_| true);
            assert_eq!(popped, Some((i + 1000) as u32));
        }

        assert!(rb.is_empty());
    }

    #[test]
    fn test_dma_ring_buf_pop_two() {
        let mmap = create_test_mmap();
        let mut rb = DmaRingBuf::<u32>::new(mmap);

        assert!(rb.push(1));
        assert!(rb.push(2));

        let (first, second) = rb.pop_two(|_| true, |&x| x == 1);
        assert_eq!(first, Some(1));
        assert_eq!(second, Some(2));
        assert!(rb.is_empty());
    }

    #[test]
    fn test_dma_ring_buf_pop_two_no_next_required() {
        let mmap = create_test_mmap();
        let mut rb = DmaRingBuf::<u32>::new(mmap);

        assert!(rb.push(1));
        assert!(rb.push(2));

        let (first, second) = rb.pop_two(|_| true, |&x| x != 1);
        assert_eq!(first, Some(1));
        assert_eq!(second, None);
        assert_eq!(rb.len(), 1);
    }

    #[test]
    fn test_dma_ring_buf_pop_two_first_invalid() {
        let mmap = create_test_mmap();
        let mut rb = DmaRingBuf::<u32>::new(mmap);

        assert!(rb.push(1));
        assert!(rb.push(2));

        let (first, second) = rb.pop_two(|&x| x != 1, |_| true);
        assert_eq!(first, None);
        assert_eq!(second, None);
        assert_eq!(rb.len(), 2);
    }

    #[test]
    fn test_dma_ring_buf_pop_two_second_invalid() {
        let mmap = create_test_mmap();
        let mut rb = DmaRingBuf::<u32>::new(mmap);

        assert!(rb.push(1));
        assert!(rb.push(2));

        let (first, second) = rb.pop_two(|&x| x == 1 || x == 3, |&x| x == 1);
        assert_eq!(first, None);
        assert_eq!(second, None);
        assert_eq!(rb.len(), 2);
    }

    #[test]
    fn test_dma_ring_buf_set_head_tail() {
        let mmap = create_test_mmap();
        let mut rb = DmaRingBuf::<u32>::new(mmap);

        rb.set_head(100);
        rb.set_tail(50);

        assert_eq!(rb.head(), 100);
        assert_eq!(rb.tail(), 50);
    }

    #[test]
    fn test_dma_ring_buf_indices() {
        let mmap = create_test_mmap();
        let mut rb = DmaRingBuf::<u32>::new(mmap);

        rb.set_head(RING_BUF_LEN as u32 + 5);
        rb.set_tail(RING_BUF_LEN as u32 + 3);

        assert_eq!(rb.head_idx(), 5);
        assert_eq!(rb.tail_idx(), 3);
    }

    #[test]
    fn test_dma_ring_buf_len_calculation() {
        let mmap = create_test_mmap();
        let mut rb = DmaRingBuf::<u32>::new(mmap);

        rb.set_head(10);
        rb.set_tail(5);
        assert_eq!(rb.len(), 5);

        rb.set_head(RING_BUF_LEN as u32 + 10);
        rb.set_tail(5);
        let expected_len = (RING_BUF_LEN + 10 - 5) & RING_BUF_LEN_WRAP_MASK;
        assert_eq!(rb.len(), expected_len);
    }

    #[test]
    fn test_dma_ring_buf_inc_operations() {
        let mmap = create_test_mmap();
        let mut rb = DmaRingBuf::<u32>::new(mmap);

        let initial_head = rb.head();
        rb.inc_head();
        assert_eq!(rb.head(), (initial_head + 1) & RING_BUF_LEN_WRAP_MASK);

        let initial_tail = rb.tail();
        rb.inc_tail();
        assert_eq!(rb.tail(), (initial_tail + 1) & RING_BUF_LEN_WRAP_MASK);

        rb.set_head(RING_BUF_LEN_WRAP_MASK as u32);
        rb.inc_head();
        assert_eq!(rb.head(), 0);
    }
}
