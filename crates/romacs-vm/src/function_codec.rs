//! One function's code as bytes, and back: its header (signature, lambda
//! list, capture layout, structural key), what its body names, and its
//! body, the IR ops. Compiled Lisp's units (`compiled_lisp/unit.rs`) and a
//! seed's code graph (`frozen/code.rs`) encode functions this one way; each
//! says what a value, a symbol, a child function and an inline cache are in
//! its own encoding (`CodeRefs`), and installs what it decoded by
//! reservation (`install`).
//!
//! A body is encoded apart from the header, after the values, child
//! functions and caches it names, each once (`BodyRefs`), so an install
//! leaves it pending (`PendingBody`) and decodes it when it is first called
//! (`Vm::decode_pending`), as a code image's bodies stay mapped until their
//! first call. Interned symbols are indices into the encoding's name table;
//! an uninterned symbol is one of the encoding's own, made afresh at each
//! install.
use crate::compiled_lisp::codec::{Reader, Writer};
use crate::{Arc, CaptureBinding, CaptureLayout, CaptureSlot, Form, Function, HashMap, Name, Op, PendingBody, Result, Value, Vm, handle, prim};

/// A reference to a value inside an encoding. What a `Node` indexes is the
/// encoding's: a unit's constant nodes, a seed's graph edges.
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum Ref {
    Nil,
    True,
    Fixnum(i64),
    Name(usize),
    Uninterned(usize),
    Node(usize),
    Primitive(u64),
}

pub(crate) fn write_ref(out: &mut Writer, r: Ref) {
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

pub(crate) fn read_ref(r: &mut Reader) -> Result<Ref> {
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

pub(crate) fn read_refs(r: &mut Reader) -> Result<Vec<Ref>> {
    let n = r.count()?;
    (0..n).map(|_| read_ref(r)).collect()
}

/// What an encoding makes of the values, symbols, functions and caches a
/// function names.
pub(crate) trait CodeRefs {
    fn vm(&self) -> &Vm;
    /// V as a reference.
    fn value(&mut self, v: Value) -> Result<Ref>;
    /// V as a constant of the function being encoded, whose earlier
    /// constants are CONSTS: by default V itself.
    fn constant(&mut self, consts: &mut Vec<Value>, v: Value) -> Result<Ref> {
        let _ = consts;
        self.value(v)
    }
    /// The interned symbol NAME's index in the name table.
    fn name(&mut self, name: &str) -> usize;
    /// The symbol SLOT: a `Ref::Name` or a `Ref::Uninterned`.
    fn symbol(&mut self, slot: usize) -> Result<Ref>;
    /// Function ID's index among the encoding's functions.
    fn function(&mut self, id: usize) -> Result<usize>;
    /// Inline cache CACHE's index among the encoding's caches.
    fn cache(&mut self, cache: usize) -> usize;
}

/// What one function's body names of its encoding, each once, in order:
/// the values (nodes and uninterned symbols), child functions and inline
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

/// The eager function F as bytes: its header, what its body names, then
/// the body.
pub(crate) fn encode(refs: &mut impl CodeRefs, f: &Function) -> Result<Vec<u8>> {
    if f.mapped_code.is_some() || f.lazy.is_some() || f.pending.is_some() {
        return Err("an unmaterialised function body".into());
    }
    let mut out = Writer::default();
    write_header(refs, &mut out, f)?;
    let mut deps = Deps::default();
    let body = encode_body(refs, f, &mut deps)?;
    out.n(deps.values.len());
    for &r in &deps.values {
        write_ref(&mut out, r);
    }
    for list in [&deps.children, &deps.caches] {
        out.n(list.len());
        list.iter().for_each(|&x| out.n(x));
    }
    out.data(&body);
    Ok(out.bytes)
}

fn encode_body(refs: &mut impl CodeRefs, f: &Function, deps: &mut Deps) -> Result<Vec<u8>> {
    let mut body = Writer::default();
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
        write_op(refs, &mut body, op, &mut consts, deps)?;
    }
    Ok(body.bytes)
}

/// F's signature, lambda list, capture layout and structural key.
pub(crate) fn write_header(refs: &mut impl CodeRefs, out: &mut Writer, f: &Function) -> Result<()> {
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
                write_form(refs, out, p)?;
            }
        }
    }
    write_environment(refs, out, f.environment.as_ref())?;
    match f.key {
        None => out.byte(0),
        Some(key) => {
            out.byte(1);
            for byte in key.to_le_bytes() { out.byte(byte); }
        }
    }
    Ok(())
}

fn write_environment(refs: &mut impl CodeRefs, out: &mut Writer, layout: Option<&CaptureLayout>) -> Result<()> {
    let Some(layout) = layout else {
        out.byte(0);
        return Ok(());
    };
    out.byte(1);
    out.byte(u8::from(layout.bare_context));
    out.n(layout.specials.len());
    for name in &layout.specials {
        out.n(refs.name(name));
    }
    out.n(layout.slots.len());
    for slot in &layout.slots {
        match slot.binding {
            CaptureBinding::Lexical => out.byte(0),
            CaptureBinding::Late { shadow: None } => out.byte(1),
            CaptureBinding::Late { shadow: Some(index) } => { out.byte(2); out.n(index); }
        }
        match &slot.name {
            None => out.byte(0),
            Some(Name::Source(name)) => write_form(refs, out, &Form::Sym(name.clone()))?,
            Some(Name::Generated(id)) => write_form(refs, out, &Form::Generated(*id))?,
        }
    }
    Ok(())
}

pub(crate) fn write_form(refs: &mut impl CodeRefs, out: &mut Writer, form: &Form) -> Result<()> {
    match form {
        Form::Int(n) => { out.byte(0); out.i(*n) }
        Form::Float(f) => { out.byte(1); out.u(f.to_bits()) }
        Form::Sym(s) => { out.byte(2); let i = refs.name(s); out.n(i) }
        Form::Generated(id) => {
            let slot = *refs.vm().generated_symbols.get(id).ok_or("unknown generated symbol")?;
            out.byte(3);
            let (Ref::Name(i) | Ref::Uninterned(i)) = refs.symbol(slot)? else { return Err("a generated symbol".into()) };
            out.n(i);
        }
        Form::List(items) | Form::Vector(items) => {
            out.byte(if matches!(form, Form::List(_)) { 4 } else { 7 });
            out.n(items.len());
            for item in items {
                write_form(refs, out, item)?;
            }
        }
        Form::String(s) => { out.byte(5); out.str(s) }
        Form::Bytes(b) => { out.byte(6); out.data(b) }
        Form::Dotted(items, tail) => {
            out.byte(8);
            out.n(items.len());
            for item in items {
                write_form(refs, out, item)?;
            }
            write_form(refs, out, tail)?;
        }
        Form::Object(object) => {
            out.byte(9);
            let v = object.value(refs.vm())?;
            let r = refs.value(v)?;
            write_ref(out, r);
        }
    }
    Ok(())
}

/// R in a body: a node or uninterned symbol by its index in DEPS.
fn write_local(out: &mut Writer, deps: &mut Deps, r: Ref) {
    match r {
        Ref::Node(_) | Ref::Uninterned(_) => {
            out.byte(5);
            out.n(deps.value(r));
        }
        r => write_ref(out, r),
    }
}

/// The symbol SLOT in a body: a name's index, shifted, or an uninterned
/// symbol's index in DEPS, shifted and odd.
fn local_symbol(refs: &mut impl CodeRefs, slot: usize, deps: &mut Deps) -> Result<usize> {
    Ok(match refs.symbol(slot)? {
        Ref::Name(i) => i << 1,
        r => deps.value(r) << 1 | 1,
    })
}

fn write_op(refs: &mut impl CodeRefs, out: &mut Writer, op: Op, consts: &mut Vec<Value>, deps: &mut Deps) -> Result<()> {
    let n = |out: &mut Writer, xs: &[usize]| xs.iter().for_each(|&x| out.n(x));
    match op {
        Op::Const(d, v) => {
            out.byte(0);
            n(out, &[d]);
            let r = refs.constant(consts, v)?;
            write_local(out, deps, r);
        }
        Op::Closure(d, id, site) => {
            out.byte(11);
            let f = refs.function(id)?;
            n(out, &[d, deps.child(f), site]);
        }
        Op::Direct(d, s, cache, start, k) => {
            out.byte(16);
            let s = local_symbol(refs, s, deps)?;
            let c = refs.cache(cache);
            n(out, &[d, s, deps.cache(c), start, k]);
        }
        Op::Condition(d, v, t) => {
            out.byte(20);
            n(out, &[d]);
            let r = refs.constant(consts, v)?;
            write_local(out, deps, r);
            n(out, &[t]);
        }
        op => write_plain_op(refs, out, op, deps)?,
    }
    Ok(())
}

/// An op that names no value, function or cache: registers, jump targets
/// and symbols.
fn write_plain_op(refs: &mut impl CodeRefs, out: &mut Writer, op: Op, deps: &mut Deps) -> Result<()> {
    let n = |out: &mut Writer, xs: &[usize]| xs.iter().for_each(|&x| out.n(x));
    let mut s = |slot: usize| local_symbol(refs, slot, deps);
    match op {
        Op::Move(a, b) => { out.byte(1); n(out, &[a, b]) }
        Op::Cell(a) => { out.byte(2); n(out, &[a]) }
        Op::ReadCell(a, b) => { out.byte(3); n(out, &[a, b]) }
        Op::WriteCell(a, b) => { out.byte(4); n(out, &[a, b]) }
        Op::Global(d, x) => { out.byte(5); n(out, &[d, s(x)?]) }
        Op::SetGlobal(x, r) => { out.byte(6); n(out, &[s(x)?, r]) }
        Op::Bind(x, r) => { out.byte(7); n(out, &[s(x)?, r]) }
        Op::Unbind(k) => { out.byte(8); n(out, &[k]) }
        Op::BindLate(x, r) => { out.byte(25); n(out, &[s(x)?, r]) }
        Op::UnbindLate(r) => { out.byte(26); n(out, &[r]) }
        Op::ReadLate(d, c, x) => { out.byte(27); n(out, &[d, c, s(x)?]) }
        Op::WriteLate(c, x, y) => { out.byte(28); n(out, &[c, x, s(y)?]) }
        Op::LateForwarded(d, c) => { out.byte(29); n(out, &[d, c]) }
        Op::Jump(t) => { out.byte(9); n(out, &[t]) }
        Op::Unless(r, t) => { out.byte(10); n(out, &[r, t]) }
        Op::Define(x, r) => { out.byte(12); n(out, &[s(x)?, r]) }
        Op::DefineMacro(x, r) => { out.byte(13); n(out, &[s(x)?, r]) }
        Op::Call(d, x, start, k) => { out.byte(15); n(out, &[d, x, start, k]) }
        Op::Sealed(d, x, start, k) => { out.byte(17); n(out, &[d, s(x)?, start, k]) }
        Op::Return(r) => { out.byte(18); n(out, &[r]) }
        Op::Catch(d, x, t) => { out.byte(19); n(out, &[d, x, t]) }
        Op::PopHandlers(k) => { out.byte(21); n(out, &[k]) }
        Op::Protect(d, c, e) => { out.byte(22); n(out, &[d, c, e]) }
        Op::LeaveProtect(r) => { out.byte(23); n(out, &[r]) }
        Op::ResumeExit => out.byte(24),
        Op::Const(..) | Op::Closure(..) | Op::Direct(..) | Op::Condition(..) => unreachable!("write_op encodes it"),
    }
    Ok(())
}

// Decoding.

/// The symbol tables a decode resolves names through: the encoding's name
/// table and its own uninterned symbols, as slots.
#[derive(Clone, Copy)]
pub(crate) struct Symbols<'a> {
    pub(crate) names: &'a [usize],
    pub(crate) uninterned: &'a [usize],
}

/// What a function's body names, where its bytes are in the encoding, and
/// the values its lambda list holds as objects (`Form::Object`), in the
/// order they appear.
pub(crate) struct BodyRefs {
    pub(crate) values: Vec<Ref>,
    pub(crate) children: Vec<usize>,
    pub(crate) caches: Vec<usize>,
    pub(crate) body: std::ops::Range<usize>,
    pub(crate) objects: Vec<Ref>,
}

/// The function at R, read out of BASE (which R reads a part of): its
/// header, a placeholder for the body, and what the body names.
pub(crate) fn read_function(r: &mut Reader, base: &[u8], symbols: Symbols, vm: &Vm) -> Result<(Function, BodyRefs)> {
    let mut objects = vec![];
    let function = read_header(r, symbols, vm, &mut objects)?;
    let values = read_refs(r)?;
    let list = |r: &mut Reader| -> Result<Vec<usize>> { let n = r.count()?; (0..n).map(|_| r.n()).collect() };
    let (children, caches) = (list(r)?, list(r)?);
    let body = r.data()?;
    let start = body.as_ptr() as usize - base.as_ptr() as usize;
    Ok((function, BodyRefs { values, children, caches, body: start..start + body.len(), objects }))
}

/// A function's header (`write_header`) at R, with no code: the objects
/// its lambda list holds are pushed on OBJECTS (`read_forms`).
pub(crate) fn read_header(r: &mut Reader, symbols: Symbols, vm: &Vm, objects: &mut Vec<Ref>) -> Result<Function> {
    let (regs, captures, arity, required) = (r.n()?, r.n()?, r.n()?, r.n()?);
    let rest = r.byte()? != 0;
    let dynamic = match r.byte()? { 0 => None, 1 => Some(false), 2 => Some(true), _ => return Err("bad dialect".into()) };
    let params = match r.byte()? {
        0 => None,
        1 => Some(read_forms(r, symbols, vm, objects)?),
        _ => return Err("bad parameter list".into()),
    };
    let environment = read_environment(r, captures, symbols, vm)?;
    let key = read_key(r)?;
    Ok(Function { capture_mutations: vec![], mapped_code: None, code: vec![], regs, captures, arity, required, rest,
                  sites: vec![], lazy: None, dynamic, environment, params, pending: None, key, macros: Box::default() })
}

/// A counted list of forms at R (`write_form`), objects as `read_form`
/// reads them.
pub(crate) fn read_forms(r: &mut Reader, symbols: Symbols, vm: &Vm, objects: &mut Vec<Ref>) -> Result<Vec<Form>> {
    let n = r.count()?;
    (0..n).map(|_| read_form(r, symbols, vm, objects)).collect()
}

fn read_key(r: &mut Reader) -> Result<Option<u128>> {
    match r.byte()? {
        0 => Ok(None),
        1 => {
            let mut bytes = [0; 16];
            for byte in &mut bytes { *byte = r.byte()?; }
            Ok(Some(u128::from_le_bytes(bytes)))
        }
        _ => Err("bad structural key".into()),
    }
}

fn read_environment(r: &mut Reader, captures: usize, symbols: Symbols, vm: &Vm) -> Result<Option<CaptureLayout>> {
    match r.byte()? {
        0 => return Ok(None),
        1 => (),
        _ => return Err("bad capture layout".into()),
    }
    let bare_context = r.byte()? != 0;
    let special_count = r.count()?;
    let specials = (0..special_count).map(|_| {
        let slot = *symbols.names.get(r.n()?).ok_or("bad local special")?;
        Ok(vm.symbol_names[slot].clone())
    }).collect::<Result<Vec<_>>>()?;
    if r.count()? != captures {
        return Err("capture layout does not match the body".into());
    }
    let slots = (0..captures).map(|_| read_capture_slot(r, captures, symbols, vm)).collect::<Result<Vec<_>>>()?;
    Ok(Some(CaptureLayout { slots, specials, bare_context }))
}

fn read_capture_slot(r: &mut Reader, captures: usize, symbols: Symbols, vm: &Vm) -> Result<CaptureSlot> {
    let binding = match r.byte()? {
        0 => CaptureBinding::Lexical,
        1 => CaptureBinding::Late { shadow: None },
        2 => {
            let index = r.n()?;
            if index >= captures {
                return Err("invalid late capture shadow".into());
            }
            CaptureBinding::Late { shadow: Some(index) }
        }
        _ => return Err("invalid capture binding".into()),
    };
    let tag = r.byte()?;
    let name = if tag == 0 { None } else { Some(read_name(r, tag, symbols, vm)?) };
    Ok(CaptureSlot { name, binding })
}

pub(crate) fn read_name(r: &mut Reader, tag: u8, symbols: Symbols, vm: &Vm) -> Result<Name> {
    let bad = || "compiled Lisp artifact: bad symbol";
    match tag {
        2 => {
            let slot = *symbols.names.get(r.n()?).ok_or_else(bad)?;
            Ok(Name::Source(vm.symbol_names[slot].clone()))
        }
        3 => {
            let slot = *symbols.uninterned.get(r.n()?).ok_or_else(bad)?;
            let id = vm.generated_slots.get(&slot).ok_or_else(bad)?;
            Ok(Name::Generated(*id))
        }
        _ => Err(bad().into()),
    }
}

/// A form of a header. An object (tag 9) is read as a placeholder, its
/// reference pushed on OBJECTS: the value exists once the encoding's
/// values are placed (`fill_objects`).
fn read_form(r: &mut Reader, symbols: Symbols, vm: &Vm, objects: &mut Vec<Ref>) -> Result<Form> {
    Ok(match r.byte()? {
        0 => Form::Int(r.i()?),
        1 => Form::Float(f64::from_bits(r.u()?)),
        tag @ (2 | 3) => match read_name(r, tag, symbols, vm)? {
            Name::Source(name) => Form::Sym(name),
            Name::Generated(id) => Form::Generated(id),
        },
        4 => Form::List(read_forms(r, symbols, vm, objects)?),
        5 => Form::String(r.str()?.to_owned()),
        6 => Form::Bytes(r.data()?.to_vec()),
        7 => Form::Vector(read_forms(r, symbols, vm, objects)?),
        8 => {
            let items = read_forms(r, symbols, vm, objects)?;
            Form::Dotted(items, Box::new(read_form(r, symbols, vm, objects)?))
        }
        9 => {
            objects.push(read_ref(r)?);
            Form::Object(handle::FormObject::new(vm, Value::NIL))
        }
        _ => return Err("compiled Lisp artifact: unsupported parameter form".into()),
    })
}

/// FORMS with each object placeholder, in order, replaced by the next of
/// VALUES.
pub(crate) fn fill_objects(vm: &Vm, forms: &mut [Form], values: &mut impl Iterator<Item = Value>) -> Result<()> {
    for form in forms {
        match form {
            Form::Object(object) => *object = handle::FormObject::new(vm, values.next().ok_or("missing header object")?),
            Form::List(items) | Form::Vector(items) => fill_objects(vm, items, values)?,
            Form::Dotted(items, tail) => {
                fill_objects(vm, items, values)?;
                fill_objects(vm, std::slice::from_mut(tail), values)?;
            }
            _ => {}
        }
    }
    Ok(())
}

/// Bounds of FUNCTIONS that hold without their bodies: each header, and
/// what each pending body names among FUNCTIONS and CACHES (a function
/// with no pending body, a lazy one, has none in REFS).
pub(crate) fn validate(functions: &[Function], refs: &[Option<BodyRefs>], caches: usize) -> Result<()> {
    let bad = |what: &str| -> Result<()> { Err(format!("compiled Lisp artifact: invalid {what}")) };
    if functions.len() != refs.len() {
        return bad("function table");
    }
    for (f, l) in functions.iter().zip(refs) {
        // A lazy body (no REFS) may have no register before it compiles.
        if (f.regs == 0 && l.is_some()) || f.regs > 65536 || f.captures + f.arity + usize::from(f.rest) > f.regs || f.required > f.arity {
            return bad("function header");
        }
        if l.as_ref().is_some_and(|l| l.children.iter().any(|&c| c >= functions.len()) || l.caches.iter().any(|&c| c >= caches)) {
            return bad("body references");
        }
    }
    Ok(())
}

/// Install FUNCTIONS, decoded with REFS out of BYTES, by reservation:
/// request-owned placeholders and CACHES cache slots first; then PLACE
/// makes the values the bodies name, given the function ids, and answers
/// how a reference resolves; then each pending body (a function's REFS)
/// is left pending and its lambda list's objects filled in; the caches
/// are claimed last. A failure leaves only placeholders and unclaimed
/// slots, which a collection reclaims. The ids, in order, and the
/// resolution.
pub(crate) fn install<F: Fn(Ref) -> Result<Value>>(
    vm: &mut Vm, functions: Vec<Function>, refs: Vec<Option<BodyRefs>>, caches: usize, bytes: &Arc<Vec<u8>>,
    names: &Arc<Vec<usize>>, place: impl FnOnce(&mut Vm, &[usize]) -> Result<F>,
) -> Result<(Vec<usize>, F)> {
    let (ids, cache_ids) = vm.reserve_request_artifact(functions, caches);
    let resolve = place(vm, &ids)?;
    // All or none: a published body owns its cache slots.
    let mut bodies = Vec::with_capacity(refs.len());
    for (l, &id) in refs.into_iter().zip(&ids) {
        let Some(l) = l else { continue };
        let objects = l.objects.iter().map(|&r| resolve(r)).collect::<Result<Vec<_>>>()?;
        bodies.push((id, PendingBody {
            bytes: Arc::clone(bytes),
            body: l.body,
            values: l.values.iter().map(|&r| resolve(r)).collect::<Result<_>>()?,
            children: l.children.iter().map(|&f| ids[f]).collect(),
            caches: l.caches.iter().map(|&c| cache_ids[c]).collect(),
            names: Arc::clone(names),
        }, objects));
    }
    for (id, pending, objects) in bodies {
        let mut params = vm.functions[id].params.clone();
        if let Some(params) = params.as_mut().filter(|_| !objects.is_empty()) {
            fill_objects(vm, params, &mut objects.into_iter())?;
        }
        let f = Arc::get_mut(vm.functions.change(id)).expect("a function just installed is private");
        f.pending = Some(Box::new(pending));
        f.params = params;
    }
    vm.claim_caches(&cache_ids);
    Ok((ids, resolve))
}

impl Vm {
    /// The function ID with its pending body decoded (`PendingBody`): its
    /// code and closure sites, each operand checked as an install's are.
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
        let n = r.count()?;
        let mut code = Vec::with_capacity(n);
        for _ in 0..n {
            code.push(read_op(&mut r, p)?);
        }
        if !r.done() {
            return Err(bad("body"));
        }
        let decoded = Function { capture_mutations, mapped_code: None, code, regs: f.regs, captures: f.captures, arity: f.arity,
                                 required: f.required, rest: f.rest, sites, lazy: None, dynamic: f.dynamic,
                                 environment: f.environment.clone(), params: f.params.clone(), pending: None, key: f.key, macros: f.macros.clone() };
        validate_body(&decoded, |child| self.functions.get(child).map(|f| f.captures), self.slots.len(), self.caches.len())?;
        Ok(decoded)
    }
}

fn bad(what: &str) -> String {
    format!("compiled Lisp artifact: invalid {what}")
}

/// A pending body's symbol operand S: a name's index, shifted, or an
/// uninterned symbol among its values, shifted and odd.
fn pending_symbol(p: &PendingBody, s: usize) -> Result<usize> {
    if s & 1 == 0 {
        p.names.get(s >> 1).copied().ok_or_else(|| bad("name"))
    } else {
        p.values.get(s >> 1).and_then(|v| v.index(7).ok()).map(|i| i - 1).ok_or_else(|| bad("symbol"))
    }
}

fn pending_value(r: &mut Reader, p: &PendingBody) -> Result<Value> {
    Ok(match r.byte()? {
        0 => Value::NIL,
        1 => Value::TRUE,
        2 => Value::integer(r.i()?)?,
        3 => Value::reference(*p.names.get(r.n()?).ok_or_else(|| bad("name"))? + 1, 7),
        5 => *p.values.get(r.n()?).ok_or_else(|| bad("value"))?,
        6 => resolve_primitive(r.u()?)?,
        _ => return Err(bad("value reference")),
    })
}

fn read_op(r: &mut Reader, p: &PendingBody) -> Result<Op> {
    let op = r.byte()?;
    let symbol = |r: &mut Reader| pending_symbol(p, r.n()?);
    Ok(match op {
        0 => { let d = r.n()?; Op::Const(d, pending_value(r, p)?) }
        5 => Op::Global(r.n()?, symbol(r)?),
        6 => { let s = symbol(r)?; Op::SetGlobal(s, r.n()?) }
        7 => { let s = symbol(r)?; Op::Bind(s, r.n()?) }
        11 => { let d = r.n()?; let c = *p.children.get(r.n()?).ok_or_else(|| bad("closure"))?; Op::Closure(d, c, r.n()?) }
        12 => { let s = symbol(r)?; Op::Define(s, r.n()?) }
        13 => { let s = symbol(r)?; Op::DefineMacro(s, r.n()?) }
        16 => {
            let d = r.n()?;
            let s = symbol(r)?;
            let c = *p.caches.get(r.n()?).ok_or_else(|| bad("cache"))?;
            Op::Direct(d, s, c, r.n()?, r.n()?)
        }
        17 => { let d = r.n()?; let s = symbol(r)?; Op::Sealed(d, s, r.n()?, r.n()?) }
        20 => { let d = r.n()?; let x = pending_value(r, p)?; Op::Condition(d, x, r.n()?) }
        25 => { let s = symbol(r)?; Op::BindLate(s, r.n()?) }
        27 => { let d = r.n()?; let c = r.n()?; Op::ReadLate(d, c, symbol(r)?) }
        28 => { let c = r.n()?; let x = r.n()?; Op::WriteLate(c, x, symbol(r)?) }
        op => read_register_op(r, op)?,
    })
}

/// An op of registers and jump targets alone.
fn read_register_op(r: &mut Reader, op: u8) -> Result<Op> {
    let mut v = || r.n();
    Ok(match op {
        1 => Op::Move(v()?, v()?),
        2 => Op::Cell(v()?),
        3 => Op::ReadCell(v()?, v()?),
        4 => Op::WriteCell(v()?, v()?),
        8 => Op::Unbind(v()?),
        9 => Op::Jump(v()?),
        10 => Op::Unless(v()?, v()?),
        15 => Op::Call(v()?, v()?, v()?, v()?),
        18 => Op::Return(v()?),
        19 => Op::Catch(v()?, v()?, v()?),
        21 => Op::PopHandlers(v()?),
        22 => Op::Protect(v()?, v()?, v()?),
        23 => Op::LeaveProtect(v()?),
        24 => Op::ResumeExit,
        26 => Op::UnbindLate(v()?),
        29 => Op::LateForwarded(v()?, v()?),
        _ => return Err(bad("op")),
    })
}

/// A primitive's value from its bits, checked.
pub(crate) fn resolve_primitive(bits: u64) -> Result<Value> {
    let v = Value(bits);
    if v.tag() != 6 || prim::name(v).is_none() {
        return Err("compiled Lisp artifact: bad primitive".into());
    }
    Ok(v)
}

/// Bounds of every operand of a decoded body F, so only valid IR reaches
/// threaded lowering (as `code_image.rs` checks its own). CAPTURES gives a
/// child function's capture count; SLOTS and CACHES bound symbols and
/// inline caches.
pub(crate) fn validate_body(f: &Function, captures: impl Fn(usize) -> Option<usize>, slots: usize, caches: usize) -> Result<()> {
    if !matches!(f.code.last(), Some(Op::Return(_) | Op::Jump(_))) {
        return Err(bad("function body"));
    }
    for op in &f.code {
        if !operands_valid(f, *op, &captures, slots, caches) {
            return Err(bad("operand"));
        }
    }
    Ok(())
}

fn operands_valid(f: &Function, op: Op, captures: &impl Fn(usize) -> Option<usize>, slots: usize, caches: usize) -> bool {
    let r = |r: usize| r < f.regs;
    let s = |s: usize| s < slots;
    let jump = |t: usize| t < f.code.len();
    let window = |start: usize, n: usize| n <= 4096 && start.checked_add(n).is_some_and(|end| end <= f.regs);
    match op {
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
    }
}
