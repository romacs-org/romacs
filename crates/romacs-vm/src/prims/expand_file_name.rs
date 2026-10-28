//! GNU fileio.c Fexpand_file_name (fileio.c:994) for Unix names.
//!
//! The defsubr is a T2 state machine: each point where GNU calls a file name
//! handler, or expands DEFAULT-DIRECTORY recursively, is a `Step::Call'.
//! Native callers run the same machine through [`expand`], which resolves
//! the recursive expansions in Rust and stops at the first handler call, so a
//! file primitive expands natively exactly when GNU would call no Lisp.
use super::*;
use super::fileio::{find_file_name_handler, var};
use crate::prim::leaf::Kind;

fn stringp(cx: &Leaf<'_>, v: Value) -> bool {
    matches!(cx.kind(v), Kind::String | Kind::Bytes)
}

/// CHECK_STRING and CHECK_STRING_NULL_BYTES.
fn check_name(cx: &Leaf<'_>, name: Value) -> std::result::Result<Vec<u8>, Fault> {
    let bytes = cx.string_bytes(name)?;
    if bytes.contains(&0) { return Err(Fault::Wrong("filenamep", name)); }
    Ok(bytes)
}

/// The expand-file-name handler selected by MATCHED, called with NAME and
/// DEFAULT; its value must be a string.
fn handled(cx: &mut Leaf<'_>, matched: Value, name: Value, default: Value) -> std::result::Result<Option<Next>, Fault> {
    let operation = cx.intern("expand-file-name");
    let handler = find_file_name_handler(cx, matched, operation)?;
    if handler == Value::NIL { return Ok(None); }
    Ok(Some(Ok(Step::Call { f: handler, args: vec![operation, name, default], then: handler_value, saved: vec![] })))
}

fn handler_value(cx: &mut Leaf<'_>, value: Value, _: &[Value]) -> Next {
    if stringp(cx, value) { Ok(Step::Done(value)) }
    else { Err(Fault::Error("Invalid handler in ‘file-name-handler-alist’".into())) }
}

fn recurse(cx: &mut Leaf<'_>, name: Value, default: Value, then: crate::prim::Then, saved: Vec<Value>) -> Next {
    let f = cx.intern("expand-file-name");
    Ok(Step::Call { f, args: vec![name, default], then, saved })
}

fn root(cx: &mut Leaf<'_>) -> Value { cx.make_string(b"/".to_vec(), false) }

defsubr! { name = "expand-file-name", gnu = "fileio.c:994", effect = Global,
fn expand_file_name(cx, name: Value; optional default_directory: Value) -> Step {
    check_name(cx, name)?;
    if let Some(call) = handled(cx, name, name, default_directory)? { return call; }
    let mut default = default_directory;
    if default == Value::NIL {
        let dir = var(cx, "default-directory")?;
        if stringp(cx, dir) {
            if cx.string_bytes(dir)?.first() == Some(&b'/') {
                check_name(cx, dir)?;
                default = dir;
            } else {
                // A relative or `~' buffer directory expands against an
                // absolute invocation-directory, else the root.
                let invocation = var(cx, "invocation-directory")?;
                let absdir = if stringp(cx, invocation) && cx.string_bytes(invocation)?.first() == Some(&b'/') {
                    invocation
                } else { root(cx) };
                return recurse(cx, dir, absdir, with_default, vec![name]);
            }
        }
    }
    with_default(cx, default, &[name])
}}

fn with_default(cx: &mut Leaf<'_>, default: Value, saved: &[Value]) -> Next {
    let name = saved[0];
    let default = if stringp(cx, default) { default } else { root(cx) };
    if let Some(call) = handled(cx, default, name, default)? { return call; }
    // A relative DEFAULT-DIRECTORY is itself expanded first; the EQ test
    // avoids infinite recursion.
    if !cx.eq(default, name) && cx.string_bytes(default)?.first() != Some(&b'/') {
        return recurse(cx, default, Value::NIL, reexpanded, vec![name]);
    }
    finish(cx, name, default)
}

fn reexpanded(cx: &mut Leaf<'_>, default: Value, saved: &[Value]) -> Next {
    let name = saved[0];
    if let Some(call) = handled(cx, default, name, default)? { return call; }
    finish(cx, name, default)
}

fn finish(cx: &mut Leaf<'_>, name: Value, default: Value) -> Next {
    let (result, unchanged) = canonical_value(cx, name, default)?;
    if unchanged { return Ok(Step::Done(result)); }
    // The result can name a remote file again ("/foo/../user@host:/x").
    if let Some(call) = handled(cx, result, result, default)? { return call; }
    Ok(Step::Done(result))
}

/// NAME against DEFAULT with GNU's multibyte reconciliation, and whether
/// NAME was already canonical and absolute (GNU then returns it at once).
fn canonical_value(cx: &mut Leaf<'_>, name: Value, default: Value) -> std::result::Result<(Value, bool), Fault> {
    let name_view = cx.string_view(name)?;
    let default_view = cx.string_view(default)?;
    let nm = name_view.bytes();
    let dir = default_view.bytes();
    let mut multibyte = name_view.is_multibyte();
    let mut name = name;
    // Keep the bytes; give the result the multibyteness GNU settles on (an
    // ASCII side converts, else both become unibyte). When both are ASCII
    // NAME keeps its own: VM2 reads every string literal as multibyte where
    // GNU reads an ASCII one as unibyte, so converting would only make
    // ordinary names unibyte copies that are no longer EQ to the argument.
    if multibyte != default_view.is_multibyte() && !(nm.is_ascii() && dir.is_ascii()) {
        if multibyte {
            if nm.is_ascii() || !dir.is_ascii() {
                name = cx.make_string(nm.to_vec(), false);
                multibyte = false;
            }
        } else if nm.is_ascii() {
            name = cx.make_string(nm.to_vec(), true);
            multibyte = true;
        }
    }
    let home = if nm.first() == Some(&b'~') && matches!(nm.get(1), None | Some(b'/')) {
        let home = home_directory(cx)?;
        Some(home_bytes(cx, home, &mut multibyte)?)
    } else { None };
    Ok(match canonical(nm, dir, home.as_deref()) {
        None => (name, true),
        // GNU would make an ill-formed multibyte string here, from a
        // multibyte HOME and a unibyte non-ASCII NAME.
        Some(bytes) if multibyte && !well_formed(&bytes) => {
            return Err(Fault::Error("VM2 expand-file-name ill-formed multibyte result is unsupported".into()));
        }
        Some(bytes) => (cx.make_string(bytes, multibyte), false),
    })
}

/// GNU get_homedir: HOME, else the passwd entry of LOGNAME, USER or the
/// real user, else "". The variables come from `process-environment' and
/// are read as C strings, ending at a NUL. A home that is not absolute,
/// an empty HOME included, is taken relative to the working directory
/// Emacs started in (emacs_wd, environment::startup_directory), as
/// splice_dir_file joins them.
fn home_directory(cx: &mut Leaf<'_>) -> std::result::Result<Vec<u8>, Fault> {
    let c_string = |mut bytes: Vec<u8>| { if let Some(nul) = bytes.iter().position(|&b| b == 0) { bytes.truncate(nul) } bytes };
    let home = match super::environment::egetenv(cx, "HOME")? {
        Some(home) => c_string(home),
        None => {
            let mut account = None;
            for variable in ["LOGNAME", "USER"] {
                let Some(user) = super::environment::egetenv(cx, variable)? else { continue };
                account = std::str::from_utf8(&c_string(user)).ok().and_then(crate::user_db::by_name);
                if account.is_some() { break; }
            }
            match account.or_else(|| crate::user_db::by_uid(crate::user_db::real_uid())).and_then(|account| account.home) {
                Some(home) => home.into_bytes(),
                None => return Ok(Vec::new()),
            }
        }
    };
    if home.first() == Some(&b'/') {
        return Ok(home);
    }
    let wd = crate::environment::startup_directory().ok_or_else(|| Fault::Error("$HOME is relative to unknown directory".into()))?;
    let mut spliced = wd.to_vec();
    if spliced.last().is_some_and(|&b| b != b'/') { spliced.push(b'/'); }
    spliced.extend_from_slice(&home);
    Ok(spliced)
}

/// fileio.c:1395: HOME's bytes as build_string makes them, decoded with
/// DECODE_FILE for a multibyte result; a multibyte HOME makes the result
/// multibyte.
fn home_bytes(cx: &mut Leaf<'_>, home: Vec<u8>, multibyte: &mut bool) -> std::result::Result<Vec<u8>, Fault> {
    let home_multibyte = crate::environment::multibyte(&home);
    if *multibyte && !home_multibyte && !home.is_ascii() {
        let unibyte = cx.make_string(home, false);
        let decoded = decode_file(cx, unibyte)?;
        return cx.string_bytes(decoded);
    }
    if home_multibyte { *multibyte = true; }
    Ok(home)
}

/// GNU decode_file_name: NAME decoded with `file-name-coding-system', else
/// `default-file-name-coding-system'. Undecoded bytes cannot join a
/// multibyte result, so no coding system is refused.
fn decode_file(cx: &mut Leaf<'_>, name: Value) -> std::result::Result<Value, Fault> {
    let mut coding = var(cx, "file-name-coding-system")?;
    if coding == Value::NIL { coding = var(cx, "default-file-name-coding-system")?; }
    if coding == Value::NIL {
        return Err(Fault::Error("VM2 expand-file-name undecoded non-ASCII HOME is unsupported".into()));
    }
    // convert_string_nocopy does not record the coding system.
    let last = cx.intern("last-coding-system-used");
    let recorded = cx.variable_value(last)?.unwrap_or(Value::NIL);
    let decoded = cx.coding_convert_string(name, coding, Value::TRUE, Value::NIL, false)?;
    cx.variable_set(last, recorded)?;
    Ok(decoded)
}

fn well_formed(mut bytes: &[u8]) -> bool {
    while let Some(&byte) = bytes.first() {
        let width = if byte.is_ascii() { 1 } else {
            match crate::lstring::decode_char(bytes) { Some((_, width)) => width, None => return false }
        };
        bytes = &bytes[width..];
    }
    true
}

/// GNU user_homedir: the absolute home of login NAME.
fn user_home(name: &[u8]) -> Option<Vec<u8>> {
    if name.is_empty() { return None; }
    let home = crate::user_db::by_name(std::str::from_utf8(name).ok()?)?.home?;
    home.starts_with('/').then(|| home.into_bytes())
}

/// Fexpand_file_name's Unix text rules (fileio.c:1316-1745) for NM against
/// the absolute or already expanded DIR, HOME being get_homedir's value when
/// NM starts with `~'. None means NM is returned unchanged.
pub(crate) fn canonical(nm: &[u8], dir: &[u8], home: Option<&[u8]>) -> Option<Vec<u8>> {
    let at = |s: &[u8], i: usize| s.get(i).copied();
    let end_or_slash = |c: Option<u8>| matches!(c, None | Some(b'/'));
    if nm.first() == Some(&b'/') {
        let lose = (0..nm.len()).any(|i| nm[i] == b'/' && (
            (at(nm, i + 1) == Some(b'.') && (end_or_slash(at(nm, i + 2))
                || (at(nm, i + 2) == Some(b'.') && end_or_slash(at(nm, i + 3)))))
            || (at(nm, i + 1) == Some(b'/') && (i != 0 || at(nm, i + 2) == Some(b'/')))));
        if !lose { return None; }
    }
    let mut nm = nm;
    let mut newdir: Option<Vec<u8>> = None;
    if nm.first() == Some(&b'~') {
        if end_or_slash(at(nm, 1)) {
            newdir = Some(home.unwrap_or_default().to_vec());
            nm = &nm[1..];
        } else {
            let end = nm.iter().position(|&c| c == b'/').unwrap_or(nm.len());
            if let Some(home) = user_home(&nm[1..end]) {
                newdir = Some(home);
                nm = &nm[end..];
            }
        }
    }
    if nm.first() != Some(&b'/') && newdir.is_none() { newdir = Some(dir.to_vec()); }
    let mut target = Vec::with_capacity(nm.len() + newdir.as_ref().map_or(0, Vec::len) + 2);
    if let Some(newdir) = newdir {
        // Ignore a slash ending NEWDIR, unless NEWDIR is "/" or "//".
        let mut length = newdir.len();
        while length > 1 && newdir[length - 1] == b'/' && !(length == 2 && newdir[0] == b'/') { length -= 1; }
        if nm.first().is_none_or(|&c| c == b'/') {
            target.extend_from_slice(&newdir[..length]);
        } else if length == 0 {
            target.extend_from_slice(b"./");
        } else {
            target.extend_from_slice(&newdir[..length]);
            if newdir[length - 1] != b'/' { target.push(b'/'); }
        }
    }
    target.extend_from_slice(nm);
    // Remove `//', `/.' and `/foo/..'; a leading "//" stays.
    let mut out: Vec<u8> = Vec::with_capacity(target.len());
    let mut p = 0;
    while p < target.len() {
        if target[p] != b'/' {
            out.push(target[p]);
            p += 1;
        } else if at(&target, p + 1) == Some(b'.') && end_or_slash(at(&target, p + 2)) {
            if out.is_empty() && at(&target, p + 2).is_none() { out.push(b'/'); }
            p += 2;
        } else if at(&target, p + 1) == Some(b'.') && at(&target, p + 2) == Some(b'.')
            && !out.is_empty() && end_or_slash(at(&target, p + 3))
        {
            let mut first = 0;
            while let Some(c) = out.pop() {
                first = c;
                if c == b'/' { break; }
            }
            // Keep the initial slash only when this is the whole name.
            if out.is_empty() && first == b'/' && at(&target, p + 3).is_none() { out.push(b'/'); }
            p += 3;
        } else if at(&target, p + 1) == Some(b'/') && (p != 0 || at(&target, p + 2) == Some(b'/')) {
            p += 1;
        } else {
            out.push(b'/');
            p += 1;
        }
    }
    Some(out)
}

/// Fexpand_file_name (NAME, DEFAULT) without calling Lisp: the expansion,
/// or None where GNU would call a file name handler.
pub(crate) fn expand(cx: &mut Leaf<'_>, name: Value, default: Value) -> std::result::Result<Option<Value>, Fault> {
    let step = expand_file_name(cx, name, default);
    drive(cx, step)
}

fn drive(cx: &mut Leaf<'_>, mut step: Next) -> std::result::Result<Option<Value>, Fault> {
    let recursion = cx.intern("expand-file-name");
    loop {
        match step? {
            Step::Done(value) => return Ok(Some(value)),
            // A recursive expansion passes a string, a handler its operation.
            Step::Call { f, args, then, saved } if f == recursion && stringp(cx, args[0]) => {
                let Some(value) = expand(cx, args[0], args[1])? else { return Ok(None) };
                step = then(cx, value, &saved);
            }
            _ => return Ok(None),
        }
    }
}

/// Fexpand_file_name (NAME, DEFAULT) for a primitive continuing in THEN with
/// the expansion: native when no handler applies, else through Lisp.
pub(super) fn expanding(cx: &mut Leaf<'_>, name: Value, default: Value, then: crate::prim::Then, saved: Vec<Value>) -> Next {
    match expand(cx, name, default)? {
        Some(value) => then(cx, value, &saved),
        None => recurse(cx, name, default, then, saved),
    }
}

#[cfg(test)]
mod tests {
    use super::canonical;

    fn expand(nm: &str, dir: &str) -> String {
        let out = canonical(nm.as_bytes(), dir.as_bytes(), Some(b"/home/u")).unwrap_or_else(|| nm.as_bytes().to_vec());
        String::from_utf8(out).unwrap()
    }

    #[test]
    fn unix_rules() {
        for (nm, dir, want) in [
            ("/tmp/x/", "/d/", "/tmp/x/"), ("a", "/d/", "/d/a"), ("a/", "/d", "/d/a/"),
            ("", "/d/", "/d"), (".", "/d/", "/d"), ("..", "/d/e/", "/d"), ("../..", "/d/", "/.."),
            ("/x/../..", "/", "/.."), ("//x", "/", "//x"), ("///x", "/", "/x"), ("/a//b/./c/", "/", "/a/b/c/"),
            ("~", "/d/", "/home/u"), ("~/", "/d/", "/home/u/"), ("~/x/../y", "/d/", "/home/u/y"),
            ("/.", "/", "/"), ("/..", "/", "/.."), ("a", "//", "//a"), ("a", "", "./a"),
        ] {
            assert_eq!(expand(nm, dir), want, "{nm:?} in {dir:?}");
        }
    }
}
