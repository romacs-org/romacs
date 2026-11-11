//! The interned symbols `commandp` may accept, kept as definitions change,
//! so M-x filters them rather than every symbol on each key
//! (lisp/romacs/romacs-commands.el).
//!
//! Writes only mark what changed; `romacs-command-symbols` brings the set
//! up to date when asked, judging the marked slots and, as an alias's
//! verdict follows its target, every alias. The set holds each symbol
//! `commandp` accepts and each `OClosure`, whose interactive form only Lisp
//! computes: a superset its callers still filter with `commandp`.

#[derive(Default)]
pub(crate) struct CommandSet {
    /// By slot: whether its symbol is in the set. Empty until first asked.
    member: Vec<bool>,
    built: bool,
    /// Slots whose definition or interning changed since the set was
    /// brought up to date.
    dirty: Vec<usize>,
    /// Counts the writes that can change the set, or what M-x's predicate
    /// reads of a member: a definition, an interning, a member's plist.
    epoch: u64,
}
romacs_snapshot::snapshot_struct!(CommandSet { member, built, dirty, epoch });

/// What to judge to bring the set up to date.
pub(crate) enum Stale {
    Fresh,
    /// Every slot: the set was never built, or more changed than it holds.
    All,
    /// These slots, and every alias.
    Changed(Vec<usize>),
}

impl CommandSet {
    /// SLOT's function cell, or whether its symbol is interned, changed.
    pub(crate) fn changed(&mut self, slot: usize) {
        self.epoch += 1;
        if !self.built {
            return;
        }
        // Rebuilding beats replaying more changes than there are symbols.
        if self.dirty.len() >= self.member.len() {
            self.built = false;
            self.dirty = Vec::new();
        } else {
            self.dirty.push(slot);
        }
    }

    /// SLOT's plist was written: its symbol's `completion-predicate`,
    /// `command-modes` or obsolescence may have changed.
    pub(crate) fn plist_written(&mut self, slot: usize) {
        if self.contains(slot) {
            self.epoch += 1;
        }
    }

    pub(crate) fn epoch(&self) -> u64 {
        self.epoch
    }

    pub(crate) fn contains(&self, slot: usize) -> bool {
        self.member.get(slot).copied().unwrap_or(false)
    }

    /// What to judge now; the set counts as up to date once each is `set`.
    pub(crate) fn take_stale(&mut self) -> Stale {
        if !self.built {
            self.built = true;
            self.dirty = Vec::new();
            self.member = Vec::new();
            return Stale::All;
        }
        if self.dirty.is_empty() {
            return Stale::Fresh;
        }
        Stale::Changed(std::mem::take(&mut self.dirty))
    }

    pub(crate) fn set(&mut self, slot: usize, member: bool) {
        if slot >= self.member.len() {
            if !member {
                return;
            }
            self.member.resize(slot + 1, false);
        }
        self.member[slot] = member;
    }

    /// The members' slots, in slot order: the initial obarray's order.
    pub(crate) fn slots(&self) -> impl Iterator<Item = usize> + '_ {
        self.member.iter().enumerate().filter(|(_, m)| **m).map(|(slot, _)| slot)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn changes_are_kept_until_taken_and_bounded_by_the_set() {
        let mut set = CommandSet::default();
        set.changed(3);
        assert!(matches!(set.take_stale(), Stale::All), "the first ask judges all");
        set.set(3, true);
        set.set(1, false);
        assert!(matches!(set.take_stale(), Stale::Fresh));
        set.changed(1);
        assert!(matches!(set.take_stale(), Stale::Changed(ref s) if s == &[1]));
        for slot in 0..10 {
            set.changed(slot);
        }
        assert!(matches!(set.take_stale(), Stale::All), "more changes than members rebuild");
    }

    #[test]
    fn only_a_members_plist_moves_the_epoch() {
        let mut set = CommandSet::default();
        let _ = set.take_stale();
        set.set(2, true);
        let epoch = set.epoch();
        set.plist_written(1);
        assert_eq!(set.epoch(), epoch);
        set.plist_written(2);
        assert!(set.epoch() > epoch);
        assert_eq!(set.slots().collect::<Vec<_>>(), [2]);
    }
}
