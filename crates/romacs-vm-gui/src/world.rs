//! The GUI's world (`romacs_vm::world`): a live `--loadup` launch keeps the
//! VM at the end of the GNU-loadup chain and the bundled packages'
//! autoloads, with the chain's report beside it. The world ends at GNU's
//! dump point, as the batch world does (`romacs--dumping`): it holds no
//! startup step, so nothing of its builder's HOME, XDG directories, locale
//! or login name, and no font, window geometry or collection floor. Those
//! are each process's: a restored launch sets its frame's geometry and
//! floor, attaches its hosts, sets its launch font (`LaunchFont`, as GNU's
//! `--font' is), runs `romacs-world-restart`, which takes the startup steps,
//! and continues with the interactive session (`startup::start_session`)
//! and the workspace. One world serves every launch of this executable over
//! this Lisp tree.
//!
//! A live `--loadup` launch uses a world unless ROMACS_WORLD=off: in
//! `--world-store`, or else in `$XDG_CACHE_HOME/romacs/worlds`
//! (`default_store`). A launch that finds its world missing or stale builds
//! it first, which costs about what a boot from source does, and boots from
//! it; later launches restore it. A build that could not capture or failed on the same
//! inputs is not tried again until one of them changes.
//!
//! The world ends after the chain and Romacs's definitions
//! (`startup::install_session_definitions`), before the session's own
//! startup steps (lisp/session/romacs-gui-loadup.el): those read and write the
//! delayed variables and the inputs of this process, which a world build
//! does not capture, so a restored launch takes them itself. The user's
//! init loads after it, its libraries from the compiled-Lisp store.
use crate::startup;
use romacs_vm::Vm;
use romacs_vm::world::store::{Launch, World};
use serde_json::json;
use std::path::{Path, PathBuf};

/// What a GUI world carries beside its VM: the chain's report.
pub struct Carry {
    pub loadup: crate::loadup::Loadup,
}
romacs_snapshot::snapshot_struct!(Carry { loadup });

/// The launch font, from ROMACS_GUI_FONT and ROMACS_GUI_FONT_SIZE: this
/// process's, as GNU's `--font FAMILY-POINTS' is, so no world holds it and
/// a world serves every launch font.
pub struct LaunchFont {
    pub family: String,
    pub points: u32,
}

/// The default face's font (lisp/session/romacs-startup.el), which a launch
/// font naming only a family or only a size keeps the rest of.
const DEFAULT_FAMILY: &str = "Iosevka";
const DEFAULT_POINTS: u32 = 14;

impl LaunchFont {
    pub fn from_env() -> Result<Option<LaunchFont>, String> {
        Self::from_values(std::env::var("ROMACS_GUI_FONT").ok(), std::env::var("ROMACS_GUI_FONT_SIZE").ok())
    }

    /// The launch font a FAMILY and a SIZE name, either of them alone
    /// keeping the default face's other half; none without either.
    fn from_values(family: Option<String>, size: Option<String>) -> Result<Option<LaunchFont>, String> {
        if family.is_none() && size.is_none() { return Ok(None) }
        let family = family.unwrap_or_else(|| DEFAULT_FAMILY.into());
        if family.chars().any(char::is_control) { return Err("font family contains a control character".into()); }
        let points = size.map_or(Ok(DEFAULT_POINTS), |size| size.parse::<u32>().map_err(|e| e.to_string()))?;
        if !(6..=72).contains(&points) { return Err("font size must be 6..72 points".into()); }
        Ok(Some(LaunchFont { family, points }))
    }

    /// The default face's spec, and the frame's `font-parameter' naming
    /// the font as given, each as Lisp.
    fn parts(&self) -> (String, String) {
        let family = lisp_string(&self.family);
        (format!("((t (:family {family} :height {})))", self.points * 10),
         format!("(modify-frame-parameters nil '((font-parameter . {})))", lisp_string(&format!("{}-{}", self.family, self.points))))
    }

    /// The font declared before GNU's Lisp, whose own `defface' (custom.el)
    /// then leaves the default face's spec alone.
    pub fn declaration(&self) -> String {
        let (spec, frame) = self.parts();
        format!("(progn (defface default '{spec} \"Launch font\") {frame})")
    }

    /// The font set in a restored session, where GNU's Lisp declared the
    /// default face, before the init runs: its spec becomes the launch
    /// font's, as the declaration leaves it, and the face takes the font on
    /// every frame, whatever theme the world enabled.
    pub fn restored(&self) -> String {
        let (spec, frame) = self.parts();
        format!("(progn (put 'default 'face-defface-spec '{spec}) (set-face-attribute 'default nil :family {} :height {}) {frame})",
                lisp_string(&self.family), self.points * 10)
    }
}

/// The steps a live `--loadup` launch takes before the chain, which its
/// world holds.
pub const INTERPROGRAM: &str = "(defvar interprogram-cut-function 'romacs-clipboard-select-text) (defvar interprogram-paste-function 'romacs-clipboard-selection-value)";

/// The world of a live `--loadup` launch in STORE, resolved against this
/// process's directory (`World::new`).
pub fn open(store: PathBuf) -> Result<World, String> {
    World::new(store, recipe()?).map_err(|e| e.to_string())
}

/// The recipe of a live `--loadup` launch: the chain over this Lisp tree.
fn recipe() -> Result<Vec<String>, String> {
    let dir = startup::lisp_root(&startup::lisp_dir())?;
    Ok(vec!["consumer romacs-vm-gui".to_owned(), "launch loadup live".into(), format!("lisp {}", dir.display())])
}

/// Where a launch keeps its worlds when no --world-store names a place:
/// `$XDG_CACHE_HOME/romacs/worlds`, or `~/.cache/romacs/worlds` (the XDG
/// base directory default), from GET. None without an absolute directory
/// to put it in.
pub fn default_store(get: impl Fn(&str) -> Option<std::ffi::OsString>) -> Option<PathBuf> {
    Some(startup::cache_dir(get)?.join("worlds"))
}

/// This launch with WORLD: the restored VM and its carry, or None to boot
/// from source. A missing or stale world is built first, or the build
/// another launch runs waited for, which costs about what a boot from
/// source does, and the launch then boots from it; ON_WAIT runs, with why
/// the world was not usable, before it blocks for that. One whose build
/// could not capture these very inputs, or failed lately
/// (`romacs_vm::world::store`), boots from source. A builder process builds
/// and exits here.
pub fn launch(world: &World, on_wait: impl FnOnce(&serde_json::Value)) -> Option<(Box<Vm>, Carry)> {
    match world.launch::<Carry>(|| builder_args(world), true, on_wait) {
        Launch::Restored { vm, carry, manifest } => {
            romacs_vm::host::event_line("romacs-world", &json!({"mode": "restored", "key": manifest["key"], "guard": format!("{:?}", romacs_vm::world::arena::executable_guard())}));
            Some((vm, carry))
        }
        Launch::Build => build_and_exit(world),
        Launch::Chain { reason } => {
            romacs_vm::host::event_line("romacs-world", &json!({"mode": "source", "reason": reason}));
            None
        }
    }
}

/// The installation's store beside this executable (`EXECUTABLE.worlds`),
/// when there is one: an installer fills it with `--world-build`.
pub fn installed_store() -> Option<PathBuf> {
    crate::batch::beside_executable("worlds").filter(|store| store.is_dir())
}

/// This launch's world from the installation's store ([`installed_store`]),
/// restored read only without asking its records again
/// (`World::restore_read_only`): None, saying why on stderr, when the store
/// has no world for this executable and Lisp tree, and the launch goes on
/// to the user's store. Nothing is built or written there.
pub fn installed() -> Option<(World, Box<Vm>, Carry)> {
    let store = installed_store()?;
    match World::restore_read_only::<Carry>(&store, recipe().ok()?) {
        Ok((world, vm, carry)) => {
            romacs_vm::host::event_line("romacs-world", &json!({"mode": "restored", "installed": true, "key": hex(&world.key),
                                                                "guard": format!("{:?}", romacs_vm::world::arena::executable_guard())}));
            Some((world, vm, carry))
        }
        Err(reason) => {
            romacs_vm::host::event_line("romacs-world", &json!({"installed": store, "reason": reason}));
            None
        }
    }
}

/// In a worker isolate: the world of KEY in STORE, restored as its session
/// restored it. From the installation's store ([`installed_store`]) read
/// only, its records not asked again and nothing written there, as a launch
/// reads it ([`installed`]), when this executable and Lisp tree key it as
/// KEY; from any other store checked and used as `World::restore_only` does.
pub fn adopt(store: PathBuf, key: [u8; 32]) -> Result<(Box<Vm>, Carry), String> {
    let installed = installed_store().and_then(|installed| std::fs::canonicalize(installed).ok());
    if installed.is_some() && std::fs::canonicalize(&store).ok() == installed {
        let (world, vm, carry) = World::restore_read_only::<Carry>(&store, recipe()?).map_err(|reason| reason.to_string())?;
        if world.key != key {
            return Err(format!("the installation's world is {}, not {}", hex(&world.key), hex(&key)));
        }
        return Ok((vm, carry));
    }
    World { store, key, recipe: Vec::new() }.restore_only::<Carry>().map_err(|reason| reason.to_string())
}

/// KEY in hexadecimal, as a manifest names it.
fn hex(key: &[u8; 32]) -> String {
    key.iter().map(|b| format!("{b:02x}")).collect()
}

/// This process is a builder: build WORLD and exit.
pub fn build_here(world: &World) -> Result<(), String> {
    let Launch::Build = world.launch::<Carry>(Vec::new, false, |_| {}) else { return Err("not a world builder".into()) };
    build_and_exit(world)
}

/// Build WORLD and exit with the outcome.
fn build_and_exit(world: &World) -> ! {
    let result = build(world);
    if let Err(error) = &result { eprintln!("romacs-world: build failed: {error}"); }
    std::process::exit(if result.is_ok() { 0 } else { 1 })
}

/// The warning a session shows once when its world store cannot be used
/// and it boots from source: REASON names the path and what is wrong. The
/// store's checks keep a directory another local user owns, or could swap,
/// from handing this session an image of theirs, which runs as code.
pub fn store_warning_form(reason: &str) -> String {
    let message = format!(
        "Romacs started from source, without its saved world, because {reason}. \
         A world store holds images this session runs as code, so Romacs uses one only where no one else \
         can change it or the directories above it. To fix it, correct the ownership or permissions named \
         above, or give the store a directory of your own with --world-store DIR or XDG_CACHE_HOME.");
    format!("(display-warning 'romacs-world {} :warning)", lisp_string(&message))
}

/// TEXT in Lisp's string read syntax.
fn lisp_string(text: &str) -> String {
    format!("\"{}\"", text.replace('\\', "\\\\").replace('"', "\\\""))
}

/// `--world-census`: the census (`romacs_vm::world::image::census`) of the
/// base world in STORE of a live `--loadup` launch, restored read only as
/// it was published, before anything of this process runs in it. Nothing
/// is written to the store.
pub fn census(store: &Path) -> Result<serde_json::Value, String> {
    let (_, vm, carry) = World::restore_read_only::<Carry>(store, recipe()?).map_err(|reason| reason.to_string())?;
    let census = romacs_vm::world::image::census(&vm, &carry);
    // The process exits next: the image need not be taken apart.
    std::mem::forget((vm, carry));
    Ok(census)
}

/// WORLD made usable for this launch's recipe, with no window: built here,
/// or waited for, when it is missing or stale (`World::prepare`). Says on
/// stderr why it builds, and how it ended with the key and the recipe, as
/// a launch says how it booted.
pub fn prepare(world: &World) -> Result<(), String> {
    let building = |reason: &serde_json::Value| romacs_vm::host::event_line("romacs-world", &json!({"event": "world-build", "reason": reason}));
    match world.prepare(|| builder_args(world), building) {
        Ok(manifest) => {
            romacs_vm::host::event_line("romacs-world", &json!({"mode": "prepared", "key": manifest["key"], "recipe": world.recipe}));
            Ok(())
        }
        Err(reason) => Err(format!("world not prepared: {reason}")),
    }
}

/// A builder's arguments for WORLD's launch: the store absolute, since the
/// builder runs in the store's pinned directory. Nothing of this launch
/// reaches it.
fn builder_args(world: &World) -> Vec<std::ffi::OsString> {
    vec!["--loadup".into(), "--world-store".into(), world.store.clone().into()]
}

/// The builder: the launch's steps up to the world's end, headless, on the
/// evaluator's stack, then publish.
fn build(world: &World) -> Result<(), String> {
    let world = world.clone();
    crate::spawn_evaluator(move || -> Result<(), String> {
        let dir = startup::lisp_root(&startup::lisp_dir())?;
        world.begin_build().map_err(|e| e.to_string())?;
        // GNU's dump point: the chain leaves its startup steps to each
        // restored process (`romacs-world-restart`).
        romacs_vm::host::end_at_dump_point();
        let len = world.build(|| build_world(&dir))?;
        romacs_vm::host::event_line("romacs-world", &json!({"event": "world-build", "ok": true, "image_bytes": len}));
        Ok(())
    })?.join().map_err(|_| "world builder panicked".to_owned())?
}

/// The build's steps once it records: up to the chain's end, with no font
/// and on GNU batch's initial frame, as the batch world's chain; each
/// restored launch sets its own frame's geometry. The world's VM, its carry
/// and its events, for `World::build` to publish.
fn build_world(dir: &std::path::Path) -> Result<(Vm, Carry, serde_json::Value), String> {
    let mut vm = Vm::new();
    vm.world_begin();
    // A live launch attaches its terminal service here; the world keeps
    // what that declares, and the restored launch attaches the service.
    vm.declare_terminal_errors()?;
    vm.set_echo_area_host(true);
    vm.set_frame_geometry(80, 25, 1, 1)?;
    vm.eval_source(INTERPROGRAM)?;
    // A world holds no launch font (`LaunchFont`).
    let loadup = startup::initialize_chain(&mut vm, startup::Launch::Loadup, None, dir)?;
    startup::install_session_definitions(&mut vm, startup::Launch::Loadup, dir)?;
    Ok((vm, Carry { loadup }, json!({})))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Either half of a launch font alone keeps the default face's other.
    #[test]
    fn a_launch_font_naming_one_half_keeps_the_default_other() {
        let font = |family: Option<&str>, size: Option<&str>| {
            LaunchFont::from_values(family.map(str::to_owned), size.map(str::to_owned))
                .map(|font| font.map(|font| (font.family, font.points)))
        };
        assert_eq!(font(None, None), Ok(None));
        assert_eq!(font(None, Some("18")), Ok(Some(("Iosevka".to_owned(), 18))));
        assert_eq!(font(Some("Monospace"), None), Ok(Some(("Monospace".to_owned(), 14))));
        assert_eq!(font(Some("Monospace"), Some("15")), Ok(Some(("Monospace".to_owned(), 15))));
        assert!(font(None, Some("90")).is_err());
    }

    #[test]
    fn an_unusable_store_says_why_and_how_to_fix_it() {
        let form = store_warning_form("/tmp/x is writable by others and not sticky");
        assert!(form.starts_with("(display-warning 'romacs-world \"Romacs started from source"), "{form}");
        assert!(form.contains("because /tmp/x is writable by others and not sticky.") && form.contains("--world-store DIR"), "{form}");
        // A reason's quotes and backslashes stay inside the one string.
        assert!(store_warning_form(r#"/a "b" \c"#).contains(r#"/a \"b\" \\c"#));
    }

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
    fn a_builder_gets_the_recipe_and_an_absolute_store_whatever_the_launch_was_given() {
        let scratch = std::env::temp_dir().join(format!("romacs-gui-world-{}", std::process::id()));
        std::fs::create_dir_all(&scratch).unwrap();
        // A relative name of the scratch store, from this process's
        // directory, without changing that directory under other tests.
        let cwd = std::env::current_dir().unwrap();
        let mut relative: PathBuf = cwd.components().skip(1).map(|_| "..").collect();
        relative.push(scratch.join("store").strip_prefix("/").unwrap());
        assert!(relative.is_relative());
        let world = open(relative).unwrap();
        assert!(world.store.is_absolute(), "{}", world.store.display());
        assert_eq!(std::fs::canonicalize(&world.store).unwrap(), std::fs::canonicalize(scratch.join("store")).unwrap());
        let args = builder_args(&world);
        let store = world.store.clone().into_os_string();
        assert_eq!(args, ["--loadup", "--world-store", store.to_str().unwrap()]
            .map(std::ffi::OsString::from));
        let _ = std::fs::remove_dir_all(&scratch);
    }
}
