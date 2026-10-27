//! Hook running from GNU src/eval.c: `run-hooks` (eval.c:2883) over
//! `run_hook` -> `Frun_hook_with_args (1, &hook)`, and `run-hook-with-args`
//! (eval.c:2908); both run `run_hook_with_args` (eval.c:3007-3062) with
//! `funcall_nil`.
use super::*;
use super::vars::{default_value, find_symbol_value, function_p};

// Tier (PRIMITIVE-DOCTRINE section 3): every hook function is a Lisp call, so
// the walk is a T2 state machine. Each call is one `Step::Call'; what the walk
// needs afterwards waits in the continuation's traced registers, never in a
// Rust local, so a collection inside a hook function forwards all of it.
//
// Saved register layout, shared by all three primitives:
//   [UNTIL_FAILURE, N, ARG1 .. ARGN, SYM, VAL, (GLOBAL), HOOK..]
// N and the ARGs are what every hook function is called with (none for
// `run-hooks'); SYM is the hook being walked, VAL the local tail, GLOBAL the
// global tail while one is walked, and the HOOKs are the `run-hooks' symbols
// not yet run.
//
// GNU's `NILP (Vrun_hooks)' early return is not modelled: Vrun_hooks is a C
// global (eval.c:44, not a DEFVAR) that is nil only before initialisation
// and while dying, neither of which Lisp can observe in VM2.
defsubr! { name = "run-hooks", gnu = "eval.c:2883", effect = Global,
fn run_hooks(cx; rest hooks) -> Step {
    next_hook(cx, &[], hooks, Mode::All)
}}

// The docstring calls the value "unspecified"; with funcall_nil it is always
// nil (RET never leaves Qnil, and the whole-value branch returns
// funcall_nil's nil), recorded live.
defsubr! { name = "run-hook-with-args", gnu = "eval.c:2908", effect = Global,
fn run_hook_with_args(cx, hook: Value; rest args) -> Step {
    next_hook(cx, args, std::slice::from_ref(&hook), Mode::All)
}}

/// run_hook_with_args's FUNCALL: funcall_nil runs every function;
/// until-failure (funcall_not) stops at nil; until-success stops at non-nil;
/// wrapped (run_hook_wrapped_funcall) calls ARGS' first element, the
/// wrapper, with each function and the rest of ARGS, and stops at non-nil.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Mode { All, UntilFailure, UntilSuccess, Wrapped }
impl Mode {
    fn from(register: Value) -> Mode {
        match register.integer_value() {
            Some(1) => Mode::UntilFailure, Some(2) => Mode::UntilSuccess, Some(3) => Mode::Wrapped, _ => Mode::All,
        }
    }
    fn stop(self, result: Value) -> Option<Value> {
        match self {
            Mode::UntilFailure if result == Value::NIL => Some(Value::NIL),
            Mode::UntilSuccess | Mode::Wrapped if result != Value::NIL => Some(result),
            _ => None,
        }
    }
}

fn split_args(saved: &[Value]) -> (&[Value], &[Value]) {
    let n = saved[0].integer_value().expect("hook argument count register") as usize;
    (&saved[1..1 + n], &saved[1 + n..])
}

// One `run_hook_with_args' per remaining HOOKS element, in order. The value is
// GNU's find_symbol_value (a non-symbol signals `(wrong-type-argument symbolp
// X)'); void or nil runs nothing; a non-cons or a FUNCTIONP value (a lambda
// list included) is called once as a whole; any other cons is walked.
fn next_hook(cx: &mut Leaf<'_>, args: &[Value], hooks: &[Value], mode: Mode) -> Next {
    let mut hooks = hooks;
    while let Some((&sym, rest)) = hooks.split_first() {
        hooks = rest;
        let val = match find_symbol_value(cx, sym)? {
            None => continue,
            Some(v) if v == Value::NIL => continue,
            Some(v) => v,
        };
        if cx.pair(val).is_none() || function_p(cx, val)? {
            return call_then(val, ran_whole, args, &[], hooks, mode);
        }
        return walk_local(cx, args, sym, val, hooks, mode);
    }
    Ok(Step::Done(if mode == Mode::UntilFailure { Value::TRUE } else { Value::NIL }))
}

fn ran_whole(cx: &mut Leaf<'_>, result: Value, saved: &[Value]) -> Next {
    let mode = Mode::from(saved[0]);
    if let Some(value) = mode.stop(result) { return Ok(Step::Done(value)); }
    let saved = &saved[1..];
    let (args, hooks) = split_args(saved);
    next_hook(cx, args, hooks, mode)
}

// The local walk, from the cons VAL. `t' runs the global value
// (Fdefault_value, which signals void-variable when it is void): skipped when
// nil, called whole when a non-cons or a cons headed by `lambda' (GNU's test
// here is that car, not FUNCTIONP), otherwise walked. A dotted tail ends the
// walk.
fn walk_local(cx: &mut Leaf<'_>, args: &[Value], sym: Value, mut val: Value, hooks: &[Value], mode: Mode) -> Next {
    while let Some((element, tail)) = cx.pair(val) {
        if element == Value::TRUE {
            let global = default_value(cx, sym)?.ok_or(Fault::VoidVariable(sym))?;
            if global != Value::NIL {
                match cx.pair(global) {
                    Some((head, _)) if head != cx.intern("lambda") => {
                        return walk_global(cx, args, sym, val, global, hooks, mode);
                    }
                    _ => return call_then(global, ran_local, args, &[sym, val], hooks, mode),
                }
            }
        } else {
            return call_then(element, ran_local, args, &[sym, val], hooks, mode);
        }
        val = tail;
    }
    next_hook(cx, args, hooks, mode)
}

// GNU reads XCDR (val) only after the call returns, so a hook function that
// edits the list it is running from changes what runs next.
fn ran_local(cx: &mut Leaf<'_>, result: Value, saved: &[Value]) -> Next {
    let mode = Mode::from(saved[0]);
    if let Some(value) = mode.stop(result) { return Ok(Step::Done(value)); }
    let saved = &saved[1..];
    let (args, rest) = split_args(saved);
    let (&[sym, val], hooks) = rest.split_at(2) else { unreachable!("two registers") };
    let tail = cx.pair(val).expect("the saved local tail is a cons").1;
    walk_local(cx, args, sym, tail, hooks, mode)
}

// The global list from the cons GLOBAL; a `t' in it is skipped (GNU's guard
// against an endless loop). When it ends, the local walk resumes after VAL.
fn walk_global(cx: &mut Leaf<'_>, args: &[Value], sym: Value, val: Value, mut global: Value, hooks: &[Value], mode: Mode) -> Next {
    while let Some((element, tail)) = cx.pair(global) {
        if element != Value::TRUE {
            return call_then(element, ran_global, args, &[sym, val, global], hooks, mode);
        }
        global = tail;
    }
    let tail = cx.pair(val).expect("the saved local tail is a cons").1;
    walk_local(cx, args, sym, tail, hooks, mode)
}

fn ran_global(cx: &mut Leaf<'_>, result: Value, saved: &[Value]) -> Next {
    let mode = Mode::from(saved[0]);
    if let Some(value) = mode.stop(result) { return Ok(Step::Done(value)); }
    let saved = &saved[1..];
    let (args, rest) = split_args(saved);
    let (&[sym, val, global], hooks) = rest.split_at(3) else { unreachable!("three registers") };
    let tail = cx.pair(global).expect("the saved global tail is a cons").1;
    walk_global(cx, args, sym, val, tail, hooks, mode)
}

// Every hook function gets the same ARGS (GNU reuses the args array; only
// args[0], the function slot, changes between calls).
fn call_then(f: Value, then: crate::prim::Then, args: &[Value], head: &[Value], hooks: &[Value], mode: Mode) -> Next {
    let mut saved = Vec::with_capacity(1 + args.len() + head.len() + hooks.len());
    saved.push(Value::integer(mode as i64).expect("hook mode register"));
    saved.push(Value::integer(args.len() as i64).expect("argument count is a fixnum"));
    saved.extend_from_slice(args);
    saved.extend_from_slice(head);
    saved.extend_from_slice(hooks);
    if mode == Mode::Wrapped {
        let (&wrapper, rest) = args.split_first().expect("run-hook-wrapped's wrapper");
        let args = std::iter::once(f).chain(rest.iter().copied()).collect();
        return Ok(Step::Call { f: wrapper, args, then, saved });
    }
    Ok(Step::Call { f, args: args.to_vec(), then, saved })
}


defsubr! { name = "run-hook-with-args-until-failure", gnu = "eval.c:2951", effect = Global,
fn run_hook_with_args_until_failure(cx, hook: Value; rest args) -> Step {
    next_hook(cx, args, std::slice::from_ref(&hook), Mode::UntilFailure)
}}

defsubr! { name = "run-hook-with-args-until-success", gnu = "eval.c:2927", effect = Global,
fn run_hook_with_args_until_success(cx, hook: Value; rest args) -> Step {
    next_hook(cx, args, std::slice::from_ref(&hook), Mode::UntilSuccess)
}}

defsubr! { name = "run-hook-wrapped", gnu = "eval.c:2981", effect = Global,
fn run_hook_wrapped(cx, hook: Value, wrap_function: Value; rest args) -> Step {
    let args: Vec<Value> = std::iter::once(wrap_function).chain(args.iter().copied()).collect();
    next_hook(cx, &args, std::slice::from_ref(&hook), Mode::Wrapped)
}}
