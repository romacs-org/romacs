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
    /// A variable watcher's signal, already raised and searched, waiting in
    /// `propagated_escape': the engine resumes it rather than signalling
    /// again, so handlers run once.
    Escape,
}
pub(crate) type Out = std::result::Result<Value, Fault>;

/// The variable slot holding the closure -> interactive-spec table.
pub(crate) const INTERACTIVE_TABLE: &str = "#:vm2-interactive-specs";
/// The echo area's current message (xdisp.c echo_area_buffer contents).
pub(crate) const ECHO_AREA_MESSAGE: &str = "#:vm2-echo-area-message";
/// Non-nil once an embedder renders the echo area (`Vm::set_echo_area_host`).
pub(crate) const ECHO_AREA_HOST: &str = "#:vm2-echo-area-host";
/// The echo-area message printing last made, which later prints extend.
pub(crate) const ECHO_AREA_PRINTED: &str = "#:vm2-echo-area-printed";
/// Lazy native root, traced and forwarded like INTERACTIVE_TABLE.
pub(crate) const FONT_ALTERNATIVE_FAMILIES: &str = "#:vm2-font-alternative-families";
/// xfaces.c Vface_alternative_font_registry_alist: staticpro'd, not a Lisp variable.
pub(crate) const FONT_ALTERNATIVE_REGISTRIES: &str = "#:vm2-font-alternative-registries";

/// A T2 continuation. A plain `fn` pointer: it cannot capture Rust state, so
/// the only Values it sees are the callback's result and the saved registers.
pub(crate) type Then = fn(&mut Leaf<'_>, Value, &[Value]) -> Next;
/// What a T2 body (or continuation) asks the engine to do next.
pub(crate) enum Step {
    /// Request collection at the engine's traced boundary, never inside a leaf.
    Collect,
    /// The command loop's idle collection there (`Vm::collect_garbage_until_key'):
    /// t when it collected, nil when held or abandoned for a key press.
    CollectIdle,
    /// An ordinary poll of the engine at the next instruction, nil returned:
    /// a collection due, a job's memory, a collection's aftermath through the
    /// gate, a quit (`threaded::poll_resume').
    Safepoint,
    /// Return this value from the primitive.
    Done(Value),
    /// Establish non-unwinding signal handlers for the dynamic extent of BODY.
    Handle { body: Value, handlers: Vec<(Value, Value)> },
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
    Leaf(LeafBody),
    Call(fn(&mut Leaf<'_>, &[Value]) -> Next),
    /// A leaf that runs Lisp while it reads: its caller publishes the
    /// engine's roots first (`generated_step').
    Reader(LeafBody),
}
pub(crate) type LeafBody = fn(&mut Leaf<'_>, &[Value]) -> Out;

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
pub(crate) use leaf::graphics::ImageEnv;
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
        WindowConfiguration,
        Sqlite,
        Process,
        Finalizer,
        Thread,
        TreesitParser,
        TreesitNode,
        TreesitQuery,
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
        /// The automatic collection's trigger (`Vm::collection_budget').
        pub(crate) fn collection_budget(&self) -> (f64, f64) {
            self.vm.collection_budget()
        }
        /// `memory-use-counts': (CONSES FLOATS VECTOR-CELLS SYMBOLS STRING-CHARS INTERVALS STRINGS).
        pub(crate) fn consed(&self) -> [u64; 7] {
            let c = &self.vm.allocation.consed;
            let symbols = (self.vm.slots.len() - self.vm.local_binding_slots.len()) as u64 + c.reused_symbols;
            [c.conses, c.floats, c.vector_cells, symbols, c.string_chars, 0, c.strings]
        }
        /// `Vm::collection_share'.
        pub(crate) fn collection_share(&self) -> f64 {
            self.vm.collection_share()
        }
        pub(crate) fn set_font_alternative_families(&mut self, raw: Value, names: Vec<Vec<String>>) {
            let slot = self.vm.slot(super::FONT_ALTERNATIVE_FAMILIES);
            self.vm.slots[slot].value = raw;
            self.vm.display_state.set_font_alternative_families(names);
        }
        pub(crate) fn set_font_alternative_registries(&mut self, raw: Value) {
            let slot = self.vm.slot(super::FONT_ALTERNATIVE_REGISTRIES);
            self.vm.slots[slot].value = raw;
            self.vm.display_state.free_realized_faces();
        }
        /// xdisp.c safe_calln (see `Vm::safe_call').
        pub(crate) fn safe_call(&mut self, function: Value, args: &[Value]) -> Value {
            self.vm.safe_call(function, args)
        }
        /// What a function `:height' of the face NAME came to, None for none.
        pub(crate) fn note_face_height(&mut self, name: String, height: Option<i64>) {
            match height {
                Some(height) => { self.vm.display_state.face_heights.insert(name, height); }
                None => { self.vm.display_state.face_heights.remove(&name); }
            }
        }
        /// The bytes of a DOC file, read once per path and modification time.
        pub(crate) fn doc_file_bytes(&mut self, path: &std::path::Path) -> Option<std::sync::Arc<Vec<u8>>> {
            let modified = crate::host::metadata(path).and_then(|m| m.modified()).ok()?;
            if let Some((cached, at, bytes)) = &self.vm.doc_file && cached == path && *at == modified {
                return Some(bytes.clone());
            }
            let bytes = std::sync::Arc::new(crate::host::read_file(path).ok()?);
            self.vm.doc_file = Some((path.to_path_buf(), modified, bytes.clone()));
            Some(bytes)
        }
        pub(crate) fn mktime_offset(&self) -> i64 { self.vm.mktime_offset }
        pub(crate) fn idle_since(&self) -> Option<std::time::Instant> { self.vm.idle_since }
        pub(crate) fn redisplay_since_mark(&self) -> Option<Option<u64>> { self.vm.redisplay_since_mark }
        pub(crate) fn set_redisplay_since_mark(&mut self, mark: Option<Option<u64>>) { self.vm.redisplay_since_mark = mark; }
        pub(crate) fn set_idle_since(&mut self, since: Option<std::time::Instant>) {
            self.vm.idle_since = since;
            crate::compiled_lisp::note_idle(since);
        }
        pub(crate) fn set_mktime_offset(&mut self, offset: i64) { self.vm.mktime_offset = offset; }
        pub(crate) fn tz_rule_dstoff(&self) -> i64 { self.vm.tz_rule_dstoff }
        pub(crate) fn set_tz_rule_dstoff(&mut self, offset: i64) { self.vm.tz_rule_dstoff = offset; }
        pub(crate) fn local_zone(&self) -> crate::LocalZone { self.vm.local_zone.clone() }
        pub(crate) fn recent_keys(&mut self, include_commands: bool) -> Value { self.vm.recent_keys_value(include_commands) }
        pub(crate) fn record_recent_key(&mut self, value: Value, command: bool) { self.vm.record_recent_value(value, command) }
        pub(crate) fn clear_recent_keys(&mut self) { self.vm.recent_keys.clear() }
        pub(crate) fn lossage_limit(&self) -> usize { self.vm.lossage_limit() }
        pub(crate) fn set_lossage_limit(&mut self, limit: usize) { self.vm.set_lossage_limit(limit) }
        pub(crate) fn set_local_zone(&mut self, zone: crate::LocalZone) { self.vm.local_zone = zone; }
        pub(crate) fn symbol_slot_of(&self, v: Value) -> std::result::Result<usize, Fault> {
            self.symbol_slot(v)
        }
        /// notify_variable_watchers from a leaf; a watcher's signal is the fault.
        pub(crate) fn notify(&mut self, slot: usize, value: Value, operation: &str, place: Option<Value>) -> std::result::Result<(), Fault> {
            if self.vm.trapped.is_empty() { return Ok(()); }
            self.vm.notify_watchers_at(slot, value, operation, place).map_err(|text| self.watcher_fault(text))
        }
        /// A write's failure as a fault: a watcher's signal stays pending as
        /// itself. A leaf cannot carry a `throw' out, so one is refused by name.
        fn watcher_fault(&mut self, text: String) -> Fault {
            match self.vm.propagated_escape {
                Some(crate::threaded::Escape::Signal(..) | crate::threaded::Escape::Error { .. }) => Fault::Escape,
                Some(crate::threaded::Escape::Throw(..)) => {
                    self.vm.propagated_escape = None;
                    Fault::Error("VM2 throw out of a variable watcher of a native write is unsupported".into())
                }
                _ => Fault::Error(text),
            }
        }
        /// A VM-owned Lisp value kept in the internal symbol NAME's value
        /// cell ("#:vm2-..."), rooted like every symbol value; None unset.
        pub(crate) fn internal_value(&mut self, name: &str) -> Option<Value> {
            let slot = self.vm.slot(name);
            Some(self.vm.slots[slot].value).filter(|&v| v != Value::UNBOUND)
        }
        pub(crate) fn set_internal_value(&mut self, name: &str, value: Value) {
            let slot = self.vm.slot(name);
            self.vm.slots[slot].value = value;
        }
        /// Fget: SYMBOL's KEY property, read as `symbol-plist' reads it.
        pub(crate) fn symbol_property(&mut self, symbol: Value, key: Value) -> std::result::Result<Value, Fault> {
            let plist = self.observed_symbol_plist(symbol)?;
            Ok(self.vm.plist_lookup(plist, key))
        }
        pub(crate) fn put_symbol_property(&mut self, symbol: Value, key: Value, value: Value) -> std::result::Result<(), Fault> {
            self.vm.put_symbol_property(symbol, key, value).map(|_| ()).map_err(Fault::Error)
        }
        /// Findirect_variable: the end of SYMBOL's variable alias chain.
        pub(crate) fn indirect_variable_symbol(&self, symbol: Value) -> std::result::Result<Value, Fault> {
            let slot = self.symbol_slot(symbol)?;
            Ok(Value::reference(self.vm.indirect_variable(slot) + 1, 7))
        }
        pub(crate) fn trapped_variable(&self, symbol: Value) -> std::result::Result<bool, Fault> {
            let slot = self.symbol_slot(symbol)?;
            Ok(self.vm.variable_trapped(self.vm.indirect_variable(slot)))
        }
        /// xfaces.c clear_face_cache with face_change: realize every face
        /// afresh and republish every window.
        pub(crate) fn clear_face_cache(&mut self) {
            self.vm.display_state.faces_changed();
            self.vm.display_state.free_realized_faces();
        }
        pub(crate) fn request_redisplay(&mut self) {
            self.vm.display_state.free_realized_faces();
        }
        pub(crate) fn trap_variable(&mut self, symbol: Value, trapped: bool) -> std::result::Result<(), Fault> {
            let slot = self.symbol_slot(symbol)?;
            let base = self.vm.indirect_variable(slot);
            self.vm.set_variable_trapped(base, trapped);
            Ok(())
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
            // CHECK_INTEGER: a bignum counts past either end of the buffer.
            let count = if count == Value::NIL { 1 } else if let Some(n) = count.integer_value() { n } else {
                match self.bigint_value(count) {
                    Some(n) if n.sign() == num_bigint::Sign::Minus => Value::MIN_FIXNUM,
                    Some(_) => Value::MAX_FIXNUM,
                    None => return Err(Fault::Wrong("integerp", count)),
                }
            };
            let index = self.vm.buffer_index().map_err(Fault::Error)?;
            let error = |e: String| Fault::Error(e);
            // Byte scans: a newline byte is never part of another character.
            let (text, point, low, high): (&dyn crate::text::Newlines, usize, usize, usize) = match &self.vm.objects[index] {
                data::Object::Buffer(b) => { let range = b.accessible(); (b.body(), b.point.0, range.start.0, range.end.0) }
                data::Object::ByteBuffer(b) => (&b.bytes, b.point.0, b.restriction.start.0, b.restriction.end.0),
                _ => unreachable!(),
            };
            // editfns.c eol: find_before_next_newline (PT, 0, N - (N <= 0)):
            // the COUNTth newline from point, or the accessible end when
            // there are fewer; bol: the start of the line COUNT - 1 lines on.
            let byte = if end { crate::prim::line_end_after(text, low, high, point, count - i64::from(count <= 0)) }
                else { crate::prim::line_beginning(text, low, high, point, count.saturating_sub(1)) };
            let position = match &self.vm.objects[index] {
                data::Object::Buffer(b) => b.char_of_byte(crate::coordinates::ByteOffset(byte)).map_err(error)?.0,
                _ => byte,
            };
            Ok(Value::integer(position as i64 + 1).unwrap())
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
        /// key-weak `eq` table held by an ordinary traced variable slot. The
        /// entry holds the slot's value as GNU makes it: SPEC, or
        /// `[SPEC MODES]' for a command with modes.
        pub(crate) fn interactive_spec(&mut self, closure: Value) -> Option<(Value, Value)> {
            let (form, thunk) = self.interactive_entry(closure)?;
            Some((self.vector_get(form, Value::integer(0).unwrap()).unwrap_or(form), thunk))
        }
        /// The MODES of a closure's `(interactive SPEC . MODES)', or nil.
        pub(crate) fn interactive_modes(&mut self, closure: Value) -> Value {
            let Some((form, _)) = self.interactive_entry(closure) else { return Value::NIL };
            self.vector_get(form, Value::integer(1).unwrap()).unwrap_or(Value::NIL)
        }
        fn interactive_entry(&mut self, closure: Value) -> Option<(Value, Value)> {
            let slot = *self.vm.names.get(super::INTERACTIVE_TABLE)?;
            let table = self.vm.slots[slot].value;
            if table == Value::UNBOUND {
                return None;
            }
            let entry = self.vm.declaration_hash_get(table, closure).ok()??;
            self.pair(entry)
        }
        pub(crate) fn closure_document(&self, closure: Value) -> Value {
            closure.index_if(4).and_then(|i| self.vm.closures.get(i))
                .map(|c| c.metadata.get(4).copied().unwrap_or(c.document)).unwrap_or(Value::NIL)
        }
        /// A closure's (REQUIRED, POSITIONAL, REST) parameter counts.
        pub(crate) fn closure_arity(&self, closure: Value) -> Option<(usize, usize, bool)> {
            let c = self.vm.closures.get(closure.index_if(4)?)?;
            let f = self.vm.functions.get(c.function)?;
            Some((f.required, f.arity, f.rest))
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
            let name = self.vm.buffer_name_text(arg).ok_or(Fault::Wrong("stringp", arg))?;
            if let Some(buffer) = self.vm.named_buffer(&name) { return Ok((buffer, false)); }
            if !create { return Err(Fault::Error(format!("No buffer named {name}"))); }
            let buffer = self.vm.create_named_buffer(&name, false).map_err(|error| {
                match self.vm.propagated_escape.take() {
                    Some(crate::threaded::Escape::Signal(condition, data)) => Fault::Signal(condition, data),
                    Some(crate::threaded::Escape::Error { condition, object, .. }) => {
                        let (_, data) = self.vm.cons_pair(object).expect("normalized signal object");
                        Fault::Signal(condition, data)
                    }
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
        /// `save-excursion` begins: its marker, recorded where it is made
        /// for `backtrace-eval`'s unrewind (`Backtrace::excursions`).
        pub(crate) fn save_excursion_data(&mut self) -> std::result::Result<Value, Fault> {
            let marker = self.vm.save_excursion_data().map_err(Fault::Error)?;
            // The excursion is its caller's: not this primitive's own
            // activation, when the call recorded one.
            let frames = &self.vm.backtrace.frames;
            let own = frames.last().is_some_and(|frame| crate::prim::name(frame.function) == Some("romacs--save-excursion"));
            let frames = frames.len() - usize::from(own);
            let dynamic = self.vm.dynamic.len();
            // Only `backtrace-eval` reads it: a frame without windows records none.
            let window = self.vm.excursion_window().unwrap_or(Value::NIL);
            self.vm.backtrace.excursion_begin(marker, window, frames, dynamic);
            Ok(marker)
        }
        pub(crate) fn restore_excursion_data(&mut self, data: Value) -> std::result::Result<Value, Fault> {
            self.vm.backtrace.excursion_end(data);
            self.vm.restore_excursion_data(data).map_err(Fault::Error)
        }
        pub(crate) fn save_restriction_data(&mut self) -> std::result::Result<Value, Fault> {
            self.vm.save_restriction_data().map_err(Fault::Error)
        }
        pub(crate) fn restore_restriction_data(&mut self, data: Value) -> std::result::Result<(), Fault> {
            self.vm.restore_restriction_data(data).map_err(Fault::Error)
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
            self.vm.alloc_cons(car, cdr)
        }
        /// ENCODE_FILE: the host bytes of the file name VALUE.
        pub fn file_name_bytes(&self, value: Value) -> std::result::Result<Vec<u8>, Fault> {
            self.vm.file_name_bytes(value).ok_or(Fault::Wrong("stringp", value))
        }
        /// The model's name for the buffer name string VALUE.
        pub fn buffer_name_text(&self, value: Value) -> std::result::Result<String, Fault> {
            self.vm.buffer_name_text(value).ok_or(Fault::Wrong("stringp", value))
        }
        /// The Lisp string of the model's buffer name NAME.
        pub fn buffer_name_value(&mut self, name: &str) -> Value {
            self.vm.buffer_name_value(name)
        }
        pub fn buffer_name_exists(&self, name: &str) -> bool {
            (name == "*scratch*" && self.vm.scratch_unmade()) || self.vm.names.get(&format!("#:vm2-buffer:{name}"))
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
        /// keymap.c's flush of where_is_cache (`Vm::note_keymap_change').
        pub(crate) fn note_keymap_change(&mut self) {
            self.vm.note_keymap_change();
        }
        pub(crate) fn keymap_generation(&self) -> u64 {
            self.vm.keymap_generation()
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
        /// GNU string-lessp's order, compared in place: a symbol orders by
        /// its name, a string by its characters.
        pub(crate) fn string_order(&self, a: Value, b: Value) -> std::result::Result<std::cmp::Ordering, Fault> {
            self.with_name_chars(a, |a| self.with_name_chars(b, |b| a.cmp(b)))?
        }
        fn with_name_chars<R>(&self, v: Value, f: impl FnOnce(&mut dyn Iterator<Item = u32>) -> R)
            -> std::result::Result<R, Fault> {
            let name = if v == Value::NIL { Some("nil") } else if v == Value::TRUE { Some("t") }
                else { self.symbol_slot(v).ok().map(|slot| self.vm.symbol_names[slot].as_str()) };
            if let Some(name) = name { return Ok(f(&mut name.chars().map(u32::from))); }
            let view = self.vm.str_view(v).ok_or(Fault::Wrong("stringp", v))?;
            Ok(f(&mut view.chars()))
        }
        /// Whether the current buffer's text is multibyte (a unibyte buffer
        /// is a byte buffer), as `enable-multibyte-characters' says.
        pub(crate) fn buffer_multibyte(&mut self) -> std::result::Result<bool, Fault> {
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
        /// syntax_prefix_flag_p of each of CODES in the current buffer's
        /// syntax table, as casing a buffer region reads it.
        pub(crate) fn casing_prefix_flags(&mut self, codes: &[u32]) -> std::result::Result<Vec<bool>, Fault> {
            let i = self.vm.buffer_index().map_err(Fault::Error)?;
            let table = match &self.vm.objects[i] {
                data::Object::Buffer(b) => b.syntax_table,
                data::Object::ByteBuffer(b) => b.syntax_table,
                _ => unreachable!("buffer_index validates"),
            };
            let standard = self.vm.names.get("#:vm2-standard-syntax").map(|&s| self.vm.slots[s].value);
            let code_of = |table: Value, code: u32| self.vm.char_table_get(table, code as usize).ok()
                .and_then(|entry| self.vm.cons_pair(entry).ok()).and_then(|(n, _)| n.integer_value());
            Ok(codes.iter().map(|&code| {
                let entry = code_of(table, code).or_else(|| standard.and_then(|t| code_of(t, code)));
                entry.is_some_and(|n| (n >> 20) & 1 != 0)
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
                    if multibyte && !s.is_multibyte() {
                        // A unibyte part widened: its runs stretch with it.
                        crate::buffer::widened_properties(&s.properties, s.bytes(), offset, &mut properties);
                    } else {
                        properties.copy_into(crate::coordinates::ByteOffset(offset), &s.properties);
                    }
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
            if Vm::image_value(value) { return Err(self.read_only(value)); }
            let bytes=self.string_view(value)?.bytes.len();
            *self.object_mut(value)? = data::Object::Bytes(vec![0;bytes]);
            Ok(Value::NIL)
        }
        pub fn sxhash(&self, value: Value) -> Value {
            Value::integer((self.vm.public_sxhash(value) & ((1u64 << 60)-1)) as i64).unwrap()
        }
        pub fn string_to_syntax(&mut self, string: Value) -> super::Out {
            let description = self.string_view(string)?;
            Ok(match crate::regexp::parse_syntax_descriptor(&description).map_err(Fault::Error)? {
                None => Value::NIL,
                Some((code, matching)) => self.vm.regexp_syntax_entry(code,
                    matching.map_or(Value::NIL, |c| Value::integer(i64::from(c)).unwrap())),
            })
        }
        pub fn sxhash_eq(&mut self, value: Value) -> super::Out {
            const MASK: u64 = (1 << 60) - 1;
            if !matches!(value.tag(), 0 | 3 | 4 | 5) || value == Value::NIL {
                return Ok(Value::integer((value.0 & MASK) as i64).unwrap());
            }
            let slot = self.vm.slot("#:vm2-sxhash-eq");
            if self.vm.slots[slot].value == Value::UNBOUND {
                let table = hash::HashTable::new_local_bindings();
                self.vm.slots[slot].value = self.vm.allocate_object(data::Object::HashTable(Box::new(table)));
            }
            let table = self.vm.slots[slot].value;
            if let Some(hash) = self.vm.declaration_hash_get(table, value).map_err(Fault::Error)? {
                return Ok(hash);
            }
            let counter = self.vm.slot("#:vm2-next-eq-hash");
            let hash = self.vm.slots[counter].value.integer_value().unwrap_or(0);
            let value_hash = Value::integer(hash).map_err(Fault::Error)?;
            self.vm.slots[counter].value = Value::integer(((hash as u64 + 1) & MASK) as i64).unwrap();
            self.vm.declaration_hash_put(table, value, value_hash).map_err(Fault::Error)?;
            Ok(value_hash)
        }
        pub fn fill_string(&mut self, value: Value, code: u32) -> std::result::Result<Value, Fault> {
            if Vm::image_value(value) { return Err(self.read_only(value)); }
            let view = self.string_view(value)?;
            let mut bytes = Vec::new();
            if view.multibyte {
                let mut encoded=[0;5]; let width=crate::lstring::char_string(code,&mut encoded);
                if width * view.chars != view.bytes.len() { return Err(Fault::Error("Attempt to change byte length of a string".into())); }
                for _ in 0..view.chars { bytes.extend_from_slice(&encoded[..width]); }
            } else { bytes.resize(view.chars, code as u8); }
            let old = self.object_mut(value)?;
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
        /// A string of TEXT whose `face` is each run's value over its byte
        /// range (UTF-8 offsets), as `propertize' would give each piece.
        pub fn faced_text(&mut self, text: String, runs: &[(std::ops::Range<usize>, Value)]) -> Value {
            let multibyte = !text.is_ascii();
            let mut string = data::LispString::from_bytes(text.into_bytes(), multibyte);
            for (range, face) in runs {
                string.properties.put(coordinates::ByteOffset(range.start)..coordinates::ByteOffset(range.end), "face", *face);
            }
            self.vm.allocate_object(if string.is_utf8() { data::Object::String(Box::new(string)) } else { data::Object::RawString(Box::new(string)) })
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
            let data::Object::Vector(a) = self.object_mut(v)? else {
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
            // Image and frozen conses are readable, never writable.
            if Vm::image_value(v) && v.tag() == 3 {
                return Err(self.read_only(v));
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
        /// The registers of match group N, if it matched.
        pub(crate) fn match_group(&self, n: usize) -> Option<(usize, usize)> {
            self.vm.regexp.groups.get(n).copied().flatten()
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
        /// `Vm::quick_cmp'.
        pub fn quick_compare(&self, a: Value, b: Value) -> Option<Option<std::cmp::Ordering>> {
            self.vm.quick_cmp(a, b)
        }
        /// Reuses GNU-compatible cycle/depth handling, preserving its typed errors.
        pub fn equal(&self, a: Value, b: Value) -> std::result::Result<bool, hash::HashFault> {
            self.vm.equal(a, b)
        }
        pub fn equal_including_properties(&self, a: Value, b: Value) -> std::result::Result<bool, hash::HashFault> {
            self.vm.equal_including_properties(a, b)
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
            self.vm.fence_saved(slot, binding, crate::world::Access::SavedRead);
            let value = self.vm.dynamic[binding].1;
            if value == Value::UNBOUND { Err(Fault::VoidVariable(v)) } else { Ok(Some(value)) }
        }
        /// Update only the oldest saved default, leaving all active cells intact.
        pub fn saved_default_cell_set(&mut self, v: Value, value: Value) -> std::result::Result<bool, Fault> {
            if v == Value::NIL || v == Value::TRUE { return Ok(false); }
            let slot = self.symbol_slot(v)?;
            let Some(binding) = self.vm.default_toplevel_binding(slot) else { return Ok(false) };
            self.vm.fence_saved(slot, binding, crate::world::Access::SavedWrite);
            self.vm.check_boolean_default(slot, value).map_err(Fault::Error)?;
            self.vm.dynamic[binding].1 = value;
            Ok(true)
        }
        pub fn provide_feature(&mut self, feature: Value, subfeatures: Value) -> std::result::Result<(), Fault> {
            self.vm.provide_feature(feature, subfeatures)
        }
        /// The value saved by the oldest local let of V in BUFFER, if any.
        pub fn local_toplevel_cell(&mut self, v: Value, buffer: Value) -> std::result::Result<Option<Value>, Fault> {
            if v == Value::NIL || v == Value::TRUE { return Ok(None); }
            let slot = self.symbol_slot(v)?;
            let Some(binding) = self.vm.local_toplevel_binding(slot, buffer) else { return Ok(None) };
            self.vm.fence_saved(slot, binding, crate::world::Access::SavedRead);
            let value = self.vm.dynamic[binding].1;
            if value == Value::UNBOUND { Err(Fault::VoidVariable(v)) } else { Ok(Some(value)) }
        }
        /// Replace that saved value; false when there is no such let.
        pub fn local_toplevel_cell_set(&mut self, v: Value, buffer: Value, value: Value) -> std::result::Result<bool, Fault> {
            if v == Value::NIL || v == Value::TRUE { return Ok(false); }
            let slot = self.symbol_slot(v)?;
            let Some(binding) = self.vm.local_toplevel_binding(slot, buffer) else { return Ok(false) };
            self.vm.fence_saved(slot, binding, crate::world::Access::SavedWrite);
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
            let value = self.vm.default_value(base).map_err(Fault::Error)?;
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
            self.notify(slot, value, "set", Some(Value::NIL))?;
            // GNU set_default_internal, SYMBOL_FORWARDED arm (data.c:2085).
            if let Some(fwd) = self.vm.native_forwarding(slot) {
                // A global variable's set_internal reports again.
                if self.vm.forwarding(slot).is_some_and(|fwd| !matches!(fwd, crate::forwarded::Fwd::Buffer(_) | crate::forwarded::Fwd::Fence(_))) {
                    self.notify(slot, value, "set", Some(Value::NIL))?;
                }
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
            self.vm.set_variable_value(slot, value).map_err(|text| self.watcher_fault(text))
        }
        /// Publish NEW as an alias of BASE once `defvaralias` has validated
        /// it. nil and t are immediates in VM2, so an alias of either targets
        /// the named slot, holding its own constant value.
        pub fn make_variable_alias(&mut self, new: Value, base: Value) -> std::result::Result<(), Fault> {
            let new = self.symbol_slot(new)?;
            // eval.c:713: NEW's watchers see the alias before it is made.
            self.notify(new, base, "defvaralias", Some(Value::NIL))?;
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
                if !self.vm.slots[slot].declared_special {
                    self.vm.slots[slot].declared_special = true;
                    self.vm.special_changes += 1;
                }
                if self.vm.names.get(&name) == Some(&slot) {
                    self.vm.specials.insert(name);
                }
            }
            Ok(())
        }
        /// Clear the symbol's declaration without following a variable alias.
        pub fn make_variable_non_special(&mut self, symbol: Value) -> std::result::Result<(), Fault> {
            let name = self.symbol_name(symbol)?;
            self.vm.special_changes += 1;
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
        pub fn attach_load_history(&mut self, entry: Value) -> std::result::Result<(), Fault> {
            self.vm.attach_load_history(entry).map_err(Fault::Error)
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
            self.custom_list_metadata(symbol, load, "custom-loads")
        }
        pub fn custom_list_metadata(&mut self, symbol: Value, load: Value, property: &str) -> super::Out {
            self.symbol_slot(symbol)?;
            let put_fault = |fault| match fault {
                annotations::PlistFault::Symbol(v) => Fault::Wrong("symbolp", v),
                annotations::PlistFault::Plistp(v) => Fault::Wrong("plistp", v),
                annotations::PlistFault::Circular(v) => Fault::Circular(v),
            };
            let key = self.intern(property);
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
            let mut seen = HashSet::default();
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
            let bits = if size == Value::NIL { obarray::DEFAULT_BITS } else {
                let n = size.integer_value().filter(|&n| n >= 0).ok_or(Fault::Wrong("wholenump", size))? as u64;
                let bits = 64 - n.leading_zeros();
                if bits > 31 { return Err(Fault::Range1(size)); }
                bits
            };
            Ok(self.vm.allocate_object(data::Object::Obarray(Box::new(obarray::Obarray::new(bits, false)))))
        }
        pub fn resolve_obarray(&mut self, value: Value) -> super::Out { self.vm.resolve_obarray(value) }
        pub fn intern_in(&mut self, table: Value, name: Value) -> super::Out { self.vm.obarray_intern(table, name) }
        /// The Lisp name NAME looks a symbol up by: a string itself, or a
        /// symbol's `symbol-name' (decoded from its key, symkey.rs).
        fn lookup_name(&mut self, name: Value) -> std::result::Result<Value, Fault> {
            if self.kind(name) == Kind::Symbol { self.symbol_name_value(name) }
            else if self.vm.str_view(name).is_some() { Ok(name) }
            else { Err(Fault::Wrong("stringp", name)) }
        }
        pub fn lookup_in(&mut self, table: Value, name: Value) -> super::Out {
            let symbol = self.kind(name) == Kind::Symbol;
            let text = self.lookup_name(name)?;
            let view = self.vm.str_view(text).expect("lookup name is a string");
            let found = self.vm.obarray_lookup_value(table, view.bytes, view.chars);
            Ok(found.filter(|&v| !symbol || v == name).unwrap_or(Value::NIL))
        }
        pub fn unintern_in(&mut self, table: Value, name: Value) -> super::Out {
            let found = self.lookup_in(table, name)?;
            // nil is both a legitimate interned symbol and the missing sentinel.
            if found == Value::NIL {
                let text = self.lookup_name(name)?;
                let is_nil = self.vm.str_view(text).is_some_and(|view| view.bytes == b"nil");
                if !is_nil || self.vm.obarray_lookup_value(table, b"nil", 3) != Some(Value::NIL) { return Ok(Value::NIL); }
            }
            Ok(if self.vm.obarray_unintern(table, found) { Value::TRUE } else { Value::NIL })
        }
        pub fn clear_obarray(&mut self, table: Value) -> std::result::Result<(), Fault> { self.vm.obarray_clear(table) }
        pub fn obarray_next(&self, table: Value, bucket: usize, current: Value) -> Option<(usize, Value)> {
            self.vm.obarray_next(table, bucket, current)
        }
        pub fn symbol_name_value(&mut self, symbol: Value) -> super::Out {
            if let Some(&name) = self.vm.permanent_symbol_names.get(&symbol.0) { return Ok(name); }
            if let Ok(slot) = self.symbol_slot(symbol) {
                if let Some(&name) = self.vm.obarray_symbol_names.get(&slot) { return Ok(name); }
            }
            let name = self.symbol_name(symbol)?;
            let value = match crate::symkey::to_internal(&name) {
                Some(internal) => self.make_string(internal, true),
                None if name.is_ascii() => self.byte_string(name.as_bytes()),
                None => self.string(&name),
            };
            self.vm.permanent_symbol_names.insert(symbol.0, value);
            Ok(value)
        }
        /// VALUE as the compiler reads a macro's expansion.
        pub fn expansion_datum(&self, value: Value) -> std::result::Result<Form, Fault> {
            self.vm.expansion_form(value, 0).map_err(Fault::Error)
        }
        /// FORM as a Lisp object.
        pub fn form_value(&mut self, form: &Form) -> std::result::Result<Value, Fault> {
            self.vm.literal(form).map_err(Fault::Error)
        }
        pub fn intern(&mut self, name: &str) -> Value {
            self.vm.intern(name)
        }
        /// The private object V names, for writing; a frozen or image value
        /// is refused with `read_only`.
        pub(crate) fn object_mut(&mut self, v: Value) -> std::result::Result<&mut data::Object, Fault> {
            if Vm::image_value(v) { return Err(self.read_only(v)); }
            self.vm.object_mut(v).map_err(Fault::Error)
        }
        /// GNU's `CHECK_IMPURE` signal for a write to the frozen or image
        /// object V (`frozen.rs`).
        pub fn read_only(&mut self, v: Value) -> Fault {
            let (condition, data) = self.vm.read_only_signal(v);
            Fault::Signal(condition, data)
        }
        /// The window tree, through window.rs's own surface: reads, writes
        /// and allocation only, never evaluation or collection.
        pub fn windows(&mut self) -> crate::window::Windows<'_> {
            crate::window::Windows::new(self.vm)
        }
        /// Fresh symbol slot outside the obarray; never re-enters the VM.
        pub fn make_symbol(&mut self, name: &str) -> Value {
            self.vm.uninterned_symbol(name)
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
        pub fn current_local_map(&mut self) -> std::result::Result<Value, Fault> {
            let index = self.vm.buffer_index().map_err(Fault::Error)?;
            Ok(*self.vm.local_keymap(index))
        }
        pub fn keymap_at_point(&mut self, property: Value) -> super::Out {
            self.vm.keymap_at_point(property).map_err(Fault::Error)
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
                0 if Vm::image_string(v) => Kind::String,
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
                    Some(data::Object::Free) => Kind::Invalid,
                    Some(data::Object::SyntaxTable(_)) => Kind::SyntaxTable,
                    Some(data::Object::CharTable(_)) => Kind::CharTable,
                    Some(data::Object::SubCharTable(_)) => Kind::SubCharTable,
                    Some(data::Object::MapKeymapCursor(_)) => Kind::Invalid,
                    Some(data::Object::Window(_)) => Kind::Window,
                    Some(data::Object::Frame(_)) => Kind::Frame,
                    Some(data::Object::Terminal(_)) => Kind::Terminal,
                    Some(data::Object::WindowConfiguration(_)) => Kind::WindowConfiguration,
                    Some(data::Object::Sqlite(_)) => Kind::Sqlite,
                    Some(data::Object::Process(_)) => Kind::Process,
                    Some(data::Object::Finalizer(_)) => Kind::Finalizer,
                    Some(data::Object::Thread) => Kind::Thread,
                    Some(data::Object::Treesit(t)) => match &**t {
                        crate::treesit::Object::Parser(_) => Kind::TreesitParser,
                        crate::treesit::Object::Node(_) => Kind::TreesitNode,
                        crate::treesit::Object::Query(_) => Kind::TreesitQuery,
                        crate::treesit::Object::Task(_) => Kind::Invalid,
                    },
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
                self.vm.native_case_roots[i] = std::sync::Arc::downgrade(&table.root);
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
        /// The value of a VM-private `#:vm2-' slot, unbound reading as nil.
        pub fn marker_last_position(&mut self, marker: Value) -> super::Out {
            let last = match self.vm.object_if(marker) {
                Some(data::Object::Marker(m)) => m.last_position,
                _ => return Err(Fault::Wrong("markerp", marker)),
            };
            match self.vm.marker_position_value(marker).map_err(Fault::Error)? {
                Value::NIL => Value::integer(last).map_err(Fault::Error),
                position => Ok(position),
            }
        }
        /// The fontset registry.
        pub fn fontsets(&self) -> &crate::fontsets::Registry { &self.vm.display_state.fontsets }
        pub fn fontsets_mut(&mut self) -> &mut crate::fontsets::Registry { &mut self.vm.display_state.fontsets }
        /// What painting reads, the host brought up to date.
        pub fn fontset_state(&mut self) -> std::sync::Arc<romacs_layout_input::FontsetState> { self.vm.fontset_state() }
        /// free_realized_fontsets: rules changed, so what was resolved from
        /// them goes, and every window lays out again.
        pub fn note_fontset_change(&mut self) {
            self.vm.fontset_state();
            self.clear_face_cache();
        }
        pub fn char_table_edits(&self, table: Value) -> std::result::Result<u64, Fault> {
            self.vm.char_table_edits(table).map_err(Fault::Error)
        }
        /// TABLE's runs of one value, in character order.
        pub fn char_table_runs(&self, table: Value) -> std::result::Result<Vec<(u32, u32, Value)>, Fault> {
            self.vm.char_table_runs(table).map_err(Fault::Error)
        }
        /// Fset or Fset_default of a plain global variable.
        pub fn set_variable_named(&mut self, name: &str, value: Value) -> std::result::Result<(), Fault> {
            let symbol = self.intern(name);
            self.variable_set(symbol, value)
        }
        /// The one string naming font NAME: GNU's font-object holds its name,
        /// and face-font and query-font answer that very string, which
        /// display-fill-column-indicator.el compares with eq.
        pub fn font_name_string(&mut self, name: &str) -> super::Out {
            const NAMES: &str = "#:vm2-font-names";
            let table = match self.hidden_value(NAMES) {
                Value::NIL => {
                    let table = self.vm.allocate_object(data::Object::HashTable(Box::new(crate::hash::HashTable::new(crate::hash::Test::Equal, crate::hash::Weakness::None, 0))));
                    self.set_hidden_value(NAMES, table);
                    table
                }
                table => table,
            };
            let text = self.string(name);
            match self.vm.declaration_hash_get(table, text).map_err(Fault::Error)? {
                Some(shared) => Ok(shared),
                None => {
                    self.vm.declaration_hash_put(table, text, text).map_err(Fault::Error)?;
                    Ok(text)
                }
            }
        }
        /// SYMBOL, an alias, made plain and void.
        pub fn delete_variable_alias(&mut self, symbol: Value) {
            if let Some(slot) = symbol.index_if(7).and_then(|n| n.checked_sub(1)) { self.vm.delete_variable_alias(slot); }
        }
        /// Fprint_preprocess.
        pub fn print_preprocess_only(&mut self, object: Value) -> std::result::Result<(), Fault> {
            self.vm.print_preprocess_only(object).map_err(Fault::Error)
        }
        pub fn hidden_value(&mut self, name: &str) -> Value {
            let slot = self.vm.slot(name);
            match self.vm.slots[slot].value { Value::UNBOUND => Value::NIL, value => value }
        }
        pub fn set_hidden_value(&mut self, name: &str, value: Value) {
            let slot = self.vm.slot(name);
            self.vm.slots[slot].value = value;
        }
        /// TABLE's own value at INDEX or its default, ignoring the parent.
        pub fn char_table_own_element(&self, table: Value, index: usize) -> super::Out {
            self.vm.char_table_own(table, index).map_err(Fault::Error)
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
                let data::Object::CharTable(t) = self.object_mut(table)? else { unreachable!() };
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
                let mut seen = HashSet::default();
                while let Some(data::Object::CharTable(t)) = self.vm.object_if(next) {
                    if next == table || !seen.insert(next.0) { return Err(Fault::Error("Attempt to make a chartable be its own parent".into())); }
                    next = t.parent;
                }
                let data::Object::CharTable(t) = self.object_mut(table)? else { unreachable!() };
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
            let data::Object::CharTable(t) = self.object_mut(table)? else { unreachable!() };
            if let Some((from, to)) = bounds { t.set_range_slot(from, to, slot).map_err(Fault::Error)?; }
            else { t.default = value; }
            Ok(value)
        }
        pub fn list_limit(&self) -> usize {
            self.vm.list_limit()
        }
        /// More links than an alias chain of distinct symbols can have.
        pub fn symbol_limit(&self) -> usize {
            self.vm.slots.len()
        }
        /// Characters FROM..TO of STRING as a fresh string of the same
        /// multibyteness, carrying STRING's text properties for that range
        /// (GNU fns.c Fsubstring: make_specified_string + copy_text_properties).
        /// The caller has validated the range against the character length.
        pub fn string_slice(&mut self, string: Value, from: usize, to: usize) -> std::result::Result<Value, Fault> {
            let view = self.vm.str_view(string).ok_or(Fault::Wrong("arrayp", string))?;
            let multibyte = view.is_multibyte();
            let lisp = match self.vm.object_if(string) {
                Some(data::Object::String(s) | data::Object::RawString(s)) => Some(s),
                _ => None,
            };
            let byte = |c: usize| {
                if !multibyte || view.len() == view.bytes().len() {
                    return c;
                }
                // A Lisp string's cursors find it from the last position
                // asked, as GNU's string_char_to_byte cache does.
                if let Some(b) = lisp.and_then(|s| s.byte_offset(coordinates::CharOffset(c))) {
                    return b;
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
    mod keymap_copy;
    mod category;
    mod fileio;
    mod filelock;
    mod file_attributes;
    mod terminal;
    mod process_jobs;
    mod animate;
    mod effects;
    mod clipboard;
    mod keyboard;
    mod filesave;
    mod charset;
    mod coding;
    mod point_predicates;
    mod subst_char;
    mod translate_region;
    mod transpose_regions;
    mod json;
    mod overlays;
    mod invisible;
    mod buffer_locals;
    mod frozen;
    mod isolate;
    mod compiled_lisp;
    mod casetab;
    mod casing;
    pub(crate) use casing::RegionCase;
    mod modification;
    mod buffer_lifecycle;
    mod editor_mark;
    mod printing;
    mod bool_vector;
    mod search;
    mod skip_chars;
    mod word_motion;
    mod field;
    mod syntax_scan;
    pub(crate) use syntax_scan::{Op as SyntaxOp, Outcome as SyntaxOutcome};
    mod face;
    pub(crate) use face::{FaceKeys, FaceProps};
    mod font;
    mod value_order;
    mod completion;
    pub(crate) use completion::{OrderlessPass, OrderlessSpec};
    mod backtrace;
    mod profiler;
    mod text_properties;
    pub(crate) use text_properties::{PropertyBegin, Retry};
    mod undo;
    mod insdel;
    mod sqlite;
    pub(crate) mod treesit;
    pub(crate) mod inotify;
    mod process;
    mod gnutls;
    pub(crate) use process::{CallOutput, Target, strerror};
    mod random;
    mod width;
    pub(crate) mod graphics;
    mod coding_safe;
    mod coding_mule;
    mod display_motion;
    mod display_elements;
    mod display_hosted;
    pub(crate) use display_hosted::{FaceKey, Shown, ShownKind};
    mod text_pixel_size;
    mod bidi;
    pub(crate) use text_pixel_size::{TextSizeArgs, LineLabels};
    mod session;
    mod world;
    mod finalizer;
}
pub(crate) use leaf::{Leaf, Kind, SyntaxOp, SyntaxOutcome, CallOutput, Target, strerror, TextSizeArgs, FaceKey, FaceKeys, FaceProps, Shown, ShownKind, LineLabels};

/// Redisplay's window start over the host's layout (Leaf::hosted_window_start).
pub(crate) fn hosted_window_start(vm: &mut Vm, window: Value, start: i64, point: i64, minimal: bool, back: i64) -> std::result::Result<Option<i64>, String> {
    Leaf::new(vm).hosted_window_start(window, start, point, minimal, back).map_err(|fault| match fault {
        Fault::Error(text) => text,
        _ => "the host's layout cannot choose this window start".into(),
    })
}

/// The stretches SPECS, a list, make in WINDOW's header or mode line
/// (Leaf::band_stretches).
pub(crate) fn band_stretches(vm: &mut Vm, window: Value, specs: Value) -> std::result::Result<Vec<Option<romacs_layout_input::StretchWidth>>, Fault> {
    Leaf::new(vm).band_stretches(window, specs)
}

/// The display elements WINDOW shows of its buffer from FROM, a line
/// start, to TO, as a host paints them (Leaf::window_shown_elements).
pub(crate) fn window_shown_elements(vm: &mut Vm, window: Value, from: i64, to: i64) -> std::result::Result<(Vec<Shown>, Vec<romacs_layout_input::InlineImage>), Fault> {
    Leaf::new(vm).window_shown_elements(window, from, to)
}

pub(crate) fn window_text_shows_glyphs(vm: &mut Vm, window: Value, from: usize, to: usize) -> std::result::Result<bool, Fault> {
    Leaf::new(vm).window_text_shows_glyphs(window, from, to)
}

/// The line numbers WINDOW shows from its start (Leaf::window_line_numbers).
pub(crate) fn window_line_numbers(vm: &mut Vm, window: Value) -> std::result::Result<Option<LineLabels>, Fault> {
    Leaf::new(vm).window_line_numbers(window)
}

/// cmds.c beginning-of-line and end-of-line's target: the line boundary
/// N lines on, constrained to point's field (Leaf::line_field_position).
pub(crate) fn line_field_position(vm: &mut Vm, n: Value, end: bool) -> std::result::Result<i64, Fault> {
    let position = Leaf::new(vm).line_field_position(n, end)?;
    Ok(position.integer_value().expect("constrain-to-field yields a position"))
}

/// Where the screen lines a window shows end over the host's layout
/// (Leaf::hosted_rows_end).
pub(crate) fn hosted_rows_end(vm: &mut Vm, window: Value, start: i64, rows: i64) -> std::result::Result<i64, Fault> {
    Leaf::new(vm).hosted_rows_end(window, start, rows)
}

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
     fn $id:ident ($($params:tt)*) -> Reader $body:block) => {
        $crate::prim::defsubr!(@emit $name, $gnu, $eff, Reader, Out, $id, ($($params)*), $body);
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
include!(concat!(env!("OUT_DIR"), "/variable_sites.rs"));

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
#[path = "prim/legacy_arity.rs"]
mod legacy_arity;

pub(crate) fn arity(value: Value) -> Option<(usize, Option<usize>)> {
    Some(match descriptor(value.index_if(6)?)? {
        Descriptor::Legacy(_, primitive) => legacy_arity::arity(primitive),
        Descriptor::Generated(desc) => (desc.min, desc.max),
    })
}

/// The generated subr declared as NAME.
pub(crate) fn subr_named(name: &str) -> Option<Value> {
    GENERATED.iter().position(|d| d.name == name).map(|i| Value::reference(PRIMITIVES.len() + i, 6))
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

/// A watcher that is a native leaf subr, which can run at a write point
/// without re-entering the engine. Symbols are resolved to their function.
pub(crate) fn native_watcher(vm: &Vm, mut watcher: Value) -> Option<&'static PrimDesc> {
    for _ in 0..64 {
        if let Some(d) = generated(watcher) { return matches!(d.body, Body::Leaf(_)).then_some(d); }
        let slot = watcher.index_if(7)?.checked_sub(1)?;
        watcher = vm.slots.get(slot)?.function;
    }
    None
}
pub(crate) fn call_native_watcher(vm: &mut Vm, watcher: Value, args: &[Value]) -> crate::Result<()> {
    let Some(d) = native_watcher(vm, watcher) else { return Err("VM2 variable watcher is not a native subr".into()); };
    let Body::Leaf(f) = d.body else { unreachable!("native_watcher checks the body") };
    let (condition, data) = if args.len() < d.min || d.max.is_some_and(|m| args.len() > m) {
        let n = Value::integer(args.len() as i64).unwrap();
        vm.condition("wrong-number-of-arguments", &[watcher, n])
    } else {
        match no_collection(vm, |vm| f(&mut Leaf::new(vm), args)) {
            Ok(_) => return Ok(()),
            Err(Fault::Escape) => return Err(format!("variable watcher {} signalled", d.name)),
            Err(fault) => vm.fault(fault),
        }
    };
    // funcall_subr passes the watcher's own signal through the write.
    vm.propagated_escape = Some(crate::threaded::Escape::Signal(condition, data));
    Err(format!("variable watcher {} signalled", d.name))
}
/// Tag-6 payloads at or above `PRIMITIVES.len()` name generated primitives.
#[inline(always)]
pub(crate) fn generated(v: Value) -> Option<&'static PrimDesc> {
    match descriptor(v.index_if(6)?)? {
        Descriptor::Generated(d) => Some(d),
        _ => None,
    }
}

/// The most arguments a leaf takes on the site path.
pub(crate) const LEAF_ARGUMENTS: usize = 8;

/// Whether a call site may run D with ARGC arguments and no activation:
/// nothing can see the missing frame. Stack readers are `Step' primitives
/// (`stack_readers_keep_their_frame'), and Lisp the leaf reaches records the
/// frame first (`Vm::record_site_leaf'). A leaf that writes is kept out all
/// the same: what it touches has not been audited for this.
pub(crate) fn site_leaf(d: &PrimDesc, argc: usize) -> Option<LeafBody> {
    let Body::Leaf(f) = d.body else { return None };
    (matches!(d.effect, Effect::Pure | Effect::Reads)
        && argc >= d.min && d.max.is_none_or(|max| argc <= max) && argc <= LEAF_ARGUMENTS)
        .then_some(f)
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
    /// NAME under the user's home, as fileio.c expands `~' and `~/REST'
    /// (`get_homedir'): the home directory, joined to REST. None for any
    /// other name, `~user' included. No file name handler is asked.
    pub(crate) fn expand_home_name(&mut self, name: &str) -> crate::Result<Option<String>> {
        let Some(rest) = name.strip_prefix('~').filter(|rest| rest.is_empty() || rest.starts_with('/')) else { return Ok(None) };
        let home = no_collection(self, |vm| prims::expand_file_name::home_directory(&mut Leaf::new(vm)));
        let home = match home {
            Ok(home) => home,
            Err(fault) => {
                let (condition, data) = self.fault(fault);
                self.propagated_escape = Some(crate::threaded::Escape::Signal(condition, data));
                return Err("expand-file-name signalled".into());
            }
        };
        let mut joined = String::from_utf8_lossy(&home).into_owned();
        if joined.is_empty() { joined.push('/'); }
        if joined.ends_with('/') && rest.starts_with('/') { joined.pop(); }
        joined.push_str(rest);
        Ok(Some(joined))
    }

    /// The legacy `expand-file-name' row (kept for its ordinal) runs the
    /// defsubr's expansion, refusing where a file name handler would be called.
    pub(crate) fn expand_file_name_legacy(&mut self, name: Value, default: Value) -> crate::Result<Value> {
        let expanded = no_collection(self, |vm| prims::expand_file_name::expand(&mut Leaf::new(vm), name, default));
        match expanded {
            Ok(Some(value)) => Ok(value),
            Ok(None) => Err("VM2 expand-file-name matching file name handler is unsupported".into()),
            Err(fault) => {
                let (condition, data) = self.fault(fault);
                self.propagated_escape = Some(crate::threaded::Escape::Signal(condition, data));
                Err("expand-file-name signalled".into())
            }
        }
    }
    /// load-with-code-conversion for a source without a coding cookie that
    /// is not UTF-8: the detection walk over the coding priority, as
    /// `insert-file-contents' decides, then the chosen system's decoding,
    /// as internal multibyte text (raw-text's bytes as raw-byte characters).
    /// `last-coding-system-used' is left as it was.
    pub(crate) fn detected_source(&mut self, bytes: &[u8]) -> crate::Result<Vec<u8>> {
        let decoded = no_collection(self, |vm| {
            let mut cx = Leaf::new(vm);
            let undecided = cx.intern("undecided");
            let spec = cx.coding_spec(undecided)?;
            if spec == Value::NIL { return Err(Fault::Error("no undecided coding system to detect with".into())) }
            let attrs = cx.vector_get(spec, Value::integer(0).unwrap())?;
            let found = cx.coding_detect(bytes, attrs)?.0.unwrap_or(undecided);
            let last = cx.intern("last-coding-system-used");
            let previous = cx.variable_value(last)?.unwrap_or(Value::NIL);
            let string = cx.make_string(bytes.to_vec(), false);
            let decoded = cx.coding_convert_string(string, found, Value::NIL, Value::NIL, false);
            cx.variable_set(last, previous)?;
            let view = cx.string_view(decoded?)?;
            if view.is_multibyte() { return Ok(view.bytes().to_vec()) }
            let mut out = Vec::with_capacity(view.bytes().len());
            for &byte in view.bytes() {
                let mut encoded = [0; 5];
                let code = if byte < 128 { u32::from(byte) } else { crate::lstring::BYTE8_OFFSET + u32::from(byte) };
                let n = crate::lstring::char_string(code, &mut encoded);
                out.extend_from_slice(&encoded[..n]);
            }
            Ok(out)
        });
        decoded.map_err(|fault| {
            let (condition, data) = self.fault(fault);
            let text = format!("source coding detection signalled {}", self.display(condition).unwrap_or_default());
            self.propagated_escape = Some(crate::threaded::Escape::Signal(condition, data));
            text
        })
    }
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
    /// condition+data, except `Fault::Escape' (None), whose escape waits in
    /// `propagated_escape'. A leaf's value comes back as `Step::Done`.
    pub(crate) fn call_generated(
        &mut self,
        target: Value,
        start: usize,
        argc: usize,
    ) -> std::result::Result<Step, Option<(Value, Value)>> {
        let d = generated(target).expect("generated primitive");
        if argc < d.min || d.max.is_some_and(|m| argc > m) {
            let n = Value::integer(argc as i64).unwrap();
            return Err(Some(self.condition("wrong-number-of-arguments", &[target, n])));
        }
        // Copied off the register stack, which the body may widen; on the
        // native stack for the common argument counts.
        let mut small = [Value::NIL; 8];
        let heap;
        let args: &[Value] = if argc <= small.len() {
            small[..argc].copy_from_slice(&self.stack[start..start + argc]);
            &small[..argc]
        } else {
            heap = self.stack[start..start + argc].to_vec();
            &heap
        };
        let run = |vm: &mut Vm| {
            let mut cx = Leaf::new(vm);
            match d.body {
                Body::Leaf(f) | Body::Reader(f) => f(&mut cx, args).map(Step::Done),
                Body::Call(f) => f(&mut cx, args),
            }
        };
        // The reader owns explicit roots and the engine publishes its suspended
        // values before entering it. Unlike ordinary leaves it can call Lisp.
        let step = if matches!(d.body, Body::Reader(_)) {
            run(self)
        } else {
            no_collection(self, run)
        };
        step.map_err(|fault| self.step_fault(fault))
    }
    /// F on the ARGC values at ARGS, with no activation or `Step': the value,
    /// or UNBOUND with FAULT set. Copied off the register stack as
    /// `call_generated' copies, the caller having checked ARGC against
    /// LEAF_ARGUMENTS.
    ///
    /// # Safety
    /// ARGS must point at ARGC live values.
    #[inline(never)]
    pub(crate) unsafe fn leaf_value(&mut self, f: LeafBody, args: *const Value, argc: usize, fault: &mut Option<Fault>) -> Value {
        let mut small = [Value::NIL; LEAF_ARGUMENTS];
        unsafe { std::ptr::copy_nonoverlapping(args, small.as_mut_ptr(), argc) };
        match no_collection(self, |vm| f(&mut Leaf::new(vm), &small[..argc])) {
            Ok(value) => value,
            Err(failed) => {
                *fault = Some(failed);
                Value::UNBOUND
            }
        }
    }
    /// A leaf's fault, as `call_generated' reports one.
    pub(crate) fn leaf_step(&mut self, fault: Fault) -> std::result::Result<Step, Option<(Value, Value)>> {
        Err(self.step_fault(fault))
    }
    /// Run a T2 continuation with the callback's value and the saved registers.
    pub(crate) fn resume_generated(
        &mut self,
        then: Then,
        result: Value,
        saved: &[Value],
    ) -> std::result::Result<Step, Option<(Value, Value)>> {
        no_collection(self, |vm| then(&mut Leaf::new(vm), result, saved))
            .map_err(|fault| self.step_fault(fault))
    }
    fn step_fault(&mut self, fault: Fault) -> Option<(Value, Value)> {
        (!matches!(fault, Fault::Escape)).then(|| self.fault(fault))
    }
    /// FAULT as condition+data. A pending `Fault::Escape' is taken and
    /// signalled afresh, by callers that cannot resume it.
    pub(super) fn fault(&mut self, fault: Fault) -> (Value, Value) {
        match fault {
            Fault::Escape => match self.propagated_escape.take() {
                Some(crate::threaded::Escape::Signal(condition, data)) => (condition, data),
                Some(crate::threaded::Escape::Error { condition, object, .. }) => {
                    (condition, self.cons_pair(object).expect("normalized signal object").1)
                }
                _ => {
                    let text = self.make_lisp_string(b"VM2 variable watcher escape was lost".to_vec(), false);
                    self.condition("error", &[text])
                }
            },
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
        self.eval_source_raw(prims::HOOKS_LISP)?;
        self.eval_source_raw(prims::PRELUDE)?;
        self.eval_source_raw(prims::PROCESS_PRELUDE)?;
        self.eval_source_raw(prims::INOTIFY_PRELUDE)?;
        self.eval_source_raw(prims::ISOLATE_PRELUDE)
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
            Body::Leaf(f) | Body::Reader(f) => f(&mut cx, args),
            Body::Call(f) => match f(&mut cx, args)? {
                Step::Done(v) => Ok(v),
                Step::Call { .. } | Step::Handle { .. } | Step::Collect | Step::CollectIdle | Step::Safepoint => panic!("expected immediate test result"),
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

/// The start of the line LINES lines from the one holding byte POINT of
/// TEXT, within LOW..HIGH (`forward-line' from that line's start): at LOW
/// or HIGH when there are fewer lines.
pub(crate) fn line_beginning(text: &(impl crate::text::Newlines + ?Sized), low: usize, high: usize, point: usize, lines: i64) -> usize {
    let start_of = |at: usize| text.previous_newline(at, low).map_or(low, |i| i + 1);
    let mut pos = start_of(point);
    if lines >= 0 {
        for _ in 0..lines {
            match text.next_newline(pos, high) {
                Some(i) => pos = i + 1,
                None => return high,
            }
        }
    } else {
        for _ in lines..0 {
            if pos == low { break; }
            pos = start_of(pos - 1);
        }
    }
    pos
}

/// GNU find_before_next_newline from byte POINT of TEXT within LOW..HIGH:
/// for COUNT > 0 the COUNTth newline on, else the -COUNTth newline back;
/// with fewer newlines, HIGH or LOW.
pub(crate) fn line_end_after(text: &(impl crate::text::Newlines + ?Sized), low: usize, high: usize, point: usize, count: i64) -> usize {
    let mut pos = point;
    if count > 0 {
        for _ in 0..count {
            match text.next_newline(pos, high) {
                Some(i) => pos = i + 1,
                None => return high,
            }
        }
        return pos - 1;
    }
    for _ in count..0 {
        match text.previous_newline(pos, low) {
            Some(i) => pos = i,
            None => return low,
        }
    }
    pos
}

#[cfg(test)]
mod line_scan_tests {
    use super::{line_beginning, line_end_after};

    /// The character-vector walks the byte scans replaced, in characters.
    fn bol_chars(chars: &[char], low: usize, high: usize, point: usize, lines: i64) -> usize {
        let mut pos = point;
        while pos > low && chars[pos - 1] != '\n' { pos -= 1; }
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
                while pos > low && chars[pos - 1] != '\n' { pos -= 1; }
            }
        }
        pos
    }
    fn eol_chars(chars: &[char], low: usize, high: usize, point: usize, count: i64) -> usize {
        let mut pos = point;
        if count > 0 {
            for _ in 0..count {
                match (pos..high).find(|&at| chars[at] == '\n') { Some(at) => pos = at + 1, None => return high }
            }
            return pos - 1;
        }
        for _ in count..0 {
            match (low..pos).rev().find(|&at| chars[at] == '\n') { Some(at) => pos = at, None => return low }
        }
        pos
    }

    #[test]
    fn byte_scans_agree_with_character_walks() {
        let text = "λx\n\nnaïve 日本\nend\n\nlast é";
        let chars: Vec<char> = text.chars().collect();
        let byte = |c: usize| text.char_indices().nth(c).map_or(text.len(), |(b, _)| b);
        let char_of = |b: usize| text[..b].chars().count();
        for (low, high) in [(0, chars.len()), (2, chars.len() - 3), (5, 5)] {
            for point in low..=high {
                for count in -4..=4 {
                    let (bl, bh, bp) = (byte(low), byte(high), byte(point));
                    assert_eq!(char_of(line_beginning(text.as_bytes(), bl, bh, bp, count)), bol_chars(&chars, low, high, point, count), "bol {low}..{high} at {point} by {count}");
                    assert_eq!(char_of(line_end_after(text.as_bytes(), bl, bh, bp, count)), eol_chars(&chars, low, high, point, count), "eol {low}..{high} at {point} by {count}");
                }
            }
        }
        // A rope of many chunks scans as its flat text does.
        let long = text.repeat(400);
        let rope = crate::text::Text::new(long.as_bytes()).unwrap();
        let body: &crate::text::TextBody = &rope;
        assert!(body.chunks().count() > 4);
        for point in (0..long.len()).step_by(97).filter(|&b| long.is_char_boundary(b)) {
            for count in [-30, -2, -1, 0, 1, 2, 30] {
                for (low, high) in [(0, long.len()), (1200, long.len() - 900)] {
                    let point = point.clamp(low, high);
                    assert_eq!(line_beginning(body, low, high, point, count), line_beginning(long.as_bytes(), low, high, point, count));
                    assert_eq!(line_end_after(body, low, high, point, count), line_end_after(long.as_bytes(), low, high, point, count));
                }
            }
        }
    }
}

#[cfg(test)]
mod site_leaf_tests {
    /// `site_leaf' runs Pure and Reads leaves with no activation, so none of
    /// them may read the stack: such a primitive is a `Step' one.
    #[test]
    fn stack_readers_keep_their_frame() {
        let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src/prims");
        for entry in std::fs::read_dir(dir).unwrap() {
            let path = entry.unwrap().path();
            let Ok(text) = std::fs::read_to_string(&path) else { continue };
            for form in text.split("defsubr! {").skip(1) {
                let header = form.split('{').next().unwrap();
                let quiet = header.contains("effect = Pure") || header.contains("effect = Reads");
                let reads_stack = form.split("defsubr!").next().unwrap().split("\n}}").next().unwrap()
                    .contains("cx.backtrace") || form.split("\n}}").next().unwrap().contains("cx.inspect_frames");
                assert!(!(quiet && reads_stack && !header.contains("-> Step")),
                    "{}: a stack reader must be a Step primitive: {}", path.display(), header.trim());
            }
        }
    }
}
