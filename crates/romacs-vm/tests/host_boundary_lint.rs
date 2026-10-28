//! Host boundary lint, rules 1 and 2 of the world-snapshot design (§6.1).
//!
//! Rule 1: outside `crates/romacs-vm/src/host.rs`, no source in the runner's
//! dependency closure calls a host query or effect API directly. Rule 2: only
//! the environment owners name the environment slots, and only the per-buffer
//! accessor looks up `default-directory`'s storage.
//!
//! The scan is over the tokens of the compiled module tree of each crate:
//! comments are removed, string contents are hidden from rule 1, whitespace
//! does not separate a spelling, and `#[cfg(test)]` items are skipped, since
//! test code builds no worlds. A forbidden item has one name outside host.rs:
//! its path. So every `use` tree is flattened and each imported path is
//! checked as a spelling, no `use` from std, core, alloc or libc may rename
//! or glob (`use-alias`, `use-glob`), and no type alias may name a type whose
//! associated functions are forbidden (`type-alias`). Type-qualified calls
//! are matched in both `Type::f` and `<Type>::f` forms.
//!
//! Every match needs an entry in `host_boundary_allow.txt`, one exact
//! `PATH:LINE API REASON` per line. An entry that matches nothing fails, so
//! the list cannot rot. `ROMACS_HOST_LINT_BLESS=1` deliberately rewrites the
//! list for the current tree: a reason follows its site, or an unused entry
//! with the same path and API, and new matches are marked `pending`, which
//! fails until it is migrated or given a reason.
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

/// One forbidden API and its spellings. A needle that ends in `*` is a
/// prefix. A needle that starts with an identifier character must not
/// continue an identifier on its left, nor, if it is a plain name, follow
/// `host::`; one that ends
/// with an identifier character must not continue on its right. A needle
/// `Type::f` also matches `Type>::f`. `strings` matches inside string
/// literals (rule 2).
struct Api {
    name: &'static str,
    needles: &'static [&'static str],
    unless_after: &'static str,
    unless_before: &'static str,
    unless_pure: bool,
    strings: bool,
    owners: &'static [&'static str],
}

const fn apis(name: &'static str, needles: &'static [&'static str]) -> Api {
    Api { name, needles, unless_after: "", unless_before: "", unless_pure: false, strings: false, owners: &[] }
}
macro_rules! api {
    ($name:literal) => { apis($name, &[$name]) };
    ($name:literal, $($needle:literal),+) => { apis($name, &[$($needle),+]) };
}
/// A Path method: called on a value, or named through `Path`/`PathBuf`.
/// Metadata and FileType answer `is_file`/`is_dir`/`is_symlink` from data
/// already read; spelled through `file_type()`, they are not a path probe.
macro_rules! path_method {
    ($name:literal, $method:literal) => {
        Api {
            unless_after: "file_type()",
            ..apis($name, &[concat!(".", $method, "("), concat!("Path::", $method), concat!("PathBuf::", $method)])
        }
    };
}
/// A `fs::` path naming only data types and extension traits queries nothing;
/// `File::open` and `File::create` are their own APIs.
const fn fs_path(needle: &'static [&'static str]) -> Api {
    Api { unless_pure: true, ..apis("std::fs", needle) }
}
const PURE_FS: &[&str] = &["File", "Metadata", "Permissions", "FileType", "FileTimes", "DirEntry", "ReadDir"];
const fn slot(name: &'static str, needle: &'static [&'static str], owners: &'static [&'static str]) -> Api {
    Api { strings: true, owners, ..apis(name, needle) }
}

const APIS: &[Api] = &[
    fs_path(&["fs::"]),
    Api { unless_before: "::", ..api!("std::fs") },
    api!("File::open"),
    api!("File::create"),
    api!("OpenOptions"),
    api!("read_dir"),
    api!("metadata", "metadata("),
    api!("symlink_metadata"),
    api!("canonicalize", ".canonicalize(", "Path::canonicalize", "PathBuf::canonicalize"),
    path_method!("Path::exists", "exists"),
    path_method!("Path::try_exists", "try_exists"),
    path_method!("Path::is_file", "is_file"),
    path_method!("Path::is_dir", "is_dir"),
    path_method!("Path::is_symlink", "is_symlink"),
    path_method!("Path::read_link", "read_link"),
    api!("std::path::absolute", "path::absolute"),
    api!("std::env::var", "env::var"),
    api!("std::env::var_os", "env::var_os"),
    api!("std::env::vars", "env::vars"),
    api!("std::env::vars_os", "env::vars_os"),
    api!("std::env::current_dir", "env::current_dir"),
    api!("std::env::current_exe", "env::current_exe"),
    api!("std::env::temp_dir", "env::temp_dir"),
    api!("std::env::set", "env::set_var", "env::remove_var", "env::set_current_dir"),
    api!("std::process::id", "process::id"),
    api!("Command::new"),
    api!("std::net", "TcpStream::connect", "TcpListener::bind", "UdpSocket::bind", "UnixStream::connect",
         "UnixStream::pair", "UnixListener::bind", "UnixDatagram::bind", "UnixDatagram::unbound",
         "UnixDatagram::pair", "to_socket_addrs", "ToSocketAddrs"),
    api!("SystemTime::now"),
    api!("Instant::now"),
    api!("available_parallelism"),
    api!("libc::getpw*"),
    api!("libc::getgr*"),
    api!("libc::getpid"),
    api!("libc::getppid"),
    api!("libc::getuid"),
    api!("libc::geteuid"),
    api!("libc::getgid"),
    api!("libc::getegid"),
    api!("libc::gethostname"),
    api!("libc::uname"),
    api!("libc::sysconf"),
    api!("libc::sched_getaffinity"),
    api!("libc::statvfs"),
    api!("libc::clock", "libc::clock_gettime", "libc::time", "libc::gettimeofday"),
    api!("libc::getrandom"),
    api!("libc::getenv", "libc::getenv", "libc::secure_getenv"),
    api!("libc::file", "libc::open*", "libc::creat*", "libc::access", "libc::eaccess", "libc::euidaccess",
         "libc::faccessat*", "libc::chmod", "libc::fchmod*", "libc::chown", "libc::fchown*", "libc::lchown",
         "libc::rename*", "libc::unlink*", "libc::mkdir*", "libc::rmdir", "libc::link*", "libc::symlink*",
         "libc::readlink*", "libc::realpath", "libc::stat", "libc::stat64", "libc::statx", "libc::statfs*",
         "libc::fstat*", "libc::lstat*", "libc::truncate*", "libc::ftruncate*", "libc::utime*", "libc::futime*",
         "libc::copy_file_range", "libc::sendfile*", "libc::mkstemp*", "libc::mkostemp*", "libc::mkdtemp",
         "libc::mkfifo*", "libc::mknod*", "libc::memfd_create", "libc::fdopendir", "libc::readdir*",
         "libc::getcwd", "libc::chdir", "libc::fchdir", "libc::getxattr", "libc::fgetxattr", "libc::lgetxattr",
         "libc::setxattr", "libc::fsetxattr", "libc::lsetxattr", "libc::listxattr", "libc::flistxattr",
         "libc::llistxattr", "libc::removexattr", "libc::fremovexattr", "libc::lremovexattr"),
    api!("libc::process", "libc::kill", "libc::killpg", "libc::tgkill", "libc::fork", "libc::vfork",
         "libc::exec*", "libc::fexecve", "libc::posix_spawn*", "libc::system", "libc::popen", "libc::clone",
         "libc::syscall"),
    api!("libc::descriptor", "libc::pipe*", "libc::eventfd", "libc::posix_openpt", "libc::socketpair"),
    api!("libc::socket", "libc::socket", "libc::connect", "libc::bind", "libc::listen", "libc::accept*",
         "libc::getaddrinfo", "libc::gethostbyname*"),
    slot("slot:process-environment", &["\"process-environment\""], ENVIRONMENT_OWNERS),
    slot("slot:initial-environment", &["\"initial-environment\""], ENVIRONMENT_OWNERS),
    slot("slot:exec-path", &["\"exec-path\""], ENVIRONMENT_OWNERS),
    slot("slot:default-directory", &["lookup(\"default-directory\")"], BUFFER_ACCESSOR),
];

/// The import forms that would give a forbidden item a second name.
const USE_ALIAS: &str = "use-alias";
const USE_GLOB: &str = "use-glob";
const TYPE_ALIAS: &str = "type-alias";
const FORMS: &[&str] = &[USE_ALIAS, USE_GLOB, TYPE_ALIAS];
/// The crates whose items no import may rename or glob.
const ROOTS: &[&str] = &["std", "core", "alloc", "libc"];
/// Types with forbidden associated functions, which no type alias may name.
const TYPES: &[&str] = &[
    "Path", "PathBuf", "File", "OpenOptions", "DirBuilder", "Instant", "SystemTime", "Command",
    "TcpStream", "TcpListener", "UdpSocket", "UnixStream", "UnixListener", "UnixDatagram",
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

/// One view as its tokens joined without whitespace (a space only between
/// two words), with each byte's source line.
struct Compact {
    text: Vec<u8>,
    lines: Vec<usize>,
}

fn compact(view: &[u8], toks: &[(usize, Tok)]) -> Compact {
    let starts: Vec<usize> = view.iter().enumerate().filter(|&(_, &b)| b == b'\n').map(|(i, _)| i).collect();
    let mut out = Compact { text: Vec::new(), lines: Vec::new() };
    let mut word = false;
    for &(at, tok) in toks {
        let line = starts.partition_point(|&n| n < at) + 1;
        let bytes: Vec<u8> = match tok {
            Tok::Ident(a, b) => view[a..b].to_vec(),
            Tok::Punct(b) => vec![b],
            Tok::Str(a, b) => [&b"\""[..], &view[a..b], b"\""].concat(),
        };
        let is_word = !matches!(tok, Tok::Punct(_));
        if word && is_word {
            out.text.push(b' ');
            out.lines.push(line);
        }
        word = is_word;
        out.lines.extend(std::iter::repeat_n(line, bytes.len()));
        out.text.extend(bytes);
    }
    out
}

/// The offsets in TEXT at which API is spelled.
fn spellings(api: &Api, text: &[u8]) -> Vec<usize> {
    let mut out = Vec::new();
    for needle in api.needles {
        let (stem, prefix) = needle.strip_suffix('*').map_or((*needle, false), |s| (s, true));
        let qualified = stem.starts_with(|c: char| c.is_ascii_uppercase()).then(|| stem.replacen("::", ">::", 1));
        for form in std::iter::once(stem.to_owned()).chain(qualified) {
            let form = form.as_bytes();
            out.extend((0..text.len()).filter(|&i| text[i..].starts_with(form) && spelled(api, form, prefix, text, i)));
        }
    }
    out
}

fn spelled(api: &Api, needle: &[u8], prefix: bool, text: &[u8], i: usize) -> bool {
    let before = &text[..i];
    let end = i + needle.len();
    // host.rs's own items (`host::read_dir`, `host::OpenOptions`) share a
    // plain name with an API; no path reaches an API through it.
    let host = before.strip_suffix(b"host::").is_some_and(|b| !b.last().is_some_and(|&c| ident(c)));
    let through_host = host && !needle.windows(2).any(|w| w == b"::");
    let left = !ident(needle[0]) || (!before.last().is_some_and(|&b| ident(b)) && !through_host);
    let right = prefix || !ident(needle[needle.len() - 1]) || !text.get(end).is_some_and(|&b| ident(b));
    let exempt = (!api.unless_after.is_empty() && before.ends_with(api.unless_after.as_bytes()))
        || (!api.unless_before.is_empty() && text[end..].starts_with(api.unless_before.as_bytes()))
        || (api.unless_pure && pure_items(&text[end..]));
    left && right && !exempt
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

/// One leaf of a `use` tree: its full path, and how it is bound.
#[derive(Debug, PartialEq)]
struct Import {
    path: Vec<String>,
    bind: Bind,
    line: usize,
}

#[derive(Debug, PartialEq)]
enum Bind {
    Name,
    Alias,
    Glob,
}

/// The leaves of every `use` declaration and `extern crate … as` in TOKS.
fn imports(code: &[u8], toks: &[(usize, Tok)]) -> Vec<Import> {
    let words: Vec<&str> = toks.iter().map(|t| match t.1 {
        Tok::Ident(a, b) => std::str::from_utf8(&code[a..b]).unwrap(),
        _ => "",
    }).collect();
    let line = |i: usize| code[..toks[i].0].iter().filter(|&&b| b == b'\n').count() + 1;
    let mut out = Vec::new();
    for i in 0..toks.len() {
        if words[i] == "use" {
            use_tree(&words, toks, i + 1, &mut Vec::new(), &mut out, &line);
        } else if words[i] == "extern" && words.get(i + 1) == Some(&"crate") && words.get(i + 3) == Some(&"as") {
            out.push(Import { path: vec![words[i + 2].to_owned()], bind: Bind::Alias, line: line(i + 2) });
        }
    }
    out
}

/// Parse the tree at token I under PREFIX into OUT; the index after it.
fn use_tree(words: &[&str], toks: &[(usize, Tok)], mut i: usize, prefix: &mut Vec<String>, out: &mut Vec<Import>,
            line: &dyn Fn(usize) -> usize) -> usize {
    let depth = prefix.len();
    let punct = |i: usize, c: u8| toks.get(i).is_some_and(|t| t.1 == Tok::Punct(c));
    let separator = |i: usize| punct(i, b':') && punct(i + 1, b':');
    if separator(i) { i += 2; }
    loop {
        if punct(i, b'*') {
            out.push(Import { path: prefix.clone(), bind: Bind::Glob, line: line(i) });
            i += 1;
            break;
        }
        if punct(i, b'{') {
            i += 1;
            while !punct(i, b'}') && i < toks.len() {
                let next = use_tree(words, toks, i, prefix, out, line);
                i = if next == i { i + 1 } else { next };
                if punct(i, b',') { i += 1; }
            }
            i += 1;
            break;
        }
        let name = words.get(i).copied().unwrap_or("");
        if name.is_empty() { break; }
        if separator(i + 1) {
            prefix.push(name.to_owned());
            i += 3;
            continue;
        }
        let at = line(i);
        let mut path = prefix.clone();
        if name != "self" { path.push(name.to_owned()); }
        i += 1;
        let bind = if words.get(i) == Some(&"as") {
            i += 2;
            if words[i - 1] == "_" { Bind::Name } else { Bind::Alias }
        } else { Bind::Name };
        out.push(Import { path, bind, line: at });
        break;
    }
    prefix.truncate(depth);
    i
}

/// The lines of `type NAME = …;` aliases that name one of TYPES.
fn type_aliases(code: &[u8], toks: &[(usize, Tok)]) -> Vec<usize> {
    let word = |t: Tok| match t {
        Tok::Ident(a, b) => std::str::from_utf8(&code[a..b]).unwrap(),
        _ => "",
    };
    let line = |at: usize| code[..at].iter().filter(|&&b| b == b'\n').count() + 1;
    (0..toks.len()).filter(|&i| word(toks[i].1) == "type" && toks.get(i + 2).is_some_and(|t| t.1 == Tok::Punct(b'=')))
        .filter(|&i| toks[i..].iter().map(|t| t.1).take_while(|&t| t != Tok::Punct(b';')).any(|t| TYPES.contains(&word(t))))
        .map(|i| line(toks[i].0)).collect()
}

fn violations(rel: &str, v: &Views) -> Vec<Violation> {
    let toks = tokens(&v.code);
    let (code, text) = (compact(&v.code, &toks), compact(&v.text, &toks));
    let mut out = Vec::new();
    for api in APIS.iter().filter(|a| !a.owners.contains(&rel)) {
        let view = if api.strings { &text } else { &code };
        out.extend(spellings(api, &view.text).into_iter().map(|i| Violation { path: rel.to_owned(), line: view.lines[i], api: api.name }));
    }
    for import in imports(&v.code, &toks) {
        let root = import.path.first().map(String::as_str).unwrap_or("");
        let reaches = ROOTS.contains(&root) || import.path.iter().any(|s| s == "libc");
        let form = match import.bind {
            Bind::Alias if reaches => Some(USE_ALIAS),
            Bind::Glob if reaches => Some(USE_GLOB),
            _ => None,
        };
        out.extend(form.map(|api| Violation { path: rel.to_owned(), line: import.line, api }));
        let spelling = import.path.join("::").into_bytes();
        for api in APIS.iter().filter(|a| !a.strings) {
            if !spellings(api, &spelling).is_empty() {
                out.push(Violation { path: rel.to_owned(), line: import.line, api: api.name });
            }
        }
    }
    out.extend(type_aliases(&v.code, &toks).into_iter().map(|line| Violation { path: rel.to_owned(), line, api: TYPE_ALIAS }));
    out
}

fn known(api: &str) -> bool {
    APIS.iter().any(|a| a.name == api) || FORMS.contains(&api)
}

struct Entry {
    path: String,
    line: usize,
    api: String,
    reason: String,
}

impl Entry {
    fn covers(&self, v: &Violation) -> bool {
        self.path == v.path && self.line == v.line && self.api == v.api
    }
}

/// The entries of TEXT, each with a reason.
fn parse_allow(text: &str) -> Vec<Entry> {
    let entries = parse_entries(text);
    for e in &entries {
        assert!(!e.reason.is_empty() && e.reason != "pending", "no reason: {}", render(e));
    }
    entries
}

fn parse_entries(text: &str) -> Vec<Entry> {
    text.lines().filter(|l| !l.trim().is_empty() && !l.starts_with('#')).map(|l| {
        let mut words = l.split_whitespace();
        let (site, api) = (words.next().unwrap(), words.next().unwrap_or_else(|| panic!("no API: {l}")));
        let reason = words.collect::<Vec<_>>().join(" ");
        let (path, line) = site.rsplit_once(':').unwrap_or_else(|| panic!("no line: {l}"));
        assert!(known(api), "unknown API: {l}");
        Entry {
            path: path.to_owned(),
            line: line.parse().unwrap_or_else(|_| panic!("bad line: {l}")),
            api: api.to_owned(),
            reason,
        }
    }).collect()
}

/// The violations no entry covers, and the entries that cover none.
fn check<'a>(entries: &'a [Entry], found: &'a [Violation]) -> (Vec<&'a Violation>, Vec<&'a Entry>) {
    let uncovered = found.iter().filter(|v| !entries.iter().any(|e| e.covers(v))).collect();
    let unused = entries.iter().filter(|e| !found.iter().any(|v| e.covers(v))).collect();
    (uncovered, unused)
}

fn render(e: &Entry) -> String {
    format!("{}:{} {} {}", e.path, e.line, e.api, e.reason)
}

/// The allowlist for FOUND: one entry per violation, its reason kept from
/// the entry at the same site, else from an unused entry with the same path
/// and API, else `pending`.
fn bless(header: &str, mut entries: Vec<Entry>, found: &[Violation]) -> String {
    let mut kept: Vec<Option<String>> = found.iter().map(|v| {
        entries.iter().position(|e| e.covers(v)).map(|i| entries.remove(i).reason)
    }).collect();
    for (v, reason) in found.iter().zip(&mut kept) {
        if reason.is_none() {
            *reason = entries.iter().position(|e| e.path == v.path && e.api == v.api).map(|i| entries.remove(i).reason);
        }
    }
    let mut out = header.to_owned();
    for (v, reason) in found.iter().zip(kept) {
        let e = Entry { path: v.path.clone(), line: v.line, api: v.api.to_owned(), reason: reason.unwrap_or("pending".into()) };
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
        text = bless(&header, parse_entries(&text), &found);
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
    let missing: Vec<&str> = APIS.iter().map(|a| a.name).chain(FORMS.iter().copied()).filter(|n| !planted.contains(n)).collect();
    assert!(missing.is_empty(), "no planted violation for {missing:?}");
    assert_eq!(found, expected);
}
