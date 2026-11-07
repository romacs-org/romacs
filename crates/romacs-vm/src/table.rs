//! A heap table: the entries a Value indexes and, beside them, one colour
//! byte each, which is both the allocator's map and the collector's marks
//! (design note gc-smooth.md 4.1). Entries enter only through this type, so
//! occupancy is exact by construction, and they do not move but in a
//! compaction.
//!
//! Between collections an entry is FREE, ALLOCATED or, under verify,
//! QUARANTINED. Marking turns ALLOCATED into MARKED; after a completed mark
//! dead is exactly ALLOCATED, and `sweep' frees those and turns MARKED back.
//! So after a finished collection a table holds no marks; one a key press or a
//! panic ended keeps them, unread, until the next mark clears them (`raw').
use std::ops::{Deref, DerefMut};
use std::sync::atomic::{AtomicU8, Ordering};

const FREE: u8 = 0;
const ALLOCATED: u8 = 1;
/// ALLOCATED with bit 1 set: clearing the bit unmarks eight entries at once.
const MARKED: u8 = 3;
const QUARANTINED: u8 = 4;
const MARK_BIT: u64 = 0x0202_0202_0202_0202;
const LOW_BITS: u64 = 0x0101_0101_0101_0101;
const HIGH_BITS: u64 = 0x8080_8080_8080_8080;

/// Bytes of entries past which a table stops copying to grow.
const RESERVE_FROM: usize = 1 << 20;
/// The address space such a table takes, once. Pages are committed as they
/// are touched; a table past this copies to grow, as any `Vec`.
const RESERVE_BYTES: usize = 8 << 30;

/// Whether a byte of WORD is zero.
#[inline(always)]
fn has_zero_byte(word: u64) -> bool {
    word.wrapping_sub(LOW_BITS) & !word & HIGH_BITS != 0
}

/// A table as a mark's threads see it (gc_mark.rs): entries one at a time, and
/// atomic colours. Good only while the table neither grows nor is otherwise
/// touched, which a stopped mutator ensures.
pub(crate) struct RawTable<T> {
    entries: *mut T,
    colors: *const AtomicU8,
    len: usize,
    /// What the table was when this was made: what `holds' asks again.
    free: usize,
    quarantined: usize,
    generation: u64,
}

// SAFETY: the one place a collection's threads meet outside the compiler's
// sight. Colours are atomic; entries are read while nothing writes them, and
// written only by the thread that claimed them, or swept them, alone. A
// thread reads an entry only if its type is `Sync' (`get'), or through a view
// that reads no `Cell' (gc_mark.rs `Seen').
unsafe impl<T> Sync for RawTable<T> {}

impl<T> RawTable<T> {
    #[inline(always)]
    fn color(&self, i: usize) -> &AtomicU8 {
        assert!(i < self.len, "a Value past its table");
        // SAFETY: in bounds, and AtomicU8 has the layout of u8; nothing reads
        // the colours but through this while a mark runs.
        unsafe { &*self.colors.add(i) }
    }
    /// Mark entry I if it is in use: true for the one caller that does.
    #[inline(always)]
    pub(crate) fn claim(&self, i: usize) -> bool {
        let color = self.color(i);
        color.load(Ordering::Relaxed) == ALLOCATED && color.swap(MARKED, Ordering::Relaxed) == ALLOCATED
    }
    pub(crate) fn is_free(&self, i: usize) -> bool {
        matches!(self.color(i).load(Ordering::Relaxed), FREE | QUARANTINED)
    }
    pub(crate) fn len(&self) -> usize {
        self.len
    }
    /// Whether TABLE is still as it was when this was made: the same storage,
    /// entries and colours both, length, free entries and marks. Anything that
    /// allocated, grew, swept or compacted it since would make a sweep through
    /// this write where it must not, or free what was made after the mark; a
    /// mark begun or cleared since would make it free what this one reached.
    /// It is the table's shape and marks, not its entries' contents, which
    /// the VM writes between a mark and its sweep (`Object::Free' for the
    /// dead, a cell's value).
    pub(crate) fn holds(&self, table: &Table<T>) -> bool {
        std::ptr::eq(self.entries, table.entries.as_ptr()) && std::ptr::eq(self.colors.cast::<u8>(), table.colors.as_ptr())
            && self.len == table.entries.len() && self.len == table.colors.len()
            && self.free == table.free && self.quarantined == table.quarantined && self.generation == table.generation
    }
    /// A sweep of entries RANGE: each dead one through DESTROY, which leaves a
    /// tombstone owning nothing, then free (QUARANTINE: quarantined), and
    /// every mark gone. The entries freed, and the end of the last still in
    /// use. SAFETY: RANGE is this caller's alone, and starts on a multiple of 8.
    pub(crate) unsafe fn sweep_range(&self, range: std::ops::Range<usize>, quarantine: bool, mut destroy: impl FnMut(&mut T)) -> (usize, usize) {
        debug_assert!(range.start % 8 == 0 && range.end <= self.len);
        let dead = if quarantine { QUARANTINED } else { FREE };
        // Only this range: other threads hold the ones beside it.
        let base = range.start;
        let colors = unsafe { std::slice::from_raw_parts_mut((self.colors as *mut u8).add(base), range.len()) };
        let (mut freed, mut last) = (0, 0);
        let mut at = 0;
        while at < colors.len() {
            let end = (at + 8).min(colors.len());
            // Eight entries with none dead, by far the commonest case when
            // the heap is mostly live: unmark them in one store.
            if let Ok(bytes) = <[u8; 8]>::try_from(&colors[at..end]) {
                let word = u64::from_le_bytes(bytes);
                if !has_zero_byte(word ^ LOW_BITS) {
                    let clean = word & !MARK_BIT;
                    colors[at..end].copy_from_slice(&clean.to_le_bytes());
                    if clean != 0 {
                        last = base + end - clean.leading_zeros() as usize / 8;
                    }
                    at = end;
                    continue;
                }
            }
            for i in at..end {
                match colors[i] {
                    ALLOCATED => {
                        destroy(unsafe { &mut *self.entries.add(base + i) });
                        colors[i] = dead;
                        freed += 1;
                        if quarantine { last = base + i + 1; }
                    }
                    MARKED => {
                        colors[i] = ALLOCATED;
                        last = base + i + 1;
                    }
                    QUARANTINED => last = base + i + 1,
                    _ => {}
                }
            }
            at = end;
        }
        (freed, last)
    }
    /// The dead entries of RANGE that WANTED, given each index, accepts.
    pub(crate) fn dead_in(&self, range: std::ops::Range<usize>, wanted: impl Fn(usize) -> bool, found: &mut Vec<u32>) {
        assert!(range.end <= self.len, "a range past its table");
        let colors = unsafe { std::slice::from_raw_parts(self.colors as *const u8, self.len) };
        let mut at = range.start;
        while at < range.end {
            let end = (at + 8).min(range.end);
            if let Ok(bytes) = <[u8; 8]>::try_from(&colors[at..end]) && !has_zero_byte(u64::from_le_bytes(bytes) ^ LOW_BITS) {
                at = end;
                continue;
            }
            for i in at..end {
                if colors[i] == ALLOCATED && wanted(i) {
                    found.push(i as u32);
                }
            }
            at = end;
        }
    }
    /// Entry I, which any thread may read. SAFETY: nothing writes it for as
    /// long as this lives.
    #[inline(always)]
    pub(crate) unsafe fn get(&self, i: usize) -> &T
    where
        T: Sync,
    {
        unsafe { self.get_unsync(i) }
    }
    /// Entry I, for a view that reads no part of it a `Cell' or a lock guards.
    /// SAFETY: as `get', and the caller reads only such parts.
    #[inline(always)]
    pub(crate) unsafe fn get_unsync(&self, i: usize) -> &T {
        assert!(i < self.len, "a Value past its table");
        unsafe { &*self.entries.add(i) }
    }
    /// Entry I, the caller's alone. SAFETY: the caller claimed it, and nothing
    /// else reaches it for as long as this lives.
    #[allow(clippy::mut_from_ref)]
    #[inline(always)]
    pub(crate) unsafe fn get_mut(&self, i: usize) -> &mut T {
        assert!(i < self.len, "a Value past its table");
        unsafe { &mut *self.entries.add(i) }
    }
}

pub(crate) struct Table<T> {
    entries: Vec<T>,
    colors: Vec<u8>,
    /// No free entry lies below this.
    cursor: usize,
    /// Entries FREE.
    free: usize,
    /// Entries QUARANTINED, which count as free.
    quarantined: usize,
    /// Whether `entries' holds its one reservation: `capacity' then says
    /// what the table has touched, and it never shrinks.
    reserved: bool,
    /// The longest the table has been since its pages were last returned.
    high: usize,
    /// A mark began and no sweep has ended it: marks may be left.
    marking: bool,
    /// Which marks the colours hold: a new mark, or marks cleared, is another.
    generation: u64,
    /// Free entries `alloc' has handed out again, for tests.
    #[cfg(test)]
    pub(crate) reused: usize,
}

impl<T> Default for Table<T> {
    fn default() -> Self {
        Self {
            entries: Vec::new(), colors: Vec::new(), cursor: 0, free: 0, quarantined: 0, reserved: false, high: 0, marking: false, generation: 0,
            #[cfg(test)]
            reused: 0,
        }
    }
}

// SAFETY: as its fields, each SnapshotSafe; none addresses anything outside
// the table's own storage.
unsafe impl<T: romacs_snapshot::SnapshotSafe> romacs_snapshot::SnapshotSafe for Table<T> {}

impl<T> Deref for Table<T> {
    type Target = [T];
    #[inline(always)]
    fn deref(&self) -> &[T] {
        &self.entries
    }
}

impl<T> DerefMut for Table<T> {
    #[inline(always)]
    fn deref_mut(&mut self) -> &mut [T] {
        &mut self.entries
    }
}

impl<'a, T> IntoIterator for &'a Table<T> {
    type Item = &'a T;
    type IntoIter = std::slice::Iter<'a, T>;
    fn into_iter(self) -> Self::IntoIter {
        self.entries.iter()
    }
}

impl<T> Table<T> {
    /// A new entry holding VALUE: the lowest free one, else one at the end.
    /// Its index.
    #[inline]
    pub(crate) fn alloc(&mut self, value: T) -> usize {
        if self.free == 0 {
            return self.push(value);
        }
        let i = self.next_free();
        #[cfg(test)]
        { self.reused += 1; }
        self.free -= 1;
        self.cursor = i + 1;
        self.colors[i] = ALLOCATED;
        self.entries[i] = value;
        i
    }
    /// A new entry at the end, whatever is free below it. Its index.
    #[inline]
    pub(crate) fn push(&mut self, value: T) -> usize {
        if self.entries.len() == self.entries.capacity() {
            self.grow();
        }
        self.entries.push(value);
        self.colors.push(ALLOCATED);
        self.entries.len() - 1
    }
    pub(crate) fn extend(&mut self, values: impl IntoIterator<Item = T>) {
        for value in values {
            self.push(value);
        }
    }
    /// The lowest free entry at or above `cursor', there being one.
    #[inline]
    fn next_free(&self) -> usize {
        let mut at = self.cursor;
        if self.colors[at] == FREE {
            return at;
        }
        let mut words = self.colors[at..].chunks_exact(8);
        for word in &mut words {
            if has_zero_byte(u64::from_le_bytes(word.try_into().expect("eight bytes"))) {
                break;
            }
            at += 8;
        }
        at + self.colors[at..].iter().position(|&color| color == FREE).expect("a table's count of free entries")
    }
    /// Room for at least one more: past RESERVE_FROM, where a copy would be
    /// a stall a person can feel, once and for all (`may_reserve').
    #[cold]
    fn grow(&mut self) {
        let size = size_of::<T>().max(1);
        if !self.reserved && self.entries.capacity() * size >= RESERVE_FROM && crate::world::arena::may_reserve() {
            let more = (RESERVE_BYTES / size).saturating_sub(self.entries.len());
            if self.entries.try_reserve_exact(more).is_ok() {
                self.reserved = true;
                let _ = self.colors.try_reserve_exact(more);
                return;
            }
        }
        self.entries.reserve(1);
    }
    pub(crate) fn reserve(&mut self, additional: usize) {
        self.entries.reserve(additional);
        self.colors.reserve(additional);
    }
    /// Entries this table's storage holds, or has touched once it reserved.
    pub(crate) fn capacity(&self) -> usize {
        if self.reserved { self.high.max(self.entries.len()) } else { self.entries.capacity() }
    }
    /// Bytes the table holds for those entries, its colours included.
    pub(crate) fn capacity_bytes(&self) -> usize {
        self.capacity() * (size_of::<T>() + 1)
    }
    /// Entries free to reuse, quarantined ones included.
    #[inline]
    pub(crate) fn unused(&self) -> usize {
        self.free + self.quarantined
    }
    /// Entries in use.
    #[inline]
    pub(crate) fn live(&self) -> usize {
        self.entries.len() - self.unused()
    }
    /// Whether entry I is free or quarantined: a Value naming it is stale.
    #[cfg(test)]
    pub(crate) fn is_free(&self, i: usize) -> bool {
        matches!(self.colors[i], FREE | QUARANTINED)
    }
    /// The table for a mark's threads, what the last mark left if a key press
    /// or a panic ended it gone: it would stop this one at entries it marked.
    /// SAFETY: until the last use of what this returns, the table is not
    /// dropped, and changes only through it; `RawTable::holds' checks that.
    pub(crate) unsafe fn raw(&mut self) -> RawTable<T> {
        if self.marking {
            self.clear_marks();
        }
        self.marking = true;
        self.generation += 1;
        RawTable {
            entries: self.entries.as_mut_ptr(), colors: self.colors.as_mut_ptr().cast(), len: self.entries.len(),
            free: self.free, quarantined: self.quarantined, generation: self.generation,
        }
    }
    /// Mark entry I, as `RawTable::claim'.
    #[cfg(test)]
    pub(crate) fn mark(&mut self, i: usize) -> bool {
        let color = &mut self.colors[i];
        if *color != ALLOCATED {
            return false;
        }
        *color = MARKED;
        true
    }
    /// Whether a mark in progress has reached entry I.
    #[inline(always)]
    pub(crate) fn marked(&self, i: usize) -> bool {
        self.colors[i] == MARKED
    }
    /// Every mark gone, and the free entries counted again: a collection
    /// stopped short, perhaps part way through a sweep.
    pub(crate) fn clear_marks(&mut self) {
        let (mut free, mut quarantined) = (0, 0);
        for color in &mut self.colors {
            if *color == MARKED {
                *color = ALLOCATED;
            }
            free += usize::from(*color == FREE);
            quarantined += usize::from(*color == QUARANTINED);
        }
        (self.free, self.quarantined, self.cursor, self.marking) = (free, quarantined, 0, false);
        self.generation += 1;
    }
    /// After a completed mark: every dead entry through DESTROY, which leaves
    /// a tombstone owning nothing, then free (under QUARANTINE, never reused
    /// and never cut), every mark gone, and the dead tail cut. Entries freed.
    #[cfg(test)]
    pub(crate) fn sweep(&mut self, quarantine: bool, destroy: impl FnMut(&mut T)) -> usize {
        let len = self.entries.len();
        // SAFETY: the whole table, and `&mut self' holds it.
        let (freed, last) = unsafe { self.raw().sweep_range(0..len, quarantine, destroy) };
        self.swept(freed, last, quarantine);
        freed
    }
    /// The books of a sweep in ranges (`RawTable::sweep_range'): FREED entries
    /// in all, the last in use before LAST.
    pub(crate) fn swept(&mut self, freed: usize, last: usize, quarantine: bool) {
        if quarantine { self.quarantined += freed } else { self.free += freed }
        (self.cursor, self.marking) = (0, false);
        self.generation += 1;
        self.cut(last);
    }
    /// Drop the entries from LEN on, all free tombstones.
    fn cut(&mut self, len: usize) {
        self.high = self.high.max(self.entries.len());
        self.free -= self.entries.len() - len;
        // SAFETY: a tombstone owns nothing (`sweep'), so forgetting one
        // leaks nothing, and the length only falls.
        unsafe { self.entries.set_len(len) };
        self.colors.truncate(len);
    }
    /// Give back what the table holds beyond twice its entries, or FLOOR:
    /// a reserved table, past what the next BUDGET allocations will touch.
    pub(crate) fn shrink(&mut self, floor: usize, budget: usize) {
        let len = self.entries.len();
        if !self.reserved {
            let keep = len.saturating_mul(2).max(floor);
            self.entries.shrink_to(keep);
            self.colors.shrink_to(keep);
            return;
        }
        let keep = len.saturating_add(budget).saturating_mul(2);
        if self.high > keep {
            let size = size_of::<T>().max(1);
            // SAFETY: past KEEP, which is past the length, both are spare
            // capacity the table owns and nothing reads, within the reservation.
            unsafe {
                crate::world::arena::release_spare(self.entries.as_mut_ptr().cast(), keep * size, self.high * size);
                crate::world::arena::release_spare(self.colors.as_mut_ptr(), keep, self.high.min(self.colors.capacity()));
            }
            self.high = keep;
        }
    }
    /// Compaction: keep the marked entries, in order, each through KEEP. The
    /// table is left dense.
    pub(crate) fn compact(&mut self, mut keep: impl FnMut(&mut T)) {
        self.high = self.high.max(self.entries.len());
        let (colors, mut i) = (&self.colors, 0);
        self.entries.retain_mut(|entry| {
            let live = colors[i] == MARKED;
            i += 1;
            if live {
                keep(entry);
            }
            live
        });
        self.dense();
    }
    /// N fillers in front of every entry, which move up past them.
    pub(crate) fn prepend(&mut self, n: usize, filler: impl FnMut() -> T) {
        self.entries.splice(0..0, std::iter::repeat_with(filler).take(n));
        self.dense();
    }
    fn dense(&mut self) {
        self.colors.clear();
        self.colors.resize(self.entries.len(), ALLOCATED);
        (self.cursor, self.free, self.quarantined, self.marking) = (self.entries.len(), 0, 0, false);
        self.generation += 1;
    }
    /// The colours as they are, for the serial re-mark to compare.
    pub(crate) fn colors(&self) -> Vec<u8> {
        self.colors.clone()
    }
    /// Whether a mark in progress reached each entry.
    pub(crate) fn marks(&self) -> impl Iterator<Item = bool> + '_ {
        self.colors.iter().map(|&color| color == MARKED)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn table(n: u64) -> Table<u64> {
        let mut table = Table::default();
        table.extend(0..n);
        table
    }
    fn sweep(table: &mut Table<u64>, live: &[usize]) -> usize {
        for &i in live {
            assert!(table.mark(i), "{i} was in use");
        }
        table.sweep(false, |entry| *entry = u64::MAX)
    }

    #[test]
    fn allocation_takes_the_lowest_free_entry_then_the_end() {
        let mut t = table(20);
        assert_eq!(sweep(&mut t, &[0, 1, 2, 3, 4, 5, 6, 7, 8, 11, 19]), 9);
        assert_eq!(t[9], u64::MAX, "a tombstone");
        assert_eq!((t.len(), t.unused(), t.live()), (20, 9, 11));
        assert_eq!(t.alloc(100), 9);
        assert_eq!(t.alloc(101), 10);
        assert_eq!(t.alloc(102), 12);
        assert_eq!((t[9], t[10], t[12]), (100, 101, 102));
        for expected in 13..19 {
            assert_eq!(t.alloc(0), expected);
        }
        assert_eq!(t.unused(), 0);
        assert_eq!(t.alloc(0), 20, "then the end");
    }

    /// Past a long run in use the scan finds the hole, at any offset.
    #[test]
    fn a_free_entry_is_found_past_a_long_live_run() {
        for hole in [1, 7, 8, 9, 63, 64, 65, 1000] {
            let mut t = table(1100);
            let live: Vec<usize> = (0..1100).filter(|&i| i != hole).collect();
            assert_eq!(sweep(&mut t, &live), 1);
            assert_eq!(t.alloc(5), hole);
            assert_eq!(t.unused(), 0);
        }
    }

    #[test]
    fn a_dead_tail_is_cut_however_it_falls_in_a_word() {
        for keep in 1..24 {
            let mut t = table(40);
            let live: Vec<usize> = (0..keep).collect();
            sweep(&mut t, &live);
            assert_eq!((t.len(), t.unused()), (keep, 0), "keeping {keep}");
        }
        let mut t = table(40);
        sweep(&mut t, &[]);
        assert_eq!((t.len(), t.unused()), (0, 0), "nothing survives");
    }

    /// A full sweep leaves every survivor allocated, so the next mark sees
    /// the same table: marks do not survive a collection.
    #[test]
    fn a_table_at_rest_holds_no_marks() {
        let mut t = table(100);
        let all: Vec<usize> = (0..100).collect();
        for _ in 0..3 {
            assert_eq!(sweep(&mut t, &all), 0);
            assert!(t.marks().all(|marked| !marked));
        }
        assert!(t.mark(5));
        assert!(!t.mark(5), "once");
        t.clear_marks();
        assert!(t.mark(5), "an abandoned mark leaves none");
    }

    #[test]
    fn a_stale_reference_marks_nothing() {
        let mut t = table(10);
        sweep(&mut t, &[0, 9]);
        assert!(t.is_free(4));
        assert!(!t.mark(4));
        assert!(!t.marked(4));
        assert_eq!(t.unused(), 8);
    }

    #[test]
    fn quarantined_entries_count_free_and_are_never_reused_or_cut() {
        let mut t = table(10);
        for i in [0, 1] {
            t.mark(i);
        }
        assert_eq!(t.sweep(true, |entry| *entry = u64::MAX), 8);
        assert_eq!((t.len(), t.unused(), t.live()), (10, 8, 2));
        assert_eq!(t.alloc(7), 10, "not reused");
        assert!(t.is_free(9));
        assert!(!t.mark(9));
        for i in [0, 1, 10] {
            t.mark(i);
        }
        assert_eq!(t.sweep(true, |entry| *entry = u64::MAX), 0, "dead once");
    }

    #[test]
    fn compaction_keeps_the_marked_in_order_and_leaves_the_table_dense() {
        let mut t = table(10);
        for i in [2, 5, 7] {
            t.mark(i);
        }
        t.compact(|entry| *entry += 100);
        assert_eq!(&t[..], [102, 105, 107]);
        assert_eq!((t.unused(), t.live()), (0, 3));
        t.prepend(2, || 0);
        assert_eq!(&t[..], [0, 0, 102, 105, 107]);
        assert_eq!(t.alloc(1), 5, "dense: the end");
    }

    /// Growth past the threshold keeps its storage: nothing copies, however
    /// long the table gets, and `capacity' says what was touched.
    #[test]
    fn a_large_table_does_not_move_to_grow() {
        let mut t: Table<u64> = Table::default();
        let bytes = 2 * RESERVE_FROM;
        while t.len() * 8 < bytes {
            t.push(0);
        }
        if !crate::world::arena::may_reserve() { return; }
        assert!(t.reserved, "past RESERVE_FROM");
        let at = t.as_ptr();
        for _ in 0..(4 << 20) {
            t.push(0);
        }
        assert_eq!(t.as_ptr(), at, "no copy");
        assert_eq!(t.capacity(), t.len(), "the reservation is not capacity");
    }

    /// A mark's view refuses its table once anything moved or changed it:
    /// its colours alone, its entries alone, or an entry handed out.
    #[test]
    fn a_changed_table_is_not_held() {
        let moved = |t: &mut Table<u64>, colors: bool| {
            if colors {
                let old = std::mem::take(&mut t.colors);
                t.colors = old.clone();
            } else {
                let old = std::mem::take(&mut t.entries);
                t.entries = old.clone();
            }
        };
        for colors in [true, false] {
            let mut t = table(100);
            let raw = unsafe { t.raw() };
            assert!(raw.holds(&t));
            moved(&mut t, colors);
            assert!(!raw.holds(&t), "moved {}", if colors { "colours" } else { "entries" });
        }
        let mut t = table(100);
        sweep(&mut t, &[0, 1]);
        let raw = unsafe { t.raw() };
        t.alloc(7);
        assert!(!raw.holds(&t), "an entry handed out");
        let mut t = table(100);
        let raw = unsafe { t.raw() };
        t.clear_marks();
        assert!(!raw.holds(&t), "its marks cleared");
        let mut t = table(100);
        let raw = unsafe { t.raw() };
        let _again = unsafe { t.raw() };
        assert!(!raw.holds(&t), "a mark begun again");
        // All live, so nothing about its shape changes: its marks still go.
        let mut t = table(100);
        let raw = unsafe { t.raw() };
        for i in 0..100 { assert!(raw.claim(i)); }
        t.compact(|_| {});
        assert!(!raw.holds(&t), "compacted with every entry kept");
        let mut t = table(100);
        let raw = unsafe { t.raw() };
        for i in 0..100 { assert!(raw.claim(i)); }
        let len = t.len();
        let (freed, last) = unsafe { raw.sweep_range(0..len, false, |_| {}) };
        t.swept(freed, last, false);
        assert!(!raw.holds(&t), "swept with every entry kept");
    }
}
