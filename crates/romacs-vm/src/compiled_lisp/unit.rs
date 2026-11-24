//! One compiled unit: the VM2 function tree a top-level form compiled to,
//! and the constants its code holds, as bytes; and back into a VM.
//!
//! Constants are a graph of nodes, so sharing and cycles survive. Its
//! functions are encoded by the shared function codec (`function_codec`),
//! which leaves each body pending until its first call; this module says
//! what a value, a symbol, a child function and a cache are in a unit.
//! Interned symbols are indices into the artifact's name table; an
//! uninterned symbol is one of the unit's own, made afresh at each load
//! (GNU's `.elc` reads `#:x` per form). Each function's quoted constants
//! are coalesced by `equal-including-properties`, as bytecomp's
//! `byte-compile-get-constant` coalesces a constant vector's.
use super::codec::{Reader, Writer};
use crate::data::Object;
use crate::function_codec::{self, BodyRefs, CodeRefs, Ref, Symbols, read_ref, read_refs, write_ref};
use crate::*;

/// The artifact's interned symbol names, shared by its units.
#[derive(Default)]
pub(super) struct Names {
    index: HashMap<String, usize>,
    pub(super) names: Vec<String>,
}
impl Names {
    fn of(&mut self, name: &str) -> usize {
        if let Some(&i) = self.index.get(name) {
            return i;
        }
        self.names.push(name.to_owned());
        self.index.insert(name.to_owned(), self.names.len() - 1);
        self.names.len() - 1
    }
}

const NODE_CONS: u8 = 0;
const NODE_FLOAT: u8 = 1;
const NODE_BIGNUM: u8 = 2;
const NODE_STRING: u8 = 3;
const NODE_BYTES: u8 = 4;
const NODE_VECTOR: u8 = 5;
const NODE_RECORD: u8 = 6;
const NODE_BOOL_VECTOR: u8 = 7;
const NODE_CLOSURE: u8 = 8;
const NODE_CELL: u8 = 9;
const NODE_CHAR_TABLE: u8 = 10;

/// Encodes one unit from the VM it was compiled in.
pub(super) struct Encoder<'a> {
    vm: &'a mut Vm,
    names: &'a mut Names,
    uninterned: HashMap<usize, usize>,
    uninterned_names: Vec<String>,
    node_of: HashMap<u64, usize>,
    nodes: Vec<Option<Vec<u8>>>,
    pending: Vec<(usize, Value)>,
    function_of: HashMap<usize, usize>,
    functions: Vec<Option<Vec<u8>>>,
    pending_functions: Vec<(usize, usize)>,
    caches: HashMap<usize, usize>,
}

impl<'a> Encoder<'a> {
    pub(super) fn new(vm: &'a mut Vm, names: &'a mut Names) -> Self {
        Encoder { vm, names, uninterned: HashMap::default(), uninterned_names: vec![], node_of: HashMap::default(),
                  nodes: vec![], pending: vec![], function_of: HashMap::default(), functions: vec![],
                  pending_functions: vec![], caches: HashMap::default() }
    }
    /// The unit whose entry is the function ENTRY.
    pub(super) fn unit(mut self, entry: usize, out: &mut Writer) -> Result<()> {
        let entry = self.function(entry)?;
        loop {
            if let Some((index, id)) = self.pending_functions.pop() {
                let bytes = self.encode_function(id)?;
                self.functions[index] = Some(bytes);
            } else if let Some((index, value)) = self.pending.pop() {
                let bytes = self.encode_node(value)?;
                self.nodes[index] = Some(bytes);
            } else {
                break;
            }
        }
        out.n(self.uninterned_names.len());
        for name in &self.uninterned_names {
            out.str(name);
        }
        out.n(self.nodes.len());
        for node in &self.nodes {
            out.raw(node.as_ref().expect("every node encoded"));
        }
        out.n(self.functions.len());
        for function in &self.functions {
            out.raw(function.as_ref().expect("every function encoded"));
        }
        out.n(entry);
        out.n(self.caches.len());
        Ok(())
    }
    fn function(&mut self, id: usize) -> Result<usize> {
        if let Some(&i) = self.function_of.get(&id) {
            return Ok(i);
        }
        // A body the world holds lazily, or mapped from a code image, is
        // compiled or decoded first, as its first call would.
        self.vm.force_lazy(id)?;
        self.vm.materialize_image_function(id)?;
        if self.vm.functions[id].lazy.is_some() {
            return Err("a lazily held function body".into());
        }
        let index = self.functions.len();
        self.functions.push(None);
        self.function_of.insert(id, index);
        self.pending_functions.push((index, id));
        Ok(index)
    }
    fn symbol_index(&mut self, slot: usize) -> usize {
        if self.vm.generated_slots.contains_key(&slot) || self.vm.obarray_symbol_names.contains_key(&slot) {
            let next = self.uninterned_names.len();
            let index = *self.uninterned.entry(slot).or_insert(next);
            if index == next {
                self.uninterned_names.push(self.vm.symbol_names[slot].clone());
            }
            index << 1 | 1
        } else {
            self.names.of(&self.vm.symbol_names[slot]) << 1
        }
    }
    fn value(&mut self, v: Value) -> Result<Ref> {
        if v == Value::NIL {
            return Ok(Ref::Nil);
        }
        if v == Value::TRUE {
            return Ok(Ref::True);
        }
        Ok(match v.tag() {
            1 => Ref::Fixnum(v.as_integer()?),
            7 if v != Value::UNBOUND => {
                let slot = v.index(7)? - 1;
                let s = self.symbol_index(slot);
                if s & 1 == 1 { Ref::Uninterned(s >> 1) } else { Ref::Name(s >> 1) }
            }
            6 => Ref::Primitive(v.0),
            0 | 3 | 4 | 5 => {
                if let Some(&i) = self.node_of.get(&v.0) {
                    return Ok(Ref::Node(i));
                }
                let i = self.nodes.len();
                self.nodes.push(None);
                self.node_of.insert(v.0, i);
                self.pending.push((i, v));
                Ref::Node(i)
            }
            _ => return Err("an unbound marker".into()),
        })
    }
    fn refs(&mut self, out: &mut Writer, values: &[Value]) -> Result<()> {
        out.n(values.len());
        for &v in values {
            let r = self.value(v)?;
            write_ref(out, r);
        }
        Ok(())
    }
    /// The char-table V: its subtype, default, parent and extra slots, and
    /// its own values as runs, read from V itself (a copy rebuilds its
    /// ASCII slot from its root). Not stored, and so its file is loaded as
    /// source: a Unicode property table, whose blocks decode through its
    /// extra slots; one whose ASCII slot is not its root's (a literal's
    /// detached slot), which runs cannot keep; and one whose parent is no
    /// char-table, which no table made from the runs could have.
    fn char_table(&mut self, v: Value, out: &mut Writer) -> Result<()> {
        let Object::CharTable(t) = self.vm.object(v)? else { return Err("not a char-table".into()) };
        if self.vm.is_uniprop_table(t) {
            return Err(unserialisable("char-table"));
        }
        if t.validate_mapping().is_err() {
            return Err(unserialisable("a char-table whose ASCII slot is not its root's"));
        }
        if t.parent() != Value::NIL && !matches!(self.vm.object(t.parent()), Ok(Object::CharTable(_))) {
            return Err(unserialisable("a char-table whose parent is not a char-table"));
        }
        let (fields, extras, runs) = ([t.purpose(), t.default(), t.parent()], t.extras().to_vec(), t.own_runs()?);
        out.byte(NODE_CHAR_TABLE);
        for field in fields {
            let r = self.value(field)?;
            write_ref(out, r);
        }
        self.refs(out, &extras)?;
        out.n(runs.len());
        for (from, to, value) in runs {
            out.u(u64::from(from));
            out.u(u64::from(to));
            let r = self.value(value)?;
            write_ref(out, r);
        }
        Ok(())
    }
    /// A closure's mutable capture: shared by the unit's closures that hold
    /// it, as in the compile process. Not stored when Lisp holds its lexical
    /// pair (`aref' slot 2): the unit would rebuild the two apart.
    fn cell(&mut self, v: Value, out: &mut Writer) -> Result<()> {
        if self.vm.cell_pairs.get(v.payload() as usize).is_some() {
            return Err(unserialisable("a capture whose lexical pair is shared"));
        }
        let content = self.vm.read_capture_cell(v.payload() as usize);
        out.byte(NODE_CELL);
        let r = self.value(content)?;
        write_ref(out, r);
        Ok(())
    }
    /// A closure: its function, captures, metadata and document. Not stored
    /// when it is a command, whose spec lives in the VM's table, not in the
    /// closure: the unit would rebuild it as no command.
    fn closure(&mut self, v: Value, out: &mut Writer) -> Result<()> {
        if self.vm.interactive_entry(v).is_some() {
            return Err(unserialisable("an interactive closure"));
        }
        let closure = self.vm.closures.get(v.index(4)?).ok_or("invalid closure")?;
        let (function, captures, metadata, document) =
            (closure.function, closure.captures.clone(), closure.metadata.clone(), closure.document);
        out.byte(NODE_CLOSURE);
        let f = self.function(function)?;
        out.n(f);
        self.refs(out, &captures)?;
        self.refs(out, &metadata)?;
        let doc = self.value(document)?;
        write_ref(out, doc);
        Ok(())
    }
    fn encode_node(&mut self, v: Value) -> Result<Vec<u8>> {
        let mut out = Writer::default();
        if v.tag() == 3 {
            let (car, cdr) = self.vm.cons_pair(v)?;
            out.byte(NODE_CONS);
            let (a, b) = (self.value(car)?, self.value(cdr)?);
            write_ref(&mut out, a);
            write_ref(&mut out, b);
            return Ok(out.bytes);
        }
        if v.tag() == 5 {
            self.cell(v, &mut out)?;
            return Ok(out.bytes);
        }
        if v.tag() == 4 {
            self.closure(v, &mut out)?;
            return Ok(out.bytes);
        }
        if Vm::image_string(v) {
            let text = self.vm.string_value(v)?.to_owned();
            out.byte(NODE_STRING);
            out.byte(1);
            out.data(text.as_bytes());
            out.n(0);
            return Ok(out.bytes);
        }
        if let Ok(Object::CharTable(_)) = self.vm.object(v) {
            self.char_table(v, &mut out)?;
            return Ok(out.bytes);
        }
        let object = self.vm.object(v)?.clone();
        match object {
            Object::Float(f) => {
                out.byte(NODE_FLOAT);
                out.u(f.to_bits());
            }
            Object::Bignum(n) => {
                out.byte(NODE_BIGNUM);
                out.data(&n.to_signed_bytes_le());
            }
            Object::String(s) | Object::RawString(s) => {
                out.byte(NODE_STRING);
                out.byte(u8::from(s.is_multibyte()));
                out.data(s.bytes());
                let end = crate::coordinates::ByteOffset(s.bytes().len());
                let runs: Vec<_> = s.properties.runs(crate::coordinates::ByteOffset(0)..end)
                    .into_iter().filter(|(_, props)| !props.is_empty()).collect();
                out.n(runs.len());
                for (range, props) in runs {
                    out.n(range.start.0);
                    out.n(range.end.0);
                    out.n(props.len());
                    for (key, value) in props {
                        // Keys are written by name, which only an interned symbol's gives back.
                        let name = self.vm.property_key_name(key)
                            .ok_or_else(|| unserialisable("a text property keyed by a symbol that is not interned"))?;
                        let key = self.names.of(name);
                        out.n(key);
                        let r = self.value(value)?;
                        write_ref(&mut out, r);
                    }
                }
            }
            Object::Bytes(b) => {
                out.byte(NODE_BYTES);
                out.data(&b);
            }
            Object::Vector(items) => {
                out.byte(NODE_VECTOR);
                self.refs(&mut out, &items)?;
            }
            Object::Record(items) => {
                out.byte(NODE_RECORD);
                self.refs(&mut out, &items)?;
            }
            Object::BoolVector(b) => {
                out.byte(NODE_BOOL_VECTOR);
                out.n(b.len);
                out.data(&b.bytes);
            }
            other => return Err(unserialisable(object_kind(&other))),
        }
        Ok(out.bytes)
    }
    /// A constant of the function being encoded: an earlier constant of
    /// that function `equal-including-properties' to V when there is one.
    fn constant(&mut self, consts: &mut Vec<Value>, v: Value) -> Result<Ref> {
        // A char-table is mutable, and two equal ones stay two: by identity.
        if matches!(v.tag(), 0 | 3) && v != Value::NIL && !matches!(self.vm.object(v), Ok(Object::CharTable(_))) {
            for &earlier in consts.iter() {
                if earlier != v && self.vm.equal_including_properties(earlier, v).unwrap_or(false) {
                    return self.value(earlier);
                }
            }
            consts.push(v);
        }
        self.value(v)
    }
    fn encode_function(&mut self, id: usize) -> Result<Vec<u8>> {
        let f = self.vm.functions[id].clone();
        function_codec::encode(self, &f)
    }
}

impl CodeRefs for Encoder<'_> {
    fn vm(&self) -> &Vm {
        self.vm
    }
    fn value(&mut self, v: Value) -> Result<Ref> {
        Encoder::value(self, v)
    }
    fn constant(&mut self, consts: &mut Vec<Value>, v: Value) -> Result<Ref> {
        Encoder::constant(self, consts, v)
    }
    fn name(&mut self, name: &str) -> usize {
        self.names.of(name)
    }
    fn symbol(&mut self, slot: usize) -> Result<Ref> {
        let s = self.symbol_index(slot);
        Ok(if s & 1 == 1 { Ref::Uninterned(s >> 1) } else { Ref::Name(s >> 1) })
    }
    fn function(&mut self, id: usize) -> Result<usize> {
        Encoder::function(self, id)
    }
    fn cache(&mut self, cache: usize) -> usize {
        let next = self.caches.len();
        *self.caches.entry(cache).or_insert(next)
    }
}

fn object_kind(o: &Object) -> &'static str {
    match o {
        Object::SyntaxTable(_) => "syntax table",
        Object::Window(_) => "window",
        Object::Frame(_) => "frame",
        Object::Terminal(_) => "terminal",
        Object::WindowConfiguration(_) => "window configuration",
        Object::Sqlite(_) => "sqlite object",
        Object::Treesit(_) => "treesit object",
        Object::Process(_) => "process",
        Object::Finalizer(_) => "finalizer",
        Object::Thread => "thread",
        Object::CharTable(_) | Object::SubCharTable(_) => "char-table",
        Object::MapKeymapCursor(_) => "keymap cursor",
        Object::Buffer(_) => "buffer",
        Object::Marker(_) => "marker",
        Object::Overlay(_) => "overlay",
        Object::ByteBuffer(_) => "byte buffer",
        Object::HashTable(_) => "hash table",
        Object::Obarray(_) => "obarray",
        _ => "object",
    }
}

/// A decoded node, its references still unresolved.
enum Node {
    Cons(Ref, Ref),
    Float(f64),
    Bignum(Vec<u8>),
    String(bool, Vec<u8>, Vec<(usize, usize, Vec<(usize, Ref)>)>),
    Bytes(Vec<u8>),
    Vector(Vec<Ref>),
    Record(Vec<Ref>),
    BoolVector(usize, Vec<u8>),
    Closure(usize, Vec<Ref>, Vec<Ref>, Ref),
    Cell(Ref),
    /// Subtype, default, parent, extra slots, and own values as runs.
    CharTable([Ref; 3], Vec<Ref>, Vec<(usize, usize, Ref)>),
}

fn read_node(r: &mut Reader) -> Result<Node> {
    Ok(match r.byte()? {
        NODE_CONS => Node::Cons(read_ref(r)?, read_ref(r)?),
        NODE_FLOAT => Node::Float(f64::from_bits(r.u()?)),
        NODE_BIGNUM => Node::Bignum(r.data()?.to_vec()),
        NODE_STRING => {
            let multibyte = r.byte()? != 0;
            let bytes = r.data()?.to_vec();
            let n = r.count()?;
            let mut runs = Vec::with_capacity(n);
            for _ in 0..n {
                let (start, end) = (r.n()?, r.n()?);
                let k = r.count()?;
                let props = (0..k).map(|_| Ok((r.n()?, read_ref(r)?))).collect::<Result<Vec<_>>>()?;
                runs.push((start, end, props));
            }
            Node::String(multibyte, bytes, runs)
        }
        NODE_BYTES => Node::Bytes(r.data()?.to_vec()),
        NODE_VECTOR => Node::Vector(read_refs(r)?),
        NODE_RECORD => Node::Record(read_refs(r)?),
        NODE_BOOL_VECTOR => Node::BoolVector(r.n()?, r.data()?.to_vec()),
        NODE_CLOSURE => Node::Closure(r.n()?, read_refs(r)?, read_refs(r)?, read_ref(r)?),
        NODE_CELL => Node::Cell(read_ref(r)?),
        NODE_CHAR_TABLE => {
            let fields = [read_ref(r)?, read_ref(r)?, read_ref(r)?];
            let extras = read_refs(r)?;
            let n = r.count()?;
            let runs = (0..n).map(|_| Ok((r.n()?, r.n()?, read_ref(r)?))).collect::<Result<Vec<_>>>()?;
            Node::CharTable(fields, extras, runs)
        }
        _ => return Err("compiled Lisp artifact: bad node".into()),
    })
}

/// What decoding needs of the artifact: its bytes, which pending bodies
/// keep, and its names' slots.
pub(super) struct Install<'a> {
    pub(super) bytes: &'a Arc<Vec<u8>>,
    pub(super) slots: &'a Arc<Vec<usize>>,
}

impl Install<'_> {
    /// Decode the unit at R into VM and install its functions, their
    /// bodies pending: the entry's id.
    pub(super) fn unit(&self, vm: &mut Vm, r: &mut Reader) -> Result<usize> {
        let n = r.count()?;
        let uninterned: Vec<usize> = (0..n)
            .map(|_| Ok(vm.uninterned_symbol(r.str()?).index(7)? - 1))
            .collect::<Result<_>>()?;
        let n = r.count()?;
        let nodes = (0..n).map(|_| read_node(r)).collect::<Result<Vec<_>>>()?;
        let n = r.count()?;
        let symbols = Symbols { names: self.slots, uninterned: &uninterned };
        let (headers, refs): (Vec<Function>, Vec<Option<BodyRefs>>) = (0..n)
            .map(|_| function_codec::read_function(r, self.bytes, symbols, vm).map(|(f, l)| (f, Some(l))))
            .collect::<Result<Vec<_>>>()?.into_iter().unzip();
        let entry = r.n()?;
        let caches = r.n()?;
        if caches > 1_000_000 {
            return Err("compiled Lisp artifact: too many caches".into());
        }
        validate(&headers, &refs, &nodes, entry, caches)?;
        let (ids, _) = function_codec::install(vm, headers, refs, caches, self.bytes, self.slots, |vm, ids| {
            let values = materialise(vm, &nodes, ids, self.slots, &uninterned)?;
            Ok(move |r| resolve_ref(r, &values, self.slots, &uninterned))
        })?;
        Ok(ids[entry])
    }
}

/// What starts the reason a unit refuses a constant it cannot keep: the
/// walk then stores its file as source (`compile_walk'), a refusal by
/// design, not a failed compile.
pub(super) const UNSERIALISABLE: &str = "a constant that cannot be serialised";

fn unserialisable(what: &str) -> String {
    format!("{UNSERIALISABLE}: {what}")
}

/// A unit's bounds that hold without its bodies: its functions
/// (`function_codec::validate`), closures among the nodes, the objects its
/// headers name, and the entry's signature.
fn validate(functions: &[Function], refs: &[Option<BodyRefs>], nodes: &[Node], entry: usize, caches: usize) -> Result<()> {
    let bad = |what: &str| -> Result<()> { Err(format!("compiled Lisp artifact: invalid {what}")) };
    if entry >= functions.len() {
        return bad("entry");
    }
    for node in nodes {
        if let Node::Closure(f, captures, ..) = node
            && functions.get(*f).is_none_or(|f| f.captures != captures.len())
        {
            return bad("closure");
        }
    }
    if refs.iter().flatten().flat_map(|l| &l.objects).any(|r| matches!(*r, Ref::Node(i) if i >= nodes.len())) {
        return bad("header object");
    }
    function_codec::validate(functions, refs, caches)?;
    let f = &functions[entry];
    if f.captures != 0 || f.arity != 0 || f.rest {
        return bad("entry signature");
    }
    Ok(())
}

fn resolve_ref(r: Ref, values: &[Value], names: &[usize], uninterned: &[usize]) -> Result<Value> {
    let symbol = |slot: usize| Value::reference(slot + 1, 7);
    Ok(match r {
        Ref::Nil => Value::NIL,
        Ref::True => Value::TRUE,
        Ref::Fixnum(n) => Value::integer(n)?,
        Ref::Name(i) => symbol(*names.get(i).ok_or("compiled Lisp artifact: bad name")?),
        Ref::Uninterned(i) => symbol(*uninterned.get(i).ok_or("compiled Lisp artifact: bad symbol")?),
        Ref::Node(i) => *values.get(i).ok_or("compiled Lisp artifact: bad node reference")?,
        Ref::Primitive(bits) => function_codec::resolve_primitive(bits)?,
    })
}

/// Allocate every node, then fill them in, so cycles resolve; closures
/// take their function ids from IDS.
fn materialise(vm: &mut Vm, nodes: &[Node], ids: &[usize], names: &[usize], uninterned: &[usize]) -> Result<Vec<Value>> {
    let mut values = Vec::with_capacity(nodes.len());
    for node in nodes {
        values.push(match node {
            Node::Cons(..) => {
                vm.alloc_cons(Value::NIL, Value::NIL)
            }
            Node::Float(f) => vm.allocate_object(Object::Float(*f)),
            Node::Bignum(bytes) => vm.allocate_object(Object::Bignum(Box::new(num_bigint::BigInt::from_signed_bytes_le(bytes)))),
            Node::String(multibyte, bytes, _) => {
                if *multibyte && !internal_encoding(bytes) {
                    return Err("compiled Lisp artifact: bad string".into());
                }
                vm.allocate_object(data::string_object(data::LispString::from_bytes(bytes.clone(), *multibyte)))
            }
            Node::Bytes(bytes) => vm.allocate_object(Object::Bytes(bytes.clone())),
            Node::Vector(items) => vm.allocate_object(Object::Vector(vec![Value::NIL; items.len()])),
            Node::Record(items) => vm.allocate_object(Object::Record(vec![Value::NIL; items.len()])),
            Node::BoolVector(len, bytes) => {
                if bytes.len() != len.div_ceil(8) {
                    return Err("compiled Lisp artifact: bad bool-vector".into());
                }
                vm.allocate_object(Object::BoolVector(Box::new(data::BoolVector { len: *len, bytes: bytes.clone() })))
            }
            Node::Cell(_) => Value::reference(vm.alloc_cell(Value::NIL), 5),
            Node::CharTable(_, extras, _) => {
                let mut table = crate::char_table::CharTable::new(Value::NIL);
                table.initialize(extras.len(), Value::NIL)?;
                vm.allocate_object(Object::CharTable(Box::new(table)))
            }
            Node::Closure(f, ..) => vm.alloc_closure(Closure { function: ids[*f], captures: vec![], metadata: vec![], document: Value::NIL }),
        });
    }
    let resolve = |r: &Ref| resolve_ref(*r, &values, names, uninterned);
    let all = |rs: &[Ref]| rs.iter().map(resolve).collect::<Result<Vec<_>>>();
    for (node, &v) in nodes.iter().zip(&values) {
        match node {
            Node::Cons(a, b) => {
                let pair = (resolve(a)?, resolve(b)?);
                let at = vm.writable_cons(v)?;
                vm.conses.set_car(at, pair.0);
                vm.conses.set_cdr(at, pair.1);
            }
            Node::Vector(items) | Node::Record(items) => {
                let filled = all(items)?;
                if let Object::Vector(slots) | Object::Record(slots) = vm.object_mut(v)? {
                    *slots = filled;
                }
            }
            Node::String(_, bytes, runs) => {
                // The funnel charged the string plain; its runs are charged here.
                let properties = |vm: &Vm| match vm.object(v) {
                    Ok(Object::String(s) | Object::RawString(s)) => s.properties.storage_bytes(),
                    _ => 0,
                };
                let plain = properties(vm);
                for (start, end, props) in runs {
                    if start >= end || *end > bytes.len() {
                        return Err("compiled Lisp artifact: bad text property run".into());
                    }
                    for (key, value) in props {
                        let key = names.get(*key).ok_or("compiled Lisp artifact: bad property name")?;
                        let key = vm.property_key(&vm.symbol_names[*key]).ok_or("compiled Lisp artifact: bad property name")?;
                        let value = resolve(value)?;
                        if let Object::String(s) | Object::RawString(s) = vm.object_mut(v)? {
                            s.properties.put(crate::coordinates::ByteOffset(*start)..crate::coordinates::ByteOffset(*end), key, value);
                        }
                    }
                }
                vm.allocation.allocated += properties(vm).saturating_sub(plain) as u64;
            }
            Node::Cell(content) => {
                let content = resolve(content)?;
                vm.cells.set(v.payload() as usize, content);
            }
            Node::CharTable([purpose, default, parent], extras, runs) => {
                let (purpose, default, parent, extras) = (resolve(purpose)?, resolve(default)?, resolve(parent)?, all(extras)?);
                let runs = runs.iter().map(|(from, to, value)| Ok((*from, *to, resolve(value)?))).collect::<Result<Vec<_>>>()?;
                if parent != Value::NIL && !matches!(vm.object(parent), Ok(Object::CharTable(_))) {
                    return Err("compiled Lisp artifact: bad char-table parent".into());
                }
                let Object::CharTable(table) = vm.object_mut(v)? else { unreachable!("allocated a char-table") };
                let plain = table.storage_bytes(false);
                table.set_purpose(purpose);
                table.set_default(default);
                table.set_parent(parent);
                table.set_extras(&extras);
                for (from, to, value) in runs {
                    if value != Value::NIL {
                        table.set_range(from, to, value).map_err(|_| "compiled Lisp artifact: bad char-table run")?;
                    }
                }
                // The funnel charged the table empty; its values are charged here.
                let filled = table.storage_bytes(false);
                vm.allocation.allocated += filled.saturating_sub(plain) as u64;
            }
            Node::Closure(_, captures, metadata, document) => {
                let (captures, metadata, document) = (all(captures)?, all(metadata)?, resolve(document)?);
                // The funnel charged the closure empty (data.rs `alloc_closure').
                vm.allocation.allocated += ((captures.len() + metadata.len()) * size_of::<Value>()) as u64;
                let closure = vm.closures.entry_mut(v.index(4)?);
                closure.captures = captures;
                closure.metadata = metadata;
                closure.document = document;
            }
            _ => {}
        }
    }
    Ok(values)
}

/// Whether BYTES is a sequence of GNU internal characters.
fn internal_encoding(mut bytes: &[u8]) -> bool {
    while !bytes.is_empty() {
        match crate::lstring::decode_char(bytes) {
            Some((_, width)) => bytes = &bytes[width..],
            None => return false,
        }
    }
    true
}
