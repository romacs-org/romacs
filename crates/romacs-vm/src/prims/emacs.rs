//! GNU src/emacs.c: ending the session.
use super::*;

// emacs.c Fkill_emacs: `kill-emacs-hook' as safe_run_hooks runs it (the
// command loop's Lisp half, prims/keyboard.rs), then the session host ends
// the process with ARG's exit code. With no session host the VM is embedded
// in a process it does not own, so it refuses; RESTART is refused too.
defsubr! { name = "kill-emacs", gnu = "emacs.c:2953", effect = Global,
fn kill_emacs(cx; optional arg: Value, restart: Value) -> Step {
    if restart != Value::NIL { return Err(Fault::Error("VM2 kill-emacs RESTART is unsupported".into())); }
    if !cx.has_session_host() { return Err(Fault::Error("VM2 kill-emacs without a session host is unsupported".into())); }
    if super::keyboard::is_installed(cx)? { return run_hook(cx, &[arg]); }
    super::keyboard::install(cx, installed, vec![arg])
}}

fn installed(cx: &mut Leaf<'_>, _: Value, saved: &[Value]) -> Next { run_hook(cx, saved) }

fn run_hook(cx: &mut Leaf<'_>, saved: &[Value]) -> Next {
    let (f, hook) = (cx.intern("romacs--safe-run-hooks"), cx.intern("kill-emacs-hook"));
    Ok(Step::Call { f, args: vec![hook], then: unlock_all_files, saved: saved.to_vec() })
}

/// shut_down_emacs's unlock_all_files (filelock.c:663): each modified
/// buffer's truename is unlocked, then the exit. SAVED: ARG, then the
/// truenames left.
fn unlock_all_files(cx: &mut Leaf<'_>, _: Value, saved: &[Value]) -> Next {
    let field = cx.intern("buffer-file-truename");
    let mut truenames = vec![saved[0]];
    let mut buffers = cx.lifecycle_list();
    while let Some((buffer, rest)) = cx.pair(buffers) {
        buffers = rest;
        if !cx.lifecycle_live_p(buffer) { continue; }
        let truename = cx.buffer_local_read(field, buffer)?;
        let state = cx.buffer_modification(buffer)?;
        if cx.string_view(truename).is_ok() && state.save_modiff < state.modiff { truenames.push(truename); }
    }
    unlock_next(cx, Value::NIL, &truenames)
}

fn unlock_next(cx: &mut Leaf<'_>, _: Value, saved: &[Value]) -> Next {
    let Some((&truename, _)) = saved[1..].split_first() else { return exit(cx, Value::NIL, saved) };
    let rest: Vec<Value> = std::iter::once(saved[0]).chain(saved[2..].iter().copied()).collect();
    Ok(Step::Call { f: cx.intern("unlock-file"), args: vec![truename], then: unlock_next, saved: rest })
}

/// Fkill_emacs's exit code: an integer ARG's, else success.
fn exit(cx: &mut Leaf<'_>, _: Value, saved: &[Value]) -> Next {
    let code = saved[0].integer_value().map_or(0, |n| if n < 0 { n as i32 | i32::MIN } else { n as i32 & !i32::MIN });
    match cx.end_session(code)? {}
}
