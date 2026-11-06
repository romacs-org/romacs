//! The init layer: a world that also holds the libraries a user's init
//! loads, so a launch restores them instead of loading them from source.
//!
//! What the init loads is recorded, not guessed: while the init runs
//! (`record_init_loads`), each `require` and each autoload the init itself
//! issues, at its own load depth, is noted with the `load-path` of the
//! moment, whether or not it loaded anything. Those entries, with the
//! init's path and content digest, are the init's plan (`Plan`), kept in
//! the store by the init's path. A layer build replays the plan after the
//! chain (`world_layer_load`): the same calls, each under its recorded
//! `load-path`, resolve and read the same files, which the build records
//! as keyed facts, so a library edited, added or removed, or a package
//! updated, makes the layer stale, and a launch boots from the base world
//! instead and records the plan again. The layer's key binds the base
//! world's and the entries, so another init, `load-path` or build keys
//! another layer; the init's digest is checked at the launch, so an edited
//! init never restores a layer planned from the old one.
//!
//! The fence (`world`) keeps the layer to what the libraries define. A
//! library's `defcustom`, `defvar`, `defconst` or `defvar-local` whose
//! value reads a per-process fact (`executable-find`, a relative
//! `expand-file-name`, a delayed variable) is what GNU evaluates in each
//! process: the build keeps the form (`romacs--world-reseed-forms`) and
//! restore evaluates it again, as GNU's delayed init does for its dumped
//! options, and a later definition reading that variable is kept too. Any
//! other form's read of such a fact, an effect, a signal, or a timer left
//! behind refuses the entry by name, and the build publishes its refusal;
//! the next launch plans without those entries, which load from source as
//! they always did. After the libraries the builder runs the startup
//! function again, as a restored process does after them, so startup's
//! inputs are the state they left; what it computed before them it must
//! compute again, or the build refuses.
//!
//! A library's load-time code may read a variable the init set before
//! requiring it; the layer loads the library before the init, so such a
//! read sees the standard value instead. That is the one semantic the
//! layer does not keep, as a dumped GNU would not either; `ROMACS_WORLD=off`
//! or no plan file gives the source order.
use crate::{Vm, host};
use serde_json::{Value as Json, json};
use std::path::{Path, PathBuf};

pub const FORMAT: &str = "romacs-init-plan 2";
/// Where a store keeps plans: `STORE/plans/<sha256 of the init's path>.json`.
const PLANS: &str = "plans";
/// The refusal kind a layer build lists for an entry it will not hold.
pub const REFUSAL: &str = "init-layer";

/// The functions that read a secret: a declared library whose load or
/// configuration calls one is refused, so the layer never holds what one
/// returns, and the library keeps its first-use load, where the call
/// happens in the session. The build's confinement already denies the
/// files and processes behind them; this names the reason.
const SECRET_READERS: [&str; 17] = [
    "auth-source-search", "auth-source-pick-first-password", "auth-source-user-or-password",
    "auth-source-user-and-password", "auth-source-pass-get", "auth-source-pass-parse-entry",
    "auth-source-pass--read-entry", "auth-source-pass-entries", "epg-decrypt-file", "epg-decrypt-string",
    "epg-start-decrypt", "epa-decrypt-file", "epa-decrypt-region", "epa-file-insert-file-contents",
    "plstore-open", "secrets-get-secret", "read-passwd",
];

/// How the init asked for a load, or declared a library it loads later.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Kind {
    /// `(require NAME FILE NOERROR)`.
    Require,
    /// An autoload's load of FILE, as `autoload-do-load` issues it; when
    /// declared, an `autoload` the init defined for FILE.
    Autoload,
    /// `eval-after-load` of a feature (or, with `file`, of a file) not
    /// loaded yet: a library the init configures for its first use, as
    /// `use-package`'s :config, :bind, :hook and :defer and
    /// `with-eval-after-load` do. Always declared.
    After,
}
romacs_snapshot::snapshot_enum!(Kind { Kind::Require => (), Kind::Autoload => (), Kind::After => () });

/// One load the init issued at its own depth, or one library it declared.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct Entry {
    pub kind: Kind,
    /// The feature's name, or the autoload's file as given.
    pub name: String,
    /// `require`'s FILENAME argument, when given; for `After`, the file
    /// `eval-after-load` named, as given, when it named one.
    pub file: Option<String>,
    pub noerror: bool,
    /// The `load-path` of the moment: an index into the plan's list.
    pub load_path: usize,
    /// The init only declared the library (an `autoload`, an
    /// `eval-after-load`), which GNU loads at its first use.
    pub declared: bool,
    /// `After`'s forms, each as printed, in the order the init gave them.
    pub forms: Vec<String>,
}
romacs_snapshot::snapshot_struct!(Entry { kind, name, file, noerror, load_path, declared, forms });

impl Entry {
    fn json(&self) -> Json {
        let kind = match self.kind { Kind::Require => "require", Kind::Autoload => "autoload", Kind::After => "after" };
        json!({"kind": kind, "name": self.name, "file": self.file, "noerror": self.noerror, "load_path": self.load_path,
               "declared": self.declared, "forms": self.forms})
    }

    fn from_json(json: &Json) -> Option<Entry> {
        let kind = match json["kind"].as_str()? { "require" => Kind::Require, "autoload" => Kind::Autoload, "after" => Kind::After, _ => return None };
        let forms = json["forms"].as_array()?.iter().map(|f| f.as_str().map(str::to_owned)).collect::<Option<_>>()?;
        Some(Entry { kind, name: json["name"].as_str()?.to_owned(), file: json["file"].as_str().map(str::to_owned),
                     noerror: json["noerror"].as_bool()?, load_path: json["load_path"].as_u64()? as usize,
                     declared: json["declared"].as_bool()?, forms })
    }

    /// The call, as the init made it; for a declared library, its load as
    /// its first use would make it. A declared file loads once, however
    /// many of its commands the init declared; an `After` entry's forms
    /// are registered first, so they run where GNU runs them, at the
    /// library's `provide` or the end of its file, and the library must be
    /// there: the layer holds what the init configured, or refuses it.
    fn form(&self) -> String {
        let noerror = if self.noerror { "t" } else { "nil" };
        let name = lisp_string(&self.name);
        match (self.kind, &self.file) {
            (Kind::Require, Some(file)) => format!("(require (intern {name}) {} {noerror})", lisp_string(file)),
            (Kind::Require, None) => format!("(require (intern {name}) nil {noerror})"),
            (Kind::Autoload, _) if self.declared => format!("(romacs--layer-load-declared {name})"),
            (Kind::Autoload, _) => format!("(load {name} {noerror} t)"),
            (Kind::After, file) => {
                let library = match file { Some(file) => lisp_string(file), None => format!("(intern {name})") };
                let forms: Vec<String> = self.forms.iter().map(|f| lisp_string(f)).collect();
                format!("(romacs--layer-load-configured {library} '({}))", forms.join(" "))
            }
        }
    }

    /// Whether this entry is the same declaration or load as OTHER. Two
    /// `After` entries of one library with as many forms are: a layer is
    /// used only while the init has the content its plan was recorded
    /// from, so the forms are the same code, though their printed text may
    /// name other generated symbols (`cl-gentemp' skips the names a layer
    /// interned).
    fn same(&self, other: &Entry) -> bool {
        let key = |e: &Entry| Entry { load_path: 0, forms: if e.kind == Kind::After { vec![String::new(); e.forms.len()] } else { e.forms.clone() }, ..e.clone() };
        key(self) == key(other)
    }
}

/// TEXT as a Lisp string literal.
fn lisp_string(text: &str) -> String {
    let mut out = String::with_capacity(text.len() + 2);
    out.push('"');
    for c in text.chars() {
        if matches!(c, '"' | '\\') { out.push('\\'); }
        out.push(c);
    }
    out.push('"');
    out
}

/// What the init loaded, recorded while it ran (`Vm::record_init_loads`).
#[derive(Default)]
pub(crate) struct Recording {
    /// The load depth of the init's own forms.
    depth: usize,
    entries: Vec<Entry>,
    load_paths: Vec<Vec<String>>,
    /// Holds on noting (`romacs--init-recording-hold`): an after-load
    /// form the init runs at once, whose loads are its library's.
    hold: usize,
    /// Each declaration's entry (`Recording::key`).
    index: std::collections::BTreeMap<String, usize>,
    /// The recording observes the calls of `SECRET_READERS`: once one ran,
    /// a form the init gives may hold what it returned.
    secrets: bool,
}
romacs_snapshot::snapshot_struct!(Recording { depth, entries, load_paths, hold, index, secrets });

impl Recording {
    /// The index key of a declaration of NAME (FILE) of KIND.
    fn key(kind: Kind, name: &str, file: Option<&str>) -> String {
        format!("{kind:?}\0{name}\0{}", file.unwrap_or("\0"))
    }
}

/// The printed form noted for a form the init gave after a secret reader
/// ran: never its text, which may hold the secret, and never replayed.
const AFTER_SECRET: &str = "#<after a secret reader>";

/// An init's plan: the loads it issued, in order, each under the
/// `load-path` of its moment, bound to the init's path and content.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Plan {
    pub init: PathBuf,
    /// SHA-256 of the init file as the plan was recorded.
    pub init_digest: String,
    pub entries: Vec<Entry>,
    pub load_paths: Vec<Vec<String>>,
    /// The names in each load-path directory's parent as the plan was
    /// recorded: a package version installed beside one changes what the
    /// init selects without touching what the layer read.
    pub siblings: Vec<(String, Vec<String>)>,
}

fn sha256(data: &[u8]) -> String {
    use sha2::Digest;
    sha2::Sha256::digest(data).iter().map(|b| format!("{b:02x}")).collect()
}

/// The digest of INIT's content, or None when it cannot be read.
fn init_digest(init: &Path) -> Option<String> {
    host::read_file(init).ok().map(|bytes| sha256(&bytes))
}

/// The names in DIRECTORY, sorted; a directory that cannot be listed has none.
fn listing(directory: &Path) -> Vec<String> {
    let mut names: Vec<String> = host::read_dir(directory).ok().into_iter().flatten()
        .filter_map(|entry| entry.ok().map(|e| e.file_name().to_string_lossy().into_owned())).collect();
    names.sort();
    names
}

/// The parents of DIRECTORIES, each once, with their listings.
fn siblings(directories: &[PathBuf]) -> Vec<(String, Vec<String>)> {
    let mut parents: Vec<PathBuf> = Vec::new();
    for parent in directories.iter().filter_map(|d| d.parent().map(Path::to_owned)) {
        if !parents.contains(&parent) { parents.push(parent); }
    }
    parents.into_iter().map(|parent| (parent.display().to_string(), listing(&parent))).collect()
}

impl Plan {
    /// The plan of INIT from a recording: ENTRIES over LOAD_PATHS, with the
    /// load-path directories' siblings as they are now. None when the init
    /// cannot be read.
    pub fn new(init: &Path, entries: Vec<Entry>, load_paths: Vec<Vec<String>>) -> Option<Plan> {
        let mut plan = Plan { init: init.to_owned(), init_digest: init_digest(init)?, entries, load_paths, siblings: Vec::new() };
        plan.siblings = siblings(&plan.directories());
        Some(plan)
    }

    /// The digest of what a layer build replays: the entries and their
    /// load paths. The init's content is not in it: an edit that loads the
    /// same libraries keeps the layer.
    pub fn digest(&self) -> String {
        let replayed = json!({"entries": self.entries.iter().map(Entry::json).collect::<Vec<_>>(), "load_paths": self.load_paths});
        sha256(replayed.to_string().as_bytes())
    }

    /// The recipe lines a layer world adds to its base's.
    pub fn recipe(&self) -> Vec<String> {
        vec![format!("init {}", self.init.display()), format!("init-layer {}", self.digest())]
    }

    /// Whether the init file still has the content the plan was recorded
    /// from, and each load-path directory's parent the same names.
    pub fn current(&self) -> bool {
        init_digest(&self.init).as_ref() == Some(&self.init_digest)
            && self.siblings.iter().all(|(parent, names)| listing(Path::new(parent)) == *names)
    }

    /// This plan without the entries at EXCLUDED, their load paths kept.
    pub fn without(&self, excluded: &[usize]) -> Plan {
        let entries = self.entries.iter().enumerate().filter(|(i, _)| !excluded.contains(i)).map(|(_, e)| e.clone()).collect();
        Plan { entries, ..self.clone() }
    }

    /// Every directory on any entry's `load-path`, each once: what a layer
    /// build reads beneath.
    pub fn directories(&self) -> Vec<PathBuf> {
        let mut dirs: Vec<PathBuf> = Vec::new();
        for path in self.load_paths.iter().flatten().map(PathBuf::from) {
            if !dirs.contains(&path) { dirs.push(path); }
        }
        dirs
    }

    pub fn json(&self) -> Json {
        json!({"format": FORMAT, "init": self.init.display().to_string(), "init_digest": self.init_digest,
               "entries": self.entries.iter().map(Entry::json).collect::<Vec<_>>(), "load_paths": self.load_paths,
               "siblings": self.siblings.iter().map(|(parent, names)| json!([parent, names])).collect::<Vec<_>>()})
    }

    pub fn from_json(json: &Json) -> Option<Plan> {
        if json["format"] != FORMAT { return None; }
        let load_paths: Vec<Vec<String>> = json["load_paths"].as_array()?.iter()
            .map(|p| p.as_array()?.iter().map(|s| s.as_str().map(str::to_owned)).collect())
            .collect::<Option<_>>()?;
        let entries: Vec<Entry> = json["entries"].as_array()?.iter().map(Entry::from_json).collect::<Option<_>>()?;
        if entries.iter().any(|e| e.load_path >= load_paths.len()) { return None; }
        let siblings = json["siblings"].as_array()?.iter().map(|pair| {
            let names = pair[1].as_array()?.iter().map(|n| n.as_str().map(str::to_owned)).collect::<Option<Vec<_>>>()?;
            Some((pair[0].as_str()?.to_owned(), names))
        }).collect::<Option<_>>()?;
        Some(Plan { init: PathBuf::from(json["init"].as_str()?), init_digest: json["init_digest"].as_str()?.to_owned(), entries, load_paths, siblings })
    }

    /// Where STORE keeps INIT's plan, or with LAST the plan of the last
    /// layer a launch could restore for it.
    fn path(store: &Path, init: &Path, last: bool) -> PathBuf {
        use std::os::unix::ffi::OsStrExt;
        let suffix = if last { ".last.json" } else { ".json" };
        store.join(PLANS).join(format!("{}{suffix}", sha256(init.as_os_str().as_bytes())))
    }

    /// Where STORE keeps INIT's plan.
    pub fn file(store: &Path, init: &Path) -> PathBuf {
        Self::path(store, init, false)
    }

    /// INIT's plan in STORE, when one is there and is INIT's.
    pub fn read(store: &Path, init: &Path) -> Option<Plan> {
        Self::read_path(&Self::file(store, init), init)
    }

    /// The plan of the last layer usable for INIT in STORE (`write_last`),
    /// with the digest of the plan it was resolved from.
    pub fn read_last(store: &Path, init: &Path) -> Option<(Plan, String)> {
        let text = host::read_store_file(Self::path(store, init, true)).ok()?;
        let json: Json = serde_json::from_str(&text).ok()?;
        let from = json["resolved_from"].as_str()?.to_owned();
        Some((Plan::from_json(&json).filter(|plan| plan.init == init)?, from))
    }

    fn read_path(file: &Path, init: &Path) -> Option<Plan> {
        let text = host::read_store_file(file).ok()?;
        Plan::from_json(&serde_json::from_str(&text).ok()?).filter(|plan| plan.init == init)
    }

    /// Keep this plan in STORE as its init's, replacing the one there whole.
    pub fn write(&self, store: &Path) -> std::io::Result<()> {
        self.write_path(&Self::file(store, &self.init), store)
    }

    /// Keep this plan in STORE as its init's last usable layer's, resolved
    /// from the plan of digest FROM: a launch restores it at once while
    /// that plan is the init's, and while a newer plan's layer builds
    /// (romacs-vm-gui world.rs).
    pub fn write_last(&self, store: &Path, from: &str) -> std::io::Result<()> {
        let mut json = self.json();
        json["resolved_from"] = json!(from);
        Self::write_json(&json, &Self::path(store, &self.init, true), store)
    }

    fn write_path(&self, file: &Path, store: &Path) -> std::io::Result<()> {
        Self::write_json(&self.json(), file, store)
    }

    fn write_json(json: &Json, file: &Path, store: &Path) -> std::io::Result<()> {
        use std::io::Write;
        let file = file.to_owned();
        super::store::store_subdir(&store.join(PLANS))?;
        let (temporary, mut out) = host::temp_file(&file.with_extension("json.XXXXXX"))?;
        out.write_all(json.to_string().as_bytes())?;
        host::rename(&temporary, &file)
    }

    /// Whether every load and declaration of this plan is one of OTHER's,
    /// under the same `load-path`: a layer of this plan holds nothing
    /// OTHER does not ask for.
    pub fn within(&self, other: &Plan) -> bool {
        let keyed = |plan: &Plan, entry: &Entry| (Entry { load_path: 0, forms: vec![String::new(); entry.forms.len()], ..entry.clone() },
                                                  plan.load_paths[entry.load_path].clone());
        let theirs: std::collections::HashSet<(Entry, Vec<String>)> = other.entries.iter().map(|e| keyed(other, e)).collect();
        self.entries.iter().all(|e| theirs.contains(&keyed(self, e)))
    }

    /// Whether PLAN replays as this one: the same entries (`Entry::same`)
    /// over the same paths.
    pub fn same_loads(&self, other: &Plan) -> bool {
        self.entries.len() == other.entries.len()
            && self.entries.iter().zip(&other.entries).all(|(a, b)| a.same(b) && self.load_paths[a.load_path] == other.load_paths[b.load_path])
    }

    /// Whether this plan, recorded now, must replace KEPT in the store: it
    /// loads differently, or the init's content or the load-path
    /// directories' siblings changed, which made KEPT not current.
    pub fn supersedes(&self, kept: Option<&Plan>) -> bool {
        kept.is_none_or(|kept| !kept.same_loads(self) || kept.init_digest != self.init_digest || kept.siblings != self.siblings)
    }

    /// This plan with what RECORDED adds: a session restored from this
    /// plan's layer issues no autoload for a library the layer holds, so
    /// RECORDED lacks those entries; they are kept, and each entry of
    /// RECORDED not in this plan is inserted where RECORDED has it. The
    /// init's digest is RECORDED's.
    pub fn merged(&self, recorded: &Plan) -> Plan {
        let path = |plan: &Plan, entry: &Entry| plan.load_paths[entry.load_path].clone();
        let same = |a: &Entry, pa: &[String], b: &Entry, pb: &[String]| a.same(b) && pa == pb;
        let mut merged: Vec<(Entry, Vec<String>)> = Vec::new();
        let mut cursor = 0;
        for entry in &recorded.entries {
            let paths = path(recorded, entry);
            match self.entries[cursor..].iter().position(|kept| same(kept, &path(self, kept), entry, &paths)) {
                Some(ahead) => {
                    merged.extend(self.entries[cursor..cursor + ahead + 1].iter().map(|kept| (kept.clone(), path(self, kept))));
                    cursor += ahead + 1;
                }
                None => merged.push((entry.clone(), paths)),
            }
        }
        merged.extend(self.entries[cursor..].iter().map(|kept| (kept.clone(), path(self, kept))));
        let mut load_paths: Vec<Vec<String>> = Vec::new();
        let entries = merged.into_iter().map(|(entry, paths)| {
            let at = load_paths.iter().position(|p| *p == paths).unwrap_or_else(|| { load_paths.push(paths); load_paths.len() - 1 });
            Entry { load_path: at, ..entry }
        }).collect();
        Plan { init: recorded.init.clone(), init_digest: recorded.init_digest.clone(), entries, load_paths, siblings: recorded.siblings.clone() }
    }
}

/// The libraries whose configuration is refused, by what loading them
/// leaves: features, and files named as `eval-after-load` names them
/// (`load-history-regexp`): without the Lisp suffixes, an absolute name by
/// its true name, a relative one as the tail of the loaded file's.
struct Deferred {
    features: std::collections::HashMap<String, String>,
    files: std::collections::HashMap<String, String>,
}

impl Deferred {
    /// The libraries ENTRIES, each a name and the file `eval-after-load`
    /// named, if it named one.
    fn new<'a>(entries: impl Iterator<Item = (&'a str, Option<&'a str>)>) -> Deferred {
        let mut deferred = Deferred { features: Default::default(), files: Default::default() };
        for (name, file) in entries {
            match file {
                None => { deferred.features.insert(name.to_owned(), name.to_owned()); }
                Some(file) => {
                    let file = if file.starts_with('/') { true_name(file) } else { file.to_owned() };
                    deferred.files.insert(bare(&file), name.to_owned());
                }
            }
        }
        deferred
    }

    /// The deferred library FEATURES or FILES (true names) hold, if any:
    /// each feature looked up, each file's name and each of its tails.
    fn loaded_by(&self, features: &[String], files: &[String]) -> Option<&str> {
        if let Some(name) = features.iter().find_map(|f| self.features.get(f)) { return Some(name); }
        if self.files.is_empty() { return None; }
        files.iter().find_map(|file| {
            let file = bare(file);
            let tails = file.match_indices('/').map(|(at, _)| &file[at + 1..]);
            std::iter::once(file.as_str()).chain(tails).find_map(|tail| self.files.get(tail))
        }).map(String::as_str)
    }
}

/// The builder's question of each entry's loads LOADED (the features it
/// provided, the files it loaded): the library of DEFERRED (names, and the
/// files `eval-after-load` named) it loaded, if any.
pub fn deferred_by(deferred: &[(String, Option<String>)], loaded: &[(Vec<String>, Vec<String>)]) -> Vec<Option<String>> {
    let deferred = Deferred::new(deferred.iter().map(|(name, file)| (name.as_str(), file.as_deref())));
    loaded.iter().map(|(features, files)| deferred.loaded_by(features, files).map(str::to_owned)).collect()
}

/// FILE's true name, as `file-truename` gives it: its directories'
/// symbolic links resolved even when FILE itself does not exist, as a
/// name without its Lisp suffix does not.
fn true_name(file: &str) -> String {
    let path = Path::new(file);
    host::canonicalize(path)
        .or_else(|_| match (path.parent(), path.file_name()) {
            (Some(parent), Some(name)) => host::canonicalize(parent).map(|dir| dir.join(name)),
            _ => Err(std::io::Error::from(std::io::ErrorKind::NotFound)),
        })
        .map_or_else(|_| file.to_owned(), |p| p.display().to_string())
}

/// NAME without the Lisp suffixes `load-history-regexp` allows.
fn bare(name: &str) -> String {
    let name = name.strip_suffix(".gz").unwrap_or(name);
    name.strip_suffix(".elc").or_else(|| name.strip_suffix(".el")).unwrap_or(name).to_owned()
}

/// Whether NAME is a plan file's: a hex digest and `.json`, or `.last.json`.
pub(super) fn plan_file(name: &str) -> bool {
    let key = name.strip_suffix(".last.json").or_else(|| name.strip_suffix(".json"));
    key.is_some_and(|key| key.len() == 64 && key.bytes().all(|b| b.is_ascii_hexdigit()))
}

/// The loader's variables, which every entry reads and writes: no library's own.
impl Vm {
    /// Record the loads the init issues from here, at the depth the next
    /// load (the init's own) runs at, until `take_init_loads`.
    pub fn record_init_loads(&mut self) {
        let secrets = !self.backtrace.observing();
        if secrets {
            let readers = SECRET_READERS.iter().map(|name| self.slot(name)).collect();
            self.backtrace.observe_only(readers);
        }
        self.loader.recording = Some(Recording { depth: self.loader.active_depth() + 1, secrets, ..Default::default() });
    }

    /// The loads recorded since `record_init_loads`, as INIT's plan, and
    /// stop recording. None when nothing was recording, or the recording
    /// broke (a `load-path` that is not all strings), or the init cannot be
    /// read.
    pub fn take_init_loads(&mut self, init: &Path) -> Option<Plan> {
        let recording = self.loader.recording.take()?;
        if recording.secrets && self.backtrace.observing() { self.backtrace.observed(); }
        let excluded = self.init_layer_exclusions();
        let entries = recording.entries.into_iter()
            .filter(|e| !excluded.iter().any(|x| *x == e.name || e.file.as_ref() == Some(x)))
            .collect();
        Plan::new(init, entries, recording.load_paths)
    }

    /// The names `romacs-init-layer-exclude` lists, as the init left it:
    /// features as their names, files as given.
    fn init_layer_exclusions(&mut self) -> Vec<String> {
        let slot = self.slot("romacs-init-layer-exclude");
        let Ok(list) = self.variable_value(slot) else { return Vec::new() };
        if list == crate::Value::UNBOUND { return Vec::new(); }
        self.list_values(list).unwrap_or_default().into_iter()
            .filter_map(|v| self.symbol_name_of(v).or_else(|| self.source_string_if(v).map(str::to_owned)))
            .collect()
    }

    /// Where the init stands, when a recording is on and the caller runs at
    /// the init's own depth: the index of the `load-path` of the moment.
    fn init_recording_point(&mut self) -> Option<usize> {
        let recording = self.loader.recording.as_ref()?;
        if recording.hold > 0 || self.loader.active_depth() != recording.depth {
            return None;
        }
        let Ok(load_path) = self.load_path_strings() else {
            self.loader.recording = None;
            return None;
        };
        let recording = self.loader.recording.as_mut().expect("the recording");
        Some(match recording.load_paths.last() {
            Some(last) if *last == load_path => recording.load_paths.len() - 1,
            _ => { recording.load_paths.push(load_path); recording.load_paths.len() - 1 }
        })
    }

    /// Hold noting (ON), or release one hold: while the init runs an
    /// after-load form at once, for a library loaded already, what the form
    /// loads is the library's configuration's, as when the form runs inside
    /// the library's load; so a launch notes the same whether its layer
    /// loaded the library or not.
    pub(crate) fn hold_init_recording(&mut self, on: bool) {
        if let Some(recording) = self.loader.recording.as_mut() {
            recording.hold = if on { recording.hold + 1 } else { recording.hold.saturating_sub(1) };
        }
    }

    /// A `require` or an autoload's load begins: noted when the init issues
    /// it at its own depth and a recording is on.
    pub(crate) fn note_init_load(&mut self, kind: Kind, name: String, file: Option<String>, noerror: bool) {
        let Some(at) = self.init_recording_point() else { return };
        let recording = self.loader.recording.as_mut().expect("the recording");
        recording.entries.push(Entry { kind, name, file, noerror, load_path: at, declared: false, forms: Vec::new() });
    }

    /// The init defined an autoload from FILE: a library it declared, noted
    /// once per file.
    pub(crate) fn note_init_declared(&mut self, file: String) {
        let Some(at) = self.init_recording_point() else { return };
        let recording = self.loader.recording.as_mut().expect("the recording");
        let key = Recording::key(Kind::Autoload, &file, None);
        if !recording.index.contains_key(&key) {
            recording.index.insert(key, recording.entries.len());
            recording.entries.push(Entry { kind: Kind::Autoload, name: file, file: None, noerror: true, load_path: at, declared: true, forms: Vec::new() });
        }
    }

    /// The position the init's next `eval-after-load` form for NAME (FILE
    /// as `note_init_after_load`) takes among that library's, when the
    /// init itself makes the call and a recording is on.
    pub(crate) fn init_after_load_ordinal(&mut self, name: &str, file: Option<&str>) -> Option<usize> {
        self.init_recording_point()?;
        let recording = self.loader.recording.as_ref()?;
        Some(recording.index.get(&Recording::key(Kind::After, name, file)).map_or(0, |&at| recording.entries[at].forms.len()))
    }

    /// The init gave `eval-after-load` FORM, as printed, for NAME (a
    /// feature's name, or with FILE the file it named) not loaded yet:
    /// noted with that library's entry, made at its first such call.
    pub(crate) fn note_init_after_load(&mut self, name: String, file: Option<String>, form: String) {
        let Some(at) = self.init_recording_point() else { return };
        let secret = self.loader.recording.as_ref().is_some_and(|r| r.secrets) && self.backtrace.observed_any();
        let form = if secret { AFTER_SECRET.to_owned() } else { form };
        let recording = self.loader.recording.as_mut().expect("the recording");
        let key = Recording::key(Kind::After, &name, file.as_deref());
        match recording.index.get(&key) {
            Some(&entry) => recording.entries[entry].forms.push(form),
            None => {
                recording.index.insert(key, recording.entries.len());
                recording.entries.push(Entry { kind: Kind::After, name, file, noerror: true, load_path: at, declared: true, forms: vec![form] });
            }
        }
    }

    /// SYMBOL's name as interned, or None for a non-symbol.
    pub(crate) fn symbol_name_of(&self, symbol: crate::Value) -> Option<String> {
        let slot = symbol.index_if(7).and_then(|i| i.checked_sub(1))?;
        self.symbol_names.get(slot).cloned()
    }

    /// `load-path` as strings; nil, the working directory, as ".".
    pub(crate) fn load_path_strings(&mut self) -> crate::Result<Vec<String>> {
        let slot = self.slot("load-path");
        let value = self.variable_value(slot)?;
        self.list_values(value)?.into_iter()
            .map(|v| if v == crate::Value::NIL { Ok(".".to_owned()) } else { self.source_string(v).map(str::to_owned) })
            .collect()
    }

    /// The first function that reads a secret (`SECRET_READERS`) called
    /// since `observe`, which ends here.
    fn secret_call(&mut self) -> Option<String> {
        // Startup's own observation, begun and ended inside the entry, took
        // it over: its reads are the fence's to judge.
        if !self.backtrace.observing() { return None; }
        let mut calls = self.backtrace.observed();
        calls.sort_unstable();
        calls.dedup();
        calls.into_iter().map(|slot| self.symbol_names[slot].as_str()).find(|name| SECRET_READERS.contains(name)).map(str::to_owned)
    }

    /// `features` as it is, rooted, and how many files the loader has
    /// loaded: `features` grows at its front and the loader's files at
    /// their end, so what an entry loads is what is new past these marks
    /// after it (`loads_since`).
    fn load_heads(&mut self) -> (crate::gc::RootFrame, usize) {
        let slot = self.slot("features");
        let features = self.variable_value(slot).unwrap_or(crate::Value::NIL);
        (self.root_frame(&[features]), self.loader.files.len())
    }

    /// The features provided and the files loaded since HEADS, walking only
    /// what is new.
    fn loads_since(&mut self, (heads, files): (crate::gc::RootFrame, usize)) -> (Vec<String>, Vec<String>) {
        let old = self.rooted(&heads, 0);
        let slot = self.slot("features");
        let mut list = self.variable_value(slot).unwrap_or(crate::Value::NIL);
        let mut features = Vec::new();
        while list != old && let Ok((feature, rest)) = self.cons_pair(list) {
            features.extend(self.symbol_name_of(feature));
            list = rest;
        }
        self.drop_frame(heads);
        let files = self.loader.files.get(files..).unwrap_or_default().iter().map(|f| f.display().to_string()).collect();
        (features, files)
    }

    fn timer_count(&mut self) -> i64 {
        self.eval_source_raw("(+ (if (boundp 'timer-list) (length timer-list) 0) (if (boundp 'timer-idle-list) (length timer-idle-list) 0))")
            .ok().and_then(|v| v.as_integer().ok()).unwrap_or(0)
    }

    /// Replay PLAN in a layer build: each entry under its recorded
    /// `load-path`, then `load-path` as it was, with whatever the libraries
    /// added ahead of it, then the startup function again. An entry that
    /// signals, reads a fenced fact outside a definition form or has an
    /// effect (the build's refusals grow), or leaves a timer behind is
    /// refused by its index and name (`REFUSAL`); the build goes on, so one
    /// build names every such entry. A definition form that reads a
    /// per-process fact is kept for restore instead (`world_form_done`).
    /// The refused entries.
    pub fn world_layer_load(&mut self, plan: &Plan, deferred: &[Entry]) -> Vec<(usize, String)> {
        let kept = self.load_path_strings().unwrap_or_default();
        let mut added: Vec<String> = Vec::new();
        let mut refused = Vec::new();
        // What each entry loaded: the features it provided and the files.
        let mut loaded: Vec<(Vec<String>, Vec<String>)> = Vec::with_capacity(plan.entries.len());
        host::layer_begin();
        // As the init's libraries find the live session: interactive
        // (rlisp/romacs-gui-loadup.el), with the init named before it loads
        // (romacs-vm-gui user_init.rs). The display stays the chain's.
        let directory = plan.init.parent().map(|d| format!("{}/", d.display())).unwrap_or_default();
        let _ = self.eval_source_raw(&format!("(setq noninteractive nil user-init-file {} user-emacs-directory {})",
                                              lisp_string(&plan.init.display().to_string()), lisp_string(&directory)));
        for (at, entry) in plan.entries.iter().enumerate() {
            let paths: Vec<PathBuf> = plan.load_paths[entry.load_path].iter().map(PathBuf::from).collect();
            let (refusals, timers) = (host::refusals(), self.timer_count());
            let heads = self.load_heads();
            host::layer_declared(entry.declared);
            let observed = entry.declared && !self.backtrace.observing();
            if observed { self.backtrace.observe(); }
            let result = self.set_load_path(&paths).and_then(|()| self.eval_source_raw(&entry.form()));
            let secret = if observed { self.secret_call() } else { None };
            for path in self.load_path_strings().unwrap_or_default() {
                if !plan.load_paths[entry.load_path].contains(&path) && !added.contains(&path) { added.push(path); }
            }
            let within = self.innermost_function();
            let why = match result {
                _ if secret.is_some() => secret.map(|f| format!("secret: {f}")),
                Err(error) => Some(format!("error: {error}{}", within.map(|f| format!(" (in {f})")).unwrap_or_default())),
                Ok(_) if host::refusals() > refusals => Some("fence".to_owned()),
                Ok(_) if self.timer_count() > timers && !host::layer_timers_allowed() => Some("timer".to_owned()),
                Ok(_) => None,
            };
            loaded.push(self.loads_since(heads));
            if let Some(why) = why {
                host::refuse(REFUSAL, json!({"entry": at, "name": entry.name, "why": why}));
                refused.push((at, why));
            }
        }
        // A library whose configuration is refused, in this build or an
        // earlier one of the plan (DEFERRED), stays deferred whole: a
        // declared entry that loaded it anyway (an `autoload' of one of its
        // commands, a library requiring it) is refused too, or the init
        // would run that configuration at every start.
        let refused_after: Vec<&Entry> = refused.iter().map(|(at, _)| &plan.entries[*at]).filter(|e| e.kind == Kind::After).collect();
        let deferred = Deferred::new(deferred.iter().chain(refused_after).map(|e| (e.name.as_str(), e.file.as_deref())));
        let mut held: std::collections::HashSet<usize> = refused.iter().map(|(at, _)| *at).collect();
        for (at, entry) in plan.entries.iter().enumerate() {
            if !entry.declared || held.contains(&at) { continue; }
            let (features, files) = &loaded[at];
            let Some(library) = deferred.loaded_by(features, files) else { continue };
            let why = format!("loads {library}, whose configuration is refused");
            host::refuse(REFUSAL, json!({"entry": at, "name": entry.name, "why": why}));
            held.insert(at);
            refused.push((at, why));
        }
        host::layer_declared(false);
        let restored: Vec<PathBuf> = added.iter().chain(kept.iter().filter(|p| !added.contains(p))).map(PathBuf::from).collect();
        let _ = self.set_load_path(&restored);
        // The startup function runs again after the libraries, as a
        // restored process runs it: its inputs are the state they left,
        // and what it computes must be what it computed before them.
        // The function is the chain's batch startup: it computes what it
        // computed before, batch, and the live session's startup takes
        // the interactive values again at restore.
        host::layer_rerun(true);
        let rerun = self.eval_source_raw("(progn (setq noninteractive t) (funcall romacs-startup-function))");
        host::layer_rerun(false);
        host::layer_end();
        match rerun {
            Ok(_) => self.world_check_startup_outputs(),
            Err(error) => host::refuse("fence", json!({"access": "startup-rerun", "error": error})),
        }
        refused
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn plan(entries: Vec<Entry>) -> Plan {
        Plan { init: PathBuf::from("/h/init.el"), init_digest: "d".repeat(64), entries, load_paths: vec![vec!["/a".into(), "/b".into()]], siblings: vec![] }
    }

    fn require(name: &str) -> Entry {
        Entry { kind: Kind::Require, name: name.into(), file: None, noerror: false, load_path: 0, declared: false, forms: Vec::new() }
    }

    #[test]
    fn a_plan_survives_its_json_and_its_digest_ignores_the_init_content() {
        let one = plan(vec![require("org"), Entry { kind: Kind::Autoload, name: "which-key".into(), file: None, noerror: true, load_path: 0, declared: false, forms: Vec::new() }]);
        assert_eq!(Plan::from_json(&one.json()), Some(one.clone()));
        let edited = Plan { init_digest: "e".repeat(64), ..one.clone() };
        assert_eq!(edited.digest(), one.digest());
        assert_ne!(one.without(&[1]).digest(), one.digest());
        assert_eq!(one.without(&[0]).entries, vec![one.entries[1].clone()]);
        assert_eq!(one.directories(), [PathBuf::from("/a"), PathBuf::from("/b")]);
        assert!(Plan::from_json(&json!({"format": "other"})).is_none());
        let mut bad = one.json();
        bad["entries"][0]["load_path"] = json!(7);
        assert!(Plan::from_json(&bad).is_none(), "an entry past the load paths");
    }

    #[test]
    fn a_recorded_plan_merges_into_the_layers_keeping_what_the_layer_satisfied() {
        let auto = |name: &str| Entry { kind: Kind::Autoload, name: name.into(), file: None, noerror: false, load_path: 0, declared: false, forms: Vec::new() };
        let layer = plan(vec![auto("display-line-numbers"), require("savehist"), auto("which-key"), require("org"), require("eat")]);
        // Restored from the layer: no autoloads, and one new require between org and eat.
        let mut recorded = plan(vec![require("savehist"), require("org"), require("new"), require("eat")]);
        recorded.init_digest = "e".repeat(64);
        let merged = layer.merged(&recorded);
        assert_eq!(merged.entries.iter().map(|e| e.name.as_str()).collect::<Vec<_>>(),
                   ["display-line-numbers", "savehist", "which-key", "org", "new", "eat"]);
        assert_eq!(merged.init_digest, recorded.init_digest);
        assert!(layer.merged(&plan(vec![require("savehist"), require("org"), require("eat")])).same_loads(&layer));
        // The store's plan is replaced for a load, an init or a sibling change, and only those.
        let kept = plan(vec![require("org")]);
        assert!(!kept.supersedes(Some(&kept)) && kept.supersedes(None));
        assert!(plan(vec![require("eat")]).supersedes(Some(&kept)));
        assert!(Plan { init_digest: "e".repeat(64), ..kept.clone() }.supersedes(Some(&kept)));
        assert!(Plan { siblings: vec![("/".into(), vec!["x".into()])], ..kept.clone() }.supersedes(Some(&kept)));
        // A layer of a plan within another holds nothing the other does not ask for.
        assert!(kept.within(&layer) && kept.within(&kept) && !layer.within(&kept));
        assert!(!plan(vec![Entry { declared: true, ..require("org") }]).within(&layer));
        // Another load path makes another entry.
        let mut elsewhere = plan(vec![require("org")]);
        elsewhere.load_paths = vec![vec!["/z".into()]];
        let merged = layer.merged(&elsewhere);
        assert_eq!(merged.entries.len(), 6);
        assert_eq!(merged.load_paths.len(), 2);
    }

    #[test]
    fn an_entry_replays_as_the_call_the_init_made() {
        assert_eq!(require("org").form(), r#"(require (intern "org") nil nil)"#);
        let named = Entry { kind: Kind::Require, name: "x".into(), file: Some("a \"q\"".into()), noerror: true, load_path: 0, declared: false, forms: Vec::new() };
        assert_eq!(named.form(), r#"(require (intern "x") "a \"q\"" t)"#);
        let auto = Entry { kind: Kind::Autoload, name: "/l/w.el".into(), file: None, noerror: false, load_path: 0, declared: false, forms: Vec::new() };
        assert_eq!(auto.form(), r#"(load "/l/w.el" nil t)"#);
        let declared = Entry { declared: true, noerror: true, ..auto.clone() };
        assert_eq!(declared.form(), r#"(romacs--layer-load-declared "/l/w.el")"#);
        let after = Entry { kind: Kind::After, name: "notmuch".into(), file: None, noerror: true, load_path: 0, declared: true,
                            forms: vec!["(t progn (f \"x\"))".into(), "(nil . 1)".into()] };
        assert_eq!(after.form(), r#"(romacs--layer-load-configured (intern "notmuch") '("(t progn (f \"x\"))" "(nil . 1)"))"#);
        let file = Entry { file: Some("dired-x".into()), name: "dired-x".into(), ..after.clone() };
        assert!(file.form().starts_with(r#"(romacs--layer-load-configured "dired-x" '("#), "{}", file.form());
        assert!(!after.same(&Entry { forms: vec![], ..after.clone() }) && after.same(&Entry { load_path: 3, ..after.clone() }));
        assert!(plan_file(&format!("{}.json", "0".repeat(64))) && !plan_file("x.json") && !plan_file(&"0".repeat(64)));
    }

    #[test]
    fn the_init_loads_the_init_issues_are_recorded_at_its_depth_with_load_path() {
        let dir = std::env::temp_dir().join(format!("romacs-init-plan-{}", std::process::id()));
        std::fs::create_dir_all(dir.join("lisp")).unwrap();
        std::fs::write(dir.join("lisp/leaf.el"), "(provide 'leaf)\n").unwrap();
        std::fs::write(dir.join("lisp/branch.el"), "(require 'leaf)\n(provide 'branch)\n").unwrap();
        let init = dir.join("init.el");
        std::fs::write(&init, format!("(setq load-path (cons {} load-path))\n(require 'branch)\n(require 'leaf)\n(require 'branch \"branch\" t)\n",
                                      lisp_string(&dir.join("lisp").display().to_string()))).unwrap();
        let mut vm = Vm::new();
        vm.record_init_loads();
        vm.load_file(&init).unwrap();
        let plan = vm.take_init_loads(&init).expect("a plan");
        assert_eq!(plan.entries.iter().map(|e| e.name.as_str()).collect::<Vec<_>>(), ["branch", "leaf", "branch"],
                   "the nested require of leaf is branch's, not the init's; a no-op require is still the init's");
        assert_eq!(plan.entries[2].file.as_deref(), Some("branch"));
        assert!(plan.entries[2].noerror && !plan.entries[0].noerror);
        assert_eq!(plan.load_paths.len(), 1);
        assert_eq!(plan.load_paths[0][0], dir.join("lisp").display().to_string());
        assert!(plan.current());
        assert_eq!(plan.siblings, [(dir.display().to_string(), vec!["init.el".to_owned(), "lisp".to_owned()])]);
        // A package installed beside the lisp directory selects anew.
        std::fs::create_dir_all(dir.join("lisp-2")).unwrap();
        assert!(!plan.current());
        std::fs::remove_dir(dir.join("lisp-2")).unwrap();
        assert!(plan.current());
        std::fs::write(&init, "(require 'leaf)\n").unwrap();
        assert!(!plan.current());
        assert!(vm.take_init_loads(&init).is_none(), "taken once");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
