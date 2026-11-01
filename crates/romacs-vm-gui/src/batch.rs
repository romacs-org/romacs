//! Headless editor host for source-loaded Lisp tests and development probes,
//! and GNU's `emacs --batch'.
//!
//! A GNU batch session boots from its own world (`romacs_vm::world`): the
//! VM at the end of the batch chain ([`chain`]), keyed by this executable
//! and [`recipe`], in `--world-store` or the user's cache
//! (`world::default_store`), unless ROMACS_WORLD=off. A missing or stale
//! world is built first, or another launch's build waited for, as a GUI
//! launch does; a world that cannot be had (a refusal on these inputs, a
//! failed build) boots the chain from source. The batch world is not the
//! GUI's: their recipes differ, so neither restores the other's.
use romacs_vm::Vm;
use romacs_vm::world::store::{Launch, World};
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
    super::spawn_evaluator(move || -> Result<(), String> {
        let mut vm = boot(store)?;
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

/// The batch session's VM up to the chain's end: its world restored, after
/// building it when missing or stale (or waiting for the build another
/// launch runs, then pruning the store), then this process's startup
/// steps (`romacs-world-restart'); or the chain from source when worlds
/// are off, there is no store, or the world cannot be had. A builder
/// builds the world and exits here.
fn boot(store: Option<PathBuf>) -> Result<Vm, String> {
    let dir = super::startup::lisp_root(&super::startup::lisp_dir())?;
    let named = store.is_some();
    let world = store.or_else(|| super::world::default_store(|name| std::env::var_os(name)))
        .filter(|_| !romacs_vm::world::store::disabled())
        .and_then(|store| match World::new(store, recipe(&dir)) {
            Ok(world) => Some(world),
            // A --world-store that cannot be used says why; the default
            // store's session stays as quiet as GNU's.
            Err(reason) => {
                if named { eprintln!("romacs-world: {}", serde_json::json!({"mode": "source", "reason": reason.to_string()})); }
                None
            }
        });
    let Some(world) = world else { return chain(&dir) };
    let waited = std::cell::Cell::new(false);
    match world.launch::<()>(|| builder_args(&world), true, || waited.set(true)) {
        Launch::Restored { vm, .. } => {
            // A build just grew the store: bound it, as a GUI launch does.
            if waited.get() { let _ = world.prune(); }
            let mut vm = *vm;
            vm.world_reinit();
            vm.eval_source("(romacs-world-restart)")?;
            Ok(vm)
        }
        Launch::Build => {
            let result = build(&world, &dir);
            match &result {
                Ok(len) => eprintln!("romacs-world: {}", serde_json::json!({"event": "world-build", "ok": true, "image_bytes": len})),
                Err(error) => eprintln!("romacs-world: build failed: {error}"),
            }
            std::process::exit(if result.is_ok() { 0 } else { 1 })
        }
        Launch::Chain { .. } => chain(&dir),
    }
}

/// What keys a batch world beside the executable: the batch chain over
/// the GNU Lisp in DIR, on GNU batch's initial terminal frame.
fn recipe(dir: &Path) -> Vec<String> {
    vec!["consumer romacs-vm-gui".into(), "launch batch".into(), "geometry 80 25 1 1".into(), format!("lisp {}", dir.display())]
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
