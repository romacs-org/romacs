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
    static CONFINED: Cell<bool> = const { Cell::new(false) };
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
/// restore re-seeds (`romacs--world-reseed`): (file suffix, form head, the P
/// fact or fenced variable the form may read, the variable it sets, which is
/// fenced once the form ends).
const RESEED: [(&str, &str, &str, Option<&str>); 4] = [
    ("/emacs-lisp/cl-extra.el", "(defvar cl--random-state", "clock", Some("cl--random-state")),
    ("/progmodes/project.el", "(defcustom project-list-file", "default-directory", Some("project-list-file")),
    ("/emacs-lisp/find-func.el", "(defvar find-function-C-source-directory", "default-directory",
     Some("find-function-C-source-directory")),
    // It asks only whether exec-path is bound: its standard value is the
    // expression cus-start gives, so nothing it sets needs re-seeding.
    ("/cus-start.el", "(let* ((quoter", "exec-path", None),
];

/// The re-seed entry of the form being loaded, if it is one.
fn reseed_entry() -> Option<(&'static str, Option<&'static str>)> {
    with_build(|b| b.site.as_deref().and_then(|site| RESEED.iter().find(|r| site.at((r.0, r.1)))).map(|r| (r.2, r.3)))
        .flatten()
}

/// Whether the form being loaded re-seeds and may read WHAT: a P fact's
/// name or a fenced variable's.
pub(crate) fn reseed_reads(what: &str) -> bool {
    let allowed = reseed_entry().is_some_and(|(reads, _)| reads == what);
    if allowed {
        record("reseed", json!({"reads": what}));
    }
    allowed
}

/// The variable the form being loaded re-seeds, if it is a re-seed entry.
pub(crate) fn reseed_output() -> Option<&'static str> {
    reseed_entry().and_then(|(_, output)| output)
}

/// The sites that may switch the startup phase, in order: the chain's batch
/// steps, then an interactive session's, which supersede them.
const STARTUP: [&str; 2] = ["/rlisp/romacs-loadup-bootstrap.el", "/rlisp/romacs-gui-loadup.el"];

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

/// How many startup runs a build has begun.
pub(crate) fn startups() -> usize {
    with_build(|b| b.startups).unwrap_or(0)
}

/// Whether the startup function is running in a build.
pub(crate) fn in_startup() -> bool {
    with_build(|b| b.phase == Phase::Startup).unwrap_or(false)
}

/// `romacs--startup-phase`: enter or leave startup. The bootstrap's single
/// run may switch, then once an interactive session's file; any other
/// switch refuses and changes nothing. True when the phase changed.
pub(crate) fn switch_startup(on: bool) -> bool {
    with_build(|b| {
        let from = match on { true => Phase::Recipe, false => Phase::Startup };
        let run = if on { b.startups } else { b.startups.saturating_sub(1) };
        let allowed = b.phase == from
            && b.site.as_ref().is_some_and(|s| STARTUP.get(run).is_some_and(|file| s.file.ends_with(file)));
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

/// The top-level form at LINE of FILE, whose first line is HEAD, is the site
/// of what follows until the guard drops: for an embedder that evaluates a
/// file's forms itself.
pub fn form_site(file: &Path, line: usize, head: &str) -> SiteGuard {
    enter_site(|| Site { file: file.to_string_lossy().into_owned(), line, head: head.chars().take(80).collect() })
}

/// Make the form SITE describes the site of what follows, until the guard
/// drops. SITE is built only in build mode.
pub(crate) fn enter_site(site: impl FnOnce() -> Site) -> SiteGuard {
    if !building() {
        return SiteGuard(None);
    }
    SiteGuard(with_build(|b| std::mem::replace(&mut b.site, Some(Rc::new(site())))))
}

/// This thread is confined (`world::sandbox`): a read it is refused fails
/// with EACCES, which the build lists as a refusal.
pub(crate) fn note_confined() {
    CONFINED.with(|c| c.set(true));
}

/// A read of PATH that failed with ERROR, listed when confinement refused it.
fn confined(path: &Path, error: &io::Error) {
    if error.raw_os_error() == Some(libc::EACCES) && CONFINED.with(Cell::get) {
        refuse("confined", json!({"path": path_json(path)}));
    }
}

/// Whether a query of PATH is keyed. A relative PATH resolves against the
/// working directory, so its query is a P read instead.
fn keyed_path(path: &Path, kind: &str) -> bool {
    if path.is_relative() {
        p_read(Fact::CurrentDir, format!("working directory, for {kind} {}", bytes(path)));
        return false;
    }
    true
}

/// Record a K fact: KIND with FIELDS and the current site.
pub(crate) fn record(kind: &str, fields: Json) {
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
            (Phase::Recipe, _) if site.is_some_and(|s| RESEED.iter().any(|r| s.at((r.0, r.1)) && what == r.2)) => Some("reseed"),
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

/// PATH as display text, for refusal messages.
fn bytes(path: &Path) -> String {
    String::from_utf8_lossy(path.as_os_str().as_bytes()).into_owned()
}

/// DATA as a record holds it: a string when it is UTF-8, else
/// `{"hex": ...}`, so that distinct host byte strings stay distinct.
pub fn encoded(data: &[u8]) -> Json {
    match std::str::from_utf8(data) {
        Ok(text) => json!(text),
        Err(_) => json!({"hex": data.iter().map(|b| format!("{b:02x}")).collect::<String>()}),
    }
}

fn path_json(path: &Path) -> Json {
    encoded(path.as_os_str().as_bytes())
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
        Err(e) => {
            confined(path, e);
            errno(e)
        }
    };
    if keyed_path(path, "read") {
        record("file-read", json!({"path": path_json(path), "result": result}));
    }
}

fn probed(kind: &str, path: &Path, result: Json) {
    if keyed_path(path, kind) {
        record("probe", json!({"probe": kind, "path": path_json(path), "result": result}));
    }
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
        _ => record("env", json!({"form": form, "name": encoded(name), "value": value.map(|v| encoded(v.as_bytes()))})),
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
/// a regular file reads its whole content from the descriptor once: that is
/// what the trace records and what every later read is served from.
pub struct Input(File, Option<PathBuf>, Option<Captured>);

/// A regular file's content as opening read it, or the errno that read
/// failed with, and the read position.
struct Captured {
    content: Result<Vec<u8>, i32>,
    at: usize,
}

impl Captured {
    /// At most LIMIT bytes from AT, moving AT past them.
    fn take(&mut self, limit: usize) -> io::Result<&[u8]> {
        let content = self.content.as_deref().map_err(|&errno| io::Error::from_raw_os_error(errno))?;
        let from = self.at.min(content.len());
        self.at = from + limit.min(content.len() - from);
        Ok(&content[from..self.at])
    }
}

/// open(2) of PATH for reading, with FLAGS (`O_NONBLOCK`, `O_NOFOLLOW` …).
pub fn open_input(path: impl AsRef<Path>, flags: i32) -> io::Result<Input> {
    use std::os::unix::fs::OpenOptionsExt;
    let result = OpenOptions::new().read(true).custom_flags(flags).open(&path);
    if !building() {
        return result.map(|file| Input(file, None, None));
    }
    let path = path.as_ref();
    match result {
        // Only a regular file's content is a fact; a FIFO's read would block.
        Ok(file) if file.metadata().is_ok_and(|m| !m.is_file()) => {
            probed("open", path, json!("special"));
            Ok(Input(file, Some(path.to_owned()), None))
        }
        Ok(file) => {
            let mut content = Vec::new();
            let read = io::Read::read_to_end(&mut &file, &mut content).map(|_| content);
            file_read(path, &read);
            let content = read.map_err(|e| e.raw_os_error().unwrap_or(libc::EIO));
            Ok(Input(file, Some(path.to_owned()), Some(Captured { content, at: 0 })))
        }
        Err(e) => {
            confined(path, &e);
            if keyed_path(path, "read") {
                record("file-read", json!({"path": path_json(path), "result": errno(&e)}));
            }
            Err(e)
        }
    }
}

/// The kernel's random source, `/dev/urandom`.
pub fn random_source() -> io::Result<Input> {
    if building() {
        p_read(Fact::Seed, "random source".into());
    }
    File::open("/dev/urandom").map(|file| Input(file, None, None))
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
        if let Some(captured) = &mut self.2 {
            captured.at = usize::try_from(offset).unwrap_or(usize::MAX);
            return captured.take(usize::try_from(limit).unwrap_or(usize::MAX)).map(<[u8]>::to_vec);
        }
        self.0.seek(SeekFrom::Start(offset))?;
        let mut bytes = Vec::new();
        (&mut self.0).take(limit).read_to_end(&mut bytes)?;
        Ok(bytes)
    }

    /// One read into BUFFER: the count, 0 at the end.
    pub fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
        let Some(captured) = &mut self.2 else { return io::Read::read(&mut self.0, buffer) };
        let bytes = captured.take(buffer.len())?;
        buffer[..bytes.len()].copy_from_slice(bytes);
        Ok(bytes.len())
    }

    pub fn read_exact(&mut self, buffer: &mut [u8]) -> io::Result<()> {
        let Some(captured) = &mut self.2 else { return io::Read::read_exact(&mut self.0, buffer) };
        let bytes = captured.take(buffer.len())?;
        if bytes.len() < buffer.len() {
            return Err(io::ErrorKind::UnexpectedEof.into());
        }
        buffer.copy_from_slice(bytes);
        Ok(())
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
        let observed = match &result { Ok(target) => path_json(target), Err(e) => errno(e) };
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
    let path = path.as_ref();
    match result {
        Ok(entries) if keyed_path(path, "listing") => Ok(ReadDir(entries, Some(Listing { path: path_json(path), names: Vec::new(), end: None }))),
        Ok(entries) => Ok(ReadDir(entries, None)),
        Err(e) => {
            confined(path, &e);
            if keyed_path(path, "listing") {
                record("dir-list", json!({"path": path_json(path), "open": errno(&e)}));
            }
            Err(e)
        }
    }
}

/// A directory listing whose entries answer their queries here.
pub struct ReadDir(std::fs::ReadDir, Option<Listing>);

/// A listing as far as it was read: `end` is None while it may continue.
struct Listing {
    path: Json,
    names: Vec<Json>,
    end: Option<Json>,
}

impl Iterator for ReadDir {
    type Item = io::Result<DirEntry>;
    fn next(&mut self) -> Option<Self::Item> {
        let next = self.0.next();
        if let Some(listing) = self.1.as_mut().filter(|l| l.end.is_none()) {
            match &next {
                Some(Ok(entry)) => listing.names.push(encoded(entry.file_name().as_bytes())),
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
        let observed = match &result { Ok(p) => path_json(p), Err(e) => errno(e) };
        if keyed_path(path.as_ref(), "canonicalize") {
            record("canon", json!({"path": path_json(path.as_ref()), "result": observed}));
        }
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

/// Whether the filesystem holding PATH folds case, as CLASSIFY observes it:
/// one keyed fact. The listings and stats that answer it are not recorded,
/// since the build consumed only the answer, and restore asks again.
pub fn case_insensitive(path: &Path, classify: fn(&Path) -> Result<bool, &'static str>) -> Result<bool, &'static str> {
    if !building() {
        return classify(path);
    }
    MODE.with(|m| m.set(Mode::Off));
    let result = classify(path);
    MODE.with(|m| m.set(Mode::Build));
    // Confinement may hide what answers the probe: a build cannot know its
    // answer is the host's.
    if result.is_err() && CONFINED.with(Cell::get) {
        refuse("confined", json!({"case probe": path_json(path), "error": result.err()}));
        return result;
    }
    let observed = match result { Ok(answer) => json!(answer), Err(reason) => json!({"error": reason}) };
    if keyed_path(path, "case probe") {
        record("case", json!({"path": path_json(path), "result": observed}));
    }
    result
}

// Replay.

/// The bytes a record's `encoded` field holds.
fn decoded(field: &Json) -> Option<Vec<u8>> {
    if let Some(text) = field.as_str() {
        return Some(text.as_bytes().to_vec());
    }
    let hex = field["hex"].as_str()?;
    (0..hex.len()).step_by(2).map(|i| u8::from_str_radix(hex.get(i..i + 2)?, 16).ok()).collect()
}

/// The record a keyed RECORD's query makes now, its site aside: the host's
/// present answer to the same question, asked the same way (§6.4). None for
/// a record that asks the host nothing. CASE answers a case probe.
pub fn replay(record: &Json, case: fn(&Path) -> Result<bool, &'static str>) -> Option<Json> {
    let kind = record["kind"].as_str()?;
    let path = || decoded(&record["path"]).map(|bytes| PathBuf::from(OsString::from_vec(bytes)));
    use std::os::unix::ffi::OsStringExt;
    let saved = (mode(), BUILD.with(|b| b.borrow_mut().take()));
    enter_build(Pinned { clock: SystemTime::UNIX_EPOCH, pid: 0, seed: [0; 4] });
    begin_recipe();
    let asked = (|| -> Option<()> {
        match kind {
            "file-read" => { let _ = read_file(path()?); }
            "probe" => match record["probe"].as_str()? {
                "Exists" => { probe(Probe::Exists, path()?); }
                "IsFile" => { probe(Probe::IsFile, path()?); }
                "IsDir" => { probe(Probe::IsDir, path()?); }
                "stat" => { let _ = metadata(path()?); }
                "lstat" => { let _ = symlink_metadata(path()?); }
                "fstat" | "open" => { if let Ok(input) = open_input(path()?, 0) { let _ = input.stat(); } }
                "readlink" => { let _ = read_link(path()?); }
                "entry-type" => {
                    let path = path()?;
                    let observed = match std::fs::symlink_metadata(&path) {
                        Ok(m) => json!(if m.is_symlink() { "symlink" } else if m.is_dir() { "dir" } else if m.is_file() { "file" } else { "other" }),
                        Err(e) => errno(&e),
                    };
                    probed("entry-type", &path, observed);
                }
                "statvfs" => { let _ = statvfs(&std::ffi::CString::new(path()?.into_os_string().into_vec()).ok()?); }
                access => {
                    let mut words = access.strip_prefix("access ")?.split(' ');
                    let mode = words.next()?.parse().ok()?;
                    let ids = match words.next()? { "Effective" => Ids::Effective, "Real" => Ids::Real, _ => return None };
                    let _ = self::access(&std::ffi::CString::new(path()?.into_os_string().into_vec()).ok()?, mode, ids);
                }
            },
            "dir-list" => {
                if let Ok(entries) = read_dir(path()?) {
                    let read = if record["terminal"] == "stopped" { record["names"].as_array()?.len() } else { usize::MAX };
                    entries.take(read).for_each(drop);
                }
            }
            "canon" => { let _ = canonicalize(path()?); }
            "env" => {
                let name = decoded(&record["name"])?;
                match record["form"].as_str()? {
                    "host" => { getenv(std::str::from_utf8(&name).ok()?); }
                    "process" => lisp_env(EnvList::Process, &name),
                    "initial" => lisp_env(EnvList::Initial, &name),
                    "explicit" => lisp_env(EnvList::Explicit, &name),
                    _ => return None,
                }
            }
            "host" => match record["fact"].as_str()? {
                "exe" => { let _ = current_exe(); }
                "uid" => { uid(); }
                "euid" => { euid(); }
                "system-name" => { let _ = system_name(); }
                "processors Current" => { processors(Processors::Current); }
                "processors Online" => { processors(Processors::Online); }
                "processors All" => { processors(Processors::All); }
                _ => return None,
            },
            "user-db" => {
                let key = &record["key"];
                if let Some(uid) = key["uid"].as_u64() {
                    let _ = user_by_uid(uid as u32, 1 << 16);
                } else if let Some(gid) = key["gid"].as_u64() {
                    let _ = group_by_gid(gid as u32, 1 << 16);
                } else {
                    let _ = user_by_name(&std::ffi::CString::new(decoded(&key["name"])?).ok()?, 1 << 16);
                }
            }
            "case" => { let _ = case_insensitive(&path()?, case); }
            _ => return None,
        }
        Some(())
    })();
    let trace = leave_build();
    MODE.with(|m| m.set(saved.0));
    BUILD.with(|b| *b.borrow_mut() = saved.1);
    asked?;
    let mut answer = trace?["records"].as_array()?.iter()
        .find(|r| r["kind"] == record["kind"] && r["probe"] == record["probe"] && r["fact"] == record["fact"])?.clone();
    answer.as_object_mut()?.remove("site");
    Some(answer)
}

/// Whether RECORD is a keyed query restore must ask again: the other kinds
/// say what the build did, not what the host answered.
pub fn keyed(record: &Json) -> bool {
    matches!(record["kind"].as_str(), Some("file-read" | "probe" | "dir-list" | "canon" | "env" | "host" | "user-db" | "case"))
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
        host_fact("exe", match &result { Ok(p) => path_json(p), Err(e) => errno(e) });
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
    user_db(json!({"uid": uid}), &result);
    result
}

pub fn user_by_name(name: &CStr, buffer: usize) -> Result<Passwd, i32> {
    let result = passwd(buffer, |entry, buffer, size, found| unsafe { libc::getpwnam_r(name.as_ptr(), entry, buffer, size, found) });
    user_db(json!({"name": encoded(name.to_bytes())}), &result);
    result
}

/// A user-database answer, recorded as a digest of the fields read.
fn user_db(key: Json, result: &Result<Passwd, i32>) {
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
        let observed = match &result { Ok(name) => encoded(name), Err(status) => json!({"status": status}) };
        record("user-db", json!({"key": {"gid": gid}, "result": observed}));
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

/// A file of a world store, opened for reading: the store's own, never a
/// fact of a build.
pub fn open_world_file(path: impl AsRef<Path>) -> io::Result<File> {
    File::open(path)
}

/// How `open_store_file` opens a world store's file.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StoreOpen {
    /// To read it.
    Read,
    /// To write it: made if missing (CREATE, readable and writable by its
    /// owner only), emptied with TRUNCATE.
    Write { create: bool, truncate: bool },
    /// To make it where none is (O_CREAT|O_EXCL), and write it.
    New,
}

/// A world store's file at PATH, opened as HOW: never through a symlink,
/// never blocking on what is no file (a FIFO opens at once, and is then
/// refused), and only when it is a regular file; truncated only then. The
/// store's own, never a fact of a build.
pub fn open_store_file(path: impl AsRef<Path>, how: StoreOpen) -> io::Result<File> {
    use std::os::unix::fs::OpenOptionsExt;
    let mut options = OpenOptions::new();
    options.custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK).mode(0o600);
    match how {
        StoreOpen::Read => { options.read(true); }
        StoreOpen::Write { create, .. } => { options.write(true).create(create); }
        StoreOpen::New => { options.write(true).create_new(true); }
    }
    let file = options.open(path.as_ref())?;
    regular_and_blocking(file, path.as_ref().display(), how)
}

/// A world store's directory at PATH, opened, and with CREATE made with
/// every missing directory on the way, each 0700 whatever the umask. The
/// walk starts at the deepest of ROOTS (the user's own directories) that
/// holds PATH, or else at `/`, and opens one component at a time relative
/// to the last (openat, O_DIRECTORY|O_NOFOLLOW), making a missing one
/// there (mkdirat, then fchmod): nothing is resolved again by path, and no
/// symlink is followed. Each directory from the root down must be owned
/// by this user or root, and writable by no one else unless sticky, or
/// another user could swap what lies below it.
pub fn open_store_dir(path: &Path, roots: &[PathBuf], create: bool) -> io::Result<std::os::fd::OwnedFd> {
    use std::os::fd::{AsRawFd, FromRawFd, OwnedFd};
    use std::os::unix::ffi::OsStrExt;
    let refuse = |why: String| io::Error::new(io::ErrorKind::PermissionDenied, why);
    if !path.is_absolute() {
        return Err(refuse(format!("{} is not absolute", path.display())));
    }
    let root = roots.iter().filter(|r| r.is_absolute() && path.starts_with(r) && r.is_dir())
        .max_by_key(|r| r.components().count()).cloned().unwrap_or_else(|| PathBuf::from("/"));
    let open = |at: Option<i32>, name: &std::ffi::CStr| -> i32 {
        let flags = libc::O_DIRECTORY | libc::O_RDONLY | libc::O_CLOEXEC;
        match at {
            Some(dir) => unsafe { libc::openat(dir, name.as_ptr(), flags | libc::O_NOFOLLOW) },
            None => unsafe { libc::open(name.as_ptr(), flags) },
        }
    };
    let root_name = std::ffi::CString::new(root.as_os_str().as_bytes()).map_err(io::Error::other)?;
    let fd = open(None, &root_name);
    if fd < 0 {
        return Err(io::Error::last_os_error());
    }
    let mut dir = unsafe { OwnedFd::from_raw_fd(fd) };
    let unshared = |dir: &OwnedFd, walked: &Path| -> io::Result<()> {
        let mut stat: libc::stat = unsafe { std::mem::zeroed() };
        if unsafe { libc::fstat(dir.as_raw_fd(), &mut stat) } != 0 {
            return Err(io::Error::last_os_error());
        }
        if stat.st_uid != euid() && stat.st_uid != 0 {
            return Err(refuse(format!("{} is owned by another user", walked.display())));
        }
        if stat.st_mode & (libc::S_IWGRP | libc::S_IWOTH) != 0 && stat.st_mode & libc::S_ISVTX == 0 {
            return Err(refuse(format!("{} is writable by others and not sticky", walked.display())));
        }
        Ok(())
    };
    unshared(&dir, &root)?;
    let mut walked = root.clone();
    for component in path.strip_prefix(&root).map_err(io::Error::other)?.components() {
        let std::path::Component::Normal(name) = component else {
            return Err(refuse(format!("{} has a component that is no name", path.display())));
        };
        walked.push(name);
        let name = std::ffi::CString::new(name.as_bytes()).map_err(io::Error::other)?;
        let mut made = false;
        let mut fd = open(Some(dir.as_raw_fd()), &name);
        if fd < 0 && create && io::Error::last_os_error().raw_os_error() == Some(libc::ENOENT) {
            effect(|| format!("make directory {}", walked.display()))?;
            made = unsafe { libc::mkdirat(dir.as_raw_fd(), name.as_ptr(), 0o700) } == 0;
            fd = open(Some(dir.as_raw_fd()), &name);
        }
        if fd < 0 {
            let error = io::Error::last_os_error();
            return Err(if error.raw_os_error() == Some(libc::ELOOP) || error.raw_os_error() == Some(libc::ENOTDIR) {
                refuse(format!("{} is no directory of its own (a symlink?)", walked.display()))
            } else { error });
        }
        let next = unsafe { OwnedFd::from_raw_fd(fd) };
        if made && unsafe { libc::fchmod(next.as_raw_fd(), 0o700) } != 0 {
            return Err(io::Error::last_os_error());
        }
        unshared(&next, &walked)?;
        dir = next;
    }
    Ok(dir)
}

/// Whether the directory DIR holds nothing.
pub fn store_dir_is_empty(dir: std::os::fd::BorrowedFd<'_>) -> io::Result<bool> {
    use std::os::fd::AsRawFd;
    let copy = unsafe { libc::fcntl(dir.as_raw_fd(), libc::F_DUPFD_CLOEXEC, 0) };
    if copy < 0 {
        return Err(io::Error::last_os_error());
    }
    let stream = unsafe { libc::fdopendir(copy) };
    if stream.is_null() {
        unsafe { libc::close(copy) };
        return Err(io::Error::last_os_error());
    }
    let mut empty = true;
    loop {
        let entry = unsafe { libc::readdir(stream) };
        if entry.is_null() {
            break;
        }
        let name = unsafe { std::ffi::CStr::from_ptr((*entry).d_name.as_ptr()) }.to_bytes();
        if name != b"." && name != b".." {
            empty = false;
            break;
        }
    }
    unsafe { libc::closedir(stream) };
    Ok(empty)
}

/// `open_store_file` for NAME in the directory DIR: opened relative to it
/// (openat), so nothing above is resolved again.
pub fn open_store_file_at(dir: std::os::fd::BorrowedFd<'_>, name: &str, how: StoreOpen) -> io::Result<File> {
    use std::os::fd::{AsRawFd, FromRawFd};
    let mut flags = libc::O_NOFOLLOW | libc::O_NONBLOCK | libc::O_CLOEXEC;
    flags |= match how {
        StoreOpen::Read => libc::O_RDONLY,
        StoreOpen::Write { create: true, .. } => libc::O_WRONLY | libc::O_CREAT,
        StoreOpen::Write { create: false, .. } => libc::O_WRONLY,
        StoreOpen::New => libc::O_WRONLY | libc::O_CREAT | libc::O_EXCL,
    };
    let cname = std::ffi::CString::new(name).map_err(io::Error::other)?;
    let fd = unsafe { libc::openat(dir.as_raw_fd(), cname.as_ptr(), flags, 0o600 as libc::c_uint) };
    if fd < 0 {
        return Err(io::Error::last_os_error());
    }
    let file = unsafe { File::from_raw_fd(fd) };
    regular_and_blocking(file, name, how)
}

/// FILE, once it is a regular file, with O_NONBLOCK cleared and, for a
/// truncating write, emptied.
fn regular_and_blocking(file: File, name: impl std::fmt::Display, how: StoreOpen) -> io::Result<File> {
    use std::os::fd::AsRawFd;
    if !file.metadata()?.file_type().is_file() {
        return Err(io::Error::new(io::ErrorKind::InvalidInput, format!("{name} is not a regular file")));
    }
    let fd = file.as_raw_fd();
    let flags = unsafe { libc::fcntl(fd, libc::F_GETFL) };
    if flags < 0 || unsafe { libc::fcntl(fd, libc::F_SETFL, flags & !libc::O_NONBLOCK) } < 0 {
        return Err(io::Error::last_os_error());
    }
    if let StoreOpen::Write { truncate: true, .. } = how {
        file.set_len(0)?;
    }
    Ok(file)
}

/// A world store's regular file at PATH, read whole (`open_store_file`).
pub fn read_store_file(path: impl AsRef<Path>) -> io::Result<String> {
    use std::io::Read;
    let mut text = String::new();
    open_store_file(path, StoreOpen::Read)?.read_to_string(&mut text)?;
    Ok(text)
}

/// Write BYTES as the world store's regular file at PATH, made or
/// emptied first (`open_store_file`).
pub fn write_store_file(path: impl AsRef<Path>, bytes: impl AsRef<[u8]>) -> io::Result<()> {
    use std::io::Write;
    open_store_file(path, StoreOpen::Write { create: true, truncate: true })?.write_all(bytes.as_ref())
}

/// Remove NAME from this process's environment, before any other thread
/// starts: world processes take their own markers out of it.
pub fn unset_env(name: &str) {
    // The world markers are read and removed while the process is single
    // threaded, at the start of main.
    unsafe { std::env::remove_var(name) };
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
