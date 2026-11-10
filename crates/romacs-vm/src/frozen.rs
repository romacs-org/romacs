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
//! a traced value names it; the region's symbols are roots while it lives;
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
/// objects region-relative, symbols by descriptor.
#[derive(Clone, Copy)]
enum Edge { Immediate(Value), Cons(usize), Object(usize), Symbol(usize) }

impl Edge {
    fn word(self) -> u64 {
        match self {
            Edge::Immediate(v) => v.0,
            Edge::Cons(i) => ((i as u64) << 3) | 3,
            Edge::Object(i) => (i as u64 + 1) << 3,
            Edge::Symbol(i) => ((i as u64 + 1) << 3) | 7,
        }
    }
    fn parse(word: u64, conses: usize, objects: usize, symbols: usize) -> Result<Self> {
        let v = Value(word);
        let index = (word >> 3) as usize;
        match word & 7 {
            1 | 5 => Ok(Edge::Immediate(v)),
            0 if word == 0 => Ok(Edge::Immediate(Value::NIL)),
            2 if word == 2 => Ok(Edge::Immediate(Value::TRUE)),
            3 if index < conses => Ok(Edge::Cons(index)),
            0 if index >= 1 && index <= objects => Ok(Edge::Object(index - 1)),
            7 if index >= 1 && index <= symbols => Ok(Edge::Symbol(index - 1)),
            _ => Err("frozen region edge is invalid".into()),
        }
    }
}

/// Where a graph walk is: the edge from a parent to the value at hand, for
/// the path a refusal names.
#[derive(Clone, Copy)]
pub(crate) enum Label {
    Root, Car, Cdr, Aref(usize), Key(usize), Val(usize), Property(usize), Purpose, Default, Parent, Extra(usize), Range(usize),
}

struct Node { value: Value, parent: Option<usize>, label: Label }

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
            Object::CharTable(t) => !self.symbol_named(t.purpose, "syntax-table"),
            // A table on a test `define-hash-table-test' named stays with
            // its isolate, as the test's definition does.
            Object::HashTable(t) => t.weakness == hash::Weakness::None && !t.local_bindings
                && !matches!(t.test, hash::Test::User(..)),
            Object::SyntaxTable(_) | Object::Window(_) | Object::Frame(_) | Object::Terminal(_) | Object::WindowConfiguration(_)
            | Object::Sqlite(_) | Object::Treesit(_) | Object::Process(_) | Object::Finalizer(_) | Object::Thread | Object::SubCharTable(_) | Object::MapKeymapCursor(_)
            | Object::Buffer(_) | Object::Marker(_) | Object::Overlay(_) | Object::ByteBuffer(_) | Object::Obarray(_)
            | Object::Free => false,
        }
    }

    /// Discover every node reachable from ROOT, assigning indices.
    fn discover(&mut self, root: Value) -> std::result::Result<(), Refusal> {
        let mut pending = vec![(root, None, Label::Root)];
        while let Some((value, parent, label)) = pending.pop() {
            match value.tag() {
                1 => continue,
                0 if value == Value::NIL => continue,
                2 if value == Value::TRUE => continue,
                7 => {
                    if value == Value::UNBOUND || self.symbol_memo.contains_key(&value.0) { continue; }
                    let slot = value.payload() as usize - 1;
                    let interned = self.vm.initial_obarray.get(slot).copied().unwrap_or(false);
                    let name = self.vm.symbol_names.get(slot).cloned().unwrap_or_default();
                    self.symbol_memo.insert(value.0, self.symbols.len());
                    self.symbols.push((name, interned));
                }
                3 => {
                    if self.memo.contains_key(&value.0) { continue; }
                    let node = self.nodes.len();
                    self.nodes.push(Node { value, parent, label });
                    // A frozen graph is closed, so a cons reached through a
                    // frozen value is always readable.
                    let Ok((car, cdr)) = self.vm.cons_pair(value) else { return Err(self.refusal(node)) };
                    self.memo.insert(value.0, Edge::Cons(self.cons_nodes.len()));
                    self.cons_nodes.push(node);
                    pending.push((cdr, Some(node), Label::Cdr));
                    pending.push((car, Some(node), Label::Car));
                }
                0 => {
                    if self.memo.contains_key(&value.0) { continue; }
                    let node = self.nodes.len();
                    self.nodes.push(Node { value, parent, label });
                    let Some(object) = self.vm.object_if(value).filter(|o| self.freezable(o)) else { return Err(self.refusal(node)) };
                    self.memo.insert(value.0, Edge::Object(self.object_nodes.len()));
                    self.object_nodes.push(node);
                    match object {
                        Object::String(s) | Object::RawString(s) => {
                            for (i, (_, values)) in s.properties.runs(coordinates::ByteOffset(0)..coordinates::ByteOffset(s.bytes().len())).into_iter().enumerate() {
                                for (_, v) in values { pending.push((v, Some(node), Label::Property(i))); }
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
                            pending.push((t.purpose, Some(node), Label::Purpose));
                            pending.push((t.default, Some(node), Label::Default));
                            pending.push((t.parent, Some(node), Label::Parent));
                            for (i, &v) in t.extras.iter().enumerate() { pending.push((v, Some(node), Label::Extra(i))); }
                            for (from, _, v) in char_table_runs(t).map_err(|_| self.refusal(node))? {
                                pending.push((v, Some(node), Label::Range(from)));
                            }
                        }
                        _ => {}
                    }
                }
                _ => {
                    // Closures, lexical cells and subrs are per VM.
                    let node = self.nodes.len();
                    self.nodes.push(Node { value, parent, label });
                    return Err(self.refusal(node));
                }
            }
        }
        Ok(())
    }

    fn edge(&self, v: Value) -> Edge {
        match v.tag() {
            7 if v != Value::UNBOUND => Edge::Symbol(self.symbol_memo[&v.0]),
            3 | 0 if !(v == Value::NIL) => self.memo[&v.0],
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
                        w.bytes(key.as_bytes());
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
                w.u64(self.edge(t.purpose).word());
                w.u64(self.edge(t.default).word());
                w.u64(self.edge(t.parent).word());
                w.u32(t.extras.len() as u32);
                for &v in &t.extras { w.u64(self.edge(v).word()); }
                let runs = char_table_runs(t).expect("discovered char-table");
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

/// A char-table's raw entries as maximal runs of its own subdivision:
/// (from, to, value), with nil where it has no value of its own.
fn char_table_runs(t: &char_table::CharTable) -> Result<Vec<(usize, usize, Value)>> {
    let mut runs = vec![];
    let mut at = 0;
    while at < char_table::CharTable::LEN {
        let (value, from, to) = char_table::CharTableNode::checked_span(&t.root, at)?;
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
    let edge = |word: u64| Edge::parse(word, conses, objects, symbols);
    let root = edge(root_word)?;
    let tables = HEADER + 16 * conses;
    let blob_start = tables + 16 * (objects + symbols);
    if blob_start.checked_add(blob_len) != Some(bytes.len()) { return Err("invalid frozen region lengths".into()); }
    let mut pairs = Vec::with_capacity(conses);
    for _ in 0..conses { pairs.push((edge(r.u64()?)?, edge(r.u64()?)?)); }
    let mut entries = Vec::with_capacity(objects + symbols);
    for _ in 0..objects + symbols {
        let offset = r.count()?;
        let word = r.u64()?;
        let len = (word & !(1 << 63)) as usize;
        let end = offset.checked_add(len).filter(|&end| end <= blob_len).ok_or("frozen region table out of range")?;
        entries.push((blob_start + offset, blob_start + end, word >> 63 == 1));
    }
    let mut decoded_objects = Vec::with_capacity(objects);
    for &(start, end, _) in &entries[..objects] {
        let mut r = Reader { bytes: &bytes[start..end], at: 0 };
        let object = match r.u8()? {
            KIND_STRING => {
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
                DecodedObject::String { multibyte, object, bytes: text, runs }
            }
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
            KIND_CHAR_TABLE => {
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
                DecodedObject::CharTable { purpose, default, parent, extras, runs }
            }
            _ => return Err("frozen object kind is invalid".into()),
        };
        if !r.done() { return Err("frozen object has trailing bytes".into()); }
        decoded_objects.push(object);
    }
    let mut names = Vec::with_capacity(symbols);
    for &(start, end, interned) in &entries[objects..] {
        let name = std::str::from_utf8(&bytes[start..end]).map_err(|_| "frozen symbol name is not UTF-8")?;
        names.push((name.to_owned(), interned));
    }
    Ok(Decoded { conses: pairs, objects: decoded_objects, symbols: names, root })
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
        }
    }
}

impl Vm {
    /// The path of a refusal as Lisp: `car`, `cdr`, `(aref N)`, `(key N)`,
    /// `(value N)`, `(property N)`, `purpose`, `default`, `parent`,
    /// `(extra N)`, `(range FROM)`.
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
        self.regions.iter().all(Option::is_none)
    }

    /// Live region ids, excluding the executable image.
    pub(crate) fn region_count(&self) -> usize {
        self.regions.iter().skip(1).filter(|r| r.is_some()).count()
    }

    /// The pressure every region puts on the collector, in objects: its
    /// conses and objects, plus one per 16 encoded bytes.
    pub(crate) fn region_weight(&self) -> usize {
        self.regions.iter().flatten().map(|r| r.conses.len() + r.objects.len() + r.bytes / 16).sum()
    }

    pub(crate) fn region_conses(&self) -> usize {
        self.regions.iter().flatten().map(|r| r.conses.len()).sum()
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
        let (conses, objects) = self.place(&decoded, place, &symbols).map_err(FreezeError::Failed)?;
        let tables: Vec<usize> = objects.iter().enumerate().filter(|(_, o)| matches!(o, Object::HashTable(_))).map(|(i, _)| i).collect();
        let profiled: usize = objects.iter().map(crate::data::profiled_bytes).sum();
        self.allocation.allocated += (conses.len() * size_of::<(Value, Value)>() + profiled) as u64;
        self.allocation.consed.conses += conses.len() as u64;
        objects.iter().for_each(|object| self.allocation.consed.count(object));
        self.conses.extend(conses);
        self.charge_payload(objects.iter().map(crate::data::payload_bytes).sum());
        self.objects.extend(objects);
        let Place::Private { objects: base, .. } = place else { unreachable!() };
        for i in tables {
            let DecodedObject::HashTable { test, entries } = &decoded.objects[i] else { unreachable!() };
            let entries: Vec<(Value, Value)> = entries.iter().map(|&(k, v)| (place.value(k, &symbols), place.value(v, &symbols))).collect();
            let table = self.table_from_entries(*test, &entries);
            // Its placeholder was charged empty (above).
            let empty = match &self.objects[base + i] { Object::HashTable(t) => t.storage_bytes(false), _ => 0 };
            let filled = table.storage_bytes(false).saturating_sub(empty);
            self.allocation.allocated += filled as u64;
            self.allocation.consed.vector_cells += (filled / size_of::<Value>()) as u64;
            *self.objects.entry_mut(base + i) = Object::HashTable(Box::new(table));
        }
        Ok(place.value(decoded.root, &symbols))
    }

    /// Attach BACKING as a new region: its root value. Signals when every
    /// id is live.
    pub(crate) fn attach(&mut self, backing: Backing) -> Result<Value> {
        let bytes = backing.bytes().map_err(|e| e.to_string())?;
        let decoded = decode(&bytes)?;
        let length = bytes.len();
        drop(bytes);
        let id = match self.region_free.pop() {
            Some(id) => id,
            None if self.regions.len() < MAX_REGIONS => { self.regions.push(None); self.regions.len() - 1 }
            None => return Err(format!("Too many frozen regions {}", MAX_REGIONS - 1)),
        };
        let place = Place::Region(id);
        let symbols = self.resolve_symbols(&decoded);
        let (conses, objects) = match self.place(&decoded, place, &symbols) {
            Ok(placed) => placed,
            Err(e) => { self.region_free.push(id); return Err(e); }
        };
        let root = place.value(decoded.root, &symbols);
        self.regions[id] = Some(Box::new(Region { conses, objects, symbols, root, bytes: length, backing }));
        // Hash tables hash their keys with this VM's function once their keys
        // are readable, which needs the region in place.
        let tables: Vec<usize> = decoded.objects.iter().enumerate().filter(|(_, o)| matches!(o, DecodedObject::HashTable { .. })).map(|(i, _)| i).collect();
        for i in tables {
            let DecodedObject::HashTable { test, entries } = &decoded.objects[i] else { unreachable!() };
            let symbols = &self.regions[id].as_ref().expect("placed region").symbols;
            let entries: Vec<(Value, Value)> = entries.iter().map(|&(k, v)| (place.value(k, symbols), place.value(v, symbols))).collect();
            let table = self.table_from_entries(*test, &entries);
            self.regions[id].as_mut().expect("placed region").objects[i] = Object::HashTable(Box::new(table));
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

    /// DECODED's conses and objects as this VM's values at PLACE, with
    /// SYMBOLS resolved. Hash tables are placed empty: their index is built
    /// once their keys can be read.
    fn place(&mut self, decoded: &Decoded, place: Place, symbols: &[Value]) -> Result<(Vec<(Value, Value)>, Vec<Object>)> {
        let value = |edge: Edge| place.value(edge, symbols);
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
                                string.properties.put(coordinates::ByteOffset(*from)..coordinates::ByteOffset(*to), key, value(*edge));
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
                    for (slot, &e) in extras.iter().enumerate() { table.extras[slot] = value(e); }
                    table.default = value(*default);
                    table.parent = value(*parent);
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
                self.regions[id] = None;
                self.region_free.push(id);
            }
        }
        self.region_collect = false;
    }

    /// The symbols a live region keeps alive: roots for the collector.
    pub(crate) fn region_symbols(&self, id: usize) -> &[Value] {
        self.regions.get(id).and_then(|r| r.as_deref()).map_or(&[], |r| &r.symbols)
    }

    /// The backing of a frozen value that is its region's root, for sending
    /// without a second encoding.
    pub(crate) fn frozen_backing(&self, v: Value) -> Option<Backing> {
        self.region(v).filter(|r| r.root == v).map(|r| r.backing.clone())
    }
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
