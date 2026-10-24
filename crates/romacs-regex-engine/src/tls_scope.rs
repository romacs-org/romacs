//! Copied from romacs-core's `tls_scope` (the `TlsScope` half, minus the
//! entry points the engine does not use): a guard that installs a thread-local value and restores the previous one on
//! drop, including on unwind.

//! Thread-confined restoration of temporary thread-local state.
//!
//! Restoration during teardown is best effort: a destroyed TLS key or an
//! outstanding `RefCell` borrow must not cause a second panic while unwinding.
//! The payloads used here have non-panicking destructors. This module does not
//! invoke user callbacks during restoration.

use std::cell::{Cell, RefCell};
use std::marker::PhantomData;
use std::rc::Rc;
use std::thread::LocalKey;

mod sealed {
    pub trait Sealed {}
    impl<T> Sealed for std::cell::Cell<T> {}
    impl<T> Sealed for std::cell::RefCell<T> {}
}

/// The two supported TLS storage kinds; callers cannot supply restoration code.
pub(crate) trait TlsSlot<T>: sealed::Sealed {
    #[cfg(any(test, feature = "test-hooks", feature = "fuzzing"))]
    fn replace(&self, value: T) -> T;
    fn try_replace(&self, value: T) -> Result<T, T>;
}

impl<T> TlsSlot<T> for Cell<T> {
    #[cfg(any(test, feature = "test-hooks", feature = "fuzzing"))]
    #[inline]
    fn replace(&self, value: T) -> T {
        Cell::replace(self, value)
    }

    #[inline]
    fn try_replace(&self, value: T) -> Result<T, T> {
        Ok(Cell::replace(self, value))
    }
}

impl<T> TlsSlot<T> for RefCell<T> {
    #[cfg(any(test, feature = "test-hooks", feature = "fuzzing"))]
    #[inline]
    fn replace(&self, value: T) -> T {
        std::mem::replace(&mut *self.borrow_mut(), value)
    }

    #[inline]
    fn try_replace(&self, value: T) -> Result<T, T> {
        match self.try_borrow_mut() {
            Ok(mut slot) => Ok(std::mem::replace(&mut *slot, value)),
            Err(_) => Err(value),
        }
    }
}

/// Owns restoration on the thread that installed a temporary value.
#[must_use = "thread-local state is restored when the scope drops"]
pub(crate) struct TlsScope<T: 'static, Slot: TlsSlot<T> + 'static> {
    key: &'static LocalKey<Slot>,
    previous: Option<T>,
    _thread: PhantomData<Rc<()>>,
}

static_assertions::assert_not_impl_any!(TlsScope<u32, Cell<u32>>: Send, Sync);
static_assertions::assert_not_impl_any!(TlsScope<Vec<u32>, RefCell<Vec<u32>>>: Send, Sync);

impl<T: 'static, Slot: TlsSlot<T> + 'static> std::fmt::Debug for TlsScope<T, Slot> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TlsScope")
            .field("active", &self.previous.is_some())
            .finish_non_exhaustive()
    }
}

impl<T: 'static, Slot: TlsSlot<T> + 'static> TlsScope<T, Slot> {
    /// Install `value`, restoring the previous value when the scope drops.
    /// Only the engine's test and fuzzing seams scope a value this way.
    #[cfg(any(test, feature = "test-hooks", feature = "fuzzing"))]
    #[inline]
    pub(crate) fn new(key: &'static LocalKey<Slot>, value: T) -> Self {
        Self {
            key,
            previous: Some(key.with(|slot| slot.replace(value))),
            _thread: PhantomData,
        }
    }

    /// Return an owned value without constructing a temporary scope.
    /// An unavailable key or borrowed slot drops the returned value instead.
    #[inline]
    pub(crate) fn restore_now(key: &'static LocalKey<Slot>, previous: T) {
        let _ = key.try_with(|slot| slot.try_replace(previous));
    }
}

impl<T: 'static, Slot: TlsSlot<T> + 'static> Drop for TlsScope<T, Slot> {
    #[inline]
    fn drop(&mut self) {
        if let Some(previous) = self.previous.take() {
            Self::restore_now(self.key, previous);
        }
    }
}
