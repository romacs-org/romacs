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
use std::os::fd::{FromRawFd, OwnedFd, RawFd};
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
    /// A compiled-Lisp compile process (`crate::compiled_lisp`): record the
    /// K facts its compile-time code reads, as a build does, but refuse
    /// nothing and pin nothing. Its effects happen, as GNU's compile's do.
    Record,
}

thread_local! {
    static MODE: Cell<Mode> = const { Cell::new(Mode::Off) };
    static BUILD: RefCell<Option<Build>> = const { RefCell::new(None) };
    static LAYER: RefCell<Option<LayerState>> = const { RefCell::new(None) };
    static CONFINED: Cell<bool> = const { Cell::new(false) };
    static FLAGS_READER: RefCell<Option<std::sync::mpsc::Sender<(PathBuf, std::sync::mpsc::Sender<io::Result<u32>>)>>> =
        const { RefCell::new(None) };
}

pub fn mode() -> Mode {
    MODE.with(Cell::get)
}

fn building() -> bool {
    mode() != Mode::Off
}

/// Whether host access is refused and P facts pinned: a world build only.
fn refusing() -> bool {
    mode() == Mode::Build
}

/// Start recording a compile process's K facts on this thread
/// (`Mode::Record`).
pub fn enter_record() {
    BUILD.with(|b| *b.borrow_mut() = Some(Build {
        pinned: Pinned { clock: SystemTime::UNIX_EPOCH, pid: 0, seed: [0; 4] }, phase: Phase::Recipe, startups: 0,
        site: None, records: Vec::new(), refusals: Vec::new(), seen: Default::default(), written: Default::default(),
    }));
    MODE.with(|m| m.set(Mode::Record));
}

/// Stop recording: the keyed records and the recorder's own (`record`
/// of a kind of its caller's, such as compiled Lisp's "loaded"), in
/// first-occurrence order, without their sites.
pub fn leave_record() -> Vec<Json> {
    MODE.with(|m| m.set(Mode::Off));
    let Some(build) = BUILD.with(|b| b.borrow_mut().take()) else { return vec![] };
    let written: std::collections::HashSet<Vec<u8>> = build.written.iter().map(|p| p.as_os_str().as_bytes().to_vec()).collect();
    build.records.into_iter()
        .filter(|r| keyed(r) || r["kind"] == "loaded" || r["kind"] == "resolved")
        .filter(|r| !decoded(&r["path"]).is_some_and(|path| written.contains(&path)))
        .map(|mut r| { r.as_object_mut().map(|o| o.remove("site")); r })
        .collect()
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
    /// The processor counts: affinity, cgroups and hotplug change them
    /// between processes, so each process asks again, as GNU's
    /// `num-processors' does.
    Processors,
    /// The machine's live load: memory and load averages change at every
    /// read, so no world is keyed on them.
    Machine,
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
pub const P_ENV: [&str; 17] = [
    "PWD", "OLDPWD", "TMPDIR", "TMP", "TEMP", "PATH", "TERM", "COLORTERM", "DISPLAY",
    "WAYLAND_DISPLAY", "INSIDE_EMACS", "SHLVL", "COLUMNS", "LINES", "_",
    // `num-processors' reads these, a per-process fact (`Fact::Processors`).
    "OMP_NUM_THREADS", "OMP_THREAD_LIMIT",
];

/// The baked site, (file suffix, form head): `emacs-build-time` is the time
/// the world was built, as a dumped GNU's is its dump time.
const BAKED: (&str, &str) = ("/version.el", "(defconst emacs-build-time");
/// jit-lock.el's default for `jit-lock-stealth-load' only asks whether
/// `load-average' works, and a dumped GNU keeps the dump's answer.
const BAKED_LOAD: (&str, &str) = ("/jit-lock.el", "(defcustom jit-lock-stealth-load");

/// Load-time forms outside startup that GNU evaluates in each process and
/// restore re-seeds (`romacs--world-reseed`): (file suffix, form head, the P
/// fact or fenced variable the form may read, the variable it sets, which is
/// fenced once the form ends).
const RESEED: [(&str, &str, &str, Option<&str>); 7] = [
    ("/emacs-lisp/cl-extra.el", "(defvar cl--random-state", "clock", Some("cl--random-state")),
    ("/progmodes/project.el", "(defcustom project-list-file", "default-directory", Some("project-list-file")),
    ("/emacs-lisp/find-func.el", "(defvar find-function-C-source-directory", "default-directory",
     Some("find-function-C-source-directory")),
    // It asks only whether exec-path is bound: its standard value is the
    // expression cus-start gives, so nothing it sets needs re-seeding.
    ("/cus-start.el", "(let* ((quoter", "exec-path", None),
    // The user's persistent connection history, this process's state: the
    // read is no keyed fact, and restore reads the file again.
    ("/net/tramp-cache.el", "(when (and (stringp tramp-persistency-file-name)", "user-file", None),
    // The lock-refresh timer org-persist starts when it loads: restore
    // starts this process's own.
    ("/org/org-persist.el", "(unless (and org-persist--disable-when-emacs-Q", "clock", Some("org-persist--refresh-gc-lock-timer")),
    // tramp-set-completion-function keeps a completion entry whose file is
    // the empty string when `(file-exists-p "")' is true: the working
    // directory exists in every process, so nothing it sets needs re-seeding.
    ("/net/tramp.el", "(run-hooks 'tramp--startup-hook)", "default-directory", None),
];

/// A layer build's state while an init's libraries load (`world::layer`):
/// the definition form being loaded, when the top-level form is one, and
/// whether it read a per-process fact, which makes it a re-seed form
/// (re-evaluated at restore); and whether the startup function may run
/// again, after the libraries.
#[derive(Default)]
struct LayerState {
    form: Option<(&'static str, String)>,
    tainted: bool,
    /// An effect refused inside the definition form: the form is cut short
    /// and deferred whole to restore (`layer_form_cut`).
    cut: bool,
    /// A re-seed form that starts a timer ran (`reseed_reads`).
    timers: bool,
    rerun: bool,
    /// The entry loading is a library the init only declared
    /// (`layer::Entry::declared`): its re-seed forms may wait for their
    /// variable's first read (`romacs--world-pend`).
    declared: bool,
}

/// The definition forms a layer re-seeds: the head's kind and the name it
/// defines, for `(KIND NAME ...`.
const DEFINITIONS: [&str; 5] = ["defcustom", "defvar", "defconst", "defvar-local", "defimage"];

fn definition(head: &str) -> Option<(&'static str, String)> {
    let mut words = head.strip_prefix('(')?.split(|c: char| c.is_whitespace() || c == ')').filter(|w| !w.is_empty());
    let first = words.next()?;
    let kind = DEFINITIONS.iter().find(|k| **k == first)?;
    Some((kind, words.next()?.to_owned()))
}

/// The libraries of a layer load from here (`world::layer`).
pub(crate) fn layer_begin() {
    LAYER.with(|l| *l.borrow_mut() = Some(LayerState::default()));
}

pub(crate) fn layer_end() {
    LAYER.with(|l| *l.borrow_mut() = None);
}

pub(crate) fn layer_active() -> bool {
    LAYER.with(|l| l.borrow().is_some())
}

/// The top-level form with HEAD begins, in a layer build.
fn layer_form(head: &str) {
    LAYER.with(|l| if let Some(layer) = l.borrow_mut().as_mut() { layer.form = definition(head); layer.tainted = false; layer.cut = false; });
}

/// An effect inside a layer's definition form: not done, and the form
/// is deferred whole to restore, where it is. True when so taken. A
/// network connection is never deferred: it refuses the entry.
fn layer_cut(what: &str) -> bool {
    if ["connect ", "listen ", "resolve "].iter().any(|network| what.starts_with(network)) {
        return false;
    }
    LAYER.with(|l| match l.borrow_mut().as_mut() {
        Some(layer) if layer.form.is_some() => { layer.tainted = true; layer.cut = true; true }
        _ => false,
    }).then(|| record("layer-effect", json!({"what": what}))).is_some()
}

/// The definition form that signalled was cut short by a refused effect:
/// its name, for the loader to define it as nil and go on; the form is
/// kept for restore (`layer_form_done`).
pub(crate) fn layer_form_cut() -> Option<String> {
    LAYER.with(|l| l.borrow().as_ref().filter(|layer| layer.cut).and_then(|layer| layer.form.as_ref().map(|(_, name)| name.clone())))
}

/// A read of a per-process fact or a fenced value in a layer build: taken
/// when the form being loaded is a definition, which the layer re-seeds
/// at restore; false when it is not, and the read refuses as usual.
pub(crate) fn layer_taint() -> bool {
    LAYER.with(|l| match l.borrow_mut().as_mut() {
        Some(layer) if layer.form.is_some() => { layer.tainted = true; true }
        _ => false,
    })
}

/// Whether the top-level form being loaded is a definition of NAME: a
/// file loaded again (circular `require`s) defines its variables again.
pub(crate) fn layer_defines(name: &str) -> bool {
    LAYER.with(|l| l.borrow().as_ref().is_some_and(|layer| layer.form.as_ref().is_some_and(|(_, defined)| defined == name)))
}

/// The top-level form ends: the definition it was, when it read a
/// per-process fact (`layer_taint`), and whether an effect cut it short.
pub(crate) fn layer_form_done() -> Option<(&'static str, String, bool)> {
    LAYER.with(|l| {
        let mut layer = l.borrow_mut();
        let layer = layer.as_mut()?;
        let tainted = std::mem::take(&mut layer.tainted);
        let cut = std::mem::take(&mut layer.cut);
        layer.form.take().filter(|_| tainted).map(|(kind, name)| (kind, name, cut))
    })
}

/// The layer's next entries are libraries the init only declared (ON),
/// or ones it loaded.
pub(crate) fn layer_declared(on: bool) {
    LAYER.with(|l| if let Some(layer) = l.borrow_mut().as_mut() { layer.declared = on; });
}

/// Whether the entry loading is a declared library's.
pub(crate) fn layer_in_declared() -> bool {
    LAYER.with(|l| l.borrow().as_ref().is_some_and(|layer| layer.declared))
}

/// Whether the startup function may run again now: after a layer's
/// libraries, as a restored process runs it after them.
pub(crate) fn layer_rerun(on: bool) {
    LAYER.with(|l| if let Some(layer) = l.borrow_mut().as_mut() { layer.rerun = on; });
}

fn layer_rerun_allowed() -> bool {
    LAYER.with(|l| l.borrow().as_ref().is_some_and(|layer| layer.rerun))
}

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
        if what == "clock" {
            LAYER.with(|l| if let Some(layer) = l.borrow_mut().as_mut() { layer.timers = true; });
        }
    }
    allowed
}

/// Whether a re-seed form that reads the clock ran since the last call:
/// a timer the form left is restarted at restore, so the layer may keep it.
pub(crate) fn layer_timers_allowed() -> bool {
    LAYER.with(|l| l.borrow_mut().as_mut().is_some_and(|layer| std::mem::take(&mut layer.timers)))
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
        Fact::Machine if site.is_some_and(|s| s.at(BAKED_LOAD)) => Class::B,
        Fact::Environment | Fact::CurrentDir | Fact::Pid | Fact::Clock | Fact::Monotonic | Fact::Seed | Fact::Processors | Fact::Machine => Class::P,
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
    /// In `Mode::Record`, the paths this process's own effects wrote: a
    /// later read of one is of the compile's making, not an input.
    written: std::collections::HashSet<PathBuf>,
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
        records: Vec::new(), refusals: Vec::new(), seen: Default::default(), written: Default::default(),
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
            && (layer_rerun_allowed() || b.site.as_ref().is_some_and(|s| STARTUP.get(run).is_some_and(|file| s.file.ends_with(file))));
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
    let site = Rc::new(site());
    layer_form(&site.head);
    SiteGuard(with_build(|b| std::mem::replace(&mut b.site, Some(site))))
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

/// How many refusals the build has listed so far; 0 outside a build.
pub(crate) fn refusals() -> usize {
    with_build(|b| b.refusals.len()).unwrap_or(0)
}

/// List a refusal: KIND with FIELDS and the current site.
pub(crate) fn refuse(kind: &str, fields: Json) {
    if !refusing() {
        return;
    }
    with_build(|b| {
        let mut item = json!({"kind": kind, "site": b.site()});
        item.as_object_mut().unwrap().extend(fields.as_object().cloned().unwrap_or_default());
        b.push(true, item);
    });
}

/// A P fact WHAT is read: recorded where it is allowed, else refused.
fn p_read(fact: Fact<'_>, what: String) {
    if !refusing() {
        return;
    }
    // Asked before the build is borrowed: it records, and notes a timer's allowance.
    let reseed = reseed_reads(&what);
    with_build(|b| {
        let site = b.site.as_deref();
        let allowed = match (b.phase, class(fact, site)) {
            (Phase::Init, _) => Some("init"),
            (Phase::Startup, _) => Some("startup"),
            (Phase::Recipe, Class::B) => Some("baked"),
            (Phase::Recipe, _) if reseed => Some("reseed"),
            (Phase::Recipe, _) if layer_taint() => Some("layer"),
            _ => None,
        };
        let item = json!({"kind": "p-read", "site": b.site(), "what": what, "allowed": allowed});
        b.push(allowed.is_none(), item);
    });
}

/// F's host queries, unrecorded in `Mode::Record`: for a recorder that
/// records their outcome as one fact of its own.
pub fn unrecorded<T>(f: impl FnOnce() -> T) -> T {
    if mode() != Mode::Record {
        return f();
    }
    MODE.with(|m| m.set(Mode::Off));
    let result = f();
    MODE.with(|m| m.set(Mode::Record));
    result
}

/// PATH is written by this process's own effect: noted in `Mode::Record`.
fn wrote(path: &Path) {
    if mode() == Mode::Record {
        let path = if path.is_relative() { std::env::current_dir().map(|d| d.join(path)).unwrap_or_else(|_| path.to_owned()) } else { path.to_owned() };
        with_build(|b| b.written.insert(path));
    }
}

/// An effect WHAT: refused, failing with EPERM; Ok outside build mode.
fn effect(what: impl FnOnce() -> String) -> io::Result<()> {
    if !refusing() {
        return Ok(());
    }
    let what = what();
    if !layer_cut(&what) {
        refuse("effect", json!({"what": what}));
    }
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
    if reseed_reads("user-file") {
        return;
    }
    // This process's own entries (its umask, its maps): a per-process fact.
    if path.starts_with("/proc/self") {
        p_read(Fact::Pid, format!("{} read", bytes(path)));
        return;
    }
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
    if reseed_reads("user-file") {
        return;
    }
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

/// The runtime tree (gnu-lisp/, rlisp/, etc/): `ROMACS_RUNTIME_ROOT` (the Nix
/// wrapper sets it), else `share/romacs` beside the real executable's `bin/`
/// (an unpacked release tarball, wherever it lives), else [`build_root`].
pub fn runtime_root() -> PathBuf {
    getenv("ROMACS_RUNTIME_ROOT").map(PathBuf::from)
        .or_else(|| installed_root(&current_exe().ok()?))
        .unwrap_or_else(build_root)
}

/// `PREFIX/share/romacs` for an executable `PREFIX/bin/NAME`, when it holds a
/// `gnu-lisp/` directory.
pub(crate) fn installed_root(exe: &Path) -> Option<PathBuf> {
    let root = exe.parent()?.parent()?.join("share/romacs");
    metadata(root.join("gnu-lisp")).is_ok_and(|m| m.is_dir()).then_some(root)
}

/// The checkout this crate was built from: a compile-time path that no
/// installed system has, kept for development binaries under `target/`.
/// It is the manifest directory's grandparent itself, not `DIR/../..`:
/// `data-directory' and the other runtime directories are directory names
/// as GNU's are, so file names built from them compare equal to their
/// expansions (find-func's library completion, bug#43393's test).
pub(crate) fn build_root() -> PathBuf {
    let manifest = Path::new(env!("CARGO_MANIFEST_DIR"));
    manifest.parent().and_then(Path::parent).map_or_else(|| manifest.join("../.."), Path::to_path_buf)
}

/// A read of environment NAME through FORM: a K name is recorded with its
/// launch VALUE, a P name is a P read.
fn env_read(form: &str, name: &[u8], value: Option<&OsString>) {
    let shown = String::from_utf8_lossy(name);
    match class(Fact::Env(name), None) {
        // A compile process (`Mode::Record`) pins nothing: a per-process
        // name it reads is an input like any other.
        Class::P if refusing() => p_read(Fact::Env(name), format!("env {shown} ({form})")),
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
            if reseed_reads("user-file") {
                return Err(e);
            }
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
            probed("entry-type", &self.0.path(), entry_type_json(&result));
        }
        result
    }
}

/// The type of the entry PATH names, not following a symlink: an lstat
/// whose record is only the type, so the entry's times and size are not
/// keyed facts.
pub fn entry_type(path: &Path) -> io::Result<FileType> {
    let result = std::fs::symlink_metadata(path).map(|m| m.file_type());
    if building() {
        probed("entry-type", path, entry_type_json(&result));
    }
    result
}

fn entry_type_json(result: &io::Result<FileType>) -> Json {
    match result {
        Ok(t) => json!(if t.is_dir() { "dir" } else if t.is_file() { "file" } else if t.is_symlink() { "symlink" } else { "other" }),
        Err(e) => errno(e),
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

/// The type of the filesystem holding PATH: statfs's `f_type`, a magic
/// number of linux/magic.h. A keyed probe.
pub fn filesystem_type(path: &Path) -> io::Result<u32> {
    let result = path_c_string(path).and_then(|name| {
        let mut buf = std::mem::MaybeUninit::<libc::statfs>::uninit();
        match unsafe { libc::statfs(name.as_ptr(), buf.as_mut_ptr()) } {
            // f_type is a signed word; the magic numbers are 32 bits.
            0 => Ok(unsafe { buf.assume_init() }.f_type as u32),
            _ => Err(io::Error::last_os_error()),
        }
    });
    if building() {
        if let Err(e) = &result { confined(path, e); }
        probed("fs-type", path, result.as_ref().map_or_else(errno, |t| json!(t)));
    }
    result
}

/// The inode flags of DIRECTORY, opened without following a symlink
/// (FS_IOC_GETFLAGS, linux/fs.h). A keyed probe. Landlock refuses a confined
/// build a directory open outside its read roots, though the flags read
/// nothing beneath it, so that thread asks the one `keep_flags_reader`
/// started before it confined itself.
pub fn directory_flags(directory: &Path) -> io::Result<u32> {
    let reader = FLAGS_READER.with(|r| r.borrow().clone());
    let result = match reader {
        Some(reader) => {
            let (reply, answer) = std::sync::mpsc::channel();
            reader.send((directory.to_owned(), reply)).map_err(|_| io::Error::other("flags reader gone"))
                .and_then(|()| answer.recv().map_err(|_| io::Error::other("flags reader gone"))?)
        }
        None => read_directory_flags(directory),
    };
    if building() {
        probed("dir-flags", directory, result.as_ref().map_or_else(errno, |f| json!(f)));
    }
    result
}

/// The POSIX access ACL of the file PATH names, following a symlink: its
/// `system.posix_acl_access' attribute, or None when it has none (its
/// mode bits are then its ACL). A keyed probe.
pub fn posix_acl(path: &Path) -> io::Result<Option<Vec<u8>>> {
    const NAME: &CStr = c"system.posix_acl_access";
    let result = (|| {
        let name = std::ffi::CString::new(path.as_os_str().as_bytes()).map_err(|_| io::Error::from_raw_os_error(libc::EINVAL))?;
        loop {
            let size = unsafe { libc::getxattr(name.as_ptr(), NAME.as_ptr(), std::ptr::null_mut(), 0) };
            if size < 0 {
                let error = io::Error::last_os_error();
                return if error.raw_os_error() == Some(libc::ENODATA) { Ok(None) } else { Err(error) };
            }
            let mut value = vec![0u8; size as usize];
            let read = unsafe { libc::getxattr(name.as_ptr(), NAME.as_ptr(), value.as_mut_ptr().cast(), value.len()) };
            if read >= 0 { value.truncate(read as usize); return Ok(Some(value)); }
            // The attribute grew between the two calls: ask its size again.
            if io::Error::last_os_error().raw_os_error() != Some(libc::ERANGE) { return Err(io::Error::last_os_error()); }
        }
    })();
    if building() {
        if let Err(e) = &result { confined(path, e); }
        let observed = match &result { Ok(Some(value)) => encoded(value), Ok(None) => json!(null), Err(e) => errno(e) };
        probed("acl", path, observed);
    }
    result
}

fn read_directory_flags(directory: &Path) -> io::Result<u32> {
    let name = path_c_string(directory)?;
    let fd = unsafe { libc::open(name.as_ptr(), libc::O_RDONLY | libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC) };
    if fd < 0 {
        return Err(io::Error::last_os_error());
    }
    let fd = unsafe { OwnedFd::from_raw_fd(fd) };
    // The kernel reads and writes an int, whatever the request's size says.
    let mut flags: libc::c_int = 0;
    use std::os::fd::AsRawFd;
    match unsafe { libc::ioctl(fd.as_raw_fd(), libc::FS_IOC_GETFLAGS, &mut flags) } {
        0 => Ok(flags as u32),
        _ => Err(io::Error::last_os_error()),
    }
}

fn path_c_string(path: &Path) -> io::Result<std::ffi::CString> {
    std::ffi::CString::new(path.as_os_str().as_bytes()).map_err(|_| io::Error::from_raw_os_error(libc::EINVAL))
}

/// Start the thread that reads directory flags for this one
/// (`directory_flags`), before this thread confines itself: it answers that
/// one query and makes no other.
pub(crate) fn keep_flags_reader() -> io::Result<()> {
    let (reader, queries) = std::sync::mpsc::channel::<(PathBuf, std::sync::mpsc::Sender<io::Result<u32>>)>();
    std::thread::Builder::new().name("romacs-flags-reader".into()).spawn(move || {
        for (directory, reply) in queries {
            let _ = reply.send(read_directory_flags(&directory));
        }
    })?;
    FLAGS_READER.with(|r| *r.borrow_mut() = Some(reader));
    Ok(())
}

/// Set the POSIX access ACL of the file PATH names to VALUE, in its
/// `system.posix_acl_access' form.
pub fn set_posix_acl(path: &Path, value: &[u8]) -> io::Result<()> {
    effect(|| format!("set ACL of {}", bytes(path)))?;
    let name = std::ffi::CString::new(path.as_os_str().as_bytes()).map_err(|_| io::Error::from_raw_os_error(libc::EINVAL))?;
    status(unsafe { libc::setxattr(name.as_ptr(), c"system.posix_acl_access".as_ptr(), value.as_ptr().cast(), value.len(), 0) })
}

/// sync(2): every pending disk update, the whole system's.
pub fn sync_all() -> io::Result<()> {
    effect(|| "sync".into())?;
    unsafe { libc::sync() };
    Ok(())
}

/// Close this process's standard stream FD (0, 1 or 2).
pub fn close_standard_stream(fd: i32) -> io::Result<()> {
    effect(|| format!("close standard stream {fd}"))?;
    status(unsafe { libc::close(fd) })
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
/// a record that asks the host nothing.
pub fn replay(record: &Json) -> Option<Json> {
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
                "entry-type" => { let _ = entry_type(&path()?); }
                "statvfs" => { let _ = statvfs(&std::ffi::CString::new(path()?.into_os_string().into_vec()).ok()?); }
                "fs-type" => { let _ = filesystem_type(&path()?); }
                "dir-flags" => { let _ = directory_flags(&path()?); }
                "acl" => { let _ = posix_acl(&path()?); }
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
                // Asked as a compile process's recorder asks it, which
                // records a per-process name too (`env_read`); a build
                // records none of those as `env`.
                MODE.with(|m| m.set(Mode::Record));
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
                "gid" => { gid(); }
                "egid" => { egid(); }
                "system-name" => { let _ = system_name(); }
                _ => return None,
            },
            "user-db" => {
                let key = &record["key"];
                if let Some(list) = key.as_str() {
                    match list { "users" => { account_names(false); } "groups" => { account_names(true); } _ => return None }
                } else if let Some(uid) = key["uid"].as_u64() {
                    let _ = user_by_uid(uid as u32, 1 << 16);
                } else if let Some(gid) = key["gid"].as_u64() {
                    let _ = group_by_gid(gid as u32, 1 << 16);
                } else if key.get("group").is_some() {
                    let _ = group_by_name(&std::ffi::CString::new(decoded(&key["group"])?).ok()?, 1 << 16);
                } else {
                    let _ = user_by_name(&std::ffi::CString::new(decoded(&key["name"])?).ok()?, 1 << 16);
                }
            }
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
    matches!(record["kind"].as_str(), Some("file-read" | "probe" | "dir-list" | "canon" | "env" | "host" | "user-db"))
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
    if refusing() {
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

/// Die with PARENT: SIGTERM when the thread that started this process
/// ends, and exit now if PARENT is gone already. A child of a session
/// thread then never outlives the session.
pub fn die_with_parent(parent: u32) {
    unsafe { libc::prctl(libc::PR_SET_PDEATHSIG, libc::SIGTERM) };
    if unsafe { libc::getppid() } as u32 != parent {
        std::process::exit(1);
    }
}

/// Lower this process to the background: the weakest CPU niceness and the
/// idle I/O class, which its children inherit. Best effort.
pub fn lower_priority() {
    const IOPRIO_WHO_PROCESS: libc::c_long = 1;
    const IOPRIO_CLASS_IDLE: libc::c_long = 3;
    unsafe {
        libc::setpriority(libc::PRIO_PROCESS, 0, 19);
        libc::syscall(libc::SYS_ioprio_set, IOPRIO_WHO_PROCESS, 0 as libc::c_long, IOPRIO_CLASS_IDLE << 13);
    }
}

pub fn euid() -> u32 {
    let euid = unsafe { libc::geteuid() };
    if building() {
        host_fact("euid", json!(euid));
    }
    euid
}

pub fn gid() -> u32 {
    let gid = unsafe { libc::getgid() };
    if building() {
        host_fact("gid", json!(gid));
    }
    gid
}

pub fn egid() -> u32 {
    let egid = unsafe { libc::getegid() };
    if building() {
        host_fact("egid", json!(egid));
    }
    egid
}

// User database.

/// A passwd entry's fields, as the C library's bytes.
pub struct Passwd {
    pub uid: u32,
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
    Ok(Passwd { uid: entry.pw_uid, name: field(entry.pw_name), gecos: bytes(entry.pw_gecos), dir: field(entry.pw_dir) })
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
            Ok(p) => json!(sha256(&[Some(p.uid.to_string().as_bytes()), p.name.as_deref(), Some(&p.gecos), p.dir.as_deref()].map(|f| f.map(<[u8]>::to_vec)).iter()
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

/// The group NAME names: its gid.
pub fn group_by_name(name: &CStr, buffer: usize) -> Result<u32, i32> {
    let mut buffer = vec![0 as libc::c_char; buffer];
    let mut entry = std::mem::MaybeUninit::<libc::group>::uninit();
    let mut found = std::ptr::null_mut();
    let status = unsafe { libc::getgrnam_r(name.as_ptr(), entry.as_mut_ptr(), buffer.as_mut_ptr(), buffer.len(), &mut found) };
    let result = if status != 0 || found.is_null() { Err(status) } else { Ok(unsafe { (*found).gr_gid }) };
    if building() {
        let observed = match &result { Ok(gid) => json!(gid), Err(status) => json!({"status": status}) };
        record("user-db", json!({"key": {"group": encoded(name.to_bytes())}, "result": observed}));
    }
    result
}

/// The names getpwent (users) or getgrent (groups) walks the whole database
/// for, in its order, then endpwent or endgrent, as dired.c Fsystem_users and
/// Fsystem_groups do (so NSS sources count). The walk's cursor is the C
/// library's, shared by the process, so one walk runs at a time.
pub fn account_names(groups: bool) -> Vec<Vec<u8>> {
    static WALK: std::sync::Mutex<()> = std::sync::Mutex::new(());
    let _walk = WALK.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    let next = || unsafe {
        let name = if groups { libc::getgrent().as_ref().map(|g| g.gr_name) } else { libc::getpwent().as_ref().map(|p| p.pw_name) };
        name.map(|name| (!name.is_null()).then(|| CStr::from_ptr(name).to_bytes().to_vec()))
    };
    let mut names = Vec::new();
    unsafe { if groups { libc::setgrent() } else { libc::setpwent() } }
    while let Some(name) = next() { names.extend(name); }
    unsafe { if groups { libc::endgrent() } else { libc::endpwent() } }
    if building() {
        record("user-db", json!({"key": if groups { "groups" } else { "users" }, "result": sha256(&names.join(&0))}));
    }
    names
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

/// A per-process fact (`Fact::Processors`): no world is keyed on it.
pub fn processors(set: Processors) -> u64 {
    p_read(Fact::Processors, "processors".into());
    count_processors(set)
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

// Performance counters.

/// User-space instructions the calling thread has retired since its first
/// call: a pinned Linux hardware counter (perf_event_open), opened once per
/// thread. Unlike time it is load-insensitive and repeatable, so scaling
/// probes compare work at two sizes by it. None where the kernel refuses
/// counters (perf_event_paranoid above 2, no PMU) or cannot keep this one
/// counting all the time: a number is always a whole count.
pub(crate) fn thread_instructions() -> Option<u64> {
    thread_local! {
        static COUNTER: std::cell::OnceCell<Option<std::os::fd::OwnedFd>> = const { std::cell::OnceCell::new() };
    }
    COUNTER.with(|counter| counter.get_or_init(open_instruction_counter).as_ref().and_then(read_counter))
}

#[cfg(target_os = "linux")]
fn open_instruction_counter() -> Option<std::os::fd::OwnedFd> {
    use std::os::fd::FromRawFd;
    // perf_event_attr, PERF_ATTR_SIZE_VER0: type HARDWARE (0) and size,
    // config INSTRUCTIONS (1), flags pinned | exclude_kernel | exclude_hv.
    const PERF_FLAG_FD_CLOEXEC: libc::c_ulong = 8;
    let mut attr = [0u64; 8];
    attr[0] = 64 << 32;
    attr[1] = 1;
    attr[5] = 1 << 2 | 1 << 5 | 1 << 6;
    // SAFETY: ATTR is a zeroed PERF_ATTR_SIZE_VER0 struct that outlives the call.
    let fd = unsafe { libc::syscall(libc::SYS_perf_event_open, attr.as_ptr(), 0, -1, -1, PERF_FLAG_FD_CLOEXEC) };
    // SAFETY: a non-negative result is a new descriptor this thread owns.
    (fd >= 0).then(|| unsafe { std::os::fd::OwnedFd::from_raw_fd(fd as i32) })
}

#[cfg(not(target_os = "linux"))]
fn open_instruction_counter() -> Option<std::os::fd::OwnedFd> {
    None
}

/// The counter's value; none once a pinned counter lost its PMU slot,
/// which the kernel reports as end of file.
fn read_counter(fd: &std::os::fd::OwnedFd) -> Option<u64> {
    use std::os::fd::AsRawFd;
    let mut count = 0u64;
    // SAFETY: COUNT is 8 writable bytes; FD is open for the thread's life.
    let read = unsafe { libc::read(fd.as_raw_fd(), (&raw mut count).cast(), 8) };
    (read == 8).then_some(count)
}

// Time.

/// The kernel's clock ticks per second (`_SC_CLK_TCK`), the unit of
/// /proc's process times.
pub fn clock_ticks_per_second() -> u64 {
    u64::try_from(unsafe { libc::sysconf(libc::_SC_CLK_TCK) }).ok().filter(|&n| n > 0).unwrap_or(100)
}

/// The time since boot, suspended time included (`CLOCK_BOOTTIME`), as
/// GNU's get_host_uptime reads it; None where the clock is missing.
pub fn boot_clock() -> Option<std::time::Duration> {
    let mut t: libc::timespec = unsafe { std::mem::zeroed() };
    let ok = unsafe { libc::clock_gettime(libc::CLOCK_BOOTTIME, &mut t) } == 0;
    ok.then(|| std::time::Duration::new(t.tv_sec as u64, t.tv_nsec as u32))
}

/// sysinfo(2)'s RAM and swap totals and free amounts, in bytes:
/// (total RAM, free RAM, total swap, free swap).
pub fn memory_info() -> Option<[u64; 4]> {
    p_read(Fact::Machine, "memory-info".into());
    let mut si: libc::sysinfo = unsafe { std::mem::zeroed() };
    if unsafe { libc::sysinfo(&mut si) } != 0 { return None; }
    let unit = u64::from(si.mem_unit);
    Some([si.totalram as u64 * unit, si.freeram as u64 * unit, si.totalswap as u64 * unit, si.freeswap as u64 * unit])
}

/// getloadavg(3): the 1, 5 and 15 minute averages it has, None on failure.
pub fn load_average() -> Option<Vec<f64>> {
    p_read(Fact::Machine, "load-average".into());
    let mut loads = [0f64; 3];
    let n = unsafe { libc::getloadavg(loads.as_mut_ptr(), 3) };
    (n >= 0).then(|| loads[..n as usize].to_vec())
}

/// The memory page size in bytes (`getpagesize`).
pub fn page_size() -> u64 {
    u64::try_from(unsafe { libc::sysconf(libc::_SC_PAGESIZE) }).ok().filter(|&n| n > 0).unwrap_or(4096)
}

/// A time-zone file's bytes: `/etc/localtime`, a TZ file or a zoneinfo entry.
pub fn zone_file(path: impl AsRef<Path>) -> io::Result<Vec<u8>> {
    read_file(path)
}

/// The wall clock; in build mode the build time.
pub fn clock() -> SystemTime {
    if refusing() {
        p_read(Fact::Clock, "clock".into());
        return with_build(|b| b.pinned.clock).unwrap_or(SystemTime::UNIX_EPOCH);
    }
    SystemTime::now()
}

/// The processor time this process has used, user and system, as
/// `getrusage (RUSAGE_SELF)` sums them (GNU sysdep.c
/// `get-internal-run-time'); in build mode none, a per-process fact.
pub fn run_time() -> io::Result<std::time::Duration> {
    if refusing() {
        p_read(Fact::Monotonic, "run time".into());
        return Ok(std::time::Duration::ZERO);
    }
    let mut usage = std::mem::MaybeUninit::<libc::rusage>::zeroed();
    // SAFETY: getrusage writes the whole struct it is given.
    if unsafe { libc::getrusage(libc::RUSAGE_SELF, usage.as_mut_ptr()) } < 0 {
        return Err(io::Error::last_os_error());
    }
    // SAFETY: initialized by the successful call above.
    let usage = unsafe { usage.assume_init() };
    let part = |t: libc::timeval| std::time::Duration::new(t.tv_sec.max(0) as u64, (t.tv_usec.clamp(0, 999_999) as u32) * 1000);
    Ok(part(usage.ru_utime) + part(usage.ru_stime))
}

/// The monotonic clock, for deadlines and measurement.
pub fn monotonic() -> Instant {
    if building() {
        p_read(Fact::Monotonic, "monotonic clock".into());
    }
    Instant::now()
}

/// When a collection started, for `gc-elapsed'; none in build mode, so a
/// world's heap never depends on how long its collections took.
pub fn collection_clock() -> Option<Instant> {
    (!building()).then(Instant::now)
}

// Effects.

/// Four bytes of kernel entropy; in build mode the pinned seed.
pub fn random_seed() -> io::Result<[u8; 4]> {
    if refusing() {
        p_read(Fact::Seed, "random seed".into());
        return Ok(with_build(|b| b.pinned.seed).unwrap_or_default());
    }
    let mut bytes = [0u8; 4];
    random_source()?.read_exact(&mut bytes)?;
    Ok(bytes)
}

/// PATH opened with OPTIONS for writing only: reads go through `Input`.
pub fn open_output(path: impl AsRef<Path>, options: &OpenOptions) -> io::Result<File> {
    wrote(path.as_ref());
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
    wrote(path.as_ref());
    effect(|| format!("write {}", self::bytes(path.as_ref())))?;
    std::fs::write(path, bytes)
}

pub fn rename(from: impl AsRef<Path>, to: impl AsRef<Path>) -> io::Result<()> {
    wrote(from.as_ref());
    wrote(to.as_ref());
    effect(|| format!("rename {} {}", bytes(from.as_ref()), bytes(to.as_ref())))?;
    std::fs::rename(from, to)
}

/// fsync the directory DIR, so a rename into it survives a host crash.
pub fn sync_dir(dir: impl AsRef<Path>) -> io::Result<()> {
    effect(|| format!("sync directory {}", bytes(dir.as_ref())))?;
    File::open(dir)?.sync_all()
}

pub fn remove_file(path: impl AsRef<Path>) -> io::Result<()> {
    wrote(path.as_ref());
    effect(|| format!("delete {}", bytes(path.as_ref())))?;
    std::fs::remove_file(path)
}

pub fn remove_dir(path: impl AsRef<Path>) -> io::Result<()> {
    wrote(path.as_ref());
    effect(|| format!("delete directory {}", bytes(path.as_ref())))?;
    std::fs::remove_dir(path)
}

/// mkdir with MODE before the umask.
pub fn create_dir(path: impl AsRef<Path>, mode: u32) -> io::Result<()> {
    wrote(path.as_ref());
    effect(|| format!("make directory {}", bytes(path.as_ref())))?;
    use std::os::unix::fs::DirBuilderExt;
    std::fs::DirBuilder::new().mode(mode).create(path)
}

pub fn create_dir_all(path: impl AsRef<Path>) -> io::Result<()> {
    wrote(path.as_ref());
    effect(|| format!("make directory {}", bytes(path.as_ref())))?;
    std::fs::create_dir_all(path)
}

pub fn hard_link(original: impl AsRef<Path>, link: impl AsRef<Path>) -> io::Result<()> {
    wrote(link.as_ref());
    effect(|| format!("link {}", bytes(link.as_ref())))?;
    std::fs::hard_link(original, link)
}

pub fn symlink(target: impl AsRef<Path>, link: impl AsRef<Path>) -> io::Result<()> {
    wrote(link.as_ref());
    effect(|| format!("symlink {}", bytes(link.as_ref())))?;
    std::os::unix::fs::symlink(target, link)
}

pub fn fchown(file: &File, uid: Option<u32>, gid: Option<u32>) -> io::Result<()> {
    std::os::unix::fs::fchown(file, uid, gid)
}

/// fchmodat: PATH's mode bits, a symlink's own under NOFOLLOW.
pub fn chmod(path: &CStr, mode: u32, nofollow: bool) -> io::Result<()> {
    wrote(Path::new(OsStr::from_bytes(path.to_bytes())));
    effect(|| format!("chmod {}", String::from_utf8_lossy(path.to_bytes())))?;
    let flags = if nofollow { libc::AT_SYMLINK_NOFOLLOW } else { 0 };
    status(unsafe { libc::fchmodat(libc::AT_FDCWD, path.as_ptr(), mode as libc::mode_t, flags) })
}

/// renameat2 with RENAME_NOREPLACE: EEXIST rather than replace TO.
pub fn rename_noreplace(from: &CStr, to: &CStr) -> io::Result<()> {
    wrote(Path::new(OsStr::from_bytes(to.to_bytes())));
    effect(|| format!("rename {}", String::from_utf8_lossy(to.to_bytes())))?;
    status(unsafe { libc::renameat2(libc::AT_FDCWD, from.as_ptr(), libc::AT_FDCWD, to.as_ptr(), libc::RENAME_NOREPLACE) })
}

/// utimensat: both of PATH's times set to TIME (seconds, nanoseconds), or
/// now; a symlink's own under NOFOLLOW.
pub fn set_times(path: &CStr, time: Option<(i64, i64)>, nofollow: bool) -> io::Result<()> {
    wrote(Path::new(OsStr::from_bytes(path.to_bytes())));
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
/// walk starts at `/` and opens one component at a time relative to the
/// last (openat, O_DIRECTORY|O_NOFOLLOW), making a missing one there
/// (mkdirat, then fchmod): nothing is resolved again by path, and no
/// symlink is followed, not even as HOME. Each directory from `/` down
/// must be owned by this user or root, and writable by no one else unless
/// sticky, or another user could swap what lies below it.
pub fn open_store_dir(path: &Path, create: bool) -> io::Result<std::os::fd::OwnedFd> {
    use std::os::fd::{AsRawFd, FromRawFd, OwnedFd};
    use std::os::unix::ffi::OsStrExt;
    let refuse = |why: String| io::Error::new(io::ErrorKind::PermissionDenied, why);
    if !path.is_absolute() {
        return Err(refuse(format!("{} is not absolute", path.display())));
    }
    let root = PathBuf::from("/");
    let open = |at: Option<i32>, name: &std::ffi::CStr| -> i32 {
        let flags = libc::O_DIRECTORY | libc::O_RDONLY | libc::O_CLOEXEC | libc::O_NOFOLLOW;
        match at {
            Some(dir) => unsafe { libc::openat(dir, name.as_ptr(), flags) },
            None => unsafe { libc::open(name.as_ptr(), flags) },
        }
    };
    let fd = open(None, c"/");
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

/// A memfd holding BYTES, sealed against writing, shrinking and growing
/// once written: the backing of a large frozen region (`frozen.rs`), which
/// another process may map or read knowing the bytes cannot change.
pub fn sealed_memfd(bytes: &[u8]) -> io::Result<File> {
    use std::io::Write;
    use std::os::fd::AsRawFd;
    effect(|| "memfd".into())?;
    let fd = unsafe { libc::memfd_create(c"romacs-frozen".as_ptr(), libc::MFD_CLOEXEC | libc::MFD_ALLOW_SEALING) };
    if fd < 0 {
        return Err(io::Error::last_os_error());
    }
    let mut file = unsafe { File::from_raw_fd(fd) };
    file.write_all(bytes)?;
    status(unsafe { libc::fcntl(file.as_raw_fd(), libc::F_ADD_SEALS, SEALS) })?;
    Ok(file)
}

const SEALS: libc::c_int = libc::F_SEAL_WRITE | libc::F_SEAL_SHRINK | libc::F_SEAL_GROW;

/// FILE's bytes, when `sealed_memfd`'s seals are on it; a descriptor another
/// process sent is refused otherwise, since its bytes could change under
/// the reader.
pub fn read_sealed(file: &File) -> io::Result<Vec<u8>> {
    use std::os::fd::AsRawFd;
    use std::os::unix::fs::FileExt;
    let seals = unsafe { libc::fcntl(file.as_raw_fd(), libc::F_GET_SEALS) };
    if seals < 0 || seals & SEALS != SEALS {
        return Err(io::Error::new(io::ErrorKind::PermissionDenied, "frozen region descriptor is not sealed"));
    }
    let len = usize::try_from(file.metadata()?.len()).map_err(|_| io::Error::other("frozen region too large"))?;
    let mut bytes = vec![0; len];
    file.read_exact_at(&mut bytes, 0)?;
    Ok(bytes)
}

// File notification (`prim/leaf/inotify.rs`): GNU inotify.c's one
// instance per Emacs, whose descriptor the keyboard waits poll.

/// inotify_init1(2), close-on-exec and nonblocking.
pub fn inotify_init() -> io::Result<OwnedFd> {
    effect(|| "inotify".into())?;
    let fd = unsafe { libc::inotify_init1(libc::IN_NONBLOCK | libc::IN_CLOEXEC) };
    if fd < 0 { return Err(io::Error::last_os_error()); }
    Ok(unsafe { OwnedFd::from_raw_fd(fd) })
}

/// inotify_add_watch(2): PATH's watch descriptor on FD with MASK.
pub fn inotify_add_watch(fd: std::os::fd::BorrowedFd<'_>, path: &CStr, mask: u32) -> io::Result<i32> {
    use std::os::fd::AsRawFd;
    effect(|| format!("inotify watch {}", path.to_string_lossy()))?;
    let wd = unsafe { libc::inotify_add_watch(fd.as_raw_fd(), path.as_ptr(), mask) };
    if wd < 0 { Err(io::Error::last_os_error()) } else { Ok(wd) }
}

/// inotify_rm_watch(2).
pub fn inotify_rm_watch(fd: std::os::fd::BorrowedFd<'_>, wd: i32) -> io::Result<()> {
    use std::os::fd::AsRawFd;
    status(unsafe { libc::inotify_rm_watch(fd.as_raw_fd(), wd) })
}

/// The `struct inotify_event' records queued on FD, as their bytes; none
/// when nothing is queued (FD is nonblocking).
pub fn inotify_read(fd: std::os::fd::BorrowedFd<'_>) -> io::Result<Vec<u8>> {
    use std::os::fd::AsRawFd;
    let mut queued: libc::c_int = 0;
    status(unsafe { libc::ioctl(fd.as_raw_fd(), libc::FIONREAD, &mut queued) })?;
    // One record at least, whatever FIONREAD said, so a read never fails
    // with EINVAL for a buffer too small for the next record.
    let mut buffer = vec![0u8; (queued.max(0) as usize).max(std::mem::size_of::<libc::inotify_event>() + 256)];
    let n = unsafe { libc::read(fd.as_raw_fd(), buffer.as_mut_ptr().cast(), buffer.len()) };
    if n < 0 {
        let e = io::Error::last_os_error();
        return if e.kind() == io::ErrorKind::WouldBlock { Ok(Vec::new()) } else { Err(e) };
    }
    buffer.truncate(n as usize);
    Ok(buffer)
}

// Isolates (`isolate.rs`): worker processes, their control sockets, the
// descriptors they pass, and the signals between them.

/// FD nonblocking: a worker's inherited control socket.
pub fn set_nonblocking(fd: std::os::fd::BorrowedFd<'_>) -> io::Result<()> {
    use std::os::fd::AsRawFd;
    let flags = unsafe { libc::fcntl(fd.as_raw_fd(), libc::F_GETFL) };
    if flags < 0 { return Err(io::Error::last_os_error()); }
    status(unsafe { libc::fcntl(fd.as_raw_fd(), libc::F_SETFL, flags | libc::O_NONBLOCK) }.min(0))
}

/// A connected pair of close-on-exec, nonblocking `SOCK_SEQPACKET` sockets:
/// one packet per `send_packet`, received whole or not at all; a full
/// buffer makes a send fail with `WouldBlock` rather than wait.
pub fn socketpair() -> io::Result<(OwnedFd, OwnedFd)> {
    effect(|| "socketpair".into())?;
    let mut fds = [0; 2];
    status(unsafe { libc::socketpair(libc::AF_UNIX, libc::SOCK_SEQPACKET | libc::SOCK_CLOEXEC | libc::SOCK_NONBLOCK, 0, fds.as_mut_ptr()) })?;
    Ok(unsafe { (OwnedFd::from_raw_fd(fds[0]), OwnedFd::from_raw_fd(fds[1])) })
}

/// BYTES as one packet on SOCKET, with FD passed beside it (`SCM_RIGHTS`);
/// `WouldBlock` when the peer's buffer is full (a bounded queue).
pub fn send_packet(socket: std::os::fd::BorrowedFd<'_>, bytes: &[u8], fd: Option<std::os::fd::BorrowedFd<'_>>) -> io::Result<()> {
    use std::os::fd::AsRawFd;
    effect(|| "sendmsg".into())?;
    let mut iov = libc::iovec { iov_base: bytes.as_ptr() as *mut libc::c_void, iov_len: bytes.len() };
    let space = unsafe { libc::CMSG_SPACE(4) } as usize;
    let mut control = vec![0u8; space];
    let mut msg: libc::msghdr = unsafe { std::mem::zeroed() };
    msg.msg_iov = &mut iov;
    msg.msg_iovlen = 1;
    if let Some(fd) = fd {
        msg.msg_control = control.as_mut_ptr().cast();
        msg.msg_controllen = space as _;
        // SAFETY: the control buffer holds exactly one descriptor header.
        unsafe {
            let cmsg = libc::CMSG_FIRSTHDR(&msg);
            (*cmsg).cmsg_level = libc::SOL_SOCKET;
            (*cmsg).cmsg_type = libc::SCM_RIGHTS;
            (*cmsg).cmsg_len = libc::CMSG_LEN(4) as _;
            std::ptr::write_unaligned(libc::CMSG_DATA(cmsg).cast::<libc::c_int>(), fd.as_raw_fd());
        }
    }
    loop {
        let n = unsafe { libc::sendmsg(socket.as_raw_fd(), &msg, libc::MSG_NOSIGNAL) };
        if n >= 0 { return Ok(()); }
        let error = io::Error::last_os_error();
        if error.kind() != io::ErrorKind::Interrupted { return Err(error); }
    }
}

/// One packet from SOCKET, up to CAPACITY bytes, with the descriptors passed
/// beside it (close-on-exec); None once the peer has closed. A packet the
/// buffer cannot hold whole is an error, never a prefix.
pub fn recv_packet(socket: std::os::fd::BorrowedFd<'_>, capacity: usize) -> io::Result<Option<(Vec<u8>, Vec<OwnedFd>)>> {
    use std::os::fd::AsRawFd;
    let mut bytes = vec![0u8; capacity];
    let mut iov = libc::iovec { iov_base: bytes.as_mut_ptr().cast(), iov_len: capacity };
    let space = unsafe { libc::CMSG_SPACE(4 * 8) } as usize;
    let mut control = vec![0u8; space];
    let mut msg: libc::msghdr = unsafe { std::mem::zeroed() };
    msg.msg_iov = &mut iov;
    msg.msg_iovlen = 1;
    msg.msg_control = control.as_mut_ptr().cast();
    msg.msg_controllen = space as _;
    let n = loop {
        let n = unsafe { libc::recvmsg(socket.as_raw_fd(), &mut msg, libc::MSG_CMSG_CLOEXEC) };
        if n >= 0 { break n as usize; }
        let error = io::Error::last_os_error();
        if error.kind() != io::ErrorKind::Interrupted { return Err(error); }
    };
    let mut fds = Vec::new();
    // SAFETY: the kernel filled the control buffer it was given.
    unsafe {
        let mut cmsg = libc::CMSG_FIRSTHDR(&msg);
        while !cmsg.is_null() {
            if (*cmsg).cmsg_level == libc::SOL_SOCKET && (*cmsg).cmsg_type == libc::SCM_RIGHTS {
                let count = ((*cmsg).cmsg_len as usize - libc::CMSG_LEN(0) as usize) / 4;
                let data = libc::CMSG_DATA(cmsg).cast::<libc::c_int>();
                for i in 0..count { fds.push(OwnedFd::from_raw_fd(std::ptr::read_unaligned(data.add(i)))); }
            }
            cmsg = libc::CMSG_NXTHDR(&msg, cmsg);
        }
    }
    if n == 0 && fds.is_empty() { return Ok(None); }
    if msg.msg_flags & libc::MSG_TRUNC != 0 {
        return Err(io::Error::new(io::ErrorKind::InvalidData, "isolate packet larger than its buffer"));
    }
    bytes.truncate(n);
    Ok(Some((bytes, fds)))
}

/// `PIDFD_SIGNAL_PROCESS_GROUP` (linux/pidfd.h, Linux 6.9): SIGNAL goes to
/// the process group of the process PIDFD names.
const PIDFD_SIGNAL_PROCESS_GROUP: libc::c_uint = 1 << 2;

/// SIGNAL through PIDFD, to the process or, with GROUP, to its whole
/// process group (Linux 6.9 and later; `EINVAL` before). A pidfd names one
/// process for good, so this never reaches a reused pid: a reaped process
/// gives `ESRCH`, while its group, still alive without it, is signalled.
pub fn pidfd_send_signal(pidfd: std::os::fd::BorrowedFd<'_>, signal: i32, group: bool) -> io::Result<()> {
    use std::os::fd::AsRawFd;
    effect(|| format!("pidfd signal {signal}"))?;
    let flags = if group { PIDFD_SIGNAL_PROCESS_GROUP } else { 0 };
    status(unsafe { libc::syscall(libc::SYS_pidfd_send_signal, pidfd.as_raw_fd(), signal, std::ptr::null::<libc::siginfo_t>(), flags) } as libc::c_int)
}

/// prctl(PR_SET_PDEATHSIG): SIGNAL once the thread that made this process
/// exits. The caller checks `parent_pid` after, for a parent gone before.
pub fn set_parent_death_signal(signal: i32) -> io::Result<()> {
    effect(|| "pdeathsig".into())?;
    status(unsafe { libc::prctl(libc::PR_SET_PDEATHSIG, signal) })
}

pub fn parent_pid() -> i32 {
    if building() {
        p_read(Fact::Pid, "ppid".into());
        return 0;
    }
    unsafe { libc::getppid() }
}

/// The worker's end of an isolate's signals: SIGUSR1 notes an interrupt for
/// the engine's safepoints, SIGTERM a termination for its loop. Either
/// also writes a byte to the wake pipe, which the worker's isolate wait
/// polls: the signal may land on any of the process's threads (the main
/// one waits in a join), so only the pipe reliably ends a blocked wait.
static INTERRUPT: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
static TERMINATE: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
/// The wake pipe's read and write ends, or -1.
static WAKE_READ: std::sync::atomic::AtomicI32 = std::sync::atomic::AtomicI32::new(-1);
static WAKE_WRITE: std::sync::atomic::AtomicI32 = std::sync::atomic::AtomicI32::new(-1);

fn wake_isolate_wait() {
    let fd = WAKE_WRITE.load(std::sync::atomic::Ordering::SeqCst);
    // Async-signal-safe; a full pipe already wakes the wait.
    if fd >= 0 { let _ = unsafe { libc::write(fd, [0u8].as_ptr().cast(), 1) }; }
}
extern "C" fn note_interrupt(_: libc::c_int) {
    INTERRUPT.store(true, std::sync::atomic::Ordering::SeqCst);
    wake_isolate_wait();
}
extern "C" fn note_terminate(_: libc::c_int) {
    TERMINATE.store(true, std::sync::atomic::Ordering::SeqCst);
    wake_isolate_wait();
}

/// The read end of the wake pipe, once `catch_isolate_signals' made it.
pub fn isolate_wake_fd() -> Option<std::os::fd::RawFd> {
    let fd = WAKE_READ.load(std::sync::atomic::Ordering::SeqCst);
    (fd >= 0).then_some(fd)
}
/// Empty the wake pipe: the flags, not the bytes, say what happened.
pub fn drain_isolate_wake() {
    if let Some(fd) = isolate_wake_fd() {
        let mut bytes = [0u8; 64];
        while unsafe { libc::read(fd, bytes.as_mut_ptr().cast(), bytes.len()) } > 0 {}
    }
}

pub fn catch_isolate_signals() -> io::Result<()> {
    if WAKE_READ.load(std::sync::atomic::Ordering::SeqCst) < 0 {
        effect(|| "isolate wake pipe".into())?;
        let mut fds = [0; 2];
        status(unsafe { libc::pipe2(fds.as_mut_ptr(), libc::O_CLOEXEC | libc::O_NONBLOCK) })?;
        // For the process's life: a handler may write to it at any time.
        WAKE_WRITE.store(fds[1], std::sync::atomic::Ordering::SeqCst);
        WAKE_READ.store(fds[0], std::sync::atomic::Ordering::SeqCst);
    }
    for (signal, handler) in [(libc::SIGUSR1, note_interrupt as extern "C" fn(libc::c_int)), (libc::SIGTERM, note_terminate)] {
        effect(|| format!("handle signal {signal}"))?;
        let mut action: libc::sigaction = unsafe { std::mem::zeroed() };
        action.sa_sigaction = handler as *const () as usize;
        action.sa_flags = 0;
        unsafe { libc::sigemptyset(&mut action.sa_mask) };
        status(unsafe { libc::sigaction(signal, &action, std::ptr::null_mut()) })?;
    }
    Ok(())
}

/// An interrupt or a termination was signalled and not yet taken: two
/// relaxed loads, for the engine's safepoints.
#[inline(always)]
pub fn interrupt_pending() -> bool {
    INTERRUPT.load(std::sync::atomic::Ordering::Relaxed) || TERMINATE.load(std::sync::atomic::Ordering::Relaxed)
}
/// What the engine's safepoints poll: an interrupt or termination, or a
/// user signal that arrived since one last looked, which may be a debug
/// event (`Vm::take_debug_event'). Three relaxed loads.
#[inline(always)]
pub fn safepoint_pending() -> bool {
    interrupt_pending() || USER_SIGNAL_ARRIVED.load(std::sync::atomic::Ordering::Relaxed)
}
/// keyboard.c handle_user_signal: SIGUSR1 and SIGUSR2 are input, counted
/// here until read as `sigusr1' and `sigusr2' events, never the end of the
/// process. An isolate worker gives SIGUSR1 its own meaning afterwards
/// (`catch_isolate_signals').
static USER_SIGNALS: [std::sync::atomic::AtomicU32; 2] =
    [std::sync::atomic::AtomicU32::new(0), std::sync::atomic::AtomicU32::new(0)];
/// In the order store_user_signal_events reads them: GNU's list holds
/// SIGUSR2 first, the one registered last.
const USER_SIGNAL_EVENTS: [(libc::c_int, &str); 2] = [(libc::SIGUSR2, "sigusr2"), (libc::SIGUSR1, "sigusr1")];
/// A user signal arrived since a safepoint last looked: it may name
/// `debug-on-event', which GNU's handler acts on at once
/// (`Vm::take_debug_event').
static USER_SIGNAL_ARRIVED: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
extern "C" fn note_user_signal(signal: libc::c_int) {
    use std::sync::atomic::Ordering;
    let index = usize::from(signal == libc::SIGUSR1);
    USER_SIGNALS[index].fetch_add(1, Ordering::SeqCst);
    USER_SIGNAL_ARRIVED.store(true, Ordering::SeqCst);
    // A waiting session wakes to read it, as GNU's select returns on a
    // signal; write(2) is async-signal-safe, and the end is non-blocking.
    let fd = TERMINATION_PIPE.load(Ordering::SeqCst);
    if fd >= 0 { let _ = unsafe { libc::write(fd, [0u8].as_ptr().cast(), 1) }; }
}
/// Forget that a user signal arrived: the safepoints stop asking.
pub fn clear_user_signal_arrival() {
    USER_SIGNAL_ARRIVED.store(false, std::sync::atomic::Ordering::SeqCst);
}
/// The user signal events, in USER_SIGNAL_EVENTS's order: bit I of a mask
/// is the Ith.
pub const USER_SIGNAL_NAMES: [&str; 2] = [USER_SIGNAL_EVENTS[0].1, USER_SIGNAL_EVENTS[1].1];
/// Whether a user signal of MASK is pending: one relaxed load each, for a
/// native probe (a search, a process wait) to stop for a debug event.
#[inline]
pub fn user_signal_waiting(mask: u8) -> bool {
    USER_SIGNALS.iter().enumerate().any(|(i, count)| mask & 1 << i != 0 && count.load(std::sync::atomic::Ordering::Relaxed) != 0)
}
/// Take one pending user signal of each event in MASK: whether any was.
pub fn take_user_signals(mask: u8) -> bool {
    USER_SIGNALS.iter().enumerate().filter(|&(i, count)| {
        mask & 1 << i != 0 && count.fetch_update(std::sync::atomic::Ordering::SeqCst, std::sync::atomic::Ordering::SeqCst, |n| n.checked_sub(1)).is_ok()
    }).count() != 0
}
pub fn catch_user_signals() -> io::Result<()> {
    for (signal, _) in USER_SIGNAL_EVENTS {
        let mut action: libc::sigaction = unsafe { std::mem::zeroed() };
        action.sa_sigaction = note_user_signal as *const () as usize;
        action.sa_flags = libc::SA_RESTART;
        unsafe { libc::sigemptyset(&mut action.sa_mask) };
        status(unsafe { libc::sigaction(signal, &action, std::ptr::null_mut()) })?;
    }
    Ok(())
}
/// A pending user signal's event, SIGUSR2's first, taking it.
pub fn take_user_signal() -> Option<&'static str> {
    USER_SIGNALS.iter().zip(USER_SIGNAL_EVENTS).find_map(|(count, (_, event))| {
        count.fetch_update(std::sync::atomic::Ordering::SeqCst, std::sync::atomic::Ordering::SeqCst, |n| n.checked_sub(1))
            .ok().map(|_| event)
    })
}

/// Take the pending interrupt, if any.
pub fn take_interrupt() -> bool {
    INTERRUPT.swap(false, std::sync::atomic::Ordering::SeqCst)
}
pub fn terminate_pending() -> bool {
    TERMINATE.load(std::sync::atomic::Ordering::SeqCst)
}
/// A termination's one `quit`: true the first time after SIGTERM, so the
/// engine leaves the Lisp it runs once and the worker's loop can end.
static TERMINATE_QUIT: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
pub fn take_terminate_quit() -> bool {
    TERMINATE.load(std::sync::atomic::Ordering::SeqCst) && !TERMINATE_QUIT.swap(true, std::sync::atomic::Ordering::SeqCst)
}

/// A worker process: PROGRAM with ARGS after `--isolate 0`, SOCKET as its
/// standard input, this process's stderr. Its pid; the caller reaps.
/// Spawned, never forked, as a world builder is (`World::start_builder`):
/// no pre_exec, so no driver's atfork handler runs.
pub fn spawn_isolate(program: &Path, args: &[OsString], socket: OwnedFd) -> io::Result<i32> {
    let mut command = command(program)?;
    command.arg("--isolate").arg("0").args(args).stdin(socket);
    Ok(command.spawn()?.id() as i32)
}

/// emacs_pipe: a close-on-exec pipe (read end, write end).
pub fn pipe() -> io::Result<(OwnedFd, OwnedFd)> {
    effect(|| "pipe".into())?;
    let mut fds = [0; 2];
    status(unsafe { libc::pipe2(fds.as_mut_ptr(), libc::O_CLOEXEC) })?;
    Ok(unsafe { (OwnedFd::from_raw_fd(fds[0]), OwnedFd::from_raw_fd(fds[1])) })
}

/// The SIGTERM or SIGHUP noted and not yet taken, else 0.
static TERMINATION: std::sync::atomic::AtomicI32 = std::sync::atomic::AtomicI32::new(0);
/// The write end of the pipe the termination and user-signal handlers wake
/// `catch_termination_signals''s thread with.
static TERMINATION_PIPE: std::sync::atomic::AtomicI32 = std::sync::atomic::AtomicI32::new(-1);

extern "C" fn note_termination(signal: libc::c_int) {
    use std::sync::atomic::Ordering;
    TERMINATION.store(signal, Ordering::SeqCst);
    let fd = TERMINATION_PIPE.load(Ordering::SeqCst);
    // Only async-signal-safe calls here: the store and write(2).
    if fd >= 0 { let _ = unsafe { libc::write(fd, [0u8].as_ptr().cast(), 1) }; }
}

/// sysdep.c init_signals's SIGTERM and SIGHUP, for an embedder that ends
/// its session as emacs.c terminate_due_to_signal does, with `kill-emacs':
/// the handler notes the signal and a thread of its own calls WAKE, which
/// a handler may not. `termination_signal' takes it.
pub fn catch_termination_signals(wake: std::sync::Arc<dyn Fn() + Send + Sync>) -> io::Result<()> {
    use std::os::fd::IntoRawFd;
    let (reader, writer) = pipe()?;
    // The handlers never block: a full pipe already holds a wake. A new
    // pipe's end has no other status flag to keep.
    status(unsafe { libc::fcntl(std::os::fd::AsRawFd::as_raw_fd(&writer), libc::F_SETFL, libc::O_NONBLOCK) })?;
    TERMINATION_PIPE.store(writer.into_raw_fd(), std::sync::atomic::Ordering::SeqCst);
    std::thread::Builder::new().name("romacs-signals".into()).spawn(move || {
        let mut byte = 0u8;
        while unsafe { libc::read(std::os::fd::AsRawFd::as_raw_fd(&reader), (&raw mut byte).cast(), 1) } == 1 {
            wake();
        }
    })?;
    for signal in [libc::SIGTERM, libc::SIGHUP] {
        effect(|| format!("handle signal {signal}"))?;
        let mut action: libc::sigaction = unsafe { std::mem::zeroed() };
        action.sa_sigaction = note_termination as *const () as usize;
        action.sa_flags = libc::SA_RESTART;
        unsafe { libc::sigemptyset(&mut action.sa_mask) };
        status(unsafe { libc::sigaction(signal, &action, std::ptr::null_mut()) })?;
    }
    Ok(())
}

/// profiler.c's SIGPROF timer, as a thread that only counts: every
/// INTERVAL nanoseconds it looks at the calling thread, and when that
/// thread is running, adds one to TICKS. A late wake samples once: the
/// thread's state over the intervals it missed is unknown. The evaluator takes them at its next
/// call boundary, a moment later, under the stack that was running. A
/// sample is never counted for a thread that waits, so like GNU's
/// CLOCK_THREAD_CPUTIME_ID timer the samples measure the thread's CPU time,
/// and no wait is charged with work done before it. (A thread CPU-time
/// timer itself is not used: under a zero RLIMIT_RTTIME hard limit
/// Linux kills a thread whose CPU timer fires.) Dropping the sender ends
/// the thread.
pub fn cpu_sampler(interval: u64, ticks: std::sync::Arc<std::sync::atomic::AtomicU64>) -> io::Result<std::sync::mpsc::Sender<()>> {
    use std::os::unix::fs::FileExt;
    effect(|| "cpu profiler sampler".into())?;
    let stat = File::open("/proc/thread-self/stat")?;
    // stat's third field, after the parenthesized command name.
    let running = move || {
        let mut text = [0u8; 512];
        let n = stat.read_at(&mut text, 0).ok()?;
        let close = text[..n].iter().rposition(|&b| b == b')')?;
        Some(text.get(close + 2) == Some(&b'R'))
    };
    running().ok_or_else(|| io::Error::other("unreadable thread state"))?;
    let (stop, stopped) = std::sync::mpsc::channel::<()>();
    let period = std::time::Duration::from_nanos(interval.max(100_000));
    std::thread::Builder::new().name("romacs-profiler".into()).stack_size(64 * 1024).spawn(move || {
        let mut due = Instant::now() + period;
        loop {
            match stopped.recv_timeout(due.saturating_duration_since(Instant::now())) {
                Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {}
                _ => break,
            }
            due = next_sample(due, Instant::now(), period);
            // The profiled thread is gone: its state no longer reads.
            let Some(running) = running() else { break };
            if running {
                ticks.fetch_add((period.as_nanos() as u64 / interval).max(1), std::sync::atomic::Ordering::Relaxed);
            }
        }
    })?;
    Ok(stop)
}

/// The first sampling deadline after NOW, on DUE's period: a late wake
/// skips the deadlines it missed.
fn next_sample(due: Instant, now: Instant, period: std::time::Duration) -> Instant {
    let into = now.saturating_duration_since(due).as_nanos() % period.as_nanos();
    now.max(due) + period - std::time::Duration::from_nanos(into as u64)
}

/// The termination signal `catch_termination_signals' noted since the
/// last call, taken; `pending' only looks.
pub fn termination_signal(pending: bool) -> Option<i32> {
    use std::sync::atomic::Ordering;
    let signal = if pending { TERMINATION.load(Ordering::SeqCst) } else { TERMINATION.swap(0, Ordering::SeqCst) };
    (signal != 0).then_some(signal)
}

static MESSAGES_TO_STDERR: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(true);

/// Whether `message' also writes to stderr while `noninteractive' is nil.
/// GNU's xdisp.c message_to_stderr writes only in batch; a bare VM's tools
/// and test runners report through stderr, so that is the default, and an
/// interactive embedder (the GUI) turns it off: its echo area is no log.
pub fn set_messages_to_stderr(on: bool) {
    MESSAGES_TO_STDERR.store(on, std::sync::atomic::Ordering::Relaxed);
}

pub fn messages_to_stderr() -> bool {
    MESSAGES_TO_STDERR.load(std::sync::atomic::Ordering::Relaxed)
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

/// A listening local (Unix domain) stream socket bound to PATH.
pub fn listen_local(path: &Path) -> io::Result<std::os::unix::net::UnixListener> {
    effect(|| format!("listen {}", bytes(path)))?;
    std::os::unix::net::UnixListener::bind(path)
}

pub fn connect_local(path: &Path) -> io::Result<std::os::unix::net::UnixStream> {
    effect(|| format!("connect {}", bytes(path)))?;
    std::os::unix::net::UnixStream::connect(path)
}

/// connect_network_socket's nonblocking connect to the local socket PATH:
/// a close-on-exec stream socket, connected or (EINPROGRESS) connecting.
pub fn connect_local_nonblocking(path: &Path) -> io::Result<OwnedFd> {
    effect(|| format!("connect {}", bytes(path)))?;
    let mut sun: libc::sockaddr_un = unsafe { std::mem::zeroed() };
    sun.sun_family = libc::AF_UNIX as libc::sa_family_t;
    let name = path.as_os_str().as_bytes();
    if name.len() >= sun.sun_path.len() {
        return Err(io::Error::from_raw_os_error(libc::ENAMETOOLONG));
    }
    for (slot, &byte) in sun.sun_path.iter_mut().zip(name) {
        *slot = byte as libc::c_char;
    }
    let s = unsafe { libc::socket(libc::AF_UNIX, libc::SOCK_STREAM | libc::SOCK_NONBLOCK | libc::SOCK_CLOEXEC, 0) };
    if s < 0 {
        return Err(io::Error::last_os_error());
    }
    let socket = unsafe { OwnedFd::from_raw_fd(s) };
    let len = size_of::<libc::sockaddr_un>() as libc::socklen_t;
    if unsafe { libc::connect(s, (&sun as *const libc::sockaddr_un).cast(), len) } != 0 {
        let error = io::Error::last_os_error();
        if !matches!(error.raw_os_error(), Some(libc::EINPROGRESS | libc::EINTR)) {
            return Err(error);
        }
    }
    Ok(socket)
}

/// accept4 on the listening socket LISTENER: the connection, close-on-exec,
/// and the peer's raw address with its length.
pub fn accept(listener: RawFd) -> io::Result<(OwnedFd, libc::sockaddr_storage, libc::socklen_t)> {
    effect(|| format!("accept on {listener}"))?;
    let mut storage: libc::sockaddr_storage = unsafe { std::mem::zeroed() };
    let mut len = size_of::<libc::sockaddr_storage>() as libc::socklen_t;
    let sa = (&mut storage as *mut libc::sockaddr_storage).cast::<libc::sockaddr>();
    let fd = unsafe { libc::accept4(listener, sa, &mut len, libc::SOCK_CLOEXEC) };
    if fd < 0 {
        return Err(io::Error::last_os_error());
    }
    Ok((unsafe { OwnedFd::from_raw_fd(fd) }, storage, len))
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

    /// The build tree is a directory name with no `..` in it, so the
    /// runtime directories built from it are what they expand to.
    #[test]
    fn the_build_root_has_no_parent_steps() {
        let root = build_root();
        assert!(root.is_absolute(), "{}", root.display());
        assert!(!root.components().any(|c| matches!(c, std::path::Component::ParentDir)), "{}", root.display());
        assert!(root.join("gnu-lisp").is_dir() && root.join("etc").is_dir(), "{}", root.display());
    }
    /// A late wake samples once, at the next deadline after it.
    #[test]
    fn a_late_sampler_wake_skips_the_deadlines_it_missed() {
        let ms = std::time::Duration::from_millis(1);
        let due = super::Instant::now();
        assert_eq!(super::next_sample(due, due, ms), due + ms);
        assert_eq!(super::next_sample(due, due + ms * 150 + ms / 2, ms), due + ms * 151);
    }
    /// A build's walk of the account database replays as the same walk.
    #[test]
    fn account_name_walks_replay() {
        for (groups, key) in [(false, "users"), (true, "groups")] {
            enter_build(Pinned { clock: SystemTime::UNIX_EPOCH, pid: 0, seed: [0; 4] });
            begin_recipe();
            account_names(groups);
            let trace = leave_build().expect("a build trace");
            let record = trace["records"].as_array().unwrap().iter()
                .find(|r| r["kind"] == "user-db" && r["key"] == key).expect("the walk is recorded").clone();
            let mut answer = replay(&record).expect("the walk replays");
            let mut recorded = record;
            recorded.as_object_mut().unwrap().remove("site");
            answer.as_object_mut().unwrap().remove("site");
            assert_eq!(answer["result"], recorded["result"], "{key}");
        }
    }
    use super::*;

    /// An unpacked release finds its runtime tree beside its `bin/`; a
    /// development binary under `target/` has none there.
    #[test]
    fn installed_root_is_share_romacs_beside_bin() {
        let prefix = std::env::temp_dir().join(format!("romacs-installed-root-{}", std::process::id()));
        let exe = prefix.join("bin/romacs-vm-gui");
        assert_eq!(installed_root(&exe), None);
        std::fs::create_dir_all(prefix.join("share/romacs/gnu-lisp")).unwrap();
        assert_eq!(installed_root(&exe), Some(prefix.join("share/romacs")));
        std::fs::remove_dir_all(&prefix).unwrap();
    }

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

/// A deadline on one call the host admits (quit.rs `DEADLINE') (an agent's evaluation): out of
/// band, as the host's C-g, since a Lisp timer cannot reach a CPU loop.
/// One thread waits for the armed deadline; at expiry it marks the shared
/// word (`DEADLINE') with the call's generation recorded, whatever the
/// evaluator's state. The evaluator takes it at a safepoint while busy, as
/// a quit, only while that generation is still the current call: a call
/// that ended first (`disarm') retired its generation, so its deadline can
/// never reach a later command. A deadline that passes while the call waits
/// for input is kept until the call runs again. Cooperative like C-g:
/// `inhibit-quit', a long primitive or a compile defer it.
pub mod call_deadline {
    use crate::QuitSignal;
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::sync::{Arc, Condvar, Mutex, OnceLock};
    use std::time::{Duration, Instant};

    /// The current call's generation, 0 for none, and the one whose
    /// deadline passed.
    static CURRENT: AtomicU64 = AtomicU64::new(0);
    static EXPIRED: AtomicU64 = AtomicU64::new(0);

    struct Armed { generation: u64, at: Instant, signal: Arc<QuitSignal> }
    struct Waiter { armed: Mutex<Option<Armed>>, changed: Condvar }

    static WAITER: OnceLock<&'static Waiter> = OnceLock::new();

    fn waiter() -> &'static Waiter {
        WAITER.get_or_init(|| {
            let waiter: &'static Waiter = Box::leak(Box::new(Waiter { armed: Mutex::new(None), changed: Condvar::new() }));
            std::thread::Builder::new().name("call-deadline".into()).spawn(move || run(waiter)).expect("deadline thread");
            waiter
        })
    }

    fn run(waiter: &'static Waiter) {
        let mut armed = waiter.armed.lock().unwrap_or_else(|e| e.into_inner());
        loop {
            armed = match armed.as_ref().map(|a| a.at.saturating_duration_since(Instant::now())) {
                None => waiter.changed.wait(armed).unwrap_or_else(|e| e.into_inner()),
                Some(left) if left > Duration::ZERO => waiter.changed.wait_timeout(armed, left).unwrap_or_else(|e| e.into_inner()).0,
                Some(_) => {
                    let Armed { generation, signal, .. } = armed.take().expect("armed");
                    EXPIRED.store(generation, Ordering::Release);
                    signal.set_deadline(true);
                    armed
                }
            };
        }
    }

    /// Call GENERATION (nonzero) starts with BUDGET before its quit.
    pub fn arm(signal: Arc<QuitSignal>, generation: u64, budget: Duration) {
        CURRENT.store(generation, Ordering::Release);
        let waiter = waiter();
        *waiter.armed.lock().unwrap_or_else(|e| e.into_inner()) = Some(Armed { generation, at: Instant::now() + budget, signal });
        waiter.changed.notify_one();
    }

    /// The call ended: its deadline, passed or not, reaches nothing more.
    pub fn disarm(signal: &QuitSignal) {
        CURRENT.store(0, Ordering::Release);
        if let Some(waiter) = WAITER.get() {
            *waiter.armed.lock().unwrap_or_else(|e| e.into_inner()) = None;
            waiter.changed.notify_one();
        }
        signal.set_deadline(false);
    }

    pub(crate) fn expired_is_current() -> bool {
        let current = CURRENT.load(Ordering::Acquire);
        current != 0 && EXPIRED.load(Ordering::Acquire) == current
    }
}
