//! One integer-character grammar for host sources and live Lisp streams.
use crate::{Form, Result, Value, Vm, data::Object};
mod build;
mod parse;
use parse::Input;

impl Vm {
    /// Read one datum, returning its UTF-8 byte end offset. Comments/whitespace
    /// alone return None. Root the value explicitly if retaining across execution.
    pub(crate) fn read_source_raw(&mut self, source: &str) -> Result<Option<(Value, usize)>> {
        self.read_source_span(source)
            .map(|v| v.map(|(value, _, end)| (value, end)))
    }
    pub(super) fn read_source_span(
        &mut self,
        source: &str,
    ) -> Result<Option<(Value, usize, usize)>> {
        self.read_internal_span(source.as_bytes())
    }
    pub(super) fn read_internal_span(
        &mut self,
        source: &[u8],
    ) -> Result<Option<(Value, usize, usize)>> {
        self.read_internal_signal(source).map_err(|(text, _)| text)
    }
    /// `read_source_span`, also returning GNU's exact `(CONDITION . DATA)`
    /// when the reader knows it. Both values are unrooted: use them before
    /// anything can collect (a leaf primitive signalling them does).
    pub(super) fn read_source_signal(
        &mut self,
        source: &str,
    ) -> std::result::Result<Option<(Value, usize, usize)>, (String, Option<(Value, Value)>)> {
        self.read_internal_signal(source.as_bytes())
    }
    /// `read_source_signal` over GNU internal multibyte text.
    pub(super) fn read_internal_signal(
        &mut self,
        source: &[u8],
    ) -> std::result::Result<Option<(Value, usize, usize)>, (String, Option<(Value, Value)>)> {
        self.read_input(Input::Host { text: source, byte: 0 })
    }
    pub(super) fn read_lisp_string(
        &mut self,
        string: Value,
        start: usize,
        end: usize,
    ) -> std::result::Result<Option<(Value, usize, usize)>, (String, Option<(Value, Value)>)> {
        let root = self.external_roots.len();
        self.external_roots.push(string);
        let view = self.str_view(string).unwrap();
        let byte = if view.is_multibyte() {
            view.chars()
                .take(start)
                .map(|c| {
                    let mut b = [0; 5];
                    crate::lstring::char_string(c, &mut b)
                })
                .sum()
        } else {
            start
        };
        let result = self.read_input(Input::String {
            root,
            byte,
            character: start,
            end,
        });
        self.external_roots.truncate(root);
        result
    }
    pub(super) fn read_lisp_stream(
        &mut self,
        stream: Value,
    ) -> std::result::Result<Option<(Value, usize, usize)>, (String, Option<(Value, Value)>)> {
        let root = self.external_roots.len();
        self.external_roots.push(stream);
        let input = if let Some(view) = self.str_view(stream) {
            Input::String {
                root,
                byte: 0,
                character: 0,
                end: view.len(),
            }
        } else {
            match self.object_if(stream) {
                Some(Object::Buffer(_) | Object::ByteBuffer(_)) => Input::Buffer { root },
                Some(Object::Marker(_)) => Input::Marker { root },
                _ => Input::Function { root },
            }
        };
        let result = self.read_input(input);
        self.external_roots.truncate(root);
        result
    }

    pub(super) fn source_form(&self, value: Value, depth: usize) -> Result<Form> {
        self.datum_form(value, depth, Datum::Source)
    }
    /// Like `source_form`, but an object with no source spelling becomes
    /// `Form::Object` instead of an error: GNU evaluates the object itself
    /// (src/eval.c eval_sub). Object constants own traced roots until every
    /// clone of the form is dropped. These VM-local handles are not persistent.
    pub(super) fn expansion_form(&self, value: Value, depth: usize) -> Result<Form> {
        self.datum_form(value, depth, Datum::Expansion)
    }
    /// Like `expansion_form`, for the datum `eval' evaluates: every
    /// `(quote X)' in it carries X itself, so the form reads and writes the
    /// identical structure (src/eval.c eval_sub returns the quoted object).
    pub(super) fn eval_datum_form(&self, value: Value) -> Result<Form> {
        self.datum_form(value, 0, Datum::Eval(0))
    }
    /// VALUE quoted as `eval' quotes it: the identical object.
    pub(super) fn eval_quoted_form(&self, value: Value) -> Form {
        Form::List(vec![Form::Sym("quote".into()), Form::Object(crate::handle::FormObject::new(self, value))])
    }
    fn datum_form(&self, value: Value, depth: usize, mode: Datum) -> Result<Form> {
        let objects = mode != Datum::Source;
        // GNU's eval has no nesting limit of its own, only the C stack: a
        // macro's expansion may nest deeper than any source does.
        if depth > 256 && !crate::eval_depth::native_stack_room() {
            return Err("source nesting limit exceeded".into());
        }
        if value == Value::NIL {
            return Ok(Form::Sym("nil".into()));
        }
        if value == Value::TRUE {
            return Ok(Form::Sym("t".into()));
        }
        match value.tag() {
            1 => Ok(Form::Int(value.as_integer()?)),
            7 => {
                let slot = value.index(7)? - 1;
                Ok(if let Some(&id) = self.generated_slots.get(&slot) {
                    Form::Generated(id)
                } else {
                    Form::Sym(self.symbol_names[slot].clone())
                })
            }
            0 => match self.object(value)? {
                Object::Bignum(_)
                | Object::Record(_)
                | Object::BoolVector(_)
                | Object::HashTable(_)
                | Object::CharTable(_)
                | Object::SubCharTable(_) => {
                    Ok(Form::Object(crate::handle::FormObject::new(self, value)))
                }
                Object::Float(n) => Ok(Form::Float(*n)),
                Object::Bytes(bytes) => Ok(Form::Bytes(bytes.clone())),
                Object::String(s) | Object::RawString(s) if !s.properties.is_empty() => {
                    Ok(Form::Object(crate::handle::FormObject::new(self, value)))
                }
                // A source literal's ASCII unibyte text is a cacheable
                // constant; other unibyte strings keep their identity.
                Object::String(s) if !s.is_multibyte() && mode == Datum::Source && s.bytes().is_ascii() => Ok(Form::Bytes(s.bytes().to_vec())),
                Object::String(s) | Object::RawString(s) if !s.is_multibyte() => Ok(Form::Object(crate::handle::FormObject::new(self, value))),
                Object::String(_) => Ok(Form::String(
                    String::from_utf8(self.string_bytes(value)?).map_err(|e| e.to_string())?,
                )),
                Object::RawString(_) => Ok(Form::Object(crate::handle::FormObject::new(self, value))),
                Object::Vector(a) => Ok(Form::Vector(
                    a.iter()
                        .map(|v| self.datum_form(*v, depth + 1, mode))
                        .collect::<Result<_>>()?,
                )),
                _ if objects => Ok(Form::Object(crate::handle::FormObject::new(self, value))),
                _ => Err("source datum required".into()),
            },
            3 => {
                let symbol_is = |name: &str, v: Value| self.names.get(name).is_some_and(|&slot| v == Value::reference(slot + 1, 7));
                let head = self.cons_pair(value).map(|(head, _)| head).unwrap_or(Value::NIL);
                if mode == Datum::Eval(0)
                    && symbol_is("quote", head)
                    && let Ok((_, rest)) = self.cons_pair(value)
                    && let Ok((quoted, Value::NIL)) = self.cons_pair(rest)
                    && (quoted.tag() == 3 || (quoted.tag() == 0 && quoted != Value::NIL))
                {
                    return Ok(self.eval_quoted_form(quoted));
                }
                // `(quote X)' returns X itself (eval.c Fquote): an X with no
                // tree spelling, one that contains itself (`#1=(a . #1#)'),
                // is carried as that object.
                if let Ok((head, rest)) = self.cons_pair(value)
                    && let Some(slot) = head.index_if(7).and_then(|s| s.checked_sub(1))
                    && self.symbol_names[slot] == "quote"
                    && !self.generated_slots.contains_key(&slot)
                    && !self.obarray_symbol_names.contains_key(&slot)
                    && let Ok((datum, Value::NIL)) = self.cons_pair(rest)
                    && !self.spellable(datum, depth + 1, &mut std::collections::HashSet::new())
                {
                    return Ok(Form::List(vec![
                        self.datum_form(head, depth + 1, mode)?,
                        Form::Object(crate::handle::FormObject::new(self, datum)),
                    ]));
                }
                // Inside a backquote template a quote may hold its commas;
                // a comma's operand is evaluated again.
                let mode = match mode {
                    Datum::Eval(level) if symbol_is("`", head) => Datum::Eval(level + 1),
                    Datum::Eval(level) if level > 0 && (symbol_is(",", head) || symbol_is(",@", head)) => Datum::Eval(level - 1),
                    mode => mode,
                };
                let mut mode = mode;
                let mut values = vec![];
                let mut tail = value;
                while tail.tag() == 3 {
                    let (car, cdr) = self.cons_pair(tail)?;
                    values.push(self.datum_form(car, depth + 1, mode)?);
                    // `(a . ,x)' reads as (a \, x): what follows is the operand.
                    if let Datum::Eval(level) = mode && level > 0 && values.len() > 1 && (symbol_is(",", car) || symbol_is(",@", car)) {
                        mode = Datum::Eval(level - 1);
                    }
                    tail = cdr;
                    if values.len() > self.list_limit() {
                        return Err("circular source list".into());
                    }
                }
                if tail == Value::NIL {
                    Ok(Form::List(values))
                } else {
                    Ok(Form::Dotted(
                        values,
                        Box::new(self.datum_form(tail, depth + 1, mode)?),
                    ))
                }
            }
            // Closures and primitives: function objects with no spelling.
            4 | 6 if objects => Ok(Form::Object(crate::handle::FormObject::new(self, value))),
            _ => Err("source datum required".into()),
        }
    }
    /// Whether VALUE has a tree spelling for `datum_form': no cons or
    /// vector within it contains itself, and nesting stays in its limit.
    /// OPEN holds the conses and vectors enclosing VALUE.
    fn spellable(&self, value: Value, depth: usize, open: &mut std::collections::HashSet<u64>) -> bool {
        if depth > 256 {
            return false;
        }
        let mut entered = vec![];
        let mut children = vec![];
        let mut tail = value;
        while tail.tag() == 3 {
            if !open.insert(tail.0) {
                entered.iter().for_each(|v| { open.remove(v); });
                return false;
            }
            entered.push(tail.0);
            let Ok((car, cdr)) = self.cons_pair(tail) else { break };
            children.push(car);
            tail = cdr;
        }
        if let Some(Object::Vector(items)) = self.object_if(tail) {
            if !open.insert(tail.0) {
                entered.iter().for_each(|v| { open.remove(v); });
                return false;
            }
            entered.push(tail.0);
            children.extend_from_slice(items);
        }
        let spelled = children.iter().all(|&child| self.spellable(child, depth + 1, open));
        entered.iter().for_each(|v| { open.remove(v); });
        spelled
    }
    /// Compile source as one progn for readable IR inspection, without executing.
    /// Source-file eval still executes top-level forms sequentially.
    pub fn compile_source(&mut self, source: &str) -> Result<usize> {
        let form = self.source_progn(source)?;
        self.compile(&form)
    }
    pub(super) fn request_source_ir(&mut self, source: &str) -> Result<String> {
        let outcome = (|| {
            let form = self.source_progn(source)?;
            let (id, _) = self.compile_request(&form)?;
            self.dump_ir(id)
        })();
        self.finish_request(outcome.as_ref().map(|_| Value::NIL).map_err(Clone::clone))?;
        outcome
    }
    fn source_progn(&mut self, mut source: &str) -> Result<Form> {
        let mut forms = vec![Form::Sym("progn".into())];
        while let Some((value, used)) = self.read_source_raw(source)? {
            forms.push(self.source_form(value, 0)?);
            source = &source[used..];
        }
        Ok(Form::List(forms))
    }
    /// Evaluate a UTF-8 source file sequentially in this persistent strand. No
    /// reader Values survive an executor safepoint: the compiler roots literals.
    ///
    /// This call may mark/compact even after a short request or error.
    /// Only the returned `Value` is rewritten for the embedder. Other raw heap
    /// `Value`s held by the caller must not be reused after this call. Short
    /// requests previously rarely collected; that was not a lifetime guarantee.
    /// Use `compile` and `execute_rooted` with mutable roots to retain such values.
    pub(crate) fn eval_source_raw(&mut self, source: &str) -> Result<Value> {
        let outcome = self.eval_source_inner(source);
        self.finish_request(outcome)
    }
    fn eval_source_inner(&mut self, source: &str) -> Result<Value> {
        if let Some(f) = self.cached_submission(source) {
            return self.execute_raw(f);
        }
        // The last form's value is rooted across reading the next one, which
        // can call Lisp (a `\N{NAME}' character).
        let last = self.root_frame(&[Value::NIL]);
        let outcome = self.eval_source_forms(source, &last);
        let [value] = self.release_frame(last);
        outcome.map(|()| value)
    }
    fn eval_source_forms(&mut self, mut source: &str, last: &crate::gc::RootFrame) -> Result<()> {
        while let Some((value, used)) = self.read_source_raw(source)? {
            let form = self.source_form(value, 0)?;
            let result = if let Some(result) = self.direct_form(&form) {
                result?
            } else {
                let function = self.cached_source_form(&source[..used], &form)?;
                self.execute_raw(function)?
            };
            self.set_rooted(last, 0, result);
            source = &source[used..];
        }
        Ok(())
    }
}

/// What a datum becomes source for: compiler source, a macro expansion
/// (objects with no spelling carried as themselves) or `eval''s datum
/// (quoted structure carried as itself too, outside backquote templates;
/// the level is the backquote nesting).
#[derive(Clone, Copy, PartialEq, Eq)]
enum Datum { Source, Expansion, Eval(u32) }
