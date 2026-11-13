//! A heap table: the entries a Value indexes and, beside them, one colour
//! byte each, which is both the allocator's map and the collector's marks
//! (design note gc-smooth.md 4.1). Entries enter only through this type, so
//! occupancy is exact by construction, and they do not move but in a
//! compaction.
//!
//! A colour is FREE, QUARANTINED (under verify), or in use with bits beside
//! it. MARK is a collection's: marking sets it, after a completed mark the dead
//! are the entries in use without it, and `sweep' frees those and clears it.
//! So after a finished collection a table holds no marks; one a key press or a
//! panic ended keeps them, unread, until the next mark clears them (`raw'),
//! or a settle does.
//!
//! OLD and DIRTY serve the generations (section 4.16 of the design note). A
//! sweep leaves each survivor exactly OLD and clean; an entry allocated since is
//! young. Every store into an entry sets its DIRTY, with no test of the entry
//! or the value: the bit means "written since the last collection", and only
//! on an old entry does a collection read it, by reading the entry again. A
//! put into a strong hash table, which is costly to read again and written
//! often, instead logs the key and the value (`Barrier`). Marks come and go
//! without touching either bit.
use crate::Value;
use crate::data::Object;
use crate::hash::HashTable;
use std::ops::Index;
use std::sync::atomic::{AtomicU8, Ordering};

const FREE: u8 = 0;
const IN_USE: u8 = 0x01;
/// This collection's mark reached the entry. The sign bit, so that "in use
/// and not yet reached", a mark's question of every reference it follows, is
/// one test: the colour, as a signed byte, is positive.
const MARK: u8 = 0x80;
/// Free under verify, never to be reused: not in use, and never positive.
const QUARANTINED: u8 = 0x84;
/// The entry was stored into since the last collection.
const DIRTY: u8 = 0x08;
/// The entry survived a collection.
const OLD: u8 = 0x10;
const LOW_BITS: u64 = 0x0101_0101_0101_0101;
const HIGH_BITS: u64 = 0x8080_8080_8080_8080;

/// Bytes of entries past which a table stops copying to grow.
const RESERVE_FROM: usize = 1 << 20;
/// The address space such a table takes, once. Pages are committed as they
/// are touched; a table past this copies to grow, as any `Vec`.
const RESERVE_BYTES: usize = 8 << 30;

/// Values a table's log holds before it gives up: only a native loop that
/// never polls puts so many between two collections.
const LOG_GIVES_UP: usize = 1 << 19;

/// Whether an entry of colour COLOR is in use and no mark has reached it.
/// STICKY is `OLD` in a minor collection's mark, to which every old entry
/// counts as reached already, and 0 in any other.
#[inline]
fn unreached(color: u8, sticky: u8) -> bool {
    color & (IN_USE | MARK | sticky) == IN_USE
}

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
    /// `OLD` for a minor's mark, 0 for any other (`unreached`).
    sticky: u8,
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
        // Of the threads that saw it unreached, one finds the bit clear.
        unreached(color.load(Ordering::Relaxed), self.sticky) && color.fetch_or(MARK, Ordering::Relaxed) & MARK == 0
    }
    pub(crate) fn is_free(&self, i: usize) -> bool {
        self.color(i).load(Ordering::Relaxed) & IN_USE == 0
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
    /// every survivor exactly old. The entries freed, and the end of the last
    /// still in use. SAFETY: RANGE is this caller's alone, and starts on a
    /// multiple of 8.
    pub(crate) unsafe fn sweep_range(&self, range: std::ops::Range<usize>, quarantine: bool, mut destroy: impl FnMut(&mut T)) -> (usize, usize) {
        assert!(range.end <= self.len, "a range past its table");
        debug_assert!(range.start % 8 == 0);
        let dead = if quarantine { QUARANTINED } else { FREE };
        // Only this range: other threads hold the ones beside it.
        let base = range.start;
        // SAFETY: RANGE lies in the colours and the entries (asserted: both
        // are `len` long) and is this caller's alone (this function's
        // contract); AtomicU8 has the layout of u8. No mark runs, so nothing
        // else reads an entry of the range while this lives.
        let (colors, entries) = unsafe {
            (std::slice::from_raw_parts_mut((self.colors as *mut u8).add(base), range.len()), std::slice::from_raw_parts_mut(self.entries.add(base), range.len()))
        };
        let (mut freed, mut last) = (0, 0);
        let mut at = 0;
        // OLD's bit of each byte, moved down beside its in-use bit, counts
        // as a mark in a minor's sweep.
        let old_too = if self.sticky == 0 { 0 } else { LOW_BITS };
        while at < colors.len() {
            let end = (at + 8).min(colors.len());
            // Eight entries with none dead, by far the commonest case when
            // the heap is mostly live: make the ones in use old in one store,
            // or in none when they are so already, as the old are to a minor.
            if let Ok(bytes) = <[u8; 8]>::try_from(&colors[at..end]) {
                let word = u64::from_le_bytes(bytes);
                let in_use = word & LOW_BITS;
                let reached = word >> 7 | (word >> 4 & old_too);
                if in_use & !reached == 0 {
                    // Each byte in use becomes OLD | IN_USE; the rest stay.
                    let clean = (word & !(in_use * 0xFF)) | (in_use * u64::from(OLD | IN_USE));
                    if clean != word { colors[at..end].copy_from_slice(&clean.to_le_bytes()); }
                    if clean != 0 {
                        last = base + end - clean.leading_zeros() as usize / 8;
                    }
                    at = end;
                    continue;
                }
            }
            for i in at..end {
                let color = colors[i];
                if unreached(color, self.sticky) {
                    destroy(&mut entries[i]);
                    colors[i] = dead;
                    freed += 1;
                    if quarantine { last = base + i + 1; }
                } else if color != FREE {
                    if color & IN_USE != 0 { colors[i] = OLD | IN_USE; }
                    last = base + i + 1;
                }
            }
            at = end;
        }
        (freed, last)
    }
    /// The dead entries of RANGE that WANTED, given each index, accepts.
    pub(crate) fn dead_in(&self, range: std::ops::Range<usize>, wanted: impl Fn(usize) -> bool, found: &mut Vec<u32>) {
        assert!(range.end <= self.len, "a range past its table");
        for i in range {
            if unreached(self.color(i).load(Ordering::Relaxed), self.sticky) && wanted(i) {
                found.push(i as u32);
            }
        }
    }
    /// Entry I, which any thread may read. SAFETY: nothing writes it for as
    /// long as this lives.
    #[inline(always)]
    pub(crate) unsafe fn get(&self, i: usize) -> &T
    where
        T: Sync,
    {
        // SAFETY: this function's contract, and T: Sync lets any thread read it.
        unsafe { self.get_unsync(i) }
    }
    /// Entry I, for a view that reads no part of it a `Cell' or a lock guards.
    /// SAFETY: as `get', and the caller reads only such parts.
    #[inline(always)]
    pub(crate) unsafe fn get_unsync(&self, i: usize) -> &T {
        assert!(i < self.len, "a Value past its table");
        // SAFETY: in bounds, and nothing writes it while this lives (the contract).
        unsafe { &*self.entries.add(i) }
    }
    /// Entry I, the caller's alone. SAFETY: the caller claimed it, and nothing
    /// else reaches it for as long as this lives.
    #[allow(clippy::mut_from_ref)]
    #[inline(always)]
    pub(crate) unsafe fn get_mut(&self, i: usize) -> &mut T {
        assert!(i < self.len, "a Value past its table");
        // SAFETY: in bounds, and only the caller reaches it while this lives (the contract).
        unsafe { &mut *self.entries.add(i) }
    }
}

/// The keys and values put into a table's old strong hash tables since the
/// last collection. Apart from the entries, so a `Stores` can hold it beside
/// a `&mut HashTable`.
#[derive(Default)]
struct Barrier {
    values: Vec<Value>,
    /// The log overflowed and was dropped: it records nothing until the next
    /// collection, which must not trust the generations.
    lost: bool,
}
romacs_snapshot::snapshot_struct!(Barrier { values, lost });

/// Whether V can name an entry of a table: an object, a cons, a closure or a
/// cell, by its physical tag, and neither nil nor t, which share two of them.
#[inline]
fn names_entry(v: Value) -> bool {
    (0b0001_1101u64 >> (v.0 & 7)) & 1 != 0 && v.0 > Value::TRUE.0
}

impl Barrier {
    /// NEW goes into a strong hash table coloured COLOR.
    #[inline(always)]
    fn stored(&mut self, color: u8, new: Value) {
        if names_entry(new) && color & OLD != 0 {
            on_store(self, new);
        }
    }
}

/// NEW, which names an entry, was stored in an old one. Only a reference
/// into the private heap can name a young entry: frozen and image values are
/// never collected by parts.
#[cold]
#[inline(never)]
fn on_store(barrier: &mut Barrier, new: Value) {
    if barrier.lost || crate::Vm::image_value(new) { return; }
    barrier.values.push(new);
    if barrier.values.len() >= LOG_GIVES_UP {
        *barrier = Barrier { lost: true, ..Barrier::default() };
    }
}

/// Why `Table::update_element` stored nothing. Holds no Value: each caller
/// already has the offending one and builds its own fault from it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum SlotFault {
    NotSlots,
    OutOfRange,
}

/// The barrier of the object a `HashTable` lives in, for that table's writing
/// methods to tell each key and value they put in.
pub(crate) struct Stores<'t> {
    barrier: &'t mut Barrier,
    color: &'t mut u8,
    /// A weak table's entries are no strong edges: the table is dirtied, for
    /// the next collection's weak rounds.
    weak: bool,
}

impl Stores<'_> {
    #[inline]
    pub(crate) fn stored(&mut self, new: Value) {
        if self.weak {
            *self.color |= DIRTY;
        } else {
            self.barrier.stored(*self.color, new);
        }
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
    /// `OLD` while a minor's mark stands, to which the old count as
    /// reached (`unreached`), and 0 otherwise.
    sticky: u8,
    /// Which marks the colours hold: a new mark, or marks cleared, is another.
    generation: u64,
    barrier: Barrier,
    /// Free entries `alloc' has handed out again, for tests.
    #[cfg(test)]
    pub(crate) reused: usize,
}

impl<T> Default for Table<T> {
    fn default() -> Self {
        Self {
            entries: Vec::new(), colors: Vec::new(), cursor: 0, free: 0, quarantined: 0, reserved: false, high: 0, marking: false, sticky: 0, generation: 0, barrier: Barrier::default(),
            #[cfg(test)]
            reused: 0,
        }
    }
}

// SAFETY: as its fields, each SnapshotSafe; none addresses anything outside
// the table's own storage.
unsafe impl<T: romacs_snapshot::SnapshotSafe> romacs_snapshot::SnapshotSafe for Table<T> {}

impl<T> Index<usize> for Table<T> {
    type Output = T;
    #[inline]
    fn index(&self, i: usize) -> &T {
        &self.entries[i]
    }
}

impl<'a, T> IntoIterator for &'a Table<T> {
    type Item = &'a T;
    type IntoIter = std::slice::Iter<'a, T>;
    fn into_iter(self) -> Self::IntoIter {
        self.entries.iter()
    }
}

impl Table<(Value, Value)> {
    #[inline]
    pub(crate) fn set_car(&mut self, i: usize, new: Value) {
        let (entry, color, _) = self.store_at(i);
        *color |= DIRTY;
        entry.0 = new;
    }
    #[inline]
    pub(crate) fn set_cdr(&mut self, i: usize, new: Value) {
        let (entry, color, _) = self.store_at(i);
        *color |= DIRTY;
        entry.1 = new;
    }
}

impl Table<Value> {
    #[inline]
    pub(crate) fn set(&mut self, i: usize, new: Value) {
        let (entry, color, _) = self.store_at(i);
        *color |= DIRTY;
        *entry = new;
    }
}

impl Table<Object> {
    /// Store NEW in slot K of the vector or record at entry I.
    #[inline]
    pub(crate) fn update_element(&mut self, i: usize, k: usize, new: Value) -> Result<(), SlotFault> {
        let (entry, color, _) = self.store_at(i);
        let (Object::Vector(items) | Object::Record(items)) = entry else {
            return Err(SlotFault::NotSlots);
        };
        let slot = items.get_mut(k).ok_or(SlotFault::OutOfRange)?;
        *color |= DIRTY;
        *slot = new;
        Ok(())
    }
    /// The hash table at entry I, if it is one, and what its stores tell. Not
    /// `entry_mut`: a whole-entry hook would re-trace the table on every put.
    #[inline]
    pub(crate) fn hash_mut(&mut self, i: usize) -> Option<(&mut HashTable, Stores<'_>)> {
        let (entry, color, barrier) = self.store_at(i);
        let Object::HashTable(table) = entry else { return None };
        let weak = table.weakness != crate::hash::Weakness::None;
        Some((&mut **table, Stores { barrier, color, weak }))
    }
    /// A put of VALUE into the hash table at entry I, as far as the barrier
    /// knows, for a test elsewhere of what empties the log.
    #[cfg(test)]
    pub(crate) fn log_a_put(&mut self, i: usize, value: Value) {
        self.hash_mut(i).expect("a hash table").1.stored(value);
    }
    pub(crate) fn swap(&mut self, a: usize, b: usize) {
        *self.store_at(a).1 |= DIRTY;
        *self.store_at(b).1 |= DIRTY;
        self.entries.swap(a, b);
    }
    /// Entry TARGET to change, beside entry SOURCE to read, a different one.
    pub(crate) fn entry_mut_with(&mut self, target: usize, source: usize) -> (&mut Object, &Object) {
        *self.store_at(target).1 |= DIRTY;
        if source < target {
            let (low, high) = self.entries.split_at_mut(target);
            (&mut high[0], &low[source])
        } else {
            let (low, high) = self.entries.split_at_mut(source);
            (&mut low[target], &high[0])
        }
    }
}

/// The kinds of entry a `&mut` may be had to. Not conses or cells, whose few
/// stores `set_car`, `set_cdr` and `set` name.
pub(crate) trait WholeEntry {}
impl WholeEntry for Object {}
impl WholeEntry for crate::Closure {}

impl<T: WholeEntry> Table<T> {
    /// Entry I, to change anywhere. The borrow keeps everything else off this
    /// table while it lives, and nothing runs Lisp without the VM it is part of.
    #[inline]
    pub(crate) fn entry_mut(&mut self, i: usize) -> &mut T {
        let (entry, color, _) = self.store_at(i);
        *color |= DIRTY;
        entry
    }
    /// As `entry_mut`, for an I that may be past the table.
    #[inline]
    pub(crate) fn get_entry_mut(&mut self, i: usize) -> Option<&mut T> {
        if i >= self.entries.len() {
            return None;
        }
        Some(self.entry_mut(i))
    }
}

impl<T> Table<T> {
    /// Entry I, its colour and the log, borrowed apart: what a store needs,
    /// for one test of I.
    #[inline(always)]
    fn store_at(&mut self, i: usize) -> (&mut T, &mut u8, &mut Barrier) {
        debug_assert!(self.colors.len() >= self.entries.len(), "a colour for every entry");
        let entry = &mut self.entries[i];
        // SAFETY: I is within `entries` (just indexed), and `colors` is never
        // the shorter, at any instruction, so that a panic part way leaves it
        // so: `push` and `collector_prepend` lengthen the colours first, `cut`
        // and `collector_compact` shorten the entries first, and `dense`
        // makes them equal. Nothing else changes either length.
        let color = unsafe { self.colors.get_unchecked_mut(i) };
        (entry, color, &mut self.barrier)
    }
    #[inline]
    pub(crate) fn len(&self) -> usize {
        self.entries.len()
    }
    pub(crate) fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }
    #[inline]
    pub(crate) fn get(&self, i: usize) -> Option<&T> {
        self.entries.get(i)
    }
    pub(crate) fn iter(&self) -> std::slice::Iter<'_, T> {
        self.entries.iter()
    }
    /// Every entry, past the barrier: the collector's own writes, which only
    /// `gc*.rs` and this file make (`tests/no_remote_references.rs`).
    pub(crate) fn collector_entries_mut(&mut self) -> &mut [T] {
        &mut self.entries
    }
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
        self.colors[i] = IN_USE;
        self.entries[i] = value;
        i
    }
    /// A new entry at the end, whatever is free below it. Its index.
    #[inline]
    pub(crate) fn push(&mut self, value: T) -> usize {
        if self.entries.len() == self.entries.capacity() {
            self.grow();
        }
        // The colour first: no entry is ever without one (`store_at`).
        self.colors.push(IN_USE);
        self.entries.push(value);
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
        self.colors[i] & IN_USE == 0
    }
    /// The table for a mark's threads, what the last mark left if a key press
    /// or a panic ended it gone: it would stop this one at entries it marked.
    /// SAFETY: until the last use of what this returns, the table is not
    /// dropped, and changes only through it; `RawTable::holds' checks that.
    pub(crate) unsafe fn raw(&mut self, minor: bool) -> RawTable<T> {
        if self.marking {
            self.clear_marks();
        }
        self.marking = true;
        self.sticky = if minor { OLD } else { 0 };
        self.generation += 1;
        RawTable {
            entries: self.entries.as_mut_ptr(), colors: self.colors.as_mut_ptr().cast(), len: self.entries.len(),
            free: self.free, quarantined: self.quarantined, generation: self.generation, sticky: self.sticky,
        }
    }
    /// Mark entry I, as `RawTable::claim'.
    #[cfg(test)]
    pub(crate) fn mark(&mut self, i: usize) -> bool {
        let color = &mut self.colors[i];
        if !unreached(*color, self.sticky) {
            return false;
        }
        *color |= MARK;
        true
    }
    /// Whether a mark in progress has reached entry I.
    #[inline(always)]
    pub(crate) fn marked(&self, i: usize) -> bool {
        let color = self.colors[i];
        color & IN_USE != 0 && color & (MARK | self.sticky) != 0
    }
    /// Every mark gone, and the free entries counted again: a collection
    /// stopped short. A mark changes neither the generations nor the log, so
    /// one stopped while it marked leaves both as they were. One stopped
    /// part way through its sweep has not: a survivor in a range it swept is
    /// old and clean, and may name a young entry in a range it did not reach.
    /// A VM used again after that promotes everything first
    /// (`Vm::restore_nesting`).
    pub(crate) fn clear_marks(&mut self) {
        let (mut free, mut quarantined) = (0, 0);
        for color in &mut self.colors {
            if *color & IN_USE != 0 { *color &= !MARK; }
            free += usize::from(*color == FREE);
            quarantined += usize::from(*color == QUARANTINED);
        }
        (self.free, self.quarantined, self.cursor, self.marking, self.sticky) = (free, quarantined, 0, false, 0);
        self.generation += 1;
    }
    /// After a completed mark: every dead entry through DESTROY, which leaves
    /// a tombstone owning nothing, then free (under QUARANTINE, never reused
    /// and never cut), every mark gone, and the dead tail cut. Entries freed.
    #[cfg(test)]
    pub(crate) fn sweep(&mut self, quarantine: bool, destroy: impl FnMut(&mut T)) -> usize {
        let raw = tests::raw_of(self);
        let (freed, last) = tests::sweep_whole(&raw, quarantine, destroy);
        self.swept(freed, last, quarantine);
        freed
    }
    /// The books of a sweep in ranges (`RawTable::sweep_range'): FREED entries
    /// in all, the last in use before LAST.
    pub(crate) fn swept(&mut self, freed: usize, last: usize, quarantine: bool) {
        if quarantine { self.quarantined += freed } else { self.free += freed }
        (self.cursor, self.marking, self.sticky) = (0, false, 0);
        self.generation += 1;
        // Every survivor is old and clean, so nothing logged still stands.
        self.barrier = Barrier::default();
        self.cut(last);
    }
    /// Drop the entries from LEN on, all free tombstones.
    fn cut(&mut self, len: usize) {
        self.high = self.high.max(self.entries.len());
        self.free -= self.entries.len() - len;
        self.entries.truncate(len);
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
    pub(crate) fn collector_compact(&mut self, mut keep: impl FnMut(&mut T)) {
        self.high = self.high.max(self.entries.len());
        let (colors, mut i) = (&self.colors, 0);
        self.entries.retain_mut(|entry| {
            let live = colors[i] & (IN_USE | MARK) == (IN_USE | MARK);
            i += 1;
            if live {
                keep(entry);
            }
            live
        });
        self.dense();
    }
    /// N fillers in front of every entry, which move up past them.
    pub(crate) fn collector_prepend(&mut self, n: usize, filler: impl FnMut() -> T) {
        // Colours for them first, whatever FILLER does (`store_at`).
        self.colors.resize(self.entries.len() + n, FREE);
        self.entries.splice(0..0, std::iter::repeat_with(filler).take(n));
        self.dense();
    }
    /// Every entry in use, and old: what a compaction leaves are survivors.
    fn dense(&mut self) {
        // Never shorter than the entries on the way (`store_at`).
        self.colors.resize(self.entries.len(), OLD | IN_USE);
        self.colors.fill(OLD | IN_USE);
        (self.cursor, self.free, self.quarantined, self.marking) = (self.entries.len(), 0, 0, false);
        self.generation += 1;
        self.barrier = Barrier::default();
    }
    /// A minor's start. To its mark every old entry counts as reached
    /// (`raw`), but those it must read again: the ones stored into since
    /// the last collection, and the ones WHOLE asks for whatever their
    /// colour, given its index. Each of those is made young for this
    /// collection and returned, for the mark to claim and read as it does
    /// the young; being roots, all survive, and the sweep leaves them old
    /// again. No other colour is written. A minor that ends any other way
    /// promotes everything (`Vm::promote_all`), which ages them again.
    pub(crate) fn read_again(&mut self, whole: impl Fn(usize, &T) -> bool) -> Vec<usize> {
        let mut again = Vec::new();
        for (i, (color, entry)) in self.colors.iter_mut().zip(&self.entries).enumerate() {
            if *color & (IN_USE | OLD) != (IN_USE | OLD) { continue; }
            if *color & DIRTY != 0 || whole(i, entry) {
                *color &= !OLD;
                again.push(i);
            }
        }
        again
    }
    /// Every entry in use becomes old, as a sweep would leave it, and the log
    /// is empty: what a world image holds, whose every entry is old to the
    /// process that restores it.
    pub(crate) fn settle_old(&mut self) {
        // What a stopped collection left first: its marks, and free entries
        // its sweep never counted.
        if self.marking {
            self.clear_marks();
        }
        for color in &mut self.colors {
            if *color & IN_USE != 0 { *color = OLD | IN_USE; }
        }
        self.sticky = 0;
        self.barrier = Barrier::default();
    }
    /// Whether every entry in use is exactly old and the log is empty: what a
    /// completed collection leaves, and what an image may hold.
    pub(crate) fn settled(&self) -> bool {
        self.barrier.values.is_empty() && !self.barrier.lost && self.colors.iter().all(|&color| matches!(color, FREE | QUARANTINED) || color == (OLD | IN_USE))
    }
    /// The keys and values put into old strong hash tables since the last
    /// collection.
    pub(crate) fn logged(&self) -> &[Value] {
        &self.barrier.values
    }
    /// The old entries stored into since the last collection.
    #[cfg(test)]
    pub(crate) fn dirty_old(&self) -> impl Iterator<Item = usize> + '_ {
        self.colors.iter().enumerate().filter(|&(_, &color)| color & (OLD | DIRTY) == (OLD | DIRTY)).map(|(i, _)| i)
    }
    /// The log overflows, as a native loop of puts with no poll makes it.
    #[cfg(test)]
    pub(crate) fn lose_the_log(&mut self) {
        self.barrier.lost = true;
    }
    /// Whether the log overflowed since the last collection, and is void.
    pub(crate) fn log_lost(&self) -> bool {
        self.barrier.lost
    }
    /// Whether entry I survived a collection.
    pub(crate) fn old(&self, i: usize) -> bool {
        self.colors[i] & OLD != 0
    }
    /// Whether entry I was stored into since the last collection.
    pub(crate) fn stored_into(&self, i: usize) -> bool {
        self.colors[i] & DIRTY != 0
    }
    /// Whether each entry is young and unmarked: what a minor's sweep frees.
    pub(crate) fn young_unmarked(&self) -> impl Iterator<Item = bool> + '_ {
        self.colors.iter().map(|&color| color & (IN_USE | OLD | MARK) == IN_USE)
    }
    /// Whether each entry holds an object: in use, whatever else its colour says.
    pub(crate) fn held(&self) -> Vec<bool> {
        self.colors.iter().map(|&color| color & IN_USE != 0).collect()
    }
    /// The colours as they are, for the serial re-mark to compare.
    pub(crate) fn colors(&self) -> Vec<u8> {
        self.colors.clone()
    }
    /// Whether a mark in progress reached each entry.
    pub(crate) fn marks(&self) -> impl Iterator<Item = bool> + '_ {
        self.colors.iter().map(|&color| color & IN_USE != 0 && color & (MARK | self.sticky) != 0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The table for a mark, as these tests use it.
    pub(super) fn raw_of<T>(t: &mut Table<T>) -> RawTable<T> {
        // SAFETY: each test drops its table only after the last use of this,
        // and whatever else changes the table is what `holds' is to notice.
        unsafe { t.raw(false) }
    }
    /// A sweep of all RAW sees, on the test's one thread.
    pub(super) fn sweep_whole<T>(raw: &RawTable<T>, quarantine: bool, destroy: impl FnMut(&mut T)) -> (usize, usize) {
        // SAFETY: the whole table is this thread's, and the range starts at 0.
        unsafe { raw.sweep_range(0..raw.len(), quarantine, destroy) }
    }
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
        t.collector_compact(|entry| *entry += 100);
        assert_eq!(t.entries, [102, 105, 107]);
        assert_eq!((t.unused(), t.live()), (0, 3));
        t.collector_prepend(2, || 0);
        assert_eq!(t.entries, [0, 0, 102, 105, 107]);
        assert_eq!(t.alloc(1), 5, "dense: the end");
    }

    fn conses(n: usize) -> Table<(Value, Value)> {
        let mut table = Table::default();
        table.extend((0..n).map(|_| (Value::NIL, Value::NIL)));
        table
    }
    /// Every entry survives a sweep: all are old from here.
    fn age<T>(table: &mut Table<T>) {
        for i in 0..table.len() { table.mark(i); }
        assert_eq!(table.sweep(false, |_| {}), 0);
    }

    /// A sweep leaves its survivors old, what is allocated after it is young
    /// until the next, and an old entry no mark reaches dies like any other.
    #[test]
    fn a_survivor_is_old_and_a_new_entry_young() {
        let mut t = table(20);
        assert!((0..20).all(|i| !t.old(i)), "never collected");
        assert!(!t.settled());
        sweep(&mut t, &[0, 1, 2, 12]);
        assert!([0, 1, 2, 12].iter().all(|&i| t.old(i)));
        assert!(t.settled());
        let young = t.alloc(7);
        assert!(!t.old(young) && !t.settled());
        assert_eq!(sweep(&mut t, &[0, young]), 3, "old and unreached is dead");
        assert!(t.old(young) && t.settled());
        // A mark's view claims old and young alike, once.
        let fresh = t.alloc(8);
        let raw = raw_of(&mut t);
        assert!(raw.claim(0) && !raw.claim(0) && raw.claim(fresh) && !raw.claim(fresh));
        assert!(!raw.claim(2), "a free entry");
    }

    fn dirty<T>(table: &Table<T>) -> Vec<usize> { table.dirty_old().collect() }

    /// An abandoned mark leaves the generations as they were.
    #[test]
    fn an_abandoned_mark_keeps_the_generations_and_what_was_written() {
        let mut t = conses(3);
        age(&mut t);
        let young = Value::reference(t.alloc((Value::NIL, Value::NIL)), 3);
        t.set_car(0, young);
        assert!(t.mark(0) && t.mark(1));
        t.clear_marks();
        assert!(t.old(0) && t.old(1) && t.old(2) && !t.old(3));
        assert_eq!(dirty(&t), [0]);
    }

    /// A store into an old cons leaves it dirty, whatever is stored; a young
    /// cons written is not an old one written. A sweep leaves every survivor
    /// clean.
    #[test]
    fn a_store_into_an_old_cons_leaves_it_dirty() {
        let mut t = conses(4);
        t.set_car(0, Value::reference(1, 3));
        assert!(dirty(&t).is_empty(), "a young holder");
        age(&mut t);
        assert!(dirty(&t).is_empty(), "and a sweep cleans what was written before it");
        let young = t.alloc((Value::NIL, Value::NIL));
        let (cons, object) = (Value::reference(young, 3), Value::reference(9, 0));
        t.set_car(0, cons);
        t.set_cdr(0, object);
        t.set_cdr(2, Value::integer(7).unwrap());
        t.set_cdr(young, cons);
        assert_eq!(dirty(&t), [0, 2]);
        assert!(t.logged().is_empty() && !t.settled());
        assert_eq!((t[0], t[2].1, t[young].1), ((cons, object), Value::integer(7).unwrap(), cons), "and the stores happen");
        age(&mut t);
        assert!(dirty(&t).is_empty() && t.settled());
        assert!(t.old(young), "the young cons written survived, old and clean");
    }

    /// Each of a cons's two stores dirties it.
    #[test]
    fn either_half_of_a_cons_written_dirties_it() {
        let mut t = conses(2);
        age(&mut t);
        t.set_car(0, Value::TRUE);
        assert_eq!(dirty(&t), [0]);
        t.set_cdr(1, Value::TRUE);
        assert_eq!(dirty(&t), [0, 1]);
    }

    /// The sweep's word at a time agrees with its entry at a time, whatever
    /// the colours beside the mark: a word whose entries all survive leaves
    /// each exactly old, and its free and quarantined entries as they were.
    #[test]
    fn a_word_of_survivors_of_every_colour_is_swept_to_old() {
        let mut t = conses(40);
        // Quarantine 3 and 12, free nothing else.
        for i in (0..40).filter(|i| ![3, 12].contains(i)) { t.mark(i); }
        assert_eq!(t.sweep(true, |_| {}), 2);
        // Old and dirty, old and clean, young and dirty, young and clean.
        let young: Vec<usize> = (0..8).map(|_| t.alloc((Value::NIL, Value::NIL))).collect();
        assert_eq!(young[0], 40, "quarantined entries are not reused");
        for i in [0, 5, 9, 17, 33, young[1], young[6]] { t.set_car(i, Value::TRUE); }
        for i in (0..48).filter(|i| ![3, 12].contains(i)) { assert!(t.mark(i), "{i}"); }
        assert!(!t.mark(3) && !t.mark(12), "nor ever marked");
        assert_eq!(t.sweep(true, |_| panic!("nothing is dead")), 0);
        assert!(t.settled() && dirty(&t).is_empty());
        assert_eq!((t.len(), t.unused(), t.live()), (48, 2, 46));
        assert!((0..48).all(|i| t.old(i) != [3, 12].contains(&i)) && t.is_free(3) && t.is_free(12));
        // And a word with one dead entry among them, by the other path.
        for i in (0..48).filter(|i| ![3, 12, 20].contains(i)) { t.mark(i); }
        assert_eq!(t.sweep(false, |_| {}), 1);
        assert!(t.settled() && t.is_free(20));
        // And a free entry in a word of survivors stays free.
        t.set_cdr(21, Value::TRUE);
        for i in (0..48).filter(|i| ![3, 12, 20].contains(i)) { assert!(t.mark(i)); }
        assert_eq!(t.sweep(false, |_| panic!("nothing is dead")), 0);
        assert!(t.settled() && t.is_free(20) && dirty(&t).is_empty());
        assert_eq!((t.unused(), t.alloc((Value::NIL, Value::NIL))), (3, 20));
    }

    /// One word holding every colour a sweep can meet with all in use
    /// surviving: free, quarantined, old and young, clean and dirty. Each in
    /// use ends exactly old, the other two as they were, nothing is freed.
    #[test]
    fn one_word_of_every_colour_sweeps_to_exactly_old() {
        let mut t = conses(6);
        for i in [0, 1, 3, 4, 5] { t.mark(i); }
        assert_eq!(t.sweep(true, |_| {}), 1, "2 is quarantined");
        for i in [0, 1, 4, 5] { t.mark(i); }
        assert_eq!(t.sweep(false, |_| {}), 1, "3 is free");
        t.push((Value::NIL, Value::NIL));
        t.push((Value::NIL, Value::NIL));
        for i in [1, 5, 7] { t.set_car(i, Value::TRUE); }
        assert_eq!(t.colors, [OLD | IN_USE, OLD | DIRTY | IN_USE, QUARANTINED, FREE, OLD | IN_USE, OLD | DIRTY | IN_USE, IN_USE, DIRTY | IN_USE]);
        for i in [0, 1, 4, 5, 6, 7] { assert!(t.mark(i)); }
        assert!(!t.mark(2) && !t.mark(3));
        assert_eq!(t.sweep(false, |_| panic!("nothing is dead")), 0);
        let old = OLD | IN_USE;
        assert_eq!(t.colors, [old, old, QUARANTINED, FREE, old, old, old, old]);
        assert_eq!((t.len(), t.unused(), t.live()), (8, 2, 6));
        assert_eq!(t.alloc((Value::NIL, Value::NIL)), 3, "the free entry, never the quarantined one");
    }

    /// A filler that panics leaves no entry without a colour: a store into
    /// what it had put in stays in bounds.
    #[test]
    fn a_panic_while_prepending_leaves_a_colour_for_every_entry() {
        let mut t = conses(3);
        let mut made = 0;
        let caught = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            t.collector_prepend(4, || {
                made += 1;
                assert!(made < 3, "a filler's panic");
                (Value::NIL, Value::NIL)
            });
        }));
        assert!(caught.is_err());
        assert!(t.colors.len() >= t.entries.len());
        // Memory safety is all that is left to ask of this table: its
        // colours no longer describe its entries.
        for i in 0..t.entries.len() { t.set_car(i, Value::TRUE); }
    }

    /// Settling a table a stopped collection left: its marks go, the free
    /// entries its sweep never counted are counted, and a view of the table
    /// from before no longer holds.
    #[test]
    fn settling_after_a_stopped_collection_recounts_and_unmarks() {
        let mut t = table(16);
        let raw = raw_of(&mut t);
        for i in (0..16).filter(|i| i % 2 == 0) { assert!(raw.claim(i)); }
        // The sweep, and no `swept': a collection stopped after its sweep.
        assert_eq!(sweep_whole(&raw, false, |entry| *entry = u64::MAX).0, 8);
        assert_eq!(t.unused(), 0, "nothing counted yet");
        t.settle_old();
        assert!(t.settled() && t.marks().all(|marked| !marked));
        assert_eq!((t.unused(), t.live()), (8, 8));
        assert!(!raw.holds(&t));
        assert_eq!(t.alloc(7), 1, "and a counted free entry is reused");
        // Stopped while it marked: the marks alone go.
        let mut t = table(8);
        for i in 0..4 { t.mark(i); }
        let raw = raw_of(&mut t);
        assert!(raw.claim(5));
        t.settle_old();
        assert!(t.settled() && t.marks().all(|marked| !marked) && !raw.holds(&t));
        assert_eq!(t.live(), 8);
    }

    /// A cell and a vector slot are dirtied as a cons is, and a store that
    /// fails writes nothing.
    #[test]
    fn a_cell_and_a_vector_written_are_dirty() {
        let mut cells: Table<Value> = Table::default();
        cells.push(Value::NIL);
        cells.push(Value::NIL);
        age(&mut cells);
        let young = Value::reference(3, 3);
        cells.set(1, young);
        assert_eq!(dirty(&cells), [1]);
        let mut objects: Table<Object> = Table::default();
        objects.push(Object::Vector(vec![Value::NIL; 2]));
        objects.push(Object::Float(1.0));
        objects.push(Object::Record(vec![Value::NIL; 40]));
        objects.push(Object::Vector(vec![Value::NIL; 2]));
        age(&mut objects);
        assert_eq!(objects.update_element(0, 2, young), Err(SlotFault::OutOfRange), "which writes nothing");
        assert_eq!(objects.update_element(1, 0, young), Err(SlotFault::NotSlots));
        assert!(dirty(&objects).is_empty());
        assert_eq!(objects.update_element(0, 1, young), Ok(()));
        assert_eq!(objects.update_element(2, 39, Value::TRUE), Ok(()));
        assert_eq!(dirty(&objects), [0, 2]);
        assert!(objects.logged().is_empty(), "a vector logs no value");
    }

    /// An old entry handed out whole is dirty; a young one is not an old one.
    #[test]
    fn an_old_entry_handed_out_whole_is_dirty() {
        let mut t: Table<Object> = Table::default();
        for _ in 0..5 { t.push(Object::Float(0.0)); }
        *t.entry_mut(0) = Object::Float(1.0);
        age(&mut t);
        let young = t.alloc(Object::Float(2.0));
        *t.entry_mut(1) = Object::Float(3.0);
        assert!(t.get_entry_mut(2).is_some() && t.get_entry_mut(9).is_none());
        *t.entry_mut(young) = Object::Float(4.0);
        assert_eq!(dirty(&t), [1, 2]);
        t.swap(0, 3);
        let _ = t.entry_mut_with(4, young);
        assert_eq!(dirty(&t), [0, 1, 2, 3, 4]);
        age(&mut t);
        assert!(dirty(&t).is_empty() && t.settled());
    }

    /// A strong table's puts log the keys and values; a weak table's entries
    /// are no strong edges, so its puts dirty the table.
    #[test]
    fn a_hash_put_logs_its_values_or_dirties_its_weak_table() {
        use crate::hash::{HashTable, Test, Weakness};
        let mut t: Table<Object> = Table::default();
        t.push(Object::HashTable(Box::new(HashTable::new(Test::Eq, Weakness::None, 8))));
        t.push(Object::HashTable(Box::new(HashTable::new(Test::Eq, Weakness::Key, 8))));
        t.push(Object::Float(0.0));
        assert!(t.hash_mut(2).is_none());
        let (key, value) = (Value::reference(5, 3), Value::reference(6, 3));
        t.hash_mut(0).unwrap().1.stored(key);
        assert!(t.logged().is_empty(), "a young table");
        age(&mut t);
        for i in 0..2 {
            let (_, mut stores) = t.hash_mut(i).unwrap();
            stores.stored(key);
            stores.stored(value);
            // Nothing that names no entry, or one no collection frees by parts.
            for inert in [Value::NIL, Value::TRUE, Value::UNBOUND, Value::integer(1).unwrap(), Value::reference(3, 7), Value::reference(crate::image::IMAGE | 3, 3)] {
                stores.stored(inert);
            }
        }
        assert_eq!(t.logged(), [key, value]);
        assert_eq!(dirty(&t), [1]);
        // An abandoned mark keeps the log, a settle and a compaction empty it.
        t.clear_marks();
        assert_eq!(t.logged(), [key, value]);
        t.settle_old();
        assert!(t.logged().is_empty() && t.settled());
        t.hash_mut(0).unwrap().1.stored(key);
        for i in 0..3 { t.mark(i); }
        t.collector_compact(|_| {});
        assert!(t.logged().is_empty() && t.settled());
    }

    /// A log that only a loop with no poll can fill gives up: it is dropped,
    /// records nothing more, and says so until a sweep begins another.
    #[test]
    fn a_log_past_its_limit_is_dropped_and_says_so() {
        use crate::hash::{HashTable, Test, Weakness};
        let young = Value::reference(3, 3);
        let mut t: Table<Object> = Table::default();
        t.push(Object::HashTable(Box::new(HashTable::new(Test::Eq, Weakness::None, 8))));
        age(&mut t);
        let put = |t: &mut Table<Object>| t.hash_mut(0).unwrap().1.stored(young);
        for _ in 0..LOG_GIVES_UP - 1 { put(&mut t); }
        assert!(!t.log_lost());
        assert_eq!(t.logged().len(), LOG_GIVES_UP - 1);
        put(&mut t);
        assert!(t.log_lost());
        put(&mut t);
        assert!(t.logged().is_empty(), "nothing more is recorded");
        assert!(!t.settled(), "a lost log is not an empty one");
        t.clear_marks();
        assert!(t.log_lost(), "nor does an abandoned mark find it again");
        age(&mut t);
        assert!(!t.log_lost() && t.settled());
        put(&mut t);
        assert_eq!(t.logged(), [young]);
    }

    /// What an image holds: every entry old, no log, whatever was young,
    /// dirty or logged before.
    #[test]
    fn settling_makes_every_entry_old_and_the_log_empty() {
        let mut t: Table<Object> = Table::default();
        for _ in 0..4 { t.push(Object::Float(0.0)); }
        sweep_objects(&mut t, &[0, 3]);
        let young = t.alloc(Object::Float(1.0));
        *t.entry_mut(0) = Object::Float(2.0);
        assert!(!t.settled());
        t.settle_old();
        assert!(t.settled() && t.old(young) && t.old(0) && t.is_free(2));
        assert_eq!(t.alloc(Object::Float(3.0)), 2, "and a free entry stays free");
    }
    fn sweep_objects(table: &mut Table<Object>, live: &[usize]) {
        for &i in live { assert!(table.mark(i)); }
        table.sweep(false, |entry| *entry = Object::Free);
    }

    /// Compaction leaves survivors: all old.
    #[test]
    fn compaction_leaves_every_entry_old() {
        let mut t = table(6);
        for i in [1, 4] { t.mark(i); }
        t.collector_compact(|_| {});
        assert!(t.settled() && t.old(0) && t.old(1));
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
        let at = t.entries.as_ptr();
        for _ in 0..(4 << 20) {
            t.push(0);
        }
        assert_eq!(t.entries.as_ptr(), at, "no copy");
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
            let raw = raw_of(&mut t);
            assert!(raw.holds(&t));
            moved(&mut t, colors);
            assert!(!raw.holds(&t), "moved {}", if colors { "colours" } else { "entries" });
        }
        let mut t = table(100);
        sweep(&mut t, &[0, 1]);
        let raw = raw_of(&mut t);
        t.alloc(7);
        assert!(!raw.holds(&t), "an entry handed out");
        let mut t = table(100);
        let raw = raw_of(&mut t);
        t.clear_marks();
        assert!(!raw.holds(&t), "its marks cleared");
        let mut t = table(100);
        let raw = raw_of(&mut t);
        let _again = raw_of(&mut t);
        assert!(!raw.holds(&t), "a mark begun again");
        // All live, so nothing about its shape changes: its marks still go.
        let mut t = table(100);
        let raw = raw_of(&mut t);
        for i in 0..100 { assert!(raw.claim(i)); }
        t.collector_compact(|_| {});
        assert!(!raw.holds(&t), "compacted with every entry kept");
        let mut t = table(100);
        let raw = raw_of(&mut t);
        for i in 0..100 { assert!(raw.claim(i)); }
        let (freed, last) = sweep_whole(&raw, false, |_| {});
        t.swept(freed, last, false);
        assert!(!raw.holds(&t), "swept with every entry kept");
    }
}
