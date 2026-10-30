//! GNU src/emacs.c: ending the session.
use super::*;

// emacs.c Fkill_emacs: `kill-emacs-hook', in batch as safe_run_hooks runs
// it (the command loop's Lisp half, prims/keyboard.rs), interactively
// through `run-hook-query-error-with-timeout', which asks whether to go on
// after an error; then shut_down_emacs auto-saves and the session host
// ends the process with ARG's exit code. With no session host the VM is embedded in a process it does not
// own, so it refuses; RESTART is refused too.
defsubr! { name = "kill-emacs", gnu = "emacs.c:2953", effect = Global,
fn kill_emacs(cx; optional arg: Value, restart: Value) -> Step {
    if restart != Value::NIL { return Err(Fault::Error("VM2 kill-emacs RESTART is unsupported".into())); }
    if !cx.has_session_host() { return Err(Fault::Error("VM2 kill-emacs without a session host is unsupported".into())); }
    if super::keyboard::is_installed(cx)? { return run_hook(cx, &[arg]); }
    super::keyboard::install(cx, installed, vec![arg])
}}

fn installed(cx: &mut Leaf<'_>, _: Value, saved: &[Value]) -> Next { run_hook(cx, saved) }

fn run_hook(cx: &mut Leaf<'_>, saved: &[Value]) -> Next {
    let noninteractive = cx.intern("noninteractive");
    let batch = cx.variable_value(noninteractive)?.unwrap_or(Value::NIL) != Value::NIL;
    // A VM without subr.el (the bare workspace) has only the batch runner.
    let query = cx.intern("run-hook-query-error-with-timeout");
    let f = if batch || cx.symbol_function(query)? == Value::NIL { cx.intern("romacs--safe-run-hooks") } else { query };
    let hook = cx.intern("kill-emacs-hook");
    Ok(Step::Call { f, args: vec![hook], then: auto_save, saved: saved.to_vec() })
}

/// shut_down_emacs's Fdo_auto_save (Qt, Qnil) (emacs.c:3156), through the
/// subr's symbol (fileio.c's, prims/autosave.rs), then the exit.
fn auto_save(cx: &mut Leaf<'_>, _: Value, saved: &[Value]) -> Next {
    let f = cx.intern("do-auto-save");
    if cx.symbol_function(f)? == Value::NIL { return exit(cx, Value::NIL, saved); }
    Ok(Step::Call { f, args: vec![Value::TRUE, Value::NIL], then: exit, saved: saved.to_vec() })
}

/// Fkill_emacs's exit code: an integer ARG's, else success.
fn exit(cx: &mut Leaf<'_>, _: Value, saved: &[Value]) -> Next {
    let code = saved[0].integer_value().map_or(0, |n| if n < 0 { n as i32 | i32::MIN } else { n as i32 & !i32::MIN });
    match cx.end_session(code)? {}
}
