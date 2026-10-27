//! syntax.c's syntax queries and scanning primitives. A scan is a pure
//! engine run (`crate::syntax_scan`); where GNU calls Lisp in the middle of
//! one (`internal--syntax-propertize', `syntax-ppss'), the run stops, the
//! engine performs the call, and the scan runs again from its arguments,
//! which wait with the answers so far in the continuation's registers.
use super::*;
use crate::prim::{SyntaxOp as Op, SyntaxOutcome as Outcome};

defsubr! { name = "char-syntax", gnu = "syntax.c:1098", effect = Reads,
fn char_syntax(cx, character: Value) { cx.char_syntax(character) }}

defsubr! { name = "syntax-class-to-char", gnu = "syntax.c:1124", effect = Pure,
fn syntax_class_to_char(cx, syntax: Value) {
    let class = syntax.integer_value().ok_or(Fault::Wrong("fixnump", syntax))?;
    let spec = crate::syntax_scan::SYNTAX_CODE_SPEC;
    match usize::try_from(class).ok().and_then(|class| spec.get(class)) {
        Some(&c) => Ok(Value::integer(i64::from(c)).expect("character")),
        None => Err(Fault::Range(Value::integer(spec.len() as i64 - 1).expect("fixnum"), syntax)),
    }
}}

defsubr! { name = "matching-paren", gnu = "syntax.c:1141", effect = Reads,
fn matching_paren(cx, character: Value) { cx.matching_paren(character) }}

defsubr! { name = "forward-comment", gnu = "syntax.c:2436", effect = Global,
fn forward_comment(cx, count: Value) -> Step {
    let function = cx.intern("forward-comment-function");
    let function = cx.variable_value(function)?.unwrap_or(Value::NIL);
    if function != Value::NIL {
        return Ok(Step::Call { f: function, args: vec![count], then: returned, saved: vec![] });
    }
    start(cx, Op::ForwardComment, &[count])
}}

defsubr! { name = "scan-lists", gnu = "syntax.c:3049", effect = Global,
fn scan_lists(cx, from: Value, count: Value, depth: Value) -> Step {
    start(cx, Op::ScanLists, &[from, count, depth])
}}

defsubr! { name = "scan-sexps", gnu = "syntax.c:3077", effect = Global,
fn scan_sexps(cx, from: Value, count: Value) -> Step {
    start(cx, Op::ScanSexps, &[from, count])
}}

defsubr! { name = "backward-prefix-chars", gnu = "syntax.c:3096", effect = Global,
fn backward_prefix_chars(cx) -> Step { start(cx, Op::BackwardPrefixChars, &[]) }}

defsubr! { name = "parse-partial-sexp", gnu = "syntax.c:3567", effect = Global,
fn parse_partial_sexp(cx, from: Value, to: Value; optional targetdepth: Value, stopbefore: Value,
                      oldstate: Value, commentstop: Value) -> Step {
    start(cx, Op::ParsePartialSexp, &[from, to, targetdepth, stopbefore, oldstate, commentstop])
}}

/// Registers: the operation, its six arguments, the `syntax-ppss' answers.
const ARGS: usize = 8;

fn returned(_cx: &mut Leaf<'_>, value: Value, _saved: &[Value]) -> Next {
    Ok(Step::Done(value))
}

fn start(cx: &mut Leaf<'_>, op: Op, args: &[Value]) -> Next {
    let mut saved = vec![Value::integer(op.code()).expect("operation"); 1];
    saved.extend(args);
    saved.resize(ARGS, Value::NIL);
    run(cx, saved)
}

fn run(cx: &mut Leaf<'_>, mut saved: Vec<Value>) -> Next {
    saved.truncate(ARGS);
    let int = |n: i64| Value::integer(n).expect("buffer position");
    match cx.syntax_scan(Op::of(saved[0]), &saved[1..7], saved[7])? {
        Outcome::Done(value) => Ok(Step::Done(value)),
        Outcome::Propertize(arg, charpos) => {
            // parse_sexp_propertize: safe_calln, then GNU's two checks.
            let modiff = cx.syntax_chars_modiff()?;
            saved.extend([int(charpos), int(modiff)]);
            let f = cx.intern("romacs--safe-call");
            let callee = cx.intern("internal--syntax-propertize");
            Ok(Step::Call { f, args: vec![callee, int(arg)], then: propertized, saved })
        }
        Outcome::Ppss(pos) => {
            // find_defun_start.
            let modiff = cx.syntax_chars_modiff()?;
            let point = cx.point()?;
            saved.extend([int(pos), int(modiff), int(point)]);
            let f = cx.intern("syntax-ppss");
            Ok(Step::Call { f, args: vec![int(pos)], then: answered, saved })
        }
    }
}

fn propertized(cx: &mut Leaf<'_>, _value: Value, saved: &[Value]) -> Next {
    let int = |v: Value| v.integer_value().expect("saved position");
    cx.syntax_propertize_check(int(saved[ARGS]), int(saved[ARGS + 1]))?;
    run(cx, saved.to_vec())
}

fn answered(cx: &mut Leaf<'_>, ppss: Value, saved: &[Value]) -> Next {
    let int = |v: Value| v.integer_value().expect("saved position");
    if cx.syntax_chars_modiff()? != int(saved[ARGS + 1]) {
        return Err(Fault::Error("syntax-ppss modified the buffer!".into()));
    }
    cx.syntax_set_point(int(saved[ARGS + 2]))?;
    // Fnth (8, ppss): a fixnum is the defun start, anything else POS.
    let mut tail = ppss;
    for _ in 0..8 {
        tail = match cx.pair(tail) {
            Some((_, cdr)) => cdr,
            None if tail == Value::NIL => Value::NIL,
            None => return Err(Fault::Wrong("listp", tail)),
        };
    }
    let start = match cx.pair(tail) {
        Some((car, _)) => car,
        None if tail == Value::NIL => Value::NIL,
        None => return Err(Fault::Wrong("listp", tail)),
    };
    let value = if start.integer_value().is_some() { start } else { saved[ARGS] };
    let answer = cx.cons(saved[ARGS], value);
    let mut saved = saved.to_vec();
    saved[7] = cx.cons(answer, saved[7]);
    run(cx, saved)
}
