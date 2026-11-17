//! Handles: the names of what the long-lived side holds.
//!
//! A handle is an integer its holder allocates counting up and never gives
//! out twice. A process restored to an older state then holds handles that
//! name the same resource or nothing, never another one. A handle means
//! something only within the epoch of the allocator that made it
//! ([`crate::hello::Hello::epoch`]).

/// A name for one held resource. Never zero.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Handle(std::num::NonZeroU64);

impl Handle {
    /// The handle a peer wrote as RAW; zero names nothing.
    #[must_use]
    pub fn from_raw(raw: u64) -> Option<Self> { std::num::NonZeroU64::new(raw).map(Self) }
    #[must_use]
    pub fn raw(self) -> u64 { self.0.get() }
}

impl std::fmt::Display for Handle {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result { self.0.fmt(f) }
}

/// Gives each handle out once.
#[derive(Default)]
pub struct HandleAllocator { last: u64 }

impl HandleAllocator {
    /// The next handle; `None` once all 2^64-1 are spent, never a reused one.
    pub fn allocate(&mut self) -> Option<Handle> {
        let next = self.last.checked_add(1)?;
        self.last = next;
        Handle::from_raw(next)
    }

    /// An allocator that goes on after ALLOCATED handles were given out:
    /// for a holder that carries its table into a new process image.
    #[must_use]
    pub fn resume(allocated: u64) -> Self { Self { last: allocated } }

    /// How many handles were ever given out.
    #[must_use]
    pub fn allocated(&self) -> u64 { self.last }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn handles_count_up_from_one() {
        let mut allocator = HandleAllocator::default();
        let handles: Vec<u64> = (0..3).map(|_| allocator.allocate().unwrap().raw()).collect();
        assert_eq!(handles, [1, 2, 3]);
        assert_eq!(allocator.allocated(), 3);
        assert_eq!(Handle::from_raw(0), None);
    }

    #[test]
    fn a_resumed_allocator_goes_on_where_it_stopped() {
        let mut allocator = HandleAllocator::default();
        let first = allocator.allocate().unwrap();
        let mut resumed = HandleAllocator::resume(allocator.allocated());
        assert_eq!(resumed.allocate().map(Handle::raw), Some(first.raw() + 1));
    }

    #[test]
    fn a_spent_allocator_refuses_rather_than_wraps() {
        let mut allocator = HandleAllocator { last: u64::MAX - 1 };
        assert_eq!(allocator.allocate().map(Handle::raw), Some(u64::MAX));
        assert_eq!(allocator.allocate(), None);
        assert_eq!(allocator.allocate(), None);
    }
}
