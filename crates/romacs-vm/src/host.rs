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
use std::os::fd::{AsFd, OwnedFd, RawFd};
use std::os::unix::ffi::OsStrExt;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::rc::Rc;
use std::time::{Instant, SystemTime};

pub use std::fs::OpenOptions;
pub use sys::Passwd;

mod sys;

/// rustix's error, as std's.
fn os(error: rustix::io::Errno) -> io::Error {
    error.into()
}

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
        pinned: Pinned { clock: SystemTime::UNIX_EPOCH, pid: 0, seed: [0; 4] }, phase: Phase::Recipe, startups: 0, dump: false,
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
    /// The machine's name: keyed, except where GNU bakes it as the build
    /// host (`BAKED_HOST`).
    SystemName,
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
/// The baked site of the machine's name: `emacs-build-system` is the host
/// the world was built on, as a dumped GNU's is its dump's.
const BAKED_HOST: (&str, &str) = ("/version.el", "(defconst emacs-build-system");
/// jit-lock.el's default for `jit-lock-stealth-load' only asks whether
/// `load-average' works, and a dumped GNU keeps the dump's answer.
const BAKED_LOAD: (&str, &str) = ("/jit-lock.el", "(defcustom jit-lock-stealth-load");

/// Load-time forms outside startup that GNU evaluates in each process and
/// restore re-seeds (`romacs--world-reseed`): (file suffix, form head, the P
/// fact or fenced variable the form may read, the variable it sets, which is
/// fenced once the form ends).
const RESEED: [(&str, &str, &str, Option<&str>); 6] = [
    ("/emacs-lisp/cl-extra.el", "(defvar cl--random-state", "clock", Some("cl--random-state")),
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

/// Whether the form being loaded is the baked build host's (`BAKED_HOST`).
pub(crate) fn at_baked_host() -> bool {
    with_build(|b| b.site.as_deref().is_some_and(|s| s.at(BAKED_HOST))).unwrap_or(false)
}

/// The variable the form being loaded re-seeds, if it is a re-seed entry.
pub(crate) fn reseed_output() -> Option<&'static str> {
    reseed_entry().and_then(|(_, output)| output)
}

/// The sites that may switch the startup phase, in order: the chain's batch
/// steps, then an interactive session's, which supersede them.
const STARTUP: [&str; 2] = ["/lisp/romacs/romacs-loadup-bootstrap.el", "/lisp/romacs/romacs-gui-loadup.el"];

pub fn class(fact: Fact<'_>, site: Option<&Site>) -> Class {
    match fact {
        Fact::Env(name) => if P_ENV.iter().any(|p| p.as_bytes() == name) { Class::P } else { Class::K },
        Fact::Clock if site.is_some_and(|s| s.at(BAKED)) => Class::B,
        Fact::Machine if site.is_some_and(|s| s.at(BAKED_LOAD)) => Class::B,
        Fact::SystemName if site.is_some_and(|s| s.at(BAKED_HOST)) => Class::B,
        Fact::Environment | Fact::CurrentDir | Fact::Pid | Fact::Clock | Fact::Monotonic | Fact::Seed | Fact::Processors | Fact::Machine => Class::P,
        Fact::SystemName | Fact::Other => Class::K,
        Fact::Effect => Class::F,
    }
}

struct Build {
    pinned: Pinned,
    phase: Phase,
    startups: usize,
    /// The build ends at GNU's dump point (`end_at_dump_point`).
    dump: bool,
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
        pinned, phase: Phase::Init, startups: 0, dump: false, site: None,
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

/// This build ends at GNU's dump point, as loadup.el does under
/// `dump-mode`: the chain leaves its startup steps to each restored
/// process (`romacs--dumping`). A no-op outside a build.
pub fn end_at_dump_point() {
    with_build(|b| b.dump = true);
}

/// Whether this process builds a world that ends at GNU's dump point.
pub(crate) fn dumping() -> bool {
    refusing() && with_build(|b| b.dump).unwrap_or(false)
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

/// An effect on PATH succeeded: while a flight runs, PATH's times are the
/// kernel's clock's, and so are its directory's when ENTRIES says the
/// effect added, removed or renamed an entry there (`crate::flight::stat`).
/// A failed effect changes nothing and marks nothing.
fn changed<T>(result: &io::Result<T>, path: &Path, entries: bool) {
    if result.is_err() {
        return;
    }
    if let Some(path) = flight_path(path) {
        if entries && let Some(parent) = path.parent() { crate::flight::wrote(parent.to_owned()); }
        crate::flight::wrote(path);
    }
}

/// Whether PATH names nothing yet, asked only while a flight runs: an
/// effect that creates it changes its directory's entries.
fn absent(path: &Path) -> bool {
    crate::flight::active() && std::fs::symlink_metadata(path).is_err()
}

/// PATH made absolute against the working directory, while a flight runs:
/// the form `wrote` keeps, for `crate::flight::stat`.
fn flight_path(path: &Path) -> Option<PathBuf> {
    crate::flight::active().then(|| if path.is_relative() { std::env::current_dir().map(|d| d.join(path)).unwrap_or_else(|_| path.to_owned()) } else { path.to_owned() })
}

/// PATH's modification and access times set to SECONDS and NANOS, a
/// symlink's own when not FOLLOW: a flight replay's retime of a file it
/// wrote (`crate::flight::stat`).
fn retime(path: &Path, seconds: i64, nanos: i64, follow: bool) -> io::Result<()> {
    let name = path_c_string(path)?;
    utimensat(&name, rustix::fs::Timespec { tv_sec: seconds, tv_nsec: nanos }, !follow)
}

/// utimensat: both of PATH's times set to STAMP, a symlink's own under
/// NOFOLLOW.
fn utimensat(path: &CStr, stamp: rustix::fs::Timespec, nofollow: bool) -> io::Result<()> {
    let flags = if nofollow { rustix::fs::AtFlags::SYMLINK_NOFOLLOW } else { rustix::fs::AtFlags::empty() };
    let times = rustix::fs::Timestamps { last_access: stamp, last_modification: stamp };
    rustix::fs::utimensat(rustix::fs::CWD, path, &times, flags).map_err(os)
}

/// FILE's modification time set to SECONDS and NANOS, as `retime` does.
fn retime_file(file: &File, seconds: i64, nanos: i64) -> io::Result<()> {
    let at = SystemTime::UNIX_EPOCH.checked_add(std::time::Duration::new(seconds.max(0) as u64, nanos.clamp(0, 999_999_999) as u32))
        .ok_or_else(|| io::Error::from_raw_os_error(libc::EINVAL))?;
    file.set_modified(at)
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
    let value = crate::flight::checked("getenv", || name.to_owned(), || std::env::var_os(name), |v| v.as_ref().map_or(u64::MAX, |v| crate::flight::digest(v.as_bytes())));
    if building() {
        env_read("host", name.as_bytes(), value.as_ref());
    }
    value
}

/// The generation of runtime tree this executable was built for: the tree's own
/// `etc/romacs-runtime-epoch`, read when the executable was compiled. It is
/// raised when the tree's layout or its contract with the executable changes
/// so that one cannot run on the other.
const RUNTIME_EPOCH: &str = include_str!("../../../etc/romacs-runtime-epoch");

/// The file beside an executable that names its runtime tree, for one that
/// was copied out of its build: one line, a directory, relative to the file's
/// own directory or absolute.
pub const RUNTIME_RECORD: &str = "romacs-runtime-root";

/// The runtime tree (lisp/gnu/, lisp/romacs/, etc/), found from where the
/// executable is, never from where it was once built:
///   1. `ROMACS_RUNTIME_ROOT`, said outright (the Nix wrapper sets it);
///   2. `share/romacs` beside the executable's `bin/` (an installation, or an
///      unpacked release, wherever it lives), or the executable's own
///      directory when the tree was laid out around it (a test install);
///   3. the tree [`RUNTIME_RECORD`] beside the executable names (a copy of a
///      development build);
///   4. the tree it was built in, while it still sits in that build's Cargo
///      target directory.
///
/// The tree found must be one of this executable's generation. With none
/// found, or a wrong one, the process says which and why and ends: loading
/// another tree's Lisp and data into this executable fails later, and
/// far from the cause.
///
/// It is found once, the first time it is asked for, and is the session's
/// from then on: a target directory cleaned or a variable changed under a
/// running session moves nothing and ends nothing. A build that records what
/// it reads has the session's tree in its record, read again as it is now:
/// the variable that may have chosen it and its generation. A tree that has
/// stopped being this executable's refuses that build; the session goes on.
pub fn runtime_root() -> PathBuf {
    static FOUND: std::sync::OnceLock<PathBuf> = std::sync::OnceLock::new();
    let root = FOUND.get_or_init(|| located_runtime().unwrap_or_else(|why| {
        eprintln!("romacs: no usable runtime tree: {why}\n  \
                   Set ROMACS_RUNTIME_ROOT to a Romacs tree (the directory holding lisp/ and etc/), \
                   or name one in a file `{RUNTIME_RECORD}' beside the executable.");
        std::process::exit(2)
    }));
    if building() {
        let _ = getenv("ROMACS_RUNTIME_ROOT");
        if let Err(why) = runtime_tree(root) {
            refuse("runtime-tree", json!({"path": path_json(root), "why": why}));
        }
    }
    root.clone()
}

/// [`runtime_root`], or why there is none.
fn located_runtime() -> Result<PathBuf, String> {
    let exe = current_exe().ok();
    let (root, from) = if let Some(dir) = getenv("ROMACS_RUNTIME_ROOT") {
        (PathBuf::from(dir), "ROMACS_RUNTIME_ROOT")
    } else if let Some(root) = exe.as_deref().and_then(installed_root) {
        (root, "share/romacs beside the executable")
    } else if let Some(root) = exe.as_deref().and_then(Path::parent).filter(|dir| probe(Probe::IsDir, dir.join("lisp/gnu"))) {
        (root.to_path_buf(), "the executable's own directory")
    } else if let Some(root) = exe.as_deref().and_then(recorded_root) {
        (root, "named beside the executable")
    } else if exe.as_deref().is_some_and(|exe| in_build_target(exe, &build_root())) {
        (build_root(), "the tree this executable was built in")
    } else {
        let exe = exe.map_or_else(|| "this executable".to_owned(), |exe| exe.display().to_string());
        return Err(format!("{exe} is neither installed (no share/romacs beside its bin/, no tree around it), nor in its build's target directory, and names no tree"));
    };
    runtime_tree(&root).map_err(|why| format!("{} ({from}) {why}", root.display()))?;
    Ok(root)
}

/// `PREFIX/share/romacs` for an executable `PREFIX/bin/NAME`, when it holds a
/// `lisp/gnu/` directory.
pub(crate) fn installed_root(exe: &Path) -> Option<PathBuf> {
    let root = exe.parent()?.parent()?.join("share/romacs");
    probe(Probe::IsDir, root.join("lisp/gnu")).then_some(root)
}

/// The tree [`RUNTIME_RECORD`] beside EXE names, when there is such a file.
fn recorded_root(exe: &Path) -> Option<PathBuf> {
    let dir = exe.parent()?;
    let text = read_to_string(dir.join(RUNTIME_RECORD)).ok()?;
    let named = text.lines().next().map(str::trim).filter(|line| !line.is_empty())?;
    let tree = dir.join(named);          // an absolute name replaces DIR
    // The directory's own name, as file names under it are compared with it.
    Some(canonicalize(&tree).unwrap_or(tree))
}

/// The file in a Cargo profile directory (`target/PROFILE`) that lists the
/// trees built into it, one a line: this crate's build script adds its own.
pub const BUILD_TREES: &str = "romacs-build-trees";

/// Whether EXE is in a Cargo profile directory (or its `deps` or `examples`)
/// that TREE was built into, by that directory's own list. An executable
/// carried into another build's target directory finds a list without its
/// tree, and one carried anywhere else finds none: it has left its build,
/// and the tree compiled into it is no longer its own to assume.
///
/// Decided, not missed: the list says which checkouts built into a
/// directory, not which directory built this executable. One carried into
/// a pooled target that also built the same checkout path is taken for that
/// directory's own, and assumes that same path. Naming the build's directory
/// in the executable would close that, and would make one tree build a
/// different executable in every target directory; checks key their results
/// on the executable, so that was the dearer loss. What a checkout holds
/// after a build is not this rule's to judge either: Lisp edited under a
/// build is meant to be picked up.
fn in_build_target(exe: &Path, tree: &Path) -> bool {
    let Some(dir) = exe.parent() else { return false };
    let profile = match dir.file_name().and_then(OsStr::to_str) {
        Some("deps" | "examples") => dir.parent().unwrap_or(dir),
        _ => dir,
    };
    read_to_string(profile.join(BUILD_TREES)).is_ok_and(|built| built.lines().any(|line| Path::new(line) == tree))
}

/// Whether ROOT is a runtime tree of this executable's generation, or why not.
fn runtime_tree(root: &Path) -> Result<(), String> {
    if !probe(Probe::IsDir, root.join("lisp/gnu")) || !probe(Probe::IsDir, root.join("etc")) {
        return Err("is not a Romacs runtime tree (it has no lisp/gnu or no etc)".into());
    }
    let wanted = RUNTIME_EPOCH.trim();
    match read_to_string(root.join("etc/romacs-runtime-epoch")) {
        Ok(found) if found.trim() == wanted => Ok(()),
        Ok(found) => Err(format!("is a runtime tree of generation {}, and this executable was built for generation {wanted}", found.trim())),
        Err(_) => Err(format!("is a runtime tree from before generations (no etc/romacs-runtime-epoch), and this executable was built for generation {wanted}")),
    }
}

/// The checkout this crate was built from: a compile-time path that no
/// installed system has, kept for development binaries under `target/`.
/// It is the manifest directory's grandparent itself, not `DIR/../..`:
/// `data-directory' and the other runtime directories are directory names
/// as GNU's are, so file names built from them compare equal to their
/// expansions (find-func's library completion, bug#43393's test).
/// The Romacs source tree this build came from, `source-directory':
/// `ROMACS_SOURCE_DIRECTORY` when set (none if it names no tree), else the
/// build's own root while it is a git checkout of one, else none.  A gate's
/// build slot mirrors a checkout without `.git`, so a build made there
/// names no tree it would not find again.  Both checks are kinds of entry,
/// not stats: a world built while the tree is edited stays valid when a
/// file under it is added or renamed, or a commit touches `.git`.
pub fn source_directory() -> Option<PathBuf> {
    let tree = |dir: &Path| probe(Probe::IsDir, dir.join("crates/romacs-vm/src/prims"));
    if let Some(dir) = getenv("ROMACS_SOURCE_DIRECTORY").filter(|d| !d.is_empty()).map(PathBuf::from) {
        // Absolute, so a buffer's own directory never resolves it.
        return std::path::absolute(&dir).ok().filter(|dir| tree(dir));
    }
    let root = build_root();
    (tree(&root) && entry_type(&root.join(".git")).is_ok()).then_some(root)
}

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
    crate::flight::checked("environment", String::new, || std::env::vars_os().collect(), |vars: &Vec<(OsString, OsString)>| {
        vars.iter().fold(0, |h, (k, v)| h.rotate_left(7) ^ crate::flight::digest(k.as_bytes()) ^ crate::flight::digest(v.as_bytes()).rotate_left(1))
    })
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
    let result = crate::flight::checked("read-file", || bytes(path.as_ref()), || std::fs::read(&path), |r| crate::flight::outcome(r, |b| crate::flight::digest(b)));
    if building() {
        file_read(path.as_ref(), &result);
    }
    result
}

pub fn read_to_string(path: impl AsRef<Path>) -> io::Result<String> {
    let result = crate::flight::checked("read-file", || bytes(path.as_ref()), || std::fs::read_to_string(&path), |r| crate::flight::outcome(r, |b| crate::flight::digest(b.as_bytes())));
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
    let observed = crate::flight::checked("probe", || bytes(path), || match kind {
        Probe::Exists => path.exists(),
        Probe::IsFile => path.is_file(),
        Probe::IsDir => path.is_dir(),
    }, |&b| u64::from(b));
    if building() {
        probed(&format!("{kind:?}"), path, json!(observed));
    }
    observed
}

pub fn metadata(path: impl AsRef<Path>) -> io::Result<Metadata> {
    let absolute = flight_path(path.as_ref());
    let result = crate::flight::stat("stat", || bytes(path.as_ref()), absolute.as_deref(), false, || std::fs::metadata(&path),
                                     |s, ns| retime(path.as_ref(), s, ns, true));
    if building() {
        probed("stat", path.as_ref(), stat_json(&result));
    }
    result
}

pub fn symlink_metadata(path: impl AsRef<Path>) -> io::Result<Metadata> {
    let absolute = flight_path(path.as_ref());
    let result = crate::flight::stat("lstat", || bytes(path.as_ref()), absolute.as_deref(), false, || std::fs::symlink_metadata(&path),
                                     |s, ns| retime(path.as_ref(), s, ns, false));
    if building() {
        probed("lstat", path.as_ref(), stat_json(&result));
    }
    result
}

/// fstat on an output file this module opened.
pub fn file_metadata(file: &File) -> io::Result<Metadata> {
    crate::flight::stat("fstat", String::new, None, true, || file.metadata(), |s, ns| retime_file(file, s, ns))
}

/// A file opened for reading. Its reads and queries go through this module,
/// so a recorder sees the bytes a caller consumes and each error; the
/// descriptor never leaves it. In build mode it keeps its path, and opening
/// a regular file reads its whole content from the descriptor once: that is
/// what the trace records and what every later read is served from.
pub struct Input(File, Option<PathBuf>, Option<Captured>, Source);

/// Where an input's bytes come from: a file, which a replay reads again,
/// or entropy, which a replay takes from its log (`crate::flight`). A
/// file's path is kept while a flight runs, to name it in a divergence.
#[derive(PartialEq, Eq)]
enum Source {
    File(Option<PathBuf>),
    Entropy,
}

impl Source {
    fn file(path: &Path) -> Source {
        Source::File(flight_path(path))
    }

    /// What a flight's divergence names this input by.
    fn what(&self) -> String {
        match self {
            Source::File(path) => path.as_deref().map(bytes).unwrap_or_default(),
            Source::Entropy => "entropy".into(),
        }
    }
}

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
    let result = crate::flight::checked("open", || bytes(path.as_ref()), || OpenOptions::new().read(true).custom_flags(flags).open(&path), |r| crate::flight::outcome(r, |_| 0));
    if !building() {
        return result.map(|file| Input(file, None, None, Source::file(path.as_ref())));
    }
    let path = path.as_ref();
    match result {
        // Only a regular file's content is a fact; a FIFO's read would block.
        Ok(file) if file.metadata().is_ok_and(|m| !m.is_file()) => {
            probed("open", path, json!("special"));
            Ok(Input(file, Some(path.to_owned()), None, Source::file(path)))
        }
        Ok(file) => {
            let mut content = Vec::new();
            let read = io::Read::read_to_end(&mut &file, &mut content).map(|_| content);
            file_read(path, &read);
            let content = read.map_err(|e| e.raw_os_error().unwrap_or(libc::EIO));
            Ok(Input(file, Some(path.to_owned()), Some(Captured { content, at: 0 }), Source::file(path)))
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
    File::open("/dev/urandom").map(|file| Input(file, None, None, Source::Entropy))
}

impl Input {
    /// fstat.
    pub fn stat(&self) -> io::Result<Metadata> {
        let path = match &self.3 { Source::File(path) => path.as_deref(), Source::Entropy => None };
        let result = crate::flight::stat("fstat", || self.3.what(), path, false, || self.0.metadata(),
                                         |s, ns| path.map_or(Err(io::Error::from_raw_os_error(libc::EBADF)), |path| retime(path, s, ns, true)));
        if let Some(path) = &self.1 {
            probed("fstat", path, stat_json(&result));
        }
        result
    }

    /// At most LIMIT bytes from OFFSET: a seek, then reads until LIMIT or the
    /// end. An error from either discards what was read.
    pub fn read_range(&mut self, offset: u64, limit: u64) -> io::Result<Vec<u8>> {
        let read = self.live_read_range(offset, limit);
        crate::flight::checked("read-range", || self.3.what(), || (), |()| crate::flight::outcome(&read, |b| crate::flight::digest(b)));
        read
    }

    fn live_read_range(&mut self, offset: u64, limit: u64) -> io::Result<Vec<u8>> {
        use std::io::{Read, Seek, SeekFrom};
        if let Some(captured) = &mut self.2 {
            captured.at = usize::try_from(offset).unwrap_or(usize::MAX);
            return captured.take(usize::try_from(limit).unwrap_or(usize::MAX)).map(<[u8]>::to_vec);
        }
        self.0.seek(SeekFrom::Start(offset))?;
        // Sized for the file: growing by doubling copies it and faults its
        // pages in again.
        let size = self.0.metadata().map_or(0, |m| m.len()).saturating_sub(offset).min(limit);
        let mut bytes = Vec::with_capacity(usize::try_from(size).unwrap_or(0));
        (&mut self.0).take(limit).read_to_end(&mut bytes)?;
        Ok(bytes)
    }

    /// One read into BUFFER: the count, 0 at the end.
    pub fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
        if self.3 == Source::Entropy && crate::flight::active() {
            let bytes = crate::flight::value("entropy", || {
                let mut bytes = vec![0u8; buffer.len()];
                io::Read::read(&mut self.0, &mut bytes).map(|n| { bytes.truncate(n); crate::flight::Bytes(bytes) })
            })?;
            let n = bytes.0.len().min(buffer.len());
            buffer[..n].copy_from_slice(&bytes.0[..n]);
            return Ok(n);
        }
        let read = self.live_read(buffer);
        crate::flight::checked("read", || self.3.what(), || (), |()| crate::flight::outcome(&read, |&n| crate::flight::digest(&buffer[..n])));
        read
    }

    fn live_read(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
        let Some(captured) = &mut self.2 else { return io::Read::read(&mut self.0, buffer) };
        let bytes = captured.take(buffer.len())?;
        buffer[..bytes.len()].copy_from_slice(bytes);
        Ok(bytes.len())
    }

    pub fn read_exact(&mut self, buffer: &mut [u8]) -> io::Result<()> {
        if self.3 == Source::Entropy && crate::flight::active() {
            let bytes = crate::flight::value("entropy", || {
                let mut bytes = vec![0u8; buffer.len()];
                io::Read::read_exact(&mut self.0, &mut bytes).map(|()| crate::flight::Bytes(bytes))
            })?;
            if bytes.0.len() != buffer.len() { return Err(io::ErrorKind::UnexpectedEof.into()); }
            buffer.copy_from_slice(&bytes.0);
            return Ok(());
        }
        let read = self.live_read_exact(buffer);
        crate::flight::checked("read-exact", || self.3.what(), || (), |()| crate::flight::outcome(&read, |()| crate::flight::digest(buffer)));
        read
    }

    fn live_read_exact(&mut self, buffer: &mut [u8]) -> io::Result<()> {
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
        crate::flight::checked("has-acl", || self.3.what(), || [c"system.posix_acl_access", c"system.nfs4_acl"].iter().any(|attribute| {
            rustix::fs::fgetxattr(&self.0, *attribute, &mut [0u8; 0][..]).is_ok()
        }), |&b| u64::from(b))
    }

    /// close(2), with its error.
    pub fn close(self) -> io::Result<()> {
        sys::close_owned(self.0.into())
    }
}

/// A file a child process reads as its standard input: this process reads
/// none of it.
pub fn open_child_input(path: impl AsRef<Path>) -> io::Result<OwnedFd> {
    effect(|| format!("child input {}", bytes(path.as_ref())))?;
    File::open(path).map(OwnedFd::from)
}

pub fn read_link(path: impl AsRef<Path>) -> io::Result<PathBuf> {
    let result = crate::flight::checked("readlink", || bytes(path.as_ref()), || std::fs::read_link(&path), |r| crate::flight::outcome(r, |p| crate::flight::digest(p.as_os_str().as_bytes())));
    if building() {
        let observed = match &result { Ok(target) => path_json(target), Err(e) => errno(e) };
        probed("readlink", path.as_ref(), observed);
    }
    result
}

/// The entries of PATH, as the host yields them. In build mode the listing
/// records the names in enumeration order and how it ended (§6.3 DirList).
pub fn read_dir(path: impl AsRef<Path>) -> io::Result<ReadDir> {
    let result = crate::flight::checked("opendir", || bytes(path.as_ref()), || std::fs::read_dir(&path), |r| crate::flight::outcome(r, |_| 0));
    let named = flight_path(path.as_ref());
    if !building() {
        return result.map(|entries| ReadDir(entries, None, named));
    }
    let path = path.as_ref();
    match result {
        Ok(entries) if keyed_path(path, "listing") => Ok(ReadDir(entries, Some(Listing { path: path_json(path), names: Vec::new(), end: None }), named)),
        Ok(entries) => Ok(ReadDir(entries, None, named)),
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
pub struct ReadDir(std::fs::ReadDir, Option<Listing>, Option<PathBuf>);

/// A listing as far as it was read: `end` is None while it may continue.
struct Listing {
    path: Json,
    names: Vec<Json>,
    end: Option<Json>,
}

impl Iterator for ReadDir {
    type Item = io::Result<DirEntry>;
    fn next(&mut self) -> Option<Self::Item> {
        let listing = &self.2;
        let next = crate::flight::checked("readdir", || listing.as_deref().map(bytes).unwrap_or_default(), || self.0.next(), |next| match next {
            Some(Ok(entry)) => crate::flight::digest(entry.file_name().as_bytes()),
            Some(Err(e)) => 0xe000_0000_0000_0000 | e.raw_os_error().unwrap_or(-1) as u32 as u64,
            None => u64::MAX,
        });
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
        let result = crate::flight::checked("entry-type", || bytes(&self.0.path()), || self.0.file_type(), |r| crate::flight::outcome(r, file_type_witness));
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
    let result = crate::flight::checked("entry-type", || bytes(path), || std::fs::symlink_metadata(path).map(|m| m.file_type()), |r| crate::flight::outcome(r, file_type_witness));
    if building() {
        probed("entry-type", path, entry_type_json(&result));
    }
    result
}

/// A file type, for a flight log.
fn file_type_witness(t: &FileType) -> u64 {
    if t.is_dir() { 1 } else if t.is_file() { 2 } else if t.is_symlink() { 3 } else { 4 }
}

fn entry_type_json(result: &io::Result<FileType>) -> Json {
    match result {
        Ok(t) => json!(if t.is_dir() { "dir" } else if t.is_file() { "file" } else if t.is_symlink() { "symlink" } else { "other" }),
        Err(e) => errno(e),
    }
}

pub fn canonicalize(path: impl AsRef<Path>) -> io::Result<PathBuf> {
    let result = crate::flight::checked("canonicalize", || bytes(path.as_ref()), || std::fs::canonicalize(&path), |r| crate::flight::outcome(r, |p| crate::flight::digest(p.as_os_str().as_bytes())));
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
    crate::flight::checked("absolute", || bytes(path.as_ref()), || std::path::absolute(&path), |r| crate::flight::outcome(r, |p| crate::flight::digest(p.as_os_str().as_bytes())))
}

/// Whose ids an access check uses.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Ids {
    Effective,
    Real,
}

/// faccessat: whether IDS may access PATH in MODE.
pub fn access(path: &CStr, mode: i32, ids: Ids) -> io::Result<()> {
    let flags = match ids { Ids::Effective => rustix::fs::AtFlags::EACCESS, Ids::Real => rustix::fs::AtFlags::empty() };
    let access = || rustix::fs::accessat(rustix::fs::CWD, path, rustix::fs::Access::from_bits_retain(mode as u32), flags).map_err(os);
    let result = crate::flight::checked("access", || String::from_utf8_lossy(path.to_bytes()).into_owned(), access, |r| crate::flight::outcome(r, |()| 0));
    if building() {
        let observed = match &result { Ok(()) => json!("ok"), Err(e) => errno(e) };
        probed(&format!("access {mode} {ids:?}"), Path::new(OsStr::from_bytes(path.to_bytes())), observed);
    }
    result
}

/// A filesystem's size and free space (statvfs(3)), in fragments of
/// `fragment` bytes. `available` is the kernel's word as is: GNU reads it
/// signed, so a filesystem reporting a negative count reports one.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FsUsage {
    pub block: u64,
    pub fragment: u64,
    pub blocks: u64,
    pub free: u64,
    pub available: u64,
}

pub fn statvfs(path: &CStr) -> io::Result<FsUsage> {
    let result = crate::flight::value("statvfs", || {
        rustix::fs::statvfs(path).map_err(os)
            .map(|u| FsUsage { block: u.f_bsize, fragment: u.f_frsize, blocks: u.f_blocks, free: u.f_bfree, available: u.f_bavail })
    });
    if building() {
        let observed = match &result {
            Ok(u) => json!([u.block, u.blocks, u.free, u.available]),
            Err(e) => errno(e),
        };
        probed("statvfs", Path::new(OsStr::from_bytes(path.to_bytes())), observed);
    }
    result
}

/// The type of the filesystem holding PATH: statfs's `f_type`, a magic
/// number of linux/magic.h. A keyed probe.
pub fn filesystem_type(path: &Path) -> io::Result<u32> {
    // f_type is a signed word; the magic numbers are 32 bits.
    let result = crate::flight::checked("fs-type", || bytes(path), || path_c_string(path).and_then(|name| rustix::fs::statfs(&*name).map_err(os).map(|s| s.f_type as u32)),
                                        |r| crate::flight::outcome(r, |&t| u64::from(t)));
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
    let result = crate::flight::checked("dir-flags", || bytes(directory), || result, |r| crate::flight::outcome(r, |&f| u64::from(f)));
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
        let name = path_c_string(path)?;
        loop {
            let size = match rustix::fs::getxattr(&*name, NAME, &mut [0u8; 0][..]) {
                Ok(size) => size,
                Err(rustix::io::Errno::NODATA) => return Ok(None),
                Err(error) => return Err(os(error)),
            };
            let mut value = vec![0u8; size];
            match rustix::fs::getxattr(&*name, NAME, &mut value[..]) {
                Ok(read) => { value.truncate(read); return Ok(Some(value)); }
                // The attribute grew between the two calls: ask its size again.
                Err(rustix::io::Errno::RANGE) => {}
                Err(error) => return Err(os(error)),
            }
        }
    })();
    if building() {
        if let Err(e) = &result { confined(path, e); }
        let observed = match &result { Ok(Some(value)) => encoded(value), Ok(None) => json!(null), Err(e) => errno(e) };
        probed("acl", path, observed);
    }
    crate::flight::checked("acl", || bytes(path), || result, |r| crate::flight::outcome(r, |acl| acl.as_deref().map_or(u64::MAX, crate::flight::digest)))
}

fn read_directory_flags(directory: &Path) -> io::Result<u32> {
    use rustix::fs::OFlags;
    let name = path_c_string(directory)?;
    let fd = rustix::fs::open(&*name, OFlags::RDONLY | OFlags::DIRECTORY | OFlags::NOFOLLOW | OFlags::CLOEXEC, rustix::fs::Mode::empty()).map_err(os)?;
    rustix::fs::ioctl_getflags(&fd).map(|flags| flags.bits()).map_err(os)
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
    let name = path_c_string(path)?;
    let result = crate::flight::checked("set-acl", || bytes(path),
                                     || rustix::fs::setxattr(&*name, c"system.posix_acl_access", value, rustix::fs::XattrFlags::empty()).map_err(os),
                                     |r| crate::flight::outcome(r, |()| 0));
    changed(&result, path, false);
    result
}

/// sync(2): every pending disk update, the whole system's.
pub fn sync_all() -> io::Result<()> {
    effect(|| "sync".into())?;
    rustix::fs::sync();
    Ok(())
}

/// Close this process's standard stream FD (0, 1 or 2).
pub fn close_standard_stream(fd: i32) -> io::Result<()> {
    effect(|| format!("close standard stream {fd}"))?;
    sys::close(fd)
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
    crate::flight::checked("cwd", String::new, std::env::current_dir, |r| crate::flight::outcome(r, |p| crate::flight::digest(p.as_os_str().as_bytes())))
}

/// A machine or user fact, recorded with its value.
fn host_fact(fact: &str, value: Json) {
    record("host", json!({"fact": fact, "value": value}));
}

pub fn current_exe() -> io::Result<PathBuf> {
    let result = crate::flight::checked("exe", String::new, std::env::current_exe, |r| crate::flight::outcome(r, |p| crate::flight::digest(p.as_os_str().as_bytes())));
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
    crate::flight::value("pid", std::process::id)
}

pub fn uid() -> u32 {
    let uid = rustix::process::getuid().as_raw();
    if building() {
        host_fact("uid", json!(uid));
    }
    uid
}

/// Die with PARENT: SIGTERM when the thread that started this process
/// ends, and exit now if PARENT is gone already. A child of a session
/// thread then never outlives the session.
pub fn die_with_parent(parent: u32) {
    let _ = rustix::process::set_parent_process_death_signal(Some(rustix::process::Signal::TERM));
    if std::os::unix::process::parent_id() != parent {
        std::process::exit(1);
    }
}

/// How far a background process steps back from the session.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Priority {
    /// Work nobody waits for, such as a background compile: the weakest
    /// CPU niceness and the idle I/O class.
    Idle,
    /// A worker isolate: work the session waits for, never ahead of it:
    /// niceness 10 and the lowest best-effort I/O level, so the session's
    /// evaluator and render threads always run first.
    Worker,
}

/// Lower the calling thread to PRIORITY; the threads and processes it
/// starts afterwards inherit it, so a process calls it at the start of
/// main. Best effort.
pub fn lower_priority(priority: Priority) {
    const IOPRIO_CLASS_BE: libc::c_long = 2;
    const IOPRIO_CLASS_IDLE: libc::c_long = 3;
    let (nice, io) = match priority {
        Priority::Idle => (19, IOPRIO_CLASS_IDLE << 13),
        Priority::Worker => (10, (IOPRIO_CLASS_BE << 13) | 7),
    };
    let _ = rustix::process::setpriority_process(None, nice);
    sys::set_io_priority(io);
}

/// Name the calling thread NAME (the kernel keeps 15 bytes). On a
/// process's main thread that is the process's own name, the one `ps` and
/// `pgrep` show.
pub fn set_thread_name(name: &str) -> io::Result<()> {
    effect(|| "thread-name".into())?;
    let name = std::ffi::CString::new(name).map_err(|e| io::Error::new(io::ErrorKind::InvalidInput, e))?;
    rustix::thread::set_name(&name).map_err(os)
}

pub fn euid() -> u32 {
    let euid = rustix::process::geteuid().as_raw();
    if building() {
        host_fact("euid", json!(euid));
    }
    euid
}

pub fn gid() -> u32 {
    let gid = rustix::process::getgid().as_raw();
    if building() {
        host_fact("gid", json!(gid));
    }
    gid
}

pub fn egid() -> u32 {
    let egid = rustix::process::getegid().as_raw();
    if building() {
        host_fact("egid", json!(egid));
    }
    egid
}

// User database.

pub fn user_by_uid(uid: u32, buffer: usize) -> Result<Passwd, i32> {
    let result = sys::passwd(Ok(uid), buffer);
    user_db(json!({"uid": uid}), &result);
    result
}

pub fn user_by_name(name: &CStr, buffer: usize) -> Result<Passwd, i32> {
    let result = sys::passwd(Err(name), buffer);
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
    let result = sys::group(Ok(gid), buffer).and_then(|(name, _)| name.ok_or(0));
    if building() {
        let observed = match &result { Ok(name) => encoded(name), Err(status) => json!({"status": status}) };
        record("user-db", json!({"key": {"gid": gid}, "result": observed}));
    }
    result
}

/// The group NAME names: its gid.
pub fn group_by_name(name: &CStr, buffer: usize) -> Result<u32, i32> {
    let result = sys::group(Err(name), buffer).map(|(_, gid)| gid);
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
    let names = sys::account_names(groups);
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
        let baked = with_build(|b| class(Fact::SystemName, b.site.as_deref()) == Class::B).unwrap_or(false);
        if baked && refusing() {
            p_read(Fact::SystemName, "system-name".into());
        } else {
            host_fact("system-name", match &result { Ok(name) => json!(name), Err(e) => errno(e) });
        }
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
    crate::flight::value("processors", || count_processors(set))
}

fn count_processors(set: Processors) -> u64 {
    let name = match set {
        Processors::Current => {
            let count = rustix::thread::sched_getaffinity(None).map_or(0, |cpus| cpus.count());
            if count > 0 { return u64::from(count); }
            libc::_SC_NPROCESSORS_ONLN
        }
        Processors::Online => libc::_SC_NPROCESSORS_ONLN,
        Processors::All => libc::_SC_NPROCESSORS_CONF,
    };
    u64::try_from(sys::sysconf(name)).ok().filter(|&n| n > 0).unwrap_or(1)
}

// Performance counters.

/// User-space instructions the calling thread has retired since its first
/// call: a pinned Linux hardware counter (perf_event_open), opened once per
/// thread. Unlike time it is load-insensitive and repeatable, so scaling
/// probes compare work at two sizes by it. None where the kernel refuses
/// counters (perf_event_paranoid above 2, no PMU) or cannot keep this one
/// counting all the time: a number is always a whole count.
pub(crate) fn thread_instructions() -> Option<u64> {
    crate::flight::value("instructions", instruction_count)
}

/// `thread_instructions` as a meter reads it (redisplay's bracket, twice
/// a cycle): neither logged for a replay nor counted among the host
/// calls. What a meter hands on to Lisp is a host result, and logged
/// there.
pub(crate) fn instruction_count() -> Option<u64> {
    INSTRUCTION_COUNTER.with(|counter| {
        let mut counter = counter.borrow_mut();
        counter.get_or_insert_with(open_instruction_counter).as_ref().and_then(read_counter)
    })
}

thread_local! {
    /// The calling thread's counter, once opened (None inside: refused).
    static INSTRUCTION_COUNTER: std::cell::RefCell<Option<Option<std::os::fd::OwnedFd>>> = const { std::cell::RefCell::new(None) };
}

fn open_instruction_counter() -> Option<std::os::fd::OwnedFd> {
    sys::open_instruction_counter()
}

/// The counter's value; none once a pinned counter lost its PMU slot,
/// which the kernel reports as end of file.
fn read_counter(fd: &std::os::fd::OwnedFd) -> Option<u64> {
    let mut count = [0u8; 8];
    (rustix::io::read(fd, &mut count) == Ok(8)).then(|| u64::from_ne_bytes(count))
}

// Time.

/// The kernel's clock ticks per second (`_SC_CLK_TCK`), the unit of
/// /proc's process times.
pub fn clock_ticks_per_second() -> u64 {
    Some(rustix::param::clock_ticks_per_second()).filter(|&n| n > 0).unwrap_or(100)
}

/// The time since boot, suspended time included (`CLOCK_BOOTTIME`), as
/// GNU's get_host_uptime reads it; None where the clock is missing.
pub fn boot_clock() -> Option<std::time::Duration> {
    crate::flight::value("boot-clock", || {
        let t = rustix::time::clock_gettime_dynamic(rustix::time::DynamicClockId::Boottime).ok()?;
        Some(std::time::Duration::new(t.tv_sec as u64, t.tv_nsec as u32))
    })
}

/// sysinfo(2)'s RAM and swap totals and free amounts, in bytes:
/// (total RAM, free RAM, total swap, free swap).
pub fn memory_info() -> Option<[u64; 4]> {
    p_read(Fact::Machine, "memory-info".into());
    crate::flight::value("memory-info", || {
        let si = rustix::system::sysinfo();
        let unit = u64::from(si.mem_unit);
        Some([si.totalram as u64 * unit, si.freeram as u64 * unit, si.totalswap as u64 * unit, si.freeswap as u64 * unit])
    })
}

/// getloadavg(3): the 1, 5 and 15 minute averages, as glibc derives them
/// from sysinfo(2)'s fixed-point loads (SI_LOAD_SHIFT, 16 bits).
pub fn load_average() -> Option<Vec<f64>> {
    p_read(Fact::Machine, "load-average".into());
    crate::flight::value("load-average", || {
        Some(rustix::system::sysinfo().loads.iter().map(|&load| load as f64 / 65536.0).collect())
    })
}

/// The memory page size in bytes (`getpagesize`).
pub fn page_size() -> u64 {
    Some(rustix::param::page_size() as u64).filter(|&n| n > 0).unwrap_or(4096)
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
    crate::flight::value("clock", SystemTime::now)
}

/// The processor time this process has used, user and system, as
/// `getrusage (RUSAGE_SELF)` sums them (GNU sysdep.c
/// `get-internal-run-time'); in build mode none, a per-process fact.
pub fn run_time() -> io::Result<std::time::Duration> {
    if refusing() {
        p_read(Fact::Monotonic, "run time".into());
        return Ok(std::time::Duration::ZERO);
    }
    crate::flight::value("run-time", live_run_time)
}

/// The processor time this process has used, read live and never logged:
/// for a process outside the session's own results (a mark twin's child,
/// `twin.rs`, whose usage starts at its fork).
pub(crate) fn live_run_time() -> io::Result<std::time::Duration> {
    sys::self_usage().map(|(user, system)| user + system)
}

/// The monotonic clock, for deadlines and measurement.
pub fn monotonic() -> Instant {
    if building() {
        p_read(Fact::Monotonic, "monotonic clock".into());
    }
    crate::flight::value("monotonic", Instant::now)
}

/// The instant a flight recording or replay counts its times from: never
/// logged itself.
pub(crate) fn flight_base() -> Instant {
    Instant::now()
}

/// F with every signal blocked on this thread, so threads it starts leave
/// signals to the threads that handle them.
pub fn with_signals_blocked<T>(f: impl FnOnce() -> T) -> T {
    sys::with_signals_blocked(f)
}

/// When a collection started, for `gc-elapsed'; none in build mode, nor in
/// a builder whose recording has ended (its capture collects), so a world's
/// heap never depends on how long its collections took.
pub fn collection_clock() -> Option<Instant> {
    (!building() && !crate::world::arena::heap_is_image()).then(Instant::now)
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
    let created = absent(path.as_ref());
    let result = crate::flight::checked("open-output", || bytes(path.as_ref()), || options.clone().read(false).write(true).open(&path), |r| crate::flight::outcome(r, |_| 0));
    changed(&result, path.as_ref(), created);
    result
}

/// GNU copy-file's fast paths: share SOURCE's extents with DEST, else
/// copy_file_range until it stops. False leaves the rest to read+write.
pub fn clone_file(dest: &File, source: &Input) -> bool {
    if rustix::fs::ioctl_ficlone(dest, &source.0).is_ok() {
        return true;
    }
    const COPY_MAX: usize = (isize::MAX as usize) >> 30 << 30;
    while rustix::fs::copy_file_range(&source.0, None, dest, None, COPY_MAX).is_ok_and(|n| n > 0) {}
    false
}

pub fn write(path: impl AsRef<Path>, bytes: impl AsRef<[u8]>) -> io::Result<()> {
    wrote(path.as_ref());
    effect(|| format!("write {}", self::bytes(path.as_ref())))?;
    let created = absent(path.as_ref());
    let result = crate::flight::checked("write", || self::bytes(path.as_ref()), || std::fs::write(&path, &bytes), |r| crate::flight::outcome(r, |()| 0));
    changed(&result, path.as_ref(), created);
    result
}

pub fn rename(from: impl AsRef<Path>, to: impl AsRef<Path>) -> io::Result<()> {
    wrote(from.as_ref());
    wrote(to.as_ref());
    effect(|| format!("rename {} {}", bytes(from.as_ref()), bytes(to.as_ref())))?;
    let result = crate::flight::checked("rename", || bytes(to.as_ref()), || std::fs::rename(&from, &to), |r| crate::flight::outcome(r, |()| 0));
    changed(&result, from.as_ref(), true);
    changed(&result, to.as_ref(), true);
    result
}

/// fsync the directory DIR, so a rename into it survives a host crash.
pub fn sync_dir(dir: impl AsRef<Path>) -> io::Result<()> {
    effect(|| format!("sync directory {}", bytes(dir.as_ref())))?;
    File::open(dir)?.sync_all()
}

pub fn remove_file(path: impl AsRef<Path>) -> io::Result<()> {
    wrote(path.as_ref());
    effect(|| format!("delete {}", bytes(path.as_ref())))?;
    let result = crate::flight::checked("remove-file", || bytes(path.as_ref()), || std::fs::remove_file(&path), |r| crate::flight::outcome(r, |()| 0));
    changed(&result, path.as_ref(), true);
    result
}

pub fn remove_dir(path: impl AsRef<Path>) -> io::Result<()> {
    wrote(path.as_ref());
    effect(|| format!("delete directory {}", bytes(path.as_ref())))?;
    let result = crate::flight::checked("remove-dir", || bytes(path.as_ref()), || std::fs::remove_dir(&path), |r| crate::flight::outcome(r, |()| 0));
    changed(&result, path.as_ref(), true);
    result
}

/// mkdir with MODE before the umask.
pub fn create_dir(path: impl AsRef<Path>, mode: u32) -> io::Result<()> {
    wrote(path.as_ref());
    effect(|| format!("make directory {}", bytes(path.as_ref())))?;
    use std::os::unix::fs::DirBuilderExt;
    let result = crate::flight::checked("mkdir", || bytes(path.as_ref()), || std::fs::DirBuilder::new().mode(mode).create(&path), |r| crate::flight::outcome(r, |()| 0));
    changed(&result, path.as_ref(), true);
    result
}

pub fn create_dir_all(path: impl AsRef<Path>) -> io::Result<()> {
    wrote(path.as_ref());
    effect(|| format!("make directory {}", bytes(path.as_ref())))?;
    let created = absent(path.as_ref());
    let result = crate::flight::checked("mkdir-all", || bytes(path.as_ref()), || std::fs::create_dir_all(&path), |r| crate::flight::outcome(r, |()| 0));
    changed(&result, path.as_ref(), created);
    result
}

pub fn hard_link(original: impl AsRef<Path>, link: impl AsRef<Path>) -> io::Result<()> {
    wrote(link.as_ref());
    effect(|| format!("link {}", bytes(link.as_ref())))?;
    let result = crate::flight::checked("link", || bytes(link.as_ref()), || std::fs::hard_link(&original, &link), |r| crate::flight::outcome(r, |()| 0));
    changed(&result, link.as_ref(), true);
    changed(&result, original.as_ref(), false);
    result
}

pub fn symlink(target: impl AsRef<Path>, link: impl AsRef<Path>) -> io::Result<()> {
    wrote(link.as_ref());
    effect(|| format!("symlink {}", bytes(link.as_ref())))?;
    let result = crate::flight::checked("symlink", || bytes(link.as_ref()), || std::os::unix::fs::symlink(&target, &link), |r| crate::flight::outcome(r, |()| 0));
    changed(&result, link.as_ref(), true);
    result
}

pub fn fchown(file: &File, uid: Option<u32>, gid: Option<u32>) -> io::Result<()> {
    crate::flight::checked("fchown", String::new, || std::os::unix::fs::fchown(file, uid, gid), |r| crate::flight::outcome(r, |()| 0))
}

/// fchmodat: PATH's mode bits, a symlink's own under NOFOLLOW.
pub fn chmod(path: &CStr, mode: u32, nofollow: bool) -> io::Result<()> {
    wrote(Path::new(OsStr::from_bytes(path.to_bytes())));
    effect(|| format!("chmod {}", String::from_utf8_lossy(path.to_bytes())))?;
    let name = Path::new(OsStr::from_bytes(path.to_bytes()));
    let result = crate::flight::checked("chmod", || bytes(name), || sys::chmod(path, mode, nofollow),
                                     |r| crate::flight::outcome(r, |()| 0));
    changed(&result, name, false);
    result
}

/// renameat2 with RENAME_NOREPLACE: EEXIST rather than replace TO.
pub fn rename_noreplace(from: &CStr, to: &CStr) -> io::Result<()> {
    wrote(Path::new(OsStr::from_bytes(to.to_bytes())));
    effect(|| format!("rename {}", String::from_utf8_lossy(to.to_bytes())))?;
    let (from_name, to_name) = (Path::new(OsStr::from_bytes(from.to_bytes())), Path::new(OsStr::from_bytes(to.to_bytes())));
    let result = crate::flight::checked("rename", || bytes(to_name),
                                     || rustix::fs::renameat_with(rustix::fs::CWD, from, rustix::fs::CWD, to, rustix::fs::RenameFlags::NOREPLACE).map_err(os),
                                     |r| crate::flight::outcome(r, |()| 0));
    changed(&result, from_name, true);
    changed(&result, to_name, true);
    result
}

/// utimensat: both of PATH's times set to TIME (seconds, nanoseconds), or
/// now; a symlink's own under NOFOLLOW.
pub fn set_times(path: &CStr, time: Option<(i64, i64)>, nofollow: bool) -> io::Result<()> {
    wrote(Path::new(OsStr::from_bytes(path.to_bytes())));
    effect(|| format!("set times {}", String::from_utf8_lossy(path.to_bytes())))?;
    let stamp = match time {
        Some((seconds, nanos)) => rustix::fs::Timespec { tv_sec: seconds, tv_nsec: nanos },
        None => rustix::fs::Timespec { tv_sec: 0, tv_nsec: rustix::fs::UTIME_NOW },
    };
    let name = Path::new(OsStr::from_bytes(path.to_bytes()));
    let result = crate::flight::checked("set-times", || bytes(name), || utimensat(path, stamp, nofollow),
                                     |r| crate::flight::outcome(r, |()| 0));
    changed(&result, name, false);
    result
}

/// mkostemp: a new close-on-exec file named TEMPLATE with its trailing
/// XXXXXX replaced, and that name.
pub fn temp_file(template: &Path) -> io::Result<(PathBuf, File)> {
    effect(|| format!("temporary file {}", bytes(template)))?;
    use std::os::unix::ffi::{OsStrExt, OsStringExt};
    let (name, file) = sys::temp_file(template.as_os_str().as_bytes())?;
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

/// A child process, as GNU's emacs_spawn starts one (callproc.c): ARGV[0],
/// a file name with a slash, run with ARGV and ENV (each `NAME=VALUE') in
/// DIR, FDS as its standard input, output and error, in a session of its
/// own, with SIGINT, SIGQUIT, SIGPROF and SIGPIPE at their defaults and no
/// signal blocked. posix_spawn: the child shares this process's memory
/// until it execs, where a fork would copy page tables whose size grows
/// with the heap. Each of FDS must be above 2, so no descriptor is
/// replaced before it is copied.
pub fn spawn_child(argv: &[Vec<u8>], env: &[Vec<u8>], dir: &[u8], fds: [std::os::fd::RawFd; 3]) -> io::Result<i32> {
    use std::ffi::CString;
    effect(|| format!("run {}", String::from_utf8_lossy(argv.first().map_or(&[][..], Vec::as_slice))))?;
    let invalid = || io::Error::from_raw_os_error(libc::EINVAL);
    let strings = |list: &[Vec<u8>]| list.iter().map(|b| CString::new(b.as_slice())).collect::<Result<Vec<_>, _>>().map_err(|_| invalid());
    let (argv, env, dir) = (strings(argv)?, strings(env)?, CString::new(dir).map_err(|_| invalid())?);
    sys::spawn(&argv, &env, &dir, fds)
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
    /// To append to it (O_APPEND), made if missing: concurrent writers'
    /// lines never overwrite each other.
    Append,
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
        StoreOpen::Append => { options.append(true).create(true); }
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
    use rustix::fs::{Mode, OFlags};
    use std::os::unix::ffi::OsStrExt;
    let refuse = |why: String| io::Error::new(io::ErrorKind::PermissionDenied, why);
    if !path.is_absolute() {
        return Err(refuse(format!("{} is not absolute", path.display())));
    }
    let root = PathBuf::from("/");
    let flags = OFlags::DIRECTORY | OFlags::RDONLY | OFlags::CLOEXEC | OFlags::NOFOLLOW;
    let mut dir = rustix::fs::open("/", flags, Mode::empty()).map_err(os)?;
    let unshared = |dir: &OwnedFd, walked: &Path| -> io::Result<()> {
        let stat = rustix::fs::fstat(dir).map_err(os)?;
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
        let mut next = rustix::fs::openat(&dir, &*name, flags, Mode::empty());
        if create && matches!(next, Err(rustix::io::Errno::NOENT)) {
            effect(|| format!("make directory {}", walked.display()))?;
            made = rustix::fs::mkdirat(&dir, &*name, Mode::from_raw_mode(0o700)).is_ok();
            next = rustix::fs::openat(&dir, &*name, flags, Mode::empty());
        }
        let next = next.map_err(|error| if matches!(error, rustix::io::Errno::LOOP | rustix::io::Errno::NOTDIR) {
            refuse(format!("{} is no directory of its own (a symlink?)", walked.display()))
        } else { os(error) })?;
        if made {
            rustix::fs::fchmod(&next, Mode::from_raw_mode(0o700)).map_err(os)?;
        }
        unshared(&next, &walked)?;
        dir = next;
    }
    Ok(dir)
}

/// Whether the directory DIR holds nothing.
pub fn store_dir_is_empty(dir: std::os::fd::BorrowedFd<'_>) -> io::Result<bool> {
    // A copy shares DIR's offset, as the stream fdopendir made of one did.
    let copy = rustix::io::fcntl_dupfd_cloexec(dir, 0).map_err(os)?;
    let mut buffer = Vec::with_capacity(8192);
    let mut entries = rustix::fs::RawDir::new(&copy, buffer.spare_capacity_mut());
    while let Some(Ok(entry)) = entries.next() {
        let name = entry.file_name().to_bytes();
        if name != b"." && name != b".." {
            return Ok(false);
        }
    }
    Ok(true)
}

/// `open_store_file` for NAME in the directory DIR: opened relative to it
/// (openat), so nothing above is resolved again.
pub fn open_store_file_at(dir: std::os::fd::BorrowedFd<'_>, name: &str, how: StoreOpen) -> io::Result<File> {
    use rustix::fs::OFlags;
    let flags = OFlags::NOFOLLOW | OFlags::NONBLOCK | OFlags::CLOEXEC | match how {
        StoreOpen::Read => OFlags::RDONLY,
        StoreOpen::Write { create: true, .. } => OFlags::WRONLY | OFlags::CREATE,
        StoreOpen::Write { create: false, .. } => OFlags::WRONLY,
        StoreOpen::New => OFlags::WRONLY | OFlags::CREATE | OFlags::EXCL,
        StoreOpen::Append => OFlags::WRONLY | OFlags::CREATE | OFlags::APPEND,
    };
    let cname = std::ffi::CString::new(name).map_err(io::Error::other)?;
    let file = File::from(rustix::fs::openat(dir, &*cname, flags, rustix::fs::Mode::from_raw_mode(0o600)).map_err(os)?);
    regular_and_blocking(file, name, how)
}

/// FILE, once it is a regular file, with O_NONBLOCK cleared and, for a
/// truncating write, emptied.
fn regular_and_blocking(file: File, name: impl std::fmt::Display, how: StoreOpen) -> io::Result<File> {
    if !file.metadata()?.file_type().is_file() {
        return Err(io::Error::new(io::ErrorKind::InvalidInput, format!("{name} is not a regular file")));
    }
    set_blocking(file.as_fd(), true)?;
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
    sys::remove_env(name);
}

/// kill(2): SIGNAL to PID, a process group when negative. Signal 0 asks
/// only whether PID exists and may be signalled.
pub fn kill(pid: i32, signal: i32) -> io::Result<()> {
    effect(|| format!("signal {signal} to {pid}"))?;
    sys::kill(pid, signal)
}

/// A pidfd for PID, readable once it exits.
pub fn pidfd_open(pid: i32) -> Option<OwnedFd> {
    effect(|| format!("pidfd {pid}")).ok()?;
    rustix::process::pidfd_open(rustix::process::Pid::from_raw(pid)?, rustix::process::PidfdFlags::empty()).ok()
}

/// A memfd holding BYTES, sealed against writing, shrinking and growing
/// once written: the backing of a large frozen region (`frozen.rs`), which
/// another process may map or read knowing the bytes cannot change.
pub fn sealed_memfd(bytes: &[u8]) -> io::Result<File> {
    use rustix::fs::MemfdFlags;
    use std::io::Write;
    effect(|| "memfd".into())?;
    let mut file = File::from(rustix::fs::memfd_create(c"romacs-frozen", MemfdFlags::CLOEXEC | MemfdFlags::ALLOW_SEALING).map_err(os)?);
    file.write_all(bytes)?;
    rustix::fs::fcntl_add_seals(&file, SEALS).map_err(os)?;
    Ok(file)
}

const SEALS: rustix::fs::SealFlags = rustix::fs::SealFlags::WRITE.union(rustix::fs::SealFlags::SHRINK).union(rustix::fs::SealFlags::GROW);

/// FILE's bytes, when `sealed_memfd`'s seals are on it; a descriptor another
/// process sent is refused otherwise, since its bytes could change under
/// the reader.
pub fn read_sealed(file: &File) -> io::Result<Vec<u8>> {
    use std::os::unix::fs::FileExt;
    if !rustix::fs::fcntl_get_seals(file).is_ok_and(|seals| seals.contains(SEALS)) {
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
    use rustix::fs::inotify::CreateFlags;
    effect(|| "inotify".into())?;
    rustix::fs::inotify::init(CreateFlags::NONBLOCK | CreateFlags::CLOEXEC).map_err(os)
}

/// inotify_add_watch(2): PATH's watch descriptor on FD with MASK.
pub fn inotify_add_watch(fd: std::os::fd::BorrowedFd<'_>, path: &CStr, mask: u32) -> io::Result<i32> {
    effect(|| format!("inotify watch {}", path.to_string_lossy()))?;
    rustix::fs::inotify::add_watch(fd, path, rustix::fs::inotify::WatchFlags::from_bits_retain(mask)).map_err(os)
}

/// inotify_rm_watch(2).
pub fn inotify_rm_watch(fd: std::os::fd::BorrowedFd<'_>, wd: i32) -> io::Result<()> {
    rustix::fs::inotify::remove_watch(fd, wd).map_err(os)
}

/// The `struct inotify_event' records queued on FD, as their bytes; none
/// when nothing is queued (FD is nonblocking).
pub fn inotify_read(fd: std::os::fd::BorrowedFd<'_>) -> io::Result<Vec<u8>> {
    let queued = rustix::io::ioctl_fionread(fd).map_err(os)?;
    // One record at least, whatever FIONREAD said, so a read never fails
    // with EINVAL for a buffer too small for the next record.
    let mut buffer = vec![0u8; (queued as usize).max(std::mem::size_of::<libc::inotify_event>() + 256)];
    match rustix::io::read(fd, &mut buffer[..]) {
        Ok(n) => { buffer.truncate(n); Ok(buffer) }
        Err(rustix::io::Errno::AGAIN) => Ok(Vec::new()),
        Err(e) => Err(os(e)),
    }
}

// Isolates (`isolate.rs`): worker processes, their control sockets, the
// descriptors they pass, and the signals between them.

/// FD nonblocking: a worker's inherited control socket.
pub fn set_nonblocking(fd: std::os::fd::BorrowedFd<'_>) -> io::Result<()> {
    set_blocking(fd, false)
}

/// FD's O_NONBLOCK cleared (BLOCKING) or set, its other status flags kept.
fn set_blocking(fd: std::os::fd::BorrowedFd<'_>, blocking: bool) -> io::Result<()> {
    use rustix::fs::OFlags;
    let flags = rustix::fs::fcntl_getfl(fd).map_err(os)?;
    let flags = if blocking { flags.difference(OFlags::NONBLOCK) } else { flags.union(OFlags::NONBLOCK) };
    rustix::fs::fcntl_setfl(fd, flags).map_err(os)
}

/// A connected pair of close-on-exec, nonblocking `SOCK_SEQPACKET` sockets:
/// one packet per `send_packet`, received whole or not at all; a full
/// buffer makes a send fail with `WouldBlock` rather than wait.
pub fn socketpair() -> io::Result<(OwnedFd, OwnedFd)> {
    use rustix::net::{AddressFamily, SocketFlags, SocketType};
    effect(|| "socketpair".into())?;
    rustix::net::socketpair(AddressFamily::UNIX, SocketType::SEQPACKET, SocketFlags::CLOEXEC | SocketFlags::NONBLOCK, None).map_err(os)
}

/// BYTES as one packet on SOCKET, with FD passed beside it (`SCM_RIGHTS`);
/// `WouldBlock` when the peer's buffer is full (a bounded queue).
pub fn send_packet(socket: std::os::fd::BorrowedFd<'_>, bytes: &[u8], fd: Option<std::os::fd::BorrowedFd<'_>>) -> io::Result<()> {
    use rustix::net::{SendAncillaryBuffer, SendAncillaryMessage, SendFlags};
    effect(|| "sendmsg".into())?;
    let fds = fd.as_slice();
    let mut space = [std::mem::MaybeUninit::uninit(); rustix::cmsg_space!(ScmRights(1))];
    let mut control = SendAncillaryBuffer::new(&mut space);
    if !fds.is_empty() {
        control.push(SendAncillaryMessage::ScmRights(fds));
    }
    loop {
        match rustix::net::sendmsg(socket, &[io::IoSlice::new(bytes)], &mut control, SendFlags::NOSIGNAL) {
            Ok(_) => return Ok(()),
            Err(rustix::io::Errno::INTR) => {}
            Err(error) => return Err(os(error)),
        }
    }
}

/// One packet from SOCKET, up to CAPACITY bytes, with the descriptors passed
/// beside it (close-on-exec); None once the peer has closed. A packet the
/// buffer cannot hold whole is an error, never a prefix.
pub fn recv_packet(socket: std::os::fd::BorrowedFd<'_>, capacity: usize) -> io::Result<Option<(Vec<u8>, Vec<OwnedFd>)>> {
    use rustix::net::{RecvAncillaryBuffer, RecvAncillaryMessage, RecvFlags, ReturnFlags};
    let mut bytes = vec![0u8; capacity];
    let mut space = [std::mem::MaybeUninit::uninit(); rustix::cmsg_space!(ScmRights(8))];
    let mut control = RecvAncillaryBuffer::new(&mut space);
    let received = loop {
        match rustix::net::recvmsg(socket, &mut [io::IoSliceMut::new(&mut bytes)], &mut control, RecvFlags::CMSG_CLOEXEC) {
            Ok(received) => break received,
            Err(rustix::io::Errno::INTR) => {}
            Err(error) => return Err(os(error)),
        }
    };
    let fds: Vec<OwnedFd> = control.drain().flat_map(|message| match message {
        RecvAncillaryMessage::ScmRights(fds) => fds.collect(),
        _ => Vec::new(),
    }).collect();
    if received.bytes == 0 && fds.is_empty() { return Ok(None); }
    if received.flags.contains(ReturnFlags::TRUNC) {
        return Err(io::Error::new(io::ErrorKind::InvalidData, "isolate packet larger than its buffer"));
    }
    bytes.truncate(received.bytes);
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
    effect(|| format!("pidfd signal {signal}"))?;
    sys::pidfd_send_signal(pidfd, signal, if group { PIDFD_SIGNAL_PROCESS_GROUP } else { 0 })
}

/// prctl(PR_SET_PDEATHSIG): SIGNAL once the thread that made this process
/// exits. The caller checks `parent_pid` after, for a parent gone before.
pub fn set_parent_death_signal(signal: i32) -> io::Result<()> {
    effect(|| "pdeathsig".into())?;
    let signal = rustix::process::Signal::from_named_raw(signal).ok_or_else(|| io::Error::from_raw_os_error(libc::EINVAL))?;
    rustix::process::set_parent_process_death_signal(Some(signal)).map_err(os)
}

pub fn parent_pid() -> i32 {
    if building() {
        p_read(Fact::Pid, "ppid".into());
        return 0;
    }
    crate::flight::value("ppid", || std::os::unix::process::parent_id() as i32)
}

/// The worker's end of an isolate's signals: SIGUSR1 notes an interrupt for
/// the engine's safepoints, SIGTERM a termination for its loop. Either
/// also writes a byte to the wake pipe, which the worker's isolate wait
/// polls: the signal may land on any of the process's threads (the main
/// one waits in a join), so only the pipe reliably ends a blocked wait.
static INTERRUPT: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
static TERMINATE: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
/// Seconds a worker has to end after its first SIGTERM, or 0: no limit
/// (`limit_termination`).
static TERMINATE_LIMIT: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
/// The wake pipe's read and write ends, or -1.
static WAKE_READ: std::sync::atomic::AtomicI32 = std::sync::atomic::AtomicI32::new(-1);
static WAKE_WRITE: std::sync::atomic::AtomicI32 = std::sync::atomic::AtomicI32::new(-1);

fn wake_isolate_wait() {
    let fd = WAKE_WRITE.load(std::sync::atomic::Ordering::SeqCst);
    // Async-signal-safe; a full pipe already wakes the wait.
    if fd >= 0 { sys::wake(fd); }
}
extern "C" fn note_interrupt(_: libc::c_int) {
    INTERRUPT.store(true, std::sync::atomic::Ordering::SeqCst);
    wake_isolate_wait();
}
extern "C" fn note_terminate(_: libc::c_int) {
    use std::sync::atomic::Ordering;
    let first = !TERMINATE.swap(true, Ordering::SeqCst);
    // alarm(2) is async-signal-safe; SIGALRM's default action ends the
    // process, whatever it is blocked in.
    let seconds = TERMINATE_LIMIT.load(Ordering::SeqCst);
    if first && seconds > 0 { sys::alarm(seconds); }
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
        sys::drain(fd);
    }
}

/// A worker ends within SECONDS of its first SIGTERM: a termination is a
/// flag its loop reads, which a worker blocked inside a primitive never
/// does, and with its spawner gone nothing else would end it.
pub fn limit_termination(seconds: u32) {
    TERMINATE_LIMIT.store(seconds, std::sync::atomic::Ordering::SeqCst);
}

pub fn catch_isolate_signals() -> io::Result<()> {
    if WAKE_READ.load(std::sync::atomic::Ordering::SeqCst) < 0 {
        use rustix::pipe::PipeFlags;
        use std::os::fd::IntoRawFd;
        effect(|| "isolate wake pipe".into())?;
        let (read, write) = rustix::pipe::pipe_with(PipeFlags::CLOEXEC | PipeFlags::NONBLOCK).map_err(os)?;
        // For the process's life: a handler may write to it at any time.
        WAKE_WRITE.store(write.into_raw_fd(), std::sync::atomic::Ordering::SeqCst);
        WAKE_READ.store(read.into_raw_fd(), std::sync::atomic::Ordering::SeqCst);
    }
    for (signal, handler) in [(libc::SIGUSR1, note_interrupt as extern "C" fn(libc::c_int)), (libc::SIGTERM, note_terminate)] {
        effect(|| format!("handle signal {signal}"))?;
        sys::catch_signal(signal, handler, 0)?;
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
    // SIGUSR2 also asks for a snapshot, which `debug-on-event' leaves alone.
    if signal == libc::SIGUSR2 && INSPECT_ENABLED.load(Ordering::SeqCst) {
        INSPECT_REQUESTED.fetch_add(1, Ordering::SeqCst);
    }
    // A waiting session wakes to read it, as GNU's select returns on a
    // signal; write(2) is async-signal-safe, and the end is non-blocking.
    let fd = TERMINATION_PIPE.load(Ordering::SeqCst);
    if fd >= 0 { sys::wake(fd); }
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
        sys::catch_signal(signal, note_user_signal, libc::SA_RESTART)?;
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
/// standard input, this process's stderr. Its standard output is null: a
/// worker's Lisp is batch, but the output it would write (`ding''s BEL,
/// `princ' to t) is no one's, and this process's may be a session's log.
/// Its pid; the caller reaps.
/// Spawned, never forked, as a world builder is (`World::start_builder`):
/// no pre_exec, so no driver's atfork handler runs.
pub fn spawn_isolate(program: &Path, args: &[OsString], socket: OwnedFd) -> io::Result<i32> {
    let mut command = command(program)?;
    command.arg("--isolate").arg("0").args(args).stdin(socket).stdout(std::process::Stdio::null());
    Ok(command.spawn()?.id() as i32)
}

/// End this process now with CODE: no destructor, atexit handler or
/// buffered output of it runs, so a forked child leaves nothing it shares
/// with its parent flushed twice.
pub fn exit_at_once(code: i32) -> ! {
    sys::exit_at_once(code)
}

/// This process was forked from a zygote (`fork_isolate`).
static FORKED: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// The threads of this process other than its main thread and the
/// caller (/proc/self/task): those a fork from the caller would lose.
pub fn other_threads() -> io::Result<Vec<i32>> {
    let (main, caller) = (std::process::id() as i32, rustix::thread::gettid().as_raw_nonzero().get());
    let mut others = Vec::new();
    for entry in std::fs::read_dir("/proc/self/task")? {
        let tid = entry?.file_name().to_string_lossy().parse::<i32>().map_err(io::Error::other)?;
        if tid != main && tid != caller { others.push(tid); }
    }
    Ok(others)
}

/// fork(2) from a zygote isolate, which checked that no thread but the
/// caller and its parked main thread exists (`other_threads`): 0 in the child, whose one
/// thread is the caller, else the child's pid. The child resets what is
/// its own with `forked_child_reset`.
pub fn fork_isolate() -> io::Result<i32> {
    let pid = fork()?;
    if pid == 0 { FORKED.store(true, std::sync::atomic::Ordering::SeqCst); }
    Ok(pid)
}

/// fork(2) from a process whose other threads run on (a mark twin's,
/// `twin.rs`): 0 in the child, else the child's pid. The child has the
/// caller's thread alone, so a lock another thread held stays held in it:
/// the allocator's is held across the fork (`arena::across_fork`), and the
/// child takes no other lock and ends with `exit_at_once`. It reads only
/// what the forking thread itself wrote, which is what lets the fork be a
/// lazy one where the heap is in a file (`romacs_allocator::file_heap`). It
/// dies with the forking thread, even one that ends without unwinding
/// (`_exit`), and ends at once if that thread was gone before it could ask.
pub(crate) fn fork_threaded() -> io::Result<i32> {
    let parent = std::process::id();
    let pid = crate::world::arena::across_fork(|| romacs_allocator::file_heap::lazily(fork))?;
    if pid == 0 && (set_parent_death_signal(libc::SIGKILL).is_err() || std::os::unix::process::parent_id() != parent) {
        exit_at_once(0);
    }
    Ok(pid)
}

fn fork() -> io::Result<i32> {
    effect(|| "fork".into())?;
    sys::fork()
}

/// Whether this process is a child forked from a zygote: it ends with
/// `exit`, since no main thread is left to return to.
pub fn forked_child() -> bool {
    FORKED.load(std::sync::atomic::Ordering::SeqCst)
}

/// In a child just forked from ZYGOTE: its process name, no signal noted
/// yet, a wake pipe of its own (the zygote's would wake both), SIGTERM
/// once the zygote is gone (fork clears it), and its own instruction
/// counter (the inherited one counts the zygote's thread).
pub fn forked_child_reset(zygote: i32) -> io::Result<()> {
    use std::sync::atomic::Ordering;
    set_process_name("romacs-worker")?;
    INTERRUPT.store(false, Ordering::SeqCst);
    TERMINATE.store(false, Ordering::SeqCst);
    TERMINATE_QUIT.store(false, Ordering::SeqCst);
    for end in [WAKE_READ.swap(-1, Ordering::SeqCst), WAKE_WRITE.swap(-1, Ordering::SeqCst)] {
        if end >= 0 { let _ = sys::close(end); }
    }
    catch_isolate_signals()?;
    INSTRUCTION_COUNTER.with(|counter| counter.replace(None));
    set_parent_death_signal(libc::SIGTERM)?;
    if parent_pid() != zygote {
        return Err(io::Error::other(format!("zygote {zygote} is gone")));
    }
    Ok(())
}

/// The calling thread's id.
pub fn thread_id() -> i32 {
    crate::flight::value("tid", || rustix::thread::gettid().as_raw_nonzero().get())
}

/// The processors thread TID may run on: how many, and a digest of which;
/// None once it is gone.
pub fn thread_affinity(tid: i32) -> Option<(u32, u64)> {
    use std::hash::{Hash, Hasher};
    let cpus = rustix::thread::sched_getaffinity(rustix::process::Pid::from_raw(tid)).ok()?;
    let (mut count, mut digest) = (0, std::collections::hash_map::DefaultHasher::new());
    for cpu in 0..rustix::thread::CpuSet::MAX_CPU {
        let set = cpus.is_set(cpu);
        count += u32::from(set);
        set.hash(&mut digest);
    }
    Some((count, digest.finish()))
}

/// PID's proportional set size, in kilobytes (/proc/PID/smaps_rollup);
/// None once it is gone.
pub fn process_pss(pid: i32) -> Option<u64> {
    let rollup = std::fs::read_to_string(format!("/proc/{pid}/smaps_rollup")).ok()?;
    rollup.lines().find_map(|line| line.strip_prefix("Pss:")?.split_whitespace().next()?.parse().ok())
}

/// Wait at GATE, a pipe's read end, until one byte arrives (true) or the
/// write end closes unwritten (false); a termination ends the wait too.
pub fn await_gate(gate: OwnedFd) -> bool {
    let mut byte = [0u8];
    loop {
        match rustix::io::read(&gate, &mut byte) {
            Ok(1) => return true,
            Err(rustix::io::Errno::INTR) if !terminate_pending() => {}
            _ => return false,
        }
    }
}

/// Open GATE, the write end `await_gate` waits on: one byte, then closed.
pub fn open_gate(gate: OwnedFd) -> io::Result<()> {
    effect(|| "gate".into())?;
    rustix::io::write(&gate, &[1u8]).map(drop).map_err(os)
}

/// The process umask, from /proc/self/status (GNU reads it once at
/// startup, fileio.c realmask). Checked by a flight as the mask alone:
/// the rest of the file (the pid, memory counters) is this process's.
pub fn umask() -> io::Result<u32> {
    let status = crate::flight::without(|| read_to_string("/proc/self/status"))?;
    let mask = status.lines().find_map(|line| line.strip_prefix("Umask:")).and_then(|mask| u32::from_str_radix(mask.trim(), 8).ok())
        .ok_or_else(|| io::Error::other("no Umask in /proc/self/status"));
    crate::flight::checked("umask", String::new, || mask, |r| crate::flight::outcome(r, |&m| u64::from(m)))
}

/// Name this process NAME (the kernel keeps 15 bytes): its thread-group
/// leader's name, the one `ps` and `pgrep` show, from any of its threads.
pub fn set_process_name(name: &str) -> io::Result<()> {
    effect(|| "process-name".into())?;
    std::fs::write("/proc/self/comm", name)
}

/// `emacs_pipe`: a close-on-exec pipe (read end, write end).
pub fn pipe() -> io::Result<(OwnedFd, OwnedFd)> {
    effect(|| "pipe".into())?;
    rustix::pipe::pipe_with(rustix::pipe::PipeFlags::CLOEXEC).map_err(os)
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
    if fd >= 0 { sys::wake(fd); }
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
    rustix::fs::fcntl_setfl(&writer, rustix::fs::OFlags::NONBLOCK).map_err(os)?;
    TERMINATION_PIPE.store(writer.into_raw_fd(), std::sync::atomic::Ordering::SeqCst);
    std::thread::Builder::new().name("romacs-signals".into()).spawn(move || {
        let mut byte = [0u8];
        while rustix::io::read(&reader, &mut byte) == Ok(1) {
            wake();
        }
    })?;
    for signal in [libc::SIGTERM, libc::SIGHUP] {
        effect(|| format!("handle signal {signal}"))?;
        sys::catch_signal(signal, note_termination, libc::SA_RESTART)?;
    }
    Ok(())
}

/// profiler.c's SIGPROF timer, as a thread that only counts: every
/// INTERVAL nanoseconds it looks at the calling thread, and when that
/// thread is running, adds one to TICKS. A late wake samples once: the
/// thread's state over the intervals it missed is unknown. The evaluator
/// takes them at its next call boundary, a moment later, under the stack
/// that was running. A sample is never counted for a thread that waits,
/// so like GNU's CLOCK_THREAD_CPUTIME_ID timer the samples measure the
/// thread's CPU time, and no wait is charged with work done before it.
/// Running means on a CPU: the thread's CPU clock advances between two
/// reads. Its scheduler state would not do, since `R' also holds while a
/// woken thread waits for a CPU, and on a loaded host that wait follows
/// every sleep. (A thread CPU-time timer itself
/// is not used: under a zero RLIMIT_RTTIME hard limit Linux kills a
/// thread whose CPU timer fires.) Dropping the sender ends the thread.
pub fn cpu_sampler(interval: u64, ticks: std::sync::Arc<std::sync::atomic::AtomicU64>) -> io::Result<std::sync::mpsc::Sender<()>> {
    effect(|| "cpu profiler sampler".into())?;
    let clock = sys::ThreadCpuClock::of_this_thread()?;
    let cpu_time = move || clock.now();
    // A thread allowed only one CPU shares it with the sampler, which
    // never sees it there: for it, runnable has to stand for running.
    // Read once: an affinity changed later takes a profiler restart.
    let alone = rustix::thread::sched_getaffinity(None).map_err(os)?.count() == 1;
    let stat = File::open("/proc/thread-self/stat")?;
    // stat's third field, after the parenthesized command name.
    let runnable = move || {
        use std::os::unix::fs::FileExt;
        let mut text = [0u8; 512];
        let n = stat.read_at(&mut text, 0).ok()?;
        let close = text[..n].iter().rposition(|&b| b == b')')?;
        Some(text.get(close + 2) == Some(&b'R'))
    };
    // A clock read is a system call, so a thread on a CPU has run between
    // two of them; a waiting one has not.
    let running = move || Some(cpu_time()? < cpu_time()? || alone && runnable()?);
    running().ok_or_else(|| io::Error::other("unreadable thread CPU clock"))?;
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
            // The profiled thread is gone: its clock no longer reads.
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

/// uname's release (editfns.c init_editfns).
pub fn os_release() -> String {
    rustix::system::uname().release().to_string_lossy().into_owned()
}

/// sysconf (_SC_ARG_MAX) / 4, GNU's crude bytes to characters (callproc.c:2249).
pub fn command_line_max() -> i64 {
    match sys::sysconf(libc::_SC_ARG_MAX) { n if n > 0 => n as i64 / 4, _ => 4096 }
}

/// The server this process is, as GNU's `--fg-daemon=NAME' is a daemon
/// (emacs.c daemon_type, `IS_DAEMON'): set once, at the command line,
/// for the whole process.
static DAEMON: std::sync::OnceLock<String> = std::sync::OnceLock::new();

pub fn set_daemon(name: &str) {
    let _ = DAEMON.set(name.to_owned());
}

/// The name `daemonp' answers, while this process is a server.
pub fn daemon() -> Option<&'static str> {
    DAEMON.get().map(String::as_str)
}

/// A close-on-exec, nonblocking eventfd: a wakeup counter no file names.
pub fn eventfd() -> io::Result<OwnedFd> {
    use rustix::event::EventfdFlags;
    effect(|| "eventfd".into())?;
    rustix::event::eventfd(0, EventfdFlags::CLOEXEC | EventfdFlags::NONBLOCK).map_err(os)
}

/// allocate_pty with posix_openpt: the master and the slave's name.
pub fn allocate_pty() -> io::Result<(OwnedFd, Vec<u8>)> {
    use rustix::pty::OpenptFlags;
    effect(|| "pty".into())?;
    let master = rustix::pty::openpt(OpenptFlags::RDWR | OpenptFlags::NOCTTY | OpenptFlags::CLOEXEC).map_err(os)?;
    rustix::pty::grantpt(&master).and_then(|()| rustix::pty::unlockpt(&master)).map_err(os)?;
    let name = rustix::pty::ptsname(&master, Vec::with_capacity(64)).map_err(os)?;
    Ok((master, name.into_bytes()))
}

/// The pty slave NAME, opened without becoming the controlling terminal.
pub fn open_tty(name: &[u8]) -> io::Result<OwnedFd> {
    effect(|| format!("terminal {}", String::from_utf8_lossy(name)))?;
    use rustix::fs::OFlags;
    let path = std::ffi::CString::new(name).map_err(|_| io::Error::from(io::ErrorKind::InvalidInput))?;
    rustix::fs::open(&*path, OFlags::RDWR | OFlags::NOCTTY | OFlags::CLOEXEC, rustix::fs::Mode::empty()).map_err(os)
}

// Sockets.

/// HOST:PORT's addresses, from the C library's resolver.
pub fn resolve(host: &str, port: u16) -> io::Result<Vec<SocketAddr>> {
    effect(|| format!("resolve {host}:{port}"))?;
    use std::net::ToSocketAddrs;
    (host, port).to_socket_addrs().map(Iterator::collect)
}

/// NAME's IP addresses as the C library's resolver gives them, one for each
/// datagram socket address (GNU's `network-lookup-address-info'): FAMILY
/// is `AF_UNSPEC', `AF_INET' or `AF_INET6'; NUMERIC treats NAME as a
/// numeric address (`AI_NUMERICHOST'), with no DNS.  Err is the
/// resolver's reason (`gai_strerror'), or the refusal in build mode.
pub fn lookup_addresses(name: &str, family: i32, numeric: bool) -> Result<Vec<std::net::IpAddr>, String> {
    effect(|| format!("resolve {name}")).map_err(|e| e.to_string())?;
    let c_name = std::ffi::CString::new(name).map_err(|_| "Name contains a NUL".to_string())?;
    sys::lookup_addresses(&c_name, family, numeric)
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
    // sockaddr_un's sun_path holds the name and its NUL.
    const SUN_PATH: usize = 108;
    if path.as_os_str().len() >= SUN_PATH {
        return Err(io::Error::from_raw_os_error(libc::ENAMETOOLONG));
    }
    let address = rustix::net::SocketAddrUnix::new(path).map_err(os)?;
    connect_in_progress(rustix::net::AddressFamily::UNIX, &address)
}

/// A close-on-exec, nonblocking stream socket of FAMILY to ADDRESS,
/// connected or (EINPROGRESS, or EINTR, which leaves a nonblocking connect
/// in progress) connecting.
fn connect_in_progress(family: rustix::net::AddressFamily, address: &impl rustix::net::addr::SocketAddrArg) -> io::Result<OwnedFd> {
    use rustix::net::{SocketFlags, SocketType};
    let socket = rustix::net::socket_with(family, SocketType::STREAM, SocketFlags::NONBLOCK | SocketFlags::CLOEXEC, None).map_err(os)?;
    match rustix::net::connect(&socket, address) {
        Ok(()) | Err(rustix::io::Errno::INPROGRESS | rustix::io::Errno::INTR) => Ok(socket),
        Err(error) => Err(os(error)),
    }
}

/// accept4 on the listening socket LISTENER: the connection, close-on-exec,
/// and the peer's raw address with its length.
pub fn accept(listener: RawFd) -> io::Result<(OwnedFd, libc::sockaddr_storage, libc::socklen_t)> {
    effect(|| format!("accept on {listener}"))?;
    sys::accept(listener)
}

/// connect_network_socket's nonblocking connect: a close-on-exec stream
/// socket to ADDR, connected or (EINPROGRESS) connecting.
pub fn connect_nonblocking(addr: SocketAddr) -> io::Result<OwnedFd> {
    effect(|| format!("connect {addr}"))?;
    let family = match addr { SocketAddr::V4(_) => rustix::net::AddressFamily::INET, SocketAddr::V6(_) => rustix::net::AddressFamily::INET6 };
    connect_in_progress(family, &addr)
}

#[cfg(test)]
mod tests {

    /// A copy of a development build names its tree in a file beside it:
    /// absolute, or relative to the file. No file, no tree.
    #[test]
    fn a_copied_executable_reads_the_tree_named_beside_it() {
        let dir = std::env::temp_dir().join(format!("romacs-runtime-record-{}", std::process::id()));
        std::fs::create_dir_all(dir.join("bin")).unwrap();
        let exe = dir.join("bin/romacs-vm-gui");
        assert_eq!(recorded_root(&exe), None);
        std::fs::write(dir.join("bin").join(RUNTIME_RECORD), "../tree\n").unwrap();
        assert_eq!(recorded_root(&exe), Some(dir.join("bin/../tree")), "a tree that is not there keeps the name given");
        std::fs::create_dir_all(dir.join("tree")).unwrap();
        assert_eq!(recorded_root(&exe), Some(std::fs::canonicalize(dir.join("tree")).unwrap()), "one that is there has its own name, with no parent steps");
        std::fs::write(dir.join("bin").join(RUNTIME_RECORD), "/somewhere/else\n").unwrap();
        assert_eq!(recorded_root(&exe), Some(PathBuf::from("/somewhere/else")));
        std::fs::write(dir.join("bin").join(RUNTIME_RECORD), "\n").unwrap();
        assert_eq!(recorded_root(&exe), None);
        std::fs::remove_dir_all(&dir).unwrap();
    }
    /// Only an executable in a target directory its tree was built into may
    /// assume that tree. The directory's list decides, not its shape:
    /// another build's target, and any other directory, give it nothing to
    /// assume. (Made-up directories only: this test's own executable may
    /// have been carried off by an archive, with its tree said outright.)
    #[test]
    fn only_an_executable_in_a_target_its_tree_built_into_assumes_that_tree() {
        let tree = build_root();
        let dir = std::env::temp_dir().join(format!("romacs-runtime-target-{}", std::process::id()));
        for sub in ["gate/deps", "gate/examples", "gate/.fingerprint", "gate/build"] {
            std::fs::create_dir_all(dir.join(sub)).unwrap();
        }
        let places = ["gate/romacs-vm-gui", "gate/deps/some_test-0123456789abcdef", "gate/examples/demo"];
        assert!(places.iter().all(|place| !in_build_target(&dir.join(place), &tree)), "a target's shape is no list");
        std::fs::write(dir.join("gate").join(BUILD_TREES), "/another/checkout\n").unwrap();
        assert!(places.iter().all(|place| !in_build_target(&dir.join(place), &tree)), "another build's target");
        std::fs::write(dir.join("gate").join(BUILD_TREES), format!("/another/checkout\n{}\n", tree.display())).unwrap();
        assert!(places.iter().all(|place| in_build_target(&dir.join(place), &tree)));
        assert!(!in_build_target(&dir.join("gate/elsewhere/romacs-vm-gui"), &tree));
        std::fs::remove_dir_all(&dir).unwrap();
    }
    /// A tree is the executable's own only with its generation: a directory
    /// that is no tree, a tree with no generation and a tree of another are
    /// each refused, by name.
    #[test]
    fn a_runtime_tree_must_be_of_the_executables_generation() {
        assert_eq!(runtime_tree(&build_root()), Ok(()));
        let dir = std::env::temp_dir().join(format!("romacs-runtime-tree-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        assert!(runtime_tree(&dir).unwrap_err().contains("not a Romacs runtime tree"));
        std::fs::create_dir_all(dir.join("lisp/gnu")).unwrap();
        std::fs::create_dir_all(dir.join("etc")).unwrap();
        assert!(runtime_tree(&dir).unwrap_err().contains("from before generations"));
        std::fs::write(dir.join("etc/romacs-runtime-epoch"), "0\n").unwrap();
        assert!(runtime_tree(&dir).unwrap_err().contains("generation 0"));
        std::fs::write(dir.join("etc/romacs-runtime-epoch"), RUNTIME_EPOCH).unwrap();
        assert_eq!(runtime_tree(&dir), Ok(()));
        std::fs::remove_dir_all(&dir).unwrap();
    }
    /// The build tree is a directory name with no `..` in it, so the
    /// runtime directories built from it are what they expand to.
    #[test]
    fn the_build_root_has_no_parent_steps() {
        let root = build_root();
        assert!(root.is_absolute(), "{}", root.display());
        assert!(!root.components().any(|c| matches!(c, std::path::Component::ParentDir)), "{}", root.display());
        assert!(root.join("lisp/gnu").is_dir() && root.join("etc").is_dir(), "{}", root.display());
    }
    /// A world built from a checkout keys only the kinds of the tree's
    /// entries, never their times, so editing the tree leaves it valid.
    #[test]
    fn source_directory_keys_no_times_of_the_tree() {
        enter_build(Pinned { clock: SystemTime::UNIX_EPOCH, pid: 0, seed: [0; 4] });
        begin_recipe();
        let found = source_directory();
        let trace = leave_build().expect("a build trace");
        let root = build_root();
        let watched = [root.join("crates/romacs-vm/src/prims"), root.join(".git")];
        let probes: Vec<_> = trace["records"].as_array().unwrap().iter()
            .filter(|r| r["kind"] == "probe" && watched.iter().any(|p| decoded(&r["path"]).as_deref() == Some(p.as_os_str().as_bytes())))
            .map(|r| r["probe"].as_str().unwrap_or("").to_owned())
            .collect();
        assert!(!probes.is_empty(), "the tree checks are recorded");
        assert!(probes.iter().all(|p| p == "IsDir" || p == "entry-type"), "{probes:?}, found {found:?}");
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
        std::fs::create_dir_all(prefix.join("share/romacs/lisp/gnu")).unwrap();
        assert_eq!(installed_root(&exe), Some(prefix.join("share/romacs")));
        std::fs::remove_dir_all(&prefix).unwrap();
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

// Inspecting a busy evaluator (inspect.rs). A request is a generation: the
// listener and SIGUSR2 raise REQUESTED, the evaluator's next poll answers
// with a snapshot and raises SERVED. The bytes are in the slot before any
// disk work, so a stalled store never delays a reply. Each process has one
// evaluator, and only a session's own process starts a listener.
static INSPECT_ENABLED: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
static INSPECT_REQUESTED: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
static INSPECT_SERVED: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
static INSPECT_SLOT: std::sync::Mutex<Option<InspectSnapshot>> = std::sync::Mutex::new(None);
static INSPECT_LISTENER: std::sync::Mutex<Option<InspectListener>> = std::sync::Mutex::new(None);
/// How long a client waits for the evaluator's next safe point.
const INSPECT_WAIT: std::time::Duration = std::time::Duration::from_secs(2);
/// How long a client has to send its line, and to take the reply. Absolute,
/// as one client at a time is served.
const INSPECT_IO: std::time::Duration = std::time::Duration::from_millis(500);
/// Snapshots waiting for the store; past it the oldest goes.
const INSPECT_PENDING: usize = 4;
/// The snapshot files kept.
const INSPECT_KEEP: usize = 8;

#[derive(Clone)]
struct InspectSnapshot {
    generation: u64,
    at: Instant,
    json: std::sync::Arc<str>,
}

/// One listener's store queue, so a stopped writer never takes a later
/// listener's snapshots.
#[derive(Default)]
struct InspectStore {
    queue: std::sync::Mutex<InspectQueue>,
    changed: std::sync::Condvar,
}

#[derive(Default)]
struct InspectQueue {
    pending: std::collections::VecDeque<InspectSnapshot>,
    stopped: bool,
}

impl InspectStore {
    fn lock(&self) -> std::sync::MutexGuard<'_, InspectQueue> {
        self.queue.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
    }
    fn push(&self, snapshot: InspectSnapshot) {
        let mut queue = self.lock();
        if queue.pending.len() >= INSPECT_PENDING { queue.pending.pop_front(); }
        queue.pending.push_back(snapshot);
        self.changed.notify_one();
    }
    fn stop(&self) {
        self.lock().stopped = true;
        self.changed.notify_all();
    }
    /// The next snapshot to save, or None once stopped.
    fn next(&self) -> Option<InspectSnapshot> {
        let mut queue = self.lock();
        loop {
            if queue.stopped { return None; }
            if let Some(snapshot) = queue.pending.pop_front() { return Some(snapshot); }
            queue = self.changed.wait(queue).unwrap_or_else(|poisoned| poisoned.into_inner());
        }
    }
}

struct InspectListener {
    path: PathBuf,
    /// The socket's device and inode: a path another process has since
    /// taken is not unlinked.
    identity: (u64, u64),
    stop: std::sync::Arc<std::sync::atomic::AtomicBool>,
    store: Option<std::sync::Arc<InspectStore>>,
}

fn inspect_lock<T>(mutex: &'static std::sync::Mutex<T>) -> std::sync::MutexGuard<'static, T> {
    mutex.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
}

/// Which thread answers inspections: the evaluator's, the one that started
/// listening (or, in a test, the one that raised the request).  Every VM in
/// the process polls, worker VMs too, and only that thread's may answer, so
/// a worker's backtrace is never taken for the evaluator's.  A count, not a
/// thread id: each registration takes the next one, and the thread keeps
/// its own copy, so the check is two relaxed loads.
static INSPECT_TARGET: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
thread_local! {
    static INSPECT_HERE: std::cell::Cell<u64> = const { std::cell::Cell::new(0) };
}

/// Make the current thread the one that answers inspections.
fn inspect_target_here() {
    let token = INSPECT_TARGET.fetch_add(1, std::sync::atomic::Ordering::SeqCst) + 1;
    INSPECT_HERE.with(|here| here.set(token));
}

/// A request this thread's evaluator has not answered. Two relaxed loads per
/// poll; whose thread answers is read only while a request waits.
#[inline(always)]
pub fn inspect_pending() -> bool {
    use std::sync::atomic::Ordering::Relaxed;
    INSPECT_SERVED.load(Relaxed) != INSPECT_REQUESTED.load(Relaxed)
        && INSPECT_HERE.with(std::cell::Cell::get) == INSPECT_TARGET.load(Relaxed)
}
/// Whether a listener runs, so that a native loop should poll for requests.
pub fn inspect_enabled() -> bool {
    INSPECT_ENABLED.load(std::sync::atomic::Ordering::Relaxed)
}
/// Raise a request, as a client does, for this thread's evaluator to answer.
#[cfg(test)]
pub fn inspect_request() {
    inspect_target_here();
    INSPECT_REQUESTED.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
}
/// Whether native loops poll for requests, as a listener makes them.
#[cfg(test)]
pub fn inspect_enable(on: bool) {
    INSPECT_ENABLED.store(on, std::sync::atomic::Ordering::Relaxed);
}
/// The JSON of the newest answer.
#[cfg(test)]
pub fn inspect_answer() -> Option<String> {
    inspect_lock(&INSPECT_SLOT).as_ref().map(|snapshot| snapshot.json.to_string())
}
/// The newest request, which a snapshot taken now answers.
pub fn inspect_generation() -> u64 {
    INSPECT_REQUESTED.load(std::sync::atomic::Ordering::SeqCst)
}
/// Publish JSON as the answer to GENERATION, then queue it for the store.
pub fn inspect_publish(generation: u64, json: String) {
    let snapshot = InspectSnapshot { generation, at: Instant::now(), json: json.into() };
    *inspect_lock(&INSPECT_SLOT) = Some(snapshot.clone());
    INSPECT_SERVED.store(generation, std::sync::atomic::Ordering::SeqCst);
    if let Some(store) = inspect_lock(&INSPECT_LISTENER).as_ref().and_then(|listener| listener.store.as_ref()) {
        store.push(snapshot);
    }
}

fn inspect_reply(answered: bool) -> String {
    let status = if answered { r#""status":"ok""# } else { r#""status":"busy","detail":"no safe point""# };
    match &*inspect_lock(&INSPECT_SLOT) {
        Some(s) => format!(r#"{{"v":1,{status},"generation":{},"age_ms":{},"snapshot":{}}}"#, s.generation, s.at.elapsed().as_millis(), s.json),
        None => format!(r#"{{"v":1,{status},"snapshot":null}}"#),
    }
}

/// The peer's uid, from the kernel's record of who connected.
fn peer_uid(stream: &std::os::unix::net::UnixStream) -> Option<u32> {
    rustix::net::sockopt::socket_peercred(stream).ok().map(|credentials| credentials.uid.as_raw())
}

/// The client's line, within DEADLINE however slowly it comes.
fn inspect_line(stream: &mut std::os::unix::net::UnixStream, deadline: Instant) -> Option<([u8; 16], usize)> {
    use std::io::Read;
    let mut line = [0u8; 16];
    let mut length = 0;
    while length < line.len() && !line[..length].contains(&b'\n') {
        let left = deadline.checked_duration_since(Instant::now()).filter(|left| !left.is_zero())?;
        stream.set_read_timeout(Some(left)).ok()?;
        match stream.read(&mut line[length..]) {
            Ok(n) if n > 0 => length += n,
            _ => return None,
        }
    }
    Some((line, length))
}

/// BYTES to the client, within DEADLINE however slowly it reads.
fn inspect_write(stream: &mut std::os::unix::net::UnixStream, mut bytes: &[u8], deadline: Instant) {
    use std::io::Write;
    while !bytes.is_empty() {
        let Some(left) = deadline.checked_duration_since(Instant::now()).filter(|left| !left.is_zero()) else { return };
        if stream.set_write_timeout(Some(left)).is_err() { return; }
        match stream.write(bytes) {
            Ok(n) if n > 0 => bytes = &bytes[n..],
            _ => return,
        }
    }
}

/// One client: the line `inspect', answered once. STOP ends the wait.
fn inspect_serve(mut stream: std::os::unix::net::UnixStream, stop: &std::sync::atomic::AtomicBool) {
    use std::sync::atomic::Ordering::SeqCst;
    if peer_uid(&stream) != Some(rustix::process::geteuid().as_raw()) { return; }
    let Some((line, length)) = inspect_line(&mut stream, Instant::now() + INSPECT_IO) else { return };
    if &line[..length] != b"inspect\n" { return; }
    let generation = INSPECT_REQUESTED.fetch_add(1, SeqCst) + 1;
    let deadline = Instant::now() + INSPECT_WAIT;
    while INSPECT_SERVED.load(SeqCst) < generation && Instant::now() < deadline && !stop.load(SeqCst) {
        std::thread::sleep(std::time::Duration::from_millis(2));
    }
    let mut reply = inspect_reply(INSPECT_SERVED.load(SeqCst) >= generation);
    reply.push('\n');
    inspect_write(&mut stream, reply.as_bytes(), Instant::now() + INSPECT_IO);
}

/// A store directory is used only if it is a real directory of ours that
/// no one else may read.
fn inspect_store_ready(store: &Path) -> bool {
    use std::os::unix::fs::{DirBuilderExt, MetadataExt};
    if std::fs::DirBuilder::new().recursive(true).mode(0o700).create(store).is_err() { return false; }
    std::fs::symlink_metadata(store).is_ok_and(|meta| {
        meta.is_dir() && meta.uid() == rustix::process::geteuid().as_raw() && meta.mode() & 0o077 == 0
    })
}

fn inspect_save(store: &Path, snapshot: &InspectSnapshot) -> io::Result<()> {
    use std::io::Write;
    use std::os::unix::fs::OpenOptionsExt;
    let pid = std::process::id();
    let time = SystemTime::now().duration_since(SystemTime::UNIX_EPOCH).map_or(0, |d| d.as_secs());
    let name = format!("inspect-{pid}-{}.json", snapshot.generation);
    let temporary = store.join(format!(".{name}.tmp"));
    let mut file = OpenOptions::new().write(true).create_new(true).mode(0o600).open(&temporary)?;
    write!(file, r#"{{"v":1,"pid":{pid},"time":{time},"generation":{},"snapshot":{}}}"#, snapshot.generation, snapshot.json)?;
    file.sync_all()?;
    std::fs::rename(&temporary, store.join(name))?;
    let mut saved: Vec<(SystemTime, PathBuf)> = std::fs::read_dir(store)?.filter_map(Result::ok)
        .filter(|entry| entry.file_name().to_string_lossy().starts_with("inspect-"))
        .filter_map(|entry| Some((entry.metadata().ok()?.modified().ok()?, entry.path())))
        .collect();
    saved.sort();
    let surplus = saved.len().saturating_sub(INSPECT_KEEP);
    for (_, path) in &saved[..surplus] { let _ = std::fs::remove_file(path); }
    Ok(())
}

fn inspect_store_thread(directory: PathBuf, store: std::sync::Arc<InspectStore>) {
    if !inspect_store_ready(&directory) { return; }
    while let Some(snapshot) = store.next() {
        let _ = inspect_save(&directory, &snapshot);
    }
}

/// Listen for inspect requests at DIRECTORY/inspect.sock, and keep snapshots
/// in STORE when given. DIRECTORY is private to this user (`romacs-mcp-start'
/// checks it). A socket no one answers on is a dead session's and is replaced.
pub fn inspect_listen(directory: &str, store: Option<&str>) -> io::Result<()> {
    use std::os::unix::fs::{MetadataExt, PermissionsExt};
    use std::os::unix::net::{UnixListener, UnixStream};
    use std::sync::atomic::Ordering::SeqCst;
    effect(|| "inspect listener".into())?;
    inspect_stop();
    let path = Path::new(directory).join("inspect.sock");
    match UnixStream::connect(&path) {
        Ok(_) => return Err(io::Error::from(io::ErrorKind::AddrInUse)),
        Err(error) if error.kind() == io::ErrorKind::ConnectionRefused => std::fs::remove_file(&path)?,
        Err(_) => {}
    }
    let listener = UnixListener::bind(&path)?;
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600))?;
    let meta = std::fs::symlink_metadata(&path)?;
    let stop = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
    let accepting = stop.clone();
    std::thread::Builder::new().name("romacs-inspect".into()).spawn(move || {
        for stream in listener.incoming() {
            if accepting.load(SeqCst) { return; }
            if let Ok(stream) = stream { inspect_serve(stream, &accepting); }
        }
    })?;
    let store = match store.map(PathBuf::from) {
        Some(directory) => {
            let store = std::sync::Arc::<InspectStore>::default();
            let storing = store.clone();
            std::thread::Builder::new().name("romacs-inspect-store".into()).spawn(move || inspect_store_thread(directory, storing))?;
            Some(store)
        }
        None => None,
    };
    *inspect_lock(&INSPECT_LISTENER) = Some(InspectListener { path, identity: (meta.dev(), meta.ino()), stop, store });
    inspect_target_here();
    INSPECT_ENABLED.store(true, SeqCst);
    Ok(())
}

/// Stop listening: the threads end and the socket goes, if it is still ours.
pub fn inspect_stop() {
    use std::os::unix::fs::MetadataExt;
    use std::sync::atomic::Ordering::SeqCst;
    let Some(listener) = inspect_lock(&INSPECT_LISTENER).take() else { return };
    INSPECT_ENABLED.store(false, SeqCst);
    listener.stop.store(true, SeqCst);
    if let Some(store) = &listener.store { store.stop(); }
    // The accept loop looks at its flag once a connection wakes it.
    let _ = std::os::unix::net::UnixStream::connect(&listener.path);
    let ours = std::fs::symlink_metadata(&listener.path).is_ok_and(|meta| (meta.dev(), meta.ino()) == listener.identity);
    if ours { let _ = std::fs::remove_file(&listener.path); }
}
