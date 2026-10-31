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

pub const FORMAT: &str = "romacs-init-plan 1";
/// Where a store keeps plans: `STORE/plans/<sha256 of the init's path>.json`.
const PLANS: &str = "plans";
/// The refusal kind a layer build lists for an entry it will not hold.
pub const REFUSAL: &str = "init-layer";

/// How the init asked for a load.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    /// `(require NAME FILE NOERROR)`.
    Require,
    /// An autoload's load of FILE, as `autoload-do-load` issues it.
    Autoload,
}
romacs_snapshot::snapshot_enum!(Kind { Kind::Require => (), Kind::Autoload => () });

/// One load the init issued at its own depth.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Entry {
    pub kind: Kind,
    /// The feature's name, or the autoload's file as given.
    pub name: String,
    /// `require`'s FILENAME argument, when given.
    pub file: Option<String>,
    pub noerror: bool,
    /// The `load-path` of the moment: an index into the plan's list.
    pub load_path: usize,
}
romacs_snapshot::snapshot_struct!(Entry { kind, name, file, noerror, load_path });

impl Entry {
    fn json(&self) -> Json {
        let kind = match self.kind { Kind::Require => "require", Kind::Autoload => "autoload" };
        json!({"kind": kind, "name": self.name, "file": self.file, "noerror": self.noerror, "load_path": self.load_path})
    }

    fn from_json(json: &Json) -> Option<Entry> {
        let kind = match json["kind"].as_str()? { "require" => Kind::Require, "autoload" => Kind::Autoload, _ => return None };
        Some(Entry { kind, name: json["name"].as_str()?.to_owned(), file: json["file"].as_str().map(str::to_owned),
                     noerror: json["noerror"].as_bool()?, load_path: json["load_path"].as_u64()? as usize })
    }

    /// The call, as the init made it.
    fn form(&self) -> String {
        let noerror = if self.noerror { "t" } else { "nil" };
        match (self.kind, &self.file) {
            (Kind::Require, Some(file)) => format!("(require (intern {}) {} {noerror})", lisp_string(&self.name), lisp_string(file)),
            (Kind::Require, None) => format!("(require (intern {}) nil {noerror})", lisp_string(&self.name)),
            (Kind::Autoload, _) => format!("(load {} {noerror} t)", lisp_string(&self.name)),
        }
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
}
romacs_snapshot::snapshot_struct!(Recording { depth, entries, load_paths });

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

    /// Where STORE keeps INIT's plan.
    pub fn file(store: &Path, init: &Path) -> PathBuf {
        use std::os::unix::ffi::OsStrExt;
        store.join(PLANS).join(format!("{}.json", sha256(init.as_os_str().as_bytes())))
    }

    /// INIT's plan in STORE, when one is there and is INIT's.
    pub fn read(store: &Path, init: &Path) -> Option<Plan> {
        let text = host::read_store_file(Self::file(store, init)).ok()?;
        Plan::from_json(&serde_json::from_str(&text).ok()?).filter(|plan| plan.init == init)
    }

    /// Keep this plan in STORE as its init's, replacing the one there whole.
    pub fn write(&self, store: &Path) -> std::io::Result<()> {
        use std::io::Write;
        let file = Self::file(store, &self.init);
        super::store::store_subdir(&store.join(PLANS))?;
        let (temporary, mut out) = host::temp_file(&file.with_extension("json.XXXXXX"))?;
        out.write_all(self.json().to_string().as_bytes())?;
        host::rename(&temporary, &file)
    }

    /// Whether PLAN replays as this one: the same entries over the same paths.
    pub fn same_loads(&self, other: &Plan) -> bool {
        self.entries == other.entries && self.load_paths == other.load_paths
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
        let same = |a: &Entry, pa: &[String], b: &Entry, pb: &[String]| a.kind == b.kind && a.name == b.name && a.file == b.file && a.noerror == b.noerror && pa == pb;
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

/// Whether NAME is a plan file's: a hex digest and `.json`.
pub(super) fn plan_file(name: &str) -> bool {
    name.strip_suffix(".json").is_some_and(|key| key.len() == 64 && key.bytes().all(|b| b.is_ascii_hexdigit()))
}

/// The loader's variables, which every entry reads and writes: no library's own.
impl Vm {
    /// Record the loads the init issues from here, at the depth the next
    /// load (the init's own) runs at, until `take_init_loads`.
    pub fn record_init_loads(&mut self) {
        self.loader.recording = Some(Recording { depth: self.loader.active_depth() + 1, ..Default::default() });
    }

    /// The loads recorded since `record_init_loads`, as INIT's plan, and
    /// stop recording. None when nothing was recording, or the recording
    /// broke (a `load-path` that is not all strings), or the init cannot be
    /// read.
    pub fn take_init_loads(&mut self, init: &Path) -> Option<Plan> {
        let recording = self.loader.recording.take()?;
        Plan::new(init, recording.entries, recording.load_paths)
    }

    /// A `require` or an autoload's load begins: noted when the init issues
    /// it at its own depth and a recording is on.
    pub(crate) fn note_init_load(&mut self, kind: Kind, name: String, file: Option<String>, noerror: bool) {
        let Some(recording) = &self.loader.recording else { return };
        if self.loader.active_depth() != recording.depth {
            return;
        }
        let Ok(load_path) = self.load_path_strings() else {
            self.loader.recording = None;
            return;
        };
        let recording = self.loader.recording.as_mut().expect("the recording");
        let at = match recording.load_paths.last() {
            Some(last) if *last == load_path => recording.load_paths.len() - 1,
            _ => { recording.load_paths.push(load_path); recording.load_paths.len() - 1 }
        };
        recording.entries.push(Entry { kind, name, file, noerror, load_path: at });
    }

    /// SYMBOL's name as interned, or None for a non-symbol.
    pub(crate) fn symbol_name_of(&self, symbol: crate::Value) -> Option<String> {
        let slot = symbol.index_if(7).and_then(|i| i.checked_sub(1))?;
        self.symbol_names.get(slot).cloned()
    }

    /// `load-path` as strings; nil, the working directory, as ".".
    fn load_path_strings(&mut self) -> crate::Result<Vec<String>> {
        let slot = self.slot("load-path");
        let value = self.variable_value(slot)?;
        self.list_values(value)?.into_iter()
            .map(|v| if v == crate::Value::NIL { Ok(".".to_owned()) } else { self.source_string(v).map(str::to_owned) })
            .collect()
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
    pub fn world_layer_load(&mut self, plan: &Plan) -> Vec<(usize, String)> {
        let kept = self.load_path_strings().unwrap_or_default();
        let mut added: Vec<String> = Vec::new();
        let mut refused = Vec::new();
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
            let result = self.set_load_path(&paths).and_then(|()| self.eval_source_raw(&entry.form()));
            for path in self.load_path_strings().unwrap_or_default() {
                if !plan.load_paths[entry.load_path].contains(&path) && !added.contains(&path) { added.push(path); }
            }
            let within = self.innermost_function();
            let why = match result {
                Err(error) => Some(format!("error: {error}{}", within.map(|f| format!(" (in {f})")).unwrap_or_default())),
                Ok(_) if host::refusals() > refusals => Some("fence".to_owned()),
                Ok(_) if self.timer_count() > timers && !host::layer_timers_allowed() => Some("timer".to_owned()),
                Ok(_) => None,
            };
            if let Some(why) = why {
                host::refuse(REFUSAL, json!({"entry": at, "name": entry.name, "why": why}));
                refused.push((at, why));
            }
        }
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
        Entry { kind: Kind::Require, name: name.into(), file: None, noerror: false, load_path: 0 }
    }

    #[test]
    fn a_plan_survives_its_json_and_its_digest_ignores_the_init_content() {
        let one = plan(vec![require("org"), Entry { kind: Kind::Autoload, name: "which-key".into(), file: None, noerror: true, load_path: 0 }]);
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
        let auto = |name: &str| Entry { kind: Kind::Autoload, name: name.into(), file: None, noerror: false, load_path: 0 };
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
        let named = Entry { kind: Kind::Require, name: "x".into(), file: Some("a \"q\"".into()), noerror: true, load_path: 0 };
        assert_eq!(named.form(), r#"(require (intern "x") "a \"q\"" t)"#);
        let auto = Entry { kind: Kind::Autoload, name: "/l/w.el".into(), file: None, noerror: false, load_path: 0 };
        assert_eq!(auto.form(), r#"(load "/l/w.el" nil t)"#);
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
