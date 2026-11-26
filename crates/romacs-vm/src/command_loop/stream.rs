//! GNU's KBOARD as a movable table of input streams (DESIGN §2.1): what an
//! input stream keeps of the command being read, its keys
//! (`this_command_keys`), where the last single command's keys start
//! (`this_single_command_key_start`) and its raw keys (`raw_keybuf`), and
//! the reads in progress on it.
//!
//! The census of §2.1 is Rust privacy: `InputStream`'s fields are private to
//! this file and `Streams::store` is their only writer, so a direct write
//! elsewhere does not compile. A read's frames (`KeyseqFrame`) are scratch,
//! not stream state: they live only while a read is suspended, so a world
//! image keeps none (`Vm::reads` is at rest).
use super::keyseq::KeyseqFrame;
use crate::Value;

/// A stream's index in the table, stable while the stream lives.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) struct StreamId(usize);

/// The stream every read uses until streams are selected per mutator (C1b).
pub(crate) const MAIN: StreamId = StreamId(0);

/// One input stream's durable state.
#[derive(romacs_snapshot::Snapshot, Default)]
pub(crate) struct InputStream {
    keys: Vec<Value>,
    single_key_start: usize,
    raw_keys: Vec<Value>,
}

/// A write to a stream: the one door its fields have.
#[derive(Clone, Copy)]
pub(crate) enum Store<'a> {
    /// `add_command_key`: EVENT after the keys.
    PushKey(Value),
    /// The keys become KEYS, the single command's start at START.
    Keys(&'a [Value], usize),
    /// The keys are cut back to their first N.
    TruncateKeys(usize),
    /// The single command's keys start at N.
    SingleStart(usize),
    /// The raw keys of the read that ended.
    RawKeys(&'a [Value]),
}

/// The streams (GNU's `all_kboards`), reached through `Vm::streams` and
/// `Vm::streams_mut` alone, so moving them is a one-place change.
#[derive(romacs_snapshot::Snapshot)]
pub(crate) struct Streams {
    table: Vec<InputStream>,
}

impl Default for Streams {
    fn default() -> Self {
        Self { table: vec![InputStream::default()] }
    }
}

impl Streams {
    fn stream(&self, id: StreamId) -> &InputStream {
        &self.table[id.0]
    }
    /// STREAM's keys (`this_command_keys`).
    pub(crate) fn keys(&self, id: StreamId) -> &[Value] {
        &self.stream(id).keys
    }
    /// Where STREAM's single command's keys start, at most its keys' count.
    pub(crate) fn single_key_start(&self, id: StreamId) -> usize {
        let stream = self.stream(id);
        stream.single_key_start.min(stream.keys.len())
    }
    /// STREAM's raw keys (`raw_keybuf`).
    pub(crate) fn raw_keys(&self, id: StreamId) -> &[Value] {
        &self.stream(id).raw_keys
    }
    /// The one writer of a stream's fields.
    pub(crate) fn store(&mut self, id: StreamId, store: Store<'_>) {
        let stream = &mut self.table[id.0];
        match store {
            Store::PushKey(event) => stream.keys.push(event),
            Store::Keys(keys, start) => {
                stream.keys = keys.to_vec();
                stream.single_key_start = start;
            }
            Store::TruncateKeys(n) => stream.keys.truncate(n),
            Store::SingleStart(n) => stream.single_key_start = n,
            Store::RawKeys(keys) => stream.raw_keys = keys.to_vec(),
        }
    }
    /// Every Value the streams hold, for the collector to mark and rewrite.
    pub(crate) fn visit_values(&mut self, visit: &mut impl FnMut(&mut Value)) {
        for stream in &mut self.table {
            stream.keys.iter_mut().chain(stream.raw_keys.iter_mut()).for_each(&mut *visit);
        }
    }
}

/// The reads in progress on each stream, innermost last: one frame per live
/// `read_key_sequence` (DESIGN §2.4), and what the reader's orchestration
/// keeps between its leaves. Scratch, never in an image.
pub(crate) struct Reads {
    frames: Vec<Vec<KeyseqFrame>>,
    /// The value of the reader's last Lisp call or walk.
    pub(crate) result: Value,
    /// The reader's Lisp calls so far: a map taken at one count is current
    /// while the count stays (no Lisp ran since).
    pub(crate) callouts: u64,
    /// Writes to watched variables, each made by a request after the leaf
    /// that asked for it, so its watchers may run Lisp.
    pub(crate) writes: Vec<(usize, Value)>,
    /// The keymap walks stopped at a filter, innermost last (`KeyWalk::values`).
    pub(crate) walks: Vec<Vec<Value>>,
}

impl Default for Reads {
    fn default() -> Self {
        Self { frames: Vec::new(), result: Value::NIL, callouts: 0, writes: Vec::new(), walks: Vec::new() }
    }
}

impl Reads {
    /// A new read on STREAM: its depth there.
    pub(crate) fn push(&mut self, id: StreamId, frame: KeyseqFrame) -> usize {
        if self.frames.len() <= id.0 { self.frames.resize_with(id.0 + 1, Vec::new); }
        self.frames[id.0].push(frame);
        self.frames[id.0].len() - 1
    }
    /// The read at DEPTH on STREAM ends; reads end innermost first.
    pub(crate) fn pop(&mut self, id: StreamId, depth: usize) {
        let frames = &mut self.frames[id.0];
        assert_eq!(frames.len(), depth + 1, "reads end innermost first");
        frames.pop();
    }
    /// The read at DEPTH on STREAM.
    pub(crate) fn frame(&mut self, id: StreamId, depth: usize) -> &mut KeyseqFrame {
        &mut self.frames[id.0][depth]
    }
    /// How many reads are in progress on STREAM.
    #[cfg(test)]
    pub(crate) fn depth(&self, id: StreamId) -> usize {
        self.frames.get(id.0).map_or(0, Vec::len)
    }
    /// Every Value the live reads hold, for the collector to mark and rewrite.
    pub(crate) fn visit_values(&mut self, visit: &mut impl FnMut(&mut Value)) {
        visit(&mut self.result);
        for (_, value) in &mut self.writes { visit(value); }
        self.walks.iter_mut().flatten().for_each(&mut *visit);
        for frame in self.frames.iter_mut().flatten() {
            frame.visit_values(visit);
        }
    }
}
