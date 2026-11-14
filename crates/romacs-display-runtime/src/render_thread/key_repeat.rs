//! Bound timer-generated repeats at the evaluator read boundary, not the
//! render-to-input bridge. GNU's GTK repeat timer shares its command thread;
//! our native event loop must not accrue seconds of commands while Lisp runs.
//! Each held key's press time also gives the usage log's `repeat` record
//! (`romacs_display_protocol::key_repeat_log`) its delays.
use romacs_display_protocol::frame_time::EventTime;
use romacs_display_protocol::input_progress::InputReceipt;
use std::collections::HashMap;
use std::time::Duration;
use winit::keyboard::PhysicalKey;
use winit::window::WindowId;

#[derive(Default)]
pub(super) struct KeyRepeats {
    pending: HashMap<(WindowId, PhysicalKey), InputReceipt>,
    /// When each held key's press was seen.
    held: HashMap<(WindowId, PhysicalKey), EventTime>,
}

impl KeyRepeats {
    pub fn admit(&self, window: WindowId, key: PhysicalKey, repeat: bool) -> bool {
        // Never coalesce physical presses (including rapid release/repress).
        !repeat
            || self
                .pending
                .get(&(window, key))
                .is_some_and(InputReceipt::taken)
    }

    pub fn queued(&mut self, window: WindowId, key: PhysicalKey, receipt: Option<InputReceipt>) {
        self.pending.remove(&(window, key));
        if let Some(receipt) = receipt {
            self.pending.insert((window, key), receipt);
        }
    }

    /// An admitted press seen AT: a physical one starts its key's hold; a
    /// repeat answers how long after the press it came.
    pub fn pressed(&mut self, window: WindowId, key: PhysicalKey, repeat: bool, at: EventTime) -> Option<Duration> {
        if repeat {
            self.held.get(&(window, key)).map(|since| at.saturating_since(*since))
        } else {
            self.held.insert((window, key), at);
            None
        }
    }

    /// KEY released, seen AT: how long it was held, when its press was seen.
    pub fn release(&mut self, window: WindowId, key: PhysicalKey, at: EventTime) -> Option<Duration> {
        self.pending.remove(&(window, key));
        self.held.remove(&(window, key)).map(|since| at.saturating_since(since))
    }

    pub fn retire_window(&mut self, window: WindowId) {
        self.pending.retain(|(owner, _), _| *owner != window);
        self.held.retain(|(owner, _), _| *owner != window);
    }
}

#[cfg(test)]
mod tests;
