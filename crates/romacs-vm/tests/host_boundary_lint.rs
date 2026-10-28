//! Host boundary lint, rules 1 and 2 of the world-snapshot design (§6.1).
//!
//! Rule 1: outside `crates/romacs-vm/src/host.rs`, no source in the runner's
//! dependency closure calls a host query or effect API directly. Rule 2: only
//! the environment owners name the environment slots, and only the per-buffer
//! accessor looks up `default-directory`'s storage.
//!
//! The scan is textual over the compiled module tree of each crate: comments
//! are removed, string contents are hidden from rule 1, and `#[cfg(test)]`
//! items are skipped, since test code builds no worlds. Every other match
//! needs an entry in `host_boundary_allow.txt`, one `PATH:LINE API REASON`
//! per line, where LINE or API may be `*`. An entry that matches nothing
//! fails, so the list cannot rot. `ROMACS_HOST_LINT_BLESS=1` rewrites the
//! list for the current tree: kept reasons follow their (path, API), and new
//! matches are marked `pending`, which fails until it is migrated or given a
//! reason.
use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

const CRATES: &[&str] = &[
    "crates/romacs-vm",
    "crates/romacs-face",
    "crates/romacs-layout-input",
    "crates/romacs-display-protocol",
    "crates/romacs-regex-engine",
    "gnu-tests/runner",
];
const BOUNDARY: &str = "crates/romacs-vm/src/host.rs";
const ALLOW: &str = "crates/romacs-vm/tests/host_boundary_allow.txt";
const FIXTURES: &str = "crates/romacs-vm/tests/host_boundary_fixtures";
const ENVIRONMENT_OWNERS: &[&str] = &[
    "crates/romacs-vm/src/environment.rs",
    "crates/romacs-vm/src/prims/environment.rs",
    "crates/romacs-vm/src/prims/user_identity.rs",
];
const BUFFER_ACCESSOR: &[&str] = &["crates/romacs-vm/src/forwarded.rs"];

/// One forbidden spelling. A needle that starts with an identifier character
/// must not continue an identifier on its left, nor follow `host::`; unless
/// `prefix`, one that ends with an identifier character must not continue on
/// its right. `strings` matches inside string literals (rule 2).
struct Api {
    name: &'static str,
    needle: &'static str,
    prefix: bool,
    unless_after: &'static str,
    unless_before: &'static str,
    unless_pure: bool,
    strings: bool,
    owners: &'static [&'static str],
}

const fn api(name: &'static str, needle: &'static str) -> Api {
    Api { name, needle, prefix: false, unless_after: "", unless_before: "", unless_pure: false, strings: false, owners: &[] }
}
/// A `fs::` path naming only data types and extension traits queries nothing;
/// `File::open` and `File::create` are their own APIs.
const fn fs_path(needle: &'static str) -> Api {
    Api { unless_pure: true, ..api("std::fs", needle) }
}
const PURE_FS: &[&str] = &["File", "Metadata", "Permissions", "FileType", "FileTimes", "DirEntry", "ReadDir"];
const fn libc_prefix(name: &'static str, needle: &'static str) -> Api {
    Api { prefix: true, ..api(name, needle) }
}
/// Metadata and FileType answer `is_file`/`is_dir` from data already read;
/// spelled through `file_type()`, they are not a path probe.
const fn path_probe(name: &'static str, needle: &'static str) -> Api {
    Api { unless_after: "file_type()", ..api(name, needle) }
}
const fn slot(name: &'static str, needle: &'static str, owners: &'static [&'static str]) -> Api {
    Api { strings: true, owners, ..api(name, needle) }
}

const APIS: &[Api] = &[
    fs_path("fs::"),
    Api { unless_before: "::", ..api("std::fs", "std::fs") },
    api("File::open", "File::open"),
    api("File::create", "File::create"),
    api("OpenOptions", "OpenOptions"),
    api("read_dir", "read_dir"),
    api("metadata", "metadata("),
    api("symlink_metadata", "symlink_metadata"),
    api("canonicalize", ".canonicalize("),
    path_probe("Path::exists", ".exists()"),
    path_probe("Path::is_file", ".is_file()"),
    path_probe("Path::is_dir", ".is_dir()"),
    api("std::path::absolute", "path::absolute"),
    api("std::env::var", "env::var"),
    api("std::env::var_os", "env::var_os"),
    api("std::env::vars", "env::vars"),
    api("std::env::vars_os", "env::vars_os"),
    api("std::env::current_dir", "env::current_dir"),
    api("std::env::current_exe", "env::current_exe"),
    api("std::env::temp_dir", "env::temp_dir"),
    api("std::process::id", "process::id"),
    api("SystemTime::now", "SystemTime::now"),
    api("Instant::now", "Instant::now"),
    api("available_parallelism", "available_parallelism"),
    libc_prefix("libc::getpw*", "libc::getpw"),
    libc_prefix("libc::getgr*", "libc::getgr"),
    api("libc::getpid", "libc::getpid"),
    api("libc::getppid", "libc::getppid"),
    api("libc::getuid", "libc::getuid"),
    api("libc::geteuid", "libc::geteuid"),
    api("libc::getgid", "libc::getgid"),
    api("libc::getegid", "libc::getegid"),
    api("libc::gethostname", "libc::gethostname"),
    api("libc::uname", "libc::uname"),
    api("libc::sysconf", "libc::sysconf"),
    api("libc::sched_getaffinity", "libc::sched_getaffinity"),
    api("libc::statvfs", "libc::statvfs"),
    api("libc::clock_gettime", "libc::clock_gettime"),
    api("libc::getrandom", "libc::getrandom"),
    slot("slot:process-environment", "\"process-environment\"", ENVIRONMENT_OWNERS),
    slot("slot:initial-environment", "\"initial-environment\"", ENVIRONMENT_OWNERS),
    slot("slot:exec-path", "\"exec-path\"", ENVIRONMENT_OWNERS),
    slot("slot:default-directory", "lookup(\"default-directory\")", BUFFER_ACCESSOR),
];

fn ident(b: u8) -> bool {
    b.is_ascii_alphanumeric() || b == b'_'
}

/// Two views of one source, byte for byte the same length with the same
/// newlines: `text` has comments blanked; `code` also blanks string and
/// character literal contents, keeping each string's two quotes.
struct Views {
    code: Vec<u8>,
    text: Vec<u8>,
}

fn views(src: &str) -> Views {
    let s = src.as_bytes();
    let mut code = s.to_vec();
    let mut text = s.to_vec();
    let blank = |v: &mut Vec<u8>, from: usize, to: usize| {
        for b in &mut v[from..to] {
            if *b != b'\n' { *b = b' '; }
        }
    };
    let mut i = 0;
    while i < s.len() {
        let rest = &s[i..];
        if rest.starts_with(b"//") {
            let end = rest.iter().position(|&b| b == b'\n').map_or(s.len(), |n| i + n);
            blank(&mut code, i, end);
            blank(&mut text, i, end);
            i = end;
        } else if rest.starts_with(b"/*") {
            let (mut depth, mut j) = (0, i);
            while j < s.len() {
                if s[j..].starts_with(b"/*") { depth += 1; j += 2; }
                else if s[j..].starts_with(b"*/") { depth -= 1; j += 2; if depth == 0 { break; } }
                else { j += 1; }
            }
            blank(&mut code, i, j);
            blank(&mut text, i, j);
            i = j;
        } else if let Some((open, hashes)) = raw_string_start(s, i) {
            let close = (open + 1..s.len())
                .find(|&j| s[j] == b'"' && s[j + 1..].iter().take(hashes).filter(|&&b| b == b'#').count() == hashes)
                .unwrap_or(s.len() - 1);
            blank(&mut code, i, s.len().min(close + 1 + hashes));
            code[open] = b'"';
            code[close] = b'"';
            i = close + 1 + hashes;
        } else if s[i] == b'"' {
            let mut j = i + 1;
            while j < s.len() && s[j] != b'"' {
                j += if s[j] == b'\\' { 2 } else { 1 };
            }
            blank(&mut code, i + 1, j.min(s.len()));
            i = j + 1;
        } else if s[i] == b'\'' {
            i = match char_literal_end(src, i) {
                Some(end) => { blank(&mut code, i, end); end }
                None => i + 1,
            };
        } else if ident(s[i]) {
            while i < s.len() && ident(s[i]) { i += 1; }
        } else {
            i += 1;
        }
    }
    Views { code, text }
}

/// `r"`, `r#"`, `br"` …, at an identifier start: the opening quote and the
/// number of hashes.
fn raw_string_start(s: &[u8], i: usize) -> Option<(usize, usize)> {
    if i > 0 && ident(s[i - 1]) { return None; }
    let mut j = i;
    if s.get(j) == Some(&b'b') { j += 1; }
    if s.get(j) != Some(&b'r') { return None; }
    j += 1;
    let hashes = s[j..].iter().take_while(|&&b| b == b'#').count();
    (s.get(j + hashes) == Some(&b'"')).then_some((j + hashes, hashes))
}

/// The end of a character literal at `i`, or None for a lifetime or label.
fn char_literal_end(src: &str, i: usize) -> Option<usize> {
    let s = src.as_bytes();
    if s.get(i + 1) == Some(&b'\\') {
        let mut j = i + 3;
        while j < s.len() && s[j] != b'\'' { j += 1; }
        return Some(j + 1);
    }
    let c = src[i + 1..].chars().next()?;
    let after = i + 1 + c.len_utf8();
    (s.get(after) == Some(&b'\'')).then_some(after + 1)
}

#[derive(Clone, Copy, PartialEq, Debug)]
enum Tok {
    Ident(usize, usize),
    Punct(u8),
    Str(usize, usize),
}

fn tokens(code: &[u8]) -> Vec<(usize, Tok)> {
    let mut out = Vec::new();
    let mut i = 0;
    while i < code.len() {
        let b = code[i];
        if b.is_ascii_whitespace() {
            i += 1;
        } else if ident(b) {
            let start = i;
            while i < code.len() && ident(code[i]) { i += 1; }
            out.push((start, Tok::Ident(start, i)));
        } else if b == b'"' {
            let close = (i + 1..code.len()).find(|&j| code[j] == b'"').unwrap_or(code.len() - 1);
            out.push((i, Tok::Str(i + 1, close)));
            i = close + 1;
        } else {
            out.push((i, Tok::Punct(b)));
            i += 1;
        }
    }
    out
}

/// What one file contributes to the walk: byte ranges of `#[cfg(test)]`
/// items, its `mod NAME;` declarations with their inline module path and any
/// `#[path]`, and its `include!("FILE")`s.
#[derive(Default)]
struct Outline {
    tests: Vec<(usize, usize)>,
    mods: Vec<(Vec<String>, String, Option<String>)>,
    includes: Vec<String>,
}

fn outline(v: &Views) -> Outline {
    let toks = tokens(&v.code);
    let word = |t: Tok| match t {
        Tok::Ident(a, b) => std::str::from_utf8(&v.code[a..b]).unwrap(),
        _ => "",
    };
    let string = |t: Tok| match t {
        Tok::Str(a, b) => Some(String::from_utf8_lossy(&v.text[a..b]).into_owned()),
        _ => None,
    };
    let mut out = Outline::default();
    let mut inline: Vec<(String, usize)> = Vec::new();
    let mut depth = 0;
    let mut path_attr = None;
    let mut i = 0;
    while i < toks.len() {
        let (at, tok) = toks[i];
        match tok {
            Tok::Punct(b'#') => {
                let inner = toks.get(i + 1).is_some_and(|t| t.1 == Tok::Punct(b'!'));
                let open = i + 1 + inner as usize;
                if toks.get(open).map(|t| t.1) != Some(Tok::Punct(b'[')) {
                    i += 1;
                    continue;
                }
                let close = matching(&toks, open);
                let body: Vec<Tok> = toks[open + 1..close].iter().map(|t| t.1).collect();
                let words: Vec<&str> = body.iter().map(|&t| word(t)).collect();
                if is_cfg_test(&body, &words) {
                    let end = if inner { v.code.len() } else { item_end(&toks, close + 1, v.code.len()) };
                    out.tests.push((at, end));
                    i = toks.iter().position(|t| t.0 >= end).unwrap_or(toks.len());
                    continue;
                }
                if words.first() == Some(&"path") {
                    path_attr = body.iter().find_map(|&t| string(t));
                }
                i = close + 1;
                continue;
            }
            Tok::Ident(..) if word(tok) == "include" && toks.get(i + 1).map(|t| t.1) == Some(Tok::Punct(b'!')) => {
                out.includes.extend(toks.get(i + 3).and_then(|t| string(t.1)));
            }
            Tok::Ident(..) if word(tok) == "mod" => {
                if let (Some(&(_, name)), Some(&(_, next))) = (toks.get(i + 1), toks.get(i + 2)) {
                    let names = inline.iter().map(|m| m.0.clone()).collect();
                    match next {
                        Tok::Punct(b';') => out.mods.push((names, word(name).to_owned(), path_attr.take())),
                        Tok::Punct(b'{') => {
                            inline.push((word(name).to_owned(), depth));
                            depth += 1;
                            i += 3;
                            continue;
                        }
                        _ => (),
                    }
                }
            }
            Tok::Punct(b'{') => depth += 1,
            Tok::Punct(b'}') => {
                depth -= 1;
                if inline.last().is_some_and(|m| m.1 == depth) { inline.pop(); }
            }
            _ => (),
        }
        if !matches!(tok, Tok::Ident(..)) || !matches!(word(tok), "pub" | "crate" | "super" | "in" | "mod") {
            if !matches!(tok, Tok::Punct(b'(' | b')')) { path_attr = None; }
        }
        i += 1;
    }
    out
}

fn is_cfg_test(body: &[Tok], words: &[&str]) -> bool {
    if words.first() != Some(&"cfg") || body.get(1) != Some(&Tok::Punct(b'(')) { return false; }
    match words.get(2) {
        Some(&"test") => body.len() == 4,
        Some(&"all") => words.get(4) == Some(&"test"),
        _ => false,
    }
}

/// The index of the bracket closing the one at `open`.
fn matching(toks: &[(usize, Tok)], open: usize) -> usize {
    let mut depth = 0;
    for (j, t) in toks.iter().enumerate().skip(open) {
        match t.1 {
            Tok::Punct(b'(' | b'[' | b'{') => depth += 1,
            Tok::Punct(b')' | b']' | b'}') => {
                depth -= 1;
                if depth == 0 { return j; }
            }
            _ => (),
        }
    }
    toks.len() - 1
}

/// The byte just past the item that starts at token `from`: its closing
/// brace, or a `;` outside every bracket.
fn item_end(toks: &[(usize, Tok)], from: usize, len: usize) -> usize {
    let mut depth = 0;
    let mut j = from;
    while j < toks.len() {
        match toks[j].1 {
            Tok::Punct(b'#') if depth == 0 => {
                j = matching(toks, j + 1) + 1;
                continue;
            }
            Tok::Punct(b'(' | b'[' | b'{') => depth += 1,
            Tok::Punct(b')' | b']') => depth -= 1,
            Tok::Punct(b'}') => {
                depth -= 1;
                if depth == 0 { return toks[j].0 + 1; }
            }
            Tok::Punct(b';') if depth == 0 => return toks[j].0 + 1,
            _ => (),
        }
        j += 1;
    }
    len
}

#[derive(Clone, PartialEq, Eq, PartialOrd, Ord, Debug)]
struct Violation {
    path: String,
    line: usize,
    api: &'static str,
}

/// The `.rs` files under DIR, recursively.
fn rust_files(dir: &Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    let Ok(entries) = std::fs::read_dir(dir) else { return out };
    for path in entries.map(|e| e.unwrap().path()) {
        if path.is_dir() { out.extend(rust_files(&path)); }
        else if path.extension().is_some_and(|x| x == "rs") { out.push(path); }
    }
    out
}

/// Every crate root under CRATE, as cargo discovers them, plus the
/// romacs-vm primitive modules that build.rs declares: each `src/prims` file
/// but `mod.rs`, test-only by build.rs's `is_test_source`.
fn roots(krate: &Path) -> Vec<PathBuf> {
    let src = krate.join("src");
    let mut roots: Vec<PathBuf> = ["lib.rs", "main.rs"].iter().map(|f| src.join(f)).filter(|p| p.is_file()).collect();
    roots.extend(rust_files(&src.join("bin")));
    if krate.ends_with("crates/romacs-vm") {
        let test = |s: &str| s.contains("/tests/") || s.ends_with("/tests.rs") || s.ends_with("_test.rs") || s.ends_with("_tests.rs");
        roots.extend(rust_files(&src.join("prims")).into_iter().filter(|p| {
            let s = p.to_string_lossy();
            !s.ends_with("src/prims/mod.rs") && !test(&s)
        }));
    }
    roots.sort();
    roots
}

/// The violations in every non-test module reachable from ROOTS, with paths
/// relative to BASE.
fn scan(base: &Path, roots: Vec<PathBuf>) -> Vec<Violation> {
    let mut seen = BTreeSet::new();
    let mut work: Vec<(PathBuf, PathBuf)> = roots.into_iter().map(|r| { let d = r.parent().unwrap().to_owned(); (r, d) }).collect();
    let mut out = Vec::new();
    while let Some((file, dir)) = work.pop() {
        if !seen.insert(file.clone()) { continue; }
        let src = std::fs::read_to_string(&file).unwrap_or_else(|e| panic!("{}: {e}", file.display()));
        let mut v = views(&src);
        let outline = outline(&v);
        for &(from, to) in &outline.tests {
            for view in [&mut v.code, &mut v.text] {
                for b in &mut view[from..to] {
                    if *b != b'\n' { *b = b' '; }
                }
            }
        }
        let rel = file.strip_prefix(base).unwrap().to_string_lossy().into_owned();
        if rel != BOUNDARY { out.extend(violations(&rel, &v)); }
        let here = file.parent().unwrap();
        work.extend(outline.includes.iter().map(|f| (here.join(f), dir.clone())));
        for (inline, name, path) in outline.mods {
            let inner = inline.iter().fold(dir.clone(), |d, m| d.join(m));
            let (child, child_dir) = match path {
                Some(p) => {
                    let base = if inline.is_empty() { file.parent().unwrap().to_owned() } else { inner };
                    let child = base.join(p);
                    let d = child.parent().unwrap().to_owned();
                    (child, d)
                }
                None if inner.join(format!("{name}.rs")).is_file() => (inner.join(format!("{name}.rs")), inner.join(&name)),
                None => (inner.join(&name).join("mod.rs"), inner.join(&name)),
            };
            assert!(child.is_file(), "{}: mod {name} has no file {}", file.display(), child.display());
            work.push((child, child_dir));
        }
    }
    out.sort();
    out.dedup();
    out
}

fn violations(rel: &str, v: &Views) -> Vec<Violation> {
    let mut out = Vec::new();
    for api in APIS.iter().filter(|a| !a.owners.contains(&rel)) {
        let view = if api.strings { &v.text } else { &v.code };
        for (n, line) in view.split(|&b| b == b'\n').enumerate() {
            if matches_line(api, line) {
                out.push(Violation { path: rel.to_owned(), line: n + 1, api: api.name });
            }
        }
    }
    out
}

fn matches_line(api: &Api, line: &[u8]) -> bool {
    let needle = api.needle.as_bytes();
    (0..line.len()).filter(|&i| line[i..].starts_with(needle)).any(|i| {
        let before = &line[..i];
        let end = i + needle.len();
        let left = !ident(needle[0]) || (!before.last().is_some_and(|&b| ident(b)) && !before.ends_with(b"host::"));
        let right = api.prefix || !ident(needle[needle.len() - 1]) || !line.get(end).is_some_and(|&b| ident(b));
        let exempt = (!api.unless_after.is_empty() && before.ends_with(api.unless_after.as_bytes()))
            || (!api.unless_before.is_empty() && line[end..].starts_with(api.unless_before.as_bytes()))
            || (api.unless_pure && pure_items(&line[end..]));
        left && right && !exempt
    })
}

/// Whether REST starts with a pure `fs` item, or a `{…}` group of them.
fn pure_items(rest: &[u8]) -> bool {
    let group = rest.first() == Some(&b'{');
    let body = if group {
        let Some(close) = rest.iter().position(|&b| b == b'}') else { return false };
        &rest[1..close]
    } else {
        &rest[..rest.iter().position(|&b| !ident(b)).unwrap_or(rest.len())]
    };
    let names: Vec<&str> = std::str::from_utf8(body).unwrap_or("").split(|c: char| !(c.is_ascii_alphanumeric() || c == '_'))
        .filter(|n| !n.is_empty()).collect();
    !names.is_empty() && names.iter().all(|n| PURE_FS.contains(n) || n.ends_with("Ext"))
}

struct Entry {
    path: String,
    line: Option<usize>,
    api: Option<String>,
    reason: String,
}

impl Entry {
    fn covers(&self, v: &Violation) -> bool {
        self.path == v.path && self.line.is_none_or(|l| l == v.line) && self.api.as_deref().is_none_or(|a| a == v.api)
    }
    fn exact(&self) -> bool {
        self.line.is_some() && self.api.is_some()
    }
}

fn parse_allow(text: &str) -> Vec<Entry> {
    text.lines().filter(|l| !l.trim().is_empty() && !l.starts_with('#')).map(|l| {
        let mut words = l.split_whitespace();
        let (site, api) = (words.next().unwrap(), words.next().unwrap_or_else(|| panic!("no API: {l}")));
        let reason = words.collect::<Vec<_>>().join(" ");
        let (path, line) = site.rsplit_once(':').unwrap_or_else(|| panic!("no line: {l}"));
        assert!(!reason.is_empty() && reason != "pending", "no reason: {l}");
        assert!(api == "*" || APIS.iter().any(|a| a.name == api), "unknown API: {l}");
        Entry {
            path: path.to_owned(),
            line: (line != "*").then(|| line.parse().unwrap_or_else(|_| panic!("bad line: {l}"))),
            api: (api != "*").then(|| api.to_owned()),
            reason,
        }
    }).collect()
}

/// Each violation goes to its exact entry, else the first wildcard that
/// covers it. Returns the uncovered violations and the unused entries.
fn check<'a>(entries: &'a [Entry], found: &'a [Violation]) -> (Vec<&'a Violation>, Vec<&'a Entry>) {
    let mut used = vec![false; entries.len()];
    let mut uncovered = Vec::new();
    for v in found {
        let owner = entries.iter().position(|e| e.exact() && e.covers(v))
            .or_else(|| entries.iter().position(|e| !e.exact() && e.covers(v)));
        match owner {
            Some(i) => used[i] = true,
            None => uncovered.push(v),
        }
    }
    let unused = entries.iter().zip(used).filter(|(_, u)| !u).map(|(e, _)| e).collect();
    (uncovered, unused)
}

fn render(e: &Entry) -> String {
    let line = e.line.map_or("*".into(), |l| l.to_string());
    format!("{}:{line} {} {}", e.path, e.api.as_deref().unwrap_or("*"), e.reason)
}

/// The allowlist for FOUND: wildcard entries that still match, then one exact
/// entry per remaining violation, its reason kept from the entry with the
/// same site, else from an unused entry with the same path and API, else
/// `pending`.
fn bless(header: &str, entries: Vec<Entry>, found: &[Violation]) -> String {
    let (wild, mut exact): (Vec<Entry>, Vec<Entry>) = entries.into_iter().partition(|e| !e.exact());
    let wild: Vec<Entry> = wild.into_iter().filter(|e| found.iter().any(|v| e.covers(v))).collect();
    let rest: Vec<&Violation> = found.iter().filter(|v| !wild.iter().any(|e| e.covers(v))).collect();
    let mut kept: Vec<Option<String>> = rest.iter().map(|v| {
        exact.iter().position(|e| e.covers(v)).map(|i| exact.remove(i).reason)
    }).collect();
    for (v, reason) in rest.iter().zip(&mut kept) {
        if reason.is_none() {
            *reason = exact.iter().position(|e| e.path == v.path && e.api.as_deref() == Some(v.api)).map(|i| exact.remove(i).reason);
        }
    }
    let mut out = header.to_owned();
    out.extend(wild.iter().map(|e| render(e) + "\n"));
    for (v, reason) in rest.iter().zip(kept) {
        let e = Entry { path: v.path.clone(), line: Some(v.line), api: Some(v.api.to_owned()), reason: reason.unwrap_or("pending".into()) };
        out.push_str(&(render(&e) + "\n"));
    }
    out
}

fn repository() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..").canonicalize().unwrap()
}

#[test]
fn host_boundary_holds() {
    let base = repository();
    let found = scan(&base, CRATES.iter().flat_map(|c| roots(&base.join(c))).collect());
    let allow = base.join(ALLOW);
    let mut text = std::fs::read_to_string(&allow).unwrap();
    if std::env::var_os("ROMACS_HOST_LINT_BLESS").is_some() {
        let header: String = text.lines().take_while(|l| l.starts_with('#')).map(|l| format!("{l}\n")).collect();
        text = bless(&header, parse_allow(&text), &found);
        std::fs::write(&allow, &text).unwrap();
    }
    let entries = parse_allow(&text);
    let (uncovered, unused) = check(&entries, &found);
    let report: Vec<String> = uncovered.iter().map(|v| format!("  unlisted {}:{} {}", v.path, v.line, v.api))
        .chain(unused.iter().map(|e| format!("  stale {}", render(e)))).collect();
    assert!(report.is_empty(), "host boundary ({ALLOW}; ROMACS_HOST_LINT_BLESS=1 regenerates):\n{}", report.join("\n"));
}

/// The fixture crate marks each expected violation with `// expect: API`
/// (several separated by spaces). Its test modules and near misses carry none.
#[test]
fn lint_flags_each_planted_violation() {
    let base = repository().join(FIXTURES);
    let found: BTreeSet<(String, usize, String)> = scan(&base, vec![base.join("src/lib.rs")]).into_iter()
        .map(|v| (v.path, v.line, v.api.to_owned())).collect();
    let mut expected = BTreeSet::new();
    let mut walk = vec![base.join("src")];
    while let Some(dir) = walk.pop() {
        for entry in std::fs::read_dir(&dir).unwrap() {
            let path = entry.unwrap().path();
            if path.is_dir() { walk.push(path); continue; }
            let rel = path.strip_prefix(&base).unwrap().to_string_lossy().into_owned();
            for (n, line) in std::fs::read_to_string(&path).unwrap().lines().enumerate() {
                if let Some((_, apis)) = line.split_once("// expect: ") {
                    expected.extend(apis.split_whitespace().map(|a| (rel.clone(), n + 1, a.to_owned())));
                }
            }
        }
    }
    let planted: BTreeSet<&str> = expected.iter().map(|e| e.2.as_str()).collect();
    let missing: Vec<&str> = APIS.iter().map(|a| a.name).filter(|n| !planted.contains(n)).collect();
    assert!(missing.is_empty(), "no planted violation for {missing:?}");
    assert_eq!(found, expected);
}
