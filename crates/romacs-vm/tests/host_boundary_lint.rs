//! Host boundary lint, rules 1 to 4 of the world-snapshot design (§6.1).
//!
//! Rule 1: outside `crates/romacs-vm/src/host.rs`, no source in the runner's
//! dependency closure calls a host query or effect API directly. Rule 2: only
//! the environment owners name the environment slots, and only the per-buffer
//! accessor looks up `default-directory`'s storage. Rule 3: each site that
//! names a variable on the B3 trace's delayed list says why, so no Rust read
//! of one bypasses the fence. Rule 4: each site that can reach a saved value
//! of the VM's binding stack (`vm.dynamic`) says why, so only specbind,
//! unbind and the saved-binding accessors, which check the fence (F2), read
//! or write one.
//!
//! The scan is over the tokens of the compiled module tree of each crate:
//! comments are removed, string contents are hidden from rule 1, whitespace
//! does not separate a spelling, and `#[cfg(test)]` items are skipped, since
//! test code builds no worlds. A forbidden item has one name outside host.rs:
//! its path. So every `use` tree is flattened and each imported path is
//! checked as a spelling; no `use` may import a host-bearing std module, a
//! non-pure item under one, or a module above one (`use-host`), so no host
//! module has a local name a later `use` could rename; no `use` from std,
//! core, alloc or libc may rename or glob, and no `use` from anywhere may
//! rename a type whose associated functions are forbidden (`use-alias`,
//! `use-glob`); and no type alias, generic or not, may name such a type
//! (`type-alias`).
//! Type-qualified calls are matched in both `Type::f` and `<Type>::f` forms.
//!
//! Every site needs a comment in the run of comment lines directly above
//! its line, `// HOST(API): REASON`, naming the API (several separated by
//! commas) and why the site may call it. The comment moves with its site
//! through edits and merges, and whoever edits the site reads its reason
//! there. A comment that names an API its line does not call, names an
//! unknown API, or gives no reason fails, so the comments cannot rot. A
//! trailing comment, a doc comment, and a comment in a `#[cfg(test)]` item
//! cover nothing.
use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

const CRATES: &[&str] = &[
    "crates/romacs-vm",
    "crates/romacs-face",
    "crates/romacs-layout-input",
    "crates/romacs-display-protocol",
    "crates/romacs-regex",
    "crates/romacs-gnu-tests",
];
const BOUNDARY: &str = "crates/romacs-vm/src/host.rs";
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
    slot("delayed-variable", DELAYED, &[]),
    api!("dynamic:saved", ".dynamic[", ".dynamic.iter*", ".dynamic.first", ".dynamic.last", ".dynamic.get*", ".dynamic.pop",
         ".dynamic.push", "self.dynamic{", "vm.dynamic{"),
];

/// The delayed list of the B3 world trace (`--world-trace`): startup
/// computes these, and the fence keeps every later access off them.
const DELAYED: &[&str] = &[
    "\"abbrev-file-name\"", "\"auto-save-file-name-transforms\"", "\"auto-save-list-file-prefix\"",
    "\"blink-cursor-mode\"", "\"compile-command\"", "\"custom-theme-directory\"", "\"default-korean-keyboard\"",
    "\"global-font-lock-mode\"", "\"insert-directory-program\"", "\"mode-line-collapse-minor-modes-to\"",
    "\"package-directory-list\"", "\"package-quickstart-file\"", "\"package-user-dir\"", "\"rmail-spool-directory\"",
    "\"small-temporary-file-directory\"", "\"temporary-file-directory\"", "\"transient-mark-mode\"",
    "\"tutorial-directory\"", "\"user-lisp-directory\"", "\"user-lisp-ignored-directories\"", "\"user-mail-address\"",
];

/// The import forms that would give a forbidden item a second name.
const USE_HOST: &str = "use-host";
const USE_ALIAS: &str = "use-alias";
const USE_GLOB: &str = "use-glob";
const TYPE_ALIAS: &str = "type-alias";
const FORMS: &[&str] = &[USE_HOST, USE_ALIAS, USE_GLOB, TYPE_ALIAS];
/// The std paths that bear host APIs, which outside host.rs are only ever
/// spelled in full.
const HOST_PATHS: &[&[&str]] = &[
    &["std", "env"], &["std", "fs"], &["std", "process"], &["std", "net"],
    &["std", "time", "Instant"], &["std", "time", "SystemTime"], &["std", "path", "absolute"],
    &["std", "os", "unix", "fs"], &["std", "os", "unix", "net"],
];
/// The crates whose items no import may rename or glob.
const ROOTS: &[&str] = &["std", "core", "alloc", "libc"];
/// Types with forbidden associated functions, which no type alias may name
/// and no `use` may rename.
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
                    let end = if inner { v.code.len() } else { item_end(&toks, close + 1, v.code.len(), !starts_item(&toks, close + 1, &word)) };
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

/// Whether the tokens at `from` start an item or a `let`, which end as an
/// item does, not a field, a variant, an argument, a match arm or an
/// expression.
fn starts_item<'a>(toks: &[(usize, Tok)], mut from: usize, word: &dyn Fn(Tok) -> &'a str) -> bool {
    let punct = |j: usize, b: u8| toks.get(j).is_some_and(|t| t.1 == Tok::Punct(b));
    while punct(from, b'#') { from = matching(toks, from + 1) + 1; }
    if toks.get(from).is_some_and(|t| word(t.1) == "pub") {
        from += 1;
        if punct(from, b'(') { from = matching(toks, from) + 1; }
    }
    toks.get(from).is_some_and(|t| matches!(word(t.1), "fn" | "mod" | "use" | "impl" | "struct" | "enum" | "union" | "const"
        | "static" | "type" | "trait" | "unsafe" | "async" | "extern" | "default" | "macro_rules" | "let"))
}

/// The byte just past what starts at token `from`: its closing brace, or a
/// `;` outside every bracket. It ends before a bracket that closes one opened
/// ahead of it, and, as a MEMBER of a list (a field, a variant, an argument,
/// a match arm), after a `,` outside every bracket.
fn item_end(toks: &[(usize, Tok)], from: usize, len: usize, member: bool) -> usize {
    let mut depth = 0;
    let mut j = from;
    while j < toks.len() {
        match toks[j].1 {
            Tok::Punct(b'#') if depth == 0 => {
                j = matching(toks, j + 1) + 1;
                continue;
            }
            Tok::Punct(b'(' | b'[' | b'{') => depth += 1,
            Tok::Punct(b')' | b']' | b'}') if depth == 0 => return toks[j].0,
            Tok::Punct(b')' | b']') => depth -= 1,
            Tok::Punct(b'}') => {
                depth -= 1;
                if depth == 0 { return toks[j].0 + 1; }
            }
            Tok::Punct(b';') if depth == 0 => return toks[j].0 + 1,
            Tok::Punct(b',') if member && depth == 0 => return toks[j].0 + 1,
            _ => (),
        }
        j += 1;
    }
    len
}

/// One site: a line of PATH that spells API.
#[derive(Clone, PartialEq, Eq, PartialOrd, Ord, Debug)]
struct Violation {
    path: String,
    line: usize,
    api: &'static str,
}

/// One `// HOST(API, ...): REASON` comment of PATH: its line, the line it
/// covers (the first below its run of comment lines, when that holds code),
/// the APIs it names and its reason.
#[derive(Debug)]
struct Tag {
    path: String,
    line: usize,
    covers: Option<usize>,
    apis: Vec<String>,
    reason: String,
}

/// The HOST comments of SRC, whose comments TEXT blanks.
fn tags(rel: &str, src: &str, text: &[u8]) -> Vec<Tag> {
    let raw: Vec<&str> = src.split('\n').collect();
    let seen: Vec<&[u8]> = text.split(|&b| b == b'\n').collect();
    let blank = |i: usize| seen[i].iter().all(u8::is_ascii_whitespace);
    let comment = |i: usize| raw[i].trim_start().starts_with("//") && blank(i);
    (0..raw.len()).filter(|&i| comment(i)).filter_map(|i| {
        let rest = raw[i].trim_start().strip_prefix("// HOST(")?;
        let (apis, reason) = rest.split_once(')').unwrap_or((rest, ""));
        let below = (i + 1..raw.len()).find(|&j| !comment(j)).unwrap_or(raw.len());
        Some(Tag {
            path: rel.to_owned(),
            line: i + 1,
            covers: (below < raw.len() && !blank(below)).then_some(below + 1),
            apis: apis.split(',').map(|a| a.trim().to_owned()).collect(),
            reason: reason.strip_prefix(':').unwrap_or("").trim().to_owned(),
        })
    }).collect()
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

/// The sites and HOST comments of every non-test module reachable from
/// ROOTS, with paths relative to BASE.
fn scan(base: &Path, roots: Vec<PathBuf>) -> (Vec<Violation>, Vec<Tag>) {
    let mut seen = BTreeSet::new();
    let mut work: Vec<(PathBuf, PathBuf)> = roots.into_iter().map(|r| { let d = r.parent().unwrap().to_owned(); (r, d) }).collect();
    let (mut sites, mut comments) = (Vec::new(), Vec::new());
    while let Some((file, dir)) = work.pop() {
        if !seen.insert(file.clone()) { continue; }
        let src = std::fs::read_to_string(&file).unwrap_or_else(|e| panic!("{}: {e}", file.display()));
        let rel = file.strip_prefix(base).unwrap().to_string_lossy().into_owned();
        let (found, said, outline) = file_scan(&rel, &src);
        sites.extend(found);
        comments.extend(said);
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
    sites.sort();
    (sites, comments)
}

/// The sites and HOST comments of the source SRC of REL, and its outline.
/// `#[cfg(test)]` items hold no site; BOUNDARY holds none either.
fn file_scan(rel: &str, src: &str) -> (Vec<Violation>, Vec<Tag>, Outline) {
    let mut v = views(src);
    let said = tags(rel, src, &v.text);
    let outline = outline(&v);
    for &(from, to) in &outline.tests {
        for view in [&mut v.code, &mut v.text] {
            for b in &mut view[from..to] {
                if *b != b'\n' { *b = b' '; }
            }
        }
    }
    let found = if rel == BOUNDARY { Vec::new() } else { violations(rel, &v) };
    (found, said, outline)
}

/// One view as its tokens joined without whitespace (a space only between
/// two words), with the source offset of each byte's token.
struct Compact {
    text: Vec<u8>,
    at: Vec<usize>,
}

fn compact(view: &[u8], toks: &[(usize, Tok)]) -> Compact {
    let mut out = Compact { text: Vec::new(), at: Vec::new() };
    let mut word = false;
    for &(at, tok) in toks {
        let bytes: Vec<u8> = match tok {
            Tok::Ident(a, b) => view[a..b].to_vec(),
            Tok::Punct(b) => vec![b],
            Tok::Str(a, b) => [&b"\""[..], &view[a..b], b"\""].concat(),
        };
        let is_word = !matches!(tok, Tok::Punct(_));
        if word && is_word {
            out.text.push(b' ');
            out.at.push(at);
        }
        word = is_word;
        out.at.extend(std::iter::repeat_n(at, bytes.len()));
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
    at: usize,
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
    let mut out = Vec::new();
    for i in 0..toks.len() {
        if words[i] == "use" {
            use_tree(&words, toks, i + 1, &mut Vec::new(), &mut out);
        } else if words[i] == "extern" && words.get(i + 1) == Some(&"crate") && words.get(i + 3) == Some(&"as") {
            out.push(Import { path: vec![words[i + 2].to_owned()], bind: Bind::Alias, at: toks[i + 2].0 });
        }
    }
    out
}

/// Parse the tree at token I under PREFIX into OUT; the index after it.
fn use_tree(words: &[&str], toks: &[(usize, Tok)], mut i: usize, prefix: &mut Vec<String>, out: &mut Vec<Import>) -> usize {
    let depth = prefix.len();
    let punct = |i: usize, c: u8| toks.get(i).is_some_and(|t| t.1 == Tok::Punct(c));
    let separator = |i: usize| punct(i, b':') && punct(i + 1, b':');
    if separator(i) { i += 2; }
    loop {
        if punct(i, b'*') {
            out.push(Import { path: prefix.clone(), bind: Bind::Glob, at: toks[i].0 });
            i += 1;
            break;
        }
        if punct(i, b'{') {
            i += 1;
            while !punct(i, b'}') && i < toks.len() {
                let next = use_tree(words, toks, i, prefix, out);
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
        let at = toks[i].0;
        let mut path = prefix.clone();
        if name != "self" { path.push(name.to_owned()); }
        i += 1;
        let bind = if words.get(i) == Some(&"as") {
            i += 2;
            if words[i - 1] == "_" { Bind::Name } else { Bind::Alias }
        } else { Bind::Name };
        out.push(Import { path, bind, at });
        break;
    }
    prefix.truncate(depth);
    i
}

/// The offsets of `type NAME<…> = …;` aliases and associated types that name
/// one of TYPES.
fn type_aliases(code: &[u8], toks: &[(usize, Tok)]) -> Vec<usize> {
    let word = |t: Tok| match t {
        Tok::Ident(a, b) => std::str::from_utf8(&code[a..b]).unwrap(),
        _ => "",
    };
    let named = |i: usize| toks.get(i + 1).is_some_and(|t| matches!(t.1, Tok::Ident(..)));
    let raw = |i: usize| i > 0 && toks[i - 1].1 == Tok::Punct(b'#');
    (0..toks.len()).filter(|&i| word(toks[i].1) == "type" && named(i) && !raw(i))
        .filter(|&i| toks[i..].iter().map(|t| t.1).take_while(|&t| t != Tok::Punct(b';')).any(|t| TYPES.contains(&word(t))))
        .map(|i| toks[i].0).collect()
}

/// Whether PATH is one of HOST_PATHS, under one, or a module above one. A
/// pure fs item or extension trait under one names no host API; renaming
/// one of them that is in TYPES (`File`) is a `use-alias`.
fn host_import(path: &[String]) -> bool {
    let pure = |h: &[&str]| path.len() > h.len() && path.last().is_some_and(|n| PURE_FS.contains(&n.as_str()) || n.ends_with("Ext"));
    !path.is_empty() && HOST_PATHS.iter().any(|h| h.iter().zip(path).all(|(a, b)| a == b) && !pure(h))
}

fn violations(rel: &str, v: &Views) -> Vec<Violation> {
    let toks = tokens(&v.code);
    let (code, text) = (compact(&v.code, &toks), compact(&v.text, &toks));
    let mut sites: Vec<(usize, &'static str)> = Vec::new();
    for api in APIS.iter().filter(|a| !a.owners.contains(&rel)) {
        let view = if api.strings { &text } else { &code };
        sites.extend(spellings(api, &view.text).into_iter().map(|i| (view.at[i], api.name)));
    }
    for import in imports(&v.code, &toks) {
        let root = import.path.first().map(String::as_str).unwrap_or("");
        let reaches = ROOTS.contains(&root) || import.path.iter().any(|s| s == "libc");
        let typed = import.path.last().is_some_and(|s| TYPES.contains(&s.as_str()));
        if host_import(&import.path) {
            sites.push((import.at, USE_HOST));
        }
        let form = match import.bind {
            Bind::Alias if reaches || typed => Some(USE_ALIAS),
            Bind::Glob if reaches => Some(USE_GLOB),
            _ => None,
        };
        sites.extend(form.map(|api| (import.at, api)));
        let spelling = import.path.join("::").into_bytes();
        sites.extend(APIS.iter().filter(|a| !a.strings && !spellings(a, &spelling).is_empty()).map(|a| (import.at, a.name)));
    }
    sites.extend(type_aliases(&v.code, &toks).into_iter().map(|at| (at, TYPE_ALIAS)));
    let newlines: Vec<usize> = v.code.iter().enumerate().filter(|&(_, &b)| b == b'\n').map(|(i, _)| i).collect();
    let line = |at: usize| newlines.partition_point(|&n| n < at) + 1;
    let mut out: Vec<Violation> = sites.into_iter().map(|(at, api)| Violation { path: rel.to_owned(), line: line(at), api }).collect();
    out.sort();
    out.dedup();
    out
}

fn known(api: &str) -> bool {
    APIS.iter().any(|a| a.name == api) || FORMS.contains(&api)
}


/// What is wrong with SITES and their HOST COMMENTS: a site no comment
/// covers, and a comment that names an unknown API, an API its line does not
/// call, or no reason.
fn problems(sites: &[Violation], comments: &[Tag]) -> Vec<String> {
    let at = |path: &str, line: Option<usize>, api: &str| sites.binary_search_by(|v| {
        (v.path.as_str(), Some(v.line), v.api).cmp(&(path, line, api))
    }).is_ok();
    let covered: BTreeSet<(&str, usize, &str)> = comments.iter()
        .flat_map(|t| t.apis.iter().filter_map(move |a| Some((t.path.as_str(), t.covers?, a.as_str())))).collect();
    let mut out: Vec<String> = sites.iter().filter(|v| !covered.contains(&(v.path.as_str(), v.line, v.api)))
        .map(|v| format!("  uncovered {}:{} {}", v.path, v.line, v.api)).collect();
    for t in comments {
        for api in &t.apis {
            let fault = if !known(api) { Some("unknown API") }
                else if !at(&t.path, t.covers, api) { Some("covers nothing") }
                else { None };
            out.extend(fault.map(|fault| format!("  {fault}: {}:{} HOST({api})", t.path, t.line)));
        }
        if t.reason.is_empty() {
            out.push(format!("  no reason: {}:{} HOST({})", t.path, t.line, t.apis.join(", ")));
        }
    }
    out
}

fn repository() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..").canonicalize().unwrap()
}


#[test]
fn host_boundary_holds() {
    let base = repository();
    let (sites, comments) = scan(&base, CRATES.iter().flat_map(|c| roots(&base.join(c))).collect());
    let report = problems(&sites, &comments);
    assert!(report.is_empty(), "host boundary: outside {BOUNDARY}, each site needs `// HOST(API): REASON` in the comment \
                                lines directly above it, or a call through host.rs:\n{}", report.join("\n"));
}


/// The fixture crate marks each expected violation with `// expect: API`
/// (several separated by spaces). Its test modules and near misses carry none.
#[test]
fn lint_flags_each_planted_violation() {
    let base = repository().join(FIXTURES);
    let found: BTreeSet<(String, usize, String)> = scan(&base, vec![base.join("src/lib.rs")]).0.into_iter()
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

/// The problems of one source file, `src/lib.rs`.
fn report(source: &str) -> Vec<String> {
    let (sites, comments, _) = file_scan("src/lib.rs", source);
    problems(&sites, &comments)
}

#[test]
fn a_comment_above_a_site_covers_it() {
    let source = "\
fn read() {
    // HOST(std::fs): reads the file the caller named
    let a = std::fs::read(\"a\");
    // A longer story first.
    // HOST(std::env::var, std::fs): both, for one reason
    // that goes on.
    // SAFETY: nothing unsafe happens here.
    let b = unsafe { std::fs::read(std::env::var(\"B\").unwrap()) };
    let c = std::fs::read(
        // HOST(std::env::var): a site's line, not its statement's
        std::env::var(\"C\").unwrap(),
    );
}
";
    assert_eq!(report(source), ["  uncovered src/lib.rs:9 std::fs"]);
    assert_eq!(report(&source.replace("    let c", "    // HOST(std::fs): reads c\n    let c")), Vec::<String>::new());
}

#[test]
fn a_site_no_comment_covers_and_a_comment_that_covers_nothing_fail() {
    let source = "\
fn read() {
    let a = std::fs::read(\"a\");
    // HOST(std::fs): a blank line away

    let b = std::fs::read(\"b\");
    let c = std::fs::read(\"c\"); // HOST(std::fs): on the line itself
    /// HOST(std::fs): a doc comment
    let d = std::fs::read(\"d\");
    // HOST(std::env::var): not what the line calls
    let e = std::fs::read(\"e\");
    // HOST(std::fs):
    let f = std::fs::read(\"f\");
    // HOST(std::fsx): no such API
    let g = 1;
}
#[cfg(test)]
mod tests {
    // HOST(std::fs): test code builds no worlds
    fn t() { std::fs::read(\"t\"); }
}
";
    assert_eq!(report(source), [
        "  uncovered src/lib.rs:2 std::fs",
        "  uncovered src/lib.rs:5 std::fs",
        "  uncovered src/lib.rs:6 std::fs",
        "  uncovered src/lib.rs:8 std::fs",
        "  uncovered src/lib.rs:10 std::fs",
        "  covers nothing: src/lib.rs:3 HOST(std::fs)",
        "  covers nothing: src/lib.rs:9 HOST(std::env::var)",
        "  no reason: src/lib.rs:11 HOST(std::fs)",
        "  unknown API: src/lib.rs:13 HOST(std::fsx)",
        "  covers nothing: src/lib.rs:18 HOST(std::fs)",
    ]);
}
