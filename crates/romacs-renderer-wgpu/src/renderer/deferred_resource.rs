//! First-use GPU resources. Construction is synchronous on the drawing owner:
//! a command cannot be submitted until the resource it uses is fully ready.

use std::ops::Deref;
use std::sync::LazyLock;

type Factory<T> = Box<dyn FnOnce() -> T + Send + Sync>;

pub(crate) struct DeferredResource<T>(LazyLock<T, Factory<T>>);

impl<T> DeferredResource<T> {
    pub(crate) fn new(make: impl FnOnce() -> T + Send + Sync + 'static) -> Self {
        Self(LazyLock::new(Box::new(make) as Factory<T>))
    }

    #[cfg(test)]
    pub(crate) fn is_ready(&self) -> bool {
        LazyLock::get(&self.0).is_some()
    }
}

impl<T> Deref for DeferredResource<T> {
    type Target = T;

    fn deref(&self) -> &T {
        LazyLock::force(&self.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    };

    #[test]
    fn construction_does_not_run_the_factory() {
        let calls = Arc::new(AtomicUsize::new(0));
        let observed = calls.clone();
        let value = DeferredResource::new(move || {
            observed.fetch_add(1, Ordering::SeqCst);
            42
        });
        assert_eq!(
            calls.load(Ordering::SeqCst),
            0,
            "unused GPU resources must not be compiled at startup"
        );
        assert!(!value.is_ready());
        assert_eq!(*value, 42);
        assert!(value.is_ready());
        assert_eq!(*value, 42);
        assert_eq!(calls.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn first_use_completes_once_across_threads() {
        let calls = Arc::new(AtomicUsize::new(0));
        let observed = calls.clone();
        let value = Arc::new(DeferredResource::new(move || {
            observed.fetch_add(1, Ordering::SeqCst);
            String::from("ready")
        }));
        std::thread::scope(|scope| {
            for _ in 0..4 {
                let value = &value;
                scope.spawn(move || assert_eq!(value.as_str(), "ready"));
            }
        });
        assert!(value.is_ready());
        assert_eq!(calls.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn dropping_an_unused_resource_drops_captures_without_running_it() {
        struct Capture(Arc<AtomicUsize>);
        impl Drop for Capture {
            fn drop(&mut self) {
                self.0.fetch_add(1, Ordering::SeqCst);
            }
        }
        let drops = Arc::new(AtomicUsize::new(0));
        let capture = Capture(drops.clone());
        let value = DeferredResource::new(move || {
            drop(capture);
            panic!("unused factory ran");
        });
        drop(value);
        assert_eq!(drops.load(Ordering::SeqCst), 1);
    }
}
