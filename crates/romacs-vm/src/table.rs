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
//!
//! The restored world image is a third generation, which no collection frees
//! (`Vm::make_image_permanent`), kept beside the colours (`ImageSet`), so the
//! restore writes none. The image is dense and never moves: its entries are the
//! indices below its length. A permanent one is one nothing has written since
//! the restore: to a full it counts as reached, as an old entry does to a
//! minor, so a full never reads it, and never writes its colour. A remembered
//! one, in the image's bitmap, was written since, or is of a kind the mark
//! itself must see: every full reads it as a root. A store makes an image entry
//! dirty, and the next collection, of either kind, remembers it before any
//! sweep clears its DIRTY (`remember_dirty`); a put into an image strong hash
//! table, which logs instead, remembers it at once.
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
/// Entries a card covers (`Table::cards`): a multiple of eight, so that a
/// card's colours are whole words.
pub(crate) const CARD: usize = 64;

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
/// counts as reached already, and 0 in any other (to which the image's
/// untouched entries count as reached by their index, `ImageSet`).
#[inline]
fn unreached(color: u8, sticky: u8) -> bool {
    color & (IN_USE | MARK | sticky) == IN_USE
}

#[path = "table_image.rs"]
mod image_set;
use image_set::ImageSet;
#[path = "table_sweep.rs"]
mod sweep;
use sweep::Sweep;

/// What a sweep of entries did (`RawTable::sweep_range`): how many it freed,
/// the end of the last still in use, and the lowest freed (`usize::MAX` for
/// none). Sweeps of several ranges add up (`RangeSwept::and`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct RangeSwept {
    pub(crate) freed: usize,
    pub(crate) last: usize,
    pub(crate) lowest: usize,
}

impl Default for RangeSwept {
    fn default() -> Self {
        Self { freed: 0, last: 0, lowest: usize::MAX }
    }
}

impl RangeSwept {
    /// This sweep's figures and OTHER's, of ranges apart.
    pub(crate) fn and(self, other: Self) -> Self {
        Self { freed: self.freed + other.freed, last: self.last.max(other.last), lowest: self.lowest.min(other.lowest) }
    }
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
    /// The image's entries, which a full's mark counts as reached unless
    /// remembered: a copy, empty for a minor's.
    image: ImageSet,
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
        unreached(color.load(Ordering::Relaxed), self.sticky) && !self.image.permanent(i) && color.fetch_or(MARK, Ordering::Relaxed) & MARK == 0
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
    /// the VM writes between a mark and its sweep (a cell's value, a weak
    /// table pruned, the dead before a compaction made `Object::Free').
    pub(crate) fn holds(&self, table: &Table<T>) -> bool {
        std::ptr::eq(self.entries, table.entries.as_ptr()) && std::ptr::eq(self.colors.cast::<u8>(), table.colors.as_ptr())
            && self.len == table.entries.len() && self.len == table.colors.len()
            && self.free == table.free && self.quarantined == table.quarantined && self.generation == table.generation
    }
    /// A sweep of entries RANGE: each dead one through DESTROY, which leaves a
    /// tombstone owning nothing, then free (QUARANTINE: quarantined), and
    /// every survivor exactly old. The entries freed, the end of the last
    /// still in use, and the lowest freed (`usize::MAX` for none). SAFETY:
    /// RANGE is this caller's alone, and starts on a multiple of 8.
    pub(crate) unsafe fn sweep_range(&self, range: std::ops::Range<usize>, quarantine: bool, mut destroy: impl FnMut(usize, &mut T)) -> RangeSwept {
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
        // OLD's bit of each byte, moved down beside its in-use bit, counts as
        // a mark in a minor's sweep; in a full's, an untouched image entry is
        // reached by its index.
        let old_too = if self.sticky == 0 { 0 } else { LOW_BITS };
        let mut sweep = Sweep { colors, entries, base, sticky: self.sticky, old_too, dead, quarantine, destroy: &mut destroy, freed: 0, last: 0, lowest: usize::MAX };
        // The range below the image's end asks which of its entries are the
        // image's; the rest is swept as if no image were restored.
        let image_end = self.image.len.saturating_sub(base).min(range.len());
        let mut at = 0;
        while at < image_end { at = sweep.step::<true>(at, image_end, &self.image); }
        while at < range.len() { at = sweep.step::<false>(at, range.len(), &self.image); }
        RangeSwept { freed: sweep.freed, last: sweep.last, lowest: sweep.lowest }
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
#[derive(Default, romacs_snapshot::Snapshot)]
struct Barrier {
    values: Vec<Value>,
    /// The log overflowed and was dropped: it records nothing until the next
    /// collection, which must not trust the generations.
    lost: bool,
}

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
    /// An image table's word of the remembered bitmap and its bit: a put sets
    /// no DIRTY, so it is remembered here, whatever is put (a symbol of an
    /// obarray of its own, which only a full reclaims, too).
    image: Option<(&'t mut u64, u64)>,
}

impl Stores<'_> {
    #[inline]
    pub(crate) fn stored(&mut self, new: Value) {
        if self.weak {
            *self.color |= DIRTY;
        } else {
            if let Some((word, bit)) = &mut self.image { **word |= *bit; }
            self.barrier.stored(*self.color, new);
        }
    }
}

#[derive(romacs_snapshot::Snapshot)]
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
    /// A byte per `CARD` entries, set when an entry of the card is handed
    /// out or stored into, and cleared when a collection leaves every entry
    /// old and clean. Outside a set card every entry in use is exactly old
    /// and clean (image bits aside), so a minor reads and sweeps the set
    /// cards alone: what changed since the last collection, not the table.
    /// Never shorter than the colours need (`store_parts`).
    cards: Vec<u8>,
    /// The restored image's entries (`ImageSet`).
    image: ImageSet,
    /// A full's mark stands: the image's untouched entries count as reached.
    image_reached: bool,
    /// Free entries `alloc' has handed out again, for tests.
    #[cfg(test)]
    pub(crate) reused: usize,
}

impl<T> Default for Table<T> {
    fn default() -> Self {
        Self {
            entries: Vec::new(), colors: Vec::new(), cursor: 0, free: 0, quarantined: 0, reserved: false, high: 0, marking: false, sticky: 0, generation: 0, barrier: Barrier::default(), image: ImageSet::default(), image_reached: false, cards: Vec::new(),
            #[cfg(test)]
            reused: 0,
        }
    }
}

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
        let (entry, color, barrier, image) = self.store_parts(i);
        let Object::HashTable(table) = entry else { return None };
        let weak = table.weakness != crate::hash::Weakness::None;
        let image = (i < image.len).then(|| (&mut image.remembered[i / 64], 1 << (i % 64)));
        Some((&mut **table, Stores { barrier, color, weak, image }))
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
        let (entry, color, barrier, _) = self.store_parts(i);
        (entry, color, barrier)
    }
    /// As `store_at`, with the image's set beside: what a hash table's
    /// stores need.
    #[inline]
    fn store_parts(&mut self, i: usize) -> (&mut T, &mut u8, &mut Barrier, &mut ImageSet) {
        debug_assert!(self.colors.len() >= self.entries.len(), "a colour for every entry");
        debug_assert!(self.cards.len() * CARD >= self.colors.len(), "a card for every colour");
        let entry = &mut self.entries[i];
        // SAFETY: I is within `entries` (just indexed), and `colors` is never
        // the shorter, at any instruction, so that a panic part way leaves it
        // so: `push` and `collector_prepend` lengthen the colours first, `cut`
        // and `collector_compact` shorten the entries first, and `dense`
        // makes them equal. Nothing else changes either length. `cards`
        // covers every colour the same way: `push`, `collector_prepend` and
        // `dense` lengthen it before the colours, and `cut` shortens it after.
        // (No bounds check on the card: this is every store's path.)
        let color = unsafe {
            *self.cards.get_unchecked_mut(i / CARD) = 1;
            self.colors.get_unchecked_mut(i)
        };
        (entry, color, &mut self.barrier, &mut self.image)
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
        self.cards[i / CARD] = 1;
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
        // The card and the colour first: no entry is ever without them (`store_at`).
        let i = self.colors.len();
        if i / CARD == self.cards.len() { self.cards.push(1) } else { self.cards[i / CARD] = 1 }
        self.colors.push(IN_USE);
        self.entries.push(value);
        self.entries.len() - 1
    }
    pub(crate) fn extend(&mut self, values: impl IntoIterator<Item = T>) {
        for value in values {
            self.push(value);
        }
    }
    /// The lowest free entry at or above `cursor', there being one: the
    /// first zero colour, found as `memchr` finds a byte, many at a time.
    #[inline]
    fn next_free(&self) -> usize {
        let at = self.cursor;
        if self.colors[at] == FREE {
            return at;
        }
        at + memchr::memchr(FREE, &self.colors[at..]).expect("a table's count of free entries")
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
        self.image_reached = !minor;
        self.generation += 1;
        RawTable {
            entries: self.entries.as_mut_ptr(), colors: self.colors.as_mut_ptr().cast(), len: self.entries.len(),
            free: self.free, quarantined: self.quarantined, generation: self.generation, sticky: self.sticky,
            image: if minor { ImageSet::default() } else { self.image.clone() },
        }
    }
    /// Mark entry I, as `RawTable::claim'.
    #[cfg(test)]
    pub(crate) fn mark(&mut self, i: usize) -> bool {
        if !unreached(self.colors[i], self.sticky) || self.reached_as_image(i) {
            return false;
        }
        self.colors[i] |= MARK;
        true
    }
    /// Whether entry I counts as reached by a full's mark standing as an
    /// untouched image entry.
    #[inline]
    fn reached_as_image(&self, i: usize) -> bool {
        self.image_reached && self.image.permanent(i)
    }
    /// Whether a mark in progress has reached entry I.
    #[inline(always)]
    pub(crate) fn marked(&self, i: usize) -> bool {
        let color = self.colors[i];
        color & IN_USE != 0 && (color & (MARK | self.sticky) != 0 || self.reached_as_image(i))
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
            // Only a mark is written: an image entry's colour page stays the image's.
            if *color & (IN_USE | MARK) == IN_USE | MARK { *color &= !MARK; }
            free += usize::from(*color == FREE);
            quarantined += usize::from(*color == QUARANTINED);
        }
        (self.free, self.quarantined, self.cursor, self.marking, self.sticky, self.image_reached) = (free, quarantined, 0, false, 0, false);
        self.generation += 1;
    }
    /// After a completed mark: every dead entry through DESTROY, which leaves
    /// a tombstone owning nothing, then free (under QUARANTINE, never reused
    /// and never cut), every mark gone, and the dead tail cut. Entries freed.
    #[cfg(test)]
    pub(crate) fn sweep(&mut self, quarantine: bool, destroy: impl FnMut(&mut T)) -> usize {
        let raw = tests::raw_of(self);
        let mut destroy = destroy;
        let swept = tests::sweep_whole(&raw, quarantine, |_, entry| destroy(entry));
        self.swept(swept, quarantine);
        swept.freed
    }
    /// The books of a sweep in ranges (`RawTable::sweep_range'), SWEPT in
    /// all: a full's of every entry, a minor's of the set cards (`touched`).
    pub(crate) fn swept(&mut self, swept: RangeSwept, quarantine: bool) {
        if quarantine { self.quarantined += swept.freed } else { self.free += swept.freed }
        // No free entry lay below the cursor, and the sweep freed none
        // below its lowest: `next_free` starts from there, not from the
        // bottom of an old generation it would cross each minor.
        if !quarantine { self.cursor = self.cursor.min(swept.lowest); }
        // A minor swept the set cards alone: what is in use in the others,
        // old, ends the table where it is above the last it swept.
        let last = if self.sticky == OLD {
            let len = self.colors.len().min(self.entries.len());
            self.colors[swept.last.min(len)..len].iter().rposition(|&color| color != FREE).map_or(swept.last, |i| swept.last + i + 1)
        } else {
            swept.last
        };
        (self.marking, self.sticky, self.image_reached) = (false, 0, false);
        self.generation += 1;
        // Every survivor is old and clean, so nothing logged still stands.
        self.barrier = Barrier::default();
        self.cut(last);
        self.cards.fill(0);
    }
    /// Drop the entries from LEN on, all free tombstones.
    fn cut(&mut self, len: usize) {
        self.high = self.high.max(self.entries.len());
        self.free -= self.entries.len() - len;
        self.entries.truncate(len);
        self.colors.truncate(len);
        self.cards.truncate(len.div_ceil(CARD));
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
    /// Whether any entry is of the restored image: one a compaction keeps
    /// where it is.
    fn holds_image(&self) -> bool {
        self.image.len > 0
    }
    /// Where a compaction after the mark standing puts each entry, and how
    /// many of PADS fillers go in front: the live ones in order, each down
    /// past the dead; `usize::MAX` for the dead. The image's entries, dense
    /// and never freed, have no dead below them and stay where they are; no
    /// filler goes in front of them, which would move them, and with nothing
    /// live, none either.
    pub(crate) fn compaction_map(&self, pads: usize) -> (Vec<usize>, usize) {
        let pinned = self.holds_image();
        let (mut map, mut next, mut any) = (Vec::with_capacity(self.entries.len()), if pinned { 0 } else { pads }, false);
        for i in 0..self.entries.len() {
            let live = self.marked(i);
            any |= live;
            map.push(if live { next += 1; next - 1 } else { usize::MAX });
        }
        (map, if any && !pinned { pads } else { 0 })
    }
    /// Compaction, the entries MAP (`compaction_map`) keeps, in order, each
    /// through KEEP but a permanent one, which names only image entries, none
    /// of which moved. The table is left dense.
    pub(crate) fn collector_compact(&mut self, map: &[usize], mut keep: impl FnMut(&mut T)) {
        self.high = self.high.max(self.entries.len());
        assert_eq!(map.len(), self.entries.len(), "a compaction map for every entry");
        let (entries, image) = (&mut self.entries, &self.image);
        let mut i = 0;
        entries.retain_mut(|entry| {
            let live = map[i] != usize::MAX;
            if live && !image.permanent(i) {
                keep(entry);
            }
            i += 1;
            live
        });
        self.dense();
    }
    /// N fillers in front of every entry, which move up past them.
    pub(crate) fn collector_prepend(&mut self, n: usize, filler: impl FnMut() -> T) {
        if n == 0 {
            return;
        }
        // Cards and colours for them first, whatever FILLER does (`store_at`).
        self.cards.resize((self.entries.len() + n).div_ceil(CARD), 0);
        self.colors.resize(self.entries.len() + n, FREE);
        self.entries.splice(0..0, std::iter::repeat_with(filler).take(n));
        self.dense();
    }
    /// Every entry in use, and old: what a compaction leaves are survivors.
    fn dense(&mut self) {
        // Never shorter than the entries on the way (`store_parts`), the
        // cards before the colours.
        self.cards.resize(self.entries.len().div_ceil(CARD), 0);
        self.colors.resize(self.entries.len(), OLD | IN_USE);
        // The image's colours are written only where a mark is to clear:
        // its pages stay the image's.
        let image = self.image.len.min(self.colors.len());
        for color in &mut self.colors[..image] {
            if *color != OLD | IN_USE { *color = OLD | IN_USE; }
        }
        self.colors[image..].fill(OLD | IN_USE);
        (self.cursor, self.free, self.quarantined, self.marking, self.sticky, self.image_reached) = (self.entries.len(), 0, 0, false, 0, false);
        self.cards.fill(0);
        self.generation += 1;
        self.barrier = Barrier::default();
    }
    /// A minor's start. To its mark every old entry counts as reached
    /// (`raw`), but those it must read again, here the ones stored into
    /// since the last collection (and `read_again_at`'s). Each of those is
    /// made young for this collection and returned, for the mark to claim
    /// and read as it does the young; being roots, all survive, and the
    /// sweep leaves them old again. No other colour is written. A minor that
    /// ends any other way promotes everything (`Vm::promote_all`), which
    /// ages them again. Only the set cards are looked at (`touched`), and in
    /// them a word of eight colours with none dirty is passed at one test.
    pub(crate) fn read_again(&mut self) -> Vec<usize> {
        let mut again = Vec::new();
        let runs = self.touched();
        let image = &mut self.image;
        for run in runs {
            let colors = &mut self.colors[run.clone()];
            for (w, word) in colors.chunks_mut(8).enumerate() {
                if word.iter().all(|&color| color & DIRTY == 0) { continue; }
                for (k, color) in word.iter_mut().enumerate() {
                    if *color & (IN_USE | OLD | DIRTY) == (IN_USE | OLD | DIRTY) {
                        let i = run.start + w * 8 + k;
                        // A store into an image entry, recorded before the
                        // sweep clears its DIRTY.
                        if i < image.len { image.remember(i); }
                        *color &= !OLD;
                        again.push(i);
                    }
                }
            }
        }
        again
    }
    /// The entries of the set cards, in runs of whole cards (the last cut
    /// at the table's end): all of the table a minor reads or sweeps.
    pub(crate) fn touched(&self) -> Vec<std::ops::Range<usize>> {
        let len = self.entries.len();
        let mut runs: Vec<std::ops::Range<usize>> = Vec::new();
        // Eight cards a word: a table's cards are mostly clear.
        for (w, word) in self.cards.chunks(8).enumerate() {
            if word.iter().all(|&set| set == 0) { continue; }
            for (k, _) in word.iter().enumerate().filter(|&(_, &set)| set != 0) {
                let card = w * 8 + k;
                let range = card * CARD..((card + 1) * CARD).min(len);
                if range.is_empty() { return runs; }
                match runs.last_mut() {
                    Some(run) if run.end == range.start => run.end = range.end,
                    _ => runs.push(range),
                }
            }
        }
        runs
    }
    /// Under verify, at a minor's start: every entry in use outside a set
    /// card is exactly old and clean, which is what lets a minor pass the
    /// card unread. A store or an allocation that set no card fails here.
    /// A mark a key press ended leaves its marks, for the next mark to
    /// clear (`raw`).
    pub(crate) fn check_cards(&self, table: &str) {
        let checked = if self.marking { OLD | DIRTY } else { OLD | DIRTY | MARK };
        for (i, &color) in self.colors[..self.entries.len()].iter().enumerate() {
            if self.cards[i / CARD] == 0 && color & IN_USE != 0 {
                assert!(color & checked == OLD, "{table} entry {i} is young or dirty (colour {color:#x}) outside a set card");
            }
        }
    }
    /// The entries in use that the mark standing did not reach and WANTED
    /// accepts: the dead, before their sweep.
    pub(crate) fn unreached(&self, wanted: impl Fn(&T) -> bool) -> Vec<u32> {
        self.colors.iter().zip(&self.entries).enumerate()
            .filter(|&(i, (&color, entry))| unreached(color, self.sticky) && !self.reached_as_image(i) && wanted(entry))
            .map(|(i, _)| i as u32).collect()
    }
    /// As `read_again`, of the entries AT alone: the old ones WHOLE asks for.
    pub(crate) fn read_again_at(&mut self, at: &[u32], whole: &dyn Fn(&T) -> bool) -> Vec<usize> {
        let mut again = Vec::new();
        let (colors, entries) = (&mut self.colors, &self.entries);
        for i in at.iter().map(|&i| i as usize) {
            let (Some(color), Some(entry)) = (colors.get_mut(i), entries.get(i)) else { continue };
            if *color & (IN_USE | OLD) == (IN_USE | OLD) && whole(entry) {
                *color &= !OLD;
                // Young for this collection: its sweep must meet it.
                self.cards[i / CARD] = 1;
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
        self.remember_dirty();
        for color in &mut self.colors {
            // Written only where it changes: an image entry's colour page
            // stays the image's.
            if *color & IN_USE != 0 && *color != OLD | IN_USE { *color = OLD | IN_USE; }
        }
        self.cards.fill(0);
        (self.sticky, self.image_reached) = (0, false);
        self.barrier = Barrier::default();
    }
    /// Whether every entry in use is exactly old, of the image or not, and
    /// the log is empty: what a completed collection leaves, and what an
    /// image may hold.
    pub(crate) fn settled(&self) -> bool {
        self.barrier.values.is_empty() && !self.barrier.lost && self.colors.iter().all(|&color| matches!(color, FREE | QUARANTINED) || color == (OLD | IN_USE))
    }
    /// Every entry becomes of the image: remembered if REMEMBERED asks for
    /// it, else permanent. A restore's, of a settled and dense table (a world
    /// is captured dense): no colour is written.
    pub(crate) fn make_permanent(&mut self, remembered: impl Fn(&T) -> bool) {
        if self.marking {
            self.clear_marks();
        }
        assert_eq!(self.unused(), 0, "an image's table is dense");
        self.image = ImageSet { len: self.entries.len(), remembered: vec![0; self.entries.len().div_ceil(64)] };
        for (i, entry) in self.entries.iter().enumerate() {
            if remembered(entry) { self.image.remember(i); }
        }
    }
    /// Every image entry stored into since the last collection, remembered.
    /// Every collection runs this, or `read_again`, before a sweep clears
    /// DIRTY. A word of eight colours with none dirty is passed at one test.
    fn remember_dirty(&mut self) {
        let len = self.image.len.min(self.colors.len());
        let (colors, image) = (&self.colors[..len], &mut self.image);
        let dirty = LOW_BITS * u64::from(DIRTY);
        let mut look = |from: usize, chunk: &[u8]| {
            for (k, &color) in chunk.iter().enumerate() {
                if color & (IN_USE | DIRTY) == IN_USE | DIRTY { image.remember(from + k); }
            }
        };
        let words = colors.chunks_exact(8);
        let rest = words.remainder();
        for (w, chunk) in words.enumerate() {
            if u64::from_le_bytes(chunk.try_into().expect("eight colours")) & dirty != 0 { look(w * 8, chunk); }
        }
        look(len - rest.len(), rest);
    }
    /// A full's roots in the image: every remembered entry, those stored
    /// into since the last collection first remembered.
    pub(crate) fn remembered(&mut self) -> Vec<usize> {
        self.remember_dirty();
        let mut found = Vec::new();
        for (w, &word) in self.image.remembered.iter().enumerate() {
            let mut bits = word;
            while bits != 0 {
                found.push(w * 64 + bits.trailing_zeros() as usize);
                bits &= bits - 1;
            }
        }
        found
    }
    /// Entry I, written by the collector past the barrier after its mark:
    /// remembered if it was permanent, and still reached by a full's mark
    /// standing, which counted it reached as permanent and must not sweep it
    /// as an unmarked remembered one. A minor's counts it reached as old: a
    /// mark there would outlive a sweep that passes its clear card.
    pub(crate) fn remember(&mut self, i: usize) {
        if self.image.permanent(i) {
            self.image.remember(i);
            if self.image_reached { self.colors[i] |= MARK; }
        }
    }
    /// Whether entry I is of the image and untouched since the restore.
    pub(crate) fn permanent(&self, i: usize) -> bool {
        self.image.permanent(i) && self.colors[i] & IN_USE != 0
    }
    /// Whether entry I is of the image and read by every full.
    #[cfg(test)]
    pub(crate) fn is_remembered(&self, i: usize) -> bool {
        i < self.image.len && self.image.remembered(i)
    }
    /// Whether the mark standing claimed entry I itself: not counted as
    /// reached for its age or permanence.
    #[cfg(test)]
    pub(crate) fn claimed(&self, i: usize) -> bool {
        self.colors[i] & (IN_USE | MARK) == IN_USE | MARK
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
    /// Entry I as if nothing were stored into it since the last collection:
    /// what a store into storage the entry does not colour leaves, for a test.
    #[cfg(test)]
    pub(crate) fn forget_store(&mut self, i: usize) {
        self.colors[i] &= !DIRTY;
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
        (0..self.colors.len()).map(|i| self.marked(i))
    }
}

#[cfg(test)]
#[path = "table_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "table_image_tests.rs"]
mod image_tests;
