//! The busy indicator: a quiet "Evaluating… C-g to quit" shown over the
//! last frame while the evaluator has run one command for more than
//! [`SHOW_AFTER`], drawn by the render thread alone (the evaluator is
//! busy, so it paints nothing).
//!
//! The render thread is not woken when the evaluator starts work, so a key
//! it sends arms a check: [`SHOW_AFTER`] later it asks for the evaluator's
//! busy span (`InputInterrupt::busy_span`), and while the indicator shows
//! it asks again every [`RECHECK`] to notice the end.
use romacs_display_protocol::frame_time::EventTime;
use std::time::Duration;

/// How long the evaluator runs one command before the indicator shows.
pub(super) const SHOW_AFTER: Duration = Duration::from_millis(300);
/// How often a shown indicator checks that the evaluator is still busy.
const RECHECK: Duration = Duration::from_millis(100);

/// What the indicator says.
pub(crate) const LABEL: &str = "Evaluating\u{2026} C-g to quit";

#[derive(Debug, Default)]
pub(super) struct BusyToast {
    /// When the first key since the last check was sent.
    armed: Option<EventTime>,
    /// The busy span seen, from when.
    span: Option<(u32, EventTime)>,
    shown: bool,
}

impl BusyToast {
    /// A key went to the evaluator at NOW: check on it later.
    pub(super) fn arm(&mut self, now: EventTime) {
        self.armed.get_or_insert(now);
    }

    /// Whether the indicator shows.
    pub(super) fn shown(&self) -> bool {
        self.shown
    }

    /// When to check next, if at all.
    pub(super) fn deadline(&self, now: EventTime) -> Option<EventTime> {
        match (self.shown, self.span, self.armed) {
            (true, _, _) => Some(now.plus(RECHECK)),
            (false, Some((_, since)), _) => Some(since.plus(SHOW_AFTER)),
            (false, None, Some(armed)) => Some(armed.plus(SHOW_AFTER)),
            (false, None, None) => None,
        }
    }

    /// Check at NOW with the evaluator's busy SPAN: whether the indicator
    /// appeared or went.
    pub(super) fn update(&mut self, now: EventTime, span: Option<u32>) -> bool {
        let due = self.span.is_some() || self.armed.is_some_and(|armed| now >= armed.plus(SHOW_AFTER));
        if !due {
            return false;
        }
        let since = match (span, self.span) {
            (None, _) => None,
            (Some(span), Some((seen, since))) if seen == span => Some(since),
            // A span first seen started no earlier than the key that armed
            // the check.
            (Some(_), _) => Some(self.armed.unwrap_or(now)),
        };
        self.span = span.zip(since);
        self.armed = None;
        let shown = since.is_some_and(|since| now >= since.plus(SHOW_AFTER));
        std::mem::replace(&mut self.shown, shown) != shown
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shows_while_one_span_lasts_and_goes_when_it_ends() {
        let start = romacs_display_protocol::frame_time::observe_platform_now();
        let mut toast = BusyToast::default();
        toast.arm(start);
        assert_eq!(toast.deadline(start), Some(start.plus(SHOW_AFTER)));
        assert!(!toast.update(start.plus(Duration::from_millis(100)), Some(4)), "not due yet");
        assert!(toast.update(start.plus(SHOW_AFTER), Some(4)), "busy since the key: shown");
        assert!(toast.shown());
        assert!(!toast.update(start.plus(Duration::from_secs(2)), Some(4)));
        assert!(toast.update(start.plus(Duration::from_secs(3)), None), "idle: gone");
        assert!(!toast.shown());
        assert_eq!(toast.deadline(start), None, "nothing to check until the next key");
    }

    #[test]
    fn a_short_command_never_shows_it() {
        let start = romacs_display_protocol::frame_time::observe_platform_now();
        let mut toast = BusyToast::default();
        toast.arm(start);
        assert!(!toast.update(start.plus(SHOW_AFTER), None));
        assert!(!toast.shown());
        assert_eq!(toast.deadline(start), None);
    }

    #[test]
    fn a_new_span_waits_its_own_delay() {
        let start = romacs_display_protocol::frame_time::observe_platform_now();
        let mut toast = BusyToast::default();
        toast.arm(start);
        assert!(toast.update(start.plus(SHOW_AFTER), Some(1)));
        let later = start.plus(Duration::from_secs(1));
        assert!(toast.update(later, Some(2)), "another command: hidden until it lasts");
        assert!(!toast.shown());
        assert_eq!(toast.deadline(start), Some(later.plus(SHOW_AFTER)));
        assert!(toast.update(later.plus(SHOW_AFTER), Some(2)));
    }
}
