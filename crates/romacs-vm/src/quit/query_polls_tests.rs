//! Observe native stop probes without timing a thread or signal.
use std::cell::Cell;

#[derive(Clone, Copy)]
struct Polls {
    count: usize,
    stops: Option<(usize, usize)>,
}

thread_local! {
    static POLLS: Cell<Option<Polls>> = const { Cell::new(None) };
}

pub(crate) fn count<T>(stop_at: Option<usize>, walk: impl FnOnce() -> T) -> (T, usize) {
    count_with(stop_at.map(|first| (first, usize::MAX)), walk)
}

pub(crate) fn count_once<T>(at: usize, walk: impl FnOnce() -> T) -> (T, usize) {
    count_with(Some((at, at)), walk)
}

fn count_with<T>(stops: Option<(usize, usize)>, walk: impl FnOnce() -> T) -> (T, usize) {
    POLLS.with(|polls| {
        assert!(polls.get().is_none(), "nested query poll measurement");
        polls.set(Some(Polls { count: 0, stops }));
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
        Some(current.stops.is_some_and(|(first, last)| (first..=last).contains(&current.count)))
    })
}
