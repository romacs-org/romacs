//! Host boundary lint, rules 1 to 4 of the world-snapshot design (§6.1).
//!
//! Rule 1: outside `crates/romacs-vm/src/host.rs`, no source in the runner's
//! dependency closure calls a host query or effect API directly. Rule 2: only
//! the environment owners name the environment slots, and only the per-buffer
//! accessor looks up `default-directory`'s storage. Rule 3: each source that
//! names a variable on the B3 trace's delayed list is listed, so no Rust read
//! of one bypasses the fence. Rule 4: each site that
//! can reach a saved value of the VM's binding stack (`vm.dynamic`) is
//! listed, so only specbind, unbind and the saved-binding accessors, which
//! check the fence (F2), read or write one.
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
//! Every match needs an entry in `host_boundary_allow.txt`, one exact `PATH
//! ITEM API MARK REASON` per line: the line of the items named ITEM in the
//! file PATH that spells API and whose text hashes to MARK. ITEM is the path
//! of the enclosing named items (`module::Type::method`; an impl is named by
//! its header, `-` is the file's top level). Two such lines alike in one item
//! fail, since nothing but its position would tell one from the other: make
//! them differ, or join them. So an entry follows its site through every edit
//! but one to the site's own line or its item's name, and an edit to the line
//! asks for its reason again. An entry that matches nothing fails, so the
//! list cannot rot. `ROMACS_HOST_LINT_BLESS=1` deliberately rewrites the list
//! for the current tree: a reason is kept where its entry still names a site;
//! a site no entry names takes the reason of an unused entry of its item,
//! else of its path and API, as a guess marked `pending:`, else it is
//! `pending`. Both fail until the site is migrated or given a reason. The
//! rewrite also re-keys `PATH:LINE API REASON` entries, the list's form until
//! branches forked before this one have merged.
//!
//! The list is a set of entries, and `.gitattributes` names a driver that
//! merges it as one (`tools/lib/host_allow.py`, installed by `tools/derived
//! setup`): two branches conflict only where they change one entry
//! differently. Without the driver git merges the text, and conflicts where
//! they change neighbouring lines.
use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

const CRATES: &[&str] = &[
    "crates/romacs-vm",
    "crates/romacs-face",
    "crates/romacs-layout-input",
    "crates/romacs-display-protocol",
    "crates/romacs-regex",
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

/// One site: the line of the items named ITEM in PATH that spells API and
/// whose tokens hash to MARK. LINE only tells a reader where that is.
#[derive(Clone, PartialEq, Eq, PartialOrd, Ord, Debug)]
struct Violation {
    path: String,
    item: String,
    api: &'static str,
    mark: String,
    line: usize,
}

/// A named item of one file: its byte range and its name.
struct Item {
    from: usize,
    to: usize,
    name: String,
}

/// The named items of CODE in source order, so an enclosing item precedes
/// the ones inside it: functions, modules, traits, impls, types, constants,
/// statics and `macro_rules!` definitions, wherever their keyword stands.
fn items(code: &[u8], toks: &[(usize, Tok)]) -> Vec<Item> {
    let words: Vec<&str> = toks.iter().map(|t| match t.1 {
        Tok::Ident(a, b) => std::str::from_utf8(&code[a..b]).unwrap(),
        _ => "",
    }).collect();
    (0..toks.len()).filter_map(|i| {
        let name = item_name(code, &words, toks, i)?;
        Some(Item { from: toks[i].0, to: item_end(toks, i, code.len(), false), name })
    }).collect()
}

/// The name of the item whose keyword is token I, or None where the word
/// there declares none: a function pointer type, `impl Trait` in a type,
/// `*const T`, `&raw const`, a const generic, `'static`, a raw identifier.
fn item_name(code: &[u8], words: &[&str], toks: &[(usize, Tok)], i: usize) -> Option<String> {
    let punct = |j: usize| match toks.get(j) {
        Some(&(_, Tok::Punct(b))) => b,
        _ => 0,
    };
    let word = |j: usize| words.get(j).copied().filter(|w| !w.is_empty()).map(str::to_owned);
    let (before, after) = (if i == 0 { 0 } else { punct(i - 1) }, words.get(i + 1).copied().unwrap_or(""));
    let previous = if i == 0 { "" } else { words[i - 1] };
    match words[i] {
        _ if before == b'#' => None,
        "fn" | "mod" | "trait" | "struct" | "enum" | "type" => word(i + 1),
        "union" if matches!(punct(i + 2), b'{' | b'<') => word(i + 1),
        "const" if !matches!(before, b'<' | b',' | b'*') && previous != "raw"
            && !matches!(after, "fn" | "unsafe" | "async" | "extern") => word(i + 1),
        "static" if before != b'\'' => word(i + 1 + usize::from(matches!(after, "mut" | "ref"))),
        "macro_rules" if punct(i + 1) == b'!' => word(i + 2).map(|name| name + "!"),
        "impl" if i == 0 || matches!(before, b'}' | b';' | b']' | b'{') || matches!(previous, "unsafe" | "default") => {
            Some(impl_header(code, words, toks, i))
        }
        _ => None,
    }
}

/// The name of the impl at token I: its header between the generics and the
/// body or `where`, spelled without whitespace, a `.` between two words
/// (`Type<T>`, `Trait.for.Type`).
fn impl_header(code: &[u8], words: &[&str], toks: &[(usize, Tok)], i: usize) -> String {
    let punct = |j: usize, b: u8| toks.get(j).is_some_and(|t| t.1 == Tok::Punct(b));
    let mut from = i + 1;
    if punct(from, b'<') {
        let mut depth = 0;
        while from < toks.len() {
            if punct(from, b'<') { depth += 1; }
            if punct(from, b'>') && !punct(from - 1, b'-') { depth -= 1; }
            from += 1;
            if depth == 0 { break; }
        }
    }
    let mut depth = 0;
    let to = (from..toks.len()).find(|&j| {
        if punct(j, b'(') || punct(j, b'[') { depth += 1; }
        if punct(j, b')') || punct(j, b']') { depth -= 1; }
        depth == 0 && (punct(j, b'{') || punct(j, b';') || words[j] == "where")
    }).unwrap_or(toks.len());
    String::from_utf8_lossy(&compact(code, &toks[from..to]).text).replace(' ', ".")
}

/// The names of the items around offset AT, outermost first, or `-`.
fn item_path(items: &[Item], at: usize) -> String {
    let names: Vec<&str> = items.iter().filter(|i| i.from <= at && at < i.to).map(|i| i.name.as_str()).collect();
    if names.is_empty() { "-".to_owned() } else { names.join("::") }
}

/// The violations of REL for SITES, each an item, an API and a line; SPELT
/// is each line's text. A site is one line of an item that spells an API,
/// however often. Its mark is the hash of the line's text, so no other line
/// can take its entry; lines alike share a key, which `problems` refuses.
fn marked(rel: &str, sites: Vec<(String, &'static str, usize)>, spelt: &BTreeMap<usize, Vec<u8>>) -> Vec<Violation> {
    // FNV-1a, 64 bits: two lines of one item do not share a mark by chance.
    let hash = |line: usize| spelt[&line].iter().fold(0xcbf2_9ce4_8422_2325_u64, |h, &b| (h ^ u64::from(b)).wrapping_mul(0x0100_0000_01b3));
    let mut sites: Vec<(String, &'static str, u64, usize)> = sites.into_iter().map(|(item, api, line)| (item, api, hash(line), line)).collect();
    sites.sort();
    sites.dedup();
    sites.into_iter().map(|(item, api, hash, line)| Violation { path: rel.to_owned(), item, api, mark: format!("{hash:016x}"), line }).collect()
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
    out
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
    // Each line's tokens on their own, a space only between two words of it.
    let mut spelt: BTreeMap<usize, Vec<u8>> = BTreeMap::new();
    let mut last: Option<(usize, bool)> = None;
    for &(at, tok) in &toks {
        let (here, word) = (line(at), !matches!(tok, Tok::Punct(_)));
        let out = spelt.entry(here).or_default();
        if word && last == Some((here, true)) { out.push(b' '); }
        last = Some((here, word));
        match tok {
            Tok::Ident(a, b) => out.extend_from_slice(&v.text[a..b]),
            Tok::Punct(b) => out.push(b),
            Tok::Str(a, b) => out.extend_from_slice(&v.text[a - 1..=b]),
        }
    }
    // A character literal is no token: the code view blanks it whole.
    for (at, (&b, &code)) in v.text.iter().zip(&v.code).enumerate() {
        if code == b' ' && !b.is_ascii_whitespace() { spelt.entry(line(at)).or_default().push(b); }
    }
    let items = items(&v.code, &toks);
    marked(rel, sites.into_iter().map(|(at, api)| (item_path(&items, at), api, line(at))).collect(), &spelt)
}

fn known(api: &str) -> bool {
    APIS.iter().any(|a| a.name == api) || FORMS.contains(&api)
}

const HEADER: &str = "# Host boundary allowlist: PATH ITEM API MARK REASON (host_boundary_lint.rs).\n\
                      # Every entry names one exact site and must match it.\n";
const PENDING: &str = "pending";

/// Where an entry's site is: the line with a mark in the items of a name,
/// or, in the list's former form, a line of the file. Delete `Line` once
/// the branches forked before the marks have merged.
#[derive(PartialEq)]
enum Place {
    Item(String, String),
    Line(usize),
}

struct Entry {
    path: String,
    place: Place,
    api: String,
    reason: String,
}

impl Entry {
    fn covers(&self, v: &Violation) -> bool {
        self.path == v.path && self.api == v.api && match &self.place {
            Place::Item(item, mark) => *item == v.item && *mark == v.mark,
            Place::Line(line) => *line == v.line,
        }
    }

    /// Whether the reason is none: nothing, `pending`, or a `pending:` guess.
    fn pending(&self) -> bool {
        self.reason.is_empty() || self.reason == PENDING || self.reason.starts_with("pending:")
    }
}

fn parse_entries(text: &str) -> Vec<Entry> {
    text.lines().filter(|l| !l.trim().is_empty() && !l.starts_with('#')).map(|l| {
        let mut words = l.split_whitespace();
        let mut word = |what: &str| words.next().unwrap_or_else(|| panic!("no {what}: {l}"));
        let first = word("path");
        let line = first.rsplit_once(':').and_then(|(path, line)| Some((path, line.parse().ok()?)));
        let (path, place, api) = if let Some((path, line)) = line {
            (path, Place::Line(line), word("API"))
        } else {
            let (item, api, mark) = (word("item"), word("API"), word("mark"));
            assert!(mark.len() == 16 && mark.bytes().all(|b| b.is_ascii_hexdigit()), "no mark: {l}");
            (first, Place::Item(item.to_owned(), mark.to_owned()), api)
        };
        assert!(known(api), "unknown API: {l}");
        Entry { path: path.to_owned(), place, api: api.to_owned(), reason: words.collect::<Vec<_>>().join(" ") }
    }).collect()
}

fn render(e: &Entry) -> String {
    match &e.place {
        Place::Item(item, mark) => format!("{} {item} {} {mark} {}", e.path, e.api, e.reason),
        Place::Line(line) => format!("{}:{line} {} {}", e.path, e.api, e.reason),
    }
}

/// What is wrong with the list TEXT for the sites FOUND: lines alike in one
/// item, an entry that is keyed by a line, has no reason or repeats
/// another's key, a site no entry covers, and an entry that covers none.
fn problems(text: &str, found: &[Violation]) -> Vec<String> {
    let entries = parse_entries(text);
    let mut out = Vec::new();
    let key = |w: &Violation| (w.path.clone(), w.item.clone(), w.api, w.mark.clone());
    for (n, v) in found.iter().enumerate() {
        if n > 0 && key(&found[n - 1]) == key(v) { continue; }
        let lines: Vec<String> = found[n..].iter().take_while(|w| key(w) == key(v)).map(|w| w.line.to_string()).collect();
        if lines.len() > 1 {
            out.push(format!("  lines alike: {} {} {} {} (lines {}): make them differ, or join them",
                             v.path, v.item, v.api, v.mark, lines.join(", ")));
        }
    }
    for (n, e) in entries.iter().enumerate() {
        let twice = entries[..n].iter().any(|d| d.path == e.path && d.place == e.place && d.api == e.api);
        let fault = if matches!(e.place, Place::Line(_)) { Some("keyed by line") }
            else if e.pending() { Some("no reason") }
            else { twice.then_some("listed twice") };
        out.extend(fault.map(|fault| format!("  {fault}: {}", render(e))));
    }
    let unlisted = found.iter().filter(|v| !entries.iter().any(|e| e.covers(v)));
    out.extend(unlisted.map(|v| format!("  unlisted {} {} {} {} (line {})", v.path, v.item, v.api, v.mark, v.line)));
    let stale = entries.iter().filter(|e| !found.iter().any(|v| e.covers(v)));
    out.extend(stale.map(|e| format!("  stale {}", render(e))));
    out
}

/// The allowlist for FOUND: one entry per site. A reason is kept from the
/// entry that covers the site. An entry keyed by a line covers a site only
/// in a list of such entries, the list before its migration; beside entries
/// keyed by items its line is of another tree. A site left without takes,
/// as a guess marked `pending:`, the reason of an unused entry with its
/// path and API: one with its mark, else of its item, else any. With none
/// it is `pending`.
fn bless(mut entries: Vec<Entry>, found: &[Violation]) -> String {
    let mut found: Vec<&Violation> = found.iter().collect();
    found.dedup_by_key(|v| (&v.path, &v.item, v.api, &v.mark));
    let migrated = entries.iter().any(|e| matches!(e.place, Place::Item(..)));
    let mut reasons: Vec<Option<String>> = found.iter().map(|v| {
        let trusted = |e: &Entry| e.covers(v) && !(migrated && matches!(e.place, Place::Line(_)));
        let at = entries.iter().position(trusted)?;
        Some(entries.remove(at).reason)
    }).collect();
    for (v, reason) in found.iter().zip(&mut reasons) {
        if reason.is_none() {
            let beside = |e: &Entry| e.path == v.path && e.api == v.api;
            let alike = |e: &Entry| beside(e) && matches!(&e.place, Place::Item(_, mark) if *mark == v.mark);
            let within = |e: &Entry| beside(e) && matches!(&e.place, Place::Item(item, _) if *item == v.item);
            let near = [&alike as &dyn Fn(&Entry) -> bool, &within, &beside].into_iter().find_map(|like| entries.iter().position(like));
            *reason = Some(match near.map(|at| entries.remove(at)) {
                Some(guess) if !guess.pending() => format!("{PENDING}: {}", guess.reason),
                Some(guess) if !guess.reason.is_empty() => guess.reason,
                _ => PENDING.to_owned(),
            });
        }
    }
    let mut out = HEADER.to_owned();
    for (v, reason) in found.iter().zip(reasons) {
        let e = Entry { path: v.path.clone(), place: Place::Item(v.item.clone(), v.mark.clone()), api: v.api.to_owned(), reason: reason.unwrap() };
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
        text = bless(parse_entries(&text), &found);
        std::fs::write(&allow, &text).unwrap();
    }
    let report = problems(&text, &found);
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

/// The sites of one source file, `src/lib.rs`.
fn sites(source: &str) -> Vec<Violation> {
    violations("src/lib.rs", &views(source))
}

/// Each site of SOURCE as its item, API and line, in line order.
fn named(source: &str) -> Vec<(String, &'static str, usize)> {
    let mut out: Vec<_> = sites(source).into_iter().map(|v| (v.item, v.api, v.line)).collect();
    out.sort_by_key(|site| site.2);
    out
}

/// The list the rewrite makes of OLD for SOURCE, each new or guessed entry
/// given the reason `for` and the source text of its line.
fn listed(old: &str, source: &str) -> String {
    let found = sites(source);
    let lines: Vec<String> = bless(parse_entries(old), &found).lines().map(|l| {
        let words: Vec<&str> = l.split(' ').collect();
        if l.starts_with('#') || !words[4].starts_with(PENDING) { return l.to_owned(); }
        let site = found.iter().find(|v| (v.item.as_str(), v.api, v.mark.as_str()) == (words[1], words[2], words[3])).unwrap();
        format!("{} for {}", words[..4].join(" "), source.lines().nth(site.line - 1).unwrap().trim())
    }).collect();
    lines.join("\n") + "\n"
}

/// The mark of the site of SOURCE whose line holds TEXT.
fn mark(source: &str, text: &str) -> String {
    sites(source).into_iter().find(|v| source.lines().nth(v.line - 1).unwrap().contains(text)).unwrap().mark
}

/// The reason LIST gives the site of SOURCE whose line holds TEXT.
fn reason(list: &str, source: &str, text: &str) -> String {
    let site = sites(source).into_iter().find(|v| source.lines().nth(v.line - 1).unwrap().contains(text)).unwrap();
    parse_entries(list).into_iter().find(|e| e.covers(&site)).map(|e| e.reason).unwrap_or_default()
}

const NONE: [&str; 0] = [];
const READS: &str = "\
fn first() {
    std::fs::read(\"a\");
}
mod inner {
    impl Disk for Tape {
        fn load(&self) {
            std::fs::read(\"b\");
            let _ = std::env::var(\"C\");
            std::fs::read(\"d\");
        }
    }
}
";

#[test]
fn an_entry_follows_its_site_through_edits_elsewhere() {
    let list = listed("", READS);
    assert_eq!(named(READS), [
        ("first".to_owned(), "std::fs", 2),
        ("inner::Disk.for.Tape::load".to_owned(), "std::fs", 7),
        ("inner::Disk.for.Tape::load".to_owned(), "std::env::var", 8),
        ("inner::Disk.for.Tape::load".to_owned(), "std::fs", 9),
    ]);
    assert_eq!(problems(&list, &sites(READS)), NONE);
    let lines = |source: &str| sites(source).iter().map(|v| v.line).collect::<Vec<_>>();
    // Lines inserted above every site and between two of one item; the
    // items in another order; two sites of one item and API exchanged; a
    // site's line laid out anew.
    let shifted = format!("// A comment.\n\nfn before() {{}}\n{}", READS.replace("let _", "\n\n            let _"));
    let (first, inner) = READS.split_at(READS.find("mod inner").unwrap());
    let moved = format!("{inner}{first}");
    let exchanged = READS.replace("read(\"b\")", "read(\"x\")").replace("read(\"d\")", "read(\"b\")").replace("read(\"x\")", "read(\"d\")");
    let returned = READS.replace("fn first() {\n    std::fs", "fn first() {\n    return\n    std::fs");
    assert_eq!(mark(&returned, "\"a\""), mark(READS, "\"a\""));
    let spaced = READS.replace("std::fs::read(\"d\");", "std :: fs::read( \"d\" ) ; // Why.");
    for source in [&shifted, &moved, &exchanged, &spaced] {
        assert_eq!(problems(&list, &sites(source)), NONE);
        assert_eq!(bless(parse_entries(&list), &sites(source)), list);
        for text in ["\"a\"", "\"b\"", "\"C\"", "\"d\""] {
            assert_eq!(reason(&list, source, text), reason(&list, READS, text), "{text} in:\n{source}");
        }
    }
    assert_ne!(lines(&shifted), lines(READS));
    assert_ne!(lines(&moved), lines(READS));
    assert_eq!(reason(&list, READS, "\"d\""), "for std::fs::read(\"d\");");
}

#[test]
fn a_removed_site_and_a_new_site_are_reported() {
    let list = listed("", READS);
    let d = mark(READS, "\"d\"");
    let removed = READS.replace("            std::fs::read(\"d\");\n", "");
    assert_eq!(problems(&list, &sites(&removed)), [
        format!("  stale src/lib.rs inner::Disk.for.Tape::load std::fs {d} for std::fs::read(\"d\");"),
    ]);
    // A new site, here ahead of a listed one of its item and API, is the
    // one reported, and the listed one keeps its entry.
    let added = READS.replace("fn first() {", "fn first() {\n    std::fs::read(\"e\");");
    assert_eq!(problems(&list, &sites(&added)), [format!("  unlisted src/lib.rs first std::fs {} (line 2)", mark(&added, "\"e\""))]);
    assert_eq!(reason(&list, &added, "\"a\""), "for std::fs::read(\"a\");");
    // One site removed and another added in its item: as many sites as
    // entries, and both are reported.
    let replaced = READS.replace("std::fs::read(\"d\");", "std::fs::read(\"z\");");
    assert_eq!(problems(&list, &sites(&replaced)), [
        format!("  unlisted src/lib.rs inner::Disk.for.Tape::load std::fs {} (line 9)", mark(&replaced, "\"z\"")),
        format!("  stale src/lib.rs inner::Disk.for.Tape::load std::fs {d} for std::fs::read(\"d\");"),
    ]);
    // The rewrite offers the unused reason of that item as a guess, which
    // does not pass.
    let rewritten = bless(parse_entries(&list), &sites(&replaced));
    assert_eq!(reason(&rewritten, &replaced, "\"z\""), "pending: for std::fs::read(\"d\");");
    assert_eq!(reason(&rewritten, &replaced, "\"b\""), "for std::fs::read(\"b\");");
    assert_eq!(problems(&rewritten, &sites(&replaced)).len(), 1, "{rewritten}");
    assert!(problems(&rewritten, &sites(&replaced))[0].starts_with("  no reason: "));
    // Two lines alike in one item fail, whatever the list says, even where
    // the list before the migration gave them reasons of their own.
    let alike = "fn both() {\n    std::fs::read(\"a\");\n    let _ = 1;\n    std::fs::read(\"a\");\n}\n";
    let by_line = "src/lib.rs:2 std::fs one reason\nsrc/lib.rs:4 std::fs another reason\n";
    let both = bless(parse_entries(by_line), &sites(alike));
    assert_eq!(both.lines().count(), 3, "{both}");
    assert_eq!(problems(&both, &sites(alike)), [
        format!("  lines alike: src/lib.rs both std::fs {} (lines 2, 4): make them differ, or join them", mark(alike, "\"a\"")),
    ]);
    let joined = alike.replacen("    std::fs::read(\"a\");\n", "", 1);
    assert_eq!(problems(&both, &sites(&joined)), NONE);
    // A character on the line is part of what the reason was given for.
    let dashed = "fn f(p: &str) { std::fs::read(p.replace('a', \"b\")); }\n";
    assert_ne!(mark(dashed, "read"), mark(&dashed.replace("'a'", "'/'"), "read"));
    let twice = format!("{list}src/lib.rs inner::Disk.for.Tape::load std::fs {d} another reason\n");
    assert_eq!(problems(&twice, &sites(READS)), [format!("  listed twice: src/lib.rs inner::Disk.for.Tape::load std::fs {d} another reason")]);
}

#[test]
fn a_site_is_named_by_its_items() {
    let source = "\
use std::fs::read as slurp;
#[cfg(unix)]
fn twin() { std::fs::read(\"unix\"); }
#[cfg(not(unix))]
fn twin() { std::fs::read(\"other\"); }
impl<'a, T: Fn() -> u8> Leaf<'a, T> {
    fn new(log: &mut impl std::io::Write) -> impl Sized {
        static ON: bool = std::env::var(\"ON\").is_ok();
        std::fs::read(\"same\");
        std::fs::read(\"same\");
        let ptr: *const u8 = std::fs::read(\"p\").unwrap().as_ptr();
    }
}
impl<const N: usize> Logged for [u64; N] where u8: Copy {
    const LOG: fn(&str) = |_| { std::fs::read(\"log\"); };
}
macro_rules! open { ($p:expr) => { std::fs::read($p) } }
unsafe impl Send for Tape {}
fn last() { std::fs::read(\"last\"); }
";
    let found = named(source);
    let item = |line: usize| found.iter().find(|site| site.2 == line && site.1 == "std::fs").map(|site| site.0.as_str());
    assert_eq!(item(1), Some("-"));
    assert_eq!((item(3), item(5)), (Some("twin"), Some("twin")));
    assert_eq!((item(9), item(10), item(11)), (Some("Leaf<'a,T>::new"), Some("Leaf<'a,T>::new"), Some("Leaf<'a,T>::new")));
    assert_eq!(found.iter().find(|site| site.1 == "std::env::var").unwrap().0, "Leaf<'a,T>::new::ON");
    assert_eq!(item(15), Some("Logged.for[u64;N]::LOG"));
    assert_eq!(item(17), Some("open!"));
    assert_eq!(item(19), Some("last"));
    // Sites have keys of their own, the twins by their lines' text; only
    // lines alike in one item share one, which fails.
    let keys: BTreeSet<(String, &str, String)> = sites(source).into_iter().map(|v| (v.item, v.api, v.mark)).collect();
    assert_eq!(keys.len(), sites(source).len() - 1);
    let alike: Vec<String> = problems(&listed("", source), &sites(source));
    assert!(alike.len() == 1 && alike[0].starts_with("  lines alike: src/lib.rs Leaf<'a,T>::new std::fs ") && alike[0].contains("(lines 9, 10)"), "{alike:?}");
}

#[test]
fn the_rewrite_keys_a_line_entry_by_its_item() {
    let by_line = "src/lib.rs:2 std::fs because\n";
    assert_eq!(problems(by_line, &sites(READS))[0], "  keyed by line: src/lib.rs:2 std::fs because");
    assert_eq!(reason(&bless(parse_entries(by_line), &sites(READS)), READS, "\"a\""), "because");
    // Beside migrated entries, a line is of a tree before them: the site
    // now at line 9 is not the one this entry was written for.
    let list = listed("", READS);
    let b = list.lines().find(|l| l.ends_with("read(\"b\");")).unwrap();
    let mixed = format!("src/lib.rs:9 std::fs of another site\n{}", list.replace(&format!("{b}\n"), ""));
    let rewritten = bless(parse_entries(&mixed), &sites(READS));
    assert_eq!(reason(&rewritten, READS, "\"d\""), "for std::fs::read(\"d\");");
    assert_eq!(reason(&rewritten, READS, "\"b\""), "pending: of another site");
    // An item under a new name: the guess is the entry with the line's mark.
    let renamed = READS.replace("fn load", "fn fetch");
    let rewritten = bless(parse_entries(&list), &sites(&renamed));
    for text in ["\"b\"", "\"C\"", "\"d\""] {
        assert_eq!(reason(&rewritten, &renamed, text), format!("pending: {}", reason(&list, READS, text)));
    }
}

/// Git in DIR, with no configuration but its own.
fn git(dir: &Path, args: &[&str]) -> bool {
    let mut command = std::process::Command::new("git");
    for (name, _) in std::env::vars_os().filter(|(name, _)| name.to_string_lossy().starts_with("GIT_")) {
        command.env_remove(name);
    }
    command.envs([("GIT_CONFIG_GLOBAL", "/dev/null"), ("GIT_CONFIG_NOSYSTEM", "1")]).current_dir(dir)
        .args(["-c", "user.name=lint", "-c", "user.email=lint@example.invalid", "-c", "commit.gpgsign=false"]);
    command.args(args).output().unwrap().status.success()
}

/// Two branches change different functions of one file. Each moves the
/// other's sites down, changes the line of one of the trunk's two
/// neighbouring entries, and lists a new site that sorts between them. With
/// the repository's merge driver their lists merge without a conflict, and
/// the merged list is the merged source's.
#[test]
fn branches_that_edit_different_functions_merge_their_lists() {
    const TRUNK: &str = "\
fn alpha() {
    std::fs::read(\"a\");
}

fn middle() {}

fn zeta() {
    std::fs::write(\"y\", \"z\");
}
";
    let root = repository();
    let attributes: String = std::fs::read_to_string(root.join(".gitattributes")).unwrap().lines()
        .filter(|l| l.starts_with(ALLOW)).map(|l| format!("{l}\n")).collect();
    assert_eq!(attributes, format!("{ALLOW} merge=romacs-host-allow\n"));
    let dir = Path::new(env!("CARGO_TARGET_TMPDIR")).join(format!("host-lint-merge-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let (source, list) = (dir.join("src/lib.rs"), dir.join(ALLOW));
    std::fs::create_dir_all(source.parent().unwrap()).unwrap();
    std::fs::create_dir_all(list.parent().unwrap()).unwrap();
    std::fs::write(dir.join(".gitattributes"), attributes).unwrap();
    let commit = |text: &str| {
        let old = std::fs::read_to_string(&list).unwrap_or_default();
        std::fs::write(&source, text).unwrap();
        std::fs::write(&list, listed(&old, text)).unwrap();
        assert!(git(&dir, &["add", "."]) && git(&dir, &["commit", "-m", "change"]));
    };
    let driver = format!("python3 -B {} merge %O %A %B", root.join("tools/lib/host_allow.py").display());
    assert!(git(&dir, &["init", "-b", "trunk"]) && git(&dir, &["config", "merge.romacs-host-allow.driver", &driver]));
    commit(TRUNK);
    assert!(git(&dir, &["checkout", "-b", "left"]));
    commit(&TRUNK.replace("read(\"a\")", "read(\"b\")").replace("fn middle() {}\n", "fn gamma() {\n\n\n    std::fs::read(\"g\");\n}\n\nfn middle() {}\n"));
    assert!(git(&dir, &["checkout", "-b", "right", "trunk"]));
    commit(&format!("{}\nfn delta() {{\n    std::fs::read(\"d\");\n}}\n", TRUNK.replace("\"z\"", "\"x\"")));
    assert!(git(&dir, &["merge", "--no-edit", "left"]), "the lists conflict");
    let (merged, both) = (std::fs::read_to_string(&list).unwrap(), std::fs::read_to_string(&source).unwrap());
    assert_eq!(named(&both).iter().map(|site| site.0.as_str()).collect::<Vec<_>>(), ["alpha", "gamma", "zeta", "delta"]);
    assert_eq!(merged, listed("", &both));
    assert_eq!(problems(&merged, &sites(&both)), NONE);
    std::fs::remove_dir_all(&dir).unwrap();
}
