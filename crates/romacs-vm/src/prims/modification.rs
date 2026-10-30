//! GNU buffer.c:1518-1687. Counter state is integer-only and moves with buffers.
use super::*;
defsubr! { name = "buffer-modified-p", gnu = "buffer.c:1518", effect = Reads,
fn buffer_modified_p(cx; optional buffer: Value) {
    let state = cx.buffer_modification(buffer)?;
    if state.save_modiff >= state.modiff { return Ok(Value::NIL); }
    Ok(if state.autosave_modiff == state.modiff { cx.intern("autosaved") } else { Value::TRUE })
}}
defsubr! { name = "buffer-modified-tick", gnu = "buffer.c:1651", effect = Reads,
fn buffer_modified_tick(cx; optional buffer: Value) {
    Value::integer(cx.buffer_modification(buffer)?.modiff as i64).map_err(Fault::Error)
}}
defsubr! { name = "buffer-chars-modified-tick", gnu = "buffer.c:1674", effect = Reads,
fn buffer_chars_modified_tick(cx; optional buffer: Value) {
    Value::integer(cx.buffer_modification(buffer)?.chars_modiff as i64).map_err(Fault::Error)
}}
defsubr! { name = "set-buffer-modified-p", gnu = "buffer.c:1561", effect = Global,
fn set_buffer_modified_p(cx, flag: Value) -> Step {
    restore(cx, flag, true, Value::NIL)
}}
defsubr! { name = "restore-buffer-modified-p", gnu = "buffer.c:1588", effect = Global,
fn restore_buffer_modified_p(cx, flag: Value) -> Step {
    restore(cx, flag, false, flag)
}}

/// Frestore_buffer_modified_p: the visited file's truename of the buffer,
/// or of its base when indirect, is locked when the buffer becomes
/// modified and unlocked when it stops being so, unless modification hooks
/// are inhibited; then FLAG is the state. RETURNS is the value.
fn restore(cx: &mut Leaf<'_>, flag: Value, redisplay: bool, returns: Value) -> Next {
    let saved = vec![flag, if redisplay { Value::TRUE } else { Value::NIL }, returns];
    if super::fileio::var(cx, "inhibit-modification-hooks")? == Value::NIL {
        let buffer = cx.current_buffer()?;
        let base = match cx.buffer_base(buffer) { Value::NIL => buffer, base => base };
        let (truename, name) = (cx.intern("buffer-file-truename"), cx.intern("buffer-file-name"));
        let truename = cx.buffer_local_read(truename, base)?;
        // Testing `buffer-file-name' makes binding it to nil effective.
        if truename != Value::NIL && cx.buffer_local_read(name, base)? != Value::NIL {
            let state = cx.buffer_modification(Value::NIL)?;
            let already = state.save_modiff < state.modiff;
            let operation = match (already, flag != Value::NIL) { (false, true) => Some("lock-file"), (true, false) => Some("unlock-file"), _ => None };
            if let Some(operation) = operation {
                return Ok(Step::Call { f: cx.intern(operation), args: vec![truename], then: restored, saved });
            }
        }
    }
    restored(cx, Value::NIL, &saved)
}

fn restored(cx: &mut Leaf<'_>, _: Value, saved: &[Value]) -> Next {
    cx.set_buffer_modified(saved[0], saved[1] != Value::NIL)?;
    Ok(Step::Done(saved[2]))
}
