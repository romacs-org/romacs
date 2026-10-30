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
fn directory_files(cx, directory: Value; optional full: Value, pattern: Value, nosort: Value, count: Value) -> Step {
    if cx.string_bytes(directory)?.contains(&0) { return Err(Fault::Wrong("filenamep", directory)); }
    super::fileio::expanding(cx, directory, files_expanded, vec![full, pattern, nosort, count, Value::NIL, Value::NIL])
}}

// Each entry with its attributes in ID-FORMAT: (NAME . ATTRIBUTES).
defsubr! { name = "directory-files-and-attributes", gnu = "dired.c:407", effect = Global,
fn directory_files_and_attributes(cx, directory: Value; optional full: Value, pattern: Value, nosort: Value,
                                  id_format: Value, count: Value) -> Step {
    if cx.string_bytes(directory)?.contains(&0) { return Err(Fault::Wrong("filenamep", directory)); }
    super::fileio::expanding(cx, directory, files_expanded, vec![full, pattern, nosort, count, Value::TRUE, id_format])
}}

/// dired.c:404-411 and 440-450: the expanded DIRECTORY's handler lists,
/// else directory_files_internal, whose Fdirectory_file_name calls its own.
/// SAVED: FULL PATTERN NOSORT COUNT ATTRS ID-FORMAT.
fn files_expanded(cx: &mut Leaf<'_>, directory: Value, saved: &[Value]) -> Next {
    let [full, pattern, nosort, count, attrs, id_format] = saved[..6] else { unreachable!() };
    let call = if attrs == Value::NIL {
        super::fileio::handler(cx, directory, "directory-files", vec![directory, full, pattern, nosort, count])?
    } else {
        super::fileio::handler(cx, directory, "directory-files-and-attributes", vec![directory, full, pattern, nosort, id_format, count])?
    };
    if let Some(call) = call { return call; }
    match super::savejourney::directory_file_name(cx, directory)? {
        Step::Done(file) => list_directory(cx, directory, file, saved),
        Step::Call { f, args, .. } => {
            let saved = saved.iter().copied().chain([directory]).collect();
            Ok(Step::Call { f, args, then: files_dir_named, saved })
        }
        _ => unreachable!("directory-file-name returns or calls its handler"),
    }
}

fn files_dir_named(cx: &mut Leaf<'_>, file: Value, saved: &[Value]) -> Next {
    list_directory(cx, saved[6], file, &saved[..6])
}

fn list_directory(cx: &mut Leaf<'_>, directory: Value, file: Value, saved: &[Value]) -> Next {
    let [full, pattern, nosort, count, attrs, id_format] = saved[..6] else { unreachable!() };
    let path = cx.string_text(directory)?;
    let directory_file = cx.string_text(file)?;
    let count = if count == Value::NIL { usize::MAX } else {
        let n = count.integer_value().filter(|n| *n >= 0).ok_or(Fault::Wrong("wholenump", count))?;
        usize::try_from(n).unwrap_or(usize::MAX)
    };
    if pattern != Value::NIL { cx.string_text(pattern)?; }
    utf8_file_names(cx, "directory-files")?;
    // Open even when COUNT is zero: GNU validates the directory first.
    let entries = crate::host::read_dir(&directory_file).map_err(|e| directory_error(cx, "Opening directory", &directory_file, e))?;
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
        let shown = if full != Value::NIL {
            format!("{}{name}", if path.ends_with('/') { path.clone() } else { format!("{path}/") })
        } else { name.clone() };
        matched.push((shown, name));
    }
    if nosort == Value::NIL { matched.sort(); } else { matched.reverse(); }
    let mut result = Value::NIL;
    for (shown, name) in matched.into_iter().rev() {
        let mut value = cx.make_string(shown.as_bytes().to_vec(), !shown.is_ascii());
        if attrs != Value::NIL {
            // dired.c file_attributes: fstatat on the directory's entry.
            let attributes = super::savejourney::native_attributes(cx, &format!("{directory_file}/{name}"), id_format)?;
            value = cx.cons(value, attributes);
        }
        result = cx.cons(value, result);
    }
    Ok(Step::Done(result))
}

// Ffile_name_all_completions: the entries of DIRECTORY that begin with FILE
// (ignoring case under `completion-ignore-case') and match every
// `completion-regexp-list' regexp, a directory's name ending in a slash, in
// reverse enumeration order. "./" and "../" are entries too.
defsubr! { name = "file-name-all-completions", gnu = "dired.c:486", effect = Global,
fn file_name_all_completions(cx, file: Value, directory: Value) -> Step {
    completion_expanding(cx, directory, all_completions_expanded, vec![file])
}}

fn all_completions_expanded(cx: &mut Leaf<'_>, directory: Value, saved: &[Value]) -> Next {
    let file = saved[0];
    let (path, _) = match completion_directory(cx, "file-name-all-completions", file, directory, None)? {
        Ok(found) => found,
        Err(handled) => return Ok(handled),
    };
    let mut result = Value::NIL;
    for candidate in completion_candidates(cx, file, &path)? {
        if !candidate.regexps { continue; }
        let name = if candidate.directoryp { format!("{}/", candidate.name) } else { candidate.name };
        let value = cx.make_string(name.as_bytes().to_vec(), !name.is_ascii());
        result = cx.cons(value, result);
    }
    Ok(Step::Done(result))
}

// Ffile_name_completion: the longest common prefix of the completions of FILE
// in DIRECTORY, t for a sole exact match, nil for none. The scan is here;
// file_name_completion's loop over the matches, which calls PREDICATE with
// `default-directory' bound to DIRECTORY, is Lisp (rlisp/vm/dired.el).
defsubr! { name = "file-name-completion", gnu = "dired.c:451", effect = Global,
fn file_name_completion(cx, file: Value, directory: Value; optional predicate: Value) -> Step {
    completion_expanding(cx, directory, completion_expanded, vec![file, predicate])
}}

fn completion_expanded(cx: &mut Leaf<'_>, directory: Value, saved: &[Value]) -> Next {
    let [file, predicate] = saved[..2] else { unreachable!() };
    let (path, directory) = match completion_directory(cx, "file-name-completion", file, directory, Some(predicate))? {
        Ok(found) => found,
        Err(handled) => return Ok(handled),
    };
    let fold = super::fileio::var(cx, "completion-ignore-case")? != Value::NIL;
    let mut ignored = vec![];
    let mut tail = super::fileio::var(cx, "completion-ignored-extensions")?;
    while let Some((element, rest)) = cx.pair(tail) {
        if cx.string_view(element).is_ok() { ignored.push(element); }
        tail = rest;
    }
    let file_bytes = cx.string_bytes(file)?.len();
    let mut entries = vec![];
    for candidate in completion_candidates(cx, file, &path)? {
        let excludable = excludable(cx, &candidate, file_bytes, &ignored, fold)?;
        let name = cx.make_string(candidate.name.as_bytes().to_vec(), !candidate.name.is_ascii());
        let flag = |b: bool| if b { Value::TRUE } else { Value::NIL };
        entries.push(cx.vector(&[name, flag(excludable), flag(candidate.regexps), flag(candidate.directoryp)]));
    }
    let mut list = Value::NIL;
    for entry in entries.into_iter().rev() { list = cx.cons(entry, list); }
    let args = vec![file, directory, list, predicate];
    let helper = cx.intern(COMPLETION_LOOP);
    if cx.symbol_function(helper)? != Value::NIL {
        return Ok(Step::Call { f: helper, args, then: completed_by_handler, saved: vec![] });
    }
    let source = cx.string(LISP_HALF);
    let form = cx.read_from_string(source, Value::NIL, Value::NIL)?;
    let (form, _) = cx.pair(form).expect("read-from-string returns (OBJECT . POSITION)");
    Ok(Step::Call { f: cx.intern("eval"), args: vec![form, Value::TRUE], then: completion_loop, saved: args })
}

const LISP_HALF: &str = include_str!("../../../../rlisp/vm/dired.el");
const COMPLETION_LOOP: &str = "romacs--file-name-completion";

fn completion_loop(cx: &mut Leaf<'_>, _: Value, saved: &[Value]) -> Next {
    Ok(Step::Call { f: cx.intern(COMPLETION_LOOP), args: saved.to_vec(), then: completed_by_handler, saved: vec![] })
}

/// Fexpand_file_name (DIRECTORY, Qnil) for THEN.
fn completion_expanding(cx: &mut Leaf<'_>, directory: Value, then: crate::prim::Then, saved: Vec<Value>) -> Next {
    if cx.string_bytes(directory)?.contains(&0) { return Err(Fault::Wrong("filenamep", directory)); }
    super::fileio::expanding(cx, directory, then, saved)
}

/// The expanded DIRECTORY as its path and itself, after FILE's and its
/// checks; or, as Err, the call to a file-name handler for OPERATION.
fn completion_directory(cx: &mut Leaf<'_>, operation: &str, file: Value, directory: Value, predicate: Option<Value>)
    -> std::result::Result<std::result::Result<(String, Value), Step>, Fault> {
    let path = cx.string_text(directory)?;
    let symbol = cx.intern(operation);
    for name in [directory, file] {
        cx.string_view(name)?;
        let handler = super::fileio::find_file_name_handler(cx, name, symbol)?;
        if handler != Value::NIL {
            let args = [symbol, file, directory].into_iter().chain(predicate).collect();
            return Ok(Err(Step::Call { f: handler, args, then: completed_by_handler, saved: vec![] }));
        }
    }
    utf8_file_names(cx, operation)?;
    Ok(Ok((path, directory)))
}

/// A directory entry that begins with the file being completed.
struct Candidate {
    name: String,
    directoryp: bool,
    /// It matches every `completion-regexp-list' regexp.
    regexps: bool,
}

/// dired.c file_name_completion's scan of PATH: the entries that begin
/// with FILE (ignoring case under `completion-ignore-case'), in
/// enumeration order, "." and ".." first.
fn completion_candidates(cx: &mut Leaf<'_>, file: Value, path: &str) -> std::result::Result<Vec<Candidate>, Fault> {
    let fold = super::fileio::var(cx, "completion-ignore-case")? != Value::NIL;
    let prefix = cx.string_bytes(file)?;
    let prefix_chars = cx.string_view(file)?.chars;
    let directory_file = if path == "/" { "/" } else { path.trim_end_matches('/') };
    let entries = crate::host::read_dir(directory_file).map_err(|e| directory_error(cx, "Opening directory", path, e))?;
    // Rust omits the dot entries, which GNU sees first on Linux file systems.
    let dots = [".", ".."].map(|name| Ok((std::ffi::OsString::from(name), Some(true))));
    let entries = entries.map(|entry| entry.map(|e| (e.file_name(), e.file_type().ok().filter(|t| !std::fs::FileType::is_symlink(t)).as_ref().map(std::fs::FileType::is_dir))));
    let regexps = super::fileio::var(cx, "completion-regexp-list")? != Value::NIL;
    // Compiled at the first entry that reaches them, as GNU checks them.
    let mut matcher = None;
    let mut out = vec![];
    for entry in dots.into_iter().chain(entries) {
        let (name, directoryp) = entry.map_err(|e| directory_error(cx, "Reading directory", path, e))?;
        let name = std::os::unix::ffi::OsStringExt::into_vec(name);
        if name.len() < prefix.len() || (!fold && !name.starts_with(&prefix)) { continue; }
        let name = String::from_utf8(name).map_err(|_| refusal("non-UTF-8 directory entry"))?;
        let value = cx.make_string(name.as_bytes().to_vec(), !name.is_ascii());
        if fold && super::string_compare::prefix(cx, value, file, prefix_chars, true)?.0 != std::cmp::Ordering::Equal {
            continue;
        }
        // A symbolic link, or an entry of unknown type, is a directory when
        // NAME/ resolves.
        let directoryp = directoryp.unwrap_or_else(|| cx.filesave_access(&format!("{directory_file}/{name}/"), libc::F_OK).is_ok());
        let matched = !regexps || {
            if matcher.is_none() {
                let (mut tail, mut patterns) = (super::fileio::var(cx, "completion-regexp-list")?, vec![]);
                while let Some((pattern, rest)) = cx.pair(tail) { patterns.push(pattern); tail = rest; }
                matcher = Some(cx.completion_matcher(&patterns, fold, true)?);
            }
            let compiled = matcher.as_mut().unwrap();
            match compiled {
                Some(compiled) => cx.completion_matches(compiled, value)?,
                None => regexps_match(cx, value, fold)?,
            }
        };
        out.push(Candidate { name, directoryp, regexps: matched });
    }
    Ok(out)
}

/// Whether VALUE matches every `completion-regexp-list' regexp, each
/// compiled for it alone.
fn regexps_match(cx: &mut Leaf<'_>, value: Value, fold: bool) -> std::result::Result<bool, Fault> {
    let mut regexps = super::fileio::var(cx, "completion-regexp-list")?;
    while let Some((regexp, rest)) = cx.pair(regexps) {
        if !cx.file_completion_regexp_match(regexp, value, fold)? { return Ok(false); }
        regexps = rest;
    }
    Ok(true)
}

/// Whether file_name_completion may leave CANDIDATE out when anything
/// else matches: "." and "..", and a longer name than FILE's FILE_BYTES
/// that ends in one of the IGNORED extensions (a directory's ending in "/").
fn excludable(cx: &mut Leaf<'_>, candidate: &Candidate, file_bytes: usize, ignored: &[Value], fold: bool)
    -> std::result::Result<bool, Fault> {
    if candidate.directoryp && matches!(candidate.name.as_str(), "." | "..") { return Ok(true); }
    if candidate.name.len() <= file_bytes { return Ok(false); }
    let name: Vec<u32> = candidate.name.chars().map(u32::from).collect();
    for &extension in ignored {
        let bytes = cx.string_bytes(extension)?;
        let mut codes = super::string_compare::codes(&cx.string_view(extension)?);
        if candidate.directoryp {
            // Only an extension ending in a slash names directories.
            if bytes.len() < 2 || bytes[bytes.len() - 1] != b'/' { continue; }
            codes.pop();
        }
        let length = bytes.len() - usize::from(candidate.directoryp);
        if candidate.name.len() < length { continue; }
        let tail = if fold {
            let Some(skip) = name.len().checked_sub(codes.len()) else { continue };
            super::string_compare::compare(cx, &name[skip..], &codes, true)?.0 == std::cmp::Ordering::Equal
        } else {
            candidate.name.as_bytes()[candidate.name.len() - length..] == bytes[..length]
        };
        if tail { return Ok(true); }
    }
    Ok(false)
}

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
