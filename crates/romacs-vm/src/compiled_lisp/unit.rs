//! One compiled unit: the VM2 function tree a top-level form compiled to,
//! and the constants its code holds, as bytes; and back into a VM.
//!
//! Constants are a graph of nodes, so sharing and cycles survive.
//! A function's body is encoded apart, after the values, functions and
//! inline caches it names, so a load installs it pending (`PendingBody`)
//! and decodes it when it is first called (`Vm::decode_pending`), as a
//! code image's bodies stay mapped until their first call; most functions
//! a file defines are not called while it loads.
//! Interned symbols are indices into the artifact's name table; an
//! uninterned symbol is one of the unit's own, made afresh at each load
//! (GNU's `.elc` reads `#:x` per form). Each function's quoted constants
//! are coalesced by `equal-including-properties`, as bytecomp's
//! `byte-compile-get-constant` coalesces a constant vector's.
use super::codec::{Reader, Writer};
use crate::data::Object;
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

/// A reference to a value inside a unit.
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
enum Ref {
    Nil,
    True,
    Fixnum(i64),
    Name(usize),
    Uninterned(usize),
    Node(usize),
    Primitive(u64),
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

/// What one function's body names of its unit, each once, in order: the
/// values (nodes and uninterned symbols), child functions and inline
/// caches. The body refers to them by these local indices.
#[derive(Default)]
struct Deps {
    values: Vec<Ref>,
    value_of: HashMap<Ref, usize>,
    children: Vec<usize>,
    child_of: HashMap<usize, usize>,
    caches: Vec<usize>,
    cache_of: HashMap<usize, usize>,
}
fn local<T: Copy + Eq + std::hash::Hash>(list: &mut Vec<T>, of: &mut HashMap<T, usize>, x: T) -> usize {
    *of.entry(x).or_insert_with(|| {
        list.push(x);
        list.len() - 1
    })
}
impl Deps {
    fn value(&mut self, r: Ref) -> usize {
        local(&mut self.values, &mut self.value_of, r)
    }
    fn child(&mut self, f: usize) -> usize {
        local(&mut self.children, &mut self.child_of, f)
    }
    fn cache(&mut self, c: usize) -> usize {
        local(&mut self.caches, &mut self.cache_of, c)
    }
}

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
    fn symbol(&mut self, slot: usize) -> Result<usize> {
        if self.vm.generated_slots.contains_key(&slot) || self.vm.obarray_symbol_names.contains_key(&slot) {
            let next = self.uninterned_names.len();
            let index = *self.uninterned.entry(slot).or_insert(next);
            if index == next {
                self.uninterned_names.push(self.vm.symbol_names[slot].clone());
            }
            Ok(index << 1 | 1)
        } else {
            Ok(self.names.of(&self.vm.symbol_names[slot]) << 1)
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
                let s = self.symbol(slot)?;
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
    fn write_ref(out: &mut Writer, r: Ref) {
        match r {
            Ref::Nil => out.byte(0),
            Ref::True => out.byte(1),
            Ref::Fixnum(n) => {
                out.byte(2);
                out.i(n);
            }
            Ref::Name(i) => {
                out.byte(3);
                out.n(i);
            }
            Ref::Uninterned(i) => {
                out.byte(4);
                out.n(i);
            }
            Ref::Node(i) => {
                out.byte(5);
                out.n(i);
            }
            Ref::Primitive(bits) => {
                out.byte(6);
                out.u(bits);
            }
        }
    }
    fn refs(&mut self, out: &mut Writer, values: &[Value]) -> Result<()> {
        out.n(values.len());
        for &v in values {
            let r = self.value(v)?;
            Self::write_ref(out, r);
        }
        Ok(())
    }
    fn encode_node(&mut self, v: Value) -> Result<Vec<u8>> {
        let mut out = Writer::default();
        if v.tag() == 3 {
            let (car, cdr) = self.vm.cons_pair(v)?;
            out.byte(NODE_CONS);
            let (a, b) = (self.value(car)?, self.value(cdr)?);
            Self::write_ref(&mut out, a);
            Self::write_ref(&mut out, b);
            return Ok(out.bytes);
        }
        if v.tag() == 5 {
            // A closure's mutable capture: shared by the unit's closures
            // that hold it, as in the compile process.
            let content = self.vm.read_capture_cell(v.payload() as usize);
            out.byte(NODE_CELL);
            let r = self.value(content)?;
            Self::write_ref(&mut out, r);
            return Ok(out.bytes);
        }
        if v.tag() == 4 {
            let closure = self.vm.closures.get(v.index(4)?).ok_or("invalid closure")?;
            let (function, captures, metadata, document) =
                (closure.function, closure.captures.clone(), closure.metadata.clone(), closure.document);
            out.byte(NODE_CLOSURE);
            let f = self.function(function)?;
            out.n(f);
            self.refs(&mut out, &captures)?;
            self.refs(&mut out, &metadata)?;
            let doc = self.value(document)?;
            Self::write_ref(&mut out, doc);
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
                        let key = self.names.of(&key);
                        out.n(key);
                        let r = self.value(value)?;
                        Self::write_ref(&mut out, r);
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
            other => return Err(format!("a constant that cannot be serialised: {}", object_kind(&other))),
        }
        Ok(out.bytes)
    }
    /// A constant of the function being encoded: an earlier constant of
    /// that function `equal-including-properties' to V when there is one.
    fn constant(&mut self, consts: &mut Vec<Value>, v: Value) -> Result<Ref> {
        if matches!(v.tag(), 0 | 3) && v != Value::NIL {
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
        if f.mapped_code.is_some() || f.lazy.is_some() {
            return Err("an unmaterialised function body".into());
        }
        let mut out = Writer::default();
        for n in [f.regs, f.captures, f.arity, f.required] {
            out.n(n);
        }
        out.byte(u8::from(f.rest));
        out.byte(match f.dynamic { None => 0, Some(false) => 1, Some(true) => 2 });
        match &f.params {
            None => out.byte(0),
            Some(params) => {
                out.byte(1);
                out.n(params.len());
                for p in params {
                    self.form(&mut out, p)?;
                }
            }
        }
        let mut body = Writer::default();
        let mut deps = Deps::default();
        body.n(f.sites.len());
        for site in &f.sites {
            body.n(site.len());
            for &r in site {
                body.n(r);
            }
        }
        body.n(f.capture_mutations.len());
        for &m in &f.capture_mutations {
            body.byte(u8::from(m));
        }
        body.n(f.code.len());
        let mut consts = vec![];
        for &op in &f.code {
            self.op(&mut body, op, &mut consts, &mut deps)?;
        }
        out.n(deps.values.len());
        for &r in &deps.values {
            Self::write_ref(&mut out, r);
        }
        for list in [&deps.children, &deps.caches] {
            out.n(list.len());
            list.iter().for_each(|&x| out.n(x));
        }
        out.data(&body.bytes);
        Ok(out.bytes)
    }
    /// R in a body: a node or uninterned symbol by its index in DEPS.
    fn write_local(out: &mut Writer, deps: &mut Deps, r: Ref) {
        match r {
            Ref::Node(_) | Ref::Uninterned(_) => {
                out.byte(5);
                out.n(deps.value(r));
            }
            r => Self::write_ref(out, r),
        }
    }
    /// The symbol SLOT in a body: a name's index, shifted, or an
    /// uninterned symbol's index in DEPS, shifted and odd.
    fn local_symbol(&mut self, slot: usize, deps: &mut Deps) -> Result<usize> {
        let s = self.symbol(slot)?;
        Ok(if s & 1 == 1 { deps.value(Ref::Uninterned(s >> 1)) << 1 | 1 } else { s })
    }
    fn op(&mut self, out: &mut Writer, op: Op, consts: &mut Vec<Value>, deps: &mut Deps) -> Result<()> {
        let n = |out: &mut Writer, xs: &[usize]| xs.iter().for_each(|&x| out.n(x));
        match op {
            Op::Const(d, v) => {
                out.byte(0);
                n(out, &[d]);
                let r = self.constant(consts, v)?;
                Self::write_local(out, deps, r);
            }
            Op::Move(a, b) => { out.byte(1); n(out, &[a, b]) }
            Op::Cell(a) => { out.byte(2); n(out, &[a]) }
            Op::ReadCell(a, b) => { out.byte(3); n(out, &[a, b]) }
            Op::WriteCell(a, b) => { out.byte(4); n(out, &[a, b]) }
            Op::Global(d, s) => { out.byte(5); let s = self.local_symbol(s, deps)?; n(out, &[d, s]) }
            Op::SetGlobal(s, r) => { out.byte(6); let s = self.local_symbol(s, deps)?; n(out, &[s, r]) }
            Op::Bind(s, r) => { out.byte(7); let s = self.local_symbol(s, deps)?; n(out, &[s, r]) }
            Op::Unbind(k) => { out.byte(8); n(out, &[k]) }
            Op::BindLate(s, r) => { out.byte(25); let s = self.local_symbol(s, deps)?; n(out, &[s, r]) }
            Op::UnbindLate(r) => { out.byte(26); n(out, &[r]) }
            Op::ReadLate(d, c, s) => { out.byte(27); let s = self.local_symbol(s, deps)?; n(out, &[d, c, s]) }
            Op::WriteLate(c, x, s) => { out.byte(28); let s = self.local_symbol(s, deps)?; n(out, &[c, x, s]) }
            Op::LateForwarded(d, c) => { out.byte(29); n(out, &[d, c]) }
            Op::Jump(t) => { out.byte(9); n(out, &[t]) }
            Op::Unless(r, t) => { out.byte(10); n(out, &[r, t]) }
            Op::Closure(d, id, site) => {
                out.byte(11);
                let f = self.function(id)?;
                n(out, &[d, deps.child(f), site])
            }
            Op::Define(s, r) => { out.byte(12); let s = self.local_symbol(s, deps)?; n(out, &[s, r]) }
            Op::DefineMacro(s, r) => { out.byte(13); let s = self.local_symbol(s, deps)?; n(out, &[s, r]) }
            Op::Call(d, x, start, k) => { out.byte(15); n(out, &[d, x, start, k]) }
            Op::Direct(d, s, cache, start, k) => {
                out.byte(16);
                let s = self.local_symbol(s, deps)?;
                let next = self.caches.len();
                let c = *self.caches.entry(cache).or_insert(next);
                n(out, &[d, s, deps.cache(c), start, k])
            }
            Op::Sealed(d, s, start, k) => { out.byte(17); let s = self.local_symbol(s, deps)?; n(out, &[d, s, start, k]) }
            Op::Return(r) => { out.byte(18); n(out, &[r]) }
            Op::Catch(d, x, t) => { out.byte(19); n(out, &[d, x, t]) }
            Op::Condition(d, v, t) => {
                out.byte(20);
                n(out, &[d]);
                let r = self.constant(consts, v)?;
                Self::write_local(out, deps, r);
                n(out, &[t]);
            }
            Op::PopHandlers(k) => { out.byte(21); n(out, &[k]) }
            Op::Protect(d, c, e) => { out.byte(22); n(out, &[d, c, e]) }
            Op::LeaveProtect(r) => { out.byte(23); n(out, &[r]) }
            Op::ResumeExit => out.byte(24),
        }
        Ok(())
    }
    fn form(&mut self, out: &mut Writer, form: &Form) -> Result<()> {
        match form {
            Form::Int(n) => { out.byte(0); out.i(*n) }
            Form::Float(f) => { out.byte(1); out.u(f.to_bits()) }
            Form::Sym(s) => { out.byte(2); let i = self.names.of(s); out.n(i) }
            Form::Generated(id) => {
                let slot = *self.vm.generated_symbols.get(id).ok_or("unknown generated symbol")?;
                out.byte(3);
                let s = self.symbol(slot)?;
                out.n(s >> 1);
            }
            Form::List(items) | Form::Vector(items) => {
                out.byte(if matches!(form, Form::List(_)) { 4 } else { 7 });
                out.n(items.len());
                for item in items {
                    self.form(out, item)?;
                }
            }
            Form::String(s) => { out.byte(5); out.str(s) }
            Form::Bytes(b) => { out.byte(6); out.data(b) }
            Form::Dotted(items, tail) => {
                out.byte(8);
                out.n(items.len());
                for item in items {
                    self.form(out, item)?;
                }
                self.form(out, tail)?;
            }
            Form::Object(object) => {
                out.byte(9);
                let v = object.value(self.vm)?;
                let r = self.value(v)?;
                Self::write_ref(out, r);
            }
        }
        Ok(())
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
}

fn read_ref(r: &mut Reader) -> Result<Ref> {
    Ok(match r.byte()? {
        0 => Ref::Nil,
        1 => Ref::True,
        2 => Ref::Fixnum(r.i()?),
        3 => Ref::Name(r.n()?),
        4 => Ref::Uninterned(r.n()?),
        5 => Ref::Node(r.n()?),
        6 => Ref::Primitive(r.u()?),
        _ => return Err("compiled Lisp artifact: bad value reference".into()),
    })
}
fn read_refs(r: &mut Reader) -> Result<Vec<Ref>> {
    let n = r.count()?;
    (0..n).map(|_| read_ref(r)).collect()
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
        _ => return Err("compiled Lisp artifact: bad node".into()),
    })
}

/// A decoded op operand that names a value or a function, resolved once
/// the unit's nodes and function ids exist.
/// What decoding needs of the artifact: its bytes, which pending bodies
/// keep, and its names' slots.
pub(super) struct Install<'a> {
    pub(super) bytes: &'a Arc<Vec<u8>>,
    pub(super) slots: &'a Arc<Vec<usize>>,
}

/// What a function's body names, and where its bytes are in the artifact.
struct BodyRefs {
    values: Vec<Ref>,
    children: Vec<usize>,
    caches: Vec<usize>,
    body: std::ops::Range<usize>,
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
        let (headers, refs): (Vec<Function>, Vec<BodyRefs>) =
            (0..n).map(|_| self.read_function(r, &uninterned, vm)).collect::<Result<Vec<_>>>()?.into_iter().unzip();
        let entry = r.n()?;
        let caches = r.n()?;
        if caches > 1_000_000 {
            return Err("compiled Lisp artifact: too many caches".into());
        }
        validate(&headers, &refs, &nodes, entry, caches)?;
        let (ids, cache_ids) = vm.reserve_request_artifact(headers, caches);
        let values = materialise(vm, &nodes, &ids, self.slots, &uninterned)?;
        // All or none: a published body owns its cache slots.
        let bodies = refs.into_iter().map(|l| {
            Ok(PendingBody {
                bytes: Arc::clone(self.bytes),
                body: l.body,
                values: l.values.iter().map(|&r| resolve_ref(r, &values, self.slots, &uninterned)).collect::<Result<_>>()?,
                children: l.children.iter().map(|&f| ids[f]).collect(),
                caches: l.caches.iter().map(|&c| cache_ids[c]).collect(),
                names: Arc::clone(self.slots),
            })
        }).collect::<Result<Vec<_>>>()?;
        for (pending, &id) in bodies.into_iter().zip(&ids) {
            Arc::get_mut(vm.functions.change(id)).expect("a function just installed is private").pending = Some(Box::new(pending));
        }
        vm.claim_caches(&cache_ids);
        Ok(ids[entry])
    }

    fn read_function(&self, r: &mut Reader, uninterned: &[usize], vm: &Vm) -> Result<(Function, BodyRefs)> {
        let (regs, captures, arity, required) = (r.n()?, r.n()?, r.n()?, r.n()?);
        let rest = r.byte()? != 0;
        let dynamic = match r.byte()? { 0 => None, 1 => Some(false), 2 => Some(true), _ => return Err("bad dialect".into()) };
        let params = match r.byte()? {
            0 => None,
            1 => {
                let n = r.count()?;
                Some((0..n).map(|_| read_form(r, self.slots, uninterned, vm)).collect::<Result<Vec<_>>>()?)
            }
            _ => return Err("bad parameter list".into()),
        };
        let values = read_refs(r)?;
        let list = |r: &mut Reader| -> Result<Vec<usize>> { let n = r.count()?; (0..n).map(|_| r.n()).collect() };
        let (children, caches) = (list(r)?, list(r)?);
        let body = r.data()?;
        let start = body.as_ptr() as usize - self.bytes.as_ptr() as usize;
        let function = Function { capture_mutations: vec![], mapped_code: None, code: vec![], regs, captures, arity, required, rest,
                                  sites: vec![], lazy: None, dynamic, params, pending: None };
        Ok((function, BodyRefs { values, children, caches, body: start..start + body.len() }))
    }
}

impl Vm {
    /// The function ID with its pending body decoded (`PendingBody`): its
    /// code and closure sites, each operand checked as a unit's are.
    pub(crate) fn decode_pending(&self, id: usize) -> Result<Function> {
        let f = self.functions.get(id).ok_or("invalid function")?;
        let p = f.pending.as_deref().ok_or("function is not pending")?;
        let mut r = Reader::new(&p.bytes[p.body.clone()]);
        let n = r.count()?;
        let mut sites = Vec::with_capacity(n);
        for _ in 0..n {
            let k = r.count()?;
            sites.push((0..k).map(|_| r.n()).collect::<Result<Vec<_>>>()?);
        }
        let n = r.count()?;
        let capture_mutations = (0..n).map(|_| Ok(r.byte()? != 0)).collect::<Result<Vec<_>>>()?;
        let bad = |what: &str| format!("compiled Lisp artifact: invalid {what}");
        let symbol = |s: usize| -> Result<usize> {
            if s & 1 == 0 {
                p.names.get(s >> 1).copied().ok_or_else(|| bad("name"))
            } else {
                p.values.get(s >> 1).and_then(|v| v.index(7).ok()).map(|i| i - 1).ok_or_else(|| bad("symbol"))
            }
        };
        let value = |r: &mut Reader| -> Result<Value> {
            Ok(match r.byte()? {
                0 => Value::NIL,
                1 => Value::TRUE,
                2 => Value::integer(r.i()?)?,
                3 => Value::reference(*p.names.get(r.n()?).ok_or_else(|| bad("name"))? + 1, 7),
                5 => *p.values.get(r.n()?).ok_or_else(|| bad("value"))?,
                6 => resolve_ref(Ref::Primitive(r.u()?), &[], &[], &[])?,
                _ => return Err(bad("value reference")),
            })
        };
        let child = |i: usize| p.children.get(i).copied().ok_or_else(|| bad("closure"));
        let n = r.count()?;
        let mut code = Vec::with_capacity(n);
        for _ in 0..n {
            let op = r.byte()?;
            let mut v = || r.n();
            code.push(match op {
                0 => { let d = v()?; Op::Const(d, value(&mut r)?) }
                1 => Op::Move(v()?, v()?),
                2 => Op::Cell(v()?),
                3 => Op::ReadCell(v()?, v()?),
                4 => Op::WriteCell(v()?, v()?),
                5 => Op::Global(v()?, symbol(v()?)?),
                6 => { let s = symbol(v()?)?; Op::SetGlobal(s, v()?) }
                7 => { let s = symbol(v()?)?; Op::Bind(s, v()?) }
                8 => Op::Unbind(v()?),
                9 => Op::Jump(v()?),
                10 => Op::Unless(v()?, v()?),
                11 => { let d = v()?; let c = child(v()?)?; Op::Closure(d, c, v()?) }
                12 => { let s = symbol(v()?)?; Op::Define(s, v()?) }
                13 => { let s = symbol(v()?)?; Op::DefineMacro(s, v()?) }
                15 => Op::Call(v()?, v()?, v()?, v()?),
                16 => {
                    let d = v()?;
                    let s = symbol(v()?)?;
                    let c = *p.caches.get(v()?).ok_or_else(|| bad("cache"))?;
                    Op::Direct(d, s, c, v()?, v()?)
                }
                17 => { let d = v()?; let s = symbol(v()?)?; Op::Sealed(d, s, v()?, v()?) }
                18 => Op::Return(v()?),
                19 => Op::Catch(v()?, v()?, v()?),
                20 => { let d = v()?; let x = value(&mut r)?; Op::Condition(d, x, r.n()?) }
                21 => Op::PopHandlers(v()?),
                22 => Op::Protect(v()?, v()?, v()?),
                23 => Op::LeaveProtect(v()?),
                24 => Op::ResumeExit,
                25 => { let s = symbol(v()?)?; Op::BindLate(s, v()?) }
                26 => Op::UnbindLate(v()?),
                27 => { let d = v()?; let c = v()?; Op::ReadLate(d, c, symbol(v()?)?) }
                28 => { let c = v()?; let x = v()?; Op::WriteLate(c, x, symbol(v()?)?) }
                29 => Op::LateForwarded(v()?, v()?),
                _ => return Err(bad("op")),
            });
        }
        if !r.done() {
            return Err(bad("body"));
        }
        let decoded = Function { capture_mutations, mapped_code: None, code, regs: f.regs, captures: f.captures, arity: f.arity,
                                 required: f.required, rest: f.rest, sites, lazy: None, dynamic: f.dynamic,
                                 params: f.params.clone(), pending: None };
        validate_body(&decoded, |child| self.functions.get(child).map(|f| f.captures), self.slots.len(), self.caches.len())?;
        Ok(decoded)
    }
}

fn read_form(r: &mut Reader, names: &[usize], uninterned: &[usize], vm: &Vm) -> Result<Form> {
    let list = |r: &mut Reader| -> Result<Vec<Form>> {
        let n = r.count()?;
        (0..n).map(|_| read_form(r, names, uninterned, vm)).collect()
    };
    Ok(match r.byte()? {
        0 => Form::Int(r.i()?),
        1 => Form::Float(f64::from_bits(r.u()?)),
        2 => Form::Sym(vm.symbol_names[*names.get(r.n()?).ok_or("compiled Lisp artifact: bad name")?].clone()),
        3 => {
            let slot = *uninterned.get(r.n()?).ok_or("compiled Lisp artifact: bad symbol")?;
            Form::Generated(*vm.generated_slots.get(&slot).ok_or("compiled Lisp artifact: bad symbol")?)
        }
        4 => Form::List(list(r)?),
        5 => Form::String(r.str()?.to_owned()),
        6 => Form::Bytes(r.data()?.to_vec()),
        7 => Form::Vector(list(r)?),
        8 => {
            let items = list(r)?;
            Form::Dotted(items, Box::new(read_form(r, names, uninterned, vm)?))
        }
        _ => return Err("compiled Lisp artifact: unsupported parameter form".into()),
    })
}

/// A unit's bounds that hold without its bodies: headers, what each body
/// names, closures among the nodes, and the entry's signature.
fn validate(functions: &[Function], refs: &[BodyRefs], nodes: &[Node], entry: usize, caches: usize) -> Result<()> {
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
    for (f, l) in functions.iter().zip(refs) {
        if f.regs == 0 || f.regs > 65536 || f.captures + f.arity + usize::from(f.rest) > f.regs || f.required > f.arity {
            return bad("function header");
        }
        if l.children.iter().any(|&c| c >= functions.len()) || l.caches.iter().any(|&c| c >= caches) {
            return bad("body references");
        }
    }
    let f = &functions[entry];
    if f.captures != 0 || f.arity != 0 || f.rest {
        return bad("entry signature");
    }
    Ok(())
}

/// Bounds of every operand of a decoded body F, so only valid IR reaches
/// threaded lowering (as `code_image.rs` checks its own). CAPTURES gives a
/// child function's capture count; SLOTS and CACHES bound symbols and
/// inline caches.
fn validate_body(f: &Function, captures: impl Fn(usize) -> Option<usize>, slots: usize, caches: usize) -> Result<()> {
    let bad = |what: &str| -> Result<()> { Err(format!("compiled Lisp artifact: invalid {what}")) };
    if !matches!(f.code.last(), Some(Op::Return(_) | Op::Jump(_))) {
        return bad("function body");
    }
    let r = |r: usize| r < f.regs;
    let s = |s: usize| s < slots;
    let jump = |t: usize| t < f.code.len();
    let window = |start: usize, n: usize| n <= 4096 && start.checked_add(n).is_some_and(|end| end <= f.regs);
    for op in &f.code {
        let ok = match *op {
            Op::Const(d, _) | Op::Cell(d) | Op::Return(d) | Op::LeaveProtect(d) => r(d),
            Op::Move(d, x) | Op::ReadCell(d, x) | Op::WriteCell(d, x) => r(d) && r(x),
            Op::ReadLate(d, x, slot) | Op::WriteLate(d, x, slot) => r(d) && r(x) && s(slot),
            Op::BindLate(slot, d) => s(slot) && r(d),
            Op::UnbindLate(d) => r(d),
            Op::LateForwarded(d, x) => r(d) && r(x),
            Op::Global(d, x) => r(d) && s(x),
            Op::SetGlobal(x, d) | Op::Bind(x, d) | Op::Define(x, d) | Op::DefineMacro(x, d) => s(x) && r(d),
            Op::Unbind(n) | Op::PopHandlers(n) => n <= f.code.len(),
            Op::Protect(d, cleanup, end) => r(d) && jump(cleanup) && jump(end),
            Op::ResumeExit => true,
            Op::Jump(t) => jump(t),
            Op::Unless(d, t) => r(d) && jump(t),
            Op::Catch(d, x, t) => r(d) && r(x) && jump(t),
            Op::Condition(d, _, t) => r(d) && jump(t),
            Op::Closure(d, id, site) => r(d) && f.sites.get(site).is_some_and(|regs| {
                captures(id) == Some(regs.len()) && regs.iter().all(|x| r(*x))
            }),
            Op::Call(d, x, start, n) => r(d) && r(x) && window(start, n),
            Op::Direct(d, x, c, start, n) => r(d) && s(x) && c < caches && window(start, n),
            Op::Sealed(d, x, start, n) => r(d) && s(x) && window(start, n),
        };
        if !ok {
            return bad("operand");
        }
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
        Ref::Primitive(bits) => {
            let v = Value(bits);
            if v.tag() != 6 || prim::name(v).is_none() {
                return Err("compiled Lisp artifact: bad primitive".into());
            }
            v
        }
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
                        let key = vm.symbol_names[*key].clone();
                        let value = resolve(value)?;
                        if let Object::String(s) | Object::RawString(s) = vm.object_mut(v)? {
                            s.properties.put(crate::coordinates::ByteOffset(*start)..crate::coordinates::ByteOffset(*end), &key, value);
                        }
                    }
                }
                vm.allocation.allocated += properties(vm).saturating_sub(plain) as u64;
            }
            Node::Cell(content) => {
                let content = resolve(content)?;
                vm.cells.set(v.payload() as usize, content);
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
