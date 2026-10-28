//! The host boundary: the VM's and its embedders' host queries and effects
//! (files, processes, sockets) go through this module, so a world builder can
//! record and classify each one. `tests/host_boundary_lint.rs` keeps other
//! sources from calling the host APIs it names directly.
//!
//! No read descriptor leaves this module (`Input`). Output files, pipes,
//! ptys, sockets and children are made only here, so a refusal at these entry
//! points also covers the descriptor calls made on them elsewhere.
//!
//! In `Mode::Off`, the default, each function is a direct passthrough: it
//! returns exactly what the host call returns. In `Mode::Build` each query is
//! also recorded with its site and classified (`class`): a keyed (K) fact is
//! recorded in full, a per-process (P) fact read outside startup refuses the
//! build, and an effect (F) refuses and fails with `EPERM`. Refusals are
//! listed, not raised: the trace names every one, and a builder refuses to
//! publish a world that has any.
use serde_json::{Value as Json, json};
use std::cell::{Cell, RefCell};
use std::ffi::{CStr, OsStr, OsString};
use std::fs::{File, FileType, Metadata};
use std::io;
use std::net::{SocketAddr, TcpListener, TcpStream};
use std::os::fd::{FromRawFd, OwnedFd};
use std::os::unix::ffi::OsStrExt;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::rc::Rc;
use std::time::{Instant, SystemTime};

pub use std::fs::OpenOptions;

/// How the boundary treats host access. Per thread: tests run VMs on
/// parallel threads, and some callers have no `Vm`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Mode {
    #[default]
    Off,
    /// A world build: record, classify, refuse.
    Build,
}

thread_local! {
    static MODE: Cell<Mode> = const { Cell::new(Mode::Off) };
    static BUILD: RefCell<Option<Build>> = const { RefCell::new(None) };
}

pub fn mode() -> Mode {
    MODE.with(Cell::get)
}

fn building() -> bool {
    mode() == Mode::Build
}

// Build mode.

/// Where a build is. P facts may be read while the VM is made (`Init`, which
/// restore repeats) and inside the startup function (`Startup`, which restore
/// reruns), not while the recipe runs.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Phase {
    Init,
    Recipe,
    Startup,
}

/// The pinned answers to the P facts that the launch environment and working
/// directory cannot carry.
#[derive(Clone, Copy, Debug)]
pub struct Pinned {
    /// The build time, `t_b`.
    pub clock: SystemTime,
    pub pid: u32,
    pub seed: [u8; 4],
}

/// The top-level form being loaded when a fact is read.
#[derive(Debug)]
pub struct Site {
    pub file: String,
    pub line: usize,
    /// The form's first line, shortened.
    pub head: String,
}

impl Site {
    fn json(&self) -> Json {
        json!({"file": self.file, "line": self.line, "form": self.head})
    }

    fn at(&self, (file, head): (&str, &str)) -> bool {
        self.file.ends_with(file) && self.head.starts_with(head)
    }
}

/// A host fact, as `class` sees it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Fact<'a> {
    /// One environment name, through the host or a Lisp environment list.
    Env(&'a [u8]),
    /// The whole environment.
    Environment,
    CurrentDir,
    Pid,
    Clock,
    Monotonic,
    Seed,
    /// Files, the user database, the machine and every other query.
    Other,
    /// A write, child, socket, lock or terminal.
    Effect,
}

/// §6.2's classes: keyed, baked, per-process, forbidden.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Class {
    K,
    B,
    P,
    F,
}

/// The per-process environment names. Every other name is keyed: a wrong K
/// costs a refused world, a wrong P a wrong one.
pub const P_ENV: [&str; 15] = [
    "PWD", "OLDPWD", "TMPDIR", "TMP", "TEMP", "PATH", "TERM", "COLORTERM", "DISPLAY",
    "WAYLAND_DISPLAY", "INSIDE_EMACS", "SHLVL", "COLUMNS", "LINES", "_",
];

/// The baked site, (file suffix, form head): `emacs-build-time` is the time
/// the world was built, as a dumped GNU's is its dump time.
const BAKED: (&str, &str) = ("/version.el", "(defconst emacs-build-time");

/// Load-time forms outside startup that GNU evaluates in each process and
/// restore re-seeds, as (file suffix, form head). Their P read is allowed.
const RESEED: [(&str, &str); 0] = [];

/// The one site that may switch the startup phase.
const STARTUP: &str = "/rlisp/romacs-loadup-bootstrap.el";

pub fn class(fact: Fact<'_>, site: Option<&Site>) -> Class {
    match fact {
        Fact::Env(name) => if P_ENV.iter().any(|p| p.as_bytes() == name) { Class::P } else { Class::K },
        Fact::Clock if site.is_some_and(|s| s.at(BAKED)) => Class::B,
        Fact::Environment | Fact::CurrentDir | Fact::Pid | Fact::Clock | Fact::Monotonic | Fact::Seed => Class::P,
        Fact::Other => Class::K,
        Fact::Effect => Class::F,
    }
}

struct Build {
    pinned: Pinned,
    phase: Phase,
    startups: usize,
    site: Option<Rc<Site>>,
    records: Vec<Json>,
    refusals: Vec<Json>,
    seen: std::collections::HashSet<String>,
}

impl Build {
    fn site(&self) -> Json {
        self.site.as_ref().map_or(Json::Null, |s| s.json())
    }

    /// Push ITEM onto the records or the refusals, once.
    fn push(&mut self, refusal: bool, item: Json) {
        let key = format!("{refusal}{item}");
        if self.seen.insert(key) {
            if refusal { self.refusals.push(item) } else { self.records.push(item) }
        }
    }
}

fn with_build<T>(f: impl FnOnce(&mut Build) -> T) -> Option<T> {
    BUILD.with(|b| b.borrow_mut().as_mut().map(f))
}

/// Start recording a build on this thread, in `Phase::Init`.
pub fn enter_build(pinned: Pinned) {
    BUILD.with(|b| *b.borrow_mut() = Some(Build {
        pinned, phase: Phase::Init, startups: 0, site: None,
        records: Vec::new(), refusals: Vec::new(), seen: Default::default(),
    }));
    MODE.with(|m| m.set(Mode::Build));
}

/// Stop recording: `{"records": [...], "refusals": [...]}`, in first-occurrence
/// order. `None` when no build was recording.
pub fn leave_build() -> Option<Json> {
    MODE.with(|m| m.set(Mode::Off));
    BUILD.with(|b| b.borrow_mut().take()).map(|b| json!({"records": b.records, "refusals": b.refusals}))
}

/// The VM is made: the recipe starts.
pub fn begin_recipe() {
    with_build(|b| b.phase = Phase::Recipe);
}

/// Whether the startup function is running in a build.
pub(crate) fn in_startup() -> bool {
    with_build(|b| b.phase == Phase::Startup).unwrap_or(false)
}

/// `romacs--startup-phase`: enter or leave startup. Only the bootstrap's
/// single run may switch; any other switch refuses and changes nothing.
/// True when the phase changed.
pub(crate) fn switch_startup(on: bool) -> bool {
    with_build(|b| {
        let from = match on { true => Phase::Recipe, false => Phase::Startup };
        let allowed = b.phase == from
            && b.site.as_ref().is_some_and(|s| s.file.ends_with(STARTUP))
            && (!on || b.startups == 0);
        if allowed {
            b.phase = if on { Phase::Startup } else { Phase::Recipe };
            b.startups += usize::from(on);
        }
        let item = json!({"kind": "startup", "site": b.site(), "on": on});
        b.push(!allowed, item);
        allowed
    }).unwrap_or(false)
}

/// Restores the enclosing site when a top-level form ends.
pub struct SiteGuard(Option<Option<Rc<Site>>>);

impl Drop for SiteGuard {
    fn drop(&mut self) {
        if let Some(previous) = self.0.take() {
            with_build(|b| b.site = previous);
        }
    }
}

/// Make the form SITE describes the site of what follows, until the guard
/// drops. SITE is built only in build mode.
pub(crate) fn enter_site(site: impl FnOnce() -> Site) -> SiteGuard {
    if !building() {
        return SiteGuard(None);
    }
    SiteGuard(with_build(|b| std::mem::replace(&mut b.site, Some(Rc::new(site())))))
}

/// Record a K fact: KIND with FIELDS and the current site.
fn record(kind: &str, fields: Json) {
    with_build(|b| {
        let mut item = json!({"kind": kind, "site": b.site()});
        item.as_object_mut().unwrap().extend(fields.as_object().cloned().unwrap_or_default());
        b.push(false, item);
    });
}

/// List a refusal: KIND with FIELDS and the current site.
pub(crate) fn refuse(kind: &str, fields: Json) {
    with_build(|b| {
        let mut item = json!({"kind": kind, "site": b.site()});
        item.as_object_mut().unwrap().extend(fields.as_object().cloned().unwrap_or_default());
        b.push(true, item);
    });
}

/// A P fact WHAT is read: recorded where it is allowed, else refused.
fn p_read(fact: Fact<'_>, what: String) {
    with_build(|b| {
        let site = b.site.as_deref();
        let allowed = match (b.phase, class(fact, site)) {
            (Phase::Init, _) => Some("init"),
            (Phase::Startup, _) => Some("startup"),
            (Phase::Recipe, Class::B) => Some("baked"),
            (Phase::Recipe, _) if site.is_some_and(|s| RESEED.iter().any(|&r| s.at(r))) => Some("reseed"),
            _ => None,
        };
        let item = json!({"kind": "p-read", "site": b.site(), "what": what, "allowed": allowed});
        b.push(allowed.is_none(), item);
    });
}

/// An effect WHAT: refused, failing with EPERM; Ok outside build mode.
fn effect(what: impl FnOnce() -> String) -> io::Result<()> {
    if !building() {
        return Ok(());
    }
    refuse("effect", json!({"what": what()}));
    Err(io::Error::from_raw_os_error(libc::EPERM))
}

fn bytes(path: &Path) -> String {
    String::from_utf8_lossy(path.as_os_str().as_bytes()).into_owned()
}

fn errno(error: &io::Error) -> Json {
    json!({"errno": error.raw_os_error()})
}

fn sha256(data: &[u8]) -> String {
    use sha2::Digest;
    sha2::Sha256::digest(data).iter().map(|b| format!("{b:02x}")).collect()
}

/// A file's content as a record: its length and digest, or the error.
fn file_read(path: &Path, result: &io::Result<impl AsRef<[u8]>>) {
    let result = match result {
        Ok(data) => json!({"len": data.as_ref().len(), "sha256": sha256(data.as_ref())}),
        Err(e) => errno(e),
    };
    record("file-read", json!({"path": bytes(path), "result": result}));
}

fn probed(kind: &str, path: &Path, result: Json) {
    record("probe", json!({"probe": kind, "path": bytes(path), "result": result}));
}

/// The facts of a stat that Lisp can observe.
fn stat_json(meta: &io::Result<Metadata>) -> Json {
    use std::os::unix::fs::MetadataExt;
    match meta {
        Ok(m) => json!({"mode": m.mode(), "len": m.len(), "mtime": [m.mtime(), m.mtime_nsec()],
                        "dev": m.dev(), "ino": m.ino(), "nlink": m.nlink(), "uid": m.uid(), "gid": m.gid()}),
        Err(e) => errno(e),
    }
}

// Environment.

pub fn getenv(name: &str) -> Option<OsString> {
    let value = std::env::var_os(name);
    if building() {
        env_read("host", name.as_bytes(), value.as_ref());
    }
    value
}

/// A read of environment NAME through FORM: a K name is recorded with its
/// launch VALUE, a P name is a P read.
fn env_read(form: &str, name: &[u8], value: Option<&OsString>) {
    let shown = String::from_utf8_lossy(name);
    match class(Fact::Env(name), None) {
        Class::P => p_read(Fact::Env(name), format!("env {shown} ({form})")),
        _ => record("env", json!({"form": form, "name": shown,
                                  "value": value.map(|v| String::from_utf8_lossy(v.as_bytes()).into_owned())})),
    }
}

/// The process environment, in the host's order.
pub fn env_snapshot() -> Vec<(OsString, OsString)> {
    if building() {
        p_read(Fact::Environment, "environment".into());
    }
    std::env::vars_os().collect()
}

/// Which Lisp list a Rust environment lookup walks.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EnvList {
    /// `process-environment`.
    Process,
    /// `initial-environment`.
    Initial,
    /// A list the caller passed.
    Explicit,
}

/// A Rust lookup of NAME in a Lisp environment list. The lookup itself reads
/// Lisp data; this marks it as a host fact, recorded with the launch value
/// whatever list was walked. A no-op in `Mode::Off`.
pub fn lisp_env(list: EnvList, name: &[u8]) {
    if building() {
        let form = match list { EnvList::Process => "process", EnvList::Initial => "initial", EnvList::Explicit => "explicit" };
        env_read(form, name, std::env::var_os(OsStr::from_bytes(name)).as_ref());
    }
}

// Files.

pub fn read_file(path: impl AsRef<Path>) -> io::Result<Vec<u8>> {
    let result = std::fs::read(&path);
    if building() {
        file_read(path.as_ref(), &result);
    }
    result
}

pub fn read_to_string(path: impl AsRef<Path>) -> io::Result<String> {
    let result = std::fs::read_to_string(&path);
    if building() {
        file_read(path.as_ref(), &result);
    }
    result
}

/// The boolean path probes of `std::path::Path`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Probe {
    Exists,
    IsFile,
    IsDir,
}

pub fn probe(kind: Probe, path: impl AsRef<Path>) -> bool {
    let path = path.as_ref();
    let observed = match kind {
        Probe::Exists => path.exists(),
        Probe::IsFile => path.is_file(),
        Probe::IsDir => path.is_dir(),
    };
    if building() {
        probed(&format!("{kind:?}"), path, json!(observed));
    }
    observed
}

pub fn metadata(path: impl AsRef<Path>) -> io::Result<Metadata> {
    let result = std::fs::metadata(&path);
    if building() {
        probed("stat", path.as_ref(), stat_json(&result));
    }
    result
}

pub fn symlink_metadata(path: impl AsRef<Path>) -> io::Result<Metadata> {
    let result = std::fs::symlink_metadata(&path);
    if building() {
        probed("lstat", path.as_ref(), stat_json(&result));
    }
    result
}

/// fstat on an output file this module opened.
pub fn file_metadata(file: &File) -> io::Result<Metadata> {
    file.metadata()
}

/// A file opened for reading. Its reads and queries go through this module,
/// so a recorder sees the bytes a caller consumes and each error; the
/// descriptor never leaves it. In build mode it keeps its path, and opening
/// records the whole file's content.
pub struct Input(File, Option<PathBuf>);

/// open(2) of PATH for reading, with FLAGS (`O_NONBLOCK`, `O_NOFOLLOW` …).
pub fn open_input(path: impl AsRef<Path>, flags: i32) -> io::Result<Input> {
    use std::os::unix::fs::OpenOptionsExt;
    let result = OpenOptions::new().read(true).custom_flags(flags).open(&path);
    if !building() {
        return result.map(|file| Input(file, None));
    }
    let path = path.as_ref();
    match &result {
        // Only a regular file's content is a fact; a FIFO's read would block.
        Ok(file) if file.metadata().is_ok_and(|m| !m.is_file()) => probed("open", path, json!("special")),
        Ok(_) => file_read(path, &std::fs::read(path)),
        Err(e) => record("file-read", json!({"path": bytes(path), "result": errno(e)})),
    }
    result.map(|file| Input(file, Some(path.to_owned())))
}

/// The kernel's random source, `/dev/urandom`.
pub fn random_source() -> io::Result<Input> {
    if building() {
        p_read(Fact::Seed, "random source".into());
    }
    File::open("/dev/urandom").map(|file| Input(file, None))
}

impl Input {
    /// fstat.
    pub fn stat(&self) -> io::Result<Metadata> {
        let result = self.0.metadata();
        if let Some(path) = &self.1 {
            probed("fstat", path, stat_json(&result));
        }
        result
    }

    /// At most LIMIT bytes from OFFSET: a seek, then reads until LIMIT or the
    /// end. An error from either discards what was read.
    pub fn read_range(&mut self, offset: u64, limit: u64) -> io::Result<Vec<u8>> {
        use std::io::{Read, Seek, SeekFrom};
        self.0.seek(SeekFrom::Start(offset))?;
        let mut bytes = Vec::new();
        (&mut self.0).take(limit).read_to_end(&mut bytes)?;
        Ok(bytes)
    }

    /// One read into BUFFER: the count, 0 at the end.
    pub fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
        io::Read::read(&mut self.0, buffer)
    }

    pub fn read_exact(&mut self, buffer: &mut [u8]) -> io::Result<()> {
        io::Read::read_exact(&mut self.0, buffer)
    }

    /// Whether the file carries an extended (POSIX or NFSv4) ACL.
    pub fn has_acl(&self) -> bool {
        use std::os::fd::AsRawFd;
        [c"system.posix_acl_access", c"system.nfs4_acl"].iter().any(|attribute| {
            let size = unsafe { libc::fgetxattr(self.0.as_raw_fd(), attribute.as_ptr(), std::ptr::null_mut(), 0) };
            size >= 0
        })
    }

    /// close(2), with its error.
    pub fn close(self) -> io::Result<()> {
        use std::os::fd::IntoRawFd;
        if unsafe { libc::close(self.0.into_raw_fd()) } < 0 {
            return Err(io::Error::last_os_error());
        }
        Ok(())
    }
}

/// A file a child process reads as its standard input: this process reads
/// none of it.
pub fn open_child_input(path: impl AsRef<Path>) -> io::Result<OwnedFd> {
    effect(|| format!("child input {}", bytes(path.as_ref())))?;
    File::open(path).map(OwnedFd::from)
}

pub fn read_link(path: impl AsRef<Path>) -> io::Result<PathBuf> {
    let result = std::fs::read_link(&path);
    if building() {
        let observed = match &result { Ok(target) => json!(bytes(target)), Err(e) => errno(e) };
        probed("readlink", path.as_ref(), observed);
    }
    result
}

/// The entries of PATH, as the host yields them. In build mode the listing
/// records the names in enumeration order and how it ended (§6.3 DirList).
pub fn read_dir(path: impl AsRef<Path>) -> io::Result<ReadDir> {
    let result = std::fs::read_dir(&path);
    if !building() {
        return result.map(|entries| ReadDir(entries, None));
    }
    let path = bytes(path.as_ref());
    match result {
        Ok(entries) => Ok(ReadDir(entries, Some(Listing { path, names: Vec::new(), end: None }))),
        Err(e) => {
            record("dir-list", json!({"path": path, "open": errno(&e)}));
            Err(e)
        }
    }
}

/// A directory listing whose entries answer their queries here.
pub struct ReadDir(std::fs::ReadDir, Option<Listing>);

/// A listing as far as it was read: `end` is None while it may continue.
struct Listing {
    path: String,
    names: Vec<String>,
    end: Option<Json>,
}

impl Iterator for ReadDir {
    type Item = io::Result<DirEntry>;
    fn next(&mut self) -> Option<Self::Item> {
        let next = self.0.next();
        if let Some(listing) = self.1.as_mut().filter(|l| l.end.is_none()) {
            match &next {
                Some(Ok(entry)) => listing.names.push(String::from_utf8_lossy(entry.file_name().as_bytes()).into_owned()),
                Some(Err(e)) => listing.end = Some(errno(e)),
                None => listing.end = Some(json!("end")),
            }
        }
        next.map(|entry| entry.map(|entry| DirEntry(entry, self.1.is_some())))
    }
}

impl Drop for ReadDir {
    fn drop(&mut self) {
        if let Some(listing) = self.1.take() {
            let terminal = listing.end.unwrap_or_else(|| json!("stopped"));
            record("dir-list", json!({"path": listing.path, "open": "ok", "names": listing.names, "terminal": terminal}));
        }
    }
}

/// An entry of a listing; it records its type query when its listing does.
pub struct DirEntry(std::fs::DirEntry, bool);

impl DirEntry {
    pub fn file_name(&self) -> OsString {
        self.0.file_name()
    }

    pub fn path(&self) -> PathBuf {
        self.0.path()
    }

    /// The type the listing gave, else an lstat's.
    pub fn file_type(&self) -> io::Result<FileType> {
        let result = self.0.file_type();
        if self.1 {
            let observed = match &result {
                Ok(t) => json!(if t.is_dir() { "dir" } else if t.is_file() { "file" } else if t.is_symlink() { "symlink" } else { "other" }),
                Err(e) => errno(e),
            };
            probed("entry-type", &self.0.path(), observed);
        }
        result
    }
}

pub fn canonicalize(path: impl AsRef<Path>) -> io::Result<PathBuf> {
    let result = std::fs::canonicalize(&path);
    if building() {
        let observed = match &result { Ok(p) => json!(bytes(p)), Err(e) => errno(e) };
        record("canon", json!({"path": bytes(path.as_ref()), "result": observed}));
    }
    result
}

/// PATH made absolute against the working directory, lexically. A relative
/// PATH reads the working directory, a P fact.
pub fn absolute(path: impl AsRef<Path>) -> io::Result<PathBuf> {
    if building() && path.as_ref().is_relative() {
        p_read(Fact::CurrentDir, format!("working directory, for {}", bytes(path.as_ref())));
    }
    std::path::absolute(path)
}

/// Whose ids an access check uses.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Ids {
    Effective,
    Real,
}

/// faccessat: whether IDS may access PATH in MODE.
pub fn access(path: &CStr, mode: i32, ids: Ids) -> io::Result<()> {
    let flags = match ids { Ids::Effective => libc::AT_EACCESS, Ids::Real => 0 };
    let result = status(unsafe { libc::faccessat(libc::AT_FDCWD, path.as_ptr(), mode, flags) });
    if building() {
        let observed = match &result { Ok(()) => json!("ok"), Err(e) => errno(e) };
        probed(&format!("access {mode} {ids:?}"), Path::new(OsStr::from_bytes(path.to_bytes())), observed);
    }
    result
}

/// A C call's 0-or-error result.
fn status(result: libc::c_int) -> io::Result<()> {
    if result == 0 { Ok(()) } else { Err(io::Error::last_os_error()) }
}

pub fn statvfs(path: &CStr) -> io::Result<libc::statvfs> {
    let mut usage = std::mem::MaybeUninit::<libc::statvfs>::uninit();
    let result = match unsafe { libc::statvfs(path.as_ptr(), usage.as_mut_ptr()) } {
        0 => Ok(unsafe { usage.assume_init() }),
        _ => Err(io::Error::last_os_error()),
    };
    if building() {
        let observed = match &result {
            Ok(u) => json!([u.f_bsize, u.f_blocks, u.f_bfree, u.f_bavail]),
            Err(e) => errno(e),
        };
        probed("statvfs", Path::new(OsStr::from_bytes(path.to_bytes())), observed);
    }
    result
}

// Process facts.

pub fn current_dir() -> io::Result<PathBuf> {
    if building() {
        p_read(Fact::CurrentDir, "working directory".into());
    }
    std::env::current_dir()
}

/// A machine or user fact, recorded with its value.
fn host_fact(fact: &str, value: Json) {
    record("host", json!({"fact": fact, "value": value}));
}

pub fn current_exe() -> io::Result<PathBuf> {
    let result = std::env::current_exe();
    if building() {
        host_fact("exe", match &result { Ok(p) => json!(bytes(p)), Err(e) => errno(e) });
    }
    result
}

pub fn pid() -> u32 {
    if building() {
        p_read(Fact::Pid, "pid".into());
        return with_build(|b| b.pinned.pid).unwrap_or_default();
    }
    std::process::id()
}

pub fn uid() -> u32 {
    let uid = unsafe { libc::getuid() };
    if building() {
        host_fact("uid", json!(uid));
    }
    uid
}

pub fn euid() -> u32 {
    let euid = unsafe { libc::geteuid() };
    if building() {
        host_fact("euid", json!(euid));
    }
    euid
}

// User database.

/// A passwd entry's fields, as the C library's bytes.
pub struct Passwd {
    /// pw_name, absent when the entry has none.
    pub name: Option<Vec<u8>>,
    pub gecos: Vec<u8>,
    /// pw_dir, absent when the entry has none.
    pub dir: Option<Vec<u8>>,
}

/// The C library's lookup into a BUFFER-byte buffer (so NSS sources count):
/// the entry, or the status, 0 when there is none.
fn passwd(buffer: usize, lookup: impl Fn(*mut libc::passwd, *mut libc::c_char, usize, *mut *mut libc::passwd) -> libc::c_int) -> Result<Passwd, i32> {
    let mut buffer = vec![0 as libc::c_char; buffer];
    let mut entry: libc::passwd = unsafe { std::mem::zeroed() };
    let mut found: *mut libc::passwd = std::ptr::null_mut();
    let status = lookup(&mut entry, buffer.as_mut_ptr(), buffer.len(), &mut found);
    if status != 0 || found.is_null() {
        return Err(status);
    }
    let bytes = |p: *const libc::c_char| if p.is_null() { Vec::new() } else { unsafe { CStr::from_ptr(p) }.to_bytes().to_vec() };
    let field = |p: *const libc::c_char| (!p.is_null()).then(|| bytes(p));
    Ok(Passwd { name: field(entry.pw_name), gecos: bytes(entry.pw_gecos), dir: field(entry.pw_dir) })
}

pub fn user_by_uid(uid: u32, buffer: usize) -> Result<Passwd, i32> {
    let result = passwd(buffer, |entry, buffer, size, found| unsafe { libc::getpwuid_r(uid, entry, buffer, size, found) });
    user_db(format!("uid {uid}"), &result);
    result
}

pub fn user_by_name(name: &CStr, buffer: usize) -> Result<Passwd, i32> {
    let result = passwd(buffer, |entry, buffer, size, found| unsafe { libc::getpwnam_r(name.as_ptr(), entry, buffer, size, found) });
    user_db(format!("name {}", String::from_utf8_lossy(name.to_bytes())), &result);
    result
}

/// A user-database answer, recorded as a digest of the fields read.
fn user_db(key: String, result: &Result<Passwd, i32>) {
    if building() {
        let result = match result {
            Ok(p) => json!(sha256(&[p.name.as_deref(), Some(&p.gecos), p.dir.as_deref()].map(|f| f.map(<[u8]>::to_vec)).iter()
                .flat_map(|f| f.as_ref().map_or(vec![0xff], |f| [f.as_slice(), &[0]].concat())).collect::<Vec<u8>>())),
            Err(status) => json!({"status": status}),
        };
        record("user-db", json!({"key": key, "result": result}));
    }
}

/// getgrgid_r into a BUFFER-byte buffer: the group's name, or the status, 0
/// when there is none.
pub fn group_by_gid(gid: u32, buffer: usize) -> Result<Vec<u8>, i32> {
    let mut buffer = vec![0 as libc::c_char; buffer];
    let mut entry = std::mem::MaybeUninit::<libc::group>::uninit();
    let mut found = std::ptr::null_mut();
    let status = unsafe { libc::getgrgid_r(gid, entry.as_mut_ptr(), buffer.as_mut_ptr(), buffer.len(), &mut found) };
    let result = if status != 0 || found.is_null() || unsafe { (*found).gr_name }.is_null() {
        Err(status)
    } else {
        Ok(unsafe { CStr::from_ptr((*found).gr_name) }.to_bytes().to_vec())
    };
    if building() {
        let observed = match &result { Ok(name) => json!(String::from_utf8_lossy(name)), Err(status) => json!({"status": status}) };
        record("user-db", json!({"key": format!("gid {gid}"), "result": observed}));
    }
    result
}

// Machine.

/// The kernel nodename, which GNU's gethostname returns, read without an
/// unsafe syscall. HOSTNAME and DNS are not consulted.
pub fn system_name() -> io::Result<String> {
    let result = std::fs::read_to_string("/proc/sys/kernel/hostname");
    if building() {
        host_fact("system-name", match &result { Ok(name) => json!(name), Err(e) => errno(e) });
    }
    result
}

/// gnulib nproc's processor sets.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Processors {
    /// NPROC_CURRENT: those this process may run on.
    Current,
    /// The online processors.
    Online,
    /// NPROC_ALL: the configured processors.
    All,
}

pub fn processors(set: Processors) -> u64 {
    let count = count_processors(set);
    if building() {
        host_fact(&format!("processors {set:?}"), json!(count));
    }
    count
}

fn count_processors(set: Processors) -> u64 {
    let name = match set {
        Processors::Current => {
            let mut cpus: libc::cpu_set_t = unsafe { std::mem::zeroed() };
            if unsafe { libc::sched_getaffinity(0, std::mem::size_of::<libc::cpu_set_t>(), &mut cpus) } == 0 {
                let count = unsafe { libc::CPU_COUNT(&cpus) };
                if count > 0 { return count as u64; }
            }
            libc::_SC_NPROCESSORS_ONLN
        }
        Processors::Online => libc::_SC_NPROCESSORS_ONLN,
        Processors::All => libc::_SC_NPROCESSORS_CONF,
    };
    u64::try_from(unsafe { libc::sysconf(name) }).ok().filter(|&n| n > 0).unwrap_or(1)
}

// Time.

/// A time-zone file's bytes: `/etc/localtime`, a TZ file or a zoneinfo entry.
pub fn zone_file(path: impl AsRef<Path>) -> io::Result<Vec<u8>> {
    read_file(path)
}

/// The wall clock; in build mode the build time.
pub fn clock() -> SystemTime {
    if building() {
        p_read(Fact::Clock, "clock".into());
        return with_build(|b| b.pinned.clock).unwrap_or(SystemTime::UNIX_EPOCH);
    }
    SystemTime::now()
}

/// The monotonic clock, for deadlines and measurement.
pub fn monotonic() -> Instant {
    if building() {
        p_read(Fact::Monotonic, "monotonic clock".into());
    }
    Instant::now()
}

// Effects.

/// Four bytes of kernel entropy; in build mode the pinned seed.
pub fn random_seed() -> io::Result<[u8; 4]> {
    if building() {
        p_read(Fact::Seed, "random seed".into());
        return Ok(with_build(|b| b.pinned.seed).unwrap_or_default());
    }
    let mut bytes = [0u8; 4];
    random_source()?.read_exact(&mut bytes)?;
    Ok(bytes)
}

/// PATH opened with OPTIONS for writing only: reads go through `Input`.
pub fn open_output(path: impl AsRef<Path>, options: &OpenOptions) -> io::Result<File> {
    effect(|| format!("write {}", bytes(path.as_ref())))?;
    options.clone().read(false).write(true).open(path)
}

/// GNU copy-file's fast paths: share SOURCE's extents with DEST, else
/// copy_file_range until it stops. False leaves the rest to read+write.
pub fn clone_file(dest: &File, source: &Input) -> bool {
    use std::os::fd::AsRawFd;
    if unsafe { libc::ioctl(dest.as_raw_fd(), libc::FICLONE, source.0.as_raw_fd()) } == 0 {
        return true;
    }
    const COPY_MAX: usize = (isize::MAX as usize) >> 30 << 30;
    while unsafe { libc::copy_file_range(source.0.as_raw_fd(), std::ptr::null_mut(), dest.as_raw_fd(), std::ptr::null_mut(), COPY_MAX, 0) } > 0 {}
    false
}

pub fn write(path: impl AsRef<Path>, bytes: impl AsRef<[u8]>) -> io::Result<()> {
    effect(|| format!("write {}", self::bytes(path.as_ref())))?;
    std::fs::write(path, bytes)
}

pub fn rename(from: impl AsRef<Path>, to: impl AsRef<Path>) -> io::Result<()> {
    effect(|| format!("rename {} {}", bytes(from.as_ref()), bytes(to.as_ref())))?;
    std::fs::rename(from, to)
}

pub fn remove_file(path: impl AsRef<Path>) -> io::Result<()> {
    effect(|| format!("delete {}", bytes(path.as_ref())))?;
    std::fs::remove_file(path)
}

pub fn remove_dir(path: impl AsRef<Path>) -> io::Result<()> {
    effect(|| format!("delete directory {}", bytes(path.as_ref())))?;
    std::fs::remove_dir(path)
}

/// mkdir with MODE before the umask.
pub fn create_dir(path: impl AsRef<Path>, mode: u32) -> io::Result<()> {
    effect(|| format!("make directory {}", bytes(path.as_ref())))?;
    use std::os::unix::fs::DirBuilderExt;
    std::fs::DirBuilder::new().mode(mode).create(path)
}

pub fn create_dir_all(path: impl AsRef<Path>) -> io::Result<()> {
    effect(|| format!("make directory {}", bytes(path.as_ref())))?;
    std::fs::create_dir_all(path)
}

pub fn hard_link(original: impl AsRef<Path>, link: impl AsRef<Path>) -> io::Result<()> {
    effect(|| format!("link {}", bytes(link.as_ref())))?;
    std::fs::hard_link(original, link)
}

pub fn symlink(target: impl AsRef<Path>, link: impl AsRef<Path>) -> io::Result<()> {
    effect(|| format!("symlink {}", bytes(link.as_ref())))?;
    std::os::unix::fs::symlink(target, link)
}

pub fn fchown(file: &File, uid: Option<u32>, gid: Option<u32>) -> io::Result<()> {
    std::os::unix::fs::fchown(file, uid, gid)
}

/// fchmodat: PATH's mode bits, a symlink's own under NOFOLLOW.
pub fn chmod(path: &CStr, mode: u32, nofollow: bool) -> io::Result<()> {
    effect(|| format!("chmod {}", String::from_utf8_lossy(path.to_bytes())))?;
    let flags = if nofollow { libc::AT_SYMLINK_NOFOLLOW } else { 0 };
    status(unsafe { libc::fchmodat(libc::AT_FDCWD, path.as_ptr(), mode as libc::mode_t, flags) })
}

/// renameat2 with RENAME_NOREPLACE: EEXIST rather than replace TO.
pub fn rename_noreplace(from: &CStr, to: &CStr) -> io::Result<()> {
    effect(|| format!("rename {}", String::from_utf8_lossy(to.to_bytes())))?;
    status(unsafe { libc::renameat2(libc::AT_FDCWD, from.as_ptr(), libc::AT_FDCWD, to.as_ptr(), libc::RENAME_NOREPLACE) })
}

/// utimensat: both of PATH's times set to TIME (seconds, nanoseconds), or
/// now; a symlink's own under NOFOLLOW.
pub fn set_times(path: &CStr, time: Option<(i64, i64)>, nofollow: bool) -> io::Result<()> {
    effect(|| format!("set times {}", String::from_utf8_lossy(path.to_bytes())))?;
    let stamp = match time {
        Some((seconds, nanos)) => libc::timespec { tv_sec: seconds as libc::time_t, tv_nsec: nanos as _ },
        None => libc::timespec { tv_sec: 0, tv_nsec: libc::UTIME_NOW },
    };
    let flags = if nofollow { libc::AT_SYMLINK_NOFOLLOW } else { 0 };
    status(unsafe { libc::utimensat(libc::AT_FDCWD, path.as_ptr(), [stamp, stamp].as_ptr(), flags) })
}

/// mkostemp: a new close-on-exec file named TEMPLATE with its trailing
/// XXXXXX replaced, and that name.
pub fn temp_file(template: &Path) -> io::Result<(PathBuf, File)> {
    effect(|| format!("temporary file {}", bytes(template)))?;
    use std::os::unix::ffi::{OsStrExt, OsStringExt};
    let mut name = template.as_os_str().as_bytes().to_vec();
    name.push(0);
    let fd = unsafe { libc::mkostemp(name.as_mut_ptr().cast(), libc::O_CLOEXEC) };
    if fd < 0 {
        return Err(io::Error::last_os_error());
    }
    let file = unsafe { File::from_raw_fd(fd) };
    name.pop();
    Ok((PathBuf::from(OsString::from_vec(name)), file))
}

// Processes. A child, and every descriptor this process holds besides its
// standard streams, comes from a function here: the descriptor and child
// calls elsewhere (read, write, poll, fcntl, ioctl, the termios calls, fsync,
// close, waitpid, shutdown, accept) act only on what these returned.

/// Every child process starts from a Command made here.
pub fn command(program: impl AsRef<OsStr>) -> io::Result<Command> {
    effect(|| format!("run {}", String::from_utf8_lossy(program.as_ref().as_bytes())))?;
    Ok(Command::new(program))
}

/// kill(2): SIGNAL to PID, a process group when negative. Signal 0 asks
/// only whether PID exists and may be signalled.
pub fn kill(pid: i32, signal: i32) -> io::Result<()> {
    effect(|| format!("signal {signal} to {pid}"))?;
    status(unsafe { libc::kill(pid, signal) })
}

/// A pidfd for PID, readable once it exits.
pub fn pidfd_open(pid: i32) -> Option<OwnedFd> {
    effect(|| format!("pidfd {pid}")).ok()?;
    let fd = unsafe { libc::syscall(libc::SYS_pidfd_open, pid, 0) };
    (fd >= 0).then(|| unsafe { OwnedFd::from_raw_fd(fd as libc::c_int) })
}

/// emacs_pipe: a close-on-exec pipe (read end, write end).
pub fn pipe() -> io::Result<(OwnedFd, OwnedFd)> {
    effect(|| "pipe".into())?;
    let mut fds = [0; 2];
    status(unsafe { libc::pipe2(fds.as_mut_ptr(), libc::O_CLOEXEC) })?;
    Ok(unsafe { (OwnedFd::from_raw_fd(fds[0]), OwnedFd::from_raw_fd(fds[1])) })
}

/// A close-on-exec, nonblocking eventfd: a wakeup counter no file names.
pub fn eventfd() -> io::Result<OwnedFd> {
    effect(|| "eventfd".into())?;
    let fd = unsafe { libc::eventfd(0, libc::EFD_CLOEXEC | libc::EFD_NONBLOCK) };
    if fd < 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(unsafe { OwnedFd::from_raw_fd(fd) })
}

/// allocate_pty with posix_openpt: the master and the slave's name.
pub fn allocate_pty() -> io::Result<(OwnedFd, Vec<u8>)> {
    effect(|| "pty".into())?;
    let fd = unsafe { libc::posix_openpt(libc::O_RDWR | libc::O_NOCTTY | libc::O_CLOEXEC) };
    if fd < 0 {
        return Err(io::Error::last_os_error());
    }
    let master = unsafe { OwnedFd::from_raw_fd(fd) };
    if unsafe { libc::grantpt(fd) } != 0 || unsafe { libc::unlockpt(fd) } != 0 {
        return Err(io::Error::last_os_error());
    }
    let mut name = [0u8; 64];
    if unsafe { libc::ptsname_r(fd, name.as_mut_ptr().cast(), name.len()) } != 0 {
        return Err(io::Error::last_os_error());
    }
    let len = name.iter().position(|&b| b == 0).unwrap_or(name.len());
    Ok((master, name[..len].to_vec()))
}

/// The pty slave NAME, opened without becoming the controlling terminal.
pub fn open_tty(name: &[u8]) -> io::Result<OwnedFd> {
    effect(|| format!("terminal {}", String::from_utf8_lossy(name)))?;
    let path = std::ffi::CString::new(name).map_err(|_| io::Error::from(io::ErrorKind::InvalidInput))?;
    let fd = unsafe { libc::open(path.as_ptr(), libc::O_RDWR | libc::O_NOCTTY | libc::O_CLOEXEC) };
    if fd < 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(unsafe { OwnedFd::from_raw_fd(fd) })
}

// Sockets.

/// HOST:PORT's addresses, from the C library's resolver.
pub fn resolve(host: &str, port: u16) -> io::Result<Vec<SocketAddr>> {
    effect(|| format!("resolve {host}:{port}"))?;
    use std::net::ToSocketAddrs;
    (host, port).to_socket_addrs().map(Iterator::collect)
}

pub fn listen(addr: SocketAddr) -> io::Result<TcpListener> {
    effect(|| format!("listen {addr}"))?;
    TcpListener::bind(addr)
}

pub fn connect(addr: SocketAddr) -> io::Result<TcpStream> {
    effect(|| format!("connect {addr}"))?;
    TcpStream::connect(addr)
}

/// connect_network_socket's nonblocking connect: a close-on-exec stream
/// socket to ADDR, connected or (EINPROGRESS) connecting.
pub fn connect_nonblocking(addr: SocketAddr) -> io::Result<OwnedFd> {
    effect(|| format!("connect {addr}"))?;
    let mut storage: libc::sockaddr_storage = unsafe { std::mem::zeroed() };
    let len = match addr {
        SocketAddr::V4(a) => {
            let sin = unsafe { &mut *(&mut storage as *mut libc::sockaddr_storage).cast::<libc::sockaddr_in>() };
            sin.sin_family = libc::AF_INET as libc::sa_family_t;
            sin.sin_port = a.port().to_be();
            sin.sin_addr = libc::in_addr { s_addr: u32::from_ne_bytes(a.ip().octets()) };
            size_of::<libc::sockaddr_in>()
        }
        SocketAddr::V6(a) => {
            let sin6 = unsafe { &mut *(&mut storage as *mut libc::sockaddr_storage).cast::<libc::sockaddr_in6>() };
            sin6.sin6_family = libc::AF_INET6 as libc::sa_family_t;
            sin6.sin6_port = a.port().to_be();
            sin6.sin6_flowinfo = a.flowinfo();
            sin6.sin6_addr = libc::in6_addr { s6_addr: a.ip().octets() };
            sin6.sin6_scope_id = a.scope_id();
            size_of::<libc::sockaddr_in6>()
        }
    };
    let s = unsafe { libc::socket(i32::from(storage.ss_family), libc::SOCK_STREAM | libc::SOCK_NONBLOCK | libc::SOCK_CLOEXEC, 0) };
    if s < 0 {
        return Err(io::Error::last_os_error());
    }
    let socket = unsafe { OwnedFd::from_raw_fd(s) };
    if unsafe { libc::connect(s, (&storage as *const libc::sockaddr_storage).cast(), len as libc::socklen_t) } != 0 {
        let error = io::Error::last_os_error();
        // EINTR leaves a nonblocking connect in progress, as EINPROGRESS does.
        if !matches!(error.raw_os_error(), Some(libc::EINPROGRESS | libc::EINTR)) {
            return Err(error);
        }
    }
    Ok(socket)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn passwd_keeps_absent_fields_absent() {
        let gecos = c"Full Name";
        let entry = passwd(16, |entry, _, _, found| {
            unsafe {
                (*entry).pw_name = std::ptr::null_mut();
                (*entry).pw_gecos = gecos.as_ptr().cast_mut();
                (*entry).pw_dir = std::ptr::null_mut();
                *found = entry;
            }
            0
        }).ok().unwrap();
        assert_eq!((entry.name, entry.gecos, entry.dir), (None, b"Full Name".to_vec(), None));
    }
}
