//! `--world-trace OUT.json` (DESIGN-M1 §5, §13 step 2): run the recipe, the
//! preloads, harness and adapter, as a world build would, and write its read
//! trace. The process re-executes itself with the pinned per-process
//! environment, confined, in build mode; the child lists every refusal
//! rather than stopping at the first, writes the trace to OUT and exits 1 if
//! there was any.
use romacs_vm::host::{self, Pinned};
use romacs_vm::world::store::{self, Launch, World};
use romacs_vm::{Vm, world::sandbox};
use serde_json::json;
use std::ffi::OsString;
use std::path::{Path, PathBuf};

/// The pinned answers that no environment entry carries, until restore's
/// recipe digest derives them (§5 step 1).
const PID: u32 = 4242;
const SEED: [u8; 4] = [0; 4];

/// The child's environment: INVOKING without its per-process names, which
/// the pinned table under DIR supplies (§5 step 1). Keyed names keep their
/// values.
pub fn pinned_env(invoking: &[(OsString, OsString)], dir: &Path) -> Vec<(OsString, OsString)> {
    store::pinned_env(invoking, &dir.join("p"))
}

/// The directory of OUT's pinned table: `OUT.d/p/{cwd,tmp,bin}`, empty and
/// read-only.
fn pinned_dir(out: &Path) -> PathBuf {
    let mut dir = out.as_os_str().to_owned();
    dir.push(".d");
    PathBuf::from(dir)
}

/// The flags whose value names a file or directory.
const PATH_FLAGS: [&str; 8] = ["--load-path", "--preload", "--harness", "--plan", "--reporter", "--audit", "--world-trace", "--world-store"];

/// ARGS, flag and value pairs as `parse` reads them, with each relative
/// path value resolved against CWD: the child runs elsewhere.
fn absolute_args(args: &[OsString], cwd: &Path) -> Vec<OsString> {
    args.chunks(2).flat_map(|pair| match pair {
        [flag, value] if PATH_FLAGS.iter().any(|f| flag == f) => vec![flag.clone(), cwd.join(value).into_os_string()],
        other => other.to_vec(),
    }).collect()
}

/// The parent: make the pinned directories and OUT, run the child with the
/// pinned environment in `p/cwd`, and exit as it does. The child creates no
/// file: its effects refuse, and OUT exists.
pub fn trace(out: &Path) -> ! {
    let status = host::current_dir().and_then(|cwd| {
        let out = cwd.join(out);
        let dir = pinned_dir(&out);
        host::create_dir_all(dir.join("p"))?;
        for name in ["cwd", "tmp", "bin"] {
            let path = dir.join("p").join(name);
            if !host::probe(host::Probe::IsDir, &path) {
                host::create_dir(&path, 0o500)?;
            }
        }
        host::write(&out, b"")?;
        let mut args = absolute_args(&std::env::args_os().skip(1).collect::<Vec<_>>(), &cwd);
        args.extend([OsString::from("--world-trace-child"), dir.clone().into_os_string()]);
        host::command(host::current_exe()?)?
            .args(args).env_clear().envs(pinned_env(&host::env_snapshot(), &dir))
            .current_dir(dir.join("p/cwd")).status()
    });
    match status {
        Ok(status) => std::process::exit(status.code().unwrap_or(70)),
        Err(e) => {
            crate::emit(json!({"event": "setup", "ok": false, "phase": "world-trace", "error": e.to_string()}));
            std::process::exit(3)
        }
    }
}

/// The child, before the VM is made: confine to the RECIPE's roots, the
/// pinned table under DIR and the binary's own, writing only OUT; then
/// record.
pub fn begin(recipe: Vec<PathBuf>, out: &Path, dir: &Path) {
    if let Err(e) = sandbox::confine(&sandbox::build_roots(recipe, dir), &[out.to_owned()]) {
        crate::emit(json!({"event": "setup", "ok": false, "phase": "world-confine", "error": e.to_string()}));
        std::process::exit(3);
    }
    host::enter_build(Pinned { clock: host::clock(), pid: PID, seed: SEED });
}

/// The child, once the recipe ran: check the startup inputs, write the
/// trace to OUT and exit, 1 if anything was refused.
pub fn finish(vm: &mut Vm, out: &Path) -> ! {
    vm.world_check_inputs();
    let trace = host::leave_build().expect("a world trace is recording");
    let count = |list: &str| trace[list].as_array().map_or(0, Vec::len);
    let (records, refusals) = (count("records"), count("refusals"));
    let written = serde_json::to_vec_pretty(&trace).map_err(std::io::Error::other).and_then(|bytes| host::write(out, bytes));
    crate::emit(json!({"event": "world-trace", "out": out, "records": records, "refusals": refusals,
                       "error": written.as_ref().err().map(ToString::to_string)}));
    std::process::exit(if written.is_err() { 3 } else if refusals > 0 { 1 } else { 0 })
}

/// How many leading preloads a world holds: the loadup bootstrap and every
/// preload after it from the same Romacs tree, up to the first that is not
/// (a suite's setup file). None without the bootstrap first.
pub fn world_preloads(preload: &[PathBuf]) -> usize {
    let Some(root) = world_root(preload) else { return 0 };
    preload.iter().take_while(|p| p.starts_with(root)).count()
}

/// The tree the world's preloads are in: the runtime root of the bootstrap
/// preload, `<root>/lisp/romacs/romacs-loadup-bootstrap.el`.
fn world_root(preload: &[PathBuf]) -> Option<&Path> {
    preload.first()
        .filter(|p| p.file_name().is_some_and(|n| n == "romacs-loadup-bootstrap.el"))
        .and_then(|p| p.parent()?.parent()?.parent())
}

/// The load path the world is built with and keyed on: the run's entries in
/// its preloads' tree, when the others, the run's own (its test's
/// directory), all follow them, as run.py lays them out; a restored world
/// then takes the run's own after them (`run_load_path`), so worlds are
/// shared across runs. Any other layout, or a tree entry given twice, which
/// leaves where the run's own go unclear, keys and builds on the whole path.
/// A preload needing a directory outside its tree fails the build, and the
/// run boots from source.
fn world_load_path(path: &[PathBuf], preload: &[PathBuf]) -> Vec<PathBuf> {
    let Some(root) = world_root(preload) else { return path.to_vec() };
    let split = path.iter().position(|p| !p.starts_with(root)).unwrap_or(path.len());
    let own = &path[..split];
    let repeated = own.iter().enumerate().any(|(i, p)| own[..i].contains(p));
    if repeated || path[split..].iter().any(|p| p.starts_with(root)) {
        return path.to_vec();
    }
    own.to_vec()
}

/// A restored world's `load-path' made the run's: its own entries, those
/// the world was not built on, appended, as a fresh VM has them after the
/// world's (whose preloads leave `load-path' as given, `load_path_kept').
fn run_load_path(vm: &mut Vm, args: &crate::Args) -> Result<(), String> {
    let own = world_load_path(&args.load_path, &args.preload);
    let extra = &args.load_path[own.len()..];
    if extra.is_empty() {
        return Ok(());
    }
    let extra = extra.iter().map(|p| lisp_string(p)).collect::<Vec<_>>().join(" ");
    crate::eval(vm, &format!("(setq load-path (append load-path (list {extra})))")).map(drop).map_err(|e| e.1)
}

/// PATH as a Lisp string literal.
fn lisp_string(path: &Path) -> String {
    serde_json::to_string(&path.to_string_lossy()).unwrap_or_default()
}

/// Whether the world's preloads left `load-path' as OWN, as given: a world
/// whose preloads changed it is not published, since where a run's own
/// entries would go among the changes is unclear, and runs boot from source.
fn load_path_kept(vm: &mut Vm, own: &[PathBuf]) -> bool {
    let own = own.iter().map(|p| lisp_string(p)).collect::<Vec<_>>().join(" ");
    crate::eval(vm, &format!("(equal load-path (list {own}))")).ok()
        .and_then(|value| value.display().ok()).is_some_and(|shown| shown == "t")
}

/// The recipe of the runner's world (§7): every embedder step before the
/// world's end, in order.
fn recipe(args: &crate::Args) -> Vec<String> {
    let mut lines = vec!["consumer romacs-gnu-ert".to_owned(), "noninteractive t".into(), "geometry 80 25 1 1".into()];
    lines.extend(world_load_path(&args.load_path, &args.preload).iter().map(|p| format!("load-path {}", p.display())));
    lines.push("prelude".into());
    lines.extend(args.preload[..world_preloads(&args.preload)].iter().map(|p| format!("preload {}", p.display())));
    lines
}

/// The VM to load the rest of the chain into, the chain entries already
/// loaded and how many preloads they cover: a restored world's, or a fresh
/// VM's. A builder runs the world's preloads, publishes and exits here.
pub fn boot(args: &crate::Args) -> (Vm, Vec<serde_json::Value>, usize) {
    let count = world_preloads(&args.preload);
    let world = match &args.world_store {
        Some(store) if count > 0 => World::new(store.clone(), recipe(args)),
        Some(_) => {
            crate::emit(json!({"event": "world", "mode": "source", "reason": "no loadup bootstrap preload"}));
            return (crate::fresh_vm(args), vec![], 0);
        }
        None => return (crate::fresh_vm(args), vec![], 0),
    };
    let world = match world {
        Ok(world) => world,
        Err(e) => {
            crate::emit(json!({"event": "world", "mode": "source", "reason": e.to_string()}));
            return (crate::fresh_vm(args), vec![], 0);
        }
    };
    let started = host::monotonic();
    let builder_args = || {
        let cwd = host::current_dir().unwrap_or_default();
        absolute_args(&std::env::args_os().skip(1).collect::<Vec<_>>(), &cwd)
    };
    match world.launch::<()>(builder_args, false, || {}) {
        Launch::Restored { vm, manifest, .. } => {
            let mut vm = *vm;
            vm.world_reinit();
            if let Err(e) = crate::eval(&mut vm, "(romacs-world-restart)") {
                crate::emit(json!({"event": "setup", "ok": false, "phase": "world-restart", "error": e.1}));
                std::process::exit(3);
            }
            if let Err(error) = run_load_path(&mut vm, args) {
                crate::emit(json!({"event": "setup", "ok": false, "phase": "load-path", "error": error}));
                std::process::exit(3);
            }
            let chain = manifest["events"]["chain"].as_array().cloned().unwrap_or_default();
            crate::emit(json!({"event": "world", "mode": "restored", "key": manifest["key"],
                               "restart_ms": started.elapsed().as_secs_f64() * 1e3}));
            (vm, chain, count)
        }
        Launch::Build => build(&world, args, count),
        Launch::Chain { reason } => {
            crate::emit(json!({"event": "world", "mode": "source", "reason": reason}));
            (crate::fresh_vm(args), vec![], 0)
        }
    }
}

/// The builder: run the world's preloads in build mode, confined, then
/// publish the world and exit, 0 when it was published.
fn build(world: &World, args: &crate::Args, count: usize) -> ! {
    let roots = world.build_roots(world_load_path(&args.load_path, &args.preload).into_iter().chain(args.preload[..count].iter().cloned()).collect());
    if let Err(e) = world.begin_build(&roots) {
        crate::emit(json!({"event": "world-build", "ok": false, "error": e.to_string()}));
        std::process::exit(3);
    }
    let built = world.build(|| {
        let mut vm = crate::fresh_vm(&crate::Args { load_path: world_load_path(&args.load_path, &args.preload), ..args.clone() });
        let mut chain = vec![];
        if !crate::load_chain(&mut vm, args, 0, count, &mut chain) || chain.iter().any(|e| e["loaded"] != true) {
            return Err(json!({"chain": chain}).to_string());
        }
        if !load_path_kept(&mut vm, &world_load_path(&args.load_path, &args.preload)) {
            return Err(json!({"chain": chain, "error": "the preloads changed load-path"}).to_string());
        }
        Ok((vm, (), json!({"chain": chain})))
    });
    match built {
        Ok(len) => {
            crate::emit(json!({"event": "world-build", "ok": true, "image_bytes": len}));
            std::process::exit(0)
        }
        Err(error) => {
            crate::emit(json!({"event": "world-build", "ok": false, "error": error}));
            std::process::exit(1)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_world_holds_the_bootstrap_and_its_tree() {
        let paths = |list: &[&str]| list.iter().map(PathBuf::from).collect::<Vec<_>>();
        let tree = paths(&["/r/lisp/romacs/romacs-loadup-bootstrap.el", "/r/lisp/gnu/help-fns.el", "/r/lisp/romacs/romacs-cl-print.el", "/case/setup.el", "/r/lisp/romacs/x.el"]);
        assert_eq!(world_preloads(&tree), 3);
        assert_eq!(world_preloads(&paths(&["/r/lisp/romacs/x.el"])), 0);
        assert_eq!(world_preloads(&[]), 0);
    }

    #[test]
    fn the_world_keys_on_its_trees_load_path_only_before_the_runs_own() {
        let paths = |list: &[&str]| list.iter().map(PathBuf::from).collect::<Vec<_>>();
        let preload = paths(&["/r/lisp/romacs/romacs-loadup-bootstrap.el"]);
        // run.py's layout: the tree's entries, then the test's directory.
        assert_eq!(world_load_path(&paths(&["/r/lisp/gnu", "/r/lisp/romacs", "/t/run-1"]), &preload), paths(&["/r/lisp/gnu", "/r/lisp/romacs"]));
        // Another layout keys on the whole path, whose order a restore keeps.
        let before = paths(&["/t/run-1", "/r/lisp/gnu", "/r/lisp/romacs"]);
        assert_eq!(world_load_path(&before, &preload), before);
        let between = paths(&["/r/lisp/gnu", "/t/run-1", "/r/lisp/romacs"]);
        assert_eq!(world_load_path(&between, &preload), between);
        // A tree entry given twice: where the run's own go is unclear.
        let twice = paths(&["/r/a", "/r/b", "/r/a", "/t/run-1"]);
        assert_eq!(world_load_path(&twice, &preload), twice);
        // No bootstrap preload, no world tree: the whole path.
        assert_eq!(world_load_path(&paths(&["/r/lisp/gnu", "/t"]), &[]), paths(&["/r/lisp/gnu", "/t"]));
    }

    #[test]
    fn pinned_env_replaces_per_process_names_and_keeps_keyed_ones() {
        let env = |pairs: &[(&str, &str)]| pairs.iter().map(|(n, v)| (OsString::from(n), OsString::from(v))).collect::<Vec<_>>();
        let invoking = env(&[("HOME", "/home/u"), ("PATH", "/usr/bin"), ("TMPDIR", "/run/t"), ("DISPLAY", ":0"),
                             ("LANG", "C.UTF-8"), ("OLDPWD", "/x"), ("TERM", "xterm"), ("XDG_CONFIG_HOME", "/home/u/.config")]);
        let pinned = pinned_env(&invoking, Path::new("/s/out.json.d"));
        assert_eq!(pinned, env(&[
            ("HOME", "/home/u"), ("LANG", "C.UTF-8"), ("XDG_CONFIG_HOME", "/home/u/.config"),
            ("PWD", "/s/out.json.d/p/cwd"), ("TMPDIR", "/s/out.json.d/p/tmp"), ("TMP", "/s/out.json.d/p/tmp"),
            ("TEMP", "/s/out.json.d/p/tmp"), ("PATH", "/s/out.json.d/p/bin"), ("TERM", "dumb"),
        ]));
        assert_eq!(pinned_dir(Path::new("/s/out.json")), Path::new("/s/out.json.d"));
    }

    #[test]
    fn relative_paths_resolve_against_the_invoking_directory() {
        let args = |list: &[&str]| list.iter().map(OsString::from).collect::<Vec<_>>();
        let invoked = args(&["--plan", "plan.json", "--selector", "--plan", "--preload", "../p.el", "--load-path", "/abs",
                             "--harness", "h.el", "--world-trace", "out.json", "--harness-kind", "real"]);
        assert_eq!(absolute_args(&invoked, Path::new("/w")), args(&[
            "--plan", "/w/plan.json", "--selector", "--plan", "--preload", "/w/../p.el", "--load-path", "/abs",
            "--harness", "/w/h.el", "--world-trace", "/w/out.json", "--harness-kind", "real"]));
    }
}
