//! The busy indicator: a quiet "Evaluating… C-g to quit" shown over the
//! last frame while the evaluator has run one command for more than
//! [`SHOW_AFTER`], drawn by the render thread alone (the evaluator is
//! busy, so it paints nothing).
//!
//! The render thread is not woken when the evaluator starts work, so a key
//! it sends arms a check: [`SHOW_AFTER`] later it asks how long the
//! evaluator has been busy (`InputInterrupt::busy_for`), and asks again
//! when its command could have run that long, or, while the indicator
//! shows, every [`RECHECK`] to notice the end. The evaluator stamps when
//! each command starts: keys that follow each other quickly, each its own
//! short command, never show the indicator however long they last.
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
    /// When to ask the evaluator next, if at all.
    due: Option<EventTime>,
    shown: bool,
}

impl BusyToast {
    /// A key went to the evaluator at NOW: check on it later.
    pub(super) fn arm(&mut self, now: EventTime) {
        let at = now.plus(SHOW_AFTER);
        self.due = Some(self.due.map_or(at, |due| due.min(at)));
    }

    /// Whether the indicator shows.
    pub(super) fn shown(&self) -> bool {
        self.shown
    }

    /// When to check next, if at all.
    pub(super) fn deadline(&self) -> Option<EventTime> {
        self.due
    }

    /// At NOW, when a check is due, ask `busy_for` how long the evaluator
    /// has run its command: whether the indicator appeared or went.
    pub(super) fn update(&mut self, now: EventTime, busy_for: impl FnOnce() -> Option<Duration>) -> bool {
        if self.due.is_none_or(|due| now < due) {
            return false;
        }
        let busy = busy_for();
        let shown = busy.is_some_and(|ran| ran >= SHOW_AFTER);
        self.due = busy.map(|ran| now.plus(if shown { RECHECK } else { SHOW_AFTER.saturating_sub(ran) }));
        std::mem::replace(&mut self.shown, shown) != shown
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ms(n: u64) -> Duration {
        Duration::from_millis(n)
    }

    #[test]
    fn shows_while_one_command_lasts_and_goes_when_it_ends() {
        let start = romacs_display_protocol::frame_time::observe_platform_now();
        let mut toast = BusyToast::default();
        toast.arm(start);
        assert_eq!(toast.deadline(), Some(start.plus(SHOW_AFTER)));
        assert!(!toast.update(start.plus(ms(100)), || panic!("not due: the evaluator is not asked")));
        assert!(toast.update(start.plus(SHOW_AFTER), || Some(SHOW_AFTER)), "busy since the key: shown");
        assert!(toast.shown());
        assert_eq!(toast.deadline(), Some(start.plus(SHOW_AFTER + RECHECK)));
        assert!(!toast.update(start.plus(ms(2000)), || Some(ms(2000))));
        assert!(toast.update(start.plus(ms(3000)), || None), "idle: gone");
        assert!(!toast.shown());
        assert_eq!(toast.deadline(), None, "nothing to check until the next key");
    }

    #[test]
    fn a_short_command_never_shows_it() {
        let start = romacs_display_protocol::frame_time::observe_platform_now();
        let mut toast = BusyToast::default();
        toast.arm(start);
        assert!(!toast.update(start.plus(SHOW_AFTER), || None));
        assert!(!toast.shown());
        assert_eq!(toast.deadline(), None);
    }

    /// A held key: a key every 25 ms, each its own command of 10 ms. The
    /// evaluator is busy at most checks, never for long.
    #[test]
    fn short_commands_one_after_another_never_show_it() {
        let start = romacs_display_protocol::frame_time::observe_platform_now();
        let mut toast = BusyToast::default();
        for key in 0..400 {
            let now = start.plus(ms(25 * key));
            toast.arm(now);
            assert!(!toast.update(now, || Some(ms(10))), "key {key}");
            assert!(!toast.shown());
        }
    }

    #[test]
    fn a_command_already_running_shows_at_the_check() {
        let start = romacs_display_protocol::frame_time::observe_platform_now();
        let mut toast = BusyToast::default();
        toast.arm(start);
        assert!(toast.update(start.plus(SHOW_AFTER), || Some(ms(2000))));
    }

    #[test]
    fn the_next_command_waits_its_own_delay() {
        let start = romacs_display_protocol::frame_time::observe_platform_now();
        let mut toast = BusyToast::default();
        toast.arm(start);
        assert!(toast.update(start.plus(SHOW_AFTER), || Some(SHOW_AFTER)));
        let later = start.plus(ms(1000));
        assert!(toast.update(later, || Some(ms(40))), "another command: hidden until it lasts");
        assert!(!toast.shown());
        assert_eq!(toast.deadline(), Some(later.plus(ms(260))), "asked again when it could have");
        assert!(toast.update(later.plus(ms(260)), || Some(SHOW_AFTER)));
        assert!(toast.shown());
    }
}
