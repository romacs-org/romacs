//! GNU fileio.c save-side filesystem primitives. Native syscalls live behind
//! Leaf; Lisp handlers return Step::Call so no evaluator runs inside a leaf.
//! See evidence/filesave/README.md for the supported filename boundary.
use super::*;
use std::os::unix::fs::MetadataExt;

fn refusal(what: &str) -> Fault {
    Fault::Error(format!("VM2 {what} is unsupported"))
}

fn native_encoding(cx: &mut Leaf<'_>) -> std::result::Result<(), Fault> {
    for variable in ["file-name-coding-system", "default-file-name-coding-system"] {
        let symbol = cx.intern(variable);
        let value = cx.variable_value(symbol)?.unwrap_or(Value::NIL);
        if value != Value::NIL
            && !cx.symbol_name(value).is_ok_and(|s| {
                matches!(
                    s.as_str(),
                    "utf-8" | "utf-8-unix" | "undecided" | "undecided-unix"
                )
            })
        {
            return Err(refusal("filesave non-UTF-8 filename coding"));
        }
    }
    Ok(())
}

fn name(
    cx: &mut Leaf<'_>,
    value: Value,
    strip_slash: bool,
) -> std::result::Result<(String, Value), Fault> {
    if cx.string_bytes(value)?.contains(&0) {
        return Err(Fault::Wrong("filenamep", value));
    }
    // Expansion callbacks require a multi-stage request; do not silently bypass
    // one merely because the final operation has no handler.
    let expansion = cx.intern("expand-file-name");
    if super::fileio::find_file_name_handler(cx, value, expansion)? != Value::NIL {
        return Err(refusal("filesave expand-file-name handler"));
    }
    let mut path = super::fileio::expand_file_name(cx, value)?;
    if strip_slash {
        let expanded = cx.string(&path);
        let operation = cx.intern("directory-file-name");
        if super::fileio::find_file_name_handler(cx, expanded, operation)? != Value::NIL {
            return Err(refusal("filesave directory-file-name handler"));
        }
    }
    if strip_slash && path.len() > 1 {
        path = path.trim_end_matches('/').to_owned();
    }
    let multibyte = cx.string_view(value)?.is_multibyte() || !path.is_ascii();
    let result = if cx.string_text(value)? == path {
        value
    } else {
        cx.make_string(path.as_bytes().to_vec(), multibyte)
    };
    Ok((path, result))
}

fn handler(
    cx: &mut Leaf<'_>,
    name: Value,
    operation: &str,
    args: Vec<Value>,
) -> std::result::Result<Option<Next>, Fault> {
    let operation = cx.intern(operation);
    let function = super::fileio::find_file_name_handler(cx, name, operation)?;
    if function == Value::NIL {
        return Ok(None);
    }
    let mut all = vec![operation];
    all.extend(args);
    Ok(Some(Ok(Step::Call {
        f: function,
        args: all,
        then: returned,
        saved: vec![],
    })))
}
fn returned(_: &mut Leaf<'_>, value: Value, _: &[Value]) -> Next {
    Ok(Step::Done(value))
}

fn errno(cx: &mut Leaf<'_>, operation: &str, paths: &[Value], error: std::io::Error) -> Fault {
    let raw = error.raw_os_error();
    let text = error.to_string();
    let text = text
        .rsplit_once(" (os error ")
        .map_or(text.as_str(), |(s, _)| s);
    let mut data = Value::NIL;
    for path in paths.iter().rev() {
        data = cx.cons(*path, data);
    }
    let message = cx.string(if raw == Some(libc::EEXIST) {
        "File already exists"
    } else {
        text
    });
    data = cx.cons(message, data);
    let condition = match raw {
        Some(libc::EEXIST) => "file-already-exists",
        Some(libc::ENOENT) => "file-missing",
        Some(libc::EACCES) => "permission-denied",
        _ => "file-error",
    };
    if raw != Some(libc::EEXIST) {
        let op = cx.string(operation);
        data = cx.cons(op, data);
    }
    Fault::Signal(cx.intern(condition), data)
}

// ENOENT and ENOTDIR are both nil for GNU's metadata queries.
fn metadata_missing(error: &std::io::Error) -> bool {
    matches!(error.raw_os_error(), Some(libc::ENOENT | libc::ENOTDIR))
}

defsubr! { name = "file-writable-p", gnu = "fileio.c:3040", effect = Global,
fn file_writable_p(cx, filename: Value) -> Step {
    let (path, expanded) = name(cx, filename, false)?;
    if let Some(call) = handler(cx, expanded, "file-writable-p", vec![expanded])? { return call; }
    native_encoding(cx)?;
    let writable = match cx.filesave_access(&path, libc::W_OK) {
        Ok(()) => true,
        Err(e) if e.raw_os_error() == Some(libc::ENOENT) => {
            let slash = path.rfind('/').unwrap_or(0);
            let dir = &path[..=slash];
            cx.filesave_access(dir, libc::W_OK | libc::X_OK).is_ok()
        }
        Err(_) => false,
    };
    Ok(Step::Done(if writable { Value::TRUE } else { Value::NIL }))
}}

defsubr! { name = "file-modes", gnu = "fileio.c:3644", effect = Global,
fn file_modes(cx, filename: Value; optional flag: Value) -> Step {
    let (path, expanded) = name(cx, filename, true)?;
    if let Some(call) = handler(cx, expanded, "file-modes", vec![expanded, flag])? { return call; }
    native_encoding(cx)?;
    let meta = if flag == Value::NIL { std::fs::metadata(&path) } else { std::fs::symlink_metadata(&path) };
    let value = match meta {
        Ok(meta) => Value::integer(i64::from(meta.mode() & 0o7777)).map_err(Fault::Error)?,
        Err(e) if metadata_missing(&e) => Value::NIL,
        Err(e) => return Err(errno(cx, "Getting attributes", &[expanded], e)),
    };
    Ok(Step::Done(value))
}}

defsubr! { name = "set-file-modes", gnu = "fileio.c:3666", effect = Global,
fn set_file_modes(cx, filename: Value, mode: Value; optional flag: Value) -> Step {
    let bits = mode.integer_value().ok_or(Fault::Wrong("fixnump", mode))?;
    let (path, expanded) = name(cx, filename, false)?;
    if let Some(call) = handler(cx, expanded, "set-file-modes", vec![expanded, mode, flag])? { return call; }
    native_encoding(cx)?;
    cx.filesave_chmod(&path, (bits & 0o7777) as u32, flag != Value::NIL)
        .map_err(|e| errno(cx, "Doing chmod", &[expanded], e))?;
    Ok(Step::Done(Value::NIL))
}}

defsubr! { name = "rename-file", gnu = "fileio.c:2724", effect = Global,
fn rename_file(cx, file: Value, newname: Value; optional overwrite: Value) -> Step {
    let (source, source_value) = name(cx, file, false)?;
    let directory_operation = cx.intern("directory-file-name");
    if super::fileio::find_file_name_handler(cx, source_value, directory_operation)? != Value::NIL {
        return Err(refusal("filesave directory-file-name handler"));
    }
    let (mut target, mut target_value) = name(cx, newname, false)?;
    if cx.string_text(newname)?.ends_with('/') {
        let base = source.trim_end_matches('/').rsplit('/').next().unwrap_or("");
        target.push_str(base);
        target_value = cx.make_string(target.as_bytes().to_vec(), !target.is_ascii());
    }
    let args = vec![source_value, target_value, overwrite];
    if let Some(call) = handler(cx, source_value, "rename-file", args.clone())? { return call; }
    if let Some(call) = handler(cx, target_value, "rename-file", args)? { return call; }
    native_encoding(cx)?;
    if overwrite.integer_value().is_some() {
        // GNU can prompt after EEXIST. The leaf must never prompt or replace
        // without confirmation; allow the non-colliding atomic operation.
        match cx.filesave_rename(&source, &target, false) {
            Ok(()) => return Ok(Step::Done(Value::NIL)),
            Err(e) if e.raw_os_error() == Some(libc::EEXIST) => return Err(refusal("rename-file interactive overwrite confirmation")),
            Err(e) if e.raw_os_error() == Some(libc::EXDEV) => return Err(refusal("rename-file cross-device copy/delete")),
            Err(e) => return Err(errno(cx, "Renaming", &[source_value, target_value], e)),
        }
    }
    match cx.filesave_rename(&source, &target, overwrite != Value::NIL) {
        Ok(()) => Ok(Step::Done(Value::NIL)),
        Err(e) if e.raw_os_error() == Some(libc::EXDEV) => Err(refusal("rename-file cross-device copy/delete")),
        Err(e) => {
            // GNU's EEXIST carries only the destination, not both names.
            let paths = if e.raw_os_error() == Some(libc::EEXIST) { vec![target_value] } else { vec![source_value, target_value] };
            Err(errno(cx, "Renaming", &paths, e))
        }
    }
}}

defsubr! { name = "make-temp-name", gnu = "fileio.c:830", effect = Global,
fn make_temp_name(cx, prefix: Value) {
    use std::io::Read;
    let bytes = cx.string_bytes(prefix)?;
    if bytes.contains(&0) { return Err(Fault::Wrong("filenamep", prefix)); }
    let text = cx.string_text(prefix)?;
    native_encoding(cx)?;
    // GNU GT_NOCREATE neither expands PREFIX nor dispatches name handlers.
    // Randomness is read from the kernel without creating any path.
    let mut random = std::fs::File::open("/dev/urandom").map_err(|e| errno(cx, "Creating file name with prefix", &[prefix], e))?;
    const ALPHABET: &[u8] = b"abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789";
    for _ in 0..238328 {
        let mut suffix = Vec::with_capacity(6);
        while suffix.len() < 6 {
            let mut byte = [0];
            random.read_exact(&mut byte).map_err(|e| errno(cx, "Creating file name with prefix", &[prefix], e))?;
            if byte[0] < 248 { suffix.push(ALPHABET[usize::from(byte[0] % 62)]); }
        }
        let mut candidate = text.as_bytes().to_vec(); candidate.extend(suffix);
        let path = std::str::from_utf8(&candidate).expect("UTF-8 prefix plus ASCII suffix");
        match std::fs::symlink_metadata(path) {
            Ok(_) => continue,
            Err(e) if e.raw_os_error() == Some(libc::ENOENT) => {
                return Ok(cx.make_string(candidate, !text.is_ascii()));
            }
            Err(e) => return Err(errno(cx, "Creating file name with prefix", &[prefix], e)),
        }
    }
    Err(errno(cx, "Creating file name with prefix", &[prefix], std::io::Error::from_raw_os_error(libc::EEXIST)))
}}
