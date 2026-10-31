//! GNU dired.c:379 and fileio.c:2046, C-owned filename operations.
//! Lisp file-name handlers and substitute-env-in-file-name stay Lisp-owned.
use super::*;

fn refusal(message: &str) -> Fault {
    Fault::Error(format!("VM2 {message} is unsupported"))
}

fn directory_error(cx: &mut Leaf<'_>, operation: &str, path: Value, error: std::io::Error) -> Fault {
    let condition = match error.raw_os_error() {
        Some(libc::ENOENT) => "file-missing",
        Some(libc::EACCES) => "permission-denied",
        _ => "file-error",
    };
    let message = error.to_string();
    let message = message
        .rsplit_once(" (os error ")
        .map_or(message.as_str(), |(s, _)| s);
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
    let (_, matched) = directory_entries(cx, "directory-files", directory, full, pattern, nosort, count)?;
    let mut result = Value::NIL;
    for name in matched.into_iter().rev() {
        let multibyte = !name.is_ascii();
        let value = cx.make_string(name, multibyte);
        result = cx.cons(value, result);
    }
    Ok(result)
}}

// dired.c directory_files_internal with ATTRS: each entry as directory-files
// names it, with its `file-attributes' in ID-FORMAT, sorted by name.
defsubr! { name = "directory-files-and-attributes", gnu = "dired.c:420", effect = Global,
fn directory_files_and_attributes(cx, directory: Value; optional full: Value, pattern: Value, nosort: Value, id_format: Value, count: Value) {
    let operation = "directory-files-and-attributes";
    let (expanded, matched) = directory_entries(cx, operation, directory, full, pattern, nosort, count)?;
    let path = cx.string_view(expanded)?;
    let path = if path.is_multibyte() { path.bytes().to_vec() } else { crate::text::octets_as_multibyte(path.bytes()).0 };
    let mut result = Value::NIL;
    for name in matched.into_iter().rev() {
        let multibyte = !name.is_ascii();
        let value = cx.make_string(name.clone(), multibyte);
        let file = if full != Value::NIL { value } else {
            let mut file = path.strip_suffix(b"/").unwrap_or(&path).to_vec();
            file.push(b'/');
            file.extend_from_slice(&name);
            let multibyte = !file.is_ascii();
            cx.make_string(file, multibyte)
        };
        let file = super::fileio::file_name_path(cx, file)?;
        let attributes = super::savejourney::local_file_attributes(cx, &file, id_format)?;
        let entry = cx.cons(value, attributes);
        result = cx.cons(entry, result);
    }
    Ok(result)
}}

/// dired.c directory_files_internal's scan for OPERATION: DIRECTORY's
/// expansion and the entry names matching PATTERN, full names when FULL,
/// at most COUNT, sorted unless NOSORT.
fn directory_entries(cx: &mut Leaf<'_>, operation: &str, directory: Value, full: Value, pattern: Value, nosort: Value, count: Value)
    -> std::result::Result<(Value, Vec<Vec<u8>>), Fault> {
    if cx.string_bytes(directory)?.contains(&0) { return Err(Fault::Wrong("filenamep", directory)); }
    let (host_path, expanded) = super::fileio::expanded_path(cx, directory)?;
    super::fileio::refuse_handler(cx, expanded, operation)?;
    // The expansion's internal multibyte bytes, for the full names: a
    // unibyte expansion is an encoded name, decoded (DECODE_FILE).
    let path = cx.string_view(expanded)?;
    let path = if path.is_multibyte() { path.bytes().to_vec() } else { crate::text::octets_as_multibyte(path.bytes()).0 };
    let count = if count == Value::NIL { usize::MAX } else {
        let n = count.integer_value().filter(|n| *n >= 0).ok_or(Fault::Wrong("wholenump", count))?;
        usize::try_from(n).unwrap_or(usize::MAX)
    };
    if pattern != Value::NIL { cx.string_bytes(pattern)?; }
    utf8_file_names(cx, operation)?;
    // Open even when COUNT is zero: GNU validates the directory first.
    use std::os::unix::ffi::OsStrExt;
    let host_bytes = host_path.as_os_str().as_bytes();
    let directory_file = if host_bytes == b"/" { host_bytes } else { host_bytes.strip_suffix(b"/").unwrap_or(host_bytes) };
    let directory_file = std::path::Path::new(std::ffi::OsStr::from_bytes(directory_file));
    let directory_value = super::fileio::file_name_value(cx, directory_file.as_os_str().as_bytes());
    let entries = crate::host::read_dir(directory_file).map_err(|e| directory_error(cx, "Opening directory", directory_value, e))?;
    // Rust omits dot entries. Their enumeration order is not specified by GNU;
    // count is applied before sorting, and NOSORT returns reverse enumeration.
    let names = [std::ffi::OsString::from("."), std::ffi::OsString::from("..")]
        .into_iter().map(Ok).chain(entries.map(|entry| entry.map(|e| e.file_name())));
    // Materializing the iterator would read entries beyond COUNT; GNU stops
    // after the first additional matching entry.
    let mut matched: Vec<Vec<u8>> = Vec::new();
    for name in names {
        use std::os::unix::ffi::OsStrExt;
        let name = name.map_err(|e| directory_error(cx, "Reading directory", expanded, e))?;
        // DECODE_FILE: a name that is not UTF-8 keeps its bytes as raw-byte
        // characters.
        let name = crate::text::octets_as_multibyte(name.as_bytes()).0;
        if pattern != Value::NIL {
            let value = cx.make_string(name.clone(), !name.is_ascii());
            if cx.file_name_match(pattern, value)?.is_none() { continue; }
        }
        if matched.len() == count { break; }
        matched.push(if full != Value::NIL {
            let mut full = path.clone();
            if full.last() != Some(&b'/') { full.push(b'/'); }
            full.extend_from_slice(&name);
            full
        } else { name });
    }
    // Fsort with string-lessp: by character codes, raw bytes last.
    let chars = |bytes: &[u8]| crate::lstring::StrView { bytes, chars: 0, multibyte: true }.chars().collect::<Vec<_>>();
    if nosort == Value::NIL { matched.sort_by_key(|name| chars(name)); } else { matched.reverse(); }
    Ok((expanded, matched))
}

// Ffile_name_all_completions: the entries of DIRECTORY that begin with FILE
// (ignoring case under `completion-ignore-case') and match every
// `completion-regexp-list' regexp, a directory's name ending in a slash, in
// reverse enumeration order. "./" and "../" are entries too.
defsubr! { name = "file-name-all-completions", gnu = "dired.c:486", effect = Global,
fn file_name_all_completions(cx, file: Value, directory: Value) -> Step {
    let (path, directory) = match completion_directory(cx, "file-name-all-completions", file, directory, None)? {
        Ok(found) => found,
        Err(handled) => return Ok(handled),
    };
    let mut result = Value::NIL;
    for candidate in completion_candidates(cx, file, &path, directory)? {
        if !candidate.regexps { continue; }
        let mut name = candidate.name;
        if candidate.directoryp { name.push(b'/'); }
        let multibyte = !name.is_ascii();
        let value = cx.make_string(name, multibyte);
        result = cx.cons(value, result);
    }
    Ok(Step::Done(result))
}}

// Ffile_name_completion: the longest common prefix of the completions of FILE
// in DIRECTORY, t for a sole exact match, nil for none. The scan is here;
// file_name_completion's loop over the matches, which calls PREDICATE with
// `default-directory' bound to DIRECTORY, is Lisp (rlisp/vm/dired.el).
defsubr! { name = "file-name-completion", gnu = "dired.c:451", effect = Global,
fn file_name_completion(cx, file: Value, directory: Value; optional predicate: Value) -> Step {
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
    for candidate in completion_candidates(cx, file, &path, directory)? {
        let excludable = excludable(cx, &candidate, file_bytes, &ignored, fold)?;
        let name = candidate.value;
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
}}

const LISP_HALF: &str = include_str!("../../../../rlisp/vm/dired.el");
const COMPLETION_LOOP: &str = "romacs--file-name-completion";

fn completion_loop(cx: &mut Leaf<'_>, _: Value, saved: &[Value]) -> Next {
    Ok(Step::Call { f: cx.intern(COMPLETION_LOOP), args: saved.to_vec(), then: completed_by_handler, saved: vec![] })
}

/// DIRECTORY expanded, as its path and a string, after FILE's and its
/// checks; or, as Err, the call to a file-name handler for OPERATION.
fn completion_directory(cx: &mut Leaf<'_>, operation: &str, file: Value, directory: Value, predicate: Option<Value>)
    -> std::result::Result<std::result::Result<(std::path::PathBuf, Value), Step>, Fault> {
    if cx.string_bytes(directory)?.contains(&0) { return Err(Fault::Wrong("filenamep", directory)); }
    let (path, directory) = super::fileio::expanded_path(cx, directory)?;
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

/// A directory entry that begins with the file being completed: its
/// decoded name (DECODE_FILE: not-UTF-8 bytes as raw-byte characters) as
/// GNU's internal bytes and as a string.
struct Candidate {
    name: Vec<u8>,
    value: Value,
    directoryp: bool,
    /// It matches every `completion-regexp-list' regexp.
    regexps: bool,
}

/// dired.c file_name_completion's scan of PATH: the entries that begin
/// with FILE (ignoring case under `completion-ignore-case'), in
/// enumeration order, "." and ".." first.
fn completion_candidates(cx: &mut Leaf<'_>, file: Value, path: &std::path::Path, directory: Value) -> std::result::Result<Vec<Candidate>, Fault> {
    let fold = super::fileio::var(cx, "completion-ignore-case")? != Value::NIL;
    // FILE as host bytes (ENCODE_FILE), compared with each entry's bytes
    // before decoding, as file_name_completion compares them.
    let prefix = cx.file_name_bytes(file)?;
    let prefix_chars = cx.string_view(file)?.chars;
    use std::os::unix::ffi::OsStrExt;
    let host_bytes = path.as_os_str().as_bytes();
    let directory_file = if host_bytes == b"/" { host_bytes } else { host_bytes.strip_suffix(b"/").unwrap_or(host_bytes) };
    let entries = crate::host::read_dir(std::path::Path::new(std::ffi::OsStr::from_bytes(directory_file))).map_err(|e| directory_error(cx, "Opening directory", directory, e))?;
    // Rust omits the dot entries, which GNU sees first on Linux file systems.
    let dots = [".", ".."].map(|name| Ok((std::ffi::OsString::from(name), Some(true))));
    let entries = entries.map(|entry| entry.map(|e| (e.file_name(), e.file_type().ok().filter(|t| !std::fs::FileType::is_symlink(t)).as_ref().map(std::fs::FileType::is_dir))));
    let regexps = super::fileio::var(cx, "completion-regexp-list")? != Value::NIL;
    // Compiled at the first entry that reaches them, as GNU checks them.
    let mut matcher = None;
    let mut out = vec![];
    for entry in dots.into_iter().chain(entries) {
        let (name, directoryp) = entry.map_err(|e| directory_error(cx, "Reading directory", directory, e))?;
        let os_name = std::os::unix::ffi::OsStringExt::into_vec(name);
        if os_name.len() < prefix.len() || (!fold && !os_name.starts_with(&prefix)) { continue; }
        let name = crate::text::octets_as_multibyte(&os_name).0;
        let value = cx.make_string(name.clone(), !name.is_ascii());
        if fold && super::string_compare::prefix(cx, value, file, prefix_chars, true)?.0 != std::cmp::Ordering::Equal {
            continue;
        }
        // A symbolic link, or an entry of unknown type, is a directory when
        // NAME/ resolves.
        let directoryp = directoryp.unwrap_or_else(|| {
            let mut probe = directory_file.to_vec();
            probe.push(b'/');
            probe.extend_from_slice(&os_name);
            probe.push(b'/');
            cx.filesave_access(std::os::unix::ffi::OsStrExt::from_bytes(&probe), libc::F_OK).is_ok()
        });
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
        out.push(Candidate { name, value, directoryp, regexps: matched });
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
    if candidate.directoryp && matches!(candidate.name.as_slice(), b"." | b"..") { return Ok(true); }
    if candidate.name.len() <= file_bytes { return Ok(false); }
    let name: Vec<u32> = super::string_compare::codes(&cx.string_view(candidate.value)?);
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
            candidate.name[candidate.name.len() - length..] == bytes[..length]
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
