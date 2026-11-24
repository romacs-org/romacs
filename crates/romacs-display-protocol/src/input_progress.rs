//! Whether the evaluator has taken a queued input event.
//!
//! The render thread wraps a pressed key in a delivery and keeps its
//! receipt. The evaluator takes the event off the input queue and drops the
//! delivery, which marks the receipt taken. Native key repeats are bounded
//! by it: a held key queues its next repeat only once the last was taken,
//! so the event loop never accrues seconds of commands while Lisp runs.
use crate::frame_time::EventTime;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

/// What the render thread keeps of a queued event.
#[derive(Clone)]
pub struct InputReceipt(Arc<AtomicBool>);

/// The queued event's half: dropping its last copy marks the receipt taken.
#[derive(Clone, Debug)]
pub struct InputDelivery(Arc<Delivery>, Option<EventTime>);

#[derive(Debug)]
struct Delivery(Arc<AtomicBool>);

impl Drop for Delivery {
    fn drop(&mut self) {
        self.0.store(true, Ordering::Release);
    }
}

impl InputDelivery {
    #[must_use]
    pub fn new() -> Self {
        Self::with_ingress(None)
    }

    /// The delivery of a native key, with its platform arrival when known.
    #[must_use]
    pub fn with_ingress(ingress: Option<EventTime>) -> Self {
        Self(Arc::new(Delivery(Arc::default())), ingress)
    }

    /// When the platform delivered the key press, stamped on the render
    /// thread; None for an event no platform key made.
    #[must_use]
    pub fn ingress(&self) -> Option<EventTime> {
        self.1
    }

    #[must_use]
    pub fn receipt(&self) -> InputReceipt {
        InputReceipt(self.0.0.clone())
    }
}

impl Default for InputDelivery {
    fn default() -> Self {
        Self::new()
    }
}

impl std::fmt::Debug for InputReceipt {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_tuple("InputReceipt").field(&self.taken()).finish()
    }
}

impl InputReceipt {
    /// The evaluator has taken the event: its delivery is gone.
    #[must_use]
    pub fn taken(&self) -> bool {
        self.0.load(Ordering::Acquire)
    }

    /// Whether OTHER is a receipt for the same event.
    #[must_use]
    pub fn same_input(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.0, &other.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dropping_the_delivery_marks_its_receipt_taken() {
        let delivery = InputDelivery::new();
        let receipt = delivery.receipt();
        let copy = delivery.clone();
        drop(delivery);
        assert!(!receipt.taken(), "a copy still queued keeps it untaken");
        drop(copy);
        assert!(receipt.taken());
    }

    #[test]
    fn cloned_delivery_carries_the_same_ingress() {
        let at = crate::frame_time::observe_platform_now();
        let delivery = InputDelivery::with_ingress(Some(at));
        assert_eq!(delivery.clone().ingress(), Some(at));
        assert_eq!(InputDelivery::new().ingress(), None);
    }

    #[test]
    fn a_receipt_names_its_own_event_only() {
        let delivery = InputDelivery::new();
        assert!(delivery.receipt().same_input(&delivery.receipt()));
        assert!(!delivery.receipt().same_input(&InputDelivery::new().receipt()));
    }
}
