//! GNU get-file-buffer is expanded-name equality, NOT inode equality.
//! The GUI's stronger visit lookup is deliberately separately named.
use super::*;
use super::fileio::find_file_name_handler;

defsubr! { name = "get-file-buffer", gnu = "buffer.c:493", effect = Global,
fn get_file_buffer(cx, filename: Value) -> Step {
    super::fileio::expanding(cx, filename, file_buffer_expanded, vec![])
}}

fn file_buffer_expanded(cx: &mut Leaf<'_>, expanded: Value, _: &[Value]) -> Next {
    let operation = cx.intern("get-file-buffer");
    let f = find_file_name_handler(cx, expanded, operation)?;
    if f != Value::NIL {
        return Ok(Step::Call { f, args: vec![operation, expanded], then: handled_buffer, saved: vec![] });
    }
    let path = super::fileio::file_name_bytes(cx, expanded)?;
    Ok(Step::Done(cx.file_visits().into_iter().find(|(_, name, _, _)| *name == path)
        .map_or(Value::NIL, |(buffer, _, _, _)| buffer)))
}

/// buffer.c:509: a handler's value that is not a buffer is nil.
fn handled_buffer(cx: &mut Leaf<'_>, value: Value, _: &[Value]) -> Next {
    Ok(Step::Done(if matches!(cx.kind(value), crate::prim::leaf::Kind::Buffer | crate::prim::leaf::Kind::ByteBuffer) { value } else { Value::NIL }))
}

/// Whether GNU would call a handler of NAME for OPERATION.
fn selected(cx: &mut Leaf<'_>, name: Value, operation: &str) -> std::result::Result<bool, Fault> {
    let operation = cx.intern(operation);
    Ok(find_file_name_handler(cx, name, operation)? != Value::NIL)
}

/// files.el `find-buffer-visiting' natively where no file name handler
/// applies to the operations it would ask; GNU's own Lisp where one does.
defsubr! { name = "romacs-find-buffer-visiting", gnu = "files.el:2354", effect = Global,
fn find_buffer_visiting(cx, filename: Value) -> Step {
    let lisp = |cx: &mut Leaf<'_>| Ok(Step::Call { f: cx.intern("find-buffer-visiting"), args: vec![filename],
                                                  then: super::fileio::returned, saved: vec![] });
    let Some(expanded) = super::expand_file_name::expand(cx, filename, Value::NIL)? else { return lisp(cx) };
    // Visits are compared as host bytes (ENCODE_FILE): a raw-named visit
    // is found by its raw name.
    use std::os::unix::ffi::OsStrExt;
    let path = super::fileio::file_name_path(cx, expanded)?;
    let bytes = path.as_os_str().as_bytes().to_vec();
    if selected(cx, expanded, "get-file-buffer")? { return lisp(cx); }
    let visits = cx.file_visits();
    if let Some((buffer, _, _, _)) = visits.iter().find(|(_, name, _, _)| *name == bytes) {
        return Ok(Step::Done(*buffer));
    }
    if selected(cx, expanded, "file-truename")? { return lisp(cx); }
    // GNU files.el checks the saved truename before the inode fallback.
    let truename = crate::host::canonicalize(&path).unwrap_or_else(|_| path.clone());
    let true_value = super::fileio::file_name_value(cx, truename.as_os_str().as_bytes());
    for operation in ["file-truename", "abbreviate-file-name", "file-attributes"] {
        if selected(cx, true_value, operation)? { return lisp(cx); }
    }
    let as_path = |bytes: &[u8]| std::path::PathBuf::from(std::ffi::OsStr::from_bytes(bytes));
    if let Some((buffer, _, _, _)) = visits.iter().find(|(_, name, _, saved)| {
        saved.as_deref().map(as_path) == Some(truename.clone())
            && crate::host::canonicalize(as_path(name)).unwrap_or_else(|_| as_path(name)) == truename
    }) {
        return Ok(Step::Done(*buffer));
    }
    let identity = crate::forwarded::file_identity(&path);
    for (buffer, name, visited, saved) in visits {
        if identity.is_some() && visited == identity {
            let candidate = super::fileio::file_name_value(cx, &name);
            let saved = super::fileio::file_name_value(cx, saved.as_deref().unwrap_or(&name));
            if selected(cx, candidate, "file-exists-p")? || selected(cx, saved, "file-attributes")? { return lisp(cx); }
            if crate::forwarded::file_identity(as_path(&name)) == identity { return Ok(Step::Done(buffer)); }
        }
    }
    Ok(Step::Done(Value::NIL))
}}
