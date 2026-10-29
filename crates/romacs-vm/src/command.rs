//! Owned command boundary. Lisp-visible state stays in ordinary traced slots.
use super::*;
use data::Object;
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum CommandEvent {
    Character(i64),
    Named(String),
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CommandSignal {
    pub condition: String,
    pub data: String,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CommandOutcome {
    pub command: Option<String>,
    pub signal: Option<CommandSignal>,
    pub bell: bool,
    pub message: Option<String>,
    pub redisplay: bool,
}
fn sym(s: &str) -> Form {
    Form::Sym(s.into())
}
fn call(s: &str, a: Vec<Form>) -> Form {
    Form::List(std::iter::once(sym(s)).chain(a).collect())
}
fn quote(f: Form) -> Form {
    call("quote", vec![f])
}
enum CommandFault {
    Internal(String),
    Boundary(&'static str),
}
impl From<String> for CommandFault {
    fn from(s: String) -> Self {
        Self::Internal(s)
    }
}
impl Vm {
    pub(super) fn form_is_string(&self, form: &Form) -> bool {
        match form {
            Form::String(_) | Form::Bytes(_) => true,
            Form::Object(object) => object.value(self).is_ok_and(|value| self.str_view(value).is_some()),
            _ => false,
        }
    }
    /// A lambda expression, compiled as GNU `Ffunction' (eval.c:575-623) builds
    /// an interpreted closure: after an optional docstring (a string with more
    /// body after it, or `(:documentation FORM)'), a leading `(interactive
    /// SPEC . MODES)' is taken out of the body and becomes the closure's
    /// interactive spec, `[SPEC MODES]' when MODES is non-nil, as
    /// make-interpreted-closure makes CLOSURE_INTERACTIVE (`command-modes'
    /// reads MODES; `interactive-form' gives SPEC alone).
    /// For a `defun' body, GNU's `defun' macro (byte-run.el) also lets the
    /// `declare' forms come before it. VM2 closures have no
    /// CLOSURE_INTERACTIVE slot, so the spec is registered against the closure
    /// object by `#:vm2-set-interactive-form', with a thunk compiled in the
    /// same lexical environment when SPEC must be evaluated (callint.c:309-319
    /// evaluates it in the closure's environment).
    pub(super) fn closure(
        &mut self,
        builder: &mut Builder,
        args: &Form,
        body: &[Form],
        definition: bool,
        exposed: bool,
    ) -> Result<usize> {
        let head_is = |form: Option<&Form>, name: &str| match form {
            Some(Form::List(items) | Form::Dotted(items, _)) => {
                matches!(items.first(), Some(Form::Sym(s)) if s == name)
            }
            _ => false,
        };
        // `defun' (byte-run.el:462-487) first takes its metadata forms in any
        // order, as `byte-run--parse-body' does (byte-run.el:324-356): the
        // first docstring (a string with more body, or `(:documentation
        // FORM)') unless a `declare' or `interactive' came before it, the
        // `declare' forms, and the first `interactive'. Out-of-order or
        // repeated ones are dropped (GNU warns). It rebuilds the lambda as
        // DOC, INTERACTIVE, BODY, with `(nil)' for an empty body.
        let parsed: Vec<Form>;
        let body: &[Form] = if definition {
            let (mut doc, mut declared, mut iform, mut j) = (None, false, None, 0);
            while let Some(form) = body.get(j) {
                let docstring = self.form_is_string(form) && j + 1 < body.len()
                    || head_is(Some(form), ":documentation");
                if docstring {
                    if doc.is_none() && !declared && iform.is_none() {
                        doc = Some(form.clone());
                    }
                } else if head_is(Some(form), "declare") {
                    declared |= iform.is_none();
                } else if head_is(Some(form), "interactive") {
                    iform.get_or_insert_with(|| form.clone());
                } else {
                    break;
                }
                j += 1;
            }
            let mut rebuilt: Vec<Form> = doc.into_iter().chain(iform).chain(body[j..].iter().cloned()).collect();
            if rebuilt.is_empty() {
                rebuilt.push(sym("nil"));
            }
            parsed = rebuilt;
            &parsed
        } else {
            body
        };
        let mut doc_register = None;
        // `(:documentation FORM)': GNU evaluates FORM when the closure is
        // made (eval_sub, eval.c:594-600) and takes it out of the body. VM2
        // retains the resulting object in CLOSURE_DOC_STRING. A
        // string docstring stays in the body, where `lambda' handles it.
        let (owned, body): (Vec<Form>, &[Form]) = if head_is(body.first(), ":documentation") {
            let doc = match &body[0] {
                // Fcar (XCDR (docstring)): a dotted tail after FORM is ignored.
                Form::List(items) | Form::Dotted(items, _) if items.len() >= 2 => items[1].clone(),
                Form::List(_) => sym("nil"),
                // `(:documentation . X)': Fcar of the atom X signals as GNU's does.
                other => call("car", vec![call("cdr", vec![quote(other.clone())])]),
            };
            doc_register = Some(self.expr(builder, &doc)?);
            (body[1..].to_vec(), &[])
        } else {
            (Vec::new(), body)
        };
        let documented = body.is_empty();
        let body: &[Form] = if documented { &owned } else { body };
        let i = match body.first() {
            Some(form) if self.form_is_string(form) && !documented && body.len() > 1 => 1,
            _ => 0,
        };
        if i == 1 && doc_register.is_none() {
            let doc = self.literal(&body[0])?;
            let register = builder.reg();
            builder.emit(Op::Const(register, doc));
            doc_register = Some(register);
        }
        if body.is_empty() {
            let result = self.lambda_with_metadata(builder, args.list()?, &[sym("nil")], exposed)?;
            return Ok(if let Some(doc) = doc_register { self.closure_native_call(builder, Primitive::ClosureDocument, &[result, doc]) } else { result });
        }
        if !head_is(body.get(i), "interactive") {
            let result = self.lambda_with_metadata(builder, args.list()?, body, exposed)?;
            return Ok(if let Some(doc) = doc_register { self.closure_native_call(builder, Primitive::ClosureDocument, &[result, doc]) } else { result });
        }
        // make-interpreted-closure (eval.c:539-544): the spec is (car (cdr
        // IFORM)); a non-nil cdr after it is MODES, kept with it as
        // [SPEC MODES]; `(interactive . X)' with X not a list signals `listp'.
        let (spec, modes) = match &body[i] {
            Form::List(items) => (
                items.get(1).cloned().unwrap_or_else(|| sym("nil")),
                (items.len() > 2).then(|| Form::List(items[2..].to_vec())),
            ),
            Form::Dotted(items, tail) if items.len() >= 2 => (
                items[1].clone(),
                Some(if items.len() > 2 { Form::Dotted(items[2..].to_vec(), tail.clone()) } else { (**tail).clone() }),
            ),
            Form::Dotted(_, tail) => {
                return self.expr(builder, &call("car", vec![quote((**tail).clone())]));
            }
            _ => unreachable!("head_is matched a list"),
        };
        let mut lambda = vec![sym("lambda"), args.clone()];
        lambda.extend_from_slice(&body[..i]);
        lambda.extend_from_slice(&body[i + 1..]);
        if i + 1 == body.len() {
            // `if (NILP (cdr)) cdr = Fcons (Qnil, Qnil)': the body is never empty.
            lambda.push(sym("nil"));
        }
        let thunk = match &spec {
            form if self.form_is_string(form) => sym("nil"),
            Form::Sym(s) if s == "nil" => sym("nil"),
            form => call("function", vec![Form::List(vec![sym("lambda"), Form::List(vec![]), form.clone()])]),
        };
        let executable = lambda[2..].to_vec();
        // One slot value, registered and in the closure's slot 5, as GNU's
        // closure holds it once. [SPEC MODES] is a fresh vector for each
        // closure, as make-interpreted-closure's Fvector makes it.
        let spec_value = self.literal(&spec)?;
        let spec_reg = builder.reg();
        builder.emit(Op::Const(spec_reg, spec_value));
        let spec_reg = match modes {
            Some(modes) => {
                let modes_value = self.literal(&modes)?;
                let modes_reg = builder.reg();
                builder.emit(Op::Const(modes_reg, modes_value));
                self.closure_native_call(builder, Primitive::Vector, &[spec_reg, modes_reg])
            }
            None => spec_reg,
        };
        let saved_captures = std::mem::replace(&mut self.keep_captures, exposed);
        let parts = self.expr(builder, &call("function", vec![Form::List(lambda)]))
            .and_then(|closure| Ok((closure, self.expr(builder, &thunk)?)));
        self.keep_captures = saved_captures;
        let (closure, thunk) = parts?;
        let register = crate::prim::subr_named("#:vm2-set-interactive-form").expect("registration subr");
        let result = Self::native_call(builder, register, &[closure, spec_reg, thunk]);
        let result = if exposed { self.expose_closure(builder, args.list()?, &executable, result)? } else { result };
        let doc = match doc_register { Some(doc) => doc, None => { let doc = builder.reg(); builder.emit(Op::Const(doc, Value::NIL)); doc } };
        Ok(self.closure_native_call(builder, Primitive::ClosureDocument, &[result, doc, spec_reg]))
    }
    pub(super) fn initialize_command_bindings(&mut self, global: Value) {
        for (code, name) in [
            (1, "beginning-of-line"),
            (2, "backward-char"),
            (4, "delete-char"),
            (5, "end-of-line"),
            (6, "forward-char"),
            (8, "delete-backward-char"),
            (127, "delete-backward-char"),
            (10, "self-insert-command"),
            (13, "self-insert-command"),
        ] {
            let command = self.intern(name);
            self.keymap_store(global, Value::integer(code).unwrap(), command, false)
                .unwrap();
        }
        for (low, high) in [(32, 126), (160, 0x10ffff)] {
            let range =
                self.keymap_cons(Value::integer(low).unwrap(), Value::integer(high).unwrap());
            let command = self.intern("self-insert-command");
            self.keymap_store(global, range, command, false).unwrap();
        }
        for (event, name) in [
            ("left", "backward-char"),
            ("right", "forward-char"),
            ("home", "beginning-of-line"),
            ("end", "end-of-line"),
            ("delete", "delete-char"),
            ("backspace", "delete-backward-char"),
        ] {
            let event = self.intern(event);
            let command = self.intern(name);
            self.keymap_store(global, event, command, false).unwrap();
        }
        for name in [
            "last-command-event",
            "this-command",
            "last-command",
            "current-prefix-arg",
            "prefix-arg",
        ] {
            if !self.names.contains_key(name) {
                self.declare_special_raw(name, Value::NIL);
            }
        }
    }
    /// Execute one complete, owned event. Recoverable Lisp signals are returned
    /// structurally, not inferred from error text; every outcome requests redraw.
    pub fn command_event(&mut self, event: CommandEvent) -> Result<CommandOutcome> {
        self.ensure_keymaps();
        let event_form = match event {
            CommandEvent::Character(c) => {
                if c < 0 {
                    return Err("invalid event".into());
                }
                Value::integer(c)?;
                Form::Int(c)
            }
            CommandEvent::Named(name) => {
                if name.is_empty() {
                    return Err("empty event symbol".into());
                }
                sym(&name)
            }
        };
        self.eval_form_raw(&call(
            "setq",
            vec![
                sym("last-command-event"),
                quote(event_form.clone()),
                sym(crate::prim::prims::keyboard::COMMAND_KEYS),
                Form::Vector(vec![event_form.clone()]),
                sym(crate::prim::prims::keyboard::SINGLE_KEY_START),
                Form::Int(0),
            ],
        ))?;
        // The GUI records the keys of a sequence it resolved itself.
        if !matches!(&event_form, Form::Sym(name) if name == "romacs-gui-sequence") {
            self.record_recent_key(match &event_form { Form::Int(c) => RecentKey::Char(*c), Form::Sym(name) => RecentKey::Event(name.clone()), _ => unreachable!("an event form") });
        }
        let binding = self.eval_form_raw(&call("key-binding", vec![Form::Vector(vec![event_form])]))?;
        if binding == Value::NIL || self.resolve_keymap(binding).is_some() {
            // An undefined key runs no command: GNU's pseudo-event (nil . nil).
            if binding == Value::NIL { self.record_recent_key(RecentKey::Command("nil".into())); }
            self.eval_form_raw(&call("setq", vec![sym("prefix-arg"), sym("nil")]))?;
            let message = "Key is undefined".to_owned();

            return Ok(CommandOutcome {
                command: None,
                signal: None,
                bell: true,
                message: Some(message),
                redisplay: true,
            });
        }
        // Convert immediately: eval_form may compact on every invocation.
        let command = self
            .symbol_names
            .get(binding.index(7)?.saturating_sub(1))
            .ok_or("symbol command required")?
            .clone();
        self.record_recent_key(RecentKey::Command(command.clone()));
        // command_loop_1: the previous command's undo boundaries, then the
        // point record_point compares against.
        self.eval_form_raw(&call("if", vec![
            call("fboundp", vec![quote(sym("undo-auto--add-boundary"))]),
            call("undo-auto--add-boundary", vec![]),
        ]))?;
        self.note_command_point()?;
        let form = call(
            "progn",
            vec![
                call("setq", vec![
                    sym("this-command"), quote(sym(&command)),
                    sym("real-this-command"), quote(sym(&command)),
                    sym("current-prefix-arg"), sym("prefix-arg"),
                    sym("prefix-arg"), sym("nil"),
                ]),
                call(
                    "condition-case",
                    vec![
                        sym("#:command-error"),
                        call(
                            "progn",
                            vec![
                                call(
                                    "unless",
                                    vec![
                                        call(
                                            "and",
                                            vec![
                                                call("fboundp", vec![quote(sym("romacs-delete-selection-before-command"))]),
                                                call("romacs-delete-selection-before-command", vec![sym("this-command")]),
                                            ],
                                        ),
                                        call("call-interactively", vec![quote(sym(&command))]),
                                    ],
                                ),
                                sym("nil"),
                            ],
                        ),
                        Form::List(vec![
                            Form::List(vec![sym("error"), sym("quit")]),
                            call("setq", vec![sym("prefix-arg"), sym("nil")]),
                            sym("#:command-error"),
                        ]),
                    ],
                ),
            ],
        );
        let form = call("let", vec![
            Form::List(vec![
                Form::List(vec![sym("#:command-buffer"), call("current-buffer", vec![])]),
                Form::List(vec![sym("#:command-tick"), call("buffer-chars-modified-tick", vec![])]),
            ]),
            call("prog1", vec![form,
                call("when", vec![
                    call("and", vec![
                        call("eq", vec![sym("#:command-buffer"), call("current-buffer", vec![])]),
                        call("/=", vec![sym("#:command-tick"), call("buffer-chars-modified-tick", vec![])]),
                        call("fboundp", vec![quote(sym("deactivate-mark"))]),
                    ]),
                    call("deactivate-mark", vec![sym("t")]),
                ]),
            ]),
        ]);
        let outcome = self.eval_form_raw(&form)?;
        let signal = if outcome == Value::NIL {
            None
        } else {
            let (condition, data) = self.cons_pair(outcome)?;
            Some(CommandSignal {
                condition: self.display(condition)?,
                data: self.display(data)?,
            })
        };
        self.eval_form_raw(&call(
            "setq",
            vec![
                sym("last-command"), sym("this-command"),
                sym("real-last-command"), sym("real-this-command"),
            ],
        ))?;
        let message = signal
            .as_ref()
            .map(|s| format!("{}: {}", s.condition, s.data));

        Ok(CommandOutcome {
            command: Some(command),
            bell: signal.is_some(),
            signal,
            message,
            redisplay: true,
        })
    }
    pub(super) fn command_primitive(
        &mut self,
        p: Primitive,
        start: usize,
        n: usize,
    ) -> Result<Value> {
        self.command_impl(p, start, n).map_err(|f| match f {
            CommandFault::Internal(s) => s,
            CommandFault::Boundary(s) => s.into(),
        })
    }
    pub(super) fn command_call(
        &mut self,
        p: Primitive,
        start: usize,
        n: usize,
        signal: &mut Option<(Value, Value)>,
    ) -> Result<Value> {
        self.command_impl(p, start, n).map_err(|f| match f {
            CommandFault::Internal(s) => {
                if s == "buffer mutation refused: read-only" {
                    let condition = self.intern("buffer-read-only");
                    let data = self.list_value(&[self.slots[self.buffer_slot].value]);
                    *signal = Some((condition, data));
                }
                s
            }
            CommandFault::Boundary(s) => {
                let condition = self.intern(s);
                *signal = Some((condition, Value::NIL));
                s.into()
            }
        })
    }
    fn command_impl(
        &mut self,
        p: Primitive,
        start: usize,
        n: usize,
    ) -> std::result::Result<Value, CommandFault> {
        use Primitive::*;
        let valid = match p {
            ForwardChar | BackwardChar | BeginningOfLine | EndOfLine => n <= 1,
            _ => false,
        };
        if !valid {
            return Err(CommandFault::Internal("wrong primitive arity".into()));
        }
        let count = if n == 0
            || (self.stack[start] == Value::NIL
                && matches!(p, ForwardChar | BackwardChar | BeginningOfLine | EndOfLine))
        {
            1
        } else {
            self.stack[start].as_integer()?
        };
        let index = self.buffer_index()?;
        // Byte scans: a newline byte is never part of another character.
        let (text, point_byte, low_byte, high_byte): (&[u8], usize, usize, usize) = match &self.objects[index] {
            Object::Buffer(b) => { let range = b.accessible(); (b.as_str().as_bytes(), b.point.0, range.start.0, range.end.0) }
            Object::ByteBuffer(b) => (&b.bytes, b.point.0, b.restriction.start.0, b.restriction.end.0),
            _ => unreachable!(),
        };
        let line = matches!(p, BeginningOfLine | EndOfLine).then(|| {
            let start = crate::prim::line_beginning(text, low_byte, high_byte, point_byte, count.saturating_sub(1));
            if p == EndOfLine { text[start..high_byte].iter().position(|&c| c == b'\n').map_or(high_byte, |i| start + i) } else { start }
        });
        let char_of = |byte: usize| -> std::result::Result<usize, CommandFault> {
            Ok(match &self.objects[index] {
                Object::Buffer(b) => b.char_of_byte(crate::coordinates::ByteOffset(byte))?.0,
                _ => byte,
            })
        };
        let (point, low, high) = (char_of(point_byte)?, char_of(low_byte)?, char_of(high_byte)?);
        let delta = if p == BackwardChar {
            -count
        } else {
            count
        };
        let target = match line {
            Some(byte) => char_of(byte)? as i64,
            None => (point as i64).saturating_add(delta),
        };
        let boundary = if target < (low as i64) {
            Some("beginning-of-buffer")
        } else if target > (high as i64) {
            Some("end-of-buffer")
        } else {
            None
        };
        let dest = target.clamp(low as i64, high as i64);
        self.command_buffer_call(GotoChar, &[Value::integer(dest + 1)?])?;
        if let Some(s) = boundary {
            return Err(CommandFault::Boundary(s));
        }
        Ok(Value::NIL)
    }
    fn command_buffer_call(&mut self, p: Primitive, args: &[Value]) -> Result<Value> {
        // These buffer kernels cannot collect or run Lisp. Preserve the original
        // register allocation: extending it could invalidate the threaded engine
        // raw register pointer even if no GC ran.
        let saved = std::mem::replace(&mut self.stack, args.to_vec());
        let result = self.buffer_primitive(p, 0, args.len());
        self.stack = saved;
        result
    }
}

impl Vm {
    /// record_char for a host that reads a key sequence's prefix keys itself.
    pub fn record_recent_event(&mut self, event: &CommandEvent) {
        self.record_recent_key(match event {
            CommandEvent::Character(c) => RecentKey::Char(*c),
            CommandEvent::Named(name) => RecentKey::Event(name.clone()),
        });
    }
    /// record_char for an event, or with COMMAND the command loop's
    /// (nil . COMMAND), as the Lisp half reads them. Only the shapes
    /// `RecentKey' holds are recorded: characters, symbol events and symbol
    /// commands; a mouse event or an anonymous command is left out.
    pub(crate) fn record_recent_value(&mut self, value: Value, command: bool) {
        let name = |vm: &Self| value.index_if(7).and_then(|i| vm.symbol_names.get(i.checked_sub(1)?)).cloned();
        let key = if command {
            if value == Value::NIL { Some(RecentKey::Command("nil".into())) } else { name(self).map(RecentKey::Command) }
        } else if let Some(c) = value.integer_value() {
            Some(RecentKey::Char(c))
        } else {
            name(self).map(RecentKey::Event)
        };
        if let Some(key) = key { self.record_recent_key(key); }
    }
    /// record_char: keep the last RECENT_KEYS entries.
    fn record_recent_key(&mut self, key: RecentKey) {
        if self.recent_keys.len() == RECENT_KEYS { self.recent_keys.pop_front(); }
        self.recent_keys.push_back(key);
    }
    /// Frecent_keys: the recorded input events, oldest first, and with
    /// INCLUDE_CMDS the commands as (nil . COMMAND).
    pub(crate) fn recent_keys_value(&mut self, include_commands: bool) -> Value {
        let keys: Vec<RecentKey> = self.recent_keys.iter().cloned().collect();
        let mut values = Vec::with_capacity(keys.len());
        for key in keys {
            match key {
                RecentKey::Char(c) => values.push(Value::integer(c).expect("event fixnum")),
                RecentKey::Event(name) => values.push(self.intern(&name)),
                RecentKey::Command(name) if include_commands => {
                    let command = if name == "nil" { Value::NIL } else { self.intern(&name) };
                    values.push(self.keymap_cons(Value::NIL, command));
                }
                RecentKey::Command(_) => {}
            }
        }
        self.allocate_object(data::Object::Vector(values))
    }
}
