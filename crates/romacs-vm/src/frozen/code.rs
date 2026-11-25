//! Code graphs (O7m §2.2, §2.3): the frozen encoding extended to closures,
//! capture cells, function code and subrs, `VM2FRZ02`. Only a seed
//! (`seed.rs`) is encoded so: `freeze`, `thaw` and a plain send stay
//! `VM2FRZ01` and refuse code. A receiver places a code graph as private,
//! mutable copies, never in a region, by reservation as compiled Lisp
//! installs a unit (`function_codec::install`): eager bodies pending over
//! the received bytes, lazy bodies lazy, compiled at their first call. No
//! Lisp runs and nothing collects while it does, and a failure leaves only
//! what a collection reclaims.
//!
//! Functions are encoded by the shared function codec, a body's values as
//! the graph's edge words (`Ref::Node`), its symbols as the graph's symbol
//! descriptors. Subrs go by name. The compiler's names a capture layout or
//! a lazy body keeps (`Name::Generated`) are made afresh in the receiver in
//! the order the sender made them, so a lazy body's capture layout, sorted
//! by name, keeps its order.
use super::{
    Arc, Closure, Counts, Decoded, Edge, Function, GeneratedName, HashMap, HashSet, Label, LazyBody, Name, NameSet, Op, Form,
    Reader, Refusal, Result, Step, Value, Vm, Walk, Writer, decode_object, expand, prim, sections, symbol_entry,
};
use crate::compiled_lisp::codec::{Reader as Bytes, Writer as Out};
use crate::function_codec::{self, BodyRefs, CodeRefs, Ref, Symbols};

const MAGIC: &[u8; 8] = b"VM2FRZ02";
/// The magic, then twelve words: the counts of conses, objects, symbols,
/// closures, cells, functions, subrs and generated names; the inline
/// caches; the blob's length; the root; the code format.
const HEADER: usize = 8 + 8 * 12;
const EAGER: u8 = 0;
const LAZY: u8 = 1;
/// A lazy name's kinds: an interned name, a generated name with a symbol
/// (its descriptor), one without (its index among the anonymous ones).
const NAME_SOURCE: u8 = 0;
const NAME_SYMBOL: u8 = 1;
const NAME_ANONYMOUS: u8 = 2;

/// The code format a graph's bodies are in: a hash (FNV-1a) of the
/// compiler's identity.
pub(super) fn code_format() -> u64 {
    env!("VM2_COMPILER_ID").bytes().fold(0xcbf2_9ce4_8422_2325, |h, b| (h ^ u64::from(b)).wrapping_mul(0x0100_0000_01b3))
}

/// Whether BYTES are a code graph.
pub(super) fn is_code(bytes: &[u8]) -> bool {
    bytes.starts_with(MAGIC)
}

/// A function as the walk found it: its eager code (decoded without being
/// installed, `Vm::function_code`) or its lazy body.
enum Code { Eager(Arc<Function>), Lazy(Arc<Function>) }

/// What stops a walk.
pub(super) enum Stop {
    Refused(Refusal),
    TooLarge,
    Unavailable(String),
}

enum Item { Value(Value, Option<usize>, Label), Function(usize, Option<usize>) }

/// A cons the encoding makes of its own, never consed in the VM (a
/// seed's list structure), or a value the walk entered.
#[derive(Clone, Copy)]
pub(super) enum Spine { Value(Value), Cons(usize) }

/// Where a walk was, to roll an entry back to.
#[derive(Clone, Copy)]
pub(super) struct Mark { nodes: usize, conses: usize, objects: usize, symbols: usize, closures: usize, cells: usize, subrs: usize, functions: usize }

/// The walk of a code graph: the data walk (`Walk`) and the code it
/// reaches, its work counted against one budget.
pub(super) struct CodeWalk<'a> {
    pub(super) data: Walk<'a>,
    closures: Vec<usize>,
    cells: Vec<usize>,
    subrs: Vec<usize>,
    functions: Vec<(usize, Code)>,
    function_of: HashMap<usize, usize>,
    /// Each symbol descriptor's symbol, NIL for a name the encoding added.
    symbol_values: Vec<Value>,
    names: HashMap<String, usize>,
    anonymous: Vec<GeneratedName>,
    anonymous_of: HashMap<GeneratedName, usize>,
    caches: HashMap<usize, usize>,
    spine: Vec<(Spine, Spine)>,
    /// The table of each command's `(SPEC . THUNK)` by its closure, or
    /// nil: read per closure, never copied.
    interactive: Value,
    pub(super) work: usize,
}

impl<'a> CodeWalk<'a> {
    pub(super) fn new(vm: &'a Vm) -> Self {
        let interactive = vm.names.get(prim::INTERACTIVE_TABLE).map_or(Value::NIL, |&slot| vm.slots[slot].value);
        CodeWalk { data: Walk::new(vm), closures: vec![], cells: vec![], subrs: vec![], functions: vec![], function_of: HashMap::default(),
                   symbol_values: vec![], names: HashMap::default(), anonymous: vec![], anonymous_of: HashMap::default(),
                   caches: HashMap::default(), spine: vec![], interactive, work: 0 }
    }

    /// CLOSURE's `(SPEC . THUNK)`, when it is a command.
    fn interactive(&self, closure: Value) -> Option<Value> {
        if self.interactive == Value::NIL || self.interactive == Value::UNBOUND { return None; }
        self.data.vm.eq_hash_get(self.interactive, closure)
    }

    pub(super) fn mark(&self) -> Mark {
        Mark { nodes: self.data.nodes.len(), conses: self.data.cons_nodes.len(), objects: self.data.object_nodes.len(),
               symbols: self.data.symbols.len(), closures: self.closures.len(), cells: self.cells.len(), subrs: self.subrs.len(),
               functions: self.functions.len() }
    }

    /// Forget everything entered since MARK; the work stays spent.
    pub(super) fn rollback(&mut self, mark: Mark) {
        for node in self.data.nodes.drain(mark.nodes..) {
            self.data.memo.remove(&node.value.0);
        }
        for symbol in self.symbol_values.drain(mark.symbols..) {
            self.data.symbol_memo.remove(&symbol.0);
        }
        for (id, _) in self.functions.drain(mark.functions..) {
            self.function_of.remove(&id);
        }
        self.data.symbols.truncate(mark.symbols);
        self.data.cons_nodes.truncate(mark.conses);
        self.data.object_nodes.truncate(mark.objects);
        self.closures.truncate(mark.closures);
        self.cells.truncate(mark.cells);
        self.subrs.truncate(mark.subrs);
    }

    /// A cons of the encoding's own, of CAR and CDR.
    pub(super) fn cons(&mut self, car: Spine, cdr: Spine) -> Spine {
        self.spine.push((car, cdr));
        Spine::Cons(self.spine.len() - 1)
    }

    fn spine_edge(&self, spine: Spine) -> Edge {
        match spine {
            Spine::Value(v) => self.data.edge(v),
            Spine::Cons(i) => Edge::Cons(self.data.cons_nodes.len() + i),
        }
    }

    /// The symbols entered since MARK: what the entry mentions.
    pub(super) fn mentions(&self, mark: Mark) -> &[Value] {
        &self.symbol_values[mark.symbols..]
    }

    /// Enter everything ROOT reaches, the work within LIMIT. Every edge
    /// followed is work, an immediate or one entered already too; so is a
    /// function's every op and each 64 bytes of an object's payload or a
    /// function's held body. Work is spent, and checked, before the work
    /// it pays for is done: a value is entered only if its payload and the
    /// edges it adds fit, a body decoded only if its bytes fit, its ops
    /// read only if they fit. It bounds the allocation and the work
    /// enqueued, except what sizing one object does before the fit is
    /// decided: a string's sizing reads, allocates and sorts every property
    /// on a character, a char table's visits its raw spans, and a cold
    /// mapped body's reads its bytes, each linear in that object, which the
    /// sender holds already.
    pub(super) fn walk(&mut self, root: Value, limit: usize) -> std::result::Result<(), Stop> {
        let mut items = vec![Item::Value(root, None, Label::Root)];
        let mut found = vec![];
        while let Some(item) = items.pop() {
            self.work += 1;
            if self.work > limit { return Err(Stop::TooLarge); }
            match item {
                Item::Value(value, parent, label) => {
                    if value == Value::UNBOUND && !matches!(label, Label::Cell) {
                        // Only a forwarded late cell holds the unbound marker.
                        return Err(Stop::Refused(self.data.refuse(value, parent, label)));
                    }
                    let (payload, edges) = self.data.ahead(value, limit.saturating_sub(self.work));
                    self.work += payload;
                    if self.work + edges > limit { return Err(Stop::TooLarge); }
                    let step = self.data.step(value, parent, label, &mut found);
                    items.extend(found.drain(..).map(|(v, p, l)| Item::Value(v, p, l)));
                    match step.map_err(Stop::Refused)? {
                        Step::New if value.tag() == 7 => self.symbol_values.push(value),
                        Step::Seen | Step::New => {}
                        Step::Code(node) => self.enter_code(value, node, &mut items)?,
                    }
                }
                Item::Function(id, parent) => {
                    if !self.function_of.contains_key(&id) {
                        self.enter_function(id, parent, &mut items, limit)?;
                    }
                }
            }
        }
        if self.work > limit { return Err(Stop::TooLarge); }
        Ok(())
    }

    fn enter_code(&mut self, value: Value, node: usize, items: &mut Vec<Item>) -> std::result::Result<(), Stop> {
        let vm = self.data.vm;
        let parent = Some(node);
        match value.tag() {
            4 => {
                let Some(closure) = value.index_if(4).and_then(|i| vm.closures.get(i)) else { return Err(Stop::Refused(self.data.refusal(node))) };
                self.data.memo.insert(value.0, Edge::Closure(self.closures.len()));
                self.closures.push(node);
                items.push(Item::Function(closure.function, parent));
                if let Some(entry) = self.interactive(value) { items.push(Item::Value(entry, parent, Label::Interactive)); }
                items.push(Item::Value(closure.document, parent, Label::Document));
                items.extend(closure.metadata.iter().enumerate().rev().map(|(i, &v)| Item::Value(v, parent, Label::Metadata(i))));
                items.extend(closure.captures.iter().enumerate().rev().map(|(i, &v)| Item::Value(v, parent, Label::Capture(i))));
            }
            5 => {
                let Some(cell) = value.index_if(5) else { return Err(Stop::Refused(self.data.refusal(node))) };
                self.data.memo.insert(value.0, Edge::Cell(self.cells.len()));
                self.cells.push(node);
                let content = vm.cell_pairs.get(cell).unwrap_or_else(|| vm.read_capture_cell(cell));
                items.push(Item::Value(content, parent, Label::Cell));
            }
            _ => {
                if prim::name(value).is_none() { return Err(Stop::Refused(self.data.refusal(node))); }
                self.data.memo.insert(value.0, Edge::Subr(self.subrs.len()));
                self.subrs.push(node);
            }
        }
        Ok(())
    }

    /// Enter function ID, reached from the closure node PARENT: the values
    /// its code names, the functions it makes closures of, and the symbols
    /// it names, which it mentions. Its work, each spent before it is done
    /// and within LIMIT: its held body's bytes / 64, then its ops.
    fn enter_function(&mut self, id: usize, parent: Option<usize>, items: &mut Vec<Item>, limit: usize) -> std::result::Result<(), Stop> {
        let vm = self.data.vm;
        let f = vm.functions.get(id).ok_or_else(|| Stop::Unavailable("invalid function".into()))?;
        self.work += vm.function_held_bytes(id) / 64;
        if self.work > limit { return Err(Stop::TooLarge); }
        let code = if f.lazy.is_some() { Code::Lazy(Arc::clone(f)) } else { Code::Eager(vm.function_code(id).map_err(Stop::Unavailable)?) };
        self.work += match &code { Code::Eager(f) | Code::Lazy(f) => f.code.len() };
        if self.work > limit { return Err(Stop::TooLarge); }
        self.function_of.insert(id, self.functions.len());
        let mut constants = 0;
        let mut constant = |v: Value, items: &mut Vec<Item>| {
            items.push(Item::Value(v, parent, Label::Constant(constants)));
            constants += 1;
        };
        let (Code::Eager(f) | Code::Lazy(f)) = &code;
        for v in f.params.iter().flatten().flat_map(|form| form_objects(vm, form)) { constant(v, items); }
        if let Some(lazy) = &f.lazy {
            items.push(Item::Value(lazy.body, parent, Label::Body));
            for v in lazy.args.iter().flat_map(|form| form_objects(vm, form)) { constant(v, items); }
        }
        for op in &f.code {
            match *op {
                Op::Const(_, v) | Op::Condition(_, v, _) => constant(v, items),
                Op::Closure(_, child, _) => items.push(Item::Function(child, parent)),
                op => if let Some(slot) = op_symbol(op) { items.push(Item::Value(Value::reference(slot + 1, 7), parent, Label::Root)) },
            }
        }
        self.functions.push((id, code));
        Ok(())
    }
}

/// The objects FORM holds (`Form::Object`), in order.
fn form_objects(vm: &Vm, form: &Form) -> Vec<Value> {
    match form {
        Form::Object(object) => object.value(vm).into_iter().collect(),
        Form::List(items) | Form::Vector(items) => items.iter().flat_map(|form| form_objects(vm, form)).collect(),
        Form::Dotted(items, tail) => items.iter().chain(std::iter::once(&**tail)).flat_map(|form| form_objects(vm, form)).collect(),
        _ => vec![],
    }
}

/// The symbol slot OP names, if it names one.
fn op_symbol(op: Op) -> Option<usize> {
    match op {
        Op::Global(_, s) | Op::SetGlobal(s, _) | Op::Bind(s, _) | Op::BindLate(s, _) | Op::ReadLate(_, _, s) | Op::WriteLate(_, _, s)
        | Op::Define(s, _) | Op::DefineMacro(s, _) | Op::Direct(_, s, ..) | Op::Sealed(_, s, ..) => Some(s),
        _ => None,
    }
}

impl CodeRefs for CodeWalk<'_> {
    fn vm(&self) -> &Vm {
        self.data.vm
    }
    fn value(&mut self, v: Value) -> Result<Ref> {
        Ok(match v.tag() {
            _ if v == Value::NIL => Ref::Nil,
            _ if v == Value::TRUE => Ref::True,
            1 => Ref::Fixnum(v.as_integer()?),
            7 if v != Value::UNBOUND => self.symbol(v.index(7)? - 1)?,
            _ => Ref::Node(usize::try_from(self.data.edge(v).word()).map_err(|_| "edge out of range")?),
        })
    }
    fn name(&mut self, name: &str) -> usize {
        let vm = self.data.vm;
        if let Some(&slot) = vm.names.get(name) && vm.initial_obarray.get(slot).copied().unwrap_or(false) {
            return self.descriptor(Value::reference(slot + 1, 7));
        }
        if let Some(&d) = self.names.get(name) { return d; }
        self.names.insert(name.to_owned(), self.data.symbols.len());
        self.data.symbols.push((name.to_owned(), true));
        self.symbol_values.push(Value::NIL);
        self.data.symbols.len() - 1
    }
    fn symbol(&mut self, slot: usize) -> Result<Ref> {
        let d = self.descriptor(Value::reference(slot + 1, 7));
        Ok(if self.data.symbols[d].1 { Ref::Name(d) } else { Ref::Uninterned(d) })
    }
    fn function(&mut self, id: usize) -> Result<usize> {
        self.function_of.get(&id).copied().ok_or_else(|| "a function outside the graph".into())
    }
    fn cache(&mut self, cache: usize) -> usize {
        let next = self.caches.len();
        *self.caches.entry(cache).or_insert(next)
    }
}

impl CodeWalk<'_> {
    /// SYMBOL's descriptor, entered now if it has none.
    fn descriptor(&mut self, symbol: Value) -> usize {
        if let Some(&d) = self.data.symbol_memo.get(&symbol.0) { return d; }
        let mut found = vec![];
        let _ = self.data.step(symbol, None, Label::Root, &mut found);
        self.symbol_values.push(symbol);
        self.data.symbol_memo[&symbol.0]
    }

    /// A lazy body's NAME: an interned name, a generated name's symbol, or
    /// a generated name with none.
    fn write_name(&mut self, out: &mut Out, name: &Name) {
        match name {
            Name::Source(name) => { out.byte(NAME_SOURCE); let d = self.name(name); out.n(d); }
            Name::Generated(id) => self.write_generated(out, *id),
        }
    }

    fn write_generated(&mut self, out: &mut Out, id: GeneratedName) {
        if let Some(&slot) = self.data.vm.generated_symbols.get(&id) {
            out.byte(NAME_SYMBOL);
            let d = self.descriptor(Value::reference(slot + 1, 7));
            out.n(d);
        } else {
            out.byte(NAME_ANONYMOUS);
            let next = self.anonymous.len();
            let k = *self.anonymous_of.entry(id).or_insert(next);
            if k == next { self.anonymous.push(id); }
            out.n(k);
        }
    }

    fn encode_function(&mut self, code: &Code) -> Result<Vec<u8>> {
        match code {
            Code::Eager(f) => {
                let mut bytes = vec![EAGER];
                bytes.extend(function_codec::encode(self, f)?);
                Ok(bytes)
            }
            Code::Lazy(f) => {
                let mut out = Out::default();
                out.byte(LAZY);
                function_codec::write_header(self, &mut out, f)?;
                self.write_lazy(&mut out, f.lazy.as_deref().ok_or("not lazy")?)?;
                Ok(out.bytes)
            }
        }
    }

    fn write_lazy(&mut self, out: &mut Out, lazy: &LazyBody) -> Result<()> {
        let body = self.value(lazy.body)?;
        function_codec::write_ref(out, body);
        out.n(lazy.args.len());
        for form in &lazy.args { function_codec::write_form(self, out, form)?; }
        out.n(lazy.names.len());
        for name in &lazy.names { self.write_name(out, name); }
        for flags in [&lazy.cells, &lazy.lates] {
            out.n(flags.len());
            for &b in flags { out.byte(u8::from(b)); }
        }
        out.n(lazy.shadows.len());
        for shadow in &lazy.shadows {
            match shadow { None => out.byte(0xff), Some(id) => self.write_generated(out, *id) }
        }
        for specials in [&lazy.local_specials, &lazy.pending_specials] {
            let mut sorted: Vec<&String> = specials.iter().collect();
            sorted.sort();
            out.n(sorted.len());
            for name in sorted { let d = self.name(name); out.n(d); }
        }
        let flags = [lazy.dynamic_scope, lazy.deferred, lazy.evaluated, lazy.file_level, lazy.interpreting];
        out.byte(flags.iter().enumerate().fold(0, |acc, (i, &b)| acc | (u8::from(b) << i)));
        Ok(())
    }

    /// The generated names in the order the sender made them, as
    /// (kind, index): a symbol's descriptor or an anonymous name's index.
    fn generated_order(&self) -> Vec<(u8, usize)> {
        let vm = self.data.vm;
        let mut order: Vec<(GeneratedName, u8, usize)> = self.symbol_values.iter().enumerate()
            .filter(|&(d, _)| !self.data.symbols[d].1)
            .filter_map(|(d, v)| Some((*vm.generated_slots.get(&(v.index_if(7)? - 1))?, NAME_SYMBOL, d)))
            .chain(self.anonymous.iter().enumerate().map(|(k, &id)| (id, NAME_ANONYMOUS, k)))
            .collect();
        order.sort_unstable();
        order.into_iter().map(|(_, kind, i)| (kind, i)).collect()
    }

    /// The encoding of the graph the walk entered, with ROOT.
    pub(super) fn encode(&mut self, root: Spine) -> Result<Vec<u8>> {
        let functions: Vec<(usize, Code)> = std::mem::take(&mut self.functions);
        let mut function_bytes = Vec::with_capacity(functions.len());
        for (_, code) in &functions { function_bytes.push(self.encode_function(code)?); }
        self.functions = functions;
        let closures = self.closures.iter().map(|&node| self.closure_entry(self.data.nodes[node].value)).collect::<Result<Vec<_>>>()?;
        let cells = self.cells.iter().map(|&node| self.cell_entry(self.data.nodes[node].value)).collect::<Result<Vec<_>>>()?;
        let subrs = self.subrs.iter().map(|&node| prim::name(self.data.nodes[node].value).unwrap_or_default().as_bytes().to_vec()).collect::<Vec<_>>();
        let generated = self.generated_order().into_iter().map(|(kind, i)| {
            let mut w = Writer(vec![kind]);
            w.u64(i as u64);
            w.0
        }).collect::<Vec<_>>();
        let counts = [self.data.cons_nodes.len() + self.spine.len(), self.data.object_nodes.len(), self.data.symbols.len(), closures.len(), cells.len(),
                      function_bytes.len(), subrs.len(), generated.len(), self.caches.len()];
        let mut sections: Vec<Vec<u8>> = self.data.object_nodes.iter().map(|&node| {
            let mut w = Writer(vec![]);
            let object = self.data.vm.object_if(self.data.nodes[node].value).ok_or("a discovered object is gone")?;
            self.data.encode_object(&mut w, object);
            Ok(w.0)
        }).collect::<Result<_>>()?;
        sections.extend(self.data.symbols.iter().map(|(name, _)| name.as_bytes().to_vec()));
        let flags: Vec<bool> = self.data.object_nodes.iter().map(|_| false).chain(self.data.symbols.iter().map(|s| s.1)).collect();
        sections.extend(closures.into_iter().chain(cells).chain(function_bytes).chain(subrs).chain(generated));
        self.layout(&counts, root, &sections, &flags)
    }

    fn closure_entry(&self, v: Value) -> Result<Vec<u8>> {
        let closure = v.index_if(4).and_then(|i| self.data.vm.closures.get(i)).ok_or("invalid closure")?;
        let mut w = Writer(vec![]);
        w.u64(self.function_of[&closure.function] as u64);
        for values in [&closure.captures, &closure.metadata] {
            w.u64(values.len() as u64);
            for &c in values { w.u64(self.data.edge(c).word()); }
        }
        w.u64(self.data.edge(closure.document).word());
        w.u64(self.interactive(v).map_or(Value::NIL.0, |entry| self.data.edge(entry).word()));
        Ok(w.0)
    }

    fn cell_entry(&self, v: Value) -> Result<Vec<u8>> {
        let vm = self.data.vm;
        let cell = v.index(5)?;
        let mut w = Writer(vec![]);
        if let Some(pair) = vm.cell_pairs.get(cell) {
            w.u8(1);
            w.u64(self.data.edge(pair).word());
        } else {
            w.u8(0);
            w.u64(self.data.edge(vm.read_capture_cell(cell)).word());
        }
        Ok(w.0)
    }

    /// The bytes: the header with COUNTS, ROOT and the code format, the
    /// cons pairs, the table of SECTIONS (each flagged per FLAGS, false
    /// past it), and the blob.
    fn layout(&self, counts: &[usize; 9], root: Spine, sections: &[Vec<u8>], flags: &[bool]) -> Result<Vec<u8>> {
        let mut w = Writer(MAGIC.to_vec());
        for &n in counts { w.u64(n as u64); }
        let blob_len: usize = sections.iter().map(Vec::len).sum();
        w.u64(blob_len as u64);
        w.u64(self.spine_edge(root).word());
        w.u64(code_format());
        debug_assert_eq!(w.0.len(), HEADER);
        for &node in &self.data.cons_nodes {
            let (car, cdr) = self.data.vm.cons_pair(self.data.nodes[node].value)?;
            w.u64(self.data.edge(car).word());
            w.u64(self.data.edge(cdr).word());
        }
        for &(car, cdr) in &self.spine {
            w.u64(self.spine_edge(car).word());
            w.u64(self.spine_edge(cdr).word());
        }
        let mut offset = 0;
        for (i, section) in sections.iter().enumerate() {
            w.u64(offset as u64);
            w.u64(section.len() as u64 | (u64::from(flags.get(i).copied().unwrap_or(false)) << 63));
            offset += section.len();
        }
        for section in sections { w.0.extend_from_slice(section); }
        Ok(w.0)
    }
}

// Decoding.

struct ClosureEntry { function: usize, captures: Vec<Edge>, metadata: Vec<Edge>, document: Edge, interactive: Edge }

enum CellEntry { Content(Edge), Pair(Edge) }

/// A decoded code graph, before it is placed.
struct Graph {
    data: Decoded,
    counts: Counts,
    closures: Vec<ClosureEntry>,
    cells: Vec<CellEntry>,
    functions: Vec<std::ops::Range<usize>>,
    subrs: Vec<String>,
    generated: Vec<(u8, usize)>,
    caches: usize,
}

fn decode(bytes: &[u8]) -> Result<Graph> {
    if bytes.len() < HEADER || !is_code(bytes) { return Err("not a frozen region".into()); }
    let mut r = Reader { bytes, at: 8 };
    let mut count = || r.count();
    let (conses, objects, symbols, closures, cells) = (count()?, count()?, count()?, count()?, count()?);
    let (functions, subrs, generated, caches, blob_len) = (count()?, count()?, count()?, count()?, count()?);
    let root_word = r.u64()?;
    if r.u64()? != code_format() { return Err("frozen code format differs".into()); }
    let counts = Counts { conses, objects, symbols, closures, cells, subrs };
    let root = Edge::parse(root_word, &counts)?;
    let entries = objects + symbols + closures + cells + functions + subrs + generated;
    let (pairs, table) = sections(&mut r, &counts, entries, blob_len)?;
    let mut at = table.iter().map(|&(start, end, interned)| (&bytes[start..end], start..end, interned));
    let data_objects = at.by_ref().take(objects).map(|(b, ..)| decode_object(b, &counts)).collect::<Result<Vec<_>>>()?;
    let names = at.by_ref().take(symbols).map(|(b, _, interned)| symbol_entry(b, interned)).collect::<Result<Vec<_>>>()?;
    let closures = at.by_ref().take(closures).map(|(b, ..)| decode_closure(b, &counts)).collect::<Result<Vec<_>>>()?;
    let cells = at.by_ref().take(cells).map(|(b, ..)| decode_cell(b, &counts)).collect::<Result<Vec<_>>>()?;
    let functions = at.by_ref().take(functions).map(|(_, range, _)| range).collect();
    let subrs = at.by_ref().take(subrs).map(|(b, ..)| std::str::from_utf8(b).map(str::to_owned).map_err(|_| "frozen subr name is not UTF-8".into()))
        .collect::<Result<Vec<_>>>()?;
    let generated = at.map(|(b, ..)| decode_generated(b)).collect::<Result<Vec<_>>>()?;
    Ok(Graph { data: Decoded { conses: pairs, objects: data_objects, symbols: names, root }, counts, closures, cells, functions, subrs, generated, caches })
}

fn decode_closure(bytes: &[u8], counts: &Counts) -> Result<ClosureEntry> {
    let mut r = Reader { bytes, at: 0 };
    // An ordinal into the graph's functions, bounded by `validate`.
    let function = usize::try_from(r.u64()?).map_err(|_| "frozen closure function is invalid")?;
    let mut edges = || -> Result<Vec<Edge>> { let n = r.count()?; (0..n).map(|_| Edge::parse(r.u64()?, counts)).collect() };
    let (captures, metadata) = (edges()?, edges()?);
    let (document, interactive) = (Edge::parse(r.u64()?, counts)?, Edge::parse(r.u64()?, counts)?);
    if !r.done() { return Err("frozen closure has trailing bytes".into()); }
    Ok(ClosureEntry { function, captures, metadata, document, interactive })
}

fn decode_cell(bytes: &[u8], counts: &Counts) -> Result<CellEntry> {
    let mut r = Reader { bytes, at: 0 };
    let entry = match r.u8()? {
        0 => CellEntry::Content(Edge::parse_content(r.u64()?, counts)?),
        1 => match Edge::parse(r.u64()?, counts)? { pair @ Edge::Cons(_) => CellEntry::Pair(pair), _ => return Err("frozen cell pair is no cons".into()) },
        _ => return Err("frozen cell kind is invalid".into()),
    };
    if !r.done() { return Err("frozen cell has trailing bytes".into()); }
    Ok(entry)
}

fn decode_generated(bytes: &[u8]) -> Result<(u8, usize)> {
    let mut r = Reader { bytes, at: 0 };
    // An ordinal into the descriptors or the anonymous names, bounded where
    // the names are made (`code_symbols`).
    let entry = (r.u8()?, usize::try_from(r.u64()?).map_err(|_| "frozen generated name is invalid")?);
    if !matches!(entry.0, NAME_SYMBOL | NAME_ANONYMOUS) || !r.done() { return Err("frozen generated name is invalid".into()); }
    Ok(entry)
}

// Placement.

/// A lazy body as decoded, its references unresolved.
struct LazyParts {
    body: Ref,
    args: Vec<Form>,
    args_objects: Vec<Ref>,
    params_objects: Vec<Ref>,
    names: Vec<Name>,
    cells: Vec<bool>,
    lates: Vec<bool>,
    shadows: Vec<Option<GeneratedName>>,
    local_specials: NameSet<String>,
    pending_specials: NameSet<String>,
    flags: u8,
}

/// What a placement resolves edges and references to: each descriptor's
/// symbol, each closure, cell and subr, and the bases the conses and
/// objects were appended at.
struct Placed {
    symbols: Vec<Value>,
    closures: Vec<Value>,
    cells: Vec<usize>,
    subrs: Vec<Value>,
    conses: usize,
    objects: usize,
    counts: Counts,
}

impl Placed {
    fn value(&self, edge: Edge) -> Value {
        match edge {
            Edge::Immediate(v) => v,
            Edge::Cons(i) => Value::reference(self.conses + i, 3),
            Edge::Object(i) => Value::reference(self.objects + i + 1, 0),
            Edge::Symbol(i) => self.symbols[i],
            Edge::Closure(i) => self.closures[i],
            Edge::Cell(i) => Value::reference(self.cells[i], 5),
            Edge::Subr(i) => self.subrs[i],
            Edge::Unbound => Value::UNBOUND,
        }
    }
    fn resolve(&self, r: Ref) -> Result<Value> {
        Ok(match r {
            Ref::Nil => Value::NIL,
            Ref::True => Value::TRUE,
            Ref::Fixnum(n) => Value::integer(n)?,
            Ref::Name(i) | Ref::Uninterned(i) => *self.symbols.get(i).ok_or("frozen code: bad symbol")?,
            Ref::Node(word) => self.value(Edge::parse(word as u64, &self.counts)?),
            Ref::Primitive(_) => return Err("frozen code names a primitive by number".into()),
        })
    }
}

/// A graph's symbols as placed: each descriptor's symbol and slot, and its
/// anonymous generated names (`Vm::code_symbols`).
type CodeSymbols = (Vec<Value>, Vec<usize>, Vec<GeneratedName>);

/// A decoded graph's functions: each header, its pending body's
/// references when eager, its lazy body when lazy.
type Functions = (Vec<Function>, Vec<Option<BodyRefs>>, Vec<Option<LazyParts>>);

impl Vm {
    /// Place the code graph BYTES as private, mutable values (O7m §2.3):
    /// its root.
    pub(crate) fn place_code(&mut self, bytes: &Arc<Vec<u8>>) -> Result<Value> {
        let graph = decode(bytes)?;
        let (symbols, slots, anonymous) = self.code_symbols(&graph)?;
        let subrs = graph.subrs.iter().map(|name| prim::subr_named(name).ok_or_else(|| format!("unknown-subr {name}")))
            .collect::<Result<Vec<_>>>()?;
        let slots = Arc::new(slots);
        let table = Symbols { names: &slots, uninterned: &slots };
        let (functions, refs, lazies) = self.code_functions(&graph, bytes, table, &anonymous)?;
        validate(&graph, &functions, &refs, &lazies)?;
        let (_, resolve) = function_codec::install(self, functions, refs, graph.caches, bytes, &slots, |vm, ids| {
            let placed = vm.place_shells(&graph, ids, symbols, subrs)?;
            vm.fill_code(&graph, &placed)?;
            for (lazy, &id) in lazies.into_iter().zip(ids) {
                if let Some(lazy) = lazy { vm.place_lazy(id, lazy, &placed)?; }
            }
            Ok(move |r| placed.resolve(r))
        })?;
        resolve(Ref::Node(usize::try_from(graph.data.root.word()).map_err(|_| "frozen root is invalid")?))
    }

    /// GRAPH's symbols, interned by name or made afresh, the generated
    /// ones in the order the sender made them; their slots; its anonymous
    /// generated names, made afresh in that order too.
    fn code_symbols(&mut self, graph: &Graph) -> Result<CodeSymbols> {
        let names = &graph.data.symbols;
        let mut symbols: Vec<Option<Value>> = vec![None; names.len()];
        let anonymous_count = graph.generated.iter().filter(|(kind, _)| *kind == NAME_ANONYMOUS).count();
        let mut anonymous: Vec<Option<GeneratedName>> = vec![None; anonymous_count];
        for &(kind, i) in &graph.generated {
            if kind == NAME_SYMBOL {
                let (name, interned) = names.get(i).ok_or("frozen generated name is invalid")?;
                if *interned || symbols[i].is_some() { return Err("frozen generated name is invalid".into()); }
                symbols[i] = Some(self.uninterned_symbol(name));
            } else {
                let slot = anonymous.get_mut(i).filter(|slot| slot.is_none()).ok_or("frozen generated name is invalid")?;
                *slot = Some(expand::generated_id());
            }
        }
        let mut slots = Vec::with_capacity(names.len());
        for ((name, interned), symbol) in names.iter().zip(&mut symbols) {
            let value = match *symbol {
                Some(value) => value,
                None if *interned => self.intern(name),
                None => self.uninterned_symbol(name),
            };
            *symbol = Some(value);
            slots.push(if *interned { self.slot(name) } else { value.index(7)? - 1 });
        }
        Ok((symbols.into_iter().flatten().collect(), slots, anonymous.into_iter().flatten().collect()))
    }

    fn code_functions(&self, graph: &Graph, bytes: &[u8], table: Symbols, anonymous: &[GeneratedName]) -> Result<Functions> {
        let (mut functions, mut refs, mut lazies) = (vec![], vec![], vec![]);
        for range in &graph.functions {
            let mut r = Bytes::new(&bytes[range.clone()]);
            match r.byte()? {
                EAGER => {
                    let (f, body) = function_codec::read_function(&mut r, bytes, table, self)?;
                    functions.push(f);
                    refs.push(Some(body));
                    lazies.push(None);
                }
                LAZY => {
                    let mut params_objects = vec![];
                    functions.push(function_codec::read_header(&mut r, table, self, &mut params_objects)?);
                    refs.push(None);
                    lazies.push(Some(self.read_lazy(&mut r, table, anonymous, params_objects)?));
                }
                _ => return Err("frozen function kind is invalid".into()),
            }
            if !r.done() { return Err("frozen function has trailing bytes".into()); }
        }
        Ok((functions, refs, lazies))
    }

    fn read_lazy(&self, r: &mut Bytes, table: Symbols, anonymous: &[GeneratedName], params_objects: Vec<Ref>)
        -> Result<LazyParts>
    {
        let body = function_codec::read_ref(r)?;
        let mut args_objects = vec![];
        let args = function_codec::read_forms(r, table, self, &mut args_objects)?;
        let n = r.count()?;
        let names = (0..n).map(|_| { let kind = r.byte()?; self.read_name(r, kind, table, anonymous) }).collect::<Result<Vec<_>>>()?;
        let mut flags = || -> Result<Vec<bool>> { let n = r.count()?; (0..n).map(|_| Ok(r.byte()? != 0)).collect() };
        let (cells, lates) = (flags()?, flags()?);
        let n = r.count()?;
        let shadows = (0..n).map(|_| match r.byte()? {
            0xff => Ok(None),
            kind => match self.read_name(r, kind, table, anonymous)? { Name::Generated(id) => Ok(Some(id)), Name::Source(_) => Err("frozen shadow is invalid".into()) },
        }).collect::<Result<Vec<_>>>()?;
        let mut specials = || -> Result<NameSet<String>> {
            let n = r.count()?;
            (0..n).map(|_| Ok(self.symbol_names[*table.names.get(r.n()?).ok_or("frozen special is invalid")?].clone())).collect()
        };
        let (local_specials, pending_specials) = (specials()?, specials()?);
        Ok(LazyParts { body, args, args_objects, params_objects, names, cells, lates, shadows, local_specials, pending_specials, flags: r.byte()? })
    }

    fn read_name(&self, r: &mut Bytes, kind: u8, table: Symbols, anonymous: &[GeneratedName]) -> Result<Name> {
        let bad = || "frozen lazy name is invalid".to_owned();
        let slot = |r: &mut Bytes| table.names.get(r.n()?).copied().ok_or_else(bad);
        Ok(match kind {
            NAME_SOURCE => Name::Source(self.symbol_names[slot(r)?].clone()),
            NAME_SYMBOL => Name::Generated(*self.generated_slots.get(&slot(r)?).ok_or_else(bad)?),
            NAME_ANONYMOUS => Name::Generated(*anonymous.get(r.n()?).ok_or_else(bad)?),
            _ => return Err(bad()),
        })
    }

    /// GRAPH's closure and cell shells, then its conses and objects
    /// (`place_private`): what its edges resolve to.
    fn place_shells(&mut self, graph: &Graph, ids: &[usize], symbols: Vec<Value>, subrs: Vec<Value>) -> Result<Placed> {
        let closures = graph.closures.iter()
            .map(|c| self.alloc_closure(Closure { function: ids[c.function], captures: vec![], metadata: vec![], document: Value::NIL }))
            .collect();
        let cells = graph.cells.iter().map(|_| self.alloc_cell(Value::NIL)).collect();
        let placed = Placed { symbols, closures, cells, subrs, conses: self.conses.len(), objects: self.objects.len(), counts: graph.counts };
        self.place_private(&graph.data, &|edge| placed.value(edge))?;
        Ok(placed)
    }

    /// Fill GRAPH's closures and cells, pair the cells Lisp held the
    /// lexical pairs of, and register its commands' specs.
    fn fill_code(&mut self, graph: &Graph, placed: &Placed) -> Result<()> {
        for (entry, &closure) in graph.closures.iter().zip(&placed.closures) {
            let captures: Vec<Value> = entry.captures.iter().map(|&e| placed.value(e)).collect();
            let metadata: Vec<Value> = entry.metadata.iter().map(|&e| placed.value(e)).collect();
            // The funnel charged the closure empty (data.rs `alloc_closure').
            self.allocation.allocated += ((captures.len() + metadata.len()) * size_of::<Value>()) as u64;
            let c = self.closures.entry_mut(closure.index(4)?);
            c.captures = captures;
            c.metadata = metadata;
            c.document = placed.value(entry.document);
        }
        for (entry, &cell) in graph.cells.iter().zip(&placed.cells) {
            match *entry {
                CellEntry::Content(e) => self.cells.set(cell, placed.value(e)),
                CellEntry::Pair(e) => {
                    let pair = placed.value(e);
                    self.cells.set(cell, self.cons_pair(pair)?.1);
                    self.cell_pairs.insert(cell, pair);
                }
            }
        }
        for (entry, &closure) in graph.closures.iter().zip(&placed.closures) {
            let command = placed.value(entry.interactive);
            if command != Value::NIL {
                let (spec, thunk) = self.cons_pair(command)?;
                self.register_interactive_spec(closure, spec, thunk)?;
            }
        }
        Ok(())
    }

    /// Function ID's lazy body, from LAZY: compiled at its first call here.
    fn place_lazy(&mut self, id: usize, lazy: LazyParts, placed: &Placed) -> Result<()> {
        let resolve = |refs: &[Ref]| refs.iter().map(|&r| placed.resolve(r)).collect::<Result<Vec<_>>>();
        let (args_objects, params_objects) = (resolve(&lazy.args_objects)?, resolve(&lazy.params_objects)?);
        let mut args = lazy.args;
        function_codec::fill_objects(self, &mut args, &mut args_objects.into_iter())?;
        let mut params = self.functions[id].params.clone();
        if let Some(params) = &mut params { function_codec::fill_objects(self, params, &mut params_objects.into_iter())?; }
        let flag = |i: u8| lazy.flags & (1 << i) != 0;
        let body = LazyBody {
            body: placed.resolve(lazy.body)?, args, names: lazy.names, cells: lazy.cells, lates: lazy.lates, shadows: lazy.shadows,
            local_specials: lazy.local_specials, pending_specials: lazy.pending_specials, dynamic_scope: flag(0), deferred: flag(1),
            evaluated: flag(2), file_level: flag(3), interpreting: flag(4),
        };
        let f = Arc::get_mut(self.functions.change(id)).ok_or("a placed function is shared")?;
        f.capture_mutations = vec![true; f.captures];
        f.lazy = Some(Arc::new(body));
        f.params = params;
        Ok(())
    }
}

/// GRAPH's bounds beyond its edges': each closure's function and capture
/// count, each lazy body's layout, and one cell to a lexical pair.
fn validate(graph: &Graph, functions: &[Function], refs: &[Option<BodyRefs>], lazies: &[Option<LazyParts>]) -> Result<()> {
    function_codec::validate(functions, refs, graph.caches)?;
    for closure in &graph.closures {
        if functions.get(closure.function).is_none_or(|f| f.captures != closure.captures.len()) {
            return Err("frozen closure does not match its function".into());
        }
    }
    for (f, lazy) in functions.iter().zip(lazies) {
        if let Some(lazy) = lazy
            && [lazy.names.len(), lazy.cells.len(), lazy.lates.len(), lazy.shadows.len()].iter().any(|&n| n != f.captures)
        {
            return Err("frozen lazy body does not match its function".into());
        }
    }
    let mut pairs = HashSet::default();
    for cell in &graph.cells {
        if let CellEntry::Pair(pair) = cell && !pairs.insert(pair.word()) {
            return Err("frozen cells share a lexical pair".into());
        }
    }
    Ok(())
}
