//! The VM's symbol slots, and which of them were written since the last
//! collection.
//!
//! A minor collection reads no slot but those: one written may name what
//! was made since, and every other names only what a collection promoted.
//! Slots are written by plain field assignment all over the VM, so the
//! table hands out `&mut Slot` through nothing but `IndexMut`, which marks
//! the card of the slot it reaches, one byte for `CARD` slots, and takes a
//! new slot through `push`, which marks its card: a write that marks no
//! card does not compile. The collector's own rewrite (`collector_iter_mut`) changes where
//! values are, not which, and marks nothing.
use crate::Slot;

/// Slots to a card.
const CARD: usize = 64;

#[derive(Default)]
pub(crate) struct Slots {
    list: Vec<Slot>,
    /// One byte for each `CARD` slots: nonzero when one of them may have
    /// been written since the last collection.
    cards: Vec<u8>,
}
romacs_snapshot::snapshot_struct!(Slots { list, cards });

impl std::ops::Deref for Slots {
    type Target = [Slot];
    fn deref(&self) -> &[Slot] {
        &self.list
    }
}
impl std::ops::Index<usize> for Slots {
    type Output = Slot;
    #[inline]
    fn index(&self, i: usize) -> &Slot {
        &self.list[i]
    }
}
impl std::ops::IndexMut<usize> for Slots {
    #[inline]
    fn index_mut(&mut self, i: usize) -> &mut Slot {
        let slot = &mut self.list[i];
        // In bounds: the cards cover every slot (`push`).
        self.cards[i / CARD] = 1;
        slot
    }
}
impl<'a> IntoIterator for &'a Slots {
    type Item = &'a Slot;
    type IntoIter = std::slice::Iter<'a, Slot>;
    fn into_iter(self) -> Self::IntoIter {
        self.list.iter()
    }
}

impl Slots {
    /// SLOT as a new slot, written.
    pub(crate) fn push(&mut self, slot: Slot) {
        let i = self.list.len();
        if i / CARD >= self.cards.len() {
            self.cards.push(0);
        }
        self.cards[i / CARD] = 1;
        self.list.push(slot);
    }
    pub(crate) fn reserve(&mut self, additional: usize) {
        self.list.reserve(additional);
        self.cards.reserve(additional / CARD + 1);
    }
    /// No slot at all.
    pub(crate) fn clear(&mut self) {
        self.list.clear();
        self.cards.clear();
    }
    /// The ranges of slots that may have been written since the last
    /// collection, a card each.
    pub(crate) fn written(&self) -> impl Iterator<Item = std::ops::Range<usize>> + '_ {
        let len = self.list.len();
        self.cards.iter().enumerate().filter(|&(_, &card)| card != 0).map(move |(i, _)| i * CARD..((i + 1) * CARD).min(len))
    }
    /// A collection ended: every slot's values are old.
    pub(crate) fn settle(&mut self) {
        self.cards.fill(0);
    }
    /// Whether no slot counts as written: what a completed collection leaves.
    #[cfg(test)]
    pub(crate) fn settled(&self) -> bool {
        self.cards.iter().all(|&card| card == 0)
    }
    /// Every slot, for the collector to rewrite the values of: it marks
    /// nothing.
    pub(crate) fn collector_iter_mut(&mut self) -> std::slice::IterMut<'_, Slot> {
        self.list.iter_mut()
    }
}
