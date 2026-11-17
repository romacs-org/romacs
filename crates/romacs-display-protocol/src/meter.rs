//! Work metered in instructions on the thread that does it: what a named
//! stretch of a thread's work retired, added to that thread's counts
//! (`call_count`). The counter is the host's (romacs-vm
//! `host::instruction_count`, a pinned hardware counter each thread opens
//! at its first read): a session that wants the figures installs it, and
//! without one a metered stretch runs as it is and reads nothing.
use std::sync::OnceLock;

/// The calling thread's instructions so far; None where it has no counter.
pub type Counter = fn() -> Option<u64>;

static COUNTER: OnceLock<Counter> = OnceLock::new();

/// Meter with COUNTER from here on. The first one installed stays.
pub fn install(counter: Counter) {
    let _ = COUNTER.set(counter);
}

/// Whether a counter is installed: whether anything is metered.
#[must_use]
pub fn installed() -> bool {
    COUNTER.get().is_some()
}

/// Run WORK and, with a counter installed, add the instructions this
/// thread retired in it to its count NAME.
pub fn spent<T>(name: &'static str, work: impl FnOnce() -> T) -> T {
    metered(COUNTER.get().copied(), name, work)
}

/// `spent` by COUNTER. A stretch the counter could not read at both ends
/// adds nothing: a count is always whole.
fn metered<T>(counter: Option<Counter>, name: &'static str, work: impl FnOnce() -> T) -> T {
    let Some(counter) = counter else { return work() };
    let before = counter();
    let value = work();
    if let (Some(before), Some(after)) = (before, counter()) {
        crate::call_count::add(name, after.saturating_sub(before));
    }
    value
}

#[cfg(test)]
mod tests {
    use super::metered;
    use crate::call_count;
    use std::cell::Cell;

    thread_local! {
        static READS: Cell<u64> = const { Cell::new(0) };
    }

    /// A counter that retires 100 instructions between two reads.
    #[expect(clippy::unnecessary_wraps, reason = "a Counter's signature")]
    fn stepping() -> Option<u64> {
        READS.set(READS.get() + 1);
        Some(READS.get() * 100)
    }

    #[test]
    fn a_metered_stretch_adds_what_the_counter_retired_in_it() {
        assert_eq!(metered(Some(stepping), "meter-test-stretch", || 7), 7);
        assert_eq!(metered(Some(stepping), "meter-test-stretch", || 8), 8);
        assert_eq!(call_count::of("meter-test-stretch"), 200);
    }

    #[test]
    fn without_a_counter_the_work_runs_and_nothing_is_counted() {
        assert_eq!(metered(None, "meter-test-none", || 7), 7);
        assert_eq!(metered(Some(|| None), "meter-test-none", || 8), 8, "a thread without a counter");
        assert_eq!(call_count::of("meter-test-none"), 0);
    }
}
