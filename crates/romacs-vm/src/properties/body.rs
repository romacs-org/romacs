//! A text body's properties with a record of what was stored into them
//! since the last collection, and the store a property plan writes through.
use super::{Key, Properties, SyntaxChanges};
use crate::{Value, coordinates::ByteOffset};
use std::ops::Range;

/// What a property primitive's plan asks of the properties it changes: a
/// string's, or a text body's, which records what is stored.
pub(crate) trait Store {
    fn put(&mut self, range: Range<ByteOffset>, key: Key, value: Value);
    fn remove(&mut self, range: Range<ByteOffset>, key: Key);
    fn clear(&mut self, range: Range<ByteOffset>);
    fn reset(&mut self);
}
impl Store for Properties {
    fn put(&mut self, range: Range<ByteOffset>, key: Key, value: Value) { Properties::put(self, range, key, value); }
    fn remove(&mut self, range: Range<ByteOffset>, key: Key) { Properties::remove(self, range, key); }
    fn clear(&mut self, range: Range<ByteOffset>) { Properties::clear(self, range); }
    fn reset(&mut self) { Properties::reset(self); }
}
impl Store for BodyProperties {
    fn put(&mut self, range: Range<ByteOffset>, key: Key, value: Value) { BodyProperties::put(self, range, key, value); }
    fn remove(&mut self, range: Range<ByteOffset>, key: Key) { BodyProperties::remove(self, range, key); }
    fn clear(&mut self, range: Range<ByteOffset>) { BodyProperties::clear(self, range); }
    fn reset(&mut self) { self.store.reset(); }
}

/// How a collection reads a text body's properties (`BodyProperties`).
#[derive(Clone, Copy, Default)]
pub(crate) struct Reading {
    /// The epoch the VM's last collection ended at (`crate::epoch`).
    pub(crate) since: u64,
    /// Every property is read, as a full collection, a compaction's rewrite
    /// and verify's reference walk do; else only what was stored since.
    pub(crate) whole: bool,
}

/// Values kept before a record gives up and the body is read whole.
const WRITTEN_LIMIT: usize = 64 << 10;

/// A text body's properties, with a record of what was stored into them.
/// A body is shared by a buffer and its indirect buffers and belongs to no
/// table entry alone, so no entry's colour says it was stored into: the
/// record is here, in the storage, and a minor collection reads it in place
/// of every property of every buffer. It hands out the properties to read
/// only (`Deref`), and each method that can store a Value records it, so a
/// store that records nothing does not compile. A string's properties need
/// none of this: its entry is dirtied.
///
/// The record is kept by epoch (`crate::epoch`), which any VM's collection
/// in the process advances: a store cannot tell whether its own VM has
/// collected since the last one, so it never drops what is recorded. The
/// collector, which knows the epoch its VM's last collection ended at,
/// reads what was recorded since and drops what was recorded before.
#[derive(Clone)]
pub(crate) struct BodyProperties {
    store: Properties,
    /// The Values stored that can name a heap entry, oldest first.
    written: Vec<Value>,
    /// The epochs `written` was recorded in, ascending, and where each one's
    /// Values begin.
    epochs: Vec<u64>,
    starts: Vec<usize>,
    /// The epoch at which everything last counted as stored: when the body
    /// was made, its properties replaced whole, or its record given up.
    all_at: u64,
}
romacs_snapshot::snapshot_struct!(BodyProperties { store, written, epochs, starts, all_at });

impl std::ops::Deref for BodyProperties {
    type Target = Properties;
    #[inline]
    fn deref(&self) -> &Properties {
        &self.store
    }
}

impl BodyProperties {
    /// STORE as a body's properties: all of it counts as just stored.
    pub(crate) fn new(store: Properties) -> Self {
        Self { store, written: Vec::new(), epochs: Vec::new(), starts: Vec::new(), all_at: crate::epoch::born() }
    }
    /// VALUES are about to be stored.  Keys are not recorded: a key is a
    /// symbol, and a minor keeps every symbol (`Vm::minor_keeps`).
    fn record(&mut self, values: impl IntoIterator<Item = Value>) {
        let now = crate::epoch::born();
        // Everything counts as stored in this epoch already.
        if self.all_at == now { return; }
        if self.epochs.last() != Some(&now) {
            self.epochs.push(now);
            self.starts.push(self.written.len());
        }
        self.written.extend(values.into_iter().filter(|v| matches!(v.tag(), 0 | 3..=5) && *v != Value::NIL));
        if self.written.len() > WRITTEN_LIMIT {
            (self.written, self.epochs, self.starts, self.all_at) = (Vec::new(), Vec::new(), Vec::new(), now);
        }
    }
    pub fn put(&mut self, range: Range<ByteOffset>, key: Key, value: Value) {
        self.record([value]);
        self.store.put(range, key, value);
    }
    pub fn inherit_inserted(&mut self, range: Range<ByteOffset>, inherited: &[(Key, Value)]) {
        self.record(inherited.iter().map(|&(_, value)| value));
        self.store.inherit_inserted(range, inherited);
    }
    pub fn copy_into(&mut self, at: ByteOffset, source: &Properties) {
        self.record(source.to_map().into_values().flat_map(|values| values.into_iter().map(|(_, value)| value)));
        self.store.copy_into(at, source);
    }
    pub fn remove(&mut self, range: Range<ByteOffset>, key: Key) {
        self.store.remove(range, key);
    }
    pub fn clear(&mut self, range: Range<ByteOffset>) {
        self.store.clear(range);
    }
    pub fn replace(&mut self, range: Range<ByteOffset>, inserted: usize) -> usize {
        self.store.replace(range, inserted)
    }
    pub fn resize_character(&mut self, range: Range<ByteOffset>, inserted: usize) {
        self.store.resize_character(range, inserted);
    }
    pub(crate) fn take_syntax_changes(&mut self) -> SyntaxChanges {
        self.store.take_syntax_changes()
    }
    /// The collector's visit, as READING says. What was recorded before the
    /// VM's last collection is dropped: that collection kept it. What was
    /// recorded since is visited, by a whole reading too: a compaction
    /// rewrites those Values, and one removed since it was stored is kept
    /// until the next collection. Nothing recorded since is dropped here:
    /// a mark a key press abandons has consumed nothing.
    pub(crate) fn visit_values(&mut self, reading: Reading, visit: &mut impl FnMut(&mut Value)) {
        let stale = self.epochs.partition_point(|&epoch| epoch < reading.since);
        if stale > 0 {
            let from = self.starts.get(stale).copied().unwrap_or(self.written.len());
            self.written.drain(..from);
            self.epochs.drain(..stale);
            self.starts.drain(..stale);
            self.starts.iter_mut().for_each(|start| *start -= from);
        }
        self.written.iter_mut().for_each(&mut *visit);
        if reading.whole || self.all_at >= reading.since {
            self.store.visit_values(visit);
        }
    }
}
