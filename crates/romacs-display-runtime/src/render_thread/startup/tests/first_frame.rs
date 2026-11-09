//! Drive the real startup handler up to a refusing native constructor, never a GPU.
#[path = "staging.rs"]
mod staging;
use std::cell::Cell;

#[test]
fn missing_constructor_lease_never_runs_native_work() {
    let waits = crate::native_window_wait::NativeWindowWaits::default();
    let constructors = Cell::new(0);
    assert!(
        waits
            .run(0, || {
                constructors.set(constructors.get() + 1);
                Ok(())
            })
            .is_err()
    );
    assert_eq!(constructors.get(), 0);
    assert!(!waits.terminal());
}
