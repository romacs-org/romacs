//! Observe the real native query poll path without timing a thread or signal.
use std::cell::Cell;

#[derive(Clone, Copy)]
struct Polls {
    count: usize,
    stop_at: Option<usize>,
}

thread_local! {
    static POLLS: Cell<Option<Polls>> = const { Cell::new(None) };
}

pub(crate) fn count<T>(stop_at: Option<usize>, walk: impl FnOnce() -> T) -> (T, usize) {
    POLLS.with(|polls| {
        assert!(polls.get().is_none(), "nested query poll measurement");
        polls.set(Some(Polls { count: 0, stop_at }));
        let result = walk();
        let measured = polls.replace(None).expect("query poll measurement");
        (result, measured.count)
    })
}

pub(super) fn record() -> Option<bool> {
    POLLS.with(|polls| {
        let mut current = polls.get()?;
        current.count += 1;
        polls.set(Some(current));
        Some(current.stop_at.is_some_and(|limit| current.count >= limit))
    })
}
