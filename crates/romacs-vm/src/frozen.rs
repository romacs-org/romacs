//! Frozen regions: immutable, closed value graphs a VM can share with
//! another VM (vm-concurrency design, §3.3).
//!
//! A frozen value is a `Value` with the `IMAGE` bit (never a private index)
//! whose payload names a region (12 bits, `REGION_SHIFT`) and an index in it.
//! Region 0 is the attached executable image (`image.rs`); regions 1 and up
//! are the ones this module makes. A region is a decoded, read-only segment
//! of the VM: its conses, objects and resolved symbols, built once when the
//! region's bytes are attached (`attach`), so every accessor that is total
//! over private objects is total over frozen ones through `Vm::object`,
//! `Vm::object_if` and `Vm::cons_pair`; mutation goes through `Vm::object_mut`
//! and `Vm::writable_cons`, which refuse with GNU's read-only error.
//!
//! One encoding (`VM2FRZ01`) serves both directions: `freeze` walks a graph
//! (private or frozen) into bytes and attaches them as a new region; `thaw`
//! walks a graph into the same bytes and decodes them into the private heap.
//! The walk is a memoised copy whose shell is entered before its edges are
//! followed, so cycles, shared substructure and uninterned-symbol identity
//! within one graph are preserved. Symbols are encoded by name (interned) or
//! as one descriptor per source symbol (uninterned) and resolved once at
//! attachment; they never carry the `IMAGE` bit.
//!
//! Bytes of a page or more are kept in a sealed memfd (`host::sealed_memfd`),
//! smaller ones inline; a receiver decodes either the same way, so a program
//! cannot tell which transport a value took. The bytes are kept beside the
//! region for forwarding (`Region::backing`).
//!
//! Liveness is the collector's (`gc.rs`): a region survives a collection iff
//! a traced value names it (a minor judges only the regions attached since
//! the last collection, as it judges only the young entries, so a dropped
//! result's region and memfd go at the next collection of either kind);
//! the region's symbols are roots while it lives;
//! weak-table entries whose frozen endpoint's region died are pruned; dead
//! regions are dropped and their ids reused. Pressure: region bytes count
//! toward the collection trigger (`Vm::heap_objects`), and at half the ids
//! in use the next engine poll collects (a generated primitive that attached
//! a region ends the engine's budget, so the poll is near); with every id
//! live, attachment signals `(error "Too many frozen regions" N)`.
use super::*;
use crate::data::Object;
use crate::image::IMAGE;
use std::sync::Arc;

mod code;
mod seed;

/// Region id bits of a frozen payload: above the 48-bit index, below `IMAGE`.
pub(crate) const REGION_SHIFT: u32 = 48;
const REGION_BITS: u32 = 12;
pub(crate) const INDEX_MASK: usize = (1 << REGION_SHIFT) - 1;
/// Ids 0..MAX_REGIONS; 0 is the executable image.
pub(crate) const MAX_REGIONS: usize = 1 << REGION_BITS;
/// Half the ids live: the next engine poll collects, which a tight
/// freeze loop reaches within a budget of a few thousand frozen values.
const COLLECT_AT: usize = MAX_REGIONS / 2;
/// Encoded bytes at or above this travel in a sealed memfd.
pub(crate) const INLINE_LIMIT: usize = 4096;
const MAGIC: &[u8; 8] = b"VM2FRZ01";
const HEADER: usize = 56;
const KIND_STRING: u8 = 1;
const KIND_VECTOR: u8 = 2;
const KIND_RECORD: u8 = 3;
const KIND_FLOAT: u8 = 4;
const KIND_BIGNUM: u8 = 5;
const KIND_BOOL_VECTOR: u8 = 6;
const KIND_HASH_TABLE: u8 = 7;
const KIND_CHAR_TABLE: u8 = 8;
/// A string's flags: multibyte, and a `LispString` (with properties) rather
/// than a bytes object.
const STRING_MULTIBYTE: u8 = 1;
const STRING_OBJECT: u8 = 2;
pub(crate) const READ_ONLY: &str = "Attempt to modify read-only object";

/// The region id of a frozen payload (0 for the executable image).
#[inline(always)]
pub(crate) fn region_of(payload: usize) -> usize {
    (payload >> REGION_SHIFT) & (MAX_REGIONS - 1)
}

/// Whether an image value's PAYLOAD is the executable image's (region 0),
/// not a frozen region's.
#[inline]
pub(crate) fn executable_image(payload: u64) -> bool {
    (payload >> REGION_SHIFT) & ((1 << REGION_BITS) - 1) == 0
}

/// The encoded bytes a region was made from, kept for forwarding.
#[derive(Clone)]
pub(crate) enum Backing {
    Inline(Arc<Vec<u8>>),
    /// A memfd sealed against writes, shrinking and growing.
    Sealed(Arc<std::fs::File>),
}

impl Backing {
    pub(crate) fn bytes(&self) -> std::io::Result<std::borrow::Cow<'_, [u8]>> {
        match self {
            Backing::Inline(bytes) => Ok(std::borrow::Cow::Borrowed(bytes)),
            Backing::Sealed(file) => crate::host::read_sealed(file).map(std::borrow::Cow::Owned),
        }
    }
    pub(crate) fn len(&self) -> usize {
        match self {
            Backing::Inline(bytes) => bytes.len(),
            Backing::Sealed(file) => crate::host::file_metadata(file).map(|m| m.len() as usize).unwrap_or(0),
        }
    }
    /// BYTES as a backing: a sealed memfd from a page up, else inline.
    pub(crate) fn new(bytes: Vec<u8>) -> std::io::Result<Self> {
        if bytes.len() >= INLINE_LIMIT {
            Ok(Backing::Sealed(Arc::new(crate::host::sealed_memfd(&bytes)?)))
        } else {
            Ok(Backing::Inline(Arc::new(bytes)))
        }
    }
}

/// The VM's frozen regions by id. Id 0 is the VM's own heap and never a
/// region, so a VM starts with that slot alone, and holds that at rest:
/// what a restored VM remakes (`romacs_snapshot::Schema::renew`).
///
/// The table grows to the most regions ever live at once and is never
/// truncated, so a sum over its slots costs every slot ever used. What
/// the attached regions add up to is kept as regions are placed and
/// removed instead: `Vm::list_limit` reads their conses on every list walk,
/// and the collector's pacing their weight at every poll.
/// Reads go through `Deref`; the table changes only here.
pub(crate) struct Regions {
    slots: Vec<Option<Box<Region>>>,
    totals: Totals,
}

/// What the attached regions add up to.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct Totals {
    pub(crate) regions: usize,
    pub(crate) conses: usize,
    /// The pressure they put on the collector, in objects (`Region::weight`).
    pub(crate) weight: usize,
}

impl Totals {
    fn of(region: &Region) -> Self {
        Totals { regions: 1, conses: region.conses.len(), weight: region.weight() }
    }
}

impl std::ops::Add for Totals {
    type Output = Self;
    fn add(self, o: Self) -> Self {
        Totals { regions: self.regions + o.regions, conses: self.conses + o.conses, weight: self.weight + o.weight }
    }
}

impl std::ops::Sub for Totals {
    type Output = Self;
    fn sub(self, o: Self) -> Self {
        Totals { regions: self.regions - o.regions, conses: self.conses - o.conses, weight: self.weight - o.weight }
    }
}

impl Default for Regions {
    fn default() -> Self {
        Self { slots: vec![None], totals: Totals::default() }
    }
}

impl Regions {
    /// A new empty slot's id, or None when the table is full.
    fn grow(&mut self) -> Option<usize> {
        (self.slots.len() < MAX_REGIONS).then(|| {
            self.slots.push(None);
            self.slots.len() - 1
        })
    }

    /// Put REGION in the free slot ID.
    fn place(&mut self, id: usize, region: Box<Region>) {
        debug_assert!(self.slots[id].is_none(), "region {id} placed over a live one");
        self.totals = self.totals + Totals::of(&region);
        self.slots[id] = Some(region);
        self.check();
    }

    /// Drop the region at ID, if one is there.
    fn remove(&mut self, id: usize) {
        if let Some(region) = self.slots.get_mut(id).and_then(Option::take) {
            self.totals = self.totals - Totals::of(&region);
            self.check();
        }
    }

    /// Replace object I of the region at ID: a hash table filled once its
    /// keys can be read. The region's totals do not change.
    fn set_object(&mut self, id: usize, i: usize, object: Object) {
        self.slots[id].as_mut().expect("placed region").objects[i] = object;
    }

    /// What the attached regions add up to, kept as regions come and go.
    pub(crate) fn totals(&self) -> Totals {
        self.totals
    }

    /// The same, summed over every slot: what `totals` keeps.
    pub(crate) fn summed(&self) -> Totals {
        self.slots.iter().flatten().fold(Totals::default(), |t, r| t + Totals::of(r))
    }

    /// Debug builds hold the kept totals to the sum where the table
    /// changes, never where they are read.
    fn check(&self) {
        debug_assert_eq!(self.totals, self.summed(), "frozen regions' totals drifted");
    }
}

impl std::ops::Deref for Regions {
    type Target = [Option<Box<Region>>];
    fn deref(&self) -> &Self::Target {
        &self.slots
    }
}

/// A region attached to this VM: decoded once, never written again.
pub(crate) struct Region {
    pub(crate) conses: Vec<(Value, Value)>,
    pub(crate) objects: Vec<Object>,
    /// The VM's symbol for each descriptor, interned or made at attachment.
    pub(crate) symbols: Vec<Value>,
    pub(crate) root: Value,
    /// Encoded length: the pressure the region puts on the collector.
    pub(crate) bytes: usize,
    pub(crate) backing: Backing,
}

impl Region {
    /// The pressure the region puts on the collector, in objects: its
    /// conses and objects, plus one per 16 encoded bytes.
    fn weight(&self) -> usize {
        self.conses.len() + self.objects.len() + self.bytes / 16
    }
}

/// A frozen value: `tag` 3 (cons) or 0 (object) in REGION at INDEX (objects
/// one-based, as private objects are).
fn frozen_value(region: usize, index: usize, tag: u64) -> Value {
    Value::reference(IMAGE | (region << REGION_SHIFT) | index, tag)
}

// The encoding.

struct Writer(Vec<u8>);
impl Writer {
    fn u8(&mut self, n: u8) { self.0.push(n); }
    fn u32(&mut self, n: u32) { self.0.extend_from_slice(&n.to_le_bytes()); }
    fn u64(&mut self, n: u64) { self.0.extend_from_slice(&n.to_le_bytes()); }
    fn bytes(&mut self, b: &[u8]) { self.u64(b.len() as u64); self.0.extend_from_slice(b); }
}

struct Reader<'a> { bytes: &'a [u8], at: usize }
impl<'a> Reader<'a> {
    fn take(&mut self, n: usize) -> Result<&'a [u8]> {
        let end = self.at.checked_add(n).filter(|&end| end <= self.bytes.len()).ok_or("truncated frozen region")?;
        let out = &self.bytes[self.at..end];
        self.at = end;
        Ok(out)
    }
    fn u8(&mut self) -> Result<u8> { Ok(self.take(1)?[0]) }
    fn u32(&mut self) -> Result<u32> { Ok(u32::from_le_bytes(self.take(4)?.try_into().unwrap())) }
    fn u64(&mut self) -> Result<u64> { Ok(u64::from_le_bytes(self.take(8)?.try_into().unwrap())) }
    fn count(&mut self) -> Result<usize> {
        let n = self.u64()?;
        usize::try_from(n).ok().filter(|&n| n <= self.bytes.len()).ok_or_else(|| "frozen region count out of range".into())
    }
    fn bytes(&mut self) -> Result<&'a [u8]> { let n = self.count()?; self.take(n) }
    fn done(&self) -> bool { self.at == self.bytes.len() }
}

/// The edges a region's words encode: immediates as their bits, conses and
/// objects region-relative, symbols by descriptor; in a code graph
/// (`code.rs`) also closures, capture cells and subrs by their index, and
/// the unbound marker a forwarded late cell holds.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum Edge { Immediate(Value), Cons(usize), Object(usize), Symbol(usize), Closure(usize), Cell(usize), Subr(usize), Unbound }

/// How many of each kind of node an encoding holds: the bounds its edges
/// are parsed against. A data graph's code counts are 0.
#[derive(Clone, Copy, Default)]
pub(crate) struct Counts {
    pub(crate) conses: usize,
    pub(crate) objects: usize,
    pub(crate) symbols: usize,
    pub(crate) closures: usize,
    pub(crate) cells: usize,
    pub(crate) subrs: usize,
}

impl Edge {
    pub(crate) fn word(self) -> u64 {
        match self {
            Edge::Immediate(v) => v.0,
            Edge::Cons(i) => ((i as u64) << 3) | 3,
            Edge::Object(i) => (i as u64 + 1) << 3,
            Edge::Symbol(i) => ((i as u64 + 1) << 3) | 7,
            Edge::Closure(i) => ((i as u64 + 1) << 3) | 4,
            Edge::Cell(i) => ((i as u64 + 1) << 3) | 2,
            Edge::Subr(i) => ((i as u64 + 1) << 3) | 6,
            Edge::Unbound => Value::UNBOUND.0,
        }
    }
    /// The edge WORD encodes, within COUNTS. The unbound marker is no edge
    /// here (`parse_content`).
    pub(crate) fn parse(word: u64, counts: &Counts) -> Result<Self> {
        let v = Value(word);
        let index = (word >> 3) as usize;
        let one_based = |n: usize| index >= 1 && index <= n;
        match word & 7 {
            1 | 5 => Ok(Edge::Immediate(v)),
            0 if word == 0 => Ok(Edge::Immediate(Value::NIL)),
            2 if word == 2 => Ok(Edge::Immediate(Value::TRUE)),
            3 if index < counts.conses => Ok(Edge::Cons(index)),
            0 if one_based(counts.objects) => Ok(Edge::Object(index - 1)),
            7 if one_based(counts.symbols) => Ok(Edge::Symbol(index - 1)),
            4 if one_based(counts.closures) => Ok(Edge::Closure(index - 1)),
            2 if one_based(counts.cells) => Ok(Edge::Cell(index - 1)),
            6 if one_based(counts.subrs) => Ok(Edge::Subr(index - 1)),
            _ => Err("frozen region edge is invalid".into()),
        }
    }
    /// A capture cell's content: an edge, or the unbound marker.
    pub(crate) fn parse_content(word: u64, counts: &Counts) -> Result<Self> {
        if word == Value::UNBOUND.0 { Ok(Edge::Unbound) } else { Self::parse(word, counts) }
    }
}

/// Where a graph walk is: the edge from a parent to the value at hand, for
/// the path a refusal names.
#[derive(Clone, Copy)]
pub(crate) enum Label {
    Root, Car, Cdr, Aref(usize), Key(usize), Val(usize), Property(usize), Purpose, Default, Parent, Extra(usize), Range(usize),
    // A code graph's (`code.rs`).
    Capture(usize), Metadata(usize), Document, Interactive, Constant(usize), Cell, Body,
}

struct Node { value: Value, parent: Option<usize>, label: Label }

/// What one step of a walk found (`Walk::step`).
enum Step {
    /// An immediate, or a value already entered.
    Seen,
    /// A cons, object or symbol, now entered.
    New,
    /// The node of a closure, capture cell or subr, for a code walk.
    Code(usize),
}

/// What `freeze` and `thaw` refuse: the object and the path of edges to it.
pub(crate) struct Refusal { pub(crate) object: Value, pub(crate) path: Vec<Label> }

/// The memoised graph walk: every reachable cons and object gets one index
/// in the encoding, in discovery order, before any edge is encoded.
struct Walk<'a> {
    vm: &'a Vm,
    nodes: Vec<Node>,
    /// Source value bits to the edge it encodes as.
    memo: HashMap<u64, Edge>,
    cons_nodes: Vec<usize>,
    object_nodes: Vec<usize>,
    symbols: Vec<(String, bool)>,
    symbol_memo: HashMap<u64, usize>,
}

impl<'a> Walk<'a> {
    fn new(vm: &'a Vm) -> Self {
        Walk { vm, nodes: vec![], memo: HashMap::default(), cons_nodes: vec![], object_nodes: vec![], symbols: vec![], symbol_memo: HashMap::default() }
    }

    fn refusal(&self, node: usize) -> Refusal {
        let mut path = vec![];
        let mut at = Some(node);
        while let Some(i) = at {
            path.push(self.nodes[i].label);
            at = self.nodes[i].parent;
        }
        path.reverse();
        Refusal { object: self.nodes[node].value, path }
    }

    /// Refuse VALUE, reached from PARENT by LABEL, without entering it.
    fn refuse(&mut self, value: Value, parent: Option<usize>, label: Label) -> Refusal {
        self.nodes.push(Node { value, parent, label });
        self.refusal(self.nodes.len() - 1)
    }

    fn symbol_named(&self, v: Value, name: &str) -> bool {
        v.index_if(7).and_then(|i| i.checked_sub(1)).and_then(|slot| self.vm.symbol_names.get(slot)).is_some_and(|n| n == name)
    }

    /// Whether the object V refers to can be frozen, by kind.
    fn freezable(&self, object: &Object) -> bool {
        match object {
            Object::String(_) | Object::RawString(_) | Object::Bytes(_) | Object::Vector(_) | Object::Record(_)
            | Object::Float(_) | Object::Bignum(_) | Object::BoolVector(_) => true,
            // A syntax table is a char-table whose purpose says so (GNU's
            // own) or the native one; both refuse, as the design states.
            Object::CharTable(t) => !self.symbol_named(t.purpose(), "syntax-table"),
            // A table on a test `define-hash-table-test' named stays with
            // its isolate, as the test's definition does.
            Object::HashTable(t) => t.weakness == hash::Weakness::None && !t.local_bindings
                && !matches!(t.test, hash::Test::User(..)),
            Object::SyntaxTable(_) | Object::Window(_) | Object::Frame(_) | Object::Terminal(_) | Object::WindowConfiguration(_)
            | Object::Sqlite(_) | Object::Treesit(_) | Object::Process(_) | Object::Finalizer(_) | Object::Thread | Object::Backing(_) | Object::SubCharTable(_) | Object::MapKeymapCursor(_)
            | Object::Buffer(_) | Object::Marker(_) | Object::Overlay(_) | Object::ByteBuffer(_) | Object::Obarray(_)
            | Object::Free => false,
        }
    }

    /// Discover every node reachable from ROOT, assigning indices. A
    /// closure, capture cell or subr is refused: they are per VM.
    fn discover(&mut self, root: Value) -> std::result::Result<(), Refusal> {
        let mut pending = vec![(root, None, Label::Root)];
        while let Some((value, parent, label)) = pending.pop() {
            if let Step::Code(node) = self.step(value, parent, label, &mut pending)? {
                return Err(self.refusal(node));
            }
        }
        Ok(())
    }

    /// Enter VALUE, reached from PARENT by LABEL: a new cons or object is
    /// given its index and its edges pushed on PENDING; a closure, capture
    /// cell or subr is given a node alone, for a code walk to enter.
    /// What entering VALUE will ask, known before it is entered: its
    /// payload's 64-byte units, and how many edges it will add beyond a
    /// few, counted no further than past ROOM. Nothing for a value entered
    /// already or with no such cost.
    fn ahead(&self, value: Value, room: usize) -> (usize, usize) {
        if self.memo.contains_key(&value.0) { return (0, 0); }
        match value.tag() {
            0 => self.vm.object_if(value).map_or((0, 0), |object| {
                let edges = match object {
                    Object::Vector(items) | Object::Record(items) => items.len(),
                    Object::HashTable(t) => 2 * t.count(),
                    Object::CharTable(t) => t.extras().len() + char_table_runs(t, room).map_or(0, |runs| runs.len()),
                    Object::String(s) | Object::RawString(s) => property_values(&s.properties, s.bytes().len(), room),
                    _ => 0,
                };
                (crate::data::payload_bytes(object) / 64, edges)
            }),
            4 => value.index_if(4).and_then(|i| self.vm.closures.get(i)).map_or((0, 0), |c| (0, c.captures.len() + c.metadata.len())),
            _ => (0, 0),
        }
    }

    fn step(&mut self, value: Value, parent: Option<usize>, label: Label, pending: &mut Vec<(Value, Option<usize>, Label)>)
        -> std::result::Result<Step, Refusal>
    {
        match value.tag() {
            7 => {
                if value == Value::UNBOUND || self.symbol_memo.contains_key(&value.0) { return Ok(Step::Seen); }
                let slot = value.payload() as usize - 1;
                let interned = self.vm.initial_obarray.get(slot).copied().unwrap_or(false);
                let name = self.vm.symbol_names.get(slot).cloned().unwrap_or_default();
                self.symbol_memo.insert(value.0, self.symbols.len());
                self.symbols.push((name, interned));
                Ok(Step::New)
            }
            3 | 4 | 5 | 6 | 0 if value != Value::NIL && value != Value::TRUE && !self.memo.contains_key(&value.0) => {
                let node = self.nodes.len();
                self.nodes.push(Node { value, parent, label });
                match value.tag() {
                    3 => self.enter_cons(value, node, pending),
                    0 => self.enter_object(value, node, pending),
                    _ => Ok(Step::Code(node)),
                }
            }
            _ => Ok(Step::Seen),
        }
    }

    fn enter_cons(&mut self, value: Value, node: usize, pending: &mut Vec<(Value, Option<usize>, Label)>)
        -> std::result::Result<Step, Refusal>
    {
        // A frozen graph is closed, so a cons reached through a frozen
        // value is always readable.
        let Ok((car, cdr)) = self.vm.cons_pair(value) else { return Err(self.refusal(node)) };
        self.memo.insert(value.0, Edge::Cons(self.cons_nodes.len()));
        self.cons_nodes.push(node);
        pending.push((cdr, Some(node), Label::Cdr));
        pending.push((car, Some(node), Label::Car));
        Ok(Step::New)
    }

    fn enter_object(&mut self, value: Value, node: usize, pending: &mut Vec<(Value, Option<usize>, Label)>)
        -> std::result::Result<Step, Refusal>
    {
        let Some(object) = self.vm.object_if(value).filter(|o| self.freezable(o)) else { return Err(self.refusal(node)) };
        self.memo.insert(value.0, Edge::Object(self.object_nodes.len()));
        self.object_nodes.push(node);
        match object {
            Object::String(s) | Object::RawString(s) => {
                for (i, (_, values)) in s.properties.runs(coordinates::ByteOffset(0)..coordinates::ByteOffset(s.bytes().len())).into_iter().enumerate() {
                    for (key, v) in values {
                        // Keys are frozen by name, which only an interned symbol's gives back.
                        if self.vm.property_key_name(key).is_none() { return Err(self.refusal(node)); }
                        pending.push((v, Some(node), Label::Property(i)));
                    }
                }
            }
            Object::Vector(items) | Object::Record(items) => {
                for (i, &v) in items.iter().enumerate().rev() { pending.push((v, Some(node), Label::Aref(i))); }
            }
            Object::HashTable(t) => {
                for (i, k, v) in t.entries() {
                    pending.push((v, Some(node), Label::Val(i)));
                    pending.push((k, Some(node), Label::Key(i)));
                }
            }
            Object::CharTable(t) => {
                pending.push((t.purpose(), Some(node), Label::Purpose));
                pending.push((t.default(), Some(node), Label::Default));
                pending.push((t.parent(), Some(node), Label::Parent));
                for (i, &v) in t.extras().iter().enumerate() { pending.push((v, Some(node), Label::Extra(i))); }
                for (from, _, v) in char_table_runs(t, usize::MAX).map_err(|_| self.refusal(node))? {
                    pending.push((v, Some(node), Label::Range(from)));
                }
            }
            _ => {}
        }
        Ok(Step::New)
    }

    fn edge(&self, v: Value) -> Edge {
        match v.tag() {
            7 if v != Value::UNBOUND => Edge::Symbol(self.symbol_memo[&v.0]),
            7 => Edge::Unbound,
            3 | 4 | 5 | 6 | 0 if v != Value::NIL && v != Value::TRUE => self.memo[&v.0],
            _ => Edge::Immediate(v),
        }
    }

    /// The bytes of the discovered graph with ROOT.
    fn encode(&self, root: Value) -> Vec<u8> {
        let mut w = Writer(Vec::new());
        w.0.extend_from_slice(MAGIC);
        w.u64(self.cons_nodes.len() as u64);
        w.u64(self.object_nodes.len() as u64);
        w.u64(self.symbols.len() as u64);
        let blob_at = w.0.len();
        w.u64(0);
        w.u64(self.edge(root).word());
        w.u64(0);
        debug_assert_eq!(w.0.len(), HEADER);
        for &node in &self.cons_nodes {
            let (car, cdr) = self.vm.cons_pair(self.nodes[node].value).expect("discovered cons");
            w.u64(self.edge(car).word());
            w.u64(self.edge(cdr).word());
        }
        // Object and symbol tables: (offset, length) into the blob, filled
        // after the blob is written.
        let tables = w.0.len();
        w.0.resize(tables + 16 * (self.object_nodes.len() + self.symbols.len()), 0);
        let blob = w.0.len();
        let mut entries = Vec::with_capacity(self.object_nodes.len() + self.symbols.len());
        for &node in &self.object_nodes {
            let start = w.0.len();
            self.encode_object(&mut w, self.vm.object_if(self.nodes[node].value).expect("discovered object"));
            entries.push((start - blob, w.0.len() - start, 0u64));
        }
        for (name, interned) in &self.symbols {
            let start = w.0.len();
            w.0.extend_from_slice(name.as_bytes());
            entries.push((start - blob, w.0.len() - start, u64::from(*interned)));
        }
        let blob_len = (w.0.len() - blob) as u64;
        w.0[blob_at..blob_at + 8].copy_from_slice(&blob_len.to_le_bytes());
        for (i, (offset, len, flag)) in entries.into_iter().enumerate() {
            let at = tables + 16 * i;
            w.0[at..at + 8].copy_from_slice(&(offset as u64).to_le_bytes());
            w.0[at + 8..at + 16].copy_from_slice(&((len as u64) | (flag << 63)).to_le_bytes());
        }
        w.0
    }

    fn encode_object(&self, w: &mut Writer, object: &Object) {
        match object {
            Object::String(s) | Object::RawString(s) => {
                w.u8(KIND_STRING);
                w.u8(u8::from(s.is_multibyte()) | STRING_OBJECT);
                w.bytes(s.bytes());
                let runs = s.properties.runs(coordinates::ByteOffset(0)..coordinates::ByteOffset(s.bytes().len()));
                w.u32(runs.len() as u32);
                for (range, values) in runs {
                    w.u64(range.start.0 as u64);
                    w.u64(range.end.0 as u64);
                    w.u32(values.len() as u32);
                    for (key, value) in values {
                        w.bytes(self.vm.property_key_name(key).expect("the walk refused other keys").as_bytes());
                        w.u64(self.edge(value).word());
                    }
                }
            }
            Object::Bytes(bytes) => {
                w.u8(KIND_STRING);
                w.u8(0);
                w.bytes(bytes);
                w.u32(0);
            }
            Object::Vector(items) | Object::Record(items) => {
                w.u8(if matches!(object, Object::Vector(_)) { KIND_VECTOR } else { KIND_RECORD });
                w.u64(items.len() as u64);
                for &v in items { w.u64(self.edge(v).word()); }
            }
            Object::Float(f) => { w.u8(KIND_FLOAT); w.u64(f.to_bits()); }
            Object::Bignum(n) => { w.u8(KIND_BIGNUM); w.bytes(&n.to_signed_bytes_le()); }
            Object::BoolVector(b) => { w.u8(KIND_BOOL_VECTOR); w.u64(b.len as u64); w.bytes(&b.bytes); }
            Object::HashTable(t) => {
                w.u8(KIND_HASH_TABLE);
                w.u8(match t.test { hash::Test::Eq => 0, hash::Test::Eql => 1, hash::Test::Equal => 2, hash::Test::User(..) => unreachable!("a user-test table is not freezable") });
                w.u64(t.count() as u64);
                for (_, k, v) in t.entries() {
                    w.u64(self.edge(k).word());
                    w.u64(self.edge(v).word());
                }
            }
            Object::CharTable(t) => {
                w.u8(KIND_CHAR_TABLE);
                w.u64(self.edge(t.purpose()).word());
                w.u64(self.edge(t.default()).word());
                w.u64(self.edge(t.parent()).word());
                w.u32(t.extras().len() as u32);
                for &v in t.extras() { w.u64(self.edge(v).word()); }
                let runs = char_table_runs(t, usize::MAX).expect("discovered char-table");
                w.u64(runs.len() as u64);
                for (from, to, v) in runs {
                    w.u64(from as u64);
                    w.u64(to as u64);
                    w.u64(self.edge(v).word());
                }
            }
            _ => unreachable!("only freezable objects are discovered"),
        }
    }
}

/// How many property values the runs of text END bytes long hold,
/// counted no further than past MAX.
fn property_values(properties: &crate::properties::Properties, end: usize, max: usize) -> usize {
    let (end, mut at, mut values) = (coordinates::ByteOffset(end), coordinates::ByteOffset(0), 0);
    while at < end && values <= max {
        values += properties.properties_at(at).len();
        at = properties.next_change(at, end);
    }
    values
}

/// A char-table's raw entries as maximal runs of its own subdivision:
/// (from, to, value), with nil where it has no value of its own. No more
/// than MAX + 1 are read, so a caller can bound a table before reading
/// it whole.
fn char_table_runs(t: &char_table::CharTable, max: usize) -> Result<Vec<(usize, usize, Value)>> {
    let mut runs = vec![];
    let mut at = 0;
    while at < char_table::CharTable::LEN && runs.len() <= max {
        let (value, from, to) = char_table::CharTableNode::checked_span(t.root(), at)?;
        if from != at { return Err("invalid sub-char-table geometry".into()); }
        if let Some(last) = runs.last_mut().filter(|(_, _, v): &&mut (usize, usize, Value)| *v == value) {
            last.1 = to;
        } else {
            runs.push((from, to, value));
        }
        at = to + 1;
    }
    Ok(runs)
}

// Decoding.

/// The decoded form of an encoding, before its objects are placed.
struct Decoded {
    conses: Vec<(Edge, Edge)>,
    objects: Vec<DecodedObject>,
    symbols: Vec<(String, bool)>,
    root: Edge,
}

enum DecodedObject {
    String { multibyte: bool, object: bool, bytes: Vec<u8>, runs: Vec<(usize, usize, Vec<(String, Edge)>)> },
    Vector(Vec<Edge>),
    Record(Vec<Edge>),
    Float(f64),
    Bignum(Vec<u8>),
    BoolVector { len: usize, bytes: Vec<u8> },
    HashTable { test: hash::Test, entries: Vec<(Edge, Edge)> },
    CharTable { purpose: Edge, default: Edge, parent: Edge, extras: Vec<Edge>, runs: Vec<(usize, usize, Edge)> },
}

fn decode(bytes: &[u8]) -> Result<Decoded> {
    if bytes.len() < HEADER || &bytes[..8] != MAGIC { return Err("not a frozen region".into()); }
    let mut r = Reader { bytes, at: 8 };
    let (conses, objects, symbols, blob_len) = (r.count()?, r.count()?, r.count()?, r.count()?);
    let root_word = r.u64()?;
    let _reserved = r.u64()?;
    let counts = Counts { conses, objects, symbols, ..Counts::default() };
    let root = Edge::parse(root_word, &counts)?;
    let (pairs, entries) = sections(&mut r, &counts, objects + symbols, blob_len)?;
    let decoded_objects = entries[..objects].iter().map(|&(start, end, _)| decode_object(&bytes[start..end], &counts)).collect::<Result<Vec<_>>>()?;
    let names = entries[objects..].iter().map(|&(start, end, interned)| symbol_entry(&bytes[start..end], interned)).collect::<Result<Vec<_>>>()?;
    Ok(Decoded { conses: pairs, objects: decoded_objects, symbols: names, root })
}

/// What follows a header at R: COUNTS' cons pairs, then a table of ENTRIES
/// (offset, length, flag) into the blob of `blob_len` bytes that ends the
/// encoding. The pairs, and each entry as its absolute range and flag.
fn sections(r: &mut Reader, counts: &Counts, entries: usize, blob_len: usize) -> Result<Sections> {
    let blob_start = r.at.checked_add(counts.conses.checked_mul(16).ok_or("frozen region count out of range")?)
        .and_then(|at| at.checked_add(entries.checked_mul(16)?)).ok_or("frozen region count out of range")?;
    if blob_start.checked_add(blob_len) != Some(r.bytes.len()) { return Err("invalid frozen region lengths".into()); }
    let mut pairs = Vec::with_capacity(counts.conses);
    for _ in 0..counts.conses { pairs.push((Edge::parse(r.u64()?, counts)?, Edge::parse(r.u64()?, counts)?)); }
    let mut table = Vec::with_capacity(entries);
    for _ in 0..entries {
        let offset = r.count()?;
        let word = r.u64()?;
        let len = (word & !(1 << 63)) as usize;
        let end = offset.checked_add(len).filter(|&end| end <= blob_len).ok_or("frozen region table out of range")?;
        table.push((blob_start + offset, blob_start + end, word >> 63 == 1));
    }
    Ok((pairs, table))
}

/// The cons pairs and the entry table of an encoding (`sections`).
type Sections = (Vec<(Edge, Edge)>, Vec<(usize, usize, bool)>);

fn symbol_entry(bytes: &[u8], interned: bool) -> Result<(String, bool)> {
    let name = std::str::from_utf8(bytes).map_err(|_| "frozen symbol name is not UTF-8")?;
    Ok((name.to_owned(), interned))
}

/// The object BYTES encode, its edges within COUNTS.
fn decode_object(bytes: &[u8], counts: &Counts) -> Result<DecodedObject> {
    let mut r = Reader { bytes, at: 0 };
    let edge = |word: u64| Edge::parse(word, counts);
    let object = match r.u8()? {
        KIND_STRING => decode_string(&mut r, &edge)?,
        kind @ (KIND_VECTOR | KIND_RECORD) => {
            let n = r.count()?;
            let items = (0..n).map(|_| edge(r.u64()?)).collect::<Result<Vec<_>>>()?;
            if kind == KIND_RECORD && items.is_empty() { return Err("frozen record is empty".into()); }
            if kind == KIND_VECTOR { DecodedObject::Vector(items) } else { DecodedObject::Record(items) }
        }
        KIND_FLOAT => DecodedObject::Float(f64::from_bits(r.u64()?)),
        KIND_BIGNUM => DecodedObject::Bignum(r.bytes()?.to_vec()),
        KIND_BOOL_VECTOR => {
            let len = usize::try_from(r.u64()?).map_err(|_| "frozen bool-vector length overflow")?;
            let bits = r.bytes()?.to_vec();
            if bits.len() != len.div_ceil(8) { return Err("frozen bool-vector length mismatch".into()); }
            DecodedObject::BoolVector { len, bytes: bits }
        }
        KIND_HASH_TABLE => {
            let test = match r.u8()? { 0 => hash::Test::Eq, 1 => hash::Test::Eql, 2 => hash::Test::Equal, _ => return Err("frozen hash table test is invalid".into()) };
            let n = r.count()?;
            let entries = (0..n).map(|_| Ok((edge(r.u64()?)?, edge(r.u64()?)?))).collect::<Result<Vec<_>>>()?;
            DecodedObject::HashTable { test, entries }
        }
        KIND_CHAR_TABLE => decode_char_table(&mut r, &edge)?,
        _ => return Err("frozen object kind is invalid".into()),
    };
    if !r.done() { return Err("frozen object has trailing bytes".into()); }
    Ok(object)
}

fn decode_string(r: &mut Reader, edge: &impl Fn(u64) -> Result<Edge>) -> Result<DecodedObject> {
    let flags = r.u8()?;
    let (multibyte, object) = (flags & STRING_MULTIBYTE != 0, flags & STRING_OBJECT != 0);
    let text = r.bytes()?.to_vec();
    let mut runs = vec![];
    for _ in 0..r.u32()? {
        let (from, to) = (r.count()?, r.count()?);
        if from > to || to > text.len() { return Err("frozen string property run out of range".into()); }
        let mut values = vec![];
        for _ in 0..r.u32()? {
            let key = std::str::from_utf8(r.bytes()?).map_err(|_| "frozen property name is not UTF-8")?.to_owned();
            values.push((key, edge(r.u64()?)?));
        }
        runs.push((from, to, values));
    }
    if multibyte && !internal_encoding(&text) { return Err("frozen multibyte string is not internal encoding".into()); }
    Ok(DecodedObject::String { multibyte, object, bytes: text, runs })
}

fn decode_char_table(r: &mut Reader, edge: &impl Fn(u64) -> Result<Edge>) -> Result<DecodedObject> {
    let (purpose, default, parent) = (edge(r.u64()?)?, edge(r.u64()?)?, edge(r.u64()?)?);
    let extras = (0..r.u32()?).map(|_| edge(r.u64()?)).collect::<Result<Vec<_>>>()?;
    let n = r.count()?;
    let mut runs = Vec::with_capacity(n);
    let mut next = 0;
    for _ in 0..n {
        let (from, to) = (r.u64()? as usize, r.u64()? as usize);
        if from != next || to < from || to >= char_table::CharTable::LEN { return Err("frozen char-table runs are not a partition".into()); }
        runs.push((from, to, edge(r.u64()?)?));
        next = to + 1;
    }
    if next != char_table::CharTable::LEN { return Err("frozen char-table runs are not a partition".into()); }
    Ok(DecodedObject::CharTable { purpose, default, parent, extras, runs })
}

/// Whether BYTES are a sequence of characters in GNU's internal encoding.
fn internal_encoding(bytes: &[u8]) -> bool {
    let mut rest = bytes;
    while !rest.is_empty() {
        match crate::lstring::decode_char(rest) {
            Some((_, width)) => rest = &rest[width..],
            None => return false,
        }
    }
    true
}

/// A decoded graph's conses and objects as values (`Vm::place`).
type Placed = (Vec<(Value, Value)>, Vec<Object>);

/// Where decoded objects are placed: a region, or the private heap at the
/// bases its arenas had when decoding began (nothing collects meanwhile).
#[derive(Clone, Copy)]
enum Place { Region(usize), Private { conses: usize, objects: usize } }

impl Place {
    fn value(self, edge: Edge, symbols: &[Value]) -> Value {
        match (self, edge) {
            (_, Edge::Immediate(v)) => v,
            (_, Edge::Symbol(i)) => symbols[i],
            (Place::Region(id), Edge::Cons(i)) => frozen_value(id, i, 3),
            (Place::Region(id), Edge::Object(i)) => frozen_value(id, i + 1, 0),
            (Place::Private { conses, .. }, Edge::Cons(i)) => Value::reference(conses + i, 3),
            (Place::Private { objects, .. }, Edge::Object(i)) => Value::reference(objects + i + 1, 0),
            (_, Edge::Unbound) => Value::UNBOUND,
            (_, Edge::Closure(_) | Edge::Cell(_) | Edge::Subr(_)) => unreachable!("a data graph's edges parse with no code counts"),
        }
    }
}

impl Vm {
    /// The path of a refusal as Lisp: `car`, `cdr`, `(aref N)`, `(key N)`,
    /// `(value N)`, `(property N)`, `purpose`, `default`, `parent`,
    /// `(extra N)`, `(range FROM)`; in a code graph `(capture N)`,
    /// `(metadata N)`, `document`, `interactive`, `(constant N)`, `cell` and
    /// `body`.
    pub(crate) fn refusal_path(&mut self, path: &[Label]) -> Value {
        let mut items = vec![];
        for label in path {
            let (name, n) = match *label {
                Label::Root => continue,
                Label::Car => ("car", None),
                Label::Cdr => ("cdr", None),
                Label::Aref(n) => ("aref", Some(n)),
                Label::Key(n) => ("key", Some(n)),
                Label::Val(n) => ("value", Some(n)),
                Label::Property(n) => ("property", Some(n)),
                Label::Purpose => ("purpose", None),
                Label::Default => ("default", None),
                Label::Parent => ("parent", None),
                Label::Extra(n) => ("extra", Some(n)),
                Label::Range(n) => ("range", Some(n)),
                Label::Capture(n) => ("capture", Some(n)),
                Label::Metadata(n) => ("metadata", Some(n)),
                Label::Document => ("document", None),
                Label::Interactive => ("interactive", None),
                Label::Constant(n) => ("constant", Some(n)),
                Label::Cell => ("cell", None),
                Label::Body => ("body", None),
            };
            let symbol = self.intern(name);
            items.push(match n {
                None => symbol,
                Some(n) => { let n = Value::integer(n as i64).expect("index fits"); self.list_value(&[symbol, n]) }
            });
        }
        self.list_value(&items)
    }

    /// Whether V is frozen: in a region this module made (never the
    /// executable image, whose values are `Fixed`).
    pub(crate) fn frozen(v: Value) -> bool {
        Self::image_value(v) && region_of(v.payload() as usize) != 0
    }

    /// V's region, if it is in one this module made.
    pub(crate) fn region(&self, v: Value) -> Option<&Region> {
        if !Self::image_value(v) { return None; }
        self.regions.get(region_of(v.payload() as usize))?.as_deref()
    }

    pub(crate) fn frozen_object(&self, payload: usize) -> Option<&Object> {
        let id = region_of(payload);
        if id == 0 { return None; }
        self.regions.get(id)?.as_ref()?.objects.get((payload & INDEX_MASK).checked_sub(1)?)
    }

    pub(crate) fn frozen_cons(&self, payload: usize) -> Result<(Value, Value)> {
        let id = region_of(payload);
        self.regions.get(id).and_then(|r| r.as_ref()).ok_or("frozen region is not attached")?
            .conses.get(payload & INDEX_MASK).copied().ok_or_else(|| "frozen cons index out of range".into())
    }

    /// GNU's `CHECK_IMPURE` error for a write to a frozen object: the signal
    /// is left pending and the text returned for the refusing primitive.
    pub(crate) fn refuse_read_only(&mut self, object: Value) -> String {
        let (condition, data) = self.read_only_signal(object);
        self.propagated_escape = Some(threaded::Escape::Signal(condition, data));
        READ_ONLY.into()
    }

    /// The same signal as a leaf's fault, for writers that return one.
    pub(crate) fn read_only_fault(&mut self, object: Value) -> prim::Fault {
        let (condition, data) = self.read_only_signal(object);
        prim::Fault::Signal(condition, data)
    }

    pub(crate) fn read_only_signal(&mut self, object: Value) -> (Value, Value) {
        let message = self.allocate_object(Object::String(Box::new(data::LispString::new(READ_ONLY))));
        let condition = self.intern("error");
        (condition, self.list_value(&[message, object]))
    }

    pub(crate) fn regions_empty(&self) -> bool {
        self.regions.totals().regions == 0
    }

    /// Live region ids, excluding the executable image.
    pub(crate) fn region_count(&self) -> usize {
        self.regions.totals().regions
    }

    /// The pressure every region puts on the collector (`Region::weight`).
    pub(crate) fn region_weight(&self) -> usize {
        self.regions.totals().weight
    }

    pub(crate) fn region_conses(&self) -> usize {
        self.regions.totals().conses
    }

    /// ROOT's graph as bytes of the encoding, or the first refusal.
    pub(crate) fn encode_frozen(&self, root: Value) -> std::result::Result<Vec<u8>, Refusal> {
        let mut walk = Walk::new(self);
        walk.discover(root)?;
        Ok(walk.encode(root))
    }

    /// Freeze ROOT: the same value when it is a region's root already, else
    /// an immutable copy in a new region.
    pub(crate) fn freeze(&mut self, root: Value) -> std::result::Result<Value, FreezeError> {
        if Self::frozen(root) && self.region(root).is_some_and(|r| r.root == root) { return Ok(root); }
        if matches!(root.tag(), 1) || root == Value::NIL || root == Value::TRUE { return Ok(root); }
        let bytes = self.encode_frozen(root).map_err(FreezeError::Refused)?;
        let backing = Backing::new(bytes).map_err(|e| FreezeError::Failed(e.to_string()))?;
        self.attach(backing).map_err(FreezeError::Failed)
    }

    /// Thaw ROOT: a private, mutable copy of a frozen graph; a private value
    /// is returned as it is.
    pub(crate) fn thaw(&mut self, root: Value) -> std::result::Result<Value, FreezeError> {
        if !Self::frozen(root) { return Ok(root); }
        let bytes = self.encode_frozen(root).map_err(FreezeError::Refused)?;
        let decoded = decode(&bytes).map_err(FreezeError::Failed)?;
        let symbols = self.resolve_symbols(&decoded);
        let place = Place::Private { conses: self.conses.len(), objects: self.objects.len() };
        let value = |edge: Edge| place.value(edge, &symbols);
        self.place_private(&decoded, &value).map_err(FreezeError::Failed)?;
        Ok(value(decoded.root))
    }

    /// Place DECODED's conses and objects in the private heap, appended to
    /// its arenas, VALUE resolving each edge (its conses and objects at the
    /// arenas' lengths now): a private copy, charged as one allocation.
    fn place_private(&mut self, decoded: &Decoded, value: &dyn Fn(Edge) -> Value) -> Result<()> {
        let base = self.objects.len();
        let (conses, objects) = self.place(decoded, value)?;
        let tables: Vec<usize> = objects.iter().enumerate().filter(|(_, o)| matches!(o, Object::HashTable(_))).map(|(i, _)| i).collect();
        let profiled: usize = objects.iter().map(crate::data::profiled_bytes).sum();
        self.allocation.allocated += (conses.len() * size_of::<(Value, Value)>() + profiled) as u64;
        self.allocation.consed.conses += conses.len() as u64;
        objects.iter().for_each(|object| self.allocation.consed.count(object));
        self.conses.extend(conses);
        self.charge_payload(objects.iter().map(crate::data::payload_bytes).sum());
        self.objects.extend(objects);
        for i in tables {
            let DecodedObject::HashTable { test, entries } = &decoded.objects[i] else { unreachable!() };
            let entries: Vec<(Value, Value)> = entries.iter().map(|&(k, v)| (value(k), value(v))).collect();
            let table = self.table_from_entries(*test, &entries);
            // Its placeholder was charged empty (above).
            let empty = match &self.objects[base + i] { Object::HashTable(t) => t.storage_bytes(false), _ => 0 };
            let filled = table.storage_bytes(false).saturating_sub(empty);
            self.allocation.allocated += filled as u64;
            self.allocation.consed.vector_cells += (filled / size_of::<Value>()) as u64;
            *self.objects.entry_mut(base + i) = Object::HashTable(Box::new(table));
        }
        Ok(())
    }

    /// Attach BACKING as a new region: its root value. Signals when every
    /// id is live. The region raises the collector's count through
    /// `region_weight`, not `Allocation::allocated`, so it asks the engine
    /// for no poll: the budget's poll bounds what it makes due.
    pub(crate) fn attach(&mut self, backing: Backing) -> Result<Value> {
        let bytes = backing.bytes().map_err(|e| e.to_string())?;
        let decoded = decode(&bytes)?;
        let length = bytes.len();
        drop(bytes);
        self.attach_decoded(&decoded, length, backing).map_err(|(Taken::Full(e) | Taken::Malformed(e))| e)
    }

    /// What a frame's BACKING is taken as (O7m §4.2): a data graph
    /// attached as a region, a code graph placed as private copies
    /// (`code.rs`); a full region table, which a later take may find room
    /// in; or malformed bytes, which no take ever reads.
    pub(crate) fn take_backing(&mut self, backing: &Backing) -> std::result::Result<Value, Taken> {
        let bytes = backing.bytes().map_err(|e| Taken::Malformed(e.to_string()))?;
        if code::is_code(&bytes) {
            return self.place_code(&Arc::new(bytes.into_owned())).map_err(Taken::Malformed);
        }
        let decoded = decode(&bytes).map_err(Taken::Malformed)?;
        let length = bytes.len();
        drop(bytes);
        self.attach_decoded(&decoded, length, backing.clone())
    }

    fn attach_decoded(&mut self, decoded: &Decoded, length: usize, backing: Backing) -> std::result::Result<Value, Taken> {
        let Some(id) = self.region_free.pop().or_else(|| self.regions.grow()) else {
            return Err(Taken::Full(format!("Too many frozen regions {}", MAX_REGIONS - 1)));
        };
        let place = Place::Region(id);
        let symbols = self.resolve_symbols(decoded);
        let (conses, objects) = match self.place(decoded, &|edge| place.value(edge, &symbols)) {
            Ok(placed) => placed,
            Err(e) => { self.region_free.push(id); return Err(Taken::Malformed(e)); }
        };
        let root = place.value(decoded.root, &symbols);
        self.regions.place(id, Box::new(Region { conses, objects, symbols, root, bytes: length, backing }));
        self.region_young.push(id);
        // Hash tables hash their keys with this VM's function once their keys
        // are readable, which needs the region in place.
        let tables: Vec<usize> = decoded.objects.iter().enumerate().filter(|(_, o)| matches!(o, DecodedObject::HashTable { .. })).map(|(i, _)| i).collect();
        for i in tables {
            let DecodedObject::HashTable { test, entries } = &decoded.objects[i] else { unreachable!() };
            let symbols = &self.regions[id].as_ref().expect("placed region").symbols;
            let entries: Vec<(Value, Value)> = entries.iter().map(|&(k, v)| (place.value(k, symbols), place.value(v, symbols))).collect();
            let table = self.table_from_entries(*test, &entries);
            self.regions.set_object(id, i, Object::HashTable(Box::new(table)));
        }
        if self.region_count() >= COLLECT_AT { self.region_collect = true; }
        Ok(root)
    }

    /// This VM's symbol for each of DECODED's descriptors: interned by
    /// name, or one fresh uninterned symbol per descriptor.
    fn resolve_symbols(&mut self, decoded: &Decoded) -> Vec<Value> {
        decoded.symbols.iter()
            .map(|(name, interned)| if *interned { self.intern(name) } else { self.uninterned_symbol(name) })
            .collect()
    }

    /// DECODED's conses and objects as this VM's values, VALUE resolving
    /// each edge. Hash tables are placed empty: their index is built once
    /// their keys can be read.
    fn place(&mut self, decoded: &Decoded, value: &dyn Fn(Edge) -> Value) -> Result<Placed> {
        let conses = decoded.conses.iter().map(|&(a, b)| (value(a), value(b))).collect();
        let mut objects = Vec::with_capacity(decoded.objects.len());
        for object in &decoded.objects {
            objects.push(match object {
                DecodedObject::String { multibyte, object, bytes, runs } => {
                    if !object {
                        Object::Bytes(bytes.clone())
                    } else {
                        let mut string = data::LispString::from_bytes(bytes.clone(), *multibyte);
                        for (from, to, values) in runs {
                            for (key, edge) in values {
                                string.properties.put(coordinates::ByteOffset(*from)..coordinates::ByteOffset(*to), self.intern_property_key(key), value(*edge));
                            }
                        }
                        if string.is_utf8() { Object::String(Box::new(string)) } else { Object::RawString(Box::new(string)) }
                    }
                }
                DecodedObject::Vector(items) => Object::Vector(items.iter().map(|&e| value(e)).collect()),
                DecodedObject::Record(items) => Object::Record(items.iter().map(|&e| value(e)).collect()),
                DecodedObject::Float(f) => Object::Float(*f),
                DecodedObject::Bignum(bytes) => Object::Bignum(Box::new(num_bigint::BigInt::from_signed_bytes_le(bytes))),
                DecodedObject::BoolVector { len, bytes } => {
                    let mut vector = data::BoolVector::new(*len, false)?;
                    vector.bytes.copy_from_slice(bytes);
                    vector.mask_padding();
                    Object::BoolVector(Box::new(vector))
                }
                DecodedObject::HashTable { test, .. } => Object::HashTable(Box::new(hash::HashTable::new(*test, hash::Weakness::None, 0))),
                DecodedObject::CharTable { purpose, default, parent, extras, runs } => {
                    let mut table = char_table::CharTable::new(value(*purpose));
                    table.initialize(extras.len(), Value::NIL)?;
                    for (slot, &e) in extras.iter().enumerate() { table.set_extra(slot, value(e)); }
                    table.set_default(value(*default));
                    table.set_parent(value(*parent));
                    for &(from, to, e) in runs { table.set_range(from, to, value(e))?; }
                    Object::CharTable(Box::new(table))
                }
            });
        }
        Ok((conses, objects))
    }

    /// The region ids a collection found unmarked are dropped and reused.
    pub(crate) fn sweep_regions(&mut self, live: &[bool]) {
        for id in 1..self.regions.len() {
            if self.regions[id].is_some() && !live.get(id).copied().unwrap_or(false) {
                self.regions.remove(id);
                self.region_free.push(id);
            }
        }
        self.region_young.clear();
        self.region_collect = false;
    }

    /// A minor's: the regions attached since the last collection that LIVE
    /// leaves unmarked are dropped and their ids reused, each with its memfd.
    /// No older value can name one: a value of it reached an older entry, a
    /// symbol slot or text only by a store since then, which the minor read.
    /// The older regions are a full's to judge.
    pub(crate) fn sweep_young_regions(&mut self, live: &[bool]) {
        for id in std::mem::take(&mut self.region_young) {
            if self.regions[id].is_some() && !live.get(id).copied().unwrap_or(false) {
                self.regions.remove(id);
                self.region_free.push(id);
            }
        }
    }

    /// The symbols a live region keeps alive: roots for the collector.
    pub(crate) fn region_symbols(&self, id: usize) -> &[Value] {
        self.regions.get(id).and_then(|r| r.as_deref()).map_or(&[], |r| &r.symbols)
    }

    /// The backing of a frozen value that is its region's root, or of a
    /// frozen backing (`romacs--seed-build`), for sending without a second
    /// encoding.
    pub(crate) fn frozen_backing(&self, v: Value) -> Option<Backing> {
        if let Some(Object::Backing(backing)) = self.object_if(v) {
            return Some(backing.clone());
        }
        self.region(v).filter(|r| r.root == v).map(|r| r.backing.clone())
    }
}

#[cfg(feature = "test-hooks")]
impl Vm {
    /// Place the code graph BYTES as an isolate's take would: its root, a
    /// private copy.
    ///
    /// # Errors
    ///
    /// When BYTES are no code graph, or name what this VM lacks.
    pub fn place_seed(&mut self, bytes: &[u8]) -> Result<Local<'_>> {
        let root = self.place_code(&Arc::new(bytes.to_vec()))?;
        Ok(Local::new(self, root))
    }
}

#[cfg(feature = "test-hooks")]
impl Vm {
    /// A copy of the bytes of the frozen backing V.
    pub(crate) fn backing_bytes(&self, v: Value) -> Result<Vec<u8>> {
        match self.object_if(v) {
            Some(Object::Backing(backing)) => Ok(backing.bytes().map_err(|e| e.to_string())?.into_owned()),
            _ => Err("not a frozen backing".into()),
        }
    }
    /// How the closure V holds its body: `eager`, `pending` (a
    /// compiled-Lisp body), `mapped` (a code image's) or `lazy`.
    pub(crate) fn function_body_kind(&self, v: Value) -> Result<&'static str> {
        let closure = v.index_if(4).and_then(|i| self.closures.get(i)).ok_or("not a closure")?;
        let f = self.functions.get(closure.function).ok_or("invalid function")?;
        Ok(if f.lazy.is_some() { "lazy" } else if f.pending.is_some() { "pending" } else if f.mapped_code.is_some() { "mapped" } else { "eager" })
    }
}

/// Why a frame's backing was not taken (`Vm::take_backing`).
pub(crate) enum Taken {
    /// Every region id is live: the frame waits for a later take.
    Full(String),
    /// The bytes are no value graph: the frame is spent.
    Malformed(String),
}

/// Why a freeze, thaw or send did not happen.
pub(crate) enum FreezeError {
    Refused(Refusal),
    Failed(String),
    /// The peer's queue (its socket buffer) is full (`isolate.rs`).
    QueueFull,
    /// The peer is gone (`isolate.rs`).
    PeerGone,
}

impl Vm {
    /// FREEZE_ERROR as the signal GNU code sees: `(error "Cannot freeze" OBJ
    /// PATH)` for a refusal, `(error TEXT)` otherwise.
    pub(crate) fn freeze_fault(&mut self, error: FreezeError) -> prim::Fault {
        match error {
            FreezeError::Refused(refusal) => {
                let message = self.allocate_object(Object::String(Box::new(data::LispString::new("Cannot freeze"))));
                let path = self.refusal_path(&refusal.path);
                let condition = self.intern("error");
                let data = self.list_value(&[message, refusal.object, path]);
                prim::Fault::Signal(condition, data)
            }
            FreezeError::Failed(text) => prim::Fault::Error(text),
            FreezeError::QueueFull => { let c = self.intern("isolate-queue-full"); prim::Fault::Signal(c, Value::NIL) }
            FreezeError::PeerGone => { let c = self.intern("isolate-died"); prim::Fault::Signal(c, Value::NIL) }
        }
    }
}

#[cfg(test)]
#[path = "frozen_tests.rs"]
mod frozen_tests;
