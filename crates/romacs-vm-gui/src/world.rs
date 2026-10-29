//! The GUI's world (`romacs_vm::world`): a live `--loadup` launch keeps the
//! VM at the end of the GNU-loadup chain and the bundled packages'
//! autoloads, with the chain's report beside it. The steps before that point
//! are the launch's own and depend only on its recipe (the launch mode, the
//! window geometry, the launch font and the collection floor) and on keyed
//! host facts. A restored launch attaches its hosts, runs
//! `romacs-world-restart` and continues with the interactive session
//! (`startup::initialize_session`) and the workspace.
//!
//! The world ends at the chain, not after the session's own startup steps
//! (rlisp/romacs-gui-loadup.el): Romacs's layers after them read and write
//! the delayed variables and the inputs of those steps, which a world build
//! refuses.
use crate::startup;
use romacs_vm::Vm;
use romacs_vm::world::store::{Launch, World};
use serde_json::json;
use std::path::PathBuf;

/// What a GUI world carries beside its VM: the chain's report.
pub struct Carry {
    pub loadup: crate::loadup::Loadup,
}

/// The source of the launch font's face, and its size in points, from
/// ROMACS_GUI_FONT and ROMACS_GUI_FONT_SIZE.
pub fn launch_font() -> Result<Option<(String, u32)>, String> {
    let Ok(family) = std::env::var("ROMACS_GUI_FONT") else { return Ok(None) };
    if family.chars().any(char::is_control) { return Err("font family contains a control character".into()); }
    let points = std::env::var("ROMACS_GUI_FONT_SIZE").unwrap_or_else(|_| "14".into())
        .parse::<u32>().map_err(|e| e.to_string())?;
    if !(6..=72).contains(&points) { return Err("font size must be 6..72 points".into()); }
    let source = format!("(defface default '((t (:foreground \"#d4c5a0\" :background \"#000000\" :family {} :height {}))) \"Launch font\")",
                         serde_json::to_string(&family).map_err(|e| e.to_string())?, points * 10);
    Ok(Some((source, points)))
}

/// The steps a live `--loadup` launch takes before the chain, which its
/// world holds.
pub const INTERPROGRAM: &str = "(defvar interprogram-cut-function 'romacs-clipboard-select-text) (defvar interprogram-paste-function 'romacs-clipboard-selection-value)";

/// The world of a live `--loadup` launch of WIDTH by HEIGHT with FLOOR, in
/// STORE.
pub fn open(store: PathBuf, width: u32, height: u32, floor: Option<usize>) -> Result<World, String> {
    let font = launch_font()?.map_or("font default".to_owned(), |(source, _)| format!("font {source}"));
    let dir = startup::lisp_root(&startup::lisp_dir())?;
    let recipe = vec![
        "consumer romacs-vm-gui".to_owned(), "launch loadup live".into(), format!("geometry {width} {height}"),
        format!("floor {floor:?}"), font, format!("lisp {}", dir.display()),
    ];
    World::new(store, recipe).map_err(|e| e.to_string())
}

/// This launch with WORLD: the restored VM and its carry, or None to boot
/// from source (a builder runs beside it then, unless WAIT built the world
/// first). A builder process builds and exits here.
pub fn launch(world: &World, width: u32, height: u32, floor: Option<usize>, wait: bool) -> Option<(Box<Vm>, Carry)> {
    match world.launch::<Carry>(|| std::env::args_os().skip(1).collect(), wait) {
        Launch::Restored { vm, carry, manifest } => {
            eprintln!("romacs-world: {}", json!({"mode": "restored", "key": manifest["key"]}));
            Some((vm, carry))
        }
        Launch::Build => {
            let result = build(world, width, height, floor);
            if let Err(error) = &result { eprintln!("romacs-world: build failed: {error}"); }
            std::process::exit(if result.is_ok() { 0 } else { 1 })
        }
        Launch::Chain { reason } => {
            eprintln!("romacs-world: {}", json!({"mode": "source", "reason": reason}));
            None
        }
    }
}

/// The read roots of a GUI build beside the store's own: the Lisp trees and
/// the fonts the layout engine measures with.
fn build_roots(dir: &std::path::Path) -> Vec<PathBuf> {
    let mut roots = vec![dir.to_owned()];
    roots.extend(dir.parent().map(|parent| parent.join("rlisp")));
    roots.extend(startup::bundled_load_path(dir).unwrap_or_default());
    roots.extend(["/usr/share/fonts", "/etc/fonts", "/nix/store"].map(PathBuf::from));
    if let Some(home) = romacs_vm::host::getenv("HOME").map(PathBuf::from) {
        roots.extend([home.join(".local/share/fonts"), home.join(".fonts"), home.join(".cache/fontconfig")]);
    }
    roots
}

/// The builder: the launch's steps up to the world's end, headless, on the
/// evaluator's stack, then publish.
fn build(world: &World, width: u32, height: u32, floor: Option<usize>) -> Result<(), String> {
    let world = world.clone();
    crate::spawn_evaluator(move || -> Result<(), String> {
        // The fonts load before the build is confined.
        let engine = std::rc::Rc::new(std::cell::RefCell::new(romacs_layout2::LayoutEngine::new()?));
        let dir = startup::lisp_root(&startup::lisp_dir())?;
        world.begin_build(&world.build_roots(build_roots(&dir))).map_err(|e| e.to_string())?;
        let mut vm = Vm::new();
        vm.world_begin();
        vm.set_layout_host(Some(Box::new(crate::measure::Host { engine })));
        // A live launch attaches its terminal service here; the world keeps
        // what that declares, and the restored launch attaches the service.
        vm.declare_terminal_errors()?;
        vm.set_echo_area_host(true);
        if let Some(floor) = floor { vm.set_collection_floor(floor); }
        let (line_height, column_width) = romacs_layout2::frame_metrics(&vm.display_default_face()?)?;
        vm.set_frame_geometry(width, height, line_height, column_width)?;
        vm.eval_source(INTERPROGRAM)?;
        let font = launch_font()?;
        let loadup = startup::initialize_chain(&mut vm, startup::Launch::Loadup, font.as_ref().map(|(s, _)| s.as_str()), &dir)?;
        vm.set_layout_host(None);
        let len = world.finish_build(vm, Carry { loadup }, json!({}))?;
        eprintln!("romacs-world: {}", json!({"event": "world-build", "ok": true, "image_bytes": len}));
        Ok(())
    })?.join().map_err(|_| "world builder panicked".to_owned())?
}
