//! A unit test's loadup from a world, as the program's launch has one
//! (`world.rs`): the chain and Romacs's definitions, which depend only on
//! the test's `Boot`. The first test process to ask for a boot builds its
//! world, as a builder of this executable that runs the same test again,
//! and every later one maps it and takes the startup steps in its own
//! process (`World::for_test`). A test then takes the session's steps
//! itself (`startup::resume_loadup`), after anything of its own process
//! (a private HOME), as a restored launch does. A world is keyed by this
//! executable and records every file its boot read, so a rebuilt
//! executable or an edited Lisp file builds it again. Anywhere else
//! (libtest's threads, a process that already maps an image, a refused
//! restore, `ROMACS_WORLD=off`) the boot runs from source, and answers the
//! same.
//!
//! Shared by the library's unit tests (lib.rs) and the program's (main.rs),
//! each executable with worlds of its own.
use crate::loadup::Loadup;
use crate::startup::{self, Launch};
use romacs_vm::Vm;
use romacs_vm::world::store::World;

/// What a test's boot depends on: the launch, the frame of WIDTH by HEIGHT
/// with cells of LINE by COLUMN, the collection floor and whether the echo
/// area has a host. A world a boot.
#[derive(Clone, Copy, Debug)]
pub struct Boot {
    pub launch: Launch,
    pub frame: (u32, u32, u32, u32),
    pub floor: Option<usize>,
    pub echo_area: bool,
}

impl Boot {
    /// `--loadup` on the tests' 1200x800 frame of 20x10 cells.
    pub const LOADUP: Boot = Boot { launch: Launch::Loadup, frame: (1200, 800, 20, 10), floor: None, echo_area: false };
    /// `--gnu-lisp` on the same frame.
    pub const GNU_ONLY: Boot = Boot { launch: Launch::GnuOnly, ..Boot::LOADUP };

    /// The world's recipe, under GNU's Lisp in DIR.
    fn recipe(self, dir: &std::path::Path) -> Vec<String> {
        let (width, height, line, column) = self.frame;
        vec!["romacs-vm-gui unit tests".into(), format!("launch {:?}", self.launch),
             format!("geometry {width} {height} {line} {column}"), format!("floor {:?}", self.floor),
             format!("echo-area {}", self.echo_area), format!("lisp {}", dir.display())]
    }
}

/// BOOT's VM after the chain and Romacs's definitions, with the chain's
/// report: from its world when this process can map one, else from source.
///
/// # Panics
///
/// When the boot fails.
pub fn chain(boot: Boot) -> (Vm, Loadup) {
    let dir = startup::lisp_root(&startup::lisp_dir()).unwrap();
    // In this executable's profile directory (it is in PROFILE/deps).
    let exe = std::env::current_exe().unwrap();
    let store = exe.parent().and_then(std::path::Path::parent).unwrap().join("romacs-vm-gui-test-worlds");
    World::for_test(store, &boot.recipe(&dir), startup::lisp_roots(&dir), || booted(boot, &dir, true))
        .unwrap_or_else(|| booted(boot, &dir, false).unwrap())
}

/// BOOT's chain from source, recording for a world when WORLD.
fn booted(boot: Boot, dir: &std::path::Path, world: bool) -> Result<(Vm, Loadup), String> {
    let mut vm = Vm::new();
    if world {
        vm.world_begin();
    }
    let (width, height, line, column) = boot.frame;
    vm.set_frame_geometry(width, height, line, column)?;
    if let Some(floor) = boot.floor {
        vm.set_collection_floor(floor);
    }
    if boot.echo_area {
        vm.set_echo_area_host(true);
    }
    let loadup = startup::initialize_chain(&mut vm, boot.launch, None, dir)?;
    startup::install_session_definitions(&mut vm, boot.launch, dir)?;
    Ok((vm, loadup))
}

/// BOOT's session after its chain (`chain`, then `startup::resume_loadup`),
/// for a test that takes no step of its own between the two.
///
/// # Panics
///
/// When the boot or the session's start fails.
pub fn started(boot: Boot) -> (Vm, startup::Started) {
    let (mut vm, loadup) = chain(boot);
    let started = startup::resume_loadup(&mut vm, boot.launch, loadup).unwrap();
    (vm, started)
}
