//! GNU fileio.c auto-saving and the process-stream DEFUNs. `do-auto-save'
//! is a subr whose loop is the Lisp half (rlisp/vm/auto-save.el), over
//! the C state it shares with `write-region': `auto_saving' and
//! `auto_save_mode_bits' are AUTO_SAVING, nil or the mode bits, read by
//! the write path (prims/fileio.rs), and `auto_save_failure_time' is
//! per-buffer modification state.
use super::*;

/// GNU `auto_saving' and `auto_save_mode_bits': nil, or the mode an
/// auto-save file is created with while `do-auto-save' runs.
pub(super) const AUTO_SAVING: &str = "#:vm2-auto-saving";

const LISP_HALF: &str = include_str!("../../../../rlisp/vm/auto-save.el");

defsubr! { name = "do-auto-save", gnu = "fileio.c:6315", effect = Global,
fn do_auto_save(cx; optional no_message: Value, current_only: Value) -> Step {
    // The loop runs `auto-save-hook' as keyboard.c safe_run_hooks does,
    // which the command loop's Lisp half defines.
    let args = vec![no_message, current_only];
    if !super::keyboard::is_installed(cx)? { return super::keyboard::install(cx, keyboard_installed, args); }
    keyboard_installed(cx, Value::NIL, &args)
}}

fn keyboard_installed(cx: &mut Leaf<'_>, _: Value, saved: &[Value]) -> Next {
    let helper = cx.intern("romacs--do-auto-save");
    if cx.symbol_function(helper)? != Value::NIL { return half_installed(cx, Value::NIL, saved); }
    let source = cx.string(LISP_HALF);
    let form = cx.read_from_string(source, Value::NIL, Value::NIL)?;
    let (form, _) = cx.pair(form).expect("read-from-string returns (OBJECT . POSITION)");
    Ok(Step::Call { f: cx.intern("eval"), args: vec![form, Value::TRUE], then: half_installed, saved: saved.to_vec() })
}

fn half_installed(cx: &mut Leaf<'_>, _: Value, saved: &[Value]) -> Next {
    Ok(Step::Call { f: cx.intern("romacs--do-auto-save"), args: saved.to_vec(), then: super::fileio::returned, saved: vec![] })
}

// Internal: fileio.c auto_save_1 for the current buffer. The auto-save file
// takes the visited file's mode, writable by its owner, else 0666; the
// write runs with `auto_saving' set, which stays set until
// `romacs--auto-save-done', as GNU's does until do_auto_save_unwind.
defsubr! { name = "romacs--auto-save-1", gnu = "fileio.c:6249", effect = Global,
fn auto_save_1(cx) -> Step {
    let filename = super::fileio::var(cx, "buffer-file-name")?;
    if !matches!(cx.kind(filename), crate::prim::leaf::Kind::String | crate::prim::leaf::Kind::Bytes) {
        return write_auto_save(cx, Value::integer(0o666).unwrap(), &[]);
    }
    match crate::host::metadata(cx.string_text(filename)?) {
        Ok(meta) => {
            use std::os::unix::fs::MetadataExt;
            write_auto_save(cx, Value::integer(i64::from((meta.mode() | 0o600) & 0o777)).unwrap(), &[])
        }
        // Remote files don't cooperate with fstatat: ask `file-modes'.
        Err(_) => Ok(Step::Call { f: cx.intern("file-modes"), args: vec![filename, Value::NIL], then: remote_modes, saved: vec![] }),
    }
}}

fn remote_modes(cx: &mut Leaf<'_>, modes: Value, _: &[Value]) -> Next {
    let bits = modes.integer_value().map_or(0o666, |m| (m | 0o600) & 0o777);
    write_auto_save(cx, Value::integer(bits).unwrap(), &[])
}

fn write_auto_save(cx: &mut Leaf<'_>, bits: Value, _: &[Value]) -> Next {
    let auto_saving = cx.intern(AUTO_SAVING);
    cx.variable_set(auto_saving, bits)?;
    let name = super::fileio::var(cx, "buffer-auto-save-file-name")?;
    let visit = if super::fileio::var(cx, "auto-save-visited-file-name")? == Value::NIL { cx.intern("lambda") } else { Value::TRUE };
    let args = vec![Value::NIL, Value::NIL, name, Value::NIL, visit, Value::NIL, Value::NIL];
    Ok(Step::Call { f: cx.intern("write-region"), args, then: super::fileio::returned, saved: vec![] })
}

// Internal: do_auto_save_unwind's `auto_saving = 0'.
defsubr! { name = "romacs--auto-save-done", gnu = "fileio.c:6283", effect = Global,
fn auto_save_done(cx) {
    let auto_saving = cx.intern(AUTO_SAVING);
    cx.variable_set(auto_saving, Value::NIL)?;
    Ok(Value::NIL)
}}

// Internal: the current buffer's `auto_save_failure_time', set to TIME when
// one is given.
defsubr! { name = "romacs--auto-save-failure-time", gnu = "fileio.c:6489", effect = Global,
fn auto_save_failure_time(cx; optional time: Value) {
    let set = match time { Value::NIL => None, t => Some(t.integer_value().ok_or(Fault::Wrong("fixnump", t))?) };
    let failure = cx.update_modification(|m| { if let Some(t) = set { m.auto_save_failure = t; } m.auto_save_failure })?;
    Value::integer(failure).map_err(Fault::Error)
}}

// Internal: the list file do-auto-save writes with stdio, TEXT's bytes in
// FILE, created or truncated as fopen "w" does; a file that cannot be
// opened is not written.
defsubr! { name = "romacs--write-auto-save-list", gnu = "fileio.c:6378", effect = Global,
fn write_auto_save_list(cx, file: Value, text: Value) {
    use std::io::Write;
    let path = cx.string_text(file)?;
    let bytes = cx.string_view(text)?.bytes().to_vec();
    let mut options = crate::host::OpenOptions::new();
    options.write(true).create(true).truncate(true);
    if let Ok(mut out) = crate::host::open_output(&path, &options) {
        let _ = out.write_all(&bytes);
        let _ = cx.fileio_close_output(out);
    }
    Ok(Value::NIL)
}}

// Internal: the loop's `BUF_AUTOSAVE_MODIFF (b) = BUF_MODIFF (b)' for the
// current buffer, leaving its saved state and failure time alone.
defsubr! { name = "romacs--auto-saved-tick", gnu = "fileio.c:6480", effect = Global,
fn auto_saved_tick(cx) {
    cx.update_modification(|m| m.autosave_modiff = m.modiff)?;
    Ok(Value::NIL)
}}

defsubr! { name = "set-buffer-auto-saved", gnu = "fileio.c:6518", effect = Global,
fn set_buffer_auto_saved(cx) {
    cx.update_modification(|m| { m.autosave_modiff = m.modiff; m.auto_save_failure = 0; })?;
    let size = cx.buffer_shape()?.z - 1;
    cx.bset("buffer-saved-size", Value::integer(size).map_err(Fault::Error)?)?;
    Ok(Value::NIL)
}}

defsubr! { name = "clear-buffer-auto-save-failure", gnu = "fileio.c:6532", effect = Global,
fn clear_buffer_auto_save_failure(cx) {
    cx.update_modification(|m| m.auto_save_failure = 0)?;
    Ok(Value::NIL)
}}

defsubr! { name = "unix-sync", gnu = "fileio.c:3783", effect = Global,
fn unix_sync(cx) {
    let _ = cx;
    crate::host::sync_all().map_err(|e| Fault::Error(e.to_string()))?;
    Ok(Value::NIL)
}}

/// file_for_stream: the standard stream STREAM names, as its descriptor.
fn standard_stream(cx: &mut Leaf<'_>, stream: Value) -> std::result::Result<i32, Fault> {
    if cx.kind(stream) != crate::prim::leaf::Kind::Symbol { return Err(Fault::Wrong("symbolp", stream)); }
    match cx.symbol_name(stream)?.as_str() {
        "stdin" => Ok(0),
        "stdout" => Ok(1),
        "stderr" => Ok(2),
        _ => {
            let message = cx.string("unsupported stream");
            let data = cx.cons(stream, Value::NIL);
            Err(Fault::Signal(cx.intern("error"), cx.cons(message, data)))
        }
    }
}

fn flush(fd: i32) {
    use std::io::Write;
    let _ = match fd { 1 => std::io::stdout().flush(), 2 => std::io::stderr().flush(), _ => Ok(()) };
}

// On POSIX systems a stream is always binary: flush it and answer t.
defsubr! { name = "set-binary-mode", gnu = "fileio.c:6588", effect = Global,
fn set_binary_mode(cx, stream: Value, mode: Value) {
    let _ = mode;
    flush(standard_stream(cx, stream)?);
    Ok(Value::TRUE)
}}

defsubr! { name = "file--close-stream", gnu = "fileio.c:6620", effect = Global,
fn file_close_stream(cx, stream: Value) {
    let fd = standard_stream(cx, stream)?;
    flush(fd);
    let _ = crate::host::close_standard_stream(fd);
    Ok(Value::NIL)
}}
