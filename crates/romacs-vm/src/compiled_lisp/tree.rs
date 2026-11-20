//! Compiling an installation's Lisp trees ahead of use, as GNU's `make`
//! compiles its `.elc` files before an install is used: every file of
//! the trees whose artifact does not hold is compiled by the same compile
//! process a load or the background worker runs, JOBS at a time, and the
//! store is maintained once at the end, keeping every artifact the run
//! stands for. An installer runs it before the build is used, so a
//! library's first load in a session finds its artifact.
//!
//! The compile children do what any compile does (the `compile_command`
//! policy, nested compiles of missing dependencies included); they only
//! publish without pruning (`NO_PRUNE`). A parent blocks while its nested
//! compile runs, so at most JOBS compile processes run at once.
//!
//! A file whose artifact holds already starts no compile process: the
//! tree's process checks the artifact's inputs as a session's load does,
//! on the `load-path' the first compile process said it searches. What it
//! cannot tell (no such `load-path', a library name under `~') goes to a
//! compile process, which tells.
use super::*;
use std::collections::{HashSet, VecDeque};
use std::sync::Mutex;

/// What became of one file.
pub(super) enum Outcome {
    /// Its artifact: whether it held already, and its status.
    Published { artifact: PathBuf, held: bool, status: store::Status },
    /// The compile process failed, or published nothing: why.
    Failed(String),
}

/// Compile every file of the configured installation's Lisp trees whose
/// artifact does not hold, JOBS at a time, then maintain the store once.
/// The summary, as JSON; Err when this process cannot compile into a
/// usable store, said before anything runs.
pub fn compile_tree(jobs: usize) -> std::result::Result<Json, String> {
    let started = host::monotonic();
    let configured = configured().ok_or("compiled Lisp is off in this process (ROMACS_COMPILED_LISP, ROMACS_WORLD)")?;
    let config = &configured.config;
    if config.compiler.is_empty() {
        return Err(format!("{}: this process may not compile into the store", config.store.display()));
    }
    private_dir(&config.store)?;
    if !store::room(&config.store) {
        return Err(format!("{}: no room on the store's filesystem", config.store.display()));
    }
    let mut files = tree_files(&config.runtime);
    // The longest compiles first: the run ends soonest.
    files.sort_by_key(|path| std::cmp::Reverse(host::metadata(path).map(|m| m.len()).unwrap_or(0)));
    let mut outcomes = Vec::with_capacity(files.len());
    let mut queue: VecDeque<PathBuf> = files.into();
    // The first compile runs alone: it makes the compile world when there
    // is none, so no two children build it, and says the `load-path' every
    // compile of the tree searches.
    let mut load_path = None;
    if let Some(first) = queue.pop_front() {
        let (outcome, searched) = compile_child(configured, &first);
        outcomes.push((first, outcome));
        load_path = searched;
        if configured.identity().is_none() {
            return Err(format!("no compile world in {}: the first compile made none (its log is {})",
                               config.store.display(), store::generation(&config.store).join("compile.log").display()));
        }
    }
    // A file whose artifact holds is decided here, as a session's load
    // decides it; a compile process runs for the rest.
    outcomes.extend(pool(queue, jobs, |path| {
        load_path.as_deref().and_then(|load_path| held(configured, path, load_path))
            .unwrap_or_else(|| compile_child(configured, path).0)
    }));
    // Every artifact the run stands for is used now, as far as pruning
    // goes, and kept by this maintenance whatever the budget.
    let keep: HashSet<PathBuf> = outcomes.iter().filter_map(|(source, outcome)| match outcome {
        Outcome::Published { artifact, .. } => {
            store::note_use(&config.store, artifact, source);
            Some(artifact.clone())
        }
        Outcome::Failed(_) => None,
    }).collect();
    let kept_bytes: u64 = keep.iter().filter_map(|path| host::metadata(path).ok()).map(|m| m.len()).sum();
    let pruned = store::prune(&config.store, store::BUDGET, &keep).map_err(|e| format!("{}: prune: {e}", config.store.display()))?;
    let lost: Vec<String> = keep.iter().filter(|path| store::open(path).is_err()).map(|p| p.display().to_string()).collect();
    let mut summary = json!({
        "store": config.store,
        "files": outcomes.len(),
        "compiled": 0, "compiled_before": 0, "held": 0,
        "source_only": [], "errors": [], "failed": [],
        "lost": lost,
        "artifact_bytes": kept_bytes,
        "over_budget": kept_bytes > store::BUDGET,
        "pruned": pruned.len(),
        "store_bytes": tree_bytes(&config.store),
        "seconds": host::monotonic().saturating_duration_since(started).as_secs_f64(),
    });
    for (source, outcome) in &outcomes {
        let file = source.display().to_string();
        if let Outcome::Published { held: true, .. } = outcome {
            summary["held"] = json!(summary["held"].as_u64().unwrap_or(0) + 1);
        }
        let (field, entry) = match outcome {
            Outcome::Published { held: true, status: store::Status::Compiled, .. } => ("compiled_before", None),
            Outcome::Published { status: store::Status::Compiled, .. } => ("compiled", None),
            Outcome::Published { status: store::Status::Source(why), .. } => ("source_only", Some(why.clone())),
            Outcome::Published { status: store::Status::Error(why), .. } => ("errors", Some(why.clone())),
            Outcome::Failed(why) => ("failed", Some(why.clone())),
        };
        match entry {
            None => summary[field] = json!(summary[field].as_u64().unwrap_or(0) + 1),
            Some(why) => summary[field].as_array_mut().expect("a list").push(json!({"file": file, "why": why})),
        }
    }
    Ok(summary)
}

/// WORK done on each of ITEMS by JOBS threads at once, in no set order.
pub(super) fn pool<T: Send, R: Send>(items: VecDeque<T>, jobs: usize, work: impl Fn(&T) -> R + Sync) -> Vec<(T, R)> {
    let queue = Mutex::new(items);
    let done = Mutex::new(Vec::new());
    std::thread::scope(|scope| {
        for _ in 0..jobs.max(1) {
            scope.spawn(|| loop {
                // The queue's lock is released before the work runs.
                let Some(item) = queue.lock().unwrap().pop_front() else { break };
                let result = work(&item);
                done.lock().unwrap().push((item, result));
            });
        }
    });
    done.into_inner().unwrap()
}

/// PATH's artifact, when one whose inputs hold on LOAD_PATH (a compile
/// process's) is in the store: what its compile process would answer,
/// decided without one. None when there is none, or a record needs the
/// compile process to tell (a library name under `~', its HOME's).
pub(super) fn held(configured: &Configured, path: &Path, load_path: &[String]) -> Option<Outcome> {
    let config = &configured.config;
    let digest = store::source_digest(&config.store, path).ok()?;
    let key = store::key(configured.identity()?, config.root.as_deref(), path, &digest);
    let roots: Vec<PathBuf> = load_path.iter().map(PathBuf::from).collect();
    let resolve = |name: &str, nosuffix: bool| {
        let name = Path::new(name);
        if name.starts_with("~") { return None; }
        let roots = if name.is_absolute() { vec![PathBuf::new()] } else { roots.clone() };
        Some(crate::loader::find_source(&roots, name, nosuffix).map(|(found, _)| found))
    };
    let (artifact, data) = store::lookup(&config.store, &key, |artifact| {
        inputs_hold(&config.store, artifact, &load_path.join("\n"), &resolve)
    })?;
    Some(Outcome::Published { artifact, held: true, status: data.status })
}

/// Compile PATH in a compile process of the tree's: what came of it, and
/// the `load-path' the process searched, when it said.
fn compile_child(configured: &Configured, path: &Path) -> (Outcome, Option<Vec<String>>) {
    let Some((program, args, mut environment)) = compile_command(configured, path, &[]) else {
        return (Outcome::Failed("not compiled by this process".into()), None);
    };
    environment.push((NO_PRUNE.into(), "1".into()));
    let Some(out) = run_compile(&configured.config.store, program, args, environment) else {
        return (Outcome::Failed("the compile process failed (see the compile log)".into()), None);
    };
    let load_path = out.lines().find_map(|line| line.strip_prefix("romacs-compile-load-path: "))
        .and_then(|text| serde_json::from_str(text).ok());
    let held = out.lines().any(|line| line == "romacs-compile-held");
    let Some(artifact) = out.lines().rev().find_map(|line| line.strip_prefix("romacs-compiled: ")).map(PathBuf::from) else {
        return (Outcome::Failed("the compile process published nothing".into()), load_path);
    };
    let outcome = match store::open(&artifact) {
        Ok(data) => Outcome::Published { artifact, held, status: data.status },
        Err(why) => Outcome::Failed(why),
    };
    (outcome, load_path)
}

/// The `.el` files of the trees ROOTS, each once by its canonical name.
fn tree_files(roots: &[PathBuf]) -> Vec<PathBuf> {
    let mut seen = HashSet::new();
    roots.iter()
        .flat_map(|root| lisp_files(root))
        .filter_map(|path| host::canonicalize(&path).ok())
        .filter(|path| seen.insert(path.clone()))
        .collect()
}

/// Refuse DIR unless it is a directory of this user's, private (0700),
/// and no symlink, as the world store's are.
fn private_dir(dir: &Path) -> std::result::Result<(), String> {
    use std::os::unix::fs::{MetadataExt, PermissionsExt};
    let meta = host::symlink_metadata(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    if !meta.file_type().is_dir() || meta.uid() != host::euid() || meta.permissions().mode() & 0o777 != 0o700 {
        return Err(format!("{}: not a directory private to this user (0700)", dir.display()));
    }
    Ok(())
}

/// The bytes of the files under DIR, whatever they are.
fn tree_bytes(dir: &Path) -> u64 {
    host::read_dir(dir).into_iter().flatten().flatten().map(|entry| {
        match host::symlink_metadata(&entry.path()) {
            Ok(meta) if meta.file_type().is_dir() => tree_bytes(&entry.path()),
            Ok(meta) => meta.len(),
            Err(_) => 0,
        }
    }).sum()
}
