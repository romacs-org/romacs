//! The GUI's world (`romacs_vm::world`): a live `--loadup` launch keeps the
//! VM at the end of the GNU-loadup chain and the bundled packages'
//! autoloads, with the chain's report beside it. The steps before that point
//! are the launch's own and depend only on its recipe (the launch mode, the
//! window geometry, the launch font and the collection floor) and on keyed
//! host facts. A restored launch attaches its hosts, runs
//! `romacs-world-restart` and continues with the interactive session
//! (`startup::initialize_session`) and the workspace.
//!
//! A live `--loadup` launch uses a world unless ROMACS_WORLD=off: in
//! `--world-store`, or else in `$XDG_CACHE_HOME/romacs/worlds`
//! (`default_store`). A launch that finds its world missing or stale builds
//! it first, which costs about what a boot from source does, and boots from
//! it; later launches restore it. A build refused or failed on the same
//! inputs is not tried again until one of them changes.
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
romacs_snapshot::snapshot_struct!(Carry { loadup });

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
/// STORE, resolved against this process's directory (`World::new`).
pub fn open(store: PathBuf, width: u32, height: u32, floor: Option<usize>) -> Result<World, String> {
    let font = launch_font()?.map_or("font default".to_owned(), |(source, _)| format!("font {source}"));
    let dir = startup::lisp_root(&startup::lisp_dir())?;
    let recipe = vec![
        "consumer romacs-vm-gui".to_owned(), "launch loadup live".into(), format!("geometry {width} {height}"),
        format!("floor {floor:?}"), font, format!("lisp {}", dir.display()),
        font_environment(|name| std::env::var_os(name)),
    ];
    World::new(store, recipe).map_err(|e| e.to_string())
}

/// What chooses Fontconfig's configuration and the launch font, from GET:
/// its own variables and the directories it finds its per-user files in.
/// No host fact records them, since Fontconfig reads them itself.
const FONT_ENVIRONMENT: [&str; 8] = ["FONTCONFIG_FILE", "FONTCONFIG_PATH", "FONTCONFIG_SYSROOT", "HOME",
                                     "XDG_CONFIG_HOME", "XDG_DATA_HOME", "XDG_CACHE_HOME", "ROMACS_GUI_FONT_SIZE"];

fn font_environment(get: impl Fn(&str) -> Option<std::ffi::OsString>) -> String {
    let values: Vec<serde_json::Value> = FONT_ENVIRONMENT.iter()
        .map(|name| get(name).map_or(serde_json::Value::Null, |v| json!(v.to_string_lossy())))
        .collect();
    format!("font-environment {}", serde_json::Value::Array(values))
}

/// The per-user configuration Fontconfig would read if it were there, from
/// GET: a build records whether each is, since one appearing changes the
/// configuration without touching any file the build read.
fn user_font_configuration(get: impl Fn(&str) -> Option<std::ffi::OsString>) -> Vec<PathBuf> {
    let home = get("HOME").map(PathBuf::from);
    let config = get("XDG_CONFIG_HOME").map(PathBuf::from).or_else(|| home.as_ref().map(|h| h.join(".config")));
    let mut paths: Vec<PathBuf> = config.into_iter()
        .flat_map(|c| ["fontconfig", "fontconfig/fonts.conf", "fontconfig/conf.d"].map(|p| c.join(p)))
        .collect();
    paths.extend(home.into_iter().flat_map(|h| [".fonts.conf", ".fonts.conf.d"].map(|p| h.join(p))));
    paths.extend(get("FONTCONFIG_FILE").map(PathBuf::from));
    paths.extend(get("FONTCONFIG_PATH").into_iter().flat_map(|p| std::env::split_paths(&p).collect::<Vec<_>>()));
    paths
}

/// Where a launch keeps its worlds when no --world-store names a place:
/// `$XDG_CACHE_HOME/romacs/worlds`, or `~/.cache/romacs/worlds` (the XDG
/// base directory default), from GET. None without an absolute directory
/// to put it in.
pub fn default_store(get: impl Fn(&str) -> Option<std::ffi::OsString>) -> Option<PathBuf> {
    let cache = get("XDG_CACHE_HOME").map(PathBuf::from).filter(|p| p.is_absolute())
        .or_else(|| get("HOME").map(PathBuf::from).filter(|p| p.is_absolute()).map(|h| h.join(".cache")))?;
    Some(cache.join("romacs/worlds"))
}

/// This launch with WORLD: the restored VM and its carry, or None to boot
/// from source. A missing or stale world is built first, or the build
/// another launch runs waited for, which costs about what a boot from
/// source does, and the launch then boots from it; ON_WAIT runs before it
/// blocks for that. One whose build was refused on these very inputs, or
/// failed lately (`romacs_vm::world::store`), boots from source. A builder
/// process builds and exits here.
pub fn launch(world: &World, width: u32, height: u32, floor: Option<usize>, on_wait: impl FnOnce()) -> Option<(Box<Vm>, Carry)> {
    match world.launch::<Carry>(|| builder_args(world, width, height, floor), true, on_wait) {
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

/// A builder's arguments for WORLD's launch: what its recipe takes from the
/// command line, the store absolute, since the builder runs in the store's
/// pinned directory. Nothing else of this launch reaches it.
fn builder_args(world: &World, width: u32, height: u32, floor: Option<usize>) -> Vec<std::ffi::OsString> {
    let mut args: Vec<std::ffi::OsString> = vec!["--loadup".into(), "--world-store".into(), world.store.clone().into(),
                                                 "--geometry".into(), format!("{width}x{height}").into()];
    if let Some(floor) = floor {
        args.extend(["--gc-floor".into(), floor.to_string().into()]);
    }
    args
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
        let len = world.build(|| build_world(width, height, floor, &dir, engine))?;
        eprintln!("romacs-world: {}", json!({"event": "world-build", "ok": true, "image_bytes": len}));
        Ok(())
    })?.join().map_err(|_| "world builder panicked".to_owned())?
}

/// The build's steps once it records: up to the chain's end. The world's
/// VM, its carry and no events, for `World::build` to publish.
fn build_world(width: u32, height: u32, floor: Option<usize>, dir: &std::path::Path,
               engine: std::rc::Rc<std::cell::RefCell<romacs_layout2::LayoutEngine>>) -> Result<(Vm, Carry, serde_json::Value), String> {
    let mut vm = Vm::new();
    vm.world_begin();
    vm.set_layout_host(Some(Box::new(crate::measure::Host { engine: engine.clone() })));
    // A live launch attaches its terminal service here; the world keeps
    // what that declares, and the restored launch attaches the service.
    vm.declare_terminal_errors()?;
    vm.set_echo_area_host(true);
    if let Some(floor) = floor { vm.set_collection_floor(floor); }
    let (line_height, column_width) = romacs_layout2::frame_metrics(&vm.display_default_face()?)?;
    vm.set_frame_geometry(width, height, line_height, column_width)?;
    vm.eval_source(INTERPROGRAM)?;
    let font = launch_font()?;
    let loadup = startup::initialize_chain(&mut vm, startup::Launch::Loadup, font.as_ref().map(|(s, _)| s.as_str()), dir)?;
    vm.set_layout_host(None);
    // The fonts the world measured with, the default face's among them:
    // their files and Fontconfig's are keyed records, so a font added,
    // removed or changed, or another configuration, rebuilds the world.
    engine.borrow_mut().frame_metrics(&vm.display_default_face()?)?;
    for input in engine.borrow().font_inputs() {
        let _ = romacs_vm::host::metadata(&input);
    }
    for path in user_font_configuration(|name| std::env::var_os(name)) {
        let _ = romacs_vm::host::metadata(&path);
    }
    Ok((vm, Carry { loadup }, json!({})))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_launch_keeps_its_worlds_in_the_users_cache_by_default() {
        let env = |pairs: &'static [(&'static str, &'static str)]| move |name: &str| {
            pairs.iter().find(|(n, _)| *n == name).map(|(_, v)| std::ffi::OsString::from(v))
        };
        assert_eq!(default_store(env(&[("XDG_CACHE_HOME", "/c"), ("HOME", "/h")])), Some(PathBuf::from("/c/romacs/worlds")));
        assert_eq!(default_store(env(&[("HOME", "/h")])), Some(PathBuf::from("/h/.cache/romacs/worlds")));
        // A relative XDG_CACHE_HOME is invalid and ignored (XDG base directories).
        assert_eq!(default_store(env(&[("XDG_CACHE_HOME", "c"), ("HOME", "/h")])), Some(PathBuf::from("/h/.cache/romacs/worlds")));
        assert_eq!(default_store(env(&[])), None);
    }

    #[test]
    fn the_font_environment_keys_a_world_and_names_the_per_user_configuration() {
        let env = |pairs: &'static [(&'static str, &'static str)]| move |name: &str| {
            pairs.iter().find(|(n, _)| *n == name).map(|(_, v)| std::ffi::OsString::from(v))
        };
        let base: &[(&str, &str)] = &[("HOME", "/h"), ("XDG_CONFIG_HOME", "/h/.config")];
        let other: &[(&str, &str)] = &[("HOME", "/h"), ("XDG_CONFIG_HOME", "/h/.config"), ("FONTCONFIG_FILE", "/etc/other.conf")];
        let path: &[(&str, &str)] = &[("HOME", "/h"), ("XDG_CONFIG_HOME", "/h/.config"), ("FONTCONFIG_PATH", "/a:/b")];
        let keys: Vec<String> = [base, other, path].iter().map(|pairs| font_environment(env(pairs))).collect();
        assert!(keys[0] != keys[1] && keys[0] != keys[2] && keys[1] != keys[2], "{keys:?}");
        let scratch = std::env::temp_dir().join(format!("romacs-gui-font-env-{}", std::process::id()));
        let world = |line: &String| World::new(scratch.join("store"), vec!["consumer test".into(), line.clone()]).unwrap().key;
        assert_ne!(world(&keys[0]), world(&keys[1]));
        assert_eq!(user_font_configuration(env(path)), ["/h/.config/fontconfig", "/h/.config/fontconfig/fonts.conf",
            "/h/.config/fontconfig/conf.d", "/h/.fonts.conf", "/h/.fonts.conf.d", "/a", "/b"].map(PathBuf::from));
        assert!(user_font_configuration(env(other)).contains(&PathBuf::from("/etc/other.conf")));
        let _ = std::fs::remove_dir_all(&scratch);
    }

    #[test]
    fn a_builder_gets_the_recipe_and_an_absolute_store_whatever_the_launch_was_given() {
        let scratch = std::env::temp_dir().join(format!("romacs-gui-world-{}", std::process::id()));
        std::fs::create_dir_all(&scratch).unwrap();
        // A relative name of the scratch store, from this process's
        // directory, without changing that directory under other tests.
        let cwd = std::env::current_dir().unwrap();
        let mut relative: PathBuf = cwd.components().skip(1).map(|_| "..").collect();
        relative.push(scratch.join("store").strip_prefix("/").unwrap());
        assert!(relative.is_relative());
        let world = open(relative, 800, 600, Some(5000)).unwrap();
        assert!(world.store.is_absolute(), "{}", world.store.display());
        assert_eq!(std::fs::canonicalize(&world.store).unwrap(), std::fs::canonicalize(scratch.join("store")).unwrap());
        let args = builder_args(&world, 800, 600, Some(5000));
        let store = world.store.clone().into_os_string();
        assert_eq!(args, ["--loadup", "--world-store", store.to_str().unwrap(), "--geometry", "800x600", "--gc-floor", "5000"]
            .map(std::ffi::OsString::from));
        let _ = std::fs::remove_dir_all(&scratch);
    }
}
