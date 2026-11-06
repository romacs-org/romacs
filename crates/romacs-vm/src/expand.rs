//! Built-in surface forms lowered entirely in Rust, before IR emission.
//! Fresh temporaries are excluded from both source symbols and lexical bindings.
use crate::{Form, Result};
use std::sync::atomic::{AtomicU64, Ordering};

// A process-wide identity never reuses an ID, including across VMs.
static NEXT_TEMP: AtomicU64 = AtomicU64::new(0);

/// The next generated identity, which a world image records.
pub(crate) fn next_generated() -> u64 {
    NEXT_TEMP.load(Ordering::Relaxed)
}

/// Never hand out an identity below NEXT again: a restored world holds them.
pub(crate) fn raise_generated(next: u64) {
    NEXT_TEMP.fetch_max(next, Ordering::Relaxed);
}

fn s(name: &str) -> Form {
    Form::Sym(name.into())
}
fn l(items: Vec<Form>) -> Form {
    Form::List(items)
}
fn call(name: &str, args: Vec<Form>) -> Form {
    let mut items = vec![s(name)];
    items.extend(args);
    l(items)
}
fn progn(forms: &[Form]) -> Form {
    call("progn", forms.to_vec())
}
fn binding(name: Form, value: Form, body: Form) -> Form {
    call("let", vec![l(vec![l(vec![name, value])]), body])
}
struct Fresh;
pub(super) fn generated_id() -> crate::GeneratedName {
    crate::GeneratedName(
        NEXT_TEMP
            .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |n| n.checked_add(1))
            .expect("generated symbol identity exhausted"),
    )
}
impl Fresh {
    fn symbol(&mut self) -> Form {
        Form::Generated(generated_id())
    }
}

struct Place {
    bindings: Vec<Form>,
    read: Form,
    store: Form,
    value: Form,
}
fn place(f: &Form, fresh: &mut Fresh) -> Result<Place> {
    let value = fresh.symbol();
    if matches!(f, Form::Sym(_) | Form::Generated(_)) {
        return Ok(Place {
            bindings: vec![],
            read: f.clone(),
            store: call("setq", vec![f.clone(), value.clone()]),
            value,
        });
    }
    let a = f.list()?;
    let head = a.first().ok_or("empty place")?.sym()?;
    if head == "gethash" && (3..=4).contains(&a.len()) {
        // GNU's gv setter: (puthash KEY VALUE TABLE); DEFAULT only reads.
        let mut bindings = vec![];
        let mut temps = vec![];
        for arg in &a[1..] {
            let temp = fresh.symbol();
            bindings.push(l(vec![temp.clone(), arg.clone()]));
            temps.push(temp);
        }
        return Ok(Place {
            bindings,
            read: call(head, temps.clone()),
            store: call(
                "puthash",
                vec![temps[0].clone(), value.clone(), temps[1].clone()],
            ),
            value,
        });
    }
    let setter = match (head, a.len()) {
        ("car", 2) => "setcar",
        ("cdr", 2) => "setcdr",
        ("aref", 3) => "aset",
        _ => return Err("unsupported generalized place".into()),
    };
    let mut bindings = vec![];
    let mut temps = vec![];
    for arg in &a[1..] {
        let temp = fresh.symbol();
        bindings.push(l(vec![temp.clone(), arg.clone()]));
        temps.push(temp);
    }
    let read = call(head, temps.clone());
    temps.push(value.clone());
    Ok(Place {
        bindings,
        read,
        store: call(setter, temps),
        value,
    })
}
fn store(p: Place, value: Form) -> Form {
    let mut bindings = p.bindings;
    bindings.push(l(vec![p.value, value]));
    call("let*", vec![l(bindings), p.store])
}

fn quote(form: Form) -> Form {
    call("quote", vec![form])
}

/// What a part of a template expands to (backquote.el's tags 0, 1 and 2).
#[derive(Clone, Copy, PartialEq)]
pub(crate) enum Kind {
    Constant,
    Computed,
    Spliced,
}

pub(crate) struct Part {
    pub(crate) kind: Kind,
    pub(crate) form: Form,
}

/// A template `backquote' refuses.
pub(crate) enum BadTemplate {
    /// A comma of this spelling given more than one operand.
    Operands(&'static str, Form),
    /// A form ending in this dotted tail, which `length' or `eval' refuses.
    Dotted(Form),
    /// A constant `quote' of this many operands, which `eval' refuses.
    Quote(usize),
}

fn is_nil(form: &Form) -> bool {
    matches!(form, Form::Sym(name) if name == "nil") || matches!(form, Form::List(items) if items.is_empty())
}

fn nil() -> Form {
    s("nil")
}

/// `car-safe'.
fn head(form: &Form) -> Option<&Form> {
    match form {
        Form::List(items) | Form::Dotted(items, _) => items.first(),
        _ => None,
    }
}

/// `eq' on symbols.
fn same(form: &Form, symbol: &Form) -> bool {
    match (form, symbol) {
        (Form::Sym(a), Form::Sym(b)) => a == b,
        (Form::Generated(a), Form::Generated(b)) => a == b,
        _ => false,
    }
}

fn quoted(form: &Form) -> bool {
    head(form).is_some_and(|first| matches!(first, Form::Sym(name) if name == "quote"))
}

impl Part {
    fn constant(form: Form) -> Part {
        Part { kind: Kind::Constant, form }
    }
    /// FORM, constant when it is a quotation.
    fn of(form: Form) -> Part {
        Part { kind: if quoted(&form) { Kind::Constant } else { Kind::Computed }, form }
    }
}

/// What is left of a list: ITEMS, then TAIL unless that is nil. With no
/// ITEMS, TAIL is an atom of the template.
#[derive(Clone, Copy)]
struct Rest<'a> {
    items: &'a [Form],
    tail: Option<&'a Form>,
}

impl<'a> Rest<'a> {
    const END: Rest<'static> = Rest { items: &[], tail: None };

    fn of(form: &'a Form) -> Rest<'a> {
        match form {
            Form::List(items) => Rest { items, tail: None },
            Form::Dotted(items, tail) if items.is_empty() => Rest::of(tail),
            Form::Dotted(items, tail) => Rest { items, tail: (!is_nil(tail)).then_some(&**tail) },
            atom if is_nil(atom) => Rest::END,
            atom => Rest { items: &[], tail: Some(atom) },
        }
    }
    fn next(self) -> Rest<'a> {
        match self.items {
            [] | [_] => self.tail.map_or(Rest::END, Rest::of),
            [_, items @ ..] => Rest { items, tail: self.tail },
        }
    }
    fn form(self) -> Form {
        match self.tail {
            Some(tail) => Form::Dotted(self.items.to_vec(), Box::new(tail.clone())),
            None => l(self.items.to_vec()),
        }
    }
}

type Processed = std::result::Result<Part, BadTemplate>;

/// What a constant part evaluates to (`backquote-listify's `eval').
fn constant_value(form: Form) -> std::result::Result<Form, BadTemplate> {
    match form {
        Form::List(mut items) if matches!(items.first(), Some(Form::Sym(name)) if name == "quote") => {
            if items.len() != 2 { return Err(BadTemplate::Quote(items.len() - 1)); }
            Ok(items.pop().expect("two items"))
        }
        Form::Dotted(items, tail) if matches!(items.first(), Some(Form::Sym(name)) if name == "quote") => Err(BadTemplate::Dotted(*tail)),
        form => Ok(form),
    }
}

/// VALUES, last first, consed onto END.
fn prepended(values: Vec<Form>, end: Form) -> Form {
    if values.is_empty() { return end; }
    let mut items: Vec<Form> = values.into_iter().rev().collect();
    match end {
        Form::List(rest) => { items.extend(rest); Form::List(items) }
        Form::Dotted(rest, tail) => { items.extend(rest); Form::Dotted(items, tail) }
        end if is_nil(&end) => Form::List(items),
        end => Form::Dotted(items, Box::new(end)),
    }
}

/// `backquote-process' (backquote.el, whose expansions these equal;
/// DIVERGENCES.md VM2-BACKQUOTE): the symbols it is advertised to take from
/// `backquote-unquote-symbol' and its siblings.
pub(crate) struct Walk {
    pub(crate) unquote: Form,
    pub(crate) splice: Form,
    pub(crate) backquote: Form,
    /// Before backquote.el defines `backquote-list*': the conses it is.
    pub(crate) bare: bool,
}

impl Walk {
    pub(crate) fn standard() -> Walk {
        Walk { unquote: s(","), splice: s(",@"), backquote: s("`"), bare: false }
    }

    /// `backquote-listify': the code for PARTS, in template order, before
    /// TAIL: `list', `cons' or `backquote-list*' as the parts need, and the
    /// constants at the end folded into one quotation.
    fn listify(&self, parts: Vec<Part>, tail: Part) -> std::result::Result<Form, BadTemplate> {
        let constant_tail = tail.kind == Kind::Constant;
        let end = if constant_tail { constant_value(tail.form)? } else { tail.form };
        let (mut heads, mut values) = (vec![], vec![]);
        for part in parts.into_iter().rev() {
            if !heads.is_empty() || !constant_tail || part.kind != Kind::Constant {
                heads.push(part.form);
            } else {
                values.push(constant_value(part.form)?);
            }
        }
        heads.reverse();
        let end = prepended(values, end);
        if is_nil(&end) { return Ok(call("list", heads)); }
        let end = if constant_tail { quote(end) } else { end };
        if heads.is_empty() { return Ok(end); }
        let spread = heads.len() > 1 || head(&heads[0]).is_some_and(|first| same(first, &self.splice));
        if spread && self.bare {
            return Ok(heads.into_iter().rev().fold(end, |tail, first| call("cons", vec![first, tail])));
        }
        heads.push(end);
        Ok(call(if spread { "backquote-list*" } else { "cons" }, heads))
    }

    /// The operand of the comma form S.
    fn operand(s: Rest, comma: &'static str) -> std::result::Result<Form, BadTemplate> {
        if let Some(tail) = s.tail { return Err(BadTemplate::Dotted(tail.clone())); }
        if s.items.len() > 2 { return Err(BadTemplate::Operands(comma, s.form())); }
        Ok(s.items.get(1).cloned().unwrap_or_else(nil))
    }

    /// `backquote-delay-process': a comma or backquote kept for a later level.
    fn delayed(&self, s: Rest, level: i64) -> Processed {
        let first = Part::constant(quote(s.items[0].clone()));
        let rest = self.process(s.next(), level)?;
        Ok(Part::of(self.listify(vec![first], rest)?))
    }

    fn vector(&self, vector: &Form, items: &[Form], level: i64) -> Processed {
        let shown = self.process(Rest { items, tail: None }, level)?;
        if shown.kind == Kind::Constant { return Ok(Part::constant(vector.clone())); }
        let applied = |form| call("apply", vec![call("function", vec![s("vector")]), form]);
        let form = match shown.form {
            Form::List(mut items) if matches!(items.first(), Some(Form::Sym(name)) if name == "list" || name == "append") => {
                let vectored = if matches!(&items[0], Form::Sym(name) if name == "list") { "vector" } else { "vconcat" };
                items[0] = s(vectored);
                Form::List(items)
            }
            form @ (Form::List(_) | Form::Dotted(..)) => applied(form),
            form if is_nil(&form) => applied(form),
            form => call("vconcat", vec![form]),
        };
        Ok(Part { kind: Kind::Computed, form })
    }

    fn atom(&self, form: Option<&Form>, level: i64) -> Processed {
        match form {
            None => Ok(Part::constant(nil())),
            Some(whole @ Form::Vector(items)) => self.vector(whole, items, level),
            Some(Form::Sym(name)) if name == "t" => Ok(Part::constant(s("t"))),
            Some(symbol @ (Form::Sym(_) | Form::Generated(_))) => Ok(Part::constant(quote(symbol.clone()))),
            Some(other) => Ok(Part::constant(other.clone())),
        }
    }

    /// One list level: what is not spliced gathers into lists, and the
    /// lists join under `append'.
    fn list(&self, s: Rest, level: i64) -> Processed {
        let (mut rest, mut first, mut pending, mut lists) = (s, vec![], vec![], vec![]);
        // A comma or backquote in a cdr heads the rest of the list: `(a . ,b)'.
        while let Some(item) = rest.items.first() && !same(item, &self.unquote) && !same(item, &self.backquote) {
            let part = self.process(Rest::of(item), level)?;
            if part.kind == Kind::Spliced {
                if lists.is_empty() { first = std::mem::take(&mut pending); }
                if !pending.is_empty() { lists.push(self.listify(std::mem::take(&mut pending), Part::constant(nil()))?); }
                lists.push(part.form);
            } else {
                pending.push(part);
            }
            rest = rest.next();
        }
        if !rest.items.is_empty() || rest.tail.is_some() || !pending.is_empty() {
            let tail = self.process(rest, level)?;
            lists.push(self.listify(pending, tail)?);
        }
        let spliced = lists.last().and_then(head).is_some_and(|first| same(first, &self.splice));
        let mut expression = if lists.len() > 1 || spliced {
            lists.insert(0, self::s("append"));
            l(lists)
        } else {
            lists.pop().unwrap_or_else(nil)
        };
        if !first.is_empty() {
            expression = self.listify(first, Part { kind: Kind::Computed, form: expression })?;
        }
        Ok(Part::of(expression))
    }

    fn process(&self, s: Rest, level: i64) -> Processed {
        match s.items.first() {
            None => self.atom(s.tail, level),
            Some(first) if same(first, &self.unquote) => {
                if level <= 0 { Ok(Part::of(Walk::operand(s, ",")?)) } else { self.delayed(s, level - 1) }
            }
            Some(first) if same(first, &self.splice) => {
                if level <= 0 { Ok(Part { kind: Kind::Spliced, form: Walk::operand(s, ",@")? }) } else { self.delayed(s, level - 1) }
            }
            Some(first) if same(first, &self.backquote) => self.delayed(s, level + 1),
            Some(_) => self.list(s, level),
        }
    }

    /// STRUCTURE at nesting LEVEL.
    pub(crate) fn template(&self, structure: &Form, level: i64) -> Processed {
        self.process(Rest::of(structure), level)
    }
}

pub(crate) fn expand(f: &Form) -> Result<Option<Form>> {
    let Form::List(a) = f else { return Ok(None) };
    let Some(Form::Sym(head)) = a.first() else {
        return Ok(None);
    };
    let args = &a[1..];
    let mut fresh = Fresh;
    let expanded = match head.as_str() {
        // cus-face.el:37 custom-declare-face records SPEC as the face's
        // `face-defface-spec' and DOC as its `face-documentation'; GNU's
        // own `defface' then leaves a face so declared alone.
        "defface" if args.len() >= 3 => {
            let spec = fresh.symbol();
            binding(spec.clone(), args[1].clone(), progn(&[
                call("put", vec![quote(args[0].clone()), quote(s("face-defface-spec")), spec.clone()]),
                call("put", vec![quote(args[0].clone()), quote(s("face-documentation")), args[2].clone()]),
                call("#:vm2-defface", vec![quote(args[0].clone()), spec]),
            ]))
        }
        "with-current-buffer" if !args.is_empty() => binding(
            s("#:vm2-current-buffer"),
            call("current-buffer", vec![]),
            call("progn", {
                let mut body = vec![call("set-buffer", vec![args[0].clone()])];
                body.extend_from_slice(&args[1..]);
                body
            }),
        ),
        // GNU editfns.c:843 `save-current-buffer' (a special form): run
        // BODY, then make the buffer that was current, current again. VM2's
        // current buffer is the special slot `with-current-buffer' binds
        // above, so this is the same binding without the `set-buffer'.
        // Reached by GNU window.el (`with-current-buffer' in split-window-below).
        "save-current-buffer" => binding(
            s("#:vm2-current-buffer"),
            call("current-buffer", vec![]),
            progn(args),
        ),
        "declare" => s("nil"),
        "`" if args.len() == 1 => Walk { bare: true, ..Walk::standard() }.template(&args[0], 0)
            .map_err(|_| "malformed backquote template".to_string())?.form,
        "with-temp-buffer" => {
            let temp = fresh.symbol();
            binding(temp.clone(), call("#:vm2-temp-buffer", vec![]),
                call("unwind-protect", vec![
                    binding(s("#:vm2-current-buffer"), temp.clone(),
                        binding(s("font-lock-defaults"), s("nil"), progn(args))),
                    call("#:vm2-temp-buffer", vec![temp]),
                ]))
        }
        "when" | "unless" => {
            if args.is_empty() {
                return Err(format!("{head} needs a condition"));
            }
            let body = progn(&args[1..]);
            if head == "when" {
                call("if", vec![args[0].clone(), body])
            } else {
                call("if", vec![args[0].clone(), s("nil"), body])
            }
        }
        "and" | "or" => {
            if args.is_empty() {
                s(if head == "and" { "t" } else { "nil" })
            } else if args.len() == 1 {
                args[0].clone()
            } else {
                let tail = call(head, args[1..].to_vec());
                if head == "and" {
                    call("if", vec![args[0].clone(), tail])
                } else {
                    let tmp = fresh.symbol();
                    binding(
                        tmp.clone(),
                        args[0].clone(),
                        call("if", vec![tmp.clone(), tmp, tail]),
                    )
                }
            }
        }
        "cond" => {
            // eval.c Fcond: Fcar of a clause that is not a list signals
            // listp when it is reached; a dotted clause's body is Fprogn of
            // its list part (a bare (TEST . X) gives nil).
            let mut tail = s("nil");
            for clause in args.iter().rev() {
                let dotted;
                let items = match clause {
                    Form::Dotted(items, _) if items.len() == 1 => { dotted = vec![items[0].clone(), s("nil")]; &dotted[..] }
                    Form::Dotted(items, _) => &items[..],
                    _ => match clause.list() {
                        Ok(items) => items,
                        Err(_) => {
                            tail = crate::let_forms::signal("wrong-type-argument", vec![s("listp"), clause.clone()]);
                            continue;
                        }
                    },
                };
                if items.is_empty() {
                    continue;
                }
                tail = if items.len() == 1 {
                    let tmp = fresh.symbol();
                    binding(
                        tmp.clone(),
                        items[0].clone(),
                        call("if", vec![tmp.clone(), tmp, tail]),
                    )
                } else {
                    call("if", vec![items[0].clone(), progn(&items[1..]), tail])
                };
            }
            tail
        }
        "prog1" | "prog2" => {
            let index = usize::from(head == "prog2");
            if args.len() <= index && head == "prog1" {
                // eval_sub: the special form's own arity error.
                let data = Form::List(vec![Form::Sym("prog1".into()), Form::Int(0)]);
                return Ok(Some(call("signal", vec![quote(Form::Sym("wrong-number-of-arguments".into())), quote(data)])));
            }
            if args.len() <= index {
                return Err(format!("{head} has too few arguments"));
            }
            let tmp = fresh.symbol();
            let mut body = args[index + 1..].to_vec();
            body.push(tmp.clone());
            let saved = binding(tmp, args[index].clone(), progn(&body));
            if index == 0 {
                saved
            } else {
                call("progn", vec![args[0].clone(), saved])
            }
        }
        "dotimes" | "dolist" => {
            if args.is_empty() {
                return Err(format!("{head} needs a binding"));
            }
            let spec = args[0].list()?;
            if !(2..=3).contains(&spec.len()) {
                return Err(format!("bad {head} binding"));
            }
            spec[0].name()?;
            let var = spec[0].clone();
            let tmp = fresh.symbol();
            let mut body = args[1..].to_vec();
            let result = spec.get(2).cloned().unwrap_or_else(|| s("nil"));
            if head == "dotimes" {
                let counter = fresh.symbol();
                let iteration = binding(var.clone(), counter.clone(), progn(&body));
                let advance = call(
                    "setq",
                    vec![counter.clone(), call("1+", vec![counter.clone()])],
                );
                let loop_form = call(
                    "while",
                    vec![
                        call("<", vec![counter.clone(), tmp.clone()]),
                        iteration,
                        advance,
                    ],
                );
                let finish = binding(var, counter.clone(), result);
                binding(
                    tmp,
                    spec[1].clone(),
                    binding(
                        counter,
                        Form::Int(0),
                        call("progn", vec![loop_form, finish]),
                    ),
                )
            } else {
                body.push(call(
                    "setq",
                    vec![tmp.clone(), call("cdr", vec![tmp.clone()])],
                ));
                let iteration = binding(var, call("car", vec![tmp.clone()]), progn(&body));
                let loop_form = call("while", vec![tmp.clone(), iteration]);
                // GNU 32.0.50's dolist result is outside the iteration binding.
                binding(tmp, spec[1].clone(), call("progn", vec![loop_form, result]))
            }
        }
        "funcall"
            if args.len() == 3
                && matches!(&args[0], Form::List(q) if q.len() == 2
                    && matches!(&q[0], Form::Sym(h) if h == "function" || h == "quote")
                    && matches!(&q[1], Form::Sym(n) if n == "maphash")) =>
        {
            // GNU's byte compiler turns (funcall #'maphash F TABLE) into a
            // direct call; lower it to the same slot walk.
            call("maphash", args[1..].to_vec())
        }
        "maphash" if args.len() == 2 => {
            // A primitive cannot call Lisp, so the slot walk of GNU's
            // DOHASH_SAFE is compiled inline. Slot indices are integers and
            // survive collection inside FUNCTION; primitive calls never
            // collect, so each key/value pair is read atomically.
            let (function, table, at, slot) = (
                fresh.symbol(),
                fresh.symbol(),
                fresh.symbol(),
                fresh.symbol(),
            );
            let step = call(
                "while",
                vec![
                    call(
                        "setq",
                        vec![
                            slot.clone(),
                            call("#:vm2-hash-next", vec![table.clone(), at.clone()]),
                        ],
                    ),
                    call("setq", vec![at.clone(), call("1+", vec![slot.clone()])]),
                    call(
                        "funcall",
                        vec![
                            function.clone(),
                            call("#:vm2-hash-key", vec![table.clone(), slot.clone()]),
                            call("#:vm2-hash-value", vec![table.clone(), slot.clone()]),
                        ],
                    ),
                ],
            );
            binding(
                function,
                args[0].clone(),
                binding(
                    table,
                    args[1].clone(),
                    binding(
                        at,
                        Form::Int(0),
                        binding(slot, s("nil"), call("progn", vec![step, s("nil")])),
                    ),
                ),
            )
        }
        "push" => {
            if args.len() != 2 {
                return Err("push needs value and place".into());
            }
            if !matches!(args[1], Form::Sym(_) | Form::Generated(_)) {
                let value = fresh.symbol();
                let p = place(&args[1], &mut fresh)?;
                let update = call("cons", vec![value.clone(), p.read.clone()]);
                return Ok(Some(binding(value, args[0].clone(), store(p, update))));
            }
            call(
                "setq",
                vec![
                    args[1].clone(),
                    call("cons", vec![args[0].clone(), args[1].clone()]),
                ],
            )
        }
        "pop" => {
            if args.len() != 1 {
                return Err("pop needs a place".into());
            }
            if !matches!(args[0], Form::Sym(_) | Form::Generated(_)) {
                let saved = fresh.symbol();
                let p = place(&args[0], &mut fresh)?;
                let mut bindings = p.bindings;
                bindings.push(l(vec![saved.clone(), p.read]));
                bindings.push(l(vec![p.value, call("cdr", vec![saved.clone()])]));
                return Ok(Some(call(
                    "let*",
                    vec![l(bindings), p.store, call("car", vec![saved])],
                )));
            }
            call(
                "prog1",
                vec![
                    call("car", vec![args[0].clone()]),
                    call(
                        "setq",
                        vec![args[0].clone(), call("cdr", vec![args[0].clone()])],
                    ),
                ],
            )
        }
        "setf" => {
            if !args.len().is_multiple_of(2) {
                return Err("setf needs place/value pairs".into());
            }
            let mut forms = vec![];
            for pair in args.chunks_exact(2) {
                let p = place(&pair[0], &mut fresh)?;
                forms.push(store(p, pair[1].clone()));
            }
            progn(&forms)
        }
        "cl-incf" | "cl-decf" => {
            if !(1..=2).contains(&args.len()) {
                return Err(format!("bad {head} arguments"));
            }
            let p = place(&args[0], &mut fresh)?;
            let delta = args.get(1).cloned().unwrap_or(Form::Int(1));
            let value = call(
                if head == "cl-incf" { "+" } else { "-" },
                vec![p.read.clone(), delta],
            );
            store(p, value)
        }
        _ => return Ok(None),
    };
    Ok(Some(expanded))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn generated_names_are_unique_across_expansions() {
        let form = call("or", vec![Form::Int(1), Form::Int(2)]);
        let first = expand(&form).unwrap().unwrap();
        let second = expand(&form).unwrap().unwrap();
        assert_ne!(
            format!("{first:?}"),
            format!("{second:?}"),
            "separate expansions must not reuse generated identities"
        );
    }

    /// Before backquote.el, `backquote-list*' is not there to call.
    #[test]
    fn a_bare_vm_builds_a_dotted_template_from_conses() {
        let mut vm = crate::Vm::new();
        let value = vm.eval_source("(let ((a 1) (b 2) (c 3)) `(,a ,b . ,c))").unwrap();
        assert_eq!(value.display().unwrap(), "(1 2 . 3)");
    }

    #[test]
    fn generated_namespace_is_disjoint_from_all_source_strings() {
        use crate::{Name, Vm};
        let Form::Generated(id) = Fresh.symbol() else {
            panic!("typed identity required")
        };
        for text in [
            format!("#:vm2-temp-{id}"),
            format!("Generated({id})"),
            format!("\0vm2-temp-{id}"),
            format!("g{id}"),
        ] {
            let source = Form::from_json(serde_json::json!(text)).unwrap();
            assert_ne!(source.name().unwrap(), Name::Generated(id));
            // Escaping makes every character (including # and NUL) reader-producible.
            let escaped = text.chars().map(|c| format!("\\{c}")).collect::<String>();
            let mut vm = Vm::new();
            let (value, _) = vm.read_source_raw(&escaped).unwrap().unwrap();
            let source = vm.source_form(value, 0).unwrap();
            assert_ne!(source.name().unwrap(), Name::Generated(id));
        }
    }

    #[test]
    fn generated_bindings_survive_user_macro_roundtrips() {
        let mut vm = crate::Vm::new();
        vm.eval_source_raw("(defmacro + (a b) (list '- a b))").unwrap();
        let value = vm
            .eval_source_raw("(let ((x (cons 5 nil))) (cl-incf (car x) 2))")
            .unwrap();
        assert_eq!(value.as_integer().unwrap(), 3);
    }

    #[test]
    fn generated_literals_remain_uninterned_across_macro_data_and_images() {
        use crate::{ImageRegion, Name, Value, Vm};
        let id = generated_id();
        let mut vm = Vm::new();
        let generated = vm.literal(&Form::Generated(id)).unwrap();
        let spelling = vm.display(generated).unwrap();
        assert_ne!(generated, vm.intern(&spelling));
        assert_eq!(
            vm.source_form(generated, 0).unwrap().name().unwrap(),
            Name::Generated(id)
        );
        let slot = vm.slot("saved-temporary");
        vm.slots[slot].value = generated;
        let entry = vm.compile_source("saved-temporary").unwrap();
        let store = std::env::temp_dir().join(format!("vm2-generated-{}-{id}", std::process::id()));
        std::fs::create_dir_all(&store).unwrap();
        let config = [17; 32];
        let key = vm.publish_code_image(&store, entry, config).unwrap();
        let image = unsafe { ImageRegion::open(&store, &key) }.unwrap();
        let (mut restored, entry) = Vm::from_code_image(image, config).unwrap();
        let value = restored.execute_raw(entry).unwrap();
        assert_ne!(value, Value::NIL);
        assert_eq!(restored.display(value).unwrap(), spelling);
        assert_ne!(value, restored.intern(&spelling));
        let Name::Generated(new_id) = restored.source_form(value, 0).unwrap().name().unwrap()
        else {
            panic!("identity must survive image restore")
        };
        assert_ne!(
            new_id, id,
            "restoration must acquire a fresh process identity"
        );
        assert_eq!(restored.literal(&Form::Generated(new_id)).unwrap(), value);
        std::fs::remove_dir_all(store).unwrap();
    }

    // Under dynamic scope a Lisp uninterned symbol binds dynamically, but a
    // compiler temporary is no symbol: compiling a form that needs one again
    // and again must not make symbol slots.
    #[test]
    fn dynamic_scope_temporaries_take_no_symbol_slots() {
        let mut vm = crate::Vm::new();
        let run = |vm: &mut crate::Vm| { vm.eval_source("(dotimes (i 50) (eval (list 'or (list 'car-safe i) i) nil))").unwrap(); };
        run(&mut vm);
        let slots = vm.slots.len();
        run(&mut vm);
        assert_eq!(vm.slots.len(), slots);
        assert_eq!(vm.eval_source("(let ((s (make-symbol \"u\"))) (eval (list 'let (list (list s 5)) (list 'symbol-value (list 'quote s))) nil))").unwrap().display().unwrap(), "5");
    }

    #[test]
    fn nested_expansions_preserve_bindings_and_closure_captures() {
        let mut vm = crate::Vm::new();
        let value = vm.eval_source_raw("(let ((vm2-temp-0 19)) (+ (or nil (or nil vm2-temp-0)) (prog1 23 (funcall (lambda () vm2-temp-0)))))").unwrap();
        assert_eq!(value.as_integer().unwrap(), 42);
        // JSON symbols need not obey reader syntax; hostile old-prefix names remain ordinary.
        let form = Form::from_json(serde_json::json!([
            "let",
            [["#:vm2-temp-0", 42]],
            ["or", "nil", "#:vm2-temp-0"]
        ]))
        .unwrap();
        let f = vm.compile(&form).unwrap();
        assert_eq!(vm.execute_raw(f).unwrap().as_integer().unwrap(), 42);
    }
}
