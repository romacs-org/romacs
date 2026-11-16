//! A compiled lambda's structural key: what GNU's `equal` compares of an
//! interpreted closure besides its environment (fns.c `internal_equal` over
//! `PVEC_CLOSURE`'s slots): its argument list and its body as written,
//! including the docstring and interactive spec, which live in the body.
//!
//! The key is an xxh3-128 hash of one canonical token stream, the same
//! whether the lambda arrives as reader forms (an eagerly compiled body) or
//! as the Lisp list a lazy body holds, so two lambdas of equal source agree
//! whichever way each was compiled. It also covers what changes what the
//! same source compiles to: the dialect (lexical or dynamic binding) and the
//! names and order of the captures, which `equal` then compares by value
//! (`hash.rs`). A body that holds something with no source spelling (a
//! generated or uninterned symbol, a heap object other than a string or a
//! number) has no key, and its closures keep identity equality.
use crate::{Form, Name, Value, Vm, data::Object};

/// The token stream, gathered and then hashed in one call: far cheaper
/// than feeding a streaming hasher a few bytes at a time.
struct Canon(Vec<u8>);

impl Canon {
    /// The stream's head; a local special that KEEP refuses is left out.
    fn new(cx: &Context<'_>, keep: impl Fn(&str) -> bool) -> Self {
        let mut canon = Self(Vec::with_capacity(256));
        canon.tag(b'D');
        canon.0.extend_from_slice(&[u8::from(cx.dynamic)]);
        canon.tag(b'C');
        canon.0.extend_from_slice(&cx.names.len().to_le_bytes());
        // A generated name (a binding the compiler or loader made) has no
        // spelling: it holds its place, and `equal` compares what it holds.
        for name in cx.names {
            match name {
                Name::Source(text) => canon.text(b'y', text.as_bytes()),
                Name::Generated(_) => canon.tag(b'g'),
            }
        }
        let mut specials: Vec<&str> = cx.specials.iter().map(String::as_str).filter(|name| keep(name)).collect();
        specials.sort_unstable();
        canon.tag(b'S');
        for special in specials { canon.text(b'y', special.as_bytes()); }
        canon
    }
    fn tag(&mut self, tag: u8) {
        self.0.extend_from_slice(&[tag]);
    }
    fn text(&mut self, tag: u8, bytes: &[u8]) {
        self.tag(tag);
        self.0.extend_from_slice(&(bytes.len()).to_le_bytes());
        self.0.extend_from_slice(bytes);
    }
    fn int(&mut self, n: i64) {
        self.tag(b'i');
        self.0.extend_from_slice(&(n).to_le_bytes());
    }
    /// A string, ASCII text alike whether it was read multibyte or not.
    fn string(&mut self, bytes: &[u8], multibyte: bool) {
        self.text(if multibyte || bytes.is_ascii() { b's' } else { b'b' }, bytes);
    }
    fn float(&mut self, f: f64) {
        self.tag(b'f');
        self.0.extend_from_slice(&(f.to_bits()).to_le_bytes());
    }
    fn finish(self) -> u128 {
        twox_hash::XxHash3_128::oneshot(&self.0)
    }
}

/// The deepest nesting and the longest list a key reads; past them the
/// lambda has no key (a circular or pathological body).
const DEPTH: usize = 200;
const LENGTH: usize = 100_000;

/// What a closure's key reads besides its code: the dialect, the captured
/// names in order and the variables declared special where it was made
/// (GNU keeps those declarations in the closure's environment).
pub(crate) struct Context<'a> {
    pub(crate) dynamic: bool,
    pub(crate) names: &'a [Name],
    pub(crate) specials: &'a crate::NameSet<String>,
}

/// Whether a local special declaration of NAME is one GNU's environment
/// keeps: eval.c Fdefvar adds a value-less `defvar' to the interpreter
/// environment only for a variable not already declared special.
fn kept_special(vm: &Vm, name: &str) -> bool {
    vm.names.get(name).is_none_or(|&slot| !vm.slot_is_special(slot))
}

/// The key of a lambda compiled from PARAMS and BODY forms, with the
/// interactive spec and modes its closure keeps apart from the body
/// (INTERACTIVE: empty, or SPEC and MODES).
pub(crate) fn of_forms(vm: &Vm, cx: &Context<'_>, params: &[Form], body: &[Form], interactive: &[Form]) -> Option<u128> {
    let mut canon = Canon::new(cx, |name| kept_special(vm, name));
    forms(vm, &mut canon, params, 0)?;
    // The docstring is the closure's own slot, which `equal` compares apart.
    let body = match body {
        [Form::String(_) | Form::Bytes(_), rest @ ..] if !rest.is_empty() => rest,
        [Form::Object(object), rest @ ..] if !rest.is_empty() && object.value(vm).is_ok_and(|v| vm.str_view(v).is_some()) => rest,
        _ => body,
    };
    forms(vm, &mut canon, body, 0)?;
    match interactive {
        [] => canon.tag(b'n'),
        items => { canon.tag(b'i'); for item in items { form(vm, &mut canon, item, 1)?; } }
    }
    Some(canon.finish())
}

/// The key of a closure `make-interpreted-closure` makes from PARAMS forms,
/// the Lisp list BODY and its interactive slot IFORM, `(interactive SPEC .
/// MODES)` or nil, written as `of_forms` writes a lambda's.
pub(crate) fn of_lazy(vm: &Vm, cx: &Context<'_>, params: &[Form], body: Value, iform: Value) -> Option<u128> {
    let mut canon = Canon::new(cx, |name| kept_special(vm, name));
    forms(vm, &mut canon, params, 0)?;
    list_value(vm, &mut canon, body, 0)?;
    if iform == Value::NIL {
        canon.tag(b'n');
    } else {
        let tail = vm.cons_pair(iform).ok()?.1;
        let (spec, modes) = if tail == Value::NIL { (Value::NIL, Value::NIL) } else { vm.cons_pair(tail).ok()? };
        canon.tag(b'i');
        value(vm, &mut canon, spec, 1)?;
        value(vm, &mut canon, modes, 1)?;
    }
    Some(canon.finish())
}

/// The key `of_lazy` would give a closure that keeps its slots SLOTS
/// (`make-interpreted-closure`'s: args, body, environment, then nil and the
/// docstring, then SPEC or `[SPEC MODES]`), from what they hold now: a
/// caller may have changed them since the closure was made.
pub(crate) fn of_slots(vm: &Vm, slots: &[Value]) -> Option<u128> {
    let (&args, &body, &env) = (slots.first()?, slots.get(1)?, slots.get(2)?);
    let (mut names, mut specials) = (Vec::new(), crate::NameSet::default());
    let mut tail = env;
    while let Ok((entry, rest)) = vm.cons_pair(tail) {
        match vm.cons_pair(entry) {
            Ok((symbol, _)) => names.push(Name::Source(interned_name(vm, symbol)?.to_owned())),
            Err(_) if entry == Value::TRUE => {}
            Err(_) => { specials.insert(interned_name(vm, entry)?.to_owned()); }
        }
        tail = rest;
    }
    names.sort();
    names.dedup();
    // An explicit environment is kept as given, as GNU's slot holds it.
    let mut canon = Canon::new(&Context { dynamic: env == Value::NIL, names: &names, specials: &specials }, |_| true);
    list_value(vm, &mut canon, args, 0)?;
    list_value(vm, &mut canon, body, 0)?;
    match slots.get(5) {
        None => canon.tag(b'n'),
        Some(&slot) => {
            let (spec, modes) = match vm.object_if(slot) {
                Some(Object::Vector(items)) if items.len() == 2 => (items[0], items[1]),
                _ => (slot, Value::NIL),
            };
            canon.tag(b'i');
            value(vm, &mut canon, spec, 1)?;
            value(vm, &mut canon, modes, 1)?;
        }
    }
    Some(canon.finish())
}

/// The values a closure that keeps its slots holds now, from its
/// environment slot ENV: each bound name's first binding, in the order of
/// the names, as its captures are ordered (`make-interpreted-closure` sorts
/// them). None for a name no form can spell.
pub(crate) fn environment_values(vm: &Vm, env: Value) -> Option<Vec<Value>> {
    let mut bindings: Vec<(&str, Value)> = Vec::new();
    let mut tail = env;
    while let Ok((entry, rest)) = vm.cons_pair(tail) {
        if let Ok((symbol, value)) = vm.cons_pair(entry) {
            let name = interned_name(vm, symbol)?;
            if !bindings.iter().any(|&(seen, _)| seen == name) { bindings.push((name, value)); }
        }
        tail = rest;
    }
    bindings.sort_by(|a, b| a.0.cmp(b.0));
    Some(bindings.into_iter().map(|(_, value)| value).collect())
}

/// A list of forms, as a proper list.
fn forms(vm: &Vm, canon: &mut Canon, items: &[Form], depth: usize) -> Option<()> {
    canon.tag(b'(');
    for item in items {
        form(vm, canon, item, depth + 1)?;
    }
    canon.tag(b')');
    Some(())
}

fn form(vm: &Vm, canon: &mut Canon, item: &Form, depth: usize) -> Option<()> {
    if depth > DEPTH { return None; }
    match item {
        Form::Int(n) => canon.int(*n),
        Form::Float(f) => canon.float(*f),
        Form::Sym(name) => canon.text(b'y', name.as_bytes()),
        Form::String(text) => canon.string(text.as_bytes(), true),
        Form::Bytes(bytes) => canon.string(bytes, false),
        // `()' reads as nil, which a Lisp body holds as the symbol.
        Form::List(items) if items.is_empty() => canon.text(b'y', b"nil"),
        Form::List(items) => forms(vm, canon, items, depth)?,
        Form::Vector(items) => {
            canon.tag(b'[');
            for item in items { form(vm, canon, item, depth + 1)?; }
            canon.tag(b']');
        }
        Form::Dotted(items, tail) => {
            canon.tag(b'(');
            for item in items { form(vm, canon, item, depth + 1)?; }
            canon.tag(b'.');
            form(vm, canon, tail, depth + 1)?;
            canon.tag(b')');
        }
        // A heap object a read or an expansion carried (a string read from a
        // buffer, mostly): its value, written as a Lisp body's would be.
        Form::Object(object) => value(vm, canon, object.value(vm).ok()?, depth)?,
        Form::Generated(_) => return None,
    }
    Some(())
}

/// The Lisp list BODY as `forms` writes a list of forms: a proper list, or
/// its elements, a `.` and its tail.
fn list_value(vm: &Vm, canon: &mut Canon, body: Value, depth: usize) -> Option<()> {
    canon.tag(b'(');
    let (mut tail, mut length) = (body, 0);
    while let Ok((item, rest)) = vm.cons_pair(tail) {
        length += 1;
        if length > LENGTH { return None; }
        value(vm, canon, item, depth + 1)?;
        tail = rest;
    }
    if tail != Value::NIL {
        canon.tag(b'.');
        value(vm, canon, tail, depth + 1)?;
    }
    canon.tag(b')');
    Some(())
}

fn value(vm: &Vm, canon: &mut Canon, item: Value, depth: usize) -> Option<()> {
    if depth > DEPTH { return None; }
    if item == Value::NIL || item == Value::TRUE {
        canon.text(b'y', if item == Value::NIL { b"nil" } else { b"t" });
        return Some(());
    }
    if let Some(n) = item.integer_value() {
        canon.int(n);
        return Some(());
    }
    if vm.cons_pair(item).is_ok() {
        return list_value(vm, canon, item, depth);
    }
    if let Some(name) = interned_name(vm, item) {
        canon.text(b'y', name.as_bytes());
        return Some(());
    }
    if Vm::image_string(item) {
        let view = vm.str_view(item)?;
        canon.string(view.bytes, view.multibyte);
        return Some(());
    }
    match vm.object_if(item)? {
        Object::Float(f) => canon.float(*f),
        Object::String(_) | Object::RawString(_) | Object::Bytes(_) => {
            let view = vm.str_view(item)?;
            canon.string(view.bytes, view.multibyte);
        }
        Object::Vector(items) => {
            canon.tag(b'[');
            for &item in items { value(vm, canon, item, depth + 1)?; }
            canon.tag(b']');
        }
        _ => return None,
    }
    Some(())
}

/// ITEM's name when it is a symbol of the standard obarray: one the reader
/// returns for that name, as a form names it.
fn interned_name(vm: &Vm, item: Value) -> Option<&str> {
    let slot = item.index_if(7)?.checked_sub(1)?;
    let name = vm.symbol_names.get(slot)?;
    (vm.names.get(name.as_str()) == Some(&slot)).then_some(name.as_str())
}
