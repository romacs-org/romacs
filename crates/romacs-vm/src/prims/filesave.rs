//! GNU fileio.c save-side filesystem primitives. Native syscalls live behind
//! Leaf; Lisp handlers return Step::Call so no evaluator runs inside a leaf.
//! See evidence/filesave/README.md for the supported filename boundary.
use super::*;
use crate::host;
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

/// An expanded name as text and value.
fn as_file(cx: &mut Leaf<'_>, expanded: Value) -> std::result::Result<(std::path::PathBuf, Value), Fault> {
    Ok((super::fileio::file_name_path(cx, expanded)?, expanded))
}

/// Fdirectory_file_name of EXPANDED for BODY whose own arguments are the
/// first ARGS of SAVED: ALWAYS as in Fdelete_directory_internal, else only
/// for a name ending in a slash as in fileio.c expand_and_dir_to_file. A
/// directory-file-name handler is called first; BODY then runs again on
/// its value, which an extra entry in SAVED marks as final.
fn dir_to_file(
    cx: &mut Leaf<'_>,
    expanded: Value,
    body: crate::prim::Then,
    saved: &[Value],
    args: usize,
    always: bool,
) -> std::result::Result<std::result::Result<(std::path::PathBuf, Value), Next>, Fault> {
    if saved.len() > args {
        if cx.string_bytes(expanded).is_err() {
            return Err(Fault::Error("Invalid handler in ‘file-name-handler-alist’".into()));
        }
        return Ok(Ok(as_file(cx, expanded)?));
    }
    let bytes = cx.string_bytes(expanded)?;
    if !always && (bytes.len() < 2 || bytes.last() != Some(&b'/')) {
        return Ok(Ok(as_file(cx, expanded)?));
    }
    match super::savejourney::directory_file_name(cx, expanded)? {
        Step::Done(file) => Ok(Ok(as_file(cx, file)?)),
        Step::Call { f, args, .. } => {
            let mut saved = saved.to_vec();
            saved.push(Value::TRUE);
            Ok(Err(Ok(Step::Call { f, args, then: body, saved })))
        }
        _ => unreachable!("directory-file-name returns or calls its handler"),
    }
}

use super::fileio::{expanding, handler};

/// Fexpand_file_name (FILENAME, BVAR (current_buffer, directory)), as
/// set-file-modes and set-file-times expand.
fn expanding_in_buffer(cx: &mut Leaf<'_>, filename: Value, then: crate::prim::Then, saved: Vec<Value>) -> Next {
    let directory = super::fileio::var(cx, "default-directory")?;
    super::expand_file_name::expanding(cx, filename, directory, then, saved)
}


/// fileio.c report_file_errno: an EEXIST carries no OPERATION.
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
    let message = cx.string(text);
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

/// fileio.c barf_or_query_if_file_exists refusing an existing NAME.
fn already_exists(cx: &mut Leaf<'_>, name: Value) -> Fault {
    let message = cx.string("File already exists");
    let data = cx.cons(name, Value::NIL);
    let data = cx.cons(message, data);
    Fault::Signal(cx.intern("file-already-exists"), data)
}

// ENOENT and ENOTDIR are both nil for GNU's metadata queries.
fn metadata_missing(error: &std::io::Error) -> bool {
    matches!(error.raw_os_error(), Some(libc::ENOENT | libc::ENOTDIR))
}

defsubr! { name = "make-directory-internal", gnu = "fileio.c", effect = Global,
fn make_directory_internal(cx, directory: Value) -> Step {
    expanding(cx, directory, make_directory_expanded, vec![])
}}
fn make_directory_expanded(cx: &mut Leaf<'_>, expanded: Value, _: &[Value]) -> Next {
    let (path, expanded) = as_file(cx, expanded)?;
    // GNU dispatches make-directory handlers in the Lisp wrapper, not here.
    native_encoding(cx)?;
    cx.filesave_mkdir(&path).map_err(|e| errno(cx, "Creating directory", &[expanded], e))?;
    Ok(Step::Done(Value::NIL))
}

defsubr! { name = "delete-file", gnu = "fileio.c", effect = Global,
fn delete_file(cx, filename: Value; optional trash: Value) -> Step {
    expanding(cx, filename, delete_file_expanded, vec![trash])
}}
fn delete_file_expanded(cx: &mut Leaf<'_>, expanded: Value, saved: &[Value]) -> Next {
    let trash = saved[0];
    if let Some(call) = handler(cx, expanded, "delete-file", vec![expanded, trash])? { return call; }
    if trash != Value::NIL { return Err(refusal("delete-file trash")); }
    // files.el unlinks the expanded name itself: "file/" is ENOTDIR.
    let (path, _) = as_file(cx, expanded)?;
    native_encoding(cx)?;
    match cx.filesave_unlink(&path) {
        Ok(()) => (),
        Err(e) if e.raw_os_error() == Some(libc::ENOENT) => (),
        Err(e) => return Err(errno(cx, "Removing old name", &[expanded], e)),
    }
    Ok(Step::Done(Value::NIL))
}

defsubr! { name = "file-writable-p", gnu = "fileio.c:3040", effect = Global,
fn file_writable_p(cx, filename: Value) -> Step {
    expanding(cx, filename, file_writable_expanded, vec![])
}}
fn file_writable_expanded(cx: &mut Leaf<'_>, expanded: Value, _: &[Value]) -> Next {
    let (path, expanded) = as_file(cx, expanded)?;
    if let Some(call) = handler(cx, expanded, "file-writable-p", vec![expanded])? { return call; }
    native_encoding(cx)?;
    let writable = match cx.filesave_access(path.as_os_str(), libc::W_OK) {
        Ok(()) => true,
        Err(e) if e.raw_os_error() == Some(libc::ENOENT) => {
            use std::os::unix::ffi::OsStrExt;
            let bytes = path.as_os_str().as_bytes();
            let slash = bytes.iter().rposition(|&b| b == b'/').unwrap_or(0);
            let dir = std::ffi::OsStr::from_bytes(&bytes[..=slash]);
            cx.filesave_access(dir, libc::W_OK | libc::X_OK).is_ok()
        }
        Err(_) => false,
    };
    Ok(Step::Done(if writable { Value::TRUE } else { Value::NIL }))
}

defsubr! { name = "file-modes", gnu = "fileio.c:3644", effect = Global,
fn file_modes(cx, filename: Value; optional flag: Value) -> Step {
    expanding(cx, filename, file_modes_expanded, vec![flag])
}}
fn file_modes_expanded(cx: &mut Leaf<'_>, expanded: Value, saved: &[Value]) -> Next {
    let flag = saved[0];
    let (path, expanded) = match dir_to_file(cx, expanded, file_modes_expanded, saved, 1, false)? { Ok(file) => file, Err(call) => return call };
    if let Some(call) = handler(cx, expanded, "file-modes", vec![expanded, flag])? { return call; }
    native_encoding(cx)?;
    let meta = if flag == Value::NIL { host::metadata(&path) } else { host::symlink_metadata(&path) };
    let value = match meta {
        Ok(meta) => Value::integer(i64::from(meta.mode() & 0o7777)).map_err(Fault::Error)?,
        Err(e) if metadata_missing(&e) => Value::NIL,
        Err(e) => return Err(errno(cx, "Getting attributes", &[expanded], e)),
    };
    Ok(Step::Done(value))
}

defsubr! { name = "set-file-modes", gnu = "fileio.c:3666", effect = Global,
fn set_file_modes(cx, filename: Value, mode: Value; optional flag: Value) -> Step {
    mode.integer_value().ok_or(Fault::Wrong("fixnump", mode))?;
    expanding_in_buffer(cx, filename, set_file_modes_expanded, vec![mode, flag])
}}
fn set_file_modes_expanded(cx: &mut Leaf<'_>, expanded: Value, saved: &[Value]) -> Next {
    let (mode, flag) = (saved[0], saved[1]);
    let bits = mode.integer_value().expect("checked fixnum");
    let (path, expanded) = as_file(cx, expanded)?;
    if let Some(call) = handler(cx, expanded, "set-file-modes", vec![expanded, mode, flag])? { return call; }
    native_encoding(cx)?;
    cx.filesave_chmod(&path, (bits & 0o7777) as u32, flag != Value::NIL)
        .map_err(|e| errno(cx, "Doing chmod", &[expanded], e))?;
    Ok(Step::Done(Value::NIL))
}

// Fdefault_file_modes and Fset_default_file_modes: the permission bits of
// newly created files, the complement of the process umask.  GNU keeps the
// mask in realmask; VM2 reads the process's own ([`umask`]).
defsubr! { name = "default-file-modes", gnu = "fileio.c:3730", effect = Reads,
fn default_file_modes(cx) {
    let _ = cx;
    Ok(Value::integer(i64::from(!umask()? & 0o777)).map_err(Fault::Error)?)
}}
defsubr! { name = "set-default-file-modes", gnu = "fileio.c:3700", effect = Global,
fn set_default_file_modes(cx, mode: Value) {
    let mode = mode.integer_value().ok_or(Fault::Wrong("fixnump", mode))?;
    cx.filesave_set_umask(!(mode as u32) & 0o777);
    Ok(Value::NIL)
}}

defsubr! { name = "rename-file", gnu = "fileio.c:2724", effect = Global,
fn rename_file(cx, file: Value, newname: Value; optional overwrite: Value) -> Step {
    expanding(cx, file, rename_staged, vec![file, newname, overwrite])
}}

/// FILE expanded, then expand_cp_target (Fdirectory_file_name (FILE),
/// NEWNAME), then the rename.
fn rename_staged(cx: &mut Leaf<'_>, value: Value, saved: &[Value]) -> Next {
    let (source_value, target_value) = match cp_stages(cx, value, saved, 3, true, rename_staged)? {
        Ok(names) => names,
        Err(next) => return next,
    };
    let overwrite = saved[2];
    let (source, target) = (super::fileio::file_name_path(cx, source_value)?, super::fileio::file_name_path(cx, target_value)?);
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
        // GNU queries before renaming, so EEXIST names only the destination.
        Err(e) if e.raw_os_error() == Some(libc::EEXIST) => Err(already_exists(cx, target_value)),
        Err(e) => Err(errno(cx, "Renaming", &[source_value, target_value], e)),
    }
}

defsubr! { name = "make-temp-name", gnu = "fileio.c:830", effect = Global,
fn make_temp_name(cx, prefix: Value) {
    let bytes = cx.string_bytes(prefix)?;
    if bytes.contains(&0) { return Err(Fault::Wrong("filenamep", prefix)); }
    let text = cx.string_text(prefix)?;
    native_encoding(cx)?;
    // GNU GT_NOCREATE neither expands PREFIX nor dispatches name handlers.
    // Randomness is read from the kernel without creating any path.
    let mut random = host::random_source().map_err(|e| errno(cx, "Creating file name with prefix", &[prefix], e))?;
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
        match host::symlink_metadata(path) {
            Ok(_) => continue,
            Err(e) if e.raw_os_error() == Some(libc::ENOENT) => {
                return Ok(cx.make_string(candidate, !text.is_ascii()));
            }
            Err(e) => return Err(errno(cx, "Creating file name with prefix", &[prefix], e)),
        }
    }
    Err(errno(cx, "Creating file name with prefix", &[prefix], std::io::Error::from_raw_os_error(libc::EEXIST)))
}}

/// fileio.c check_file_access: expand, dispatch a handler, then faccessat.
/// The access mode, saved as a fixnum, names the operation.
fn check_file_access(cx: &mut Leaf<'_>, expanded: Value, saved: &[Value]) -> Next {
    let mode = saved[0].integer_value().expect("access mode") as i32;
    let operation = match mode {
        libc::F_OK => "file-exists-p",
        libc::X_OK => "file-executable-p",
        _ => "file-readable-p",
    };
    let (path, expanded) = as_file(cx, expanded)?;
    if let Some(call) = handler(cx, expanded, operation, vec![expanded])? { return call; }
    native_encoding(cx)?;
    Ok(Step::Done(if cx.filesave_access(path.as_os_str(), mode).is_ok() { Value::TRUE } else { Value::NIL }))
}

fn access(cx: &mut Leaf<'_>, filename: Value, mode: i32) -> Next {
    expanding(cx, filename, check_file_access, vec![Value::integer(mode.into()).unwrap()])
}

defsubr! { name = "file-exists-p", gnu = "fileio.c:3010", effect = Global,
fn file_exists_p(cx, filename: Value) -> Step { access(cx, filename, libc::F_OK) }}

defsubr! { name = "file-executable-p", gnu = "fileio.c:3022", effect = Global,
fn file_executable_p(cx, filename: Value) -> Step { access(cx, filename, libc::X_OK) }}

defsubr! { name = "file-readable-p", gnu = "fileio.c:3032", effect = Global,
fn file_readable_p(cx, filename: Value) -> Step { access(cx, filename, libc::R_OK) }}

defsubr! { name = "file-regular-p", gnu = "fileio.c:3359", effect = Global,
fn file_regular_p(cx, filename: Value) -> Step {
    expanding(cx, filename, file_regular_expanded, vec![])
}}
fn file_regular_expanded(cx: &mut Leaf<'_>, expanded: Value, saved: &[Value]) -> Next {
    // expand_and_dir_to_file, then stat following symlinks.
    let (path, expanded) = match dir_to_file(cx, expanded, file_regular_expanded, saved, 0, false)? { Ok(file) => file, Err(call) => return call };
    if let Some(call) = handler(cx, expanded, "file-regular-p", vec![expanded])? { return call; }
    native_encoding(cx)?;
    Ok(Step::Done(if host::metadata(&path).is_ok_and(|m| m.file_type().is_file()) { Value::TRUE } else { Value::NIL }))
}

defsubr! { name = "file-directory-p", gnu = "fileio.c:3185", effect = Global,
fn file_directory_p(cx, filename: Value) -> Step {
    expanding(cx, filename, file_directory_expanded, vec![])
}}
fn file_directory_expanded(cx: &mut Leaf<'_>, expanded: Value, saved: &[Value]) -> Next {
    // expand_and_dir_to_file, then a directory or a symlink to one.
    let (path, expanded) = match dir_to_file(cx, expanded, file_directory_expanded, saved, 0, false)? { Ok(file) => file, Err(call) => return call };
    if let Some(call) = handler(cx, expanded, "file-directory-p", vec![expanded])? { return call; }
    native_encoding(cx)?;
    Ok(Step::Done(if host::probe(host::Probe::IsDir, &path) { Value::TRUE } else { Value::NIL }))
}

/// gnulib gen_tempname's six random filename characters.
fn temp_name_letters() -> String {
    use std::hash::{BuildHasher, Hasher};
    const LETTERS: &[u8] = b"abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789";
    let mut hasher = std::collections::hash_map::RandomState::new().build_hasher();
    hasher.write_u128(host::clock().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_nanos()));
    let mut bits = hasher.finish();
    (0..6).map(|_| { let c = LETTERS[(bits % 62) as usize]; bits /= 62; c as char }).collect()
}

// Fmake_temp_file_internal: PREFIX, six random characters, SUFFIX, created
// atomically as a file (mode 0600, DIR-FLAG nil), a directory (0700) or,
// for DIR-FLAG 0, only a name that does not exist. TEXT, when non-empty,
// is written by `write-region'.
defsubr! { name = "make-temp-file-internal", gnu = "fileio.c:767", effect = Global,
fn make_temp_file_internal(cx, prefix: Value, dir_flag: Value, suffix: Value, text: Value) -> Step {
    use std::os::unix::fs::OpenOptionsExt;
    let (head, tail) = (cx.string_text(prefix)?, cx.string_text(suffix)?);
    native_encoding(cx)?;
    let kind = if dir_flag == Value::NIL { 0 } else if dir_flag.integer_value() == Some(0) { 2 } else { 1 };
    let mut attempt = 0;
    let path = loop {
        let path = format!("{head}{}{tail}", temp_name_letters());
        let made = match kind {
            0 => host::open_output(&path, host::OpenOptions::new().write(true).create_new(true).mode(0o600)).map(drop),
            1 => host::create_dir(&path, 0o700),
            _ => match host::symlink_metadata(&path) {
                Ok(_) => Err(std::io::Error::from_raw_os_error(libc::EEXIST)),
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
                Err(e) => Err(e),
            },
        };
        attempt += 1;
        match made {
            Ok(()) => break path,
            Err(e) if e.raw_os_error() == Some(libc::EEXIST) && attempt < 238328 => continue,
            Err(e) => {
                let operation = ["Creating file with prefix", "Creating directory with prefix", "Creating file name with prefix"][kind];
                return Err(errno(cx, operation, &[prefix], e));
            }
        }
    };
    let multibyte = cx.string_view(prefix)?.is_multibyte() || cx.string_view(suffix)?.is_multibyte();
    let file = cx.make_string(path.into_bytes(), multibyte);
    if cx.string_view(text).is_ok_and(|view| !view.bytes().is_empty()) {
        let write = cx.intern("write-region");
        return Ok(Step::Call { f: write, args: vec![text, Value::NIL, file], then: temp_file_written, saved: vec![file] });
    }
    Ok(Step::Done(file))
}}
fn temp_file_written(_: &mut Leaf<'_>, _: Value, saved: &[Value]) -> Next {
    Ok(Step::Done(saved[0]))
}

// Fdelete_file_internal: unlink the expanded FILENAME; a missing file is
// not an error. files.el's `delete-file' handles handlers and trash.
defsubr! { name = "delete-file-internal", gnu = "fileio.c:2602", effect = Global,
fn delete_file_internal(cx, filename: Value) -> Step {
    expanding(cx, filename, delete_file_internal_expanded, vec![])
}}
fn delete_file_internal_expanded(cx: &mut Leaf<'_>, expanded: Value, _: &[Value]) -> Next {
    let (path, expanded) = as_file(cx, expanded)?;
    native_encoding(cx)?;
    match cx.filesave_unlink(&path) {
        Err(e) if e.raw_os_error() != Some(libc::ENOENT) => Err(errno(cx, "Removing old name", &[expanded], e)),
        _ => Ok(Step::Done(Value::NIL)),
    }
}

// Fdelete_directory_internal: rmdir the directory file name of the
// expansion, which a directory-file-name handler may map elsewhere.
defsubr! { name = "delete-directory-internal", gnu = "fileio.c:2583", effect = Global,
fn delete_directory_internal(cx, directory: Value) -> Step {
    expanding(cx, directory, delete_directory_expanded, vec![])
}}
fn delete_directory_expanded(cx: &mut Leaf<'_>, expanded: Value, saved: &[Value]) -> Next {
    let (path, expanded) = match dir_to_file(cx, expanded, delete_directory_expanded, saved, 0, true)? { Ok(file) => file, Err(call) => return call };
    native_encoding(cx)?;
    host::remove_dir(&path).map_err(|e| errno(cx, "Removing directory", &[expanded], e))?;
    Ok(Step::Done(Value::NIL))
}

/// fileio.c expand_cp_target (FILE, NEWNAME) for a primitive whose N
/// arguments lead SAVED, FILE first and NEWNAME second, VALUE being the
/// last step's result: FILE expanded, the name whose last component is
/// taken (FILE's directory file name under DIRECTORY_FILE, as
/// `rename-file' takes it, else FILE), that component when NEWNAME is a
/// directory name, else nil, and the target expanded. ME continues each
/// step that calls Lisp; the expanded FILE and target, once both are known.
fn cp_stages(cx: &mut Leaf<'_>, value: Value, saved: &[Value], n: usize, directory_file: bool, me: crate::prim::Then)
    -> std::result::Result<std::result::Result<(Value, Value), Next>, Fault> {
    let mut s = saved.to_vec();
    s.push(value);
    let newname = s[1];
    let lisp = |step: Step, s: Vec<Value>| match step {
        Step::Done(value) => Ok(Some(value)),
        Step::Call { f, args, .. } => Err(Ok(Step::Call { f, args, then: me, saved: s })),
        _ => unreachable!("a file name primitive returns or calls its handler"),
    };
    loop {
        match s.len() - n {
            1 => {
                let step = if directory_file { super::savejourney::directory_file_name(cx, s[n])? } else { Step::Done(s[n]) };
                match lisp(step, s.clone()) { Ok(v) => s.extend(v), Err(next) => return Ok(Err(next)) }
            }
            2 => {
                if cx.string_bytes(newname)?.last() != Some(&b'/') {
                    s.push(Value::NIL);
                    return Ok(Err(expanding(cx, newname, me, s)));
                }
                let step = super::savejourney::file_name_nondirectory(cx, s[n + 1])?;
                match lisp(step, s.clone()) { Ok(v) => s.extend(v), Err(next) => return Ok(Err(next)) }
            }
            3 => return Ok(Err(super::expand_file_name::expanding(cx, s[n + 2], newname, me, s))),
            _ => return Ok(Ok((s[n], s[n + 3]))),
        }
    }
}

/// barf_or_query_if_file_exists for a NAME known to exist: nil signals,
/// an integer asks, which a leaf cannot do; any other value replaces.
fn may_replace(cx: &mut Leaf<'_>, ok: Value, name: Value, operation: &str) -> std::result::Result<(), Fault> {
    if ok == Value::NIL {
        return Err(already_exists(cx, name));
    }
    if ok.integer_value().is_some() {
        return Err(refusal(&format!("{operation} interactive overwrite confirmation")));
    }
    Ok(())
}

/// Create a name with CREATE; over an existing PATH, unlink it and try
/// once more when OK allows (Fadd_name_to_file, Fmake_symbolic_link).
fn create_name(
    cx: &mut Leaf<'_>,
    create: impl Fn() -> std::io::Result<()>,
    path: &std::path::Path,
    name: Value,
    ok: Value,
    operation: &str,
) -> std::result::Result<std::io::Result<()>, Fault> {
    match create() {
        Err(e) if e.raw_os_error() == Some(libc::EEXIST) => {
            may_replace(cx, ok, name, operation)?;
            let _ = cx.filesave_unlink(path);
            Ok(create())
        }
        result => Ok(result),
    }
}

/// The process umask, which GNU reads once at startup (fileio.c realmask).
fn umask() -> std::result::Result<u32, Fault> {
    let status = host::read_to_string("/proc/self/status").map_err(|e| Fault::Error(format!("VM2 umask: {e}")))?;
    status.lines().find_map(|line| line.strip_prefix("Umask:"))
        .and_then(|mask| u32::from_str_radix(mask.trim(), 8).ok())
        .ok_or_else(|| Fault::Error("VM2 umask: no Umask in /proc/self/status".into()))
}

// Fcopy_file: a regular FILE's bytes into NEWNAME, created exclusively with
// FILE's mode bits (only the owner's under PRESERVE-UID-GID) or truncated in
// place when OK-IF-ALREADY-EXISTS allows replacing it. GNU's mode, owner
// and time rules follow; extended ACLs are refused, not dropped.
defsubr! { name = "copy-file", gnu = "fileio.c:2252", effect = Global,
fn copy_file(cx, file: Value, newname: Value; optional ok_if_already_exists: Value, keep_time: Value,
             preserve_uid_gid: Value, preserve_permissions: Value) -> Step {
    expanding(cx, file, copy_staged, vec![file, newname, ok_if_already_exists, keep_time, preserve_uid_gid, preserve_permissions])
}}

fn copy_staged(cx: &mut Leaf<'_>, value: Value, saved: &[Value]) -> Next {
    use std::io::Write;
    use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
    let (file, newname) = match cp_stages(cx, value, saved, 6, false, copy_staged)? {
        Ok(names) => names,
        Err(next) => return next,
    };
    let [_, _, ok_if_already_exists, keep_time, preserve_uid_gid, preserve_permissions] = saved[..6] else { unreachable!() };
    let (source, target) = (super::fileio::file_name_path(cx, file)?, super::fileio::file_name_path(cx, newname)?);
    let args = vec![file, newname, ok_if_already_exists, keep_time, preserve_uid_gid, preserve_permissions];
    if let Some(call) = handler(cx, file, "copy-file", args.clone())? { return call; }
    if let Some(call) = handler(cx, newname, "copy-file", args)? { return call; }
    native_encoding(cx)?;
    let mut input = host::open_input(&source, libc::O_NONBLOCK)
        .map_err(|e| errno(cx, "Opening input file", &[file], e))?;
    let status = input.stat().map_err(|e| errno(cx, "Input file status", &[file], e))?;
    if !status.file_type().is_file() {
        let code = if status.file_type().is_dir() { libc::EISDIR } else { libc::EINVAL };
        return Err(errno(cx, "Non-regular file", &[file], std::io::Error::from_raw_os_error(code)));
    }
    if preserve_permissions != Value::NIL && input.has_acl() {
        return Err(refusal("copy-file ACL preservation"));
    }
    let new_mask = status.mode() & if preserve_uid_gid != Value::NIL { 0o700 } else { 0o777 };
    let created = host::open_output(&target, host::OpenOptions::new().write(true).create_new(true).mode(new_mask));
    let (mut output, already_exists) = match created {
        Ok(output) => (output, false),
        Err(e) if e.raw_os_error() == Some(libc::EEXIST) => {
            may_replace(cx, ok_if_already_exists, newname, "copy-file")?;
            let output = host::open_output(&target, host::OpenOptions::new().write(true).truncate(true))
                .map_err(|e| errno(cx, "Opening output file", &[newname], e))?;
            (output, true)
        }
        Err(e) => return Err(errno(cx, "Opening output file", &[newname], e)),
    };
    if already_exists {
        // GNU has already truncated the output when it finds it is the input.
        let existing = host::file_metadata(&output).map_err(|e| errno(cx, "Output file status", &[newname], e))?;
        if (existing.dev(), existing.ino()) == (status.dev(), status.ino()) {
            let same = std::io::Error::from_raw_os_error(0);
            return Err(errno(cx, "Input and output files are the same", &[file, newname], same));
        }
    }
    if !host::clone_file(&output, &input) {
        // Read and write the rest, whatever copy_file_range did, as GNU does.
        let mut buffer = vec![0; 1 << 16];
        loop {
            let count = match input.read(&mut buffer) {
                Ok(0) => break,
                Ok(count) => count,
                Err(e) if e.kind() == std::io::ErrorKind::Interrupted => continue,
                Err(e) => return Err(errno(cx, "Read error", &[file], e)),
            };
            output.write_all(&buffer[..count]).map_err(|e| errno(cx, "Write error", &[newname], e))?;
        }
    }
    let realmask = umask()?;
    let mut preserved = status.mode() & 0o7777;
    let mut default = status.mode() & 0o777 & !realmask;
    if preserve_uid_gid != Value::NIL
        && host::fchown(&output, Some(status.uid()), Some(status.gid())).is_err()
    {
        if host::fchown(&output, None, Some(status.gid())).is_ok() {
            preserved &= !0o4000;
        } else {
            // The group is wrong: drop set-id bits, give group the other bits.
            preserved = (preserved & !0o6070) | ((preserved & 7) << 3);
            default = (default & !0o70) | ((default & 7) << 3);
        }
    }
    let mode = if preserve_permissions != Value::NIL {
        Some(preserved)
    } else if already_exists || new_mask & !realmask == default {
        None
    } else {
        Some(default)
    };
    if let Some(mode) = mode {
        output.set_permissions(std::fs::Permissions::from_mode(mode))
            .map_err(|e| errno(cx, "Copying permissions to", &[newname], e))?;
    }
    if keep_time != Value::NIL {
        let times = status.accessed().and_then(|atime| Ok(std::fs::FileTimes::new()
            .set_accessed(atime).set_modified(status.modified()?)));
        if times.and_then(|times| output.set_times(times)).is_err() {
            let message = cx.string("Cannot set file date");
            let data = cx.cons(newname, Value::NIL);
            let data = cx.cons(message, data);
            return Err(Fault::Signal(cx.intern("file-date-error"), data));
        }
    }
    cx.fileio_close_output(output).map_err(|e| errno(cx, "Write error", &[newname], e))?;
    Ok(Step::Done(Value::NIL))
}

defsubr! { name = "add-name-to-file", gnu = "fileio.c:2851", effect = Global,
fn add_name_to_file(cx, file: Value, newname: Value; optional ok_if_already_exists: Value) -> Step {
    expanding(cx, file, add_name_staged, vec![file, newname, ok_if_already_exists])
}}

fn add_name_staged(cx: &mut Leaf<'_>, value: Value, saved: &[Value]) -> Next {
    let (file, newname) = match cp_stages(cx, value, saved, 3, false, add_name_staged)? {
        Ok(names) => names,
        Err(next) => return next,
    };
    let ok_if_already_exists = saved[2];
    let (source, target) = (super::fileio::file_name_path(cx, file)?, super::fileio::file_name_path(cx, newname)?);
    let args = vec![file, newname, ok_if_already_exists];
    if let Some(call) = handler(cx, file, "add-name-to-file", args.clone())? { return call; }
    if let Some(call) = handler(cx, newname, "add-name-to-file", args)? { return call; }
    native_encoding(cx)?;
    create_name(cx, || host::hard_link(&source, &target), &target, newname, ok_if_already_exists, "add-name-to-file")?
        .map_err(|e| errno(cx, "Adding new name", &[file, newname], e))?;
    Ok(Step::Done(Value::NIL))
}

// Fmake_symbolic_link: TARGET is stored literally; only an integer
// OK-IF-ALREADY-EXISTS expands its leading `~' or strips its leading `/:'.
defsubr! { name = "make-symbolic-link", gnu = "fileio.c:2907", effect = Global,
fn make_symbolic_link(cx, target: Value, linkname: Value; optional ok_if_already_exists: Value) -> Step {
    let bytes = cx.string_bytes(target)?;
    let args = vec![target, linkname, ok_if_already_exists];
    if ok_if_already_exists.integer_value().is_none() {
        link_staged(cx, target, &args)
    } else if bytes.first() == Some(&b'~') {
        expanding(cx, target, link_staged, args)
    } else if bytes.starts_with(b"/:") {
        let multibyte = cx.string_view(target)?.is_multibyte();
        let target = cx.make_string(bytes[2..].to_vec(), multibyte);
        link_staged(cx, target, &args)
    } else {
        link_staged(cx, target, &args)
    }
}}

/// TARGET as stored, then expand_cp_target (TARGET, LINKNAME).
fn link_staged(cx: &mut Leaf<'_>, value: Value, saved: &[Value]) -> Next {
    let (target, linkname) = match cp_stages(cx, value, saved, 3, false, link_staged)? {
        Ok(names) => names,
        Err(next) => return next,
    };
    let ok_if_already_exists = saved[2];
    let link = super::fileio::file_name_path(cx, linkname)?;
    if let Some(call) = handler(cx, linkname, "make-symbolic-link", vec![target, linkname, ok_if_already_exists])? {
        return call;
    }
    native_encoding(cx)?;
    if cx.string_bytes(target)?.contains(&0) {
        return Err(Fault::Wrong("filenamep", target));
    }
    let literal = super::fileio::file_name_path(cx, target)?;
    create_name(cx, || host::symlink(&literal, &link), &link, linkname, ok_if_already_exists, "make-symbolic-link")?
        .map_err(|e| errno(cx, "Making symbolic link", &[target, linkname], e))?;
    Ok(Step::Done(Value::NIL))
}

// Fset_file_times: both times to TIMESTAMP, or now when it is nil; any
// non-nil FLAG sets a symbolic link's own times. Returns t.
defsubr! { name = "set-file-times", gnu = "fileio.c:3741", effect = Global,
fn set_file_times(cx, filename: Value; optional timestamp: Value, flag: Value) -> Step {
    if timestamp != Value::NIL { super::time::timespec(cx, timestamp)?; }
    expanding_in_buffer(cx, filename, set_file_times_expanded, vec![timestamp, flag])
}}
fn set_file_times_expanded(cx: &mut Leaf<'_>, expanded: Value, saved: &[Value]) -> Next {
    let (timestamp, flag) = (saved[0], saved[1]);
    let time = if timestamp == Value::NIL { None } else { Some(super::time::timespec(cx, timestamp)?) };
    let (path, expanded) = as_file(cx, expanded)?;
    if let Some(call) = handler(cx, expanded, "set-file-times", vec![expanded, timestamp, flag])? { return call; }
    native_encoding(cx)?;
    cx.filesave_utimens(&path, time, flag != Value::NIL)
        .map_err(|e| errno(cx, "Setting file times", &[expanded], e))?;
    Ok(Step::Done(Value::TRUE))
}

// Faccess_file: the expanded FILENAME must be readable; else the error,
// STRING its operation, names FILENAME as given.
defsubr! { name = "access-file", gnu = "fileio.c:3078", effect = Global,
fn access_file(cx, filename: Value, string: Value) -> Step {
    cx.string_view(filename)?;
    expanding(cx, filename, access_expanded, vec![filename, string])
}}
fn access_expanded(cx: &mut Leaf<'_>, absname: Value, saved: &[Value]) -> Next {
    let [filename, string] = saved[..2] else { unreachable!() };
    let operation = cx.string_text(string)?;
    if let Some(call) = handler(cx, absname, "access-file", vec![absname, string])? { return call; }
    native_encoding(cx)?;
    let path = super::fileio::file_name_path(cx, absname)?;
    cx.filesave_access(path.as_os_str(), libc::R_OK).map_err(|e| errno(cx, &operation, &[filename], e))?;
    Ok(Step::Done(Value::NIL))
}

// Ffile_newer_than_file_p: both names through expand_and_dir_to_file, then
// either's handler; nil when FILE1 is missing, t when FILE2 is, else
// whether FILE1's modification time is later.
defsubr! { name = "file-newer-than-file-p", gnu = "fileio.c:3793", effect = Global,
fn file_newer_than_file_p(cx, file1: Value, file2: Value) -> Step {
    cx.string_view(file1)?;
    cx.string_view(file2)?;
    expanding(cx, file1, newer_staged, vec![file1, file2])
}}

/// SAVED: FILE1 FILE2, then each step's value: FILE1 expanded, as a file
/// name, FILE2 expanded, as a file name.
fn newer_staged(cx: &mut Leaf<'_>, value: Value, saved: &[Value]) -> Next {
    let mut s = saved.to_vec();
    s.push(value);
    loop {
        match s.len() {
            3 | 5 => {
                // expand_and_dir_to_file: a name ending in a slash, but for
                // the root, loses it through Fdirectory_file_name.
                let name = s[s.len() - 1];
                let bytes = cx.string_bytes(name)?;
                if bytes.len() < 2 || bytes.last() != Some(&b'/') { s.push(name); continue; }
                match super::savejourney::directory_file_name(cx, name)? {
                    Step::Done(file) => s.push(file),
                    Step::Call { f, args, .. } => return Ok(Step::Call { f, args, then: newer_staged, saved: s }),
                    _ => unreachable!("directory-file-name returns or calls its handler"),
                }
            }
            4 => return expanding(cx, s[1], newer_staged, s),
            _ => break,
        }
    }
    let (absname1, absname2) = (s[3], s[5]);
    let args = vec![absname1, absname2];
    if let Some(call) = handler(cx, absname1, "file-newer-than-file-p", args.clone())? { return call; }
    if let Some(call) = handler(cx, absname2, "file-newer-than-file-p", args)? { return call; }
    native_encoding(cx)?;
    // file_attribute_errno: a missing file is no error.
    let missing = |e: &std::io::Error| matches!(e.raw_os_error(), Some(libc::ENOENT | libc::ENOTDIR));
    let modified = |m: &std::fs::Metadata| { use std::os::unix::fs::MetadataExt; (m.mtime(), m.mtime_nsec()) };
    let first = match host::metadata(super::fileio::file_name_path(cx, absname1)?) {
        Ok(m) => modified(&m),
        Err(e) if missing(&e) => return Ok(Step::Done(Value::NIL)),
        Err(e) => return Err(errno(cx, "Getting attributes", &[absname1], e)),
    };
    let second = match host::metadata(super::fileio::file_name_path(cx, absname2)?) {
        Ok(m) => modified(&m),
        Err(e) if missing(&e) => return Ok(Step::Done(Value::TRUE)),
        Err(e) => return Err(errno(cx, "Getting attributes", &[absname2], e)),
    };
    Ok(Step::Done(if second < first { Value::TRUE } else { Value::NIL }))
}

// ACLs and SELinux contexts, as pinned GNU answers them: built with libacl
// (USE_ACL, HAVE_ACL_SET_FILE) and without libselinux. An access ACL is the
// file's `system.posix_acl_access' attribute, else its mode bits, in
// libacl's text forms (acl_to_text, acl_from_text).
const ACL_USER_OBJ: u16 = 0x01;
const ACL_USER: u16 = 0x02;
const ACL_GROUP_OBJ: u16 = 0x04;
const ACL_GROUP: u16 = 0x08;
const ACL_MASK: u16 = 0x10;
const ACL_OTHER: u16 = 0x20;
const ACL_UNDEFINED_ID: u32 = u32::MAX;

/// An ACL entry: its tag, permission bits (r 4, w 2, x 1) and qualifier.
type AclEntry = (u16, u16, u32);

/// The entries of an attribute value: version 2, then 8-byte entries.
fn acl_entries(value: &[u8]) -> Option<Vec<AclEntry>> {
    let (header, body) = (value.get(..4)?, &value[4..]);
    if u32::from_le_bytes(header.try_into().ok()?) != 2 || body.len() % 8 != 0 { return None; }
    Some(body.chunks(8).map(|e| (u16::from_le_bytes([e[0], e[1]]), u16::from_le_bytes([e[2], e[3]]), u32::from_le_bytes([e[4], e[5], e[6], e[7]]))).collect())
}

/// acl_from_mode: the ACL a file without an attribute has.
fn acl_from_mode(mode: u32) -> Vec<AclEntry> {
    let bits = |shift: u32| ((mode >> shift) & 7) as u16;
    vec![(ACL_USER_OBJ, bits(6), ACL_UNDEFINED_ID), (ACL_GROUP_OBJ, bits(3), ACL_UNDEFINED_ID), (ACL_OTHER, bits(0), ACL_UNDEFINED_ID)]
}

fn acl_perms(perm: u16) -> String {
    [(4, 'r'), (2, 'w'), (1, 'x')].iter().map(|&(bit, c)| if perm & bit != 0 { c } else { '-' }).collect()
}

/// acl_to_text: one line per entry, a named user or group by its name when
/// it has one, and the rights a mask withholds noted as `#effective'.
fn acl_text(entries: &[AclEntry]) -> String {
    let mask = entries.iter().find(|e| e.0 == ACL_MASK).map(|e| e.1);
    let mut text = String::new();
    for &(tag, perm, id) in entries {
        let user = || host::user_by_uid(id, 1 << 16).ok().and_then(|p| p.name).map_or(id.to_string(), |n| String::from_utf8_lossy(&n).into_owned());
        let group = || host::group_by_gid(id, 1 << 16).ok().map_or(id.to_string(), |n| String::from_utf8_lossy(&n).into_owned());
        let qualified = match tag {
            ACL_USER_OBJ => "user:".to_owned(),
            ACL_USER => format!("user:{}", user()),
            ACL_GROUP_OBJ => "group:".to_owned(),
            ACL_GROUP => format!("group:{}", group()),
            ACL_MASK => "mask:".to_owned(),
            _ => "other:".to_owned(),
        };
        text.push_str(&format!("{qualified}:{}", acl_perms(perm)));
        if let Some(mask) = mask.filter(|_| matches!(tag, ACL_USER | ACL_GROUP_OBJ | ACL_GROUP)) {
            if perm & !mask != 0 { text.push_str(&format!("\t#effective:{}", acl_perms(perm & mask))); }
        }
        text.push('\n');
    }
    text
}

/// acl_from_text: entries separated by commas or newlines, `#' comments,
/// long or one-letter tags, a user or group by name or number. None is
/// EINVAL. The entries come back in the order the kernel requires.
fn acl_parse(text: &str) -> Option<Vec<AclEntry>> {
    let mut entries = vec![];
    for line in text.lines() {
        let line = line.split('#').next().unwrap_or("");
        for entry in line.split(',').map(str::trim).filter(|e| !e.is_empty()) {
            let fields: Vec<&str> = entry.split(':').collect();
            let tag = match fields[0] { "user" | "u" => ACL_USER_OBJ, "group" | "g" => ACL_GROUP_OBJ, "mask" | "m" => ACL_MASK, "other" | "o" => ACL_OTHER, _ => return None };
            let (qualifier, perms) = match (tag, fields.len()) {
                (ACL_MASK | ACL_OTHER, 2) => ("", fields[1]),
                (_, 3) => (fields[1], fields[2]),
                _ => return None,
            };
            let mut perm = 0;
            for c in perms.trim().chars() {
                let bit = match c { 'r' => 4, 'w' => 2, 'x' => 1, '-' => 0, _ => return None };
                if perm & bit != 0 { return None; }
                perm |= bit;
            }
            let qualifier = qualifier.trim();
            let (tag, id) = match (tag, qualifier.is_empty()) {
                (_, true) => (tag, ACL_UNDEFINED_ID),
                (ACL_USER_OBJ, false) => (ACL_USER, qualifier.parse().ok().or_else(|| {
                    host::user_by_name(&std::ffi::CString::new(qualifier).ok()?, 1 << 16).ok().map(|p| p.uid)
                })?),
                (ACL_GROUP_OBJ, false) => (ACL_GROUP, qualifier.parse().ok().or_else(|| {
                    host::group_by_name(&std::ffi::CString::new(qualifier).ok()?, 1 << 16).ok()
                })?),
                _ => return None,
            };
            entries.push((tag, perm, id));
        }
    }
    entries.sort_by_key(|&(tag, _, id)| (tag, id));
    Some(entries)
}

defsubr! { name = "file-acl", gnu = "fileio.c:3534", effect = Global,
fn file_acl(cx, filename: Value) -> Step {
    expanding(cx, filename, acl_expanded, vec![])
}}
fn acl_expanded(cx: &mut Leaf<'_>, expanded: Value, saved: &[Value]) -> Next {
    let (path, absname) = match dir_to_file(cx, expanded, acl_expanded, saved, 0, false)? { Ok(file) => file, Err(call) => return call };
    if let Some(call) = handler(cx, absname, "file-acl", vec![absname])? { return call; }
    native_encoding(cx)?;
    // acl_get_file (ACL_TYPE_ACCESS): the attribute, else the mode bits.
    // A missing file, and an error acl_errno_valid rejects, answer nil.
    let entries = match host::posix_acl(std::path::Path::new(&path)) {
        Ok(Some(value)) => acl_entries(&value),
        Ok(None) => match host::metadata(&path) {
            Ok(meta) => { use std::os::unix::fs::MetadataExt; Some(acl_from_mode(meta.mode())) }
            Err(e) => return acl_errno(cx, "Getting ACLs", absname, e),
        },
        Err(e) => return acl_errno(cx, "Getting ACLs", absname, e),
    };
    let Some(entries) = entries else { return Ok(Step::Done(Value::NIL)) };
    Ok(Step::Done(cx.string(&acl_text(&entries))))
}

/// An ACL call's error: nil when the file is missing or acl_errno_valid
/// rejects it (gnulib: EBUSY, EINVAL, ENOSYS, ENOTSUP), else signalled.
fn acl_errno(cx: &mut Leaf<'_>, operation: &str, absname: Value, e: std::io::Error) -> Next {
    match e.raw_os_error() {
        Some(libc::ENOENT | libc::ENOTDIR | libc::EBUSY | libc::EINVAL | libc::ENOSYS | libc::ENOTSUP) => Ok(Step::Done(Value::NIL)),
        _ => Err(errno(cx, operation, &[absname], e)),
    }
}

defsubr! { name = "set-file-acl", gnu = "fileio.c:3580", effect = Global,
fn set_file_acl(cx, filename: Value, acl_string: Value) -> Step {
    expanding_in_buffer(cx, filename, set_acl_expanded, vec![acl_string])
}}
fn set_acl_expanded(cx: &mut Leaf<'_>, absname: Value, saved: &[Value]) -> Next {
    let acl_string = saved[0];
    if let Some(call) = handler(cx, absname, "set-file-acl", vec![absname, acl_string])? { return call; }
    if !matches!(cx.kind(acl_string), crate::prim::leaf::Kind::String | crate::prim::leaf::Kind::Bytes) { return Ok(Step::Done(Value::NIL)); }
    native_encoding(cx)?;
    // acl_from_text's EINVAL is not a valid ACL errno: nil.
    let Some(entries) = acl_parse(&cx.string_text(acl_string)?) else { return Ok(Step::Done(Value::NIL)) };
    let mut value = 2u32.to_le_bytes().to_vec();
    for (tag, perm, id) in entries { value.extend(tag.to_le_bytes().into_iter().chain(perm.to_le_bytes()).chain(id.to_le_bytes())); }
    let path = super::fileio::file_name_path(cx, absname)?;
    match host::set_posix_acl(&path, &value) {
        Ok(()) => Ok(Step::Done(Value::TRUE)),
        Err(e) if matches!(e.raw_os_error(), Some(libc::EBUSY | libc::EINVAL | libc::ENOSYS | libc::ENOTSUP)) => Ok(Step::Done(Value::NIL)),
        Err(e) => Err(errno(cx, "Setting ACL", &[absname], e)),
    }
}

// Pinned GNU has no libselinux: a context is (nil nil nil nil) and setting
// one does nothing, once a handler had its chance.
defsubr! { name = "file-selinux-context", gnu = "fileio.c:3392", effect = Global,
fn file_selinux_context(cx, filename: Value) -> Step {
    expanding(cx, filename, context_expanded, vec![])
}}
fn context_expanded(cx: &mut Leaf<'_>, expanded: Value, saved: &[Value]) -> Next {
    let (_, absname) = match dir_to_file(cx, expanded, context_expanded, saved, 0, false)? { Ok(file) => file, Err(call) => return call };
    if let Some(call) = handler(cx, absname, "file-selinux-context", vec![absname])? { return call; }
    let mut context = Value::NIL;
    for _ in 0..4 { context = cx.cons(Value::NIL, context); }
    Ok(Step::Done(context))
}

defsubr! { name = "set-file-selinux-context", gnu = "fileio.c:3446", effect = Global,
fn set_file_selinux_context(cx, filename: Value, context: Value) -> Step {
    expanding_in_buffer(cx, filename, set_context_expanded, vec![context])
}}
fn set_context_expanded(cx: &mut Leaf<'_>, absname: Value, saved: &[Value]) -> Next {
    if let Some(call) = handler(cx, absname, "set-file-selinux-context", vec![absname, saved[0]])? { return call; }
    Ok(Step::Done(Value::NIL))
}
