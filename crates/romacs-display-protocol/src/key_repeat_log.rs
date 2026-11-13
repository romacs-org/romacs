//! What the render thread saw of native key repeat, for the usage log
//! (lisp/romacs/romacs-usage-log.el, its `repeat` record): how many repeats
//! it let through to the evaluator, how long after their key's press each
//! came, and the longest a key was held, from the press it saw to the
//! release it saw. A key repeats on the client while its release has not
//! been seen, so a release seen late shows as repeats the user never held
//! the key for.
//!
//! The render thread counts into a [`KeyRepeatLog`] it shares with the
//! evaluator, which takes the counts since its last look ([`KeyRepeatLog::take`]).
//! Counts only: no row per key.
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

/// Buckets of the press-to-repeat histogram, the usage log's gap buckets:
/// under 1 ms, then one per doubling bound from 1 ms to 32768 ms, the last
/// for 32768 ms or more.
pub const BUCKETS: usize = 17;

/// The bucket of a delay of MS milliseconds: the number of the bounds
/// 1, 2, 4, .., 32768 it is at least.
#[must_use]
pub fn bucket(ms: u64) -> usize {
    let bits = (u64::BITS - ms.leading_zeros()) as usize;
    bits.min(BUCKETS - 1)
}

/// The counts since the last take.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct KeyRepeatCounts {
    /// Repeats let through to the evaluator.
    pub repeats: u64,
    /// Those repeats by the delay from their key's press, by [`bucket`].
    pub after_press: [u64; BUCKETS],
    /// The longest a key was held, press seen to release seen, in ms.
    pub longest_hold_ms: u64,
}

/// The render thread's counts, shared with the evaluator.
#[derive(Debug, Default)]
pub struct KeyRepeatLog {
    after_press: [AtomicU64; BUCKETS],
    longest_hold_ms: AtomicU64,
}

impl KeyRepeatLog {
    /// A repeat let through, AFTER its key's press.
    pub fn repeat(&self, after: Duration) {
        self.after_press[bucket(millis(after))].fetch_add(1, Ordering::Relaxed);
    }

    /// A key released after being held for HELD.
    pub fn released(&self, held: Duration) {
        self.longest_hold_ms.fetch_max(millis(held), Ordering::Relaxed);
    }

    /// The counts since the last take, which start again from none. The
    /// repeats are the histogram's sum, so a repeat counted between the
    /// buckets' resets is never split across two takes.
    #[must_use]
    pub fn take(&self) -> KeyRepeatCounts {
        let after_press: [u64; BUCKETS] = std::array::from_fn(|i| self.after_press[i].swap(0, Ordering::Relaxed));
        KeyRepeatCounts {
            repeats: after_press.iter().sum(),
            after_press,
            longest_hold_ms: self.longest_hold_ms.swap(0, Ordering::Relaxed),
        }
    }
}

fn millis(duration: Duration) -> u64 {
    u64::try_from(duration.as_millis()).unwrap_or(u64::MAX)
}

#[cfg(test)]
#[path = "tests/key_repeat_log_test.rs"]
mod tests;
