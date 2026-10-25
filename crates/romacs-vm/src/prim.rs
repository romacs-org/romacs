//! Declarative primitives: the PRIMITIVE-DOCTRINE harness, with its three-tier
//! GC boundary (doctrine section 3) enforced by types rather than review.
//!
//! A primitive is declared ONCE with `defsubr!` in any `src/prims/*.rs` file.
//! `build.rs` finds every declaration and generates `GENERATED` (sorted by
//! name, so no append-only ordinal and no shared hand-edited table) plus a JSON
//! manifest that the differential harness and introspection read.
//!
//! The invariant: no collection can occur between entry and exit of Rust
//! primitive code. Collection happens only at engine wait points
//! (`poll_resume`) and request boundaries, and allocation never collects.
//!
//! * T1 leaf, `fn(cx, args..) { .. }` returning `Out`. The body receives
//!   `&mut Leaf`, never `&mut Vm`. `Leaf` exposes reads and allocation only and
//!   lives in a private module, so its `vm` field is private even to
//!   `prims::*` (compile-fail probe, `tests/primitive_gc_boundary.rs`). Every
//!   invocation also runs under a debug assertion that the collection count is
//!   unchanged, which catches a future allocator that collects.
//! * T2 call request, `fn(cx, args..) -> Step { .. }`. A body that must call
//!   Lisp returns `Step::Call { f, args, then, saved }` instead of calling. The
//!   engine performs `f` as an ordinary Lisp call from a continuation
//!   activation whose registers hold `saved`, so every Value that must survive
//!   the callback is a traced, forwarded register, never a Rust local. `then` is
//!   a `fn` pointer, not a closure, so it cannot capture Rust state; it receives
//!   the callback's value plus the saved registers and returns the next `Step`.
//! * T3 Lisp prelude: loops that call Lisp per element are ordinary compiled
//!   Lisp over leaf cursors (`prims/prelude.el`), following the `maphash`
//!   expansion precedent. All loop state is then Lisp registers by construction.
use super::*;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[allow(dead_code)]
pub(crate) enum Effect {
    /// Result depends only on arguments; no observable writes. Stageable.
    Pure,
    /// Reads mutable world state (buffers, specials) but writes none.
    Reads,
    /// Mutates its arguments (setcar-like). Write-barrier candidate.
    MutatesArgs,
    /// Writes hidden global state (match data, current buffer, ...).
    Global,
}

/// Every way a leaf may fail. Each maps to GNU's condition AND data, so no
/// primitive formats a string that `route_escape` must later guess from.
#[allow(dead_code)]
pub(crate) enum Fault {
    /// `(wrong-type-argument PREDICATE VALUE)`, GNU `wrong_type_argument`.
    Wrong(&'static str, Value),
    /// Named boundary for strings that cannot be consumed as Unicode text.
    NotUnicode(Value),
    /// `(error MESSAGE)`, preserving the complete diagnostic.
    Error(String),
    /// Reader signals carry their own condition rather than generic errors.
    EndOfFile,
    ReadSyntax(String),
    /// A signal whose condition symbol and data list the body already built
    /// (the reader knows GNU's exact data); signalled as given.
    Signal(Value, Value),
    /// `(circular-list LIST)`.
    Circular(Value),
    /// `(args-out-of-range A)`, GNU `data.c:string-to-number`.
    Range1(Value),
    /// `(args-out-of-range A B)`, GNU `data.c:140`.
    Range(Value, Value),
    /// `(args-out-of-range A B C)`, GNU `data.c:146`.
    Range3(Value, Value, Value),
    /// `(void-function ORIGINAL-FUNCTION)`, GNU `eval.c:2752`.
    VoidFunction(Value),
    /// `(void-variable SYMBOL)`, GNU `data.c:1657`.
    VoidVariable(Value),
    /// `(cyclic-function-indirection SYMBOL)`, GNU `data.c:916`.
    CyclicFunctionIndirection(Value),
    /// `(cyclic-variable-indirection SYMBOL)`, GNU `eval.c:658`.
    CyclicVariableIndirection(Value),
    /// `(wrong-number-of-arguments FUNCTION COUNT)`, GNU `eval.c:3311`.
    WrongNumberOfArguments(Value, Value),
    /// `(setting-constant SYMBOL)`, GNU `data.c:908`.
    SettingConstant(Value),
}
pub(crate) type Out = std::result::Result<Value, Fault>;

/// The variable slot holding the closure -> interactive-spec table.
pub(crate) const INTERACTIVE_TABLE: &str = "#:vm2-interactive-specs";
/// Lazy native root, traced and forwarded like INTERACTIVE_TABLE.
pub(crate) const FONT_ALTERNATIVE_FAMILIES: &str = "#:vm2-font-alternative-families";

/// A T2 continuation. A plain `fn` pointer: it cannot capture Rust state, so
/// the only Values it sees are the callback's result and the saved registers.
pub(crate) type Then = fn(&mut Leaf<'_>, Value, &[Value]) -> Next;
/// What a T2 body (or continuation) asks the engine to do next.
pub(crate) enum Step {
    /// Request collection at the engine's traced boundary, never inside a leaf.
    Collect,
    /// Return this value from the primitive.
    Done(Value),
    /// Call `f` with `args` as an ordinary Lisp call, then run `then` with its
    /// value and `saved`. The engine copies `f`, `args` and `saved` into the
    /// continuation activation's registers before any wait point.
    Call {
        f: Value,
        args: Vec<Value>,
        then: Then,
        saved: Vec<Value>,
    },
}
pub(crate) type Next = std::result::Result<Step, Fault>;

/// The two declarable tiers. Control primitives stay hand-written in the engine.
#[derive(Clone, Copy)]
pub(crate) enum Body {
    Leaf(fn(&mut Leaf<'_>, &[Value]) -> Out),
    Call(fn(&mut Leaf<'_>, &[Value]) -> Next),
}

pub(crate) struct PrimDesc {
    pub name: &'static str,
    pub min: usize,
    pub max: Option<usize>,
    pub effect: Effect,
    pub gnu: &'static str,
    pub file: &'static str,
    pub line: u32,
    pub body: Body,
}

/// The only view of the VM a leaf primitive gets. It lives in its own module so
/// the `vm` field is private even to `prims::*` (a child of `prim` would
/// otherwise see `prim`'s private fields): a leaf body cannot name `Vm`, so it
/// cannot call `collect`, `execute` or `load`. Proven by compile-fail probe.
mod leaf {
    use super::super::*;
    use super::Fault;

    /// Storage kinds, including VM-internal cells and the unbound sentinel.
    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    pub(crate) enum Kind {
        Symbol,
        Integer,
        Bignum,
        Cons,
        Closure,
        Cell,
        Subr,
        Unbound,
        Float,
        String,
        Bytes,
        Vector,
        BoolVector,
        Record,
        Buffer,
        Marker,
        Overlay,
        ByteBuffer,
        HashTable,
        Obarray,
        SyntaxTable,
        CharTable,
        SubCharTable,
        Window,
        Frame,
        Terminal,
        Invalid,
    }

    pub(crate) struct Leaf<'a> {
        vm: &'a mut Vm,
    }
    #[allow(dead_code)]
    impl<'a> Leaf<'a> {
        pub(in crate::prim) fn new(vm: &'a mut Vm) -> Self {
            Leaf { vm }
        }
        pub(crate) fn set_font_alternative_families(&mut self, raw: Value, names: Vec<Vec<String>>) {
            let slot = self.vm.slot(super::FONT_ALTERNATIVE_FAMILIES);
            self.vm.slots[slot].value = raw;
            self.vm.display_state.set_font_alternative_families(names);
        }
        pub(crate) fn set_font_selection_order(&mut self, order: [romacs_layout_input::FontSelectionAttribute; 4]) {
            self.vm.display_state.set_font_selection_order(order);
        }
        pub(crate) fn decode_buffer(&mut self, value: Value) -> std::result::Result<Value, Fault> {
            let buffer = if value == Value::NIL {
                let index = self.vm.buffer_index().map_err(|e| Fault::Error(e.to_string()))?;
                Value::reference(index + 1, 0)
            } else { value };
            if !matches!(self.kind(buffer), Kind::Buffer | Kind::ByteBuffer) {
                return Err(Fault::Wrong("bufferp", buffer));
            }
            Ok(buffer)
        }
        /// A line boundary without moving point (GNU editfns.c).
        pub fn line_boundary_position(&mut self, count: Value, end: bool) -> super::Out {
            let count = if count == Value::NIL { 1 } else {
                count.integer_value().ok_or(Fault::Wrong("integerp", count))?
            };
            let index = self.vm.buffer_index().map_err(Fault::Error)?;
            let (chars, point, low, high) = match &self.vm.objects[index] {
                data::Object::Buffer(b) => {
                    let range = b.accessible();
                    (b.as_str().chars().collect::<Vec<_>>(),
                     b.char_of_byte(b.point).map_err(|e| Fault::Error(e.to_string()))?.0,
                     b.char_of_byte(range.start).map_err(|e| Fault::Error(e.to_string()))?.0,
                     b.char_of_byte(range.end).map_err(|e| Fault::Error(e.to_string()))?.0)
                },
                data::Object::ByteBuffer(b) => (b.bytes.iter().map(|&b| b as char).collect(), b.point.0, b.restriction.start.0, b.restriction.end.0),
                _ => unreachable!(),
            };
            let mut pos = point;
            while pos > low && chars[pos-1] != '\n' { pos -= 1; }
            let lines = count.saturating_sub(1);
            if lines >= 0 {
                for _ in 0..lines {
                    while pos < high && chars[pos] != '\n' { pos += 1; }
                    if pos == high { break; }
                    pos += 1;
                }
            } else {
                for _ in lines..0 {
                    if pos == low { break; }
                    pos -= 1;
                    while pos > low && chars[pos-1] != '\n' { pos -= 1; }
                }
            }
            if end { while pos < high && chars[pos] != '\n' { pos += 1; } }
            Ok(Value::integer(pos as i64 + 1).unwrap())
        }
        /// Point of the current buffer as a 1-based character position.
        pub(crate) fn point(&mut self) -> std::result::Result<i64, Fault> {
            let index = self.vm.buffer_index().map_err(|e| Fault::Error(e.to_string()))?;
            match &self.vm.objects[index] {
                data::Object::Buffer(t) => Ok(t.char_of_byte(t.point).map_err(|e| Fault::Error(e.to_string()))?.0 as i64 + 1),
                data::Object::ByteBuffer(b) => Ok(b.point.0 as i64 + 1),
                _ => Err(Fault::Error("buffer required".into())),
            }
        }
        /// The interactive spec a compiled closure was created with (GNU's
        /// CLOSURE_INTERACTIVE slot), as `(SPEC . THUNK)`: THUNK evaluates a
        /// non-string SPEC in the closure's own lexical environment, or is nil.
        /// VM2 closures have no such slot, so the compiler registers it in a
        /// key-weak `eq` table held by an ordinary traced variable slot.
        pub(crate) fn interactive_spec(&mut self, closure: Value) -> Option<(Value, Value)> {
            let slot = *self.vm.names.get(super::INTERACTIVE_TABLE)?;
            let table = self.vm.slots[slot].value;
            if table == Value::UNBOUND {
                return None;
            }
            let entry = self.vm.declaration_hash_get(table, closure).ok()??;
            self.pair(entry)
        }
        pub(crate) fn set_interactive_spec(&mut self, closure: Value, spec: Value, thunk: Value) -> std::result::Result<(), Fault> {
            let slot = self.vm.slot(super::INTERACTIVE_TABLE);
            if self.vm.slots[slot].value == Value::UNBOUND {
                // A private key-weak `eq' table that also sheds dead slots and
                // capacity after each collection (`compact_local_bindings'):
                // an ordinary GNU table keeps its slot array, which would hold
                // the high-water mark of every interactive closure ever made.
                let table = hash::HashTable::new_local_bindings();
                self.vm.slots[slot].value = self.vm.allocate_object(data::Object::HashTable(Box::new(table)));
            }
            let table = self.vm.slots[slot].value;
            let entry = self.cons(spec, thunk);
            self.vm.declaration_hash_put(table, closure, entry).map_err(Fault::Error)
        }
        pub(crate) fn lifecycle_buffer(&mut self, arg: Value, create: bool, inhibit: bool) -> std::result::Result<(Value, bool), Fault> {
            if matches!(self.kind(arg), Kind::Buffer | Kind::ByteBuffer) { return Ok((arg, false)); }
            let name = self.string_text(arg)?;
            if let Some(buffer) = self.vm.named_buffer(&name) { return Ok((buffer, false)); }
            if !create { return Err(Fault::Error(format!("No buffer named {name}"))); }
            let buffer = self.vm.create_named_buffer(&name, false).map_err(|error| {
                match self.vm.propagated_escape.take() {
                    Some(crate::threaded::Escape::Signal(condition, data)) => Fault::Signal(condition, data),
                    escape => {
                        self.vm.propagated_escape = escape;
                        Fault::Error(error)
                    }
                }
            })?;
            self.vm.buffer_state_mut(buffer).map_err(Fault::Error)?.inhibit_buffer_hooks = inhibit;
            Ok((buffer, true))
        }
        pub(crate) fn kill_buffer(&mut self, buffer: Value) -> std::result::Result<(), Fault> {
            self.vm.kill_buffer_model(buffer).map_err(Fault::Error)
        }
        pub(crate) fn buffer_state(&self, buffer: Value) -> Option<&crate::bytes::BufferState> {
            self.vm.buffer_state(buffer).ok()
        }
        #[inline(always)]
        pub fn pair(&self, v: Value) -> Option<(Value, Value)> {
            (v.tag() == 3).then(|| self.vm.cons_pair(v).ok()).flatten()
        }
        #[inline(always)]
        pub fn cons(&mut self, car: Value, cdr: Value) -> Value {
            let i = self.vm.conses.len();
            self.vm.conses.push((car, cdr));
            Value::reference(i, 3)
        }
        pub fn buffer_name_exists(&self, name: &str) -> bool {
            name == "*scratch*" || self.vm.names.get(&format!("#:vm2-buffer:{name}"))
                .is_some_and(|&slot| {
                    self.vm.buffer_state(self.vm.slots[slot].value).ok()
                        .is_some_and(|state| state.live && state.name.as_deref() == Some(name))
                })
        }
        /// GNU concat2 retains NAME's text properties, not those of IGNORE.
        pub fn buffer_name_string(&mut self, name: Value, bytes: Vec<u8>, multibyte: bool) -> Value {
            let properties = match self.vm.object_if(name) {
                Some(data::Object::String(s) | data::Object::RawString(s)) => Some(s.properties.clone()),
                _ => None,
            };
            if let Some(properties) = properties.filter(|p| !p.is_empty()) {
                let mut string = data::LispString::from_bytes(bytes, multibyte);
                string.properties = properties;
                let string = Box::new(string);
                let object = if string.is_utf8() { data::Object::String(string) }
                    else { data::Object::RawString(string) };
                self.vm.allocate_object(object)
            } else {
                self.vm.make_lisp_string(bytes, multibyte)
            }
        }
        pub fn string(&mut self, text: &str) -> Value {
            self.vm
                .allocate_object(data::Object::String(Box::new(data::LispString::new(text))))
        }
        pub fn string_view(
            &self,
            v: Value,
        ) -> std::result::Result<crate::lstring::OwnedStr, Fault> {
            self.vm
                .str_view(v)
                .map(|view| view.to_owned())
                .ok_or(Fault::Wrong("stringp", v))
        }
        pub(crate) fn casing_multibyte(&mut self) -> std::result::Result<bool, Fault> {
            let i = self.vm.buffer_index().map_err(Fault::Error)?;
            Ok(matches!(self.vm.objects[i], data::Object::Buffer(_)))
        }
        /// Owned word flags from the current syntax table, ignoring text
        /// syntax properties as GNU's string case conversion does.
        pub(crate) fn casing_word_flags(&mut self, codes: &[u32], defaults: &[u8]) -> std::result::Result<Vec<bool>, Fault> {
            use romacs_regex_engine::SyntaxClass;
            let symbols = if let Some(&slot) = self.vm.names.get("case-symbols-as-words") {
                let value = self.vm.variable_value(slot).map_err(Fault::Error)?;
                value != Value::NIL && value != Value::UNBOUND
            } else { false };
            let i = self.vm.buffer_index().map_err(Fault::Error)?;
            let table = match &self.vm.objects[i] {
                data::Object::Buffer(b) => b.syntax_table,
                data::Object::ByteBuffer(b) => b.syntax_table,
                _ => unreachable!("buffer_index validates"),
            };
            let standard = self.vm.names.get("#:vm2-standard-syntax").map(|&s| self.vm.slots[s].value);
            let lookup = |table, code| self.vm.regexp_table_class(table, code);
            Ok(codes.iter().zip(defaults).map(|(&code, &default)| {
                let class = lookup(table, code)
                    .or_else(|| standard.and_then(|t| lookup(t, code)))
                    .unwrap_or(match default { 1 => SyntaxClass::Word, 2 => SyntaxClass::Symbol, _ => SyntaxClass::Punctuation });
                class == SyntaxClass::Word || (symbols && class == SyntaxClass::Symbol)
            }).collect())
        }
        pub fn make_string(&mut self, bytes: Vec<u8>, multibyte: bool) -> Value {
            self.vm.make_lisp_string(bytes, multibyte)
        }
        pub fn byte_string(&mut self, bytes: &[u8]) -> Value {
            self.vm.byte_string(bytes)
        }
        /// Owned Unicode text; non-ASCII unibyte data requires `string_bytes`.
        pub fn string_text(&self, v: Value) -> std::result::Result<String, Fault> {
            self.vm
                .str_view(v)
                .ok_or(Fault::Wrong("stringp", v))?
                .as_str()
                .map(str::to_owned)
                .ok_or(Fault::NotUnicode(v))
        }
        /// Owned storage bytes (UTF-8 for multibyte strings, raw for unibyte).
        pub fn string_bytes(&self, v: Value) -> std::result::Result<Vec<u8>, Fault> {
            self.vm
                .str_view(v)
                .map(|view| view.bytes().to_vec())
                .ok_or(Fault::Wrong("stringp", v))
        }
        /// GNU `fetch_string_char_advance` at byte offset AT of the string V
        /// as it is NOW, without copying it: the raw byte on unibyte storage,
        /// STRING_CHAR on multibyte. Returns the code and its byte width, or
        /// None when V is no longer a string or AT is not the start of a
        /// whole character in it. A caller that saved AT across Lisp gets
        /// None, never a panic, if that Lisp changed the string's bytes.
        pub fn string_char_at(&self, v: Value, at: usize) -> Option<(u32, usize)> {
            let view = self.vm.str_view(v)?;
            let bytes = view.bytes().get(at..).filter(|b| !b.is_empty())?;
            if !view.is_multibyte() {
                return Some((u32::from(bytes[0]), 1));
            }
            let width = match bytes[0] {
                0x00..=0x7f => 1,
                0xc0..=0xdf => 2,
                0xe0..=0xef => 3,
                0xf0..=0xf7 => 4,
                0xf8 => 5,
                _ => return None,
            };
            let whole = bytes.get(..width)?;
            whole[1..].iter().all(|b| b & 0xc0 == 0x80).then(|| crate::lstring::string_char(whole))
        }
        pub fn concat_result(&mut self, bytes: Vec<u8>, multibyte: bool, parts: &[(Value, usize)]) -> Value {
            let mut properties = crate::properties::Properties::new();
            for &(value, offset) in parts {
                if let Some(data::Object::String(s) | data::Object::RawString(s)) = self.vm.object_if(value) {
                    properties.copy_into(crate::coordinates::ByteOffset(offset), &s.properties);
                }
            }
            if properties.is_empty() { return self.vm.make_lisp_string(bytes, multibyte); }
            let mut string = data::LispString::from_bytes(bytes, multibyte);
            string.properties = properties;
            let object = if string.is_utf8() { data::Object::String(Box::new(string)) }
                else { data::Object::RawString(Box::new(string)) };
            self.vm.allocate_object(object)
        }
        pub fn concat_ascii(&mut self, args: &[Value]) -> Option<Value> {
            let mut size=0;
            for &arg in args {
                let view=self.vm.str_view(arg)?;
                if !(view.is_multibyte() && view.len()==view.bytes().len()) && !view.bytes().is_ascii() { return None; }
                size+=view.bytes().len();
            }
            let mut bytes=Vec::with_capacity(size);
            let mut parts=Vec::with_capacity(args.len());
            for &arg in args {
                parts.push((arg, bytes.len()));
                bytes.extend_from_slice(self.vm.str_view(arg)?.bytes());
            }
            Some(self.concat_result(bytes,false,&parts))
        }
        pub fn clear_string(&mut self, value: Value) -> std::result::Result<Value, Fault> {
            if Vm::image_value(value) { return Err(Fault::Error("immutable image string".into())); }
            let bytes=self.string_view(value)?.bytes.len();
            self.vm.objects[value.payload() as usize-1]=data::Object::Bytes(vec![0;bytes]);
            Ok(Value::NIL)
        }
        pub fn sxhash(&self, value: Value) -> Value {
            Value::integer((self.vm.public_sxhash(value) & ((1u64 << 60)-1)) as i64).unwrap()
        }
        pub fn fill_string(&mut self, value: Value, code: u32) -> std::result::Result<Value, Fault> {
            if Vm::image_value(value) { return Err(Fault::Error("immutable image string".into())); }
            let view = self.string_view(value)?;
            let mut bytes = Vec::new();
            if view.multibyte {
                let mut encoded=[0;5]; let width=crate::lstring::char_string(code,&mut encoded);
                if width * view.chars != view.bytes.len() { return Err(Fault::Error("Attempt to change byte length of a string".into())); }
                for _ in 0..view.chars { bytes.extend_from_slice(&encoded[..width]); }
            } else { bytes.resize(view.chars, code as u8); }
            let old = &mut self.vm.objects[value.payload() as usize - 1];
            let properties = match old { data::Object::String(s) | data::Object::RawString(s) => Some(s.properties.clone()), _=>None };
            let mut string = data::LispString::from_bytes(bytes, view.multibyte);
            if let Some(mut properties) = properties {
                // Resize the original layers, retaining stamps, ordering and run identity.
                // After each resize, the following old character starts at the new offset.
                if !properties.is_empty() {
                    let mut byte = 0;
                    let new_width = if view.multibyte { string.bytes().len() / view.chars } else { 1 };
                    for code in view.chars() {
                        let old_width = if view.multibyte {
                            let mut encoded = [0; 5];
                            crate::lstring::char_string(code, &mut encoded)
                        } else { 1 };
                        if old_width != new_width {
                            properties.resize_character(crate::coordinates::ByteOffset(byte)..crate::coordinates::ByteOffset(byte + old_width), new_width);
                        }
                        byte += new_width;
                    }
                }
                string.properties = properties;
            }
            *old = if !view.multibyte && matches!(old,data::Object::Bytes(_)) { data::Object::Bytes(string.bytes().to_vec()) }
                else if string.is_utf8() { data::Object::String(Box::new(string)) } else { data::Object::RawString(Box::new(string)) };
            Ok(value)
        }
        pub fn record(&mut self, values: Vec<Value>) -> std::result::Result<Value, Fault> {
            self.vm.allocate_record(values).map_err(Fault::Error)
        }
        pub fn record_type(&self, value: Value) -> Value {
            let Some(data::Object::Record(slots)) = self.vm.object_if(value) else { unreachable!() };
            let tag = slots[0];
            match self.vm.object_if(tag) {
                Some(data::Object::Record(class)) if class.len() > 1 => class[1],
                _ => tag,
            }
        }
        pub fn vector(&mut self, values: &[Value]) -> Value {
            self.vm
                .allocate_object(data::Object::Vector(values.to_vec()))
        }
        /// Allocate a vector that takes VALUES without copying (make-vector
        /// reserves fallibly, so a large vector is never held twice).
        pub fn vector_from(&mut self, values: Vec<Value>) -> Value {
            self.vm.allocate_object(data::Object::Vector(values))
        }
        /// GNU editfns.c Fpropertize: `Fcopy_sequence (STRING)` (bytes,
        /// multibyteness and existing text properties), then
        /// `Fadd_text_properties` over the whole copy, one property at a time
        /// in PROPERTIES order. A key already present keeps its place (its
        /// stamp) and takes the new value; a new key goes first, as
        /// textprop.c add_properties prepends it.
        pub fn propertized_copy(
            &mut self,
            string: Value,
            properties: &[(String, Value)],
        ) -> std::result::Result<Value, Fault> {
            let view = self.string_view(string)?;
            let mut copy = data::LispString::from_bytes(view.bytes, view.multibyte);
            if let Some(data::Object::String(s) | data::Object::RawString(s)) = self.vm.object_if(string) {
                copy.properties = s.properties.clone();
            }
            let all = coordinates::ByteOffset(0)..coordinates::ByteOffset(copy.bytes().len());
            for (key, value) in properties {
                copy.properties.put(all.clone(), key, *value);
            }
            let object = if copy.is_utf8() {
                data::Object::String(Box::new(copy))
            } else {
                data::Object::RawString(Box::new(copy))
            };
            Ok(self.vm.allocate_object(object))
        }
        pub fn vector_values(&self, v: Value) -> std::result::Result<Vec<Value>, Fault> {
            match self.vm.object_if(v) {
                Some(data::Object::Vector(a)) => Ok(a.clone()),
                _ => Err(Fault::Wrong("vectorp", v)),
            }
        }
        pub fn vector_len(&self, v: Value) -> std::result::Result<usize, Fault> {
            match self.vm.object_if(v) {
                Some(data::Object::Vector(a)) => Ok(a.len()),
                _ => Err(Fault::Wrong("vectorp", v)),
            }
        }
        pub fn vector_get(&self, v: Value, index: Value) -> std::result::Result<Value, Fault> {
            let n = index
                .integer_value()
                .ok_or(Fault::Wrong("integerp", index))?;
            match self.vm.object_if(v) {
                Some(data::Object::Vector(a)) => usize::try_from(n)
                    .ok()
                    .and_then(|i| a.get(i).copied())
                    .ok_or(Fault::Range(v, index)),
                _ => Err(Fault::Wrong("vectorp", v)),
            }
        }
        pub fn vector_set(
            &mut self,
            v: Value,
            index: Value,
            value: Value,
        ) -> std::result::Result<Value, Fault> {
            let n = index
                .integer_value()
                .ok_or(Fault::Wrong("integerp", index))?;
            if !matches!(self.vm.object_if(v), Some(data::Object::Vector(_))) {
                return Err(Fault::Wrong("vectorp", v));
            }
            let data::Object::Vector(a) = &mut self.vm.objects[v.payload() as usize - 1] else {
                unreachable!()
            };
            let item = usize::try_from(n)
                .ok()
                .and_then(|i| a.get_mut(i))
                .ok_or(Fault::Range(v, index))?;
            *item = value;
            Ok(value)
        }
        pub fn setcar(&mut self, v: Value, value: Value) -> std::result::Result<Value, Fault> {
            self.set_pair(v, value, true)
        }
        pub fn setcdr(&mut self, v: Value, value: Value) -> std::result::Result<Value, Fault> {
            self.set_pair(v, value, false)
        }
        fn set_pair(
            &mut self,
            v: Value,
            value: Value,
            car: bool,
        ) -> std::result::Result<Value, Fault> {
            // Image conses are readable, never writable. No VM reference escapes.
            if Vm::image_value(v) {
                return Err(Fault::Wrong("consp", v));
            }
            let pair = v
                .index_if(3)
                .and_then(|i| self.vm.conses.get_mut(i))
                .ok_or(Fault::Wrong("consp", v))?;
            if car {
                pair.0 = value;
            } else {
                pair.1 = value;
            }
            Ok(value)
        }
        pub fn read_from_string(&mut self, string: Value, start: Value, end: Value) -> std::result::Result<Value, Fault> {
            let len = self.vm.str_view(string).ok_or(Fault::Wrong("stringp", string))?.len() as i64;
            let index = |v: Value, default| {
                if v == Value::NIL { Ok(default) }
                else { v.as_integer().map_err(|_| Fault::Wrong("integerp", v)) }
            };
            let a = index(start, 0)?;
            let b = index(end, len)?;
            let a = if a < 0 { len + a } else { a };
            let b = if b < 0 { len + b } else { b };
            if a < 0 || b < a || b > len {
                return Err(Fault::Range3(string, start, end));
            }
            let read = self.vm.read_lisp_string(string, a as usize, b as usize).map_err(|(message, signal)| {
                if let Some((condition, data)) = signal { Fault::Signal(condition, data) }
                else if message.starts_with("end-of-file") { Fault::EndOfFile }
                else { Fault::ReadSyntax(message) }
            })?.map(|(value, _, end)| (value, end)).ok_or(Fault::EndOfFile)?;
            let position = Value::integer(a + read.1 as i64).map_err(Fault::Error)?;
            Ok(self.cons(read.0, position))
        }
        pub fn read(&mut self, mut stream: Value) -> std::result::Result<Value, Fault> {
            if stream == Value::NIL {
                let symbol = self.intern("standard-input");
                stream = self.variable_value(symbol)?.unwrap_or(Value::TRUE);
            }
            if stream == Value::TRUE {
                return Err(self.read_error("Reader interactive stdin is not wired"));
            }
            let root = self.vm.external_roots.len();
            self.vm.external_roots.push(stream);
            let result=self.vm.read_lisp_stream(stream);
            let stream = self.vm.external_roots[root];
            self.vm.external_roots.truncate(root);
            let result=result.map_err(|(message,signal)|{
                if let Some((condition,data))=signal {Fault::Signal(condition,data)}
                else if message.starts_with("end-of-file") {self.read_eof(stream)}
                else if message.starts_with("Reader ") || message.contains("stream is not wired") {self.read_error(&message)}
                else {Fault::ReadSyntax(message)}
            })?;
            match result {
                Some((value,_,_))=>Ok(value),
                None=>Err(self.read_eof(stream)),
            }
        }
        fn read_error(&mut self, message: &str) -> Fault {
            let condition=self.vm.intern("error");
            let text=self.vm.make_lisp_string(message.as_bytes().to_vec(),true);
            let data=self.vm.list_value(&[text]);
            Fault::Signal(condition,data)
        }
        fn read_eof(&mut self, stream: Value) -> Fault {
            if matches!(self.vm.object_if(stream),Some(data::Object::Buffer(_)|data::Object::ByteBuffer(_))) {
                let condition=self.vm.intern("end-of-file");
                let data=self.vm.list_value(&[stream]);
                Fault::Signal(condition,data)
            }else{Fault::EndOfFile}
        }
        pub fn translate_match_data(&mut self, delta: i64) {
            self.vm.regexp_translate_match_data(delta);
        }
        pub fn set_match_data(&mut self, list: Value, reseat: Value) -> std::result::Result<Value, Fault> {
            self.vm.regexp_set_match_data(list, reseat)
        }
        /// Owned integer payload: no arena borrow escapes the leaf boundary.
        pub fn bigint_value(&self, v: Value) -> Option<num_bigint::BigInt> {
            self.vm.bigint_value(v)
        }
        pub fn bigint(&mut self, n: num_bigint::BigInt) -> Value {
            self.vm.bigint(n)
        }
        pub fn float(&mut self, value: f64) -> Value {
            self.vm.allocate_object(data::Object::Float(value))
        }
        /// GNU `marker_position`: `None` if V is not a marker, `Some(None)`
        /// if it points nowhere, else its 1-based character position.
        pub fn marker_position(&self, v: Value) -> Option<Option<i64>> {
            if !matches!(self.vm.object_if(v), Some(data::Object::Marker(_))) {
                return None;
            }
            Some(self.vm.marker_position_value(v).ok().and_then(|p| p.integer_value()))
        }
        pub fn float_value(&self, v: Value) -> std::result::Result<f64, Fault> {
            match self.vm.object_if(v) {
                Some(data::Object::Float(n)) => Ok(*n),
                _ => Err(Fault::Wrong("floatp", v)),
            }
        }
        pub fn eq(&self, a: Value, b: Value) -> bool {
            a == b
        }
        pub fn eql(&self, a: Value, b: Value) -> bool {
            self.vm.eql(a, b)
        }
        /// Reuses GNU-compatible cycle/depth handling, preserving its typed errors.
        pub fn equal(&self, a: Value, b: Value) -> std::result::Result<bool, hash::HashFault> {
            self.vm.equal(a, b)
        }
        fn symbol_slot(&self, v: Value) -> std::result::Result<usize, Fault> {
            v.index_if(7)
                .and_then(|i| i.checked_sub(1))
                .filter(|&i| i < self.vm.slots.len())
                .ok_or(Fault::Wrong("symbolp", v))
        }
        pub fn symbol_name(&self, v: Value) -> std::result::Result<String, Fault> {
            if v == Value::NIL {
                return Ok("nil".into());
            }
            if v == Value::TRUE {
                return Ok("t".into());
            }
            Ok(self.vm.symbol_names[self.symbol_slot(v)?].clone())
        }
        /// Outermost saved default, before alias resolution (GNU eval.c:747).
        /// `None` means no saved binding; a saved void value is a condition.
        pub fn saved_default_cell(&mut self, v: Value) -> std::result::Result<Option<Value>, Fault> {
            if v == Value::NIL || v == Value::TRUE { return Ok(None); }
            let slot = self.symbol_slot(v)?;
            let Some(binding) = self.vm.default_toplevel_binding(slot) else { return Ok(None) };
            let value = self.vm.dynamic[binding].1;
            if value == Value::UNBOUND { Err(Fault::VoidVariable(v)) } else { Ok(Some(value)) }
        }
        /// Update only the oldest saved default, leaving all active cells intact.
        pub fn saved_default_cell_set(&mut self, v: Value, value: Value) -> std::result::Result<bool, Fault> {
            if v == Value::NIL || v == Value::TRUE { return Ok(false); }
            let slot = self.symbol_slot(v)?;
            let Some(binding) = self.vm.default_toplevel_binding(slot) else { return Ok(false) };
            self.vm.dynamic[binding].1 = value;
            Ok(true)
        }
        /// Resolve the current buffer's binding, including dynamic/local redirects.
        pub fn effective_cell(&mut self, v: Value) -> std::result::Result<Value, Fault> {
            if v == Value::NIL || v == Value::TRUE { return Ok(v); }
            let slot = self.symbol_slot(v)?;
            let value = self.vm.variable_value(slot).map_err(Fault::Error)?;
            if value == Value::UNBOUND { Err(Fault::VoidVariable(v)) } else { Ok(value) }
        }
        /// Raw/default value cell, not buffer-local variable resolution.
        pub fn default_cell(&mut self, v: Value) -> std::result::Result<Value, Fault> {
            if v == Value::NIL || v == Value::TRUE {
                return Ok(v);
            }
            let base = self.vm.indirect_variable(self.symbol_slot(v)?);
            // GNU default_value, SYMBOL_FORWARDED arm (data.c:1983).
            if let Some(fwd) = self.vm.forwarding(base) {
                return self.vm.forwarded_default(fwd).map_err(Fault::Error);
            }
            let value = self.vm.slots[base].value;
            if value == Value::UNBOUND {
                Err(Fault::VoidVariable(v))
            } else {
                Ok(value)
            }
        }
        pub fn default_cell_set(
            &mut self,
            v: Value,
            value: Value,
        ) -> std::result::Result<Value, Fault> {
            if v == Value::NIL || v == Value::TRUE {
                return Err(Fault::SettingConstant(v));
            }
            let slot = self.vm.indirect_variable(self.symbol_slot(v)?);
            // Only an INTERNED keyword is SYMBOL_NOWRITE (GNU lread.c
            // intern_sym); an uninterned `:x' is an ordinary variable.
            let name = &self.vm.symbol_names[slot];
            if self.vm.protected_variable_slot(slot)
                || (name.starts_with(':') && self.vm.names.get(name) == Some(&slot))
            {
                return Err(Fault::SettingConstant(v));
            }
            // GNU set_default_internal, SYMBOL_FORWARDED arm (data.c:2085).
            if let Some(fwd) = self.vm.forwarding(slot) {
                self.vm.set_forwarded_default(slot, fwd, value, crate::forwarded::SetFlag::Set)?;
                return Ok(value);
            }
            self.vm.slots[slot].value = value;
            Ok(value)
        }
        /// GNU `SYMBOL_CONSTANT_P` (lisp.h): nil, t, keywords and the
        /// read-only built-ins VM2 protects.
        pub fn variable_constant(&self, v: Value) -> std::result::Result<bool, Fault> {
            if v == Value::NIL || v == Value::TRUE {
                return Ok(true);
            }
            let slot = self.symbol_slot(v)?;
            Ok(self.vm.protected_variable_slot(slot) || self.vm.symbol_names[slot].starts_with(':'))
        }
        /// GNU `SYMBOL_FORWARDED`: V is a built-in variable. Not followed
        /// through aliases (an alias is `SYMBOL_VARALIAS`).
        pub fn variable_forwarded(&mut self, v: Value) -> std::result::Result<bool, Fault> {
            if v == Value::NIL || v == Value::TRUE {
                return Ok(false);
            }
            let slot = self.symbol_slot(v)?;
            Ok(self.vm.forwarding(slot).is_some())
        }
        /// One step of GNU's `SYMBOL_VARALIAS` redirect; `None` if V is not
        /// an alias. nil and t are never aliases.
        pub fn variable_alias(&self, v: Value) -> std::result::Result<Option<Value>, Fault> {
            if v == Value::NIL || v == Value::TRUE {
                return Ok(None);
            }
            let slot = self.symbol_slot(v)?;
            Ok(match self.vm.variable_redirects.get(slot) {
                Some(Some(VariableRedirect::Alias(base))) => Some(Value::reference(base + 1, 7)),
                _ => None,
            })
        }
        /// GNU `SYMBOL_LOCALIZED`: V itself has buffer-local bindings
        /// (made local, or automatically buffer-local). Not followed through
        /// aliases: an alias is `SYMBOL_VARALIAS`, never localized.
        pub fn variable_localized(&mut self, v: Value) -> std::result::Result<bool, Fault> {
            if v == Value::NIL || v == Value::TRUE {
                return Ok(false);
            }
            let slot = self.symbol_slot(v)?;
            Ok(self.vm.local_table(slot).map_err(Fault::Error)?.is_some())
        }
        /// A live dynamic (`let`) binding of V, GNU's `specpdl` scan in
        /// `Fdefvaralias`. Bindings record the variable at the chain's end.
        pub fn variable_let_bound(&self, v: Value) -> std::result::Result<bool, Fault> {
            if v == Value::NIL || v == Value::TRUE {
                return Ok(false);
            }
            let slot = self.vm.indirect_variable(self.symbol_slot(v)?);
            Ok(self.vm.dynamic.iter().any(|(bound, ..)| *bound == slot))
        }
        /// GNU `find_symbol_value`: the current binding through aliases,
        /// `None` when void. nil and t are their own values.
        pub fn variable_value(&mut self, v: Value) -> std::result::Result<Option<Value>, Fault> {
            if v == Value::NIL || v == Value::TRUE {
                return Ok(Some(v));
            }
            let slot = self.symbol_slot(v)?;
            let value = self.vm.variable_value(slot).map_err(Fault::Error)?;
            Ok((value != Value::UNBOUND).then_some(value))
        }
        /// GNU `set_internal (..., SET_INTERNAL_BIND)` of an unbound variable.
        pub fn variable_set(&mut self, v: Value, value: Value) -> std::result::Result<(), Fault> {
            if v == Value::NIL || v == Value::TRUE {
                return Err(Fault::SettingConstant(v));
            }
            let slot = self.symbol_slot(v)?;
            self.vm.set_variable_value(slot, value).map_err(Fault::Error)
        }
        /// Publish NEW as an alias of BASE once `defvaralias` has validated
        /// it. nil and t are immediates in VM2, so an alias of either targets
        /// the named slot, holding its own constant value.
        pub fn make_variable_alias(&mut self, new: Value, base: Value) -> std::result::Result<(), Fault> {
            let new = self.symbol_slot(new)?;
            let base = if base == Value::NIL || base == Value::TRUE {
                let slot = self.vm.slot(if base == Value::NIL { "nil" } else { "t" });
                self.vm.slots[slot].value = base;
                slot
            } else {
                let slot = self.symbol_slot(base)?;
                // GNU binds an interned keyword to itself (lread.c intern);
                // VM2 self-evaluates keywords without a value cell, so give
                // the cell its GNU value before an alias can read it.
                if self.vm.symbol_names[slot].starts_with(':') && self.vm.slots[slot].value == Value::UNBOUND {
                    self.vm.slots[slot].value = base;
                }
                slot
            };
            self.vm.make_variable_alias(new, base);
            Ok(())
        }
        /// Mark SYMBOL itself special, without following its value redirect.
        /// GNU eval.c:908 also permits constant symbols; this does not write
        /// their value cells. Compiled lexical bindings, unlike interpreter
        /// environments, are not visible to GNU's lexbound_p.
        pub fn declare_variable_special(&mut self, symbol: Value) -> std::result::Result<(), Fault> {
            let name = self.symbol_name(symbol)?;
            // `make-symbol` preserves its spelling, not obarray identity.
            if symbol == Value::NIL || symbol == Value::TRUE {
                self.vm.specials.insert(name);
            } else {
                let slot = self.symbol_slot(symbol)?;
                self.vm.slots[slot].declared_special = true;
                if self.vm.names.get(&name) == Some(&slot) {
                    self.vm.specials.insert(name);
                }
            }
            Ok(())
        }
        /// Clear the symbol's declaration without following a variable alias.
        pub fn make_variable_non_special(&mut self, symbol: Value) -> std::result::Result<(), Fault> {
            let name = self.symbol_name(symbol)?;
            if symbol == Value::NIL || symbol == Value::TRUE {
                self.vm.specials.remove(&name);
            } else {
                let slot = self.symbol_slot(symbol)?;
                self.vm.slots[slot].declared_special = false;
                if self.vm.names.get(&name) == Some(&slot) {
                    self.vm.specials.remove(&name);
                }
            }
            Ok(())
        }
        /// GNU LOADHIST_ATTACH (lread.h), also outside an active load.
        pub fn attach_variable_history(&mut self, symbol: Value) -> std::result::Result<(), Fault> {
            let slot = self.vm.slot("current-load-list");
            let old = self.vm.variable_value(slot).map_err(Fault::Error)?;
            let history = self.cons(symbol, old);
            self.vm.set_variable_value(slot, history).map_err(Fault::Error)
        }
        /// Declaration metadata only: GNU custom.el's custom-autoload/custom-add-load.
        pub fn custom_autoload_metadata(&mut self, symbol: Value, load: Value, noset: Value) -> super::Out {
            self.custom_load_metadata(symbol, load, Some(noset))
        }
        pub fn custom_load_metadata(&mut self, symbol: Value, load: Value, noset: Option<Value>) -> super::Out {
            self.symbol_slot(symbol)?;
            let put_fault = |fault| match fault {
                annotations::PlistFault::Symbol(v) => Fault::Wrong("symbolp", v),
                annotations::PlistFault::Plistp(v) => Fault::Wrong("plistp", v),
                annotations::PlistFault::Circular(v) => Fault::Circular(v),
            };
            if let Some(noset) = noset {
                let autoload = self.intern("custom-autoload");
                let marker = if noset == Value::NIL { Value::TRUE } else { self.intern("noset") };
                self.vm.put_symbol_property_checked(symbol, autoload, marker).map_err(put_fault)?;
            }
            let key = self.intern("custom-loads");
            let loads = self.vm.symbol_property(symbol, key).map_err(Fault::Error)?;
            let mut tail = loads;
            let mut seen = std::collections::HashSet::new();
            while tail != Value::NIL {
                if !seen.insert(tail.0) { return Err(Fault::Circular(tail)); }
                let (item, next) = self.pair(tail).ok_or(Fault::Wrong("listp", tail))?;
                if self.equal(item, load).map_err(|e| match e {
                    hash::HashFault::Wrong(p, v) => Fault::Wrong(p, v),
                    hash::HashFault::Error("circular-list", Some(v)) => Fault::Circular(v),
                    hash::HashFault::Error(m, _) => Fault::Error(m.into()),
                    _ => Fault::Error("equal failed".into()),
                })? { return Ok(Value::NIL); }
                tail = next;
            }
            let loads = self.cons(load, loads);
            self.vm.put_symbol_property_checked(symbol, key, loads).map_err(put_fault)?;
            Ok(loads)
        }
        /// `(put SYMBOL 'variable-documentation DOC)`, GNU eval.c:723.
        pub fn put_variable_documentation(&mut self, v: Value, doc: Value) -> std::result::Result<(), Fault> {
            let key = self.vm.intern("variable-documentation");
            self.vm.put_symbol_property_checked(v, key, doc).map_err(|fault| match fault {
                annotations::PlistFault::Symbol(v) => Fault::Wrong("symbolp", v),
                annotations::PlistFault::Plistp(v) => Fault::Wrong("plistp", v),
                annotations::PlistFault::Circular(v) => Fault::Circular(v),
            })?;
            Ok(())
        }
        /// GNU calls `display-warning`, which writes to stderr in batch.
        pub fn batch_warning(&self, kind: &str, text: &str) {
            eprintln!("Warning ({kind}): {text}");
        }
        /// The function cell itself; aliases are not followed and void is nil.
        pub fn symbol_function(&self, v: Value) -> std::result::Result<Value, Fault> {
            let slot = if v == Value::NIL || v == Value::TRUE {
                let name = if v == Value::NIL { "nil" } else { "t" };
                let Some(&slot) = self.vm.names.get(name) else {
                    return Ok(Value::NIL);
                };
                slot
            } else {
                self.symbol_slot(v)?
            };
            let value = self.vm.slots[slot].function;
            Ok(if value == Value::UNBOUND {
                Value::NIL
            } else {
                value
            })
        }
        /// Reconstruct the Lisp macro descriptor from the versioned slot flag.
        /// Leaf allocation cannot compact, so RAW stays valid until returned.
        pub fn symbol_definition(&mut self, v: Value) -> std::result::Result<Value, Fault> {
            let raw = self.symbol_function(v)?;
            if v.index_if(7).and_then(|i| i.checked_sub(1))
                .is_some_and(|slot| self.vm.macros.contains(&slot)) {
                let tag = self.intern("macro");
                Ok(self.cons(tag, raw))
            } else {
                Ok(raw)
            }
        }
        /// Writes a function cell without executing or validating a callable descriptor.
        pub fn set_symbol_function(
            &mut self,
            v: Value,
            value: Value,
        ) -> std::result::Result<Value, Fault> {
            if v == Value::NIL {
                return if value == Value::NIL {
                    Ok(value)
                } else {
                    Err(Fault::SettingConstant(v))
                };
            }
            let slot = if v == Value::TRUE {
                self.vm.slot("t")
            } else {
                self.symbol_slot(v)?
            };
            let (definition, is_macro) = if let Some((head, body)) = self.pair(value) {
                if head == self.intern("macro") {
                    (body, true)
                } else {
                    (value, false)
                }
            } else {
                (value, false)
            };
            let mut target = definition;
            let mut seen = HashSet::new();
            while !is_macro && target != Value::NIL && self.kind(target) == Kind::Symbol {
                if target == v || !seen.insert(target.0) {
                    return Err(Fault::CyclicFunctionIndirection(v));
                }
                target = self.symbol_function(target)?;
            }
            // Keep compiler/dispatch cache invalidation identical to defalias.
            self.vm
                .define_function(slot, definition, is_macro)
                .expect("function version exhausted");
            Ok(value)
        }
        /// `defalias`'s cell write (data.c:971): the autoload-queue record of
        /// a non-nil old definition (autoload.rs), then `set_symbol_function`.
        pub fn defalias_function(&mut self, v: Value, value: Value) -> std::result::Result<Value, Fault> {
            if v != Value::NIL && v != Value::TRUE && let Ok(slot) = self.symbol_slot(v) {
                self.vm.autoload_note_defalias(slot);
            }
            self.set_symbol_function(v, value)
        }
        /// GNU `XSYMBOL (SYMBOL)->u.s.plist` (data.c:818): the plist owned by
        /// the symbol object itself; uninterned twins never share it.
        pub fn symbol_plist(&self, symbol: Value) -> std::result::Result<Value, Fault> {
            self.vm.symbol_plist(symbol).map_err(|_| Fault::Wrong("symbolp", symbol))
        }
        /// GNU `set_symbol_plist` (lisp.h): install PLIST exactly as given.
        pub fn set_symbol_plist(&mut self, symbol: Value, plist: Value) -> std::result::Result<Value, Fault> {
            self.vm.set_symbol_plist(symbol, plist).map_err(|_| Fault::Wrong("symbolp", symbol))
        }
        pub fn is_obarray(&self, value: Value) -> bool {
            matches!(self.vm.object_if(value), Some(data::Object::Obarray(_)))
        }
        pub fn make_obarray(&mut self, size: Value) -> super::Out {
            let bits = if size == Value::NIL { 3 } else {
                let n = size.integer_value().filter(|&n| n >= 0).ok_or(Fault::Wrong("wholenump", size))? as u64;
                let bits = 64 - n.leading_zeros();
                if bits > 31 { return Err(Fault::Range1(size)); }
                bits
            };
            Ok(self.vm.allocate_object(data::Object::Obarray(Box::new(obarray::Obarray::new(bits, false)))))
        }
        pub fn resolve_obarray(&mut self, value: Value) -> super::Out { self.vm.resolve_obarray(value) }
        pub fn intern_in(&mut self, table: Value, name: Value) -> super::Out { self.vm.obarray_intern(table, name) }
        pub fn lookup_in(&mut self, table: Value, name: Value) -> super::Out {
            let symbol = self.kind(name) == Kind::Symbol;
            let text;
            let (bytes, chars) = if symbol {
                text = self.symbol_name(name)?;
                (text.as_bytes(), text.chars().count())
            } else {
                let view = self.vm.str_view(name).ok_or(Fault::Wrong("stringp", name))?;
                (view.bytes, view.chars)
            };
            let found = self.vm.obarray_lookup_value(table, bytes, chars);
            Ok(found.filter(|&v| !symbol || v == name).unwrap_or(Value::NIL))
        }
        pub fn unintern_in(&mut self, table: Value, name: Value) -> super::Out {
            let found = self.lookup_in(table, name)?;
            // nil is both a legitimate interned symbol and the missing sentinel.
            if found == Value::NIL {
                let text = if self.kind(name) == Kind::Symbol { self.symbol_name(name)? } else { self.string_text(name)? };
                if text != "nil" || self.vm.obarray_lookup_value(table, b"nil", 3) != Some(Value::NIL) { return Ok(Value::NIL); }
            }
            Ok(if self.vm.obarray_unintern(table, found) { Value::TRUE } else { Value::NIL })
        }
        pub fn obarray_next(&self, table: Value, bucket: usize, current: Value) -> Option<(usize, Value)> {
            self.vm.obarray_next(table, bucket, current)
        }
        pub fn symbol_name_value(&mut self, symbol: Value) -> super::Out {
            if let Ok(slot) = self.symbol_slot(symbol) {
                if let Some(&name) = self.vm.obarray_symbol_names.get(&slot) { return Ok(name); }
            }
            let name = self.symbol_name(symbol)?;
            Ok(if name.is_ascii() { self.byte_string(name.as_bytes()) } else { self.string(&name) })
        }
        pub fn intern(&mut self, name: &str) -> Value {
            self.vm.intern(name)
        }
        /// The window tree, through window.rs's own surface: reads, writes
        /// and allocation only, never evaluation or collection.
        pub fn windows(&mut self) -> crate::window::Windows<'_> {
            crate::window::Windows::new(self.vm)
        }
        /// Fresh symbol slot outside the obarray; never re-enters the VM.
        pub fn make_symbol(&mut self, name: &str) -> Value {
            let value = self.vm.generated_symbol(expand::generated_id());
            let slot = value.index_if(7).unwrap() - 1;
            self.vm.symbol_names[slot] = name.to_owned();
            value
        }
        /// GNU `get_keymap (OBJECT, 0, 0)` (keymap.c:178): OBJECT if it is a
        /// keymap, else the keymap at the end of its function chain, else
        /// None. Autoloaded keymaps are not loaded (DIVERGENCES.md
        /// VM2-AUTOLOAD1).
        pub fn keymap(&self, v: Value) -> Option<Value> {
            if v == Value::NIL { return None; }
            self.vm.resolve_keymap(v)
        }
        pub fn use_global_map(&mut self, map: Value) -> std::result::Result<Value, Fault> {
            self.vm.ensure_keymaps();
            let map = self.vm.resolve_keymap(map).ok_or(Fault::Wrong("keymapp", map))?;
            let slot = self.vm.names["#:vm2-global-keymap"];
            self.vm.slots[slot].value = map;
            Ok(Value::NIL)
        }
        pub fn obarray_lookup(&self, name: &str) -> Option<Value> {
            match name {
                "nil" => Some(Value::NIL),
                "t" => Some(Value::TRUE),
                _ => self.vm.names.get(name).map(|&i| Value::reference(i + 1, 7)),
            }
        }
        /// Snapshot of the default obarray; excludes uninterned/generated slots.
        pub fn obarray_symbols(&self) -> Vec<Value> {
            let mut symbols = vec![Value::NIL, Value::TRUE];
            symbols.extend(
                self.vm
                    .names
                    .iter()
                    .filter(|(name, _)| !matches!(name.as_str(), "nil" | "t"))
                    .map(|(_, &i)| Value::reference(i + 1, 7)),
            );
            symbols.sort_unstable_by_key(|v| v.0);
            symbols
        }
        pub fn kind(&self, v: Value) -> Kind {
            if v == Value::NIL || v == Value::TRUE {
                return Kind::Symbol;
            }
            if v == Value::UNBOUND {
                return Kind::Unbound;
            }
            match v.tag() {
                1 => Kind::Integer,
                3 if self.pair(v).is_some() => Kind::Cons,
                4 if (v.payload() as usize) < self.vm.closures.len() => Kind::Closure,
                5 if (v.payload() as usize) < self.vm.cells.len() => Kind::Cell,
                6 if super::descriptor(v.payload() as usize).is_some() => Kind::Subr,
                7 if self.symbol_slot(v).is_ok() => Kind::Symbol,
                0 if Vm::image_value(v) && self.vm.string_value(v).is_ok() => Kind::String,
                0 => match self.vm.object_if(v) {
                    Some(data::Object::Float(_)) => Kind::Float,
                    Some(data::Object::Bignum(_)) => Kind::Bignum,
                    Some(data::Object::String(_) | data::Object::RawString(_)) => Kind::String,
                    Some(data::Object::Bytes(_)) => Kind::Bytes,
                    Some(data::Object::Vector(_)) => Kind::Vector,
                    Some(data::Object::BoolVector(_)) => Kind::BoolVector,
                    Some(data::Object::Record(_)) => Kind::Record,
                    Some(data::Object::Buffer(_)) => Kind::Buffer,
                    Some(data::Object::Marker(_)) => Kind::Marker,
                    Some(data::Object::Overlay(_)) => Kind::Overlay,
                    Some(data::Object::ByteBuffer(_)) => Kind::ByteBuffer,
                    Some(data::Object::HashTable(_)) => Kind::HashTable,
                    Some(data::Object::Obarray(_)) => Kind::Obarray,
                    Some(data::Object::SyntaxTable(_)) => Kind::SyntaxTable,
                    Some(data::Object::CharTable(_)) => Kind::CharTable,
                    Some(data::Object::SubCharTable(_)) => Kind::SubCharTable,
                    Some(data::Object::MapKeymapCursor(_)) => Kind::Invalid,
                    Some(data::Object::Window(_)) => Kind::Window,
                    Some(data::Object::Frame(_)) => Kind::Frame,
                    Some(data::Object::Terminal(_)) => Kind::Terminal,
                    None => Kind::Invalid,
                },
                _ => Kind::Invalid,
            }
        }
        pub fn dense_keymap(&mut self, prompt: Value) -> Value {
            self.vm.ensure_keymaps();
            self.vm.dense_keymap(prompt)
        }
        pub fn standard_case_table(&mut self) -> super::Out {
            if self.vm.standard_case_tables[0] != Value::NIL {
                return Ok(self.vm.standard_case_tables[0]);
            }
            // GNU casetab.c:init_casetab_once, before Lisp Unicode enrichment.
            // This entire construction is a non-collecting leaf window.
            let purpose = self.intern("case-table");
            let mut tables = Vec::new();
            for kind in 0..4 {
                let mut table = char_table::CharTable::new(purpose);
                table.initialize(3, Value::NIL).map_err(Fault::Error)?;
                for i in 0..128usize {
                    let mapped = match kind {
                        0 | 2 if (65..=90).contains(&i) => i + 32,
                        1 | 3 if (97..=122).contains(&i) => i - 32,
                        3 if (65..=90).contains(&i) => i + 32,
                        _ => i,
                    };
                    table.set(i, Value::integer(mapped as i64).unwrap()).map_err(Fault::Error)?;
                }
                tables.push(table);
            }
            let mut values = [Value::NIL; 4];
            for (i, table) in tables.into_iter().enumerate() {
                values[i] = self.vm.allocate_object(data::Object::CharTable(Box::new(table)));
            }
            let data::Object::CharTable(down) = &mut self.vm.objects[values[0].index(0).unwrap() - 1] else { unreachable!() };
            down.extras.copy_from_slice(&values[1..]);
            let data::Object::CharTable(canon) = &mut self.vm.objects[values[2].index(0).unwrap() - 1] else { unreachable!() };
            canon.extras[2] = values[3];
            self.vm.standard_case_tables = values;
            Ok(values[0])
        }
        /// Existing char-table lookup, including defaults and parent fallback.
        /// Callers validate the Lisp character index before entering the substrate.
        pub fn char_table_element(&self, table: Value, index: usize) -> super::Out {
            self.vm.char_table_get(table, index).map_err(Fault::Error)
        }
        pub fn make_char_table(&mut self, purpose: Value, init: Value) -> super::Out {
            let mut plist = self.symbol_plist(purpose)?;
            let property = self.intern("char-table-extra-slots");
            let mut count = Value::NIL;
            for _ in 0..self.list_limit() {
                let Some((key, tail)) = self.pair(plist) else { break };
                let Some((value, tail)) = self.pair(tail) else { break };
                if key == property { count = value; break; }
                plist = tail;
            }
            let n = if count == Value::NIL { 0 } else {
                count.as_integer().ok().filter(|&n| n >= 0)
                    .ok_or(Fault::Wrong("wholenump", count))? as usize
            };
            let mut table = char_table::CharTable::new(purpose);
            table.initialize(n, init).map_err(Fault::Error)?;
            let slot = self.vm.literal_table_slot(init);
            table.root.lock().unwrap().slots.fill(slot.clone());
            table.ascii = crate::char_table::CharTable::ascii_from_root(&table.root);
            Ok(self.vm.allocate_object(data::Object::CharTable(Box::new(table))))
        }
        pub fn char_table_extra(&mut self, table: Value, n: Value, set: Option<Value>) -> super::Out {
            let Some(data::Object::CharTable(t)) = self.vm.object_if(table) else {
                return Err(Fault::Wrong("char-table-p", table));
            };
            let i = n.as_integer().map_err(|_| Fault::Wrong("fixnump", n))?;
            if i < 0 || i as usize >= t.extras.len() { return Err(Fault::Range(table, n)); }
            let old = t.extras[i as usize];
            if let Some(value) = set {
                let data::Object::CharTable(t) = &mut self.vm.objects[table.index(0).unwrap() - 1] else { unreachable!() };
                t.extras[i as usize] = value;
                Ok(value)
            } else { Ok(old) }
        }
        pub fn char_table_subtype(&self, table: Value) -> std::result::Result<Value, Fault> {
            match self.vm.object_if(table) {
                Some(data::Object::CharTable(t)) => Ok(t.purpose),
                _ => Err(Fault::Wrong("char-table-p", table)),
            }
        }
        pub fn char_table_parent(&mut self, table: Value, parent: Option<Value>) -> super::Out {
            let Some(data::Object::CharTable(t)) = self.vm.object_if(table) else {
                return Err(Fault::Wrong("char-table-p", table));
            };
            let old = t.parent;
            if let Some(parent) = parent {
                if parent != Value::NIL && !matches!(self.vm.object_if(parent), Some(data::Object::CharTable(_))) {
                    return Err(Fault::Wrong("char-table-p", parent));
                }
                let mut next = parent;
                let mut seen = HashSet::new();
                while let Some(data::Object::CharTable(t)) = self.vm.object_if(next) {
                    if next == table || !seen.insert(next.0) { return Err(Fault::Error("Attempt to make a chartable be its own parent".into())); }
                    next = t.parent;
                }
                let data::Object::CharTable(t) = &mut self.vm.objects[table.index(0).unwrap() - 1] else { unreachable!() };
                t.parent = parent;
                Ok(parent)
            } else { Ok(old) }
        }
        pub fn set_char_table_range(&mut self, table: Value, range: Value, value: Value) -> super::Out {
            if !matches!(self.vm.object_if(table), Some(data::Object::CharTable(_))) {
                return Err(Fault::Wrong("char-table-p", table));
            }
            let bounds = if range == Value::NIL { None }
            else if range == Value::TRUE { Some((0, char_table::CharTable::LEN - 1)) }
            else if let Some(c) = range.integer_value().filter(|&n| n >= 0 && n < char_table::CharTable::LEN as i64) {
                Some((c as usize, c as usize))
            } else if let Some((from, to)) = self.pair(range) {
                let character = |v: Value| v.integer_value().filter(|&n| n >= 0 && n < char_table::CharTable::LEN as i64)
                    .map(|n| n as usize).ok_or(Fault::Wrong("characterp", v));
                Some((character(from)?, character(to)?))
            } else { return Err(Fault::Error("Invalid RANGE argument to ‘set-char-table-range’".into())); };
            let slot = self.vm.literal_table_slot(value);
            let data::Object::CharTable(t) = &mut self.vm.objects[table.index(0).unwrap() - 1] else { unreachable!() };
            if let Some((from, to)) = bounds { t.set_range_slot(from, to, slot).map_err(Fault::Error)?; }
            else { t.default = value; }
            Ok(value)
        }
        pub fn list_limit(&self) -> usize {
            self.vm.list_limit()
        }
        /// Characters FROM..TO of STRING as a fresh string of the same
        /// multibyteness, carrying STRING's text properties for that range
        /// (GNU fns.c Fsubstring: make_specified_string + copy_text_properties).
        /// The caller has validated the range against the character length.
        pub fn string_slice(&mut self, string: Value, from: usize, to: usize) -> std::result::Result<Value, Fault> {
            let view = self.vm.str_view(string).ok_or(Fault::Wrong("arrayp", string))?;
            let multibyte = view.is_multibyte();
            let byte = |c: usize| {
                if !multibyte || view.len() == view.bytes().len() {
                    return c;
                }
                let mut byte = 0;
                for _ in 0..c {
                    byte += crate::lstring::string_char(&view.bytes()[byte..]).1;
                }
                byte
            };
            let range = coordinates::ByteOffset(byte(from))..coordinates::ByteOffset(byte(to));
            let bytes = view.bytes()[range.start.0..range.end.0].to_vec();
            if let Some(data::Object::String(s) | data::Object::RawString(s)) = self.vm.object_if(string) {
                let mut slice = data::LispString::from_bytes(bytes, multibyte);
                slice.properties = s.properties.slice(range);
                let object = if slice.is_utf8() {
                    data::Object::String(Box::new(slice))
                } else {
                    data::Object::RawString(Box::new(slice))
                };
                return Ok(self.vm.allocate_object(object));
            }
            Ok(self.vm.make_lisp_string(bytes, multibyte))
        }
    }
    // Buffer text for prims/fileio.rs (src/prim/leaf/fileio.rs). A child
    // module, so it sees `vm`.
    mod map_keymap;
    mod fileio;
    mod filelock;
    mod file_attributes;
    mod terminal;
    mod process_jobs;
    mod clipboard;
    mod filesave;
    mod charset;
    mod coding;
    mod point_predicates;
    mod overlays;
    mod invisible;
    mod buffer_locals;
    mod casetab;
    mod casing;
    mod modification;
    mod buffer_lifecycle;
    mod editor_mark;
    mod printing;
    mod bool_vector;
    mod search;
    mod skip_chars;
    mod word_motion;
    mod face;
    mod value_order;
}
pub(crate) use leaf::{Leaf, Kind};

/// Realize C-initialized coding variables before their first Lisp observation.
/// Like the charset slot hook, this preserves allocation-free VM construction.
pub(crate) fn initialize_coding_slot(vm: &mut Vm, name: &str) {
    if matches!(name, "coding-system-list" | "coding-system-alist" | "coding-category-list")
        || name.starts_with("coding-category-")
    {
        assert!(Leaf::new(vm).coding_init().is_ok(), "coding seed initialization failed");
    }
}
#[cfg(test)]
#[path = "leaf_tests.rs"]
mod leaf_tests;

/// GNU's CHECK_* macros as types: the parameter type IS the check, so arity,
/// coercion and the signalled predicate are declared, never re-implemented.
pub(crate) trait Arg: Sized {
    fn check(cx: &Leaf<'_>, v: Value) -> std::result::Result<Self, Fault>;
}
impl Arg for Value {
    #[inline(always)]
    fn check(_: &Leaf<'_>, v: Value) -> std::result::Result<Self, Fault> {
        Ok(v)
    }
}
/// CHECK_INTEGER (fixnums only until VM2 has bignums).
pub(crate) struct Integer(pub i64);
impl Arg for Integer {
    #[inline(always)]
    fn check(_: &Leaf<'_>, v: Value) -> std::result::Result<Self, Fault> {
        v.integer_value()
            .map(Integer)
            .ok_or(Fault::Wrong("integerp", v))
    }
}
/// CHECK_LIST: nil or a cons.
pub(crate) struct List(pub Value);
impl Arg for List {
    #[inline(always)]
    fn check(_: &Leaf<'_>, v: Value) -> std::result::Result<Self, Fault> {
        if v == Value::NIL || v.tag() == 3 {
            Ok(List(v))
        } else {
            Err(Fault::Wrong("listp", v))
        }
    }
}

/// Declare required arguments, then `; optional a: Ty, b: Ty`, then
/// `; rest args` (MANY). Omitted optional arguments are checked as nil;
/// rest is an uncoerced `&[Value]`. Both suffixes work with T1 and `-> Step` T2.
/// Arity and the adapter's left-to-right checks are derived from the signature.
macro_rules! defsubr {
    (name = $name:literal, gnu = $gnu:literal, effect = $eff:ident,
     fn $id:ident ($($params:tt)*) -> Step $body:block) => {
        $crate::prim::defsubr!(@emit $name, $gnu, $eff, Call, Next, $id, ($($params)*), $body);
    };
    (name = $name:literal, gnu = $gnu:literal, effect = $eff:ident,
     fn $id:ident ($($params:tt)*) $body:block) => {
        $crate::prim::defsubr!(@emit $name, $gnu, $eff, Leaf, Out, $id, ($($params)*), $body);
    };
    (@max $count:expr) => { Some($count) };
    (@max $count:expr, $rest:ident) => { None };
    (@emit $name:literal, $gnu:literal, $eff:ident, $tier:ident, $out:ident, $id:ident,
     ($cx:ident $(, $arg:ident : $ty:ty)*
      $(; optional $($opt:ident : $oty:ty),+)? $(; rest $rest:ident)? $(,)?), $body:block) => {
        #[allow(unused_variables, unused_mut)]
        pub(crate) fn $id($cx: &mut $crate::prim::Leaf<'_> $(, $arg: $ty)*
            $($(, $opt: $oty)+)? $(, $rest: &[Value])?) -> $crate::prim::$out $body
        #[allow(non_snake_case)]
        pub(crate) mod $id {
            use super::*;
            pub(crate) const DESC: $crate::prim::PrimDesc = $crate::prim::PrimDesc {
                name: $name,
                min: [stringify!($cx), $(stringify!($arg)),*].len() - 1,
                max: $crate::prim::defsubr!(@max
                    [stringify!($cx), $(stringify!($arg),)* $($(stringify!($opt)),+)?].len() - 1
                    $(, $rest)?),
                effect: $crate::prim::Effect::$eff,
                gnu: $gnu,
                file: file!(),
                line: line!(),
                body: $crate::prim::Body::$tier(adapter),
            };
            #[allow(unused_mut, unused_variables, unused_assignments)]
            fn adapter(cx: &mut $crate::prim::Leaf<'_>, args: &[Value]) -> $crate::prim::$out {
                let mut i = 0;
                $( let $arg = <$ty as $crate::prim::Arg>::check(cx, args[i])?; i += 1; )*
                $($( let $opt = <$oty as $crate::prim::Arg>::check(
                    cx, args.get(i).copied().unwrap_or(Value::NIL))?; i += 1; )+)?
                $(let $rest = args.get(i..).unwrap_or(&[]);)?
                super::$id(cx $(, $arg)* $($(, $opt)+)? $(, $rest)?)
            }
        }
    };
}
pub(crate) use defsubr;

#[path = "prims/mod.rs"]
pub(crate) mod prims;

include!(concat!(env!("OUT_DIR"), "/generated_prims.rs"));

/// The single ordinal lookup boundary for legacy and generated subrs.
#[derive(Clone, Copy)]
pub(crate) enum Descriptor {
    Legacy(&'static str, Primitive),
    Generated(&'static PrimDesc),
}
impl Descriptor {
    pub(crate) fn name(self) -> &'static str {
        match self {
            Self::Legacy(name, _) => name,
            Self::Generated(d) => d.name,
        }
    }
    // Traversal kernels specialize a legacy descriptor at compile time. Keep
    // this projection inline across gate-profile codegen units so their next
    // handler remains a sibling call (traversal's bounded-stack regression).
    #[inline(always)]
    pub(crate) fn legacy(self) -> Option<Primitive> {
        match self {
            Self::Legacy(_, p) => Some(p),
            Self::Generated(_) => None,
        }
    }
}
#[inline(always)]
pub(crate) fn descriptor(index: usize) -> Option<Descriptor> {
    if index < PRIMITIVES.len() {
        let &(name, p) = PRIMITIVES.get(index)?;
        Some(Descriptor::Legacy(name, p))
    } else {
        GENERATED
            .get(index - PRIMITIVES.len())
            .map(Descriptor::Generated)
    }
}
pub(crate) fn legacy(v: Value) -> Option<Primitive> {
    descriptor(v.index_if(6)?)?.legacy()
}
pub(crate) fn name(v: Value) -> Option<&'static str> {
    Some(descriptor(v.index_if(6)?)?.name())
}
pub(crate) fn count() -> usize {
    PRIMITIVES.len() + GENERATED.len()
}
pub(crate) fn legacy_value(p: Primitive) -> Option<Value> {
    (0..PRIMITIVES.len())
        .find(|&i| descriptor(i).and_then(Descriptor::legacy) == Some(p))
        .map(|i| Value::reference(i, 6))
}

/// Tag-6 payloads at or above `PRIMITIVES.len()` name generated primitives.
#[inline(always)]
pub(crate) fn generated(v: Value) -> Option<&'static PrimDesc> {
    match descriptor(v.index_if(6)?)? {
        Descriptor::Generated(d) => Some(d),
        _ => None,
    }
}

/// Run `run` with the VM and assert, in debug builds, that it did not collect.
/// Every leaf body and T2 continuation goes through here: Values held in Rust
/// locals during the call are valid only because nothing in it can collect.
fn no_collection<R>(vm: &mut Vm, run: impl FnOnce(&mut Vm) -> R) -> R {
    #[cfg(debug_assertions)]
    let before = vm.gc_stats.collections;
    let out = run(vm);
    #[cfg(debug_assertions)]
    assert_eq!(
        vm.gc_stats.collections, before,
        "a primitive leaf reached a collection point"
    );
    out
}

impl Vm {
    /// Legacy fset shares the checked Leaf cell writer without exposing Vm to leaves.
    pub(crate) fn set_function_leaf(
        &mut self,
        symbol: Value,
        definition: Value,
    ) -> std::result::Result<Value, Fault> {
        no_collection(self, |vm| {
            Leaf::new(vm).set_symbol_function(symbol, definition)
        })
    }
    pub(crate) fn register_generated(&mut self) {
        for (i, d) in GENERATED.iter().enumerate() {
            let slot = self.slot(d.name);
            self.slots[slot].function = Value::reference(PRIMITIVES.len() + i, 6);
        }
    }
    /// Enter one generated primitive. Arity and every fault become GNU
    /// condition+data. A leaf's value comes back as `Step::Done`.
    pub(crate) fn call_generated(
        &mut self,
        target: Value,
        start: usize,
        argc: usize,
    ) -> std::result::Result<Step, (Value, Value)> {
        let d = generated(target).expect("generated primitive");
        if argc < d.min || d.max.is_some_and(|m| argc > m) {
            let n = Value::integer(argc as i64).unwrap();
            return Err(self.condition("wrong-number-of-arguments", &[target, n]));
        }
        let args: Vec<Value> = self.stack[start..start + argc].to_vec();
        let run = |vm: &mut Vm| {
            let mut cx = Leaf::new(vm);
            match d.body {
                Body::Leaf(f) => f(&mut cx, &args).map(Step::Done),
                Body::Call(f) => f(&mut cx, &args),
            }
        };
        // The reader owns explicit roots and the engine publishes its suspended
        // values before entering it. Unlike ordinary leaves it can call Lisp.
        let step = if matches!(d.name, "read" | "read-from-string") {
            run(self)
        } else {
            no_collection(self, run)
        };
        step.map_err(|fault| self.fault(fault))
    }
    /// Run a T2 continuation with the callback's value and the saved registers.
    pub(crate) fn resume_generated(
        &mut self,
        then: Then,
        result: Value,
        saved: &[Value],
    ) -> std::result::Result<Step, (Value, Value)> {
        no_collection(self, |vm| then(&mut Leaf::new(vm), result, saved))
            .map_err(|fault| self.fault(fault))
    }
    pub(super) fn fault(&mut self, fault: Fault) -> (Value, Value) {
        match fault {
            Fault::Wrong(pred, v) => {
                let p = self.intern(pred);
                self.condition("wrong-type-argument", &[p, v])
            }
            Fault::NotUnicode(v) => self.condition("not-unicode-string", &[v]),
            Fault::Error(message) => {
                let text = self.make_lisp_string(message.into_bytes(), true);
                self.condition("error", &[text])
            }
            Fault::EndOfFile => self.condition("end-of-file", &[]),
            Fault::Signal(condition, data) => (condition, data),
            Fault::ReadSyntax(message) => {
                let text = self.make_lisp_string(message.into_bytes(), true);
                self.condition("invalid-read-syntax", &[text])
            }
            Fault::Circular(v) => self.condition("circular-list", &[v]),
            Fault::Range1(a) => self.condition("args-out-of-range", &[a]),
            Fault::Range(a, b) => self.condition("args-out-of-range", &[a, b]),
            Fault::Range3(a, b, c) => self.condition("args-out-of-range", &[a, b, c]),
            Fault::VoidFunction(f) => self.condition("void-function", &[f]),
            Fault::VoidVariable(symbol) => self.condition("void-variable", &[symbol]),
            Fault::CyclicFunctionIndirection(symbol) => {
                self.condition("cyclic-function-indirection", &[symbol])
            }
            Fault::CyclicVariableIndirection(symbol) => {
                self.condition("cyclic-variable-indirection", &[symbol])
            }
            Fault::WrongNumberOfArguments(f, count) => {
                self.condition("wrong-number-of-arguments", &[f, count])
            }
            Fault::SettingConstant(symbol) => self.condition("setting-constant", &[symbol]),
        }
    }
    fn condition(&mut self, name: &str, data: &[Value]) -> (Value, Value) {
        let c = self.intern(name);
        (c, self.list_value(data))
    }
    /// Evaluate the built-in T3 Lisp prelude (`prims/prelude.el`): callback
    /// loops written in Lisp over leaf cursors, so their state is registers.
    pub(crate) fn install_lisp_prelude_raw(&mut self) -> Result<Value> {
        self.eval_source_raw(prims::PRELUDE)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn typed_faults_preserve_gnu_condition_and_data() {
        let mut vm = Vm::new();
        let symbol = vm.intern("fault-test-symbol");
        let other = vm.intern("fault-test-other");
        let count = Value::integer(3).unwrap();
        // GNU xsignal1/2/3 construct list1/2/3 (eval.c:2084-2098).
        // Distinct, nontrivial payloads detect omission, reordering or wrapping.
        let function = vm.list_value(&[symbol, other]);
        let cases = [
            (
                Fault::Wrong("integerp", symbol),
                "wrong-type-argument",
                vec![vm.intern("integerp"), symbol],
            ),
            (Fault::Circular(function), "circular-list", vec![function]),
            (
                Fault::Range(function, other),
                "args-out-of-range",
                vec![function, other],
            ),
            (
                Fault::Range3(function, other, count),
                "args-out-of-range",
                vec![function, other, count],
            ),
            (
                Fault::VoidFunction(function),
                "void-function",
                vec![function],
            ),
            (Fault::VoidVariable(symbol), "void-variable", vec![symbol]),
            (
                Fault::CyclicFunctionIndirection(symbol),
                "cyclic-function-indirection",
                vec![symbol],
            ),
            (
                Fault::WrongNumberOfArguments(function, count),
                "wrong-number-of-arguments",
                vec![function, count],
            ),
            (
                Fault::SettingConstant(Value::NIL),
                "setting-constant",
                vec![Value::NIL],
            ),
        ];
        // `(error MESSAGE)` carries a fresh string, so compare its text.
        let (condition, data) = vm.fault(Fault::Error("Marker does not point anywhere".into()));
        assert_eq!(condition, vm.intern("error"));
        let (message, tail) = vm.cons_pair(data).expect("proper condition data");
        assert_eq!(tail, Value::NIL, "extra condition data for error");
        assert_eq!(vm.source_string(message).unwrap(), "Marker does not point anywhere");
        for (fault, name, expected) in cases {
            let (condition, mut data) = vm.fault(fault);
            assert_eq!(condition, vm.intern(name), "condition for {name}");
            for value in expected {
                let (actual, tail) = vm.cons_pair(data).expect("proper condition data");
                assert_eq!(actual, value, "data element for {name}");
                data = tail;
            }
            assert_eq!(data, Value::NIL, "extra condition data for {name}");
        }
    }

    #[test]
    fn fault_surface_has_typed_gnu_conditions() {
        // Keep additions representable as typed faults, not string diagnostics.
        let source = include_str!("prim.rs");
        let faults = source
            .split("pub(crate) enum Fault {")
            .nth(1)
            .unwrap()
            .split("\n}")
            .next()
            .unwrap();
        for variant in [
            "VoidFunction",
            "VoidVariable",
            "CyclicFunctionIndirection",
            "Range3",
            "WrongNumberOfArguments",
            "SettingConstant",
        ] {
            assert!(
                faults.contains(&format!("{variant}(")),
                "missing typed Fault::{variant}"
            );
        }
    }

    defsubr! { name = "romacs--zero-arity-test", gnu = "data.c:688", effect = Pure,
    fn zero_arity(cx) { Ok(Value::NIL) }}

    #[test]
    fn zero_argument_declarations_count_only_lisp_parameters() {
        assert_eq!(zero_arity::DESC.min, 0);
        assert_eq!(zero_arity::DESC.max, Some(0));
        let mut vm = Vm::new();
        let Body::Leaf(body) = zero_arity::DESC.body else {
            panic!("expected leaf")
        };
        assert!(matches!(body(&mut Leaf::new(&mut vm), &[]), Ok(Value::NIL)));
    }

    defsubr! { name = "romacs--optional-leaf-test", gnu = "data.c:1", effect = Pure,
    fn optional_leaf(cx, required: Integer; optional first: Value, second: List) {
        let tail = cx.cons(first, second.0);
        Ok(cx.cons(Value::integer(required.0).unwrap(), tail))
    }}
    defsubr! { name = "romacs--many-leaf-test", gnu = "data.c:1", effect = Pure,
    fn many_leaf(cx; rest args) { Ok(Value::integer(args.len() as i64).unwrap()) }}
    defsubr! { name = "romacs--optional-call-test", gnu = "data.c:1", effect = Pure,
    fn optional_call(cx, required: Integer; optional extra: Value) -> Step {
        Ok(Step::Done(extra))
    }}
    defsubr! { name = "romacs--many-call-test", gnu = "data.c:1", effect = Pure,
    fn many_call(cx, required: Integer; rest args) -> Step {
        Ok(Step::Done(Value::integer(args.len() as i64).unwrap()))
    }}
    defsubr! { name = "romacs--mixed-leaf-test", gnu = "data.c:1", effect = Pure,
    fn mixed_leaf(cx; optional extra: Value; rest args) {
        Ok(cx.cons(extra, Value::integer(args.len() as i64).unwrap()))
    }}
    defsubr! { name = "romacs--mixed-call-test", gnu = "data.c:1", effect = Pure,
    fn mixed_call(cx; optional extra: Value; rest args) -> Step {
        Ok(Step::Done(cx.cons(extra, Value::integer(args.len() as i64).unwrap())))
    }}

    fn arity_result(
        vm: &mut Vm,
        d: &PrimDesc,
        args: &[Value],
    ) -> std::result::Result<Value, Fault> {
        let mut cx = Leaf::new(vm);
        match d.body {
            Body::Leaf(f) => f(&mut cx, args),
            Body::Call(f) => match f(&mut cx, args)? {
                Step::Done(v) => Ok(v),
                Step::Call { .. } | Step::Collect => panic!("expected immediate test result"),
            },
        }
    }

    #[test]
    fn widened_arity_optional_defaults_and_checks_follow_parameter_order() {
        let mut vm = Vm::new();
        let one = Value::integer(1).unwrap();
        assert_eq!(
            (optional_leaf::DESC.min, optional_leaf::DESC.max),
            (1, Some(3))
        );
        for (args, expected) in [
            (vec![one], "(1 nil)"),
            (vec![one, one], "(1 1)"),
            (vec![one, one, Value::NIL], "(1 1)"),
        ] {
            let result = arity_result(&mut vm, &optional_leaf::DESC, &args);
            let Ok(value) = result else {
                panic!("optional leaf failed")
            };
            assert_eq!(vm.display(value).unwrap(), expected);
        }
        assert!(matches!(
            arity_result(&mut vm, &optional_leaf::DESC, &[Value::NIL, one, one]),
            Err(Fault::Wrong("integerp", Value::NIL))
        ));
        assert!(
            matches!(arity_result(&mut vm, &optional_leaf::DESC, &[one, one, one]),
            Err(Fault::Wrong("listp", v)) if v == one)
        );
        assert_eq!(
            (optional_call::DESC.min, optional_call::DESC.max),
            (1, Some(2))
        );
        assert!(matches!(
            arity_result(&mut vm, &optional_call::DESC, &[one]),
            Ok(Value::NIL)
        ));
        assert!(
            matches!(arity_result(&mut vm, &optional_call::DESC, &[one, one]), Ok(v) if v == one)
        );
    }

    #[test]
    fn widened_arity_many_and_optional_many_work_for_both_tiers() {
        let mut vm = Vm::new();
        let one = Value::integer(1).unwrap();
        for (d, required) in [(&many_leaf::DESC, 0), (&many_call::DESC, 1)] {
            assert_eq!((d.min, d.max), (required, None));
            for count in [0, 1, 32] {
                let args = vec![one; required + count];
                assert!(
                    matches!(arity_result(&mut vm, d, &args), Ok(v) if v.integer_value() == Some(count as i64))
                );
            }
        }
        for d in [&mixed_leaf::DESC, &mixed_call::DESC] {
            assert_eq!((d.min, d.max), (0, None));
            for (args, expected) in [
                (vec![], "(nil . 0)"),
                (vec![one], "(1 . 0)"),
                (vec![one; 4], "(1 . 3)"),
            ] {
                let Ok(v) = arity_result(&mut vm, d, &args) else {
                    panic!("mixed arity failed")
                };
                assert_eq!(vm.display(v).unwrap(), expected);
            }
        }
    }

    // Exercise the build-time declaration parser with the same runtime harness.
    #[allow(dead_code)]
    mod arity_registry {
        include!(concat!(env!("CARGO_MANIFEST_DIR"), "/build.rs"));

        #[test]
        fn widened_arity_registry_accepts_optional_and_many() {
            for (signature, min, max) in [
                ("cx", 0, Some(0)),
                ("cx, required: Value; optional extra: Value", 1, Some(2)),
                ("cx, required: Value; rest args", 1, None),
                (
                    "cx, required: Value; optional extra: Value; rest args",
                    1,
                    None,
                ),
                ("cx; optional first: List, second: Value,", 0, Some(2)),
                ("cx; rest args,", 0, None),
            ] {
                let source = format!(
                    "defsubr! {{ name = \"arity-probe\", gnu = \"data.c:1\", effect = Pure, fn probe({signature}) {{ Ok(Value::NIL) }} }}"
                );
                let path = std::path::Path::new("arity-probe.rs");
                let ts = tokens(path, &source);
                let parsed = std::panic::catch_unwind(|| {
                    declaration(path, &ts, 0, "probe", "arity-probe.rs")
                });
                assert!(
                    parsed.is_ok(),
                    "widened arity declaration must be accepted: {signature}"
                );
                let row = parsed.unwrap();
                assert_eq!(
                    (row.arity, row.max),
                    (min, max),
                    "derived arity for {signature}"
                );
            }
        }

        #[test]
        fn widened_arity_registry_rejects_malformed_suffixes() {
            for signature in [
                "cx; optional",
                "cx; optional a: Value; optional b: Value",
                "cx; rest args; optional a: Value",
                "cx, a: Value; rest a",
            ] {
                let source = format!(
                    "defsubr! {{ name = \"arity-probe\", gnu = \"data.c:1\", effect = Pure, fn probe({signature}) {{ Ok(Value::NIL) }} }}"
                );
                let path = std::path::Path::new("arity-probe.rs");
                let ts = tokens(path, &source);
                assert!(
                    std::panic::catch_unwind(|| declaration(
                        path,
                        &ts,
                        0,
                        "probe",
                        "arity-probe.rs"
                    ))
                    .is_err(),
                    "malformed signature accepted: {signature}"
                );
            }
        }
    }

    #[test]
    #[cfg(debug_assertions)]
    #[should_panic(expected = "a primitive leaf reached a collection point")]
    fn leaf_guard_rejects_a_collection() {
        let mut vm = Vm::new();
        no_collection(&mut vm, |vm| vm.collect(&mut []));
    }

    #[test]
    fn installed_prelude_is_the_committed_file() {
        let mut vm = Vm::new();
        vm.install_lisp_prelude_raw().unwrap();
        let v = vm
            .eval_source_raw("(mapcar (lambda (x) (* x x)) '(1 2 3))")
            .unwrap();
        assert_eq!(vm.display(v).unwrap(), "(1 4 9)");
        assert!(prims::PRELUDE.contains("(defun mapcar (function sequence)"));
    }

    #[test]
    fn leaf_guard_passes_allocation() {
        let mut vm = Vm::new();
        let v = no_collection(&mut vm, |vm| Leaf::new(vm).cons(Value::NIL, Value::NIL));
        assert_eq!(vm.display(v).unwrap(), "(nil)");
    }
}
