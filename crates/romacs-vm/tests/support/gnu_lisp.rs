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
use romacs_vm::world::store::World;
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

/// The boot from its world, when this process can map one
/// (`World::for_test`): in the `gnu` target, one test a process.
fn restored(root: &Path) -> Option<Vm> {
    let store = Path::new(env!("CARGO_TARGET_TMPDIR")).join("gnu-lisp-worlds");
    let recipe = vec!["romacs-vm tests gnu_lisp".into(), format!("lisp {}", root.display())];
    World::for_test(store, &recipe, vec![root.to_owned()], || {
        romacs_vm::host::end_at_dump_point();
        Ok((booted(root, true), ()))
    }).map(|(vm, ())| vm)
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
