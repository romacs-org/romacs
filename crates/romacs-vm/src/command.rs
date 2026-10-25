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
    /// interactive spec; MODES are dropped, as `interactive-form' drops them.
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
        // IFORM)); a non-nil cdr after it is MODES, which `interactive-form'
        // drops; `(interactive . X)' with X not a list signals `listp'.
        let spec = match &body[i] {
            Form::List(items) => items.get(1).cloned().unwrap_or_else(|| sym("nil")),
            Form::Dotted(items, _) if items.len() >= 2 => items[1].clone(),
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
        let saved_captures = std::mem::replace(&mut self.keep_captures, exposed);
        let result = self.expr(
            builder,
            &call(
                "#:vm2-set-interactive-form",
                vec![call("function", vec![Form::List(lambda)]), quote(spec.clone()), thunk],
            ),
        );
        self.keep_captures = saved_captures;
        let result = result?;
        let result = if exposed { self.expose_closure(builder, args.list()?, &executable, result)? } else { result };
        let spec_value = self.literal(&spec)?;
        let spec_reg = builder.reg();
        builder.emit(Op::Const(spec_reg, spec_value));
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
            "#:command-keys",
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
                sym("#:command-keys"),
                Form::Vector(vec![event_form.clone()]),
            ],
        ))?;
        let binding = self.eval_form_raw(&call("key-binding", vec![Form::Vector(vec![event_form])]))?;
        if binding == Value::NIL || self.resolve_keymap(binding).is_some() {
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
            SelfInsertCommand | DeleteChar | DeleteBackwardChar => (1..=2).contains(&n),
            ForwardChar | BackwardChar | BeginningOfLine | EndOfLine => n <= 1,
            ThisCommandKeys => n == 0,
            _ => false,
        };
        if !valid {
            return Err(CommandFault::Internal("wrong primitive arity".into()));
        }
        if p == ThisCommandKeys {
            return Ok(self
                .names
                .get("#:command-keys")
                .map_or(Value::NIL, |&s| self.slots[s].value));
        }
        let count = if n == 0
            || (self.stack[start] == Value::NIL
                && matches!(p, ForwardChar | BackwardChar | BeginningOfLine | EndOfLine))
        {
            1
        } else {
            self.stack[start].as_integer()?
        };
        if p == SelfInsertCommand {
            let event = if n == 2 && self.stack[start + 1] != Value::NIL {
                let value = self.stack[start + 1];
                let slot = self.slot("last-command-event");
                self.slots[slot].value = value;
                value
            } else {
                let slot = self.slot("last-command-event");
                self.slots[slot].value
            };
            if count < 0 {
                return Err(CommandFault::Internal(format!(
                    "Negative repetition argument {count}"
                )));
            }
            let Some(c) = event
                .integer_value()
                .and_then(|c| u32::try_from(c).ok())
                .and_then(char::from_u32)
            else {
                return Ok(Value::NIL);
            };
            let size = (count as usize)
                .checked_mul(c.len_utf8())
                .ok_or("insertion too large".to_owned())?;
            let mut inserted = String::new();
            inserted
                .try_reserve(size)
                .map_err(|_| "insertion too large".to_owned())?;
            for _ in 0..count {
                inserted.push(c);
            }
            let value = self.allocate_object(Object::String(Box::new(data::LispString::new(
                &inserted,
            ))));
            return self
                .command_buffer_call(Insert, &[value])
                .map_err(Into::into);
        }
        if n == 2 && self.stack[start + 1] != Value::NIL {
            return Err(CommandFault::Internal("killflag is not supported".into()));
        }
        let index = self.buffer_index()?;
        let (chars, point, low, high) = match &self.objects[index] {
            Object::Buffer(b) => {
                let chars = b.as_str().chars().collect::<Vec<_>>();
                let point = b.char_of_byte(b.point)?.0;
                let range = b.accessible();
                (
                    chars,
                    point,
                    b.char_of_byte(range.start)?.0,
                    b.char_of_byte(range.end)?.0,
                )
            }
            Object::ByteBuffer(b) => (
                b.bytes.iter().map(|&b| b as char).collect(),
                b.point.0,
                b.restriction.start.0,
                b.restriction.end.0,
            ),
            _ => unreachable!(),
        };
        let delta = if matches!(p, BackwardChar | DeleteBackwardChar) {
            -count
        } else {
            count
        };
        let mut target = (point as i64).saturating_add(delta);
        if matches!(p, BeginningOfLine | EndOfLine) {
            let mut pos = point;
            while pos > low && chars[pos - 1] != '\n' {
                pos -= 1;
            }
            let lines = count.saturating_sub(1);
            if lines >= 0 {
                for _ in 0..lines {
                    while pos < high && chars[pos] != '\n' {
                        pos += 1;
                    }
                    if pos < high {
                        pos += 1;
                    } else {
                        break;
                    }
                }
            } else {
                for _ in lines..0 {
                    if pos == low {
                        break;
                    }
                    pos -= 1;
                    while pos > low && chars[pos - 1] != '\n' {
                        pos -= 1;
                    }
                }
            }
            if p == EndOfLine {
                while pos < high && chars[pos] != '\n' {
                    pos += 1;
                }
            }
            target = pos as i64;
        }
        let boundary = if target < (low as i64) {
            Some("beginning-of-buffer")
        } else if target > (high as i64) {
            Some("end-of-buffer")
        } else {
            None
        };
        let dest = target.clamp(low as i64, high as i64);
        if matches!(p, DeleteChar | DeleteBackwardChar) {
            if let Some(s) = boundary {
                return Err(CommandFault::Boundary(s));
            }
            self.command_buffer_call(
                DeleteRegion,
                &[Value::integer(point as i64 + 1)?, Value::integer(dest + 1)?],
            )?;
        } else {
            self.command_buffer_call(GotoChar, &[Value::integer(dest + 1)?])?;
            if let Some(s) = boundary {
                return Err(CommandFault::Boundary(s));
            }
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
