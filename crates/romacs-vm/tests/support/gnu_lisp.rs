//! A VM with GNU's Lisp, booted as GNU batch: its terminal frame, its
//! installed load path, the prelude and the loadup bootstrap.
//!
//! In a test executable whose allocator is the world arena (the `gnu`
//! target), the boot is a world, as GNU batch's is (`src/batch.rs`): the
//! first test process to ask builds it up to GNU's dump point, as a builder
//! of this executable (`World::launch` runs the same test again to build),
//! and every later one maps it and takes the startup steps in its own
//! process (`romacs-world-restart`): its environment, temporary directory
//! and user are its own, not the builder's. A world
//! is keyed by this executable and records every file the boot read, so a
//! rebuilt executable or an edited Lisp file builds it again. Anywhere else
//! (another target, a process that already holds an image, a refused
//! restore, `ROMACS_WORLD=off`) the boot runs from source, as it answers the
//! same.
use romacs_vm::Vm;
use romacs_vm::world::arena;
use romacs_vm::world::store::{Launch, World};
use std::path::{Path, PathBuf};

/// GNU batch's VM after the loadup bootstrap, with each file of THEN (a
/// path under the checkout) loaded after it.
pub fn gnu_lisp(then: &[&str]) -> Vm {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let mut vm = restored(&root).unwrap_or_else(|| booted(&root, false));
    for name in then {
        load(&mut vm, &root, name);
    }
    vm
}

/// The boot from its world, when this process can map one: its global
/// allocator is the arena, it runs one test (nextest's process per test, as
/// every check runs them), and it maps no image yet. A process maps one
/// image, and its mapping is not to be raced: under libtest's threads,
/// which run many tests in one process, every boot is from source, as is a
/// test's second boot.
fn restored(root: &Path) -> Option<Vm> {
    let alone = std::env::var_os("NEXTEST_EXECUTION_MODE").is_some_and(|mode| mode == "process-per-test");
    if !alone || !arena_allocates() || arena::mapped_image().is_some() {
        return None;
    }
    let store = Path::new(env!("CARGO_TARGET_TMPDIR")).join("gnu-lisp-worlds");
    let world = World::new(store, vec!["consumer romacs-vm tests gnu_lisp".into(), format!("lisp {}", root.display())]).ok()?;
    match world.launch::<()>(|| std::env::args_os().skip(1).collect(), true, || {}) {
        Launch::Restored { mut vm, .. } => {
            vm.world_reinit();
            vm.eval_source("(romacs-world-restart)").unwrap();
            Some(*vm)
        }
        Launch::Build => {
            let built = world.begin_build().map_err(|e| e.to_string())
                .and_then(|()| {
                    romacs_vm::host::end_at_dump_point();
                    world.build(|| Ok((booted(root, true), (), serde_json::Value::Null)))
                });
            if let Err(error) = &built {
                eprintln!("GNU Lisp world: {error}");
            }
            std::process::exit(i32::from(built.is_err()));
        }
        Launch::Chain { .. } => None,
    }
}

/// Whether this process's global allocator is the arena (the `gnu`
/// target's): a new block lies in its range. The arena exists in any
/// process that asks for it, whatever allocator serves the heap.
fn arena_allocates() -> bool {
    let block = Box::new(0_u8);
    let at = std::ptr::from_ref(&*block) as usize;
    arena::current_range().is_some_and(|(base, len)| (base..base + len).contains(&at))
}

/// The boot from source, recording for a world when WORLD.
fn booted(root: &Path, world: bool) -> Vm {
    let mut vm = Vm::new();
    if world {
        vm.world_begin();
    }
    // GNU batch, as the gnu-ert runner sets it before the chain loads:
    // noninteractive, on its 80x25 terminal frame of 1x1 characters.
    vm.eval_source("(setq noninteractive t)").unwrap();
    vm.set_frame_geometry(80, 25, 1, 1).unwrap();
    vm.set_load_path(&load_path(root)).unwrap();
    vm.install_lisp_prelude().unwrap();
    load(&mut vm, root, "lisp/romacs/romacs-loadup-bootstrap.el");
    vm
}

/// Load NAME, a path under ROOT, or panic with the load's failed steps.
fn load(vm: &mut Vm, root: &Path, name: &str) {
    if let Err(error) = vm.load_file(&root.join(name)) {
        let report = vm.load_report();
        let failed = report["steps"].as_array().unwrap().iter()
            .filter(|step| step["status"] != "ok").collect::<Vec<_>>();
        panic!("{name}: {error}\n{failed:?}");
    }
}

/// GNU's installed load-path (`test/gnu/run.py` `load_path`): the Lisp
/// root, its subdirectories in reverse name order with obsolete last, then
/// `lisp/romacs`.
fn load_path(root: &Path) -> Vec<PathBuf> {
    let lisp = root.join("lisp/gnu");
    let mut names: Vec<String> = std::fs::read_dir(&lisp).unwrap()
        .filter_map(|entry| entry.ok().filter(|e| e.path().is_dir()).and_then(|e| e.file_name().into_string().ok()))
        .filter(|name| !["term", "RCS", "CVS", "Old"].contains(&name.as_str()) && !name.starts_with('=')
            && !name.ends_with('~') && !name.ends_with(".orig") && !name.ends_with(".rej"))
        .collect();
    names.sort();
    let (obsolete, rest): (Vec<&String>, Vec<&String>) = names.iter().rev().partition(|n| *n == "obsolete");
    std::iter::once(lisp.clone())
        .chain(rest.into_iter().chain(obsolete).map(|n| lisp.join(n)))
        .chain(std::iter::once(root.join("lisp/romacs")))
        .collect()
}
