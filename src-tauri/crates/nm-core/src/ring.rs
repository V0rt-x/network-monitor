//! Fixed-capacity ring buffer.

use crate::Error;

/// A buffer that holds at most `capacity` items, overwriting the oldest when full.
///
/// The backing storage is allocated once at construction and never grows, so recording a
/// sample costs no allocation — the property that lets the core keep per-endpoint history
/// for hours inside a fixed memory budget.
#[derive(Debug, Clone)]
pub struct RingBuffer<T> {
    items: Vec<T>,
    capacity: usize,
    /// Index that will be overwritten next. Meaningful only once the buffer is full.
    next: usize,
}

impl<T> RingBuffer<T> {
    /// Creates an empty buffer with room for `capacity` items.
    ///
    /// # Errors
    ///
    /// Returns [`Error::ZeroCapacity`] if `capacity` is zero, which would silently
    /// discard every sample.
    pub fn new(capacity: usize) -> Result<Self, Error> {
        if capacity == 0 {
            return Err(Error::ZeroCapacity);
        }
        Ok(Self {
            items: Vec::with_capacity(capacity),
            capacity,
            next: 0,
        })
    }

    /// Appends an item, returning the one it evicted if the buffer was already full.
    pub fn push(&mut self, item: T) -> Option<T> {
        if self.items.len() < self.capacity {
            self.items.push(item);
            return None;
        }
        let evicted = std::mem::replace(&mut self.items[self.next], item);
        self.next = (self.next + 1) % self.capacity;
        Some(evicted)
    }

    /// Number of items currently held.
    #[must_use]
    pub fn len(&self) -> usize {
        self.items.len()
    }

    /// Whether the buffer holds nothing.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.items.is_empty()
    }

    /// Whether the next push will evict the oldest item.
    #[must_use]
    pub fn is_full(&self) -> bool {
        self.items.len() == self.capacity
    }

    /// Maximum number of items the buffer will hold.
    #[must_use]
    pub const fn capacity(&self) -> usize {
        self.capacity
    }

    /// Iterates from the oldest item to the newest.
    ///
    /// Double-ended, so `next_back()` yields the most recent item without a scan.
    #[must_use]
    pub fn iter(&self) -> impl DoubleEndedIterator<Item = &T> + '_ {
        // Before the buffer fills, `next` is still 0 while the oldest item sits at index
        // 0, so the split point has to be the length instead.
        let split = if self.is_full() {
            self.next
        } else {
            self.items.len()
        };
        self.items[split..].iter().chain(&self.items[..split])
    }

    /// Drops every item, keeping the allocation for reuse.
    pub fn clear(&mut self) {
        self.items.clear();
        self.next = 0;
    }
}

impl<'a, T> IntoIterator for &'a RingBuffer<T> {
    type Item = &'a T;
    type IntoIter = std::iter::Chain<std::slice::Iter<'a, T>, std::slice::Iter<'a, T>>;

    fn into_iter(self) -> Self::IntoIter {
        let split = if self.is_full() {
            self.next
        } else {
            self.items.len()
        };
        self.items[split..].iter().chain(&self.items[..split])
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn collect(buffer: &RingBuffer<u32>) -> Vec<u32> {
        buffer.iter().copied().collect()
    }

    #[test]
    fn rejects_a_capacity_that_could_hold_nothing() {
        assert_eq!(RingBuffer::<u32>::new(0).unwrap_err(), Error::ZeroCapacity);
    }

    #[test]
    fn reports_an_empty_buffer_honestly() {
        let buffer = RingBuffer::<u32>::new(4).unwrap();
        assert!(buffer.is_empty());
        assert!(!buffer.is_full());
        assert_eq!(buffer.len(), 0);
        assert_eq!(buffer.capacity(), 4);
        assert_eq!(collect(&buffer), Vec::<u32>::new());
    }

    #[test]
    fn keeps_insertion_order_while_filling() {
        let mut buffer = RingBuffer::new(4).unwrap();
        for value in 1..=3 {
            assert_eq!(buffer.push(value), None);
        }
        assert_eq!(collect(&buffer), vec![1, 2, 3]);
        assert!(!buffer.is_full());
    }

    #[test]
    fn evicts_the_oldest_once_full() {
        let mut buffer = RingBuffer::new(3).unwrap();
        for value in 1..=3 {
            assert_eq!(buffer.push(value), None);
        }
        assert!(buffer.is_full());

        assert_eq!(buffer.push(4), Some(1));
        assert_eq!(collect(&buffer), vec![2, 3, 4]);

        assert_eq!(buffer.push(5), Some(2));
        assert_eq!(collect(&buffer), vec![3, 4, 5]);
    }

    #[test]
    fn stays_at_capacity_no_matter_how_much_is_pushed() {
        let mut buffer = RingBuffer::new(8).unwrap();
        for value in 0..1_000 {
            buffer.push(value);
        }
        assert_eq!(buffer.len(), 8);
        assert_eq!(collect(&buffer), (992..1_000).collect::<Vec<_>>());
    }

    #[test]
    fn never_reallocates_its_backing_storage() {
        // The no-allocation-while-sampling promise: capacity is reserved once and the
        // buffer must never outgrow it, however many samples pass through.
        let mut buffer = RingBuffer::new(4).unwrap();
        let reserved = buffer.items.capacity();
        for value in 0..500 {
            buffer.push(value);
        }
        assert_eq!(buffer.items.capacity(), reserved);
    }

    #[test]
    fn wraps_correctly_with_a_capacity_of_one() {
        let mut buffer = RingBuffer::new(1).unwrap();
        assert_eq!(buffer.push(1), None);
        assert_eq!(buffer.push(2), Some(1));
        assert_eq!(buffer.push(3), Some(2));
        assert_eq!(collect(&buffer), vec![3]);
    }

    #[test]
    fn clearing_resets_the_wrap_point() {
        let mut buffer = RingBuffer::new(3).unwrap();
        for value in 1..=5 {
            buffer.push(value);
        }
        buffer.clear();
        assert!(buffer.is_empty());

        for value in 1..=2 {
            buffer.push(value);
        }
        assert_eq!(collect(&buffer), vec![1, 2]);
    }

    #[test]
    fn borrows_iterate_in_the_same_order() {
        let mut buffer = RingBuffer::new(3).unwrap();
        for value in 1..=5 {
            buffer.push(value);
        }
        let by_method: Vec<_> = buffer.iter().copied().collect();
        let by_for_loop: Vec<_> = (&buffer).into_iter().copied().collect();
        assert_eq!(by_method, by_for_loop);
        assert_eq!(by_method, vec![3, 4, 5]);
    }
}
