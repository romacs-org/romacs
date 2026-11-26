//! GNU's KBOARD as a movable table of input streams (DESIGN §2.1): what an
//! input stream keeps of the command being read, its keys
//! (`this_command_keys`), where the last single command's keys start
//! (`this_single_command_key_start`) and its raw keys (`raw_keybuf`), and
//! the reads in progress on it.
//!
//! The census of §2.1 is Rust privacy: `InputStream`'s fields are private to
//! this file and `Streams::store` is their only writer, so a direct write
//! elsewhere does not compile. The reads in progress (`Reads`) are scratch,
//! not stream state: they live only while a read is suspended, so a world
//! image keeps none (`Vm::reads` is at rest, and `world_quiescent` asks it
//! idle).
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

/// One reading activation (a `read_key_sequence`, or a `read-event`'s
/// filtered read): what it keeps between its leaves, its own so a read
/// nested in its Lisp neither overwrites nor drains it (DESIGN §2.4).
pub(crate) struct Act {
    /// The value of its last Lisp call or keymap walk.
    pub(super) result: Value,
    /// A write to a watched variable its last leaf made, which a request
    /// completes before anything else happens (`super::flush`).
    pub(super) write: Option<(usize, Value)>,
    /// Its keymap walk while stopped at a filter (`KeyWalk::values`).
    pub(super) walk: Vec<Value>,
    /// Its key sequence's state, for a `read_key_sequence`.
    pub(super) keyseq: Option<KeyseqFrame>,
}

impl Act {
    /// An activation with nothing done yet, reading a key sequence with KEYSEQ.
    pub(super) fn new(keyseq: Option<KeyseqFrame>) -> Self {
        Self { result: Value::NIL, write: None, walk: Vec::new(), keyseq }
    }
}

/// The reading activations in progress, innermost last, and the count of
/// the reader's Lisp calls. Scratch, never in an image.
#[derive(Default)]
pub(crate) struct Reads {
    acts: Vec<Act>,
    /// The reader's Lisp calls so far: a map taken at one count is current
    /// while the count stays (no Lisp ran since).
    pub(super) callouts: u64,
}

impl Reads {
    /// A new activation: its index.
    pub(super) fn push(&mut self, act: Act) -> usize {
        self.acts.push(act);
        self.acts.len() - 1
    }
    /// The activation at AT ends; activations end innermost first.
    pub(super) fn pop(&mut self, at: usize) -> Act {
        assert_eq!(self.acts.len(), at + 1, "reads end innermost first");
        self.acts.pop().unwrap_or_else(|| Act::new(None))
    }
    /// The activation at AT.
    pub(super) fn act(&mut self, at: usize) -> &mut Act {
        &mut self.acts[at]
    }
    /// Whether no read is in progress, as a world image requires.
    pub(crate) fn idle(&self) -> bool {
        self.acts.is_empty()
    }
    /// Every Value the live reads hold, for the collector to mark and rewrite.
    pub(crate) fn visit_values(&mut self, visit: &mut impl FnMut(&mut Value)) {
        for act in &mut self.acts {
            visit(&mut act.result);
            if let Some((_, value)) = &mut act.write { visit(value); }
            act.walk.iter_mut().for_each(&mut *visit);
            if let Some(frame) = &mut act.keyseq { frame.visit_values(visit); }
        }
    }
}
