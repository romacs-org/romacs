//! GNU dired.c:379 and fileio.c:2046, C-owned filename operations.
//! Lisp file-name handlers and substitute-env-in-file-name stay Lisp-owned.
use super::*;

fn refusal(message: &str) -> Fault {
    Fault::Error(format!("VM2 {message} is unsupported"))
}

fn directory_error(cx: &mut Leaf<'_>, operation: &str, path: &str, error: std::io::Error) -> Fault {
    let condition = match error.raw_os_error() {
        Some(libc::ENOENT) => "file-missing",
        Some(libc::EACCES) => "permission-denied",
        _ => "file-error",
    };
    let message = error.to_string();
    let message = message
        .rsplit_once(" (os error ")
        .map_or(message.as_str(), |(s, _)| s);
    let path = cx.string(path);
    let data = cx.cons(path, Value::NIL);
    let message = cx.string(message);
    let data = cx.cons(message, data);
    let operation = cx.string(operation);
    let data = cx.cons(operation, data);
    Fault::Signal(cx.intern(condition), data)
}

/// File names decode as UTF-8 only; other file-name codings are refused.
pub(super) fn utf8_file_names(cx: &mut Leaf<'_>, operation: &str) -> std::result::Result<(), Fault> {
    for variable in ["file-name-coding-system", "default-file-name-coding-system"] {
        let symbol = cx.intern(variable);
        let coding = cx.variable_value(symbol)?.unwrap_or(Value::NIL);
        if coding != Value::NIL && !cx.symbol_name(coding).is_ok_and(|name| matches!(name.as_str(), "utf-8" | "utf-8-unix" | "undecided" | "undecided-unix")) {
            return Err(refusal(&format!("{operation} non-UTF-8 filename coding")));
        }
    }
    Ok(())
}

defsubr! { name = "directory-files", gnu = "dired.c:379", effect = Global,
fn directory_files(cx, directory: Value; optional full: Value, pattern: Value, nosort: Value, count: Value) {
    if cx.string_bytes(directory)?.contains(&0) { return Err(Fault::Wrong("filenamep", directory)); }
    let path = super::fileio::expand_file_name(cx, directory)?;
    let expanded = cx.string(&path);
    super::fileio::refuse_handler(cx, expanded, "directory-files")?;
    let count = if count == Value::NIL { usize::MAX } else {
        let n = count.integer_value().filter(|n| *n >= 0).ok_or(Fault::Wrong("wholenump", count))?;
        usize::try_from(n).unwrap_or(usize::MAX)
    };
    if pattern != Value::NIL { cx.string_text(pattern)?; }
    utf8_file_names(cx, "directory-files")?;
    // Open even when COUNT is zero: GNU validates the directory first.
    let directory_file = if path == "/" { "/" } else { path.trim_end_matches('/') };
    let entries = crate::host::read_dir(directory_file).map_err(|e| directory_error(cx, "Opening directory", directory_file, e))?;
    // Rust omits dot entries. Their enumeration order is not specified by GNU;
    // count is applied before sorting, and NOSORT returns reverse enumeration.
    let names = [std::ffi::OsString::from("."), std::ffi::OsString::from("..")]
        .into_iter().map(Ok).chain(entries.map(|entry| entry.map(|e| e.file_name())));
    // Materializing the iterator would read entries beyond COUNT; GNU stops
    // after the first additional matching entry.
    let mut matched = Vec::new();
    for name in names {
        let name = name.map_err(|e| directory_error(cx, "Reading directory", &path, e))?
            .into_string().map_err(|_| refusal("non-UTF-8 directory entry"))?;
        if pattern != Value::NIL {
            let value = cx.string(&name);
            if cx.file_name_match(pattern, value)?.is_none() { continue; }
        }
        if matched.len() == count { break; }
        matched.push(if full != Value::NIL {
            format!("{}{name}", if path.ends_with('/') { path.clone() } else { format!("{path}/") })
        } else { name });
    }
    if nosort == Value::NIL { matched.sort(); } else { matched.reverse(); }
    let mut result = Value::NIL;
    for name in matched.into_iter().rev() {
        let value = cx.make_string(name.as_bytes().to_vec(), !name.is_ascii());
        result = cx.cons(value, result);
    }
    Ok(result)
}}

// Ffile_name_all_completions: the entries of DIRECTORY that begin with FILE
// (ignoring case under `completion-ignore-case') and match every
// `completion-regexp-list' regexp, a directory's name ending in a slash, in
// reverse enumeration order. "./" and "../" are entries too.
defsubr! { name = "file-name-all-completions", gnu = "dired.c:486", effect = Global,
fn file_name_all_completions(cx, file: Value, directory: Value) -> Step {
    if cx.string_bytes(directory)?.contains(&0) { return Err(Fault::Wrong("filenamep", directory)); }
    let path = super::fileio::expand_file_name(cx, directory)?;
    let directory = cx.string(&path);
    let operation = cx.intern("file-name-all-completions");
    for name in [directory, file] {
        cx.string_view(name)?;
        let handler = super::fileio::find_file_name_handler(cx, name, operation)?;
        if handler != Value::NIL {
            return Ok(Step::Call { f: handler, args: vec![operation, file, directory], then: completed_by_handler, saved: vec![] });
        }
    }
    utf8_file_names(cx, "file-name-all-completions")?;
    let fold = super::fileio::var(cx, "completion-ignore-case")? != Value::NIL;
    let prefix = cx.string_bytes(file)?;
    let prefix_chars = cx.string_view(file)?.chars;
    let directory_file = if path == "/" { "/" } else { path.trim_end_matches('/') };
    let entries = crate::host::read_dir(directory_file).map_err(|e| directory_error(cx, "Opening directory", &path, e))?;
    // Rust omits the dot entries, which GNU sees first on Linux file systems.
    let dots = [".", ".."].map(|name| Ok((std::ffi::OsString::from(name), Some(true))));
    let entries = entries.map(|entry| entry.map(|e| (e.file_name(), e.file_type().ok().filter(|t| !std::fs::FileType::is_symlink(t)).as_ref().map(std::fs::FileType::is_dir))));
    let mut result = Value::NIL;
    for entry in dots.into_iter().chain(entries) {
        let (name, directoryp) = entry.map_err(|e| directory_error(cx, "Reading directory", &path, e))?;
        let name = std::os::unix::ffi::OsStringExt::into_vec(name);
        if name.len() < prefix.len() || (!fold && !name.starts_with(&prefix)) { continue; }
        let name = String::from_utf8(name).map_err(|_| refusal("non-UTF-8 directory entry"))?;
        let mut value = cx.make_string(name.as_bytes().to_vec(), !name.is_ascii());
        if fold && super::string_compare::prefix(cx, value, file, prefix_chars, true)?.0 != std::cmp::Ordering::Equal {
            continue;
        }
        // A symbolic link, or an entry of unknown type, is a directory when
        // NAME/ resolves.
        let directoryp = directoryp.unwrap_or_else(|| cx.filesave_access(&format!("{directory_file}/{name}/"), libc::F_OK).is_ok());
        let mut regexps = super::fileio::var(cx, "completion-regexp-list")?;
        let mut matched = true;
        while let Some((regexp, rest)) = cx.pair(regexps) {
            if !cx.file_completion_regexp_match(regexp, value, fold)? { matched = false; break; }
            regexps = rest;
        }
        if !matched { continue; }
        if directoryp {
            value = cx.make_string(format!("{name}/").into_bytes(), !name.is_ascii());
        }
        result = cx.cons(value, result);
    }
    Ok(Step::Done(result))
}}

fn completed_by_handler(_: &mut Leaf<'_>, result: Value, _: &[Value]) -> Next {
    Ok(Step::Done(result))
}

defsubr! { name = "substitute-in-file-name", gnu = "fileio.c:2046", effect = Global,
fn substitute_in_file_name(cx, filename: Value) -> Step {
    let multibyte = cx.string_view(filename)?.is_multibyte();
    let operation = cx.intern("substitute-in-file-name");
    let handler = super::fileio::find_file_name_handler(cx, filename, operation)?;
    if handler != Value::NIL {
        return Ok(Step::Call {
            f: handler, args: vec![operation, filename],
            then: substituted_by_handler, saved: vec![],
        });
    }
    // GNU strips embedded absolute names before calling the Lisp helper.
    let filename = strip_embedded_absolute(cx, filename, multibyte)?;
    let helper = cx.intern("substitute-env-in-file-name");
    if cx.fboundp(helper) {
        return Ok(Step::Call {
            f: helper,
            args: vec![filename],
            then: substituted_environment,
            saved: vec![filename, Value::integer(i64::from(multibyte)).unwrap()],
        });
    }
    Ok(Step::Done(filename))
}}

fn substituted_by_handler(cx: &mut Leaf<'_>, result: Value, _: &[Value]) -> Next {
    if !matches!(cx.kind(result), Kind::String | Kind::Bytes) {
        return Err(Fault::Error("Invalid handler in `file-name-handler-alist'".into()));
    }
    Ok(Step::Done(result))
}

fn substituted_environment(cx: &mut Leaf<'_>, result: Value, saved: &[Value]) -> Next {
    cx.string_view(result)?;
    // GNU does not re-enter the helper (or handlers) after substitution.
    let result = if result == saved[0] {
        result
    } else {
        strip_embedded_absolute(cx, result, saved[1].integer_value() == Some(1))?
    };
    Ok(Step::Done(result))
}

fn strip_embedded_absolute(
    cx: &mut Leaf<'_>,
    filename: Value,
    multibyte: bool,
) -> std::result::Result<Value, Fault> {
    let bytes = cx.string_bytes(filename)?;
    let mut start = 0;
    for i in 1..bytes.len() {
        if bytes[i - 1] != b'/' {
            continue;
        }
        if bytes[i] == b'/' || (bytes[i] == b'~' && (i + 1 == bytes.len() || matches!(bytes[i + 1], b'/' | 0)))
        {
            start = i;
        } else if bytes[i] == b'~' {
            return Err(refusal("named-user embedded filename lookup"));
        }
    }
    if start == 0 {
        return Ok(filename);
    }
    Ok(cx.make_string(bytes[start..].to_vec(), multibyte))
}

defsubr! { name = "file-name-absolute-p", gnu = "fileio.c:2967", effect = Reads,
fn file_name_absolute_p(cx, filename: Value) {
    let bytes = cx.string_bytes(filename)?;
    // SSDATA is read as a C string.
    let name = bytes.split(|&b| b == 0).next().unwrap_or(&[]);
    let absolute = match name {
        [b'/', ..] | [b'~'] | [b'~', b'/', ..] => true,
        [b'~', rest @ ..] => user_homedir(rest),
        _ => false,
    };
    Ok(if absolute { Value::TRUE } else { Value::NIL })
}}

/// GNU user_homedir: NAME, up to its first slash, is a login whose home
/// directory is absolute.
fn user_homedir(name: &[u8]) -> bool {
    let login = name.split(|&b| b == b'/').next().unwrap_or(&[]);
    std::str::from_utf8(login).ok().and_then(crate::user_db::by_name)
        .and_then(|account| account.home)
        .is_some_and(|home| home.starts_with('/'))
}

defsubr! { name = "directory-name-p", gnu = "fileio.c:703", effect = Pure,
fn directory_name_p(cx, name: Value) {
    let view = cx.string_view(name).map_err(|_| Fault::Wrong("stringp", name))?;
    Ok(if view.bytes().last() == Some(&b'/') { Value::TRUE } else { Value::NIL })
}}
