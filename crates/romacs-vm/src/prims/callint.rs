//! The command-loop entry points of GNU src/callint.c (`call-interactively`,
//! `funcall-interactively`, `prefix-numeric-value`) and data.c's
//! `interactive-form`, which `call-interactively` calls through its symbol.
//!
//! All of them may run Lisp, so they are T2 call-request state machines
//! (PRIMITIVE-DOCTRINE section 3): every Value needed after a callback is a
//! saved register of the continuation activation, never a Rust local.
use super::*;
use crate::prim::leaf::Kind;

fn symbolp(cx: &Leaf<'_>, v: Value) -> bool {
    v == Value::NIL || v == Value::TRUE || cx.kind(v) == Kind::Symbol
}
fn fixnum(n: i64) -> Value {
    Value::integer(n).expect("fixnum")
}
fn index(v: Value) -> usize {
    v.integer_value().expect("index register") as usize
}
fn done(_: &mut Leaf<'_>, value: Value, _: &[Value]) -> Next {
    Ok(Step::Done(value))
}

/// GNU `indirect_function` (data.c): follow symbol function cells; a void
/// cell ends the chain as nil. `fset`/`defalias` refuse cycles; the bound
/// only keeps a cycle VM2 could not build from becoming a hang.
fn indirect(cx: &Leaf<'_>, mut f: Value) -> std::result::Result<Value, Fault> {
    for _ in 0..=cx.list_limit() {
        if f == Value::NIL || !symbolp(cx, f) {
            return Ok(f);
        }
        f = cx.symbol_function(f)?;
    }
    Err(Fault::CyclicFunctionIndirection(f))
}

/// GNU `Fget` on a symbol's own plist (fns.c `plist_get`, eq keys).
fn get(cx: &Leaf<'_>, symbol: Value, property: Value) -> std::result::Result<Value, Fault> {
    let mut tail = cx.symbol_plist(symbol)?;
    for _ in 0..=cx.list_limit() {
        let Some((key, rest)) = cx.pair(tail) else { return Ok(Value::NIL) };
        let Some((value, rest)) = cx.pair(rest) else { return Ok(Value::NIL) };
        if key == property {
            return Ok(value);
        }
        tail = rest;
    }
    Ok(Value::NIL)
}

/// GNU `Fcar`/`Fcdr`: nil gives nil, a non-list signals `listp`.
fn car(cx: &Leaf<'_>, v: Value) -> std::result::Result<Value, Fault> {
    if v == Value::NIL { return Ok(v) }
    cx.pair(v).map(|p| p.0).ok_or(Fault::Wrong("listp", v))
}
fn cdr(cx: &Leaf<'_>, v: Value) -> std::result::Result<Value, Fault> {
    if v == Value::NIL { return Ok(v) }
    cx.pair(v).map(|p| p.1).ok_or(Fault::Wrong("listp", v))
}

/// The elements of a proper list, as `Fapply` spreads them (eval.c
/// `list_length`): a non-list value signals `(wrong-type-argument listp
/// VALUE)`, a dotted list names its final tail (recorded: `'(1 . 2)` signals
/// `(wrong-type-argument listp 2)`), a cycle signals `(circular-list LIST)`.
fn proper_list(cx: &Leaf<'_>, list: Value) -> std::result::Result<Vec<Value>, Fault> {
    let mut out = Vec::new();
    let mut tail = list;
    while let Some((head, rest)) = cx.pair(tail) {
        out.push(head);
        if out.len() > cx.list_limit() {
            return Err(Fault::Circular(list));
        }
        tail = rest;
    }
    if tail != Value::NIL {
        return Err(Fault::Wrong("listp", tail));
    }
    Ok(out)
}

fn list(cx: &mut Leaf<'_>, items: &[Value]) -> Value {
    items.iter().rev().fold(Value::NIL, |tail, &v| cx.cons(v, tail))
}

// ---------------------------------------------------------------------------
// prefix-numeric-value

defsubr! { name = "prefix-numeric-value", gnu = "callint.c:803", effect = Pure,
fn prefix_numeric_value(cx, raw: Value) {
    Ok(prefix_numeric(cx, raw))
}}

/// GNU `Fprefix_numeric_value` (callint.c:810-823).
fn prefix_numeric(cx: &mut Leaf<'_>, raw: Value) -> Value {
    if raw == Value::NIL {
        return fixnum(1);
    }
    if raw == cx.intern("-") {
        return fixnum(-1);
    }
    if let Some((head, _)) = cx.pair(raw) {
        if head.integer_value().is_some() {
            return head;
        }
    } else if raw.integer_value().is_some() {
        return raw;
    }
    fixnum(1)
}

// ---------------------------------------------------------------------------
// funcall-interactively

// GNU's body is `Ffuncall' inside a specpdl frame; `called-interactively-p'
// (subr.el) finds the frame by walking the backtrace. VM2 has no backtrace
// frames, so the mark itself is not observable (DIVERGENCES VM2-CALLINT1).
defsubr! { name = "funcall-interactively", gnu = "callint.c:239", effect = Global,
fn funcall_interactively(cx, function: Value; rest args) -> Step {
    Ok(Step::Call { f: function, args: args.to_vec(), then: done, saved: vec![] })
}}

// ---------------------------------------------------------------------------
// interactive-form

defsubr! { name = "interactive-form", gnu = "data.c:1134", effect = Global,
fn interactive_form(cx, cmd: Value) -> Step {
    form_of(cx, cmd)
}}

/// GNU `Finteractive_form` (data.c:1139-1210).
fn form_of(cx: &mut Leaf<'_>, cmd: Value) -> Next {
    if indirect(cx, cmd)? == Value::NIL {
        return Ok(Step::Done(Value::NIL));
    }
    // An `interactive-form' property anywhere on the alias chain wins.
    let property = cx.intern("interactive-form");
    let mut fun = cmd;
    for _ in 0..=cx.list_limit() {
        if !symbolp(cx, fun) {
            break;
        }
        let spec = get(cx, fun, property)?;
        if spec != Value::NIL {
            return Ok(Step::Done(spec));
        }
        fun = cx.symbol_function(fun)?;
    }
    let interactive = cx.intern("interactive");
    match cx.kind(fun) {
        Kind::Subr => match subr_intspec(fun) {
            None => Ok(Step::Done(Value::NIL)),
            Some(None) => Ok(Step::Done(list(cx, &[interactive, Value::NIL]))),
            Some(Some(spec)) => {
                let spec = if spec.starts_with('(') {
                    let text = cx.string(spec);
                    let read = cx.read_from_string(text, Value::NIL, Value::NIL)?;
                    cx.pair(read).expect("read-from-string returns a cons").0
                } else {
                    cx.string(spec)
                };
                Ok(Step::Done(list(cx, &[interactive, spec])))
            }
        },
        // GNU's CLOSURE_INTERACTIVE slot; VM2 closures carry it in the
        // compiler's registration table. A closure created without an
        // `interactive' form, and OClosures (none exist in VM2), give nil.
        Kind::Closure => Ok(Step::Done(match cx.interactive_spec(fun) {
            Some((spec, _)) => list(cx, &[interactive, spec]),
            None => Value::NIL,
        })),
        Kind::Cons => {
            let head = cx.pair(fun).expect("cons").0;
            if head == cx.intern("autoload") {
                // Finteractive_form (Fautoload_do_load (fun, cmd, Qnil)).
                let f = cx.intern("autoload-do-load");
                return Ok(Step::Call { f, args: vec![fun, cmd, Value::NIL], then: loaded, saved: vec![] });
            }
            if head == cx.intern("lambda") {
                let body = cdr(cx, cdr(cx, fun)?)?;
                let spec = assq(cx, interactive, body)?;
                if cdr(cx, cdr(cx, spec)?)? == Value::NIL {
                    return Ok(Step::Done(spec));
                }
                let spec = car(cx, cdr(cx, spec)?)?;
                return Ok(Step::Done(list(cx, &[interactive, spec])));
            }
            Ok(Step::Done(Value::NIL))
        }
        _ => Ok(Step::Done(Value::NIL)),
    }
}

fn loaded(cx: &mut Leaf<'_>, definition: Value, _: &[Value]) -> Next {
    form_of(cx, definition)
}

/// GNU `Fassq`: non-cons elements are skipped; a dotted tail signals `listp'
/// with the whole list (CHECK_LIST_END), a cycle `circular-list'.
fn assq(cx: &Leaf<'_>, key: Value, alist: Value) -> std::result::Result<Value, Fault> {
    let mut tail = alist;
    for _ in 0..=cx.list_limit() {
        let Some((element, rest)) = cx.pair(tail) else {
            return if tail == Value::NIL { Ok(Value::NIL) } else { Err(Fault::Wrong("listp", alist)) };
        };
        if cx.pair(element).is_some_and(|(k, _)| k == key) {
            return Ok(element);
        }
        tail = rest;
    }
    Err(Fault::Circular(alist))
}

/// The interactive specs of the VM2 subrs that GNU defines as commands. The
/// strings are GNU's own DEFUN intspec arguments (each recorded live through
/// `interactive-form`, evidence `interactive-form.json`); `delete-backward-char`
/// and `emacs-lisp-mode` are Lisp in GNU (simple.el, elisp-mode.el) and VM2
/// stand-ins here, with GNU's Lisp spec. GNU subrs not listed are not commands.
fn subr_intspec(subr: Value) -> Option<Option<&'static str>> {
    Some(Some(match crate::prim::name(subr)? {
        "forward-char" | "backward-char" | "beginning-of-line" | "end-of-line" | "forward-line" => "^p",
        "delete-char" | "delete-backward-char" => "p\nP",
        "self-insert-command" => "(list (prefix-numeric-value current-prefix-arg) last-command-event)",
        "goto-char" => "(goto-char--read-natnum-interactive \"Go to char: \")",
        "delete-region" | "narrow-to-region" | "base64-encode-region" | "base64url-encode-region" | "base64-decode-region" => "r",
        "widen" | "garbage-collect" | "recursive-edit" | "top-level" | "exit-recursive-edit" | "abort-recursive-edit" => "",
        "erase-buffer" => "*",
        "search-forward" => "MSearch: ",
        "re-search-forward" => "sRE search: ",
        "re-search-backward" => "sRE search backward: ",
        "modify-syntax-entry" => "cSet syntax for character: \nsSet syntax for %s to: ",
        "make-variable-buffer-local" => "vMake Variable Buffer Local: ",
        "make-local-variable" => "vMake Local Variable: ",
        // Lisp in GNU: (interactive) with no spec.
        "emacs-lisp-mode" => return Some(None),
        _ => return None,
    }))
}

// ---------------------------------------------------------------------------
// call-interactively

// Saved registers of the call-interactively walk.
const FUNCTION: usize = 0;
const RECORD: usize = 1;
const KEYS: usize = 2; // the KEYS vector, or nil for this-command-keys
const PREFIX: usize = 3; // current-prefix-arg at entry
const SAVED_VARS: usize = 4; // this-command, this-original-command, real-this-command, last-command
const SPEC: usize = 8;
const POS: usize = 9; // byte offset into SPEC
const NEXT_EVENT: usize = 10;
const ARGS: usize = 11; // reversed argument list
const HISTORY: usize = 12; // reversed command-history items
const FLAGS: usize = 13; // RECORD_THEN_FAIL | ARG_FROM_TTY
const POSITION_ARGS: usize = 14;
const WIDTH: usize = 15;
const RECORD_THEN_FAIL: i64 = 1;
const ARG_FROM_TTY: i64 = 2;
const COMMAND_VARS: [&str; 4] = ["this-command", "this-original-command", "real-this-command", "last-command"];

defsubr! { name = "call-interactively", gnu = "callint.c:256", effect = Global,
fn call_interactively(cx, function: Value; optional record_flag: Value, keys: Value) -> Step {
    // callint.c:282-299, in GNU's order: save the command variables, check
    // KEYS (CHECK_VECTOR), take the prefix, then ask `interactive-form'
    // through its symbol (so advice on it applies).
    let mut saved = vec![Value::NIL; WIDTH];
    for (i, name) in COMMAND_VARS.iter().enumerate() {
        let symbol = cx.intern(name);
        saved[SAVED_VARS + i] = cx.variable_value(symbol)?.unwrap_or(Value::NIL);
    }
    if keys != Value::NIL && cx.kind(keys) != Kind::Vector {
        return Err(Fault::Wrong("vectorp", keys));
    }
    let prefix = cx.intern("current-prefix-arg");
    saved[FUNCTION] = function;
    saved[RECORD] = record_flag;
    saved[KEYS] = keys;
    saved[PREFIX] = cx.variable_value(prefix)?.unwrap_or(Value::NIL);
    saved[POS] = fixnum(0);
    saved[NEXT_EVENT] = fixnum(0);
    saved[FLAGS] = fixnum(0);
    let f = cx.intern("interactive-form");
    Ok(Step::Call { f, args: vec![function], then: got_form, saved })
}}

fn got_form(cx: &mut Leaf<'_>, form: Value, saved: &[Value]) -> Next {
    let function = saved[FUNCTION];
    if cx.pair(form).is_none() {
        return Err(Fault::Wrong("commandp", function));
    }
    let specs = car(cx, cdr(cx, form)?)?;
    let mut saved = saved.to_vec();
    if !matches!(cx.kind(specs), Kind::String | Kind::Bytes) {
        // callint.c:309-340: evaluate the spec. GNU evaluates it in an
        // interpreted closure's lexical environment; VM2 calls the closure's
        // registered spec thunk, compiled in that same environment, when the
        // spec is the closure's own. Any other spec (an `interactive-form'
        // property, a lambda list, a subr's) is evaluated as GNU does for a
        // non-closure: `(eval SPEC nil)'.
        let target = indirect(cx, function)?;
        if cx.kind(target) == Kind::Closure {
            if let Some((own, thunk)) = cx.interactive_spec(target) {
                if own == specs && thunk != Value::NIL {
                    return Ok(Step::Call { f: thunk, args: vec![], then: got_values, saved });
                }
            }
        }
        let eval = cx.intern("eval");
        return Ok(Step::Call { f: eval, args: vec![specs, Value::NIL], then: got_values, saved });
    }
    // callint.c:343-349 copies the string first, so a prefix callback (`^')
    // that edits the spec cannot change the codes still to be read.
    let multibyte = cx.kind(specs) == Kind::String;
    let bytes = cx.string_bytes(specs)?;
    saved[SPEC] = cx.make_string(bytes, multibyte);
    prefix_codes(cx, saved)
}

/// The arguments of a non-string spec. RECORD-FLAG puts the call on
/// `command-history' (callint.c:322-333; no input events exist in VM2, so the
/// other condition never holds), then the command runs through the symbol
/// `funcall-interactively' as `Fapply' would.
fn got_values(cx: &mut Leaf<'_>, values: Value, saved: &[Value]) -> Next {
    let mut saved = saved.to_vec();
    let args = proper_list(cx, values)?;
    saved[ARGS] = list(cx, &args.iter().rev().copied().collect::<Vec<_>>());
    if saved[RECORD] != Value::NIL {
        let mut quoted: Vec<Value> = args.iter().map(|&a| quotify(cx, a)).collect();
        fix_command(cx, saved[FUNCTION], &mut quoted)?;
        quoted.insert(0, saved[FUNCTION]);
        let item = list(cx, &quoted);
        return add_to_history(cx, item, saved);
    }
    run_command(cx, &saved)
}

/// GNU `fix_command` (callint.c:175-197): an `interactive-args' alist on the
/// command's symbol replaces recorded values by index.
fn fix_command(cx: &mut Leaf<'_>, function: Value, values: &mut [Value]) -> std::result::Result<(), Fault> {
    if values.is_empty() || !symbolp(cx, function) {
        return Ok(());
    }
    let key = cx.intern("interactive-args");
    let reps = get(cx, function, key)?;
    if cx.pair(reps).is_none() {
        return Ok(());
    }
    for (i, value) in values.iter_mut().enumerate() {
        let rep = assq(cx, fixnum(i as i64), reps)?;
        if let Some((_, form)) = cx.pair(rep) {
            *value = form;
        }
    }
    Ok(())
}

/// GNU `quotify_arg` (callint.c:127-135).
fn quotify(cx: &mut Leaf<'_>, v: Value) -> Value {
    if cx.pair(v).is_some() || (symbolp(cx, v) && v != Value::NIL && v != Value::TRUE) {
        let quote = cx.intern("quote");
        list(cx, &[quote, v])
    } else {
        v
    }
}

fn add_to_history(cx: &mut Leaf<'_>, item: Value, saved: Vec<Value>) -> Next {
    // calln (Qadd_to_history, Qcommand_history, ITEM, Qnil, Qt).
    let f = cx.intern("add-to-history");
    let history = cx.intern("command-history");
    if cx.symbol_function(f)? != Value::NIL {
        return Ok(Step::Call { f, args: vec![history, item, Value::NIL, Value::TRUE], then: recorded, saved });
    }
    // add-to-history is subr.el Lisp, not yet loadable in VM2; this is its
    // body for KEEP-ALL t (subr.el:2703-2735).
    keep_all_history(cx, history, item)?;
    recorded(cx, Value::NIL, &saved)
}

fn keep_all_history(cx: &mut Leaf<'_>, var: Value, item: Value) -> std::result::Result<(), Fault> {
    let key = cx.intern("history-length");
    let mut maxelt = get(cx, var, key)?;
    if maxelt == Value::NIL {
        maxelt = cx.variable_value(key)?.unwrap_or(Value::NIL);
    }
    let history = cx.variable_value(var)?.ok_or(Fault::VoidVariable(var))?;
    if history != Value::NIL && cx.pair(history).is_none() {
        return Ok(());
    }
    let duplicates = cx.intern("history-delete-duplicates");
    if cx.variable_value(duplicates)?.unwrap_or(Value::NIL) != Value::NIL {
        return Err(unsupported("history-delete-duplicates"));
    }
    let mut history = cx.cons(item, history);
    if let Some(max) = maxelt.integer_value() {
        if max <= 0 {
            history = Value::NIL;
        } else {
            let mut tail = history;
            for _ in 1..max {
                match cx.pair(tail) {
                    Some((_, rest)) => tail = rest,
                    None => break,
                }
            }
            if cx.pair(tail).is_some() {
                cx.setcdr(tail, Value::NIL)?;
            }
        }
    }
    cx.variable_set(var, history)
}

fn recorded(cx: &mut Leaf<'_>, _: Value, saved: &[Value]) -> Next {
    if index(saved[FLAGS]) as i64 & RECORD_THEN_FAIL != 0 {
        barf_if_read_only(cx)?;
    }
    run_command(cx, saved)
}

/// Restore the command variables (callint.c:792-795) and call
/// `funcall-interactively' through its symbol with the arguments.
fn run_command(cx: &mut Leaf<'_>, saved: &[Value]) -> Next {
    for (i, name) in COMMAND_VARS.iter().enumerate() {
        let symbol = cx.intern(name);
        cx.variable_set(symbol, saved[SAVED_VARS + i])?;
    }
    let mut args = vec![saved[FUNCTION]];
    let mut tail = saved[ARGS];
    let mut reversed = Vec::new();
    let mut positions = saved[POSITION_ARGS];
    while let Some((head, rest)) = cx.pair(tail) {
        let (position, next) = cx.pair(positions).unwrap_or((Value::NIL, Value::NIL));
        reversed.push((head, position != Value::NIL));
        positions = next;
        tail = rest;
    }
    for (value, position) in reversed.into_iter().rev() {
        let value = if position {
            match cx.marker_position(value) {
                Some(Some(position)) => fixnum(position),
                _ => value,
            }
        } else { value };
        args.push(value);
    }
    let f = cx.intern("funcall-interactively");
    Ok(Step::Call { f, args, then: done, saved: vec![] })
}

/// GNU `Fbarf_if_buffer_read_only (Qnil)` (buffer.c:2471-2482). VM2 has no
/// `inhibit-read-only' text property support, so only the variables count.
fn barf_if_read_only(cx: &mut Leaf<'_>) -> std::result::Result<(), Fault> {
    let read_only = cx.intern("buffer-read-only");
    let inhibit = cx.intern("inhibit-read-only");
    if cx.variable_value(read_only)?.unwrap_or(Value::NIL) != Value::NIL
        && cx.variable_value(inhibit)?.unwrap_or(Value::NIL) == Value::NIL
    {
        let condition = cx.intern("buffer-read-only");
        let buffer = cx.decode_buffer(Value::NIL)?;
        let data = list(cx, &[buffer]);
        return Err(Fault::Signal(condition, data));
    }
    Ok(())
}

fn spec_bytes(cx: &Leaf<'_>, saved: &[Value]) -> std::result::Result<Vec<u8>, Fault> {
    cx.string_bytes(saved[SPEC])
}

/// The KEYS vector (callint.c:286-292): the argument, else this-command-keys.
fn keys(cx: &mut Leaf<'_>, saved: &[Value]) -> std::result::Result<Vec<Value>, Fault> {
    let keys = if saved[KEYS] != Value::NIL {
        saved[KEYS]
    } else {
        let symbol = cx.intern(super::keyboard::COMMAND_KEYS);
        cx.variable_value(symbol)?.unwrap_or(Value::NIL)
    };
    Ok(if cx.kind(keys) == Kind::Vector { cx.vector_values(keys)? } else { vec![] })
}

/// The special leading characters `+ * - @ ^' (callint.c:358-411).
fn prefix_codes(cx: &mut Leaf<'_>, mut saved: Vec<Value>) -> Next {
    let spec = spec_bytes(cx, &saved)?;
    let keys = keys(cx, &saved)?;
    if index(saved[POS]) == 0 && !inhibit_mouse_event_check(cx)? {
        // The first event with parameters (EVENT_HAS_PARAMETERS is CONSP).
        let first = keys.iter().position(|&k| cx.pair(k).is_some()).unwrap_or(keys.len());
        saved[NEXT_EVENT] = fixnum(first as i64);
    }
    let mut pos = index(saved[POS]);
    while let Some(&c) = spec.get(pos) {
        match c {
            b'+' => return Err(Fault::Error("‘+’ is not used in ‘interactive’ for ordinary commands".into())),
            b'*' => {
                let read_only = cx.intern("buffer-read-only");
                if cx.variable_value(read_only)?.unwrap_or(Value::NIL) != Value::NIL {
                    if saved[RECORD] != Value::NIL {
                        if spec[pos + 1..].iter().any(|b| !b"rRpP\n".contains(b)) {
                            barf_if_read_only(cx)?;
                        }
                        saved[FLAGS] = fixnum(index(saved[FLAGS]) as i64 | RECORD_THEN_FAIL);
                    } else {
                        barf_if_read_only(cx)?;
                    }
                }
            }
            b'-' => {}
            b'@' => {
                let event = keys.get(index(saved[NEXT_EVENT])).copied().unwrap_or(Value::NIL);
                let window = cx.pair(event).and_then(|(_, w)| cx.pair(w)).and_then(|(w, _)| cx.pair(w)).map(|(w, _)| w);
                if window.is_some_and(|w| cx.kind(w) == Kind::Window) {
                    return Err(unsupported("@ with a window event"));
                }
            }
            b'^' => {
                // `call0 (Qhandle_shift_selection)'. GNU always has it (simple.el
                // is preloaded); in a VM2 world without simple.el it is void and
                // the shift-selection step is skipped (VM2-CALLINT1).
                let f = cx.intern("handle-shift-selection");
                if cx.symbol_function(f)? != Value::NIL {
                    saved[POS] = fixnum(pos as i64 + 1);
                    return Ok(Step::Call { f, args: vec![], then: resume_prefix, saved });
                }
            }
            _ => break,
        }
        pos += 1;
    }
    saved[POS] = fixnum(pos as i64);
    arg_codes(cx, saved)
}

/// callint.c DEFVAR_BOOL `inhibit-mouse-event-check' (default false).
fn inhibit_mouse_event_check(cx: &mut Leaf<'_>) -> std::result::Result<bool, Fault> {
    let symbol = cx.intern("inhibit-mouse-event-check");
    Ok(cx.variable_value(symbol)?.unwrap_or(Value::NIL) != Value::NIL)
}

fn resume_prefix(cx: &mut Leaf<'_>, _: Value, saved: &[Value]) -> Next {
    prefix_codes(cx, saved.to_vec())
}

fn unsupported(what: &str) -> Fault {
    Fault::Error(format!("call-interactively: {what} is not supported in VM2 yet"))
}

fn push(cx: &mut Leaf<'_>, saved: &mut [Value], arg: Value, history: Value) {
    saved[ARGS] = cx.cons(arg, saved[ARGS]);
    saved[HISTORY] = cx.cons(history, saved[HISTORY]);
    saved[POSITION_ARGS] = cx.cons(Value::NIL, saved[POSITION_ARGS]);
}

fn push_position(cx: &mut Leaf<'_>, saved: &mut [Value], arg: Value, history: Value) -> std::result::Result<(), Fault> {
    push(cx, saved, arg, history);
    cx.setcar(saved[POSITION_ARGS], Value::TRUE)?;
    Ok(())
}

fn call_form(cx: &mut Leaf<'_>, name: &str) -> Value {
    let f = cx.intern(name);
    list(cx, &[f])
}

/// One code letter per line (callint.c:449-762), from byte POS onward.
fn arg_codes(cx: &mut Leaf<'_>, mut saved: Vec<Value>) -> Next {
    let spec = spec_bytes(cx, &saved)?;
    let mut pos = index(saved[POS]);
    while pos < spec.len() {
        let line_end = spec[pos + 1..].iter().position(|&b| b == b'\n').map_or(spec.len(), |i| pos + 1 + i);
        let prompt = &spec[pos + 1..line_end];
        // callint_message = Fformat_message (prompt, previous args): VM2 has
        // no format-message, so a prompt with a `%' is not supported yet.
        if prompt.contains(&b'%') {
            return Err(unsupported("a `%' in an interactive prompt"));
        }
        let prefix = saved[PREFIX];
        match spec[pos] {
            b'i' => push(cx, &mut saved, Value::NIL, Value::NIL),
            b'P' => { let h = quotify(cx, prefix); push(cx, &mut saved, prefix, h) }
            b'p' => { let n = prefix_numeric(cx, prefix); push(cx, &mut saved, n, n) }
            b'N' if prefix != Value::NIL => { let n = prefix_numeric(cx, prefix); push(cx, &mut saved, n, n) }
            b'Z' if prefix == Value::NIL => push(cx, &mut saved, Value::NIL, Value::NIL),
            b'd' => {
                let point = fixnum(cx.point()?);
                let h = call_form(cx, "point");
                push(cx, &mut saved, point, h);
            }
            b'm' => {
                cx.editor_checked_mark(false)?;
                let mark = cx.editor_mark_marker()?;
                let h = call_form(cx, "mark");
                push_position(cx, &mut saved, mark, h)?;
            }
            b'r' => push_region(cx, &mut saved, false)?,
            b'R' => {
                saved[POS] = fixnum(pos as i64);
                let f = cx.intern("use-region-p");
                return Ok(Step::Call { f, args: vec![], then: region_p, saved });
            }
            b'e' => {
                let keys = keys(cx, &saved)?;
                let next = index(saved[NEXT_EVENT]);
                let Some(&event) = keys.get(next) else {
                    let function = saved[FUNCTION];
                    let name = if symbolp(cx, function) { cx.symbol_name(function)? } else { "command".into() };
                    return Err(Fault::Error(format!("{name} must be bound to an event with parameters")));
                };
                let after = if inhibit_mouse_event_check(cx)? {
                    next + 1
                } else {
                    keys[next + 1..].iter().position(|&k| cx.pair(k).is_some()).map_or(keys.len(), |i| next + 1 + i)
                };
                saved[NEXT_EVENT] = fixnum(after as i64);
                let history = quotify(cx, event);
                push(cx, &mut saved, event, history);
            }
            // No `k'/`K' code is supported, so no up-event is ever saved; an
            // argument with varies == 0 makes the call arg_from_tty.
            b'U' => {
                push(cx, &mut saved, Value::NIL, Value::NIL);
                saved[FLAGS] = fixnum(index(saved[FLAGS]) as i64 | ARG_FROM_TTY);
            }
            b'a' | b'b' | b'B' | b'c' | b'C' | b'D' | b'f' | b'F' | b'G' | b'k' | b'K' | b'M' | b'n' | b'N'
            | b's' | b'S' | b'v' | b'x' | b'X' | b'z' | b'Z' => {
                let code = spec[pos] as char;
                return Err(unsupported(&format!("the input-reading code `{code}'")));
            }
            _ => {
                let text = String::from_utf8_lossy(&spec[pos..]);
                let letter = text.chars().next().map_or(spec[pos] as u32, |c| c as u32);
                let shown = char::from_u32(letter).unwrap_or('\u{fffd}');
                return Err(Fault::Error(format!(
                    "Invalid control letter ‘{shown}’ (#o{letter:03o}, #x{letter:04x}) in interactive calling string"
                )));
            }
        }
        pos = if line_end < spec.len() { line_end + 1 } else { spec.len() };
    }
    saved[POS] = fixnum(pos as i64);
    finish(cx, saved)
}

fn push_region(cx: &mut Leaf<'_>, saved: &mut Vec<Value>, optional: bool) -> std::result::Result<(), Fault> {
    let mark_position = cx.editor_checked_mark(true)?.integer_value().unwrap();
    let mark = cx.editor_mark_marker()?;
    let point_position = cx.point()?;
    let point = cx.editor_point_marker()?;
    let (begin, end) = if point_position < mark_position { (point, mark) } else { (mark, point) };
    let h_begin = call_form(cx, if optional { "use-region-beginning" } else { "region-beginning" });
    let h_end = call_form(cx, if optional { "use-region-end" } else { "region-end" });
    push_position(cx, saved, begin, h_begin)?;
    push_position(cx, saved, end, h_end)?;
    Ok(())
}

fn region_p(cx: &mut Leaf<'_>, active: Value, saved: &[Value]) -> Next {
    let mut saved = saved.to_vec();
    if active != Value::NIL {
        push_region(cx, &mut saved, true)?;
    } else {
        let begin = call_form(cx, "use-region-beginning");
        let end = call_form(cx, "use-region-end");
        push(cx, &mut saved, Value::NIL, begin);
        push(cx, &mut saved, Value::NIL, end);
    }
    let spec = spec_bytes(cx, &saved)?;
    let pos = index(saved[POS]);
    let next = spec[pos + 1..].iter().position(|&b| b == b'\n').map_or(spec.len(), |i| pos + 2 + i);
    saved[POS] = fixnum(next.min(spec.len()) as i64);
    arg_codes(cx, saved)
}

/// callint.c:763-801: record, fail if `*' deferred it, restore, call.
fn finish(cx: &mut Leaf<'_>, saved: Vec<Value>) -> Next {
    let from_tty = index(saved[FLAGS]) as i64 & ARG_FROM_TTY != 0;
    if from_tty || saved[RECORD] != Value::NIL {
        let mut items = vec![saved[FUNCTION]];
        let mut tail = saved[HISTORY];
        let mut reversed = Vec::new();
        while let Some((head, rest)) = cx.pair(tail) {
            reversed.push(head);
            tail = rest;
        }
        items.extend(reversed.into_iter().rev());
        let item = list(cx, &items);
        return add_to_history(cx, item, saved);
    }
    recorded(cx, Value::NIL, &saved)
}

// ---------------------------------------------------------------------------
// The compiler's registration of a closure's interactive spec.

// `(function (lambda ARGS [DOC] (interactive SPEC . MODES) . BODY))' compiles
// to a call of this with the closure, SPEC, and a thunk evaluating SPEC in
// the closure's lexical environment (nil when SPEC is a string or nil). It
// stands in for GNU's CLOSURE_INTERACTIVE slot (eval.c:604-612,
// make-interpreted-closure), which VM2 closures do not have.
defsubr! { name = "#:vm2-set-interactive-form", gnu = "eval.c:604", effect = Global,
fn set_interactive_form(cx, closure: Value, spec: Value, thunk: Value) {
    if cx.kind(closure) != Kind::Closure {
        return Err(Fault::Wrong("closurep", closure));
    }
    cx.set_interactive_spec(closure, spec, thunk)?;
    Ok(closure)
}}

defsubr! { name = "commandp", gnu = "eval.c:2301", effect = Global,
fn commandp(cx, function: Value; optional for_call_interactively: Value) -> Step {
    command_predicate(cx, function, for_call_interactively, command_result, vec![])
}}

fn command_result(_: &mut Leaf<'_>, value: Value, _: &[Value]) -> Next {
    Ok(Step::Done(if value == Value::NIL { Value::NIL } else { Value::TRUE }))
}

pub(super) fn command_predicate(
    cx: &mut Leaf<'_>, function: Value, for_call: Value,
    then: crate::prim::Then, saved: Vec<Value>,
) -> Next {
    let fun = indirect(cx, function)?;
    let mut generic = false;
    let accepted = match cx.kind(fun) {
        Kind::Subr => subr_intspec(fun).is_some(),
        Kind::Closure => {
            if cx.interactive_spec(fun).is_some() {
                true
            } else {
                let doc = cx.closure_document(fun);
                let saved_doc = cx.pair(doc).is_some_and(|(file, offset)|
                    matches!(cx.kind(file), Kind::String | Kind::Bytes) && offset.integer_value().is_some());
                generic = doc != Value::NIL && !matches!(cx.kind(doc), Kind::String | Kind::Bytes)
                    && doc.integer_value().is_none() && !saved_doc;
                false
            }
        }
        Kind::String | Kind::Bytes | Kind::Vector => return then(cx, if for_call == Value::NIL { Value::TRUE } else { Value::NIL }, &saved),
        Kind::Cons => {
            let head = car(cx, fun)?;
            if head == cx.intern("autoload") {
                car(cx, cdr(cx, cdr(cx, cdr(cx, fun)?)?)?)? != Value::NIL
            } else if head == cx.intern("lambda") {
                let interactive = cx.intern("interactive");
                let body = cdr(cx, cdr(cx, fun)?)?;
                return then(cx, if assq(cx, interactive, body)? == Value::NIL { Value::NIL } else { Value::TRUE }, &saved);
            } else {
                return then(cx, Value::NIL, &saved);
            }
        }
        _ => return then(cx, Value::NIL, &saved),
    };
    if accepted { return then(cx, Value::TRUE, &saved); }
    let property = cx.intern("interactive-form");
    let mut symbol = function;
    while symbolp(cx, symbol) {
        if get(cx, symbol, property)? != Value::NIL {
            return Err(Fault::Error("Found an 'interactive-form' property!".into()));
        }
        symbol = cx.symbol_function(symbol)?;
    }
    if generic {
        let f = cx.intern("interactive-form");
        Ok(Step::Call { f, args: vec![fun], then, saved })
    } else {
        then(cx, Value::NIL, &saved)
    }
}

// ---------------------------------------------------------------------------
// command-modes

// GNU `Fcommand_modes` (data.c:1218-1278). VM2 subrs declare no modes.
defsubr! { name = "command-modes", gnu = "data.c:1218", effect = Reads,
fn command_modes(cx, command: Value) {
    if indirect(cx, command)? == Value::NIL {
        return Ok(Value::NIL);
    }
    // A `command-modes' property anywhere on the alias chain wins.
    let property = cx.intern("command-modes");
    let mut fun = command;
    for _ in 0..=cx.list_limit() {
        if !symbolp(cx, fun) {
            break;
        }
        let modes = get(cx, fun, property)?;
        if modes != Value::NIL {
            return Ok(modes);
        }
        fun = cx.symbol_function(fun)?;
    }
    match cx.kind(fun) {
        Kind::Closure => Ok(cx.interactive_modes(fun)),
        Kind::Cons => {
            let head = car(cx, fun)?;
            if head == cx.intern("autoload") {
                // (nth 3 FUN), the autoload's INTERACTIVE, when it is a list.
                let modes = car(cx, cdr(cx, cdr(cx, cdr(cx, fun)?)?)?)?;
                Ok(if cx.pair(modes).is_some() { modes } else { Value::NIL })
            } else if head == cx.intern("lambda") {
                let interactive = cx.intern("interactive");
                let form = assq(cx, interactive, cdr(cx, cdr(cx, fun)?)?)?;
                cdr(cx, cdr(cx, form)?)
            } else {
                Ok(Value::NIL)
            }
        }
        _ => Ok(Value::NIL),
    }
}}
