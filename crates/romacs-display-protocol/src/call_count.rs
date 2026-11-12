//! Calls counted by name on the thread that makes them: what the evaluator
//! asks of its hosts (romacs-vm `host_calls`) and of the paint sender
//! (romacs-display-runtime), in the one place both can reach.
use std::cell::RefCell;
use std::collections::HashMap;

thread_local! {
    static COUNTS: RefCell<HashMap<&'static str, u64>> = RefCell::new(HashMap::new());
}

/// Count one call of NAME on this thread.
pub fn count(name: &'static str) {
    COUNTS.with(|counts| *counts.borrow_mut().entry(name).or_insert(0) += 1);
}

/// This thread's counts so far, by name.
#[must_use]
pub fn counts() -> Vec<(&'static str, u64)> {
    let mut counts: Vec<_> = COUNTS.with(|counts| counts.borrow().iter().map(|(&name, &n)| (name, n)).collect());
    counts.sort_unstable();
    counts
}

#[cfg(test)]
mod tests {
    #[test]
    fn a_thread_counts_its_own_calls() {
        super::count("call-count-test");
        std::thread::spawn(|| super::count("call-count-test")).join().unwrap();
        assert_eq!(super::counts().into_iter().find(|row| row.0 == "call-count-test"), Some(("call-count-test", 1)));
    }
}
