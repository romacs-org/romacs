//! Headless editor host for source-loaded Lisp tests and development probes,
//! and GNU's `emacs --batch'.
//!
//! A GNU batch session boots from its own world (`romacs_vm::world`): the
//! VM at the end of the batch chain ([`chain`]), keyed by this executable
//! and [`recipe`], in `--world-store` or the user's cache
//! (`world::default_store`) when that exists, unless ROMACS_WORLD=off. No
//! session is slower than a boot from source: one whose world is missing
//! or stale boots the chain from source while a detached builder, under
//! the world's lock, prepares it for the next; a session in a fresh HOME
//! makes no store at all. The batch
//! world is not the GUI's: their recipes differ, so neither restores the
//! other's, and `prune` bounds each family alone.
use romacs_vm::Vm;
use romacs_vm::world::store::{Launch, World};
use std::ffi::OsString;
use std::io::Read;
use std::path::{Path, PathBuf};

/// `--batch RLISP_DIRECTORY [--init-file FILE]' runs the forms on stdin
/// over Romacs's own Lisp; any other `--batch' command line is GNU's
/// ([`run_emacs`]). QUICK is a `-Q' before `--batch', STORE a
/// `--world-store' before it.
pub fn run(args: Vec<String>, quick: bool, store: Option<PathBuf>) -> Result<(), String> {
    let own = matches!(args.as_slice(), [dir] | [dir, _, _] if std::path::Path::new(dir).is_dir())
        && (args.len() == 1 || args[1] == "--init-file");
    if !own { return run_emacs(args, quick, store); }
    let init = args.get(2).map(PathBuf::from);
    let root = PathBuf::from(&args[0]).canonicalize().map_err(|error| error.to_string())?;
    let mut source = String::new();
    std::io::stdin().read_to_string(&mut source).map_err(|error| error.to_string())?;
    if source.trim().is_empty() { return Err("batch input is empty".into()); }
    super::spawn_evaluator(move || {
        let mut vm = Vm::new();
        vm.set_frame_geometry(1000, 700, 20, 10)?;
        super::startup::initialize_files_from_sources(&mut vm, &[], &root)?;
        if let Some(path) = init {
            if !super::user_init::load(&mut vm, &path)? {
                return Err(format!("Explicit init file does not exist: {}", path.display()));
            }
        }
        let value = vm.eval_source(&source)?.display()?;
        println!("{value}");
        vm.shutdown_background_processes()
    })?.join().map_err(|_| "batch evaluator panicked".to_owned())?
}

/// GNU's `emacs --batch ARGS': the GNU-loadup chain `--loadup' shares with
/// the batch package runner, then startup.el's `command-line' as a batch
/// session takes it: the leading options it consumes, no init file and no
/// packages (startup.el:1367), then `command-line-1' over the rest (`-L',
/// `-l', `-f', `--eval' and the others), then `(kill-emacs t)'
/// (startup.el:1748-1751). An error that reaches top level goes to
/// `command-error-function' as keyboard.c cmd_error sends it, whose batch
/// default prints it and ends the session with status 255; top_level_1's
/// handler is GNU's C `error', so the debugger may run first. The VM up to
/// the chain's end is the batch world's ([`boot`]).
fn run_emacs(args: Vec<String>, quick: bool, store: Option<PathBuf>) -> Result<(), String> {
    let (quick, args) = command_line_options(args, quick);
    configure_compiled_lisp(store.as_deref(), false);
    super::spawn_evaluator(move || -> Result<(), String> {
        let mut vm = boot(store, false)?;
        vm.set_session_host(Some(Box::new(|code| {
            use std::io::Write;
            let _ = std::io::stdout().flush();
            let _ = std::io::stderr().flush();
            std::process::exit(code)
        })));
        let args: Vec<String> = args.iter().map(|arg| super::startup::lisp_string(arg)).collect();
        let quick = if quick { super::startup::QUICK_START } else { "" };
        vm.eval_source(&format!(
            "(progn (setq command-line-args (cons (expand-file-name invocation-name invocation-directory) (list {}))
                          command-line-default-directory default-directory)
                    {quick}
                    (condition-case err (command-line-1 (cdr command-line-args))
                      ((debug t) (funcall command-error-function err \"\" nil) (kill-emacs -1)))
                    (kill-emacs t))",
            args.join(" ")))?;
        Err("kill-emacs returned".into())
    })?.join().map_err(|_| "batch evaluator panicked".to_owned())?
}

/// The environment variables a compile process takes from the session that
/// starts it. Every other one has the compile environment's own value
/// ([`compile_environment`]), the same in every session, so the compile
/// identity binds it and a compile's reads of it are no inputs.
const COMPILE_INHERITS: [&str; 4] = ["PATH", "LD_LIBRARY_PATH", "ROMACS_RUNTIME_ROOT", "ROMACS_LISP_DIR"];
/// The compiled-Lisp store a compile process publishes in, as the session
/// that started it named it.
const COMPILE_STORE: &str = "ROMACS_COMPILED_LISP_STORE";

/// A compile process's whole environment, for the store STORE: the
/// [`COMPILE_INHERITS`] names, a HOME and temporary directory of its own
/// in the store (HOME is its working directory too) and the C.UTF-8
/// locale. None of the user's HOME, XDG directories, locale or identity
/// reaches a compile, as none reaches GNU's installed `.elc`
/// files, which `make` compiled: what a compile makes of a file is the
/// same under every HOME, and every HOME shares it.
fn compile_environment(store: &Path) -> Vec<(OsString, OsString)> {
    COMPILE_INHERITS.iter()
        .filter_map(|name| Some((OsString::from(name), std::env::var_os(name)?)))
        .chain([("HOME".into(), store.join("home").into()), ("TMPDIR".into(), store.join("tmp").into()),
                ("LANG".into(), "C.UTF-8".into()), (COMPILE_STORE.into(), store.into())])
        .collect()
}

/// The compiled-Lisp store, and whether this process may compile into it:
/// the one [`COMPILE_STORE`] names (a compile process's); else `lisp/`
/// beside a named world store WORLDS, when that exists; else the
/// installation's, `EXECUTABLE.lisp/` beside the executable, which every
/// HOME shares as it shares GNU's installed `.elc` files, or the user's
/// cache when the installation's cannot be made. Only a user with a world
/// store compiles, or an INTERACTIVE session, which makes one as it starts:
/// a batch session in a fresh HOME reads what is compiled and loads the
/// rest from source, as GNU does without `.elc` files.
fn compiled_store(worlds: Option<&Path>, interactive: bool) -> Option<(PathBuf, bool)> {
    if let Some(store) = std::env::var_os(COMPILE_STORE) { return Some((store.into(), true)); }
    let made = |store: &Path| interactive || store.is_dir();
    if let Some(worlds) = worlds { return made(worlds).then(|| (worlds.with_file_name("lisp"), true)); }
    let user = super::world::default_store(|name| std::env::var_os(name)).filter(|store| made(store));
    let exe = std::env::current_exe().ok()?;
    let installed = exe.with_file_name(format!("{}.lisp", exe.file_name()?.to_string_lossy()));
    let private = |dir: &Path| { use std::os::unix::fs::DirBuilderExt; std::fs::DirBuilder::new().mode(0o700).create(dir) };
    if installed.is_dir() || (user.is_some() && private(&installed).is_ok()) {
        return Some((installed, user.is_some()));
    }
    user.map(|worlds| (worlds.with_file_name("lisp"), true))
}

/// Load compiled Lisp in this process (`romacs_vm::compiled_lisp`) when
/// it is asked for (by default in an INTERACTIVE session) and worlds are
/// on, from the store [`compiled_store`] finds (WORLDS is a
/// `--world-store` given). A compile process restores the compile world,
/// the batch world of the compile environment in the store's `worlds/`,
/// and that world's identity keys every artifact. An interactive session
/// never waits for a compile: a load with no artifact reads source and the
/// session compiles the file in the background (rlisp/romacs-precompile.el).
pub fn configure_compiled_lisp(worlds: Option<&Path>, interactive: bool) {
    if !romacs_vm::compiled_lisp::requested(interactive) || romacs_vm::world::store::disabled() { return; }
    let Some((store, compiles)) = compiled_store(worlds, interactive) else { return };
    configure_store(store, compiles, interactive);
}

/// Load compiled Lisp in a worker isolate from STORE, the store its
/// spawner, an interactive session, loads from (`isolate-compiled-store`),
/// without ever compiling into it. A miss reads source; the spawner's
/// background compiles reach every file of the installation's trees in
/// time.
pub fn configure_worker_compiled_lisp(store: PathBuf) {
    if !romacs_vm::compiled_lisp::requested(true) || romacs_vm::world::store::disabled() { return; }
    configure_store(store, false, true);
}

/// Load compiled Lisp from STORE; COMPILES when this process may compile
/// into it; DEFER_MISSES when no load may wait for a compile.
fn configure_store(store: PathBuf, compiles: bool, defer_misses: bool) {
    let Ok(dir) = super::startup::lisp_root(&super::startup::lisp_dir()) else { return };
    let Ok(exe) = std::env::current_exe() else { return };
    let mut runtime = vec![dir.clone()];
    if let Some(parent) = dir.parent() { runtime.push(parent.join("rlisp")); }
    runtime.extend(super::startup::bundled_load_path(&dir).unwrap_or_default());
    let environment = compile_environment(&store);
    let recipe = recipe_in(&dir, |name| environment.iter().find(|(n, _)| n == name).map(|(_, v)| v.clone()));
    let worlds = store.join("worlds");
    let made = |name| { use std::os::unix::fs::DirBuilderExt; std::fs::DirBuilder::new().recursive(true).mode(0o700).create(store.join(name)).is_ok() };
    let compiler = if compiles && made("home") && made("tmp") {
        vec![exe.into(), "--world-store".into(), worlds.clone().into(), "--compile-lisp".into()]
    } else {
        vec![]
    };
    romacs_vm::compiled_lisp::configure(romacs_vm::compiled_lisp::Config {
        compiler,
        environment,
        inherited: &COMPILE_INHERITS,
        defer_misses,
        store,
        // The world is made once (its key hashes the executable and
        // libraries); only whether its build was published is asked again.
        identity: Box::new({
            let world = std::sync::OnceLock::new();
            move || world.get_or_init(|| World::new(worlds.clone(), recipe.clone()).ok()).as_ref()?
                .lisp_identity().map(|id| format!("compile world {id}"))
        }),
        runtime,
    });
}

/// `--compile-lisp FILE`: a compile process. The batch session's VM, with
/// the session's `load-path` (`compiled_lisp::LOAD_PATH`), compiles FILE
/// and publishes its artifact, whose path the last line of standard
/// output names.
pub fn compile_lisp(file: PathBuf, store: Option<PathBuf>) -> Result<(), String> {
    let file = std::path::absolute(&file).map_err(|e| e.to_string())?;
    if let Some(home) = std::env::var_os("HOME") { std::env::set_current_dir(home).map_err(|e| e.to_string())?; }
    configure_compiled_lisp(store.as_deref(), false);
    super::spawn_evaluator(move || -> Result<(), String> {
        let mut vm = boot(store, true)?;
        // A tree's compile (none given) keeps the chain's own.
        if let Ok(path) = std::env::var(romacs_vm::compiled_lisp::LOAD_PATH)
            && !path.is_empty()
        {
            let dirs: Vec<PathBuf> = path.lines().map(PathBuf::from).collect();
            vm.set_load_path(&dirs)?;
        }
        let (artifact, held) = vm.compile_lisp_file(&file)?;
        if held { println!("romacs-compile-held"); }
        println!("romacs-compiled: {}", artifact.display());
        vm.shutdown_background_processes()
    })?.join().map_err(|_| "compile process panicked".to_owned())?
}

/// `--compile-lisp-tree [--jobs N]`: compile the installation's Lisp
/// trees into its store ahead of use (`compiled_lisp::compile_tree`), N
/// compile processes at a time, as an interactive session of this
/// installation would compile them, and print the summary as JSON. Fails
/// when compiled Lisp is off, the store is unusable, a file's compile
/// process failed or its artifact did not survive the store's maintenance,
/// or the tree's artifacts exceed the store's budget. A run whose only
/// faults are files that failed to compile (their artifacts say so, and
/// they load as source) exits with status 2, for the installer to judge.
pub fn compile_lisp_tree(jobs: usize) -> Result<(), String> {
    configure_compiled_lisp(None, true);
    let summary = romacs_vm::compiled_lisp::compile_tree(jobs)?;
    println!("{summary}");
    let empty = |field: &str| summary[field].as_array().is_none_or(Vec::is_empty);
    if !empty("failed") || !empty("lost") {
        return Err("some files of the tree have no usable artifact (see the summary)".into());
    }
    if summary["over_budget"] == true {
        return Err("the tree's artifacts exceed the compiled-Lisp store's budget (see the summary)".into());
    }
    if !empty("errors") {
        eprintln!("romacs-compile: some files of the tree failed to compile and load as source (see the summary)");
        std::process::exit(2);
    }
    Ok(())
}

/// The batch session's VM up to the chain's end: its world restored, then
/// this process's startup steps (`romacs-world-restart'); else the chain
/// from source, as a session without worlds boots, while a builder
/// prepares the world for the next session ([`world`]), or, when WAIT
/// (a compile process, whose world keys what it compiles), once it has.
/// A builder prunes the store, builds the world and exits here.
fn boot(store: Option<PathBuf>, wait: bool) -> Result<Vm, String> {
    let dir = super::startup::lisp_root(&super::startup::lisp_dir())?;
    let Some(world) = world(store, &dir) else { return chain(&dir) };
    match world.launch::<()>(|| builder_args(&world), wait, || {}) {
        Launch::Restored { vm, .. } => {
            let mut vm = *vm;
            vm.world_reinit();
            vm.eval_source("(romacs-world-restart)")?;
            Ok(vm)
        }
        Launch::Build => {
            // Room for this world among its family's (its lock file there
            // makes it the current one), before the build confines this
            // process to reading the store; no other family's world goes.
            match world.prune_family() {
                Ok(deleted) if deleted.is_empty() => {}
                Ok(deleted) => eprintln!("romacs-world: {}", serde_json::json!({"event": "pruned", "worlds": deleted})),
                Err(error) => eprintln!("romacs-world: prune failed: {error}"),
            }
            let result = build(&world, &dir);
            match &result {
                Ok(len) => eprintln!("romacs-world: {}", serde_json::json!({"event": "world-build", "ok": true, "image_bytes": len})),
                Err(error) => eprintln!("romacs-world: build failed: {error}"),
            }
            std::process::exit(if result.is_ok() { 0 } else { 1 })
        }
        Launch::Chain { reason } => {
            // Quiet as GNU's batch, unless a harness asks why the world
            // was not used.
            if std::env::var_os("ROMACS_DIAGNOSTICS").is_some_and(|v| v == "1") {
                eprintln!("romacs-world: {}", serde_json::json!({"mode": "source", "reason": reason}));
            }
            chain(&dir)
        }
    }
}

/// The batch world over the GNU Lisp in DIR, in STORE (`--world-store`)
/// or the default store when that exists already: a session never waits
/// for a build, and one in a fresh HOME makes no store. None when worlds
/// are off, or the store cannot be used, which a named store says.
fn world(store: Option<PathBuf>, dir: &Path) -> Option<World> {
    if romacs_vm::world::store::disabled() { return None; }
    let named = store.is_some();
    let store = store.or_else(|| super::world::default_store(|name| std::env::var_os(name)).filter(|store| store.is_dir()))?;
    match World::new(store, recipe(dir)) {
        Ok(world) => Some(world),
        // The default store's session stays as quiet as GNU's.
        Err(reason) => {
            if named { eprintln!("romacs-world: {}", serde_json::json!({"mode": "source", "reason": reason.to_string()})); }
            None
        }
    }
}

/// The environment the batch chain's startup steps read (the user, the
/// locale, the per-user directories), which a world's records bind: keyed
/// too, so the worlds of each HOME or locale a user runs under sit side by
/// side, `prune` keeping the most recent few, rather than rebuilding one.
/// The processor count and the OpenMP limits it reads are per-process,
/// asked again by the restored startup, so they key nothing.
const STARTUP_ENVIRONMENT: [&str; 13] = ["HOME", "XDG_CONFIG_HOME", "LANG", "LC_ALL", "LC_CTYPE", "LC_MESSAGES",
    "USER", "LOGNAME", "NAME", "EMAIL", "SHELL", "ROMACS_RUNTIME_ROOT", "HANGUL_KEYBOARD_TYPE"];

/// What keys a batch world beside the executable: the batch chain over
/// the GNU Lisp in DIR, on GNU batch's initial terminal frame, from this
/// executable's path (the family `prune` bounds) in [`STARTUP_ENVIRONMENT`].
fn recipe(dir: &Path) -> Vec<String> {
    recipe_in(dir, |name| std::env::var_os(name))
}

/// [`recipe`] in the environment GET reads.
fn recipe_in(dir: &Path, get: impl Fn(&str) -> Option<OsString>) -> Vec<String> {
    let exe = std::env::current_exe().map_or_else(|e| e.to_string(), |p| p.display().to_string());
    let environment: Vec<serde_json::Value> = STARTUP_ENVIRONMENT.iter()
        .map(|name| get(name).map_or(serde_json::Value::Null, |v| v.to_string_lossy().into()))
        .collect();
    vec!["consumer romacs-vm-gui".into(), "launch batch".into(), format!("exe {exe}"), "geometry 80 25 1 1".into(),
         format!("lisp {}", dir.display()), format!("environment {}", serde_json::Value::Array(environment))]
}

/// A builder's arguments for WORLD: its store, absolute, then `--batch'.
fn builder_args(world: &World) -> Vec<std::ffi::OsString> {
    vec!["--world-store".into(), world.store.clone().into(), "--batch".into()]
}

/// The batch chain from source: a VM on GNU batch's initial terminal
/// frame, 80 columns by 25 lines of 1x1 characters (DIVERGENCES.md
/// VM2-WIN5), through the GNU-loadup chain over DIR. What a batch world
/// holds, and what a session without one boots.
fn chain(dir: &Path) -> Result<Vm, String> {
    let mut vm = Vm::new();
    vm.world_begin();
    vm.set_frame_geometry(80, 25, 1, 1)?;
    super::startup::drive_loadup(&mut vm, dir)?;
    Ok(vm)
}

/// The builder: the chain in build mode, confined to its read roots, then
/// the world published. The image's length.
fn build(world: &World, dir: &Path) -> Result<usize, String> {
    world.begin_build(&world.build_roots(super::world::build_roots(dir))).map_err(|e| e.to_string())?;
    world.build(|| Ok((chain(dir)?, (), serde_json::json!({}))))
}

/// The leading options startup.el `command-line' consumes before
/// `command-line-1' (startup.el:1375-1440), as far as a batch session
/// heeds them: whether one was -Q, and the rest.
fn command_line_options(args: Vec<String>, mut quick: bool) -> (bool, Vec<String>) {
    let mut rest = args.into_iter().peekable();
    while let Some(arg) = rest.peek() {
        match arg.as_str() {
            "-Q" | "-quick" | "--quick" => quick = true,
            "-q" | "-no-init-file" | "--no-init-file" | "-no-site-file" | "--no-site-file"
            | "-no-x-resources" | "--no-x-resources" => {}
            _ => break,
        }
        rest.next();
    }
    (quick, rest.collect())
}
