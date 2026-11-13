//! Native buffer modification counters, shared by text and binary storage.
//!
//! This records edits and explicit clean/dirty transitions only. Undo history
//! and undo-based restoration of a saved state are not available here.

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Modification {
    pub modiff: u64,
    pub chars_modiff: u64,
    pub save_modiff: u64,
    pub autosave_modiff: u64,
    /// GNU `auto_save_failure_time': when an auto-save last took over a
    /// minute (seconds since the epoch), else 0.
    pub auto_save_failure: i64,
    /// GNU `BUF_COMPACT`: MODIFF when a collection last truncated the undo
    /// list (`Vm::undo_round`).
    pub compact: u64,
}
romacs_snapshot::snapshot_struct!(Modification { modiff, chars_modiff, save_modiff, autosave_modiff, auto_save_failure, compact });

impl Default for Modification {
    fn default() -> Self {
        Self {
            modiff: 1,
            chars_modiff: 1,
            save_modiff: 1,
            autosave_modiff: 0,
            auto_save_failure: 0,
            compact: 1,
        }
    }
}

impl Modification {
    /// GNU `modiff_incr` (lisp.h): one plus floor(log2(len)), or one
    /// for zero. Storage callers supply the removed plus inserted character count.
    /// A character edit sets CHARS_MODIFF to the resulting MODIFF, rather
    /// than independently incrementing it; property-only edits leave it alone.
    pub fn changed(&mut self, len: usize, chars: bool) {
        let increment = if len == 0 {
            1
        } else {
            u64::from(usize::BITS - len.leading_zeros())
        };
        self.modiff = self
            .modiff
            .checked_add(increment)
            .expect("modification counter overflow");
        if chars {
            self.chars_modiff = self.modiff;
        }
    }

    /// GNU `Frestore_buffer_modified_p` (buffer.c): marking a clean buffer
    /// dirty saves the old MODIFF and increments only MODIFF, not CHARS_MODIFF.
    pub fn set_modified(&mut self, flag: bool) {
        if !flag {
            self.save_modiff = self.modiff;
        } else if self.save_modiff >= self.modiff {
            self.save_modiff = self.modiff;
            self.changed(1, false);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn logarithmic_increment_and_character_tick_follow_modiff() {
        for (len, increment) in [(0, 1), (1, 1), (2, 2), (3, 2), (4, 3), (7, 3), (8, 4)] {
            let mut state = Modification::default();
            state.changed(len, false);
            assert_eq!(state.modiff, 1 + increment);
            assert_eq!(state.chars_modiff, 1);
            state.changed(1, true);
            assert_eq!(state.chars_modiff, state.modiff);
            assert_eq!(state.save_modiff, 1);
            assert_eq!(state.autosave_modiff, 0);
        }
    }

    #[test]
    fn explicit_modified_transitions_do_not_fabricate_character_edits() {
        let mut state = Modification::default();
        state.set_modified(true);
        assert_eq!(
            (state.modiff, state.chars_modiff, state.save_modiff),
            (2, 1, 1)
        );
        state.set_modified(true);
        assert_eq!(state.modiff, 2);
        state.set_modified(false);
        assert_eq!(state.save_modiff, 2);
        state.set_modified(true);
        assert_eq!(
            (state.modiff, state.chars_modiff, state.save_modiff),
            (3, 1, 2)
        );
        state.save_modiff = 100;
        state.set_modified(true);
        assert_eq!((state.modiff, state.save_modiff), (4, 3));
    }
}
