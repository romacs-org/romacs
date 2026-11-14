//! The GUI's world (`romacs_vm::world`): a live `--loadup` launch keeps the
//! VM at the end of the GNU-loadup chain and the bundled packages'
//! autoloads, with the chain's report beside it. The steps before that point
//! are the launch's own and depend only on its recipe (the launch mode, the
//! window geometry, the launch font and the collection floor) and on keyed
//! host facts. A restored launch attaches its hosts, runs
//! `romacs-world-restart` and continues with the interactive session
//! (`startup::start_session`) and the workspace.
//!
//! A live `--loadup` launch uses a world unless ROMACS_WORLD=off: in
//! `--world-store`, or else in `$XDG_CACHE_HOME/romacs/worlds`
//! (`default_store`). A launch that finds its world missing or stale builds
//! it first, which costs about what a boot from source does, and boots from
//! it; later launches restore it. A build refused or failed on the same
//! inputs is not tried again until one of them changes.
//!
//! The world ends after the chain and Romacs's definitions
//! (`startup::install_session_definitions`), before the session's own
//! startup steps (lisp/romacs/romacs-gui-loadup.el): those read and write the
//! delayed variables and the inputs of this process, which a world build
//! refuses, so a restored launch takes them itself.
//!
//! A launch that loads the user's init restores the init's layer instead
//! when the store holds one (`romacs_vm::world::layer`): the base world
//! with the libraries the init's recorded plan loads, keyed by the base
//! and the plan, replayed after the chain. The launch checks the init is
//! the one planned and leaves out the entries a build refused; a stale or
//! missing layer boots from the base world, records the plan the init
//! makes now, and builds the layer ahead of the next launch, on a thread
//! of its own (`after_init`). `--world-build` prepares the layer with the
//! base once a plan exists.
use crate::startup;
use romacs_vm::Vm;
use romacs_vm::world::layer::{Entry, Kind, Plan, REFUSAL};
use romacs_vm::world::store::{Launch, World};
use serde_json::json;
use std::path::{Path, PathBuf};

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
    // As GNU's `--font FAMILY-POINTS': the default face's font, and the
    // frame's `font-parameter' naming the font as given.
    let source = format!("(progn (defface default '((t (:family {} :height {}))) \"Launch font\")
  (modify-frame-parameters nil '((font-parameter . {}))))",
                         serde_json::to_string(&family).map_err(|e| e.to_string())?, points * 10,
                         serde_json::to_string(&format!("{family}-{points}")).map_err(|e| e.to_string())?);
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
    Some(startup::cache_dir(get)?.join("worlds"))
}

/// This launch with WORLD: the restored VM and its carry, or None to boot
/// from source. A missing or stale world is built first, or the build
/// another launch runs waited for, which costs about what a boot from
/// source does, and the launch then boots from it; ON_WAIT runs before it
/// blocks for that. One whose build was refused on these very inputs, or
/// failed lately (`romacs_vm::world::store`), boots from source. A builder
/// process builds and exits here.
pub fn launch(world: &World, width: u32, height: u32, floor: Option<usize>, on_wait: impl FnOnce()) -> Option<(Box<Vm>, Carry)> {
    match world.launch::<Carry>(|| builder_args(world, width, height, floor, None), true, on_wait) {
        Launch::Restored { vm, carry, manifest } => {
            eprintln!("romacs-world: {}", json!({"mode": "restored", "key": manifest["key"]}));
            Some((vm, carry))
        }
        Launch::Build => build_and_exit(world, width, height, floor, None),
        Launch::Chain { reason } => {
            eprintln!("romacs-world: {}", json!({"mode": "source", "reason": reason}));
            None
        }
    }
}

/// This process is a builder: build BASE's world, or INIT's layer over it
/// when a launch asked for one (`--init-layer`, the arguments
/// `builder_args` gives), and exit. The plan is the store's for INIT less
/// EXCLUDED, and must have DIGEST: a plan that changed since the launch
/// resolved it is not built under its key. The configurations EXCLUDED
/// refused stay deferred in this build too (`Vm::world_layer_load`).
pub fn build_here(base: &World, width: u32, height: u32, floor: Option<usize>, init: Option<&InitLayerArgs>) -> Result<(), String> {
    let Launch::Build = base.launch::<Carry>(Vec::new, false, || {}) else { return Err("not a world builder".into()) };
    let Some(args) = init else { build_and_exit(base, width, height, floor, None) };
    let full = Plan::read(&base.store, &args.init).ok_or_else(|| format!("no plan for {}", args.init.display()))?;
    let plan = full.without(&args.excluded);
    if plan.digest() != args.digest {
        return Err(format!("the plan for {} changed since the launch resolved it", args.init.display()));
    }
    let deferred: Vec<Entry> = args.excluded.iter().filter_map(|&at| full.entries.get(at)).filter(|e| e.kind == Kind::After).cloned().collect();
    build_and_exit(&base.layered(plan.recipe()), width, height, floor, Some((plan, deferred)))
}

/// Build WORLD, with PLAN's libraries for a layer, and exit with the outcome.
fn build_and_exit(world: &World, width: u32, height: u32, floor: Option<usize>, plan: Option<(Plan, Vec<Entry>)>) -> ! {
    let result = build(world, width, height, floor, plan);
    if let Err(error) = &result { eprintln!("romacs-world: build failed: {error}"); }
    std::process::exit(if result.is_ok() { 0 } else { 1 })
}

/// What `--init-layer` names: the init, the resolved plan's digest and the
/// entries left out.
pub struct InitLayerArgs {
    pub init: PathBuf,
    pub digest: String,
    pub excluded: Vec<usize>,
}

impl InitLayerArgs {
    /// From the three words after `--init-layer`: the init, the digest and
    /// the excluded entries as a comma-separated list, or `-` for none.
    pub fn parse(init: &str, digest: &str, excluded: &str) -> Result<InitLayerArgs, String> {
        let excluded = match excluded {
            "-" => Vec::new(),
            list => list.split(',').map(|n| n.parse::<usize>().map_err(|e| format!("--init-layer: {e}"))).collect::<Result<_, _>>()?,
        };
        Ok(InitLayerArgs { init: PathBuf::from(init), digest: digest.to_owned(), excluded })
    }

    fn words(&self) -> [std::ffi::OsString; 3] {
        let excluded = if self.excluded.is_empty() { "-".to_owned() } else {
            self.excluded.iter().map(usize::to_string).collect::<Vec<_>>().join(",")
        };
        [self.init.clone().into_os_string(), self.digest.clone().into(), excluded.into()]
    }
}

/// An init's layer over its base world, as a launch resolved it: the plan
/// the store keeps for the init, the entries left out (indices in that
/// plan) because a build refused them, and the plan replayed.
pub struct Layer {
    pub world: World,
    full: Plan,
    excluded: Vec<usize>,
    pub plan: Plan,
    /// For the last usable layer standing in (`layer`): the digest of the
    /// plan it was resolved from.
    resolved_from: Option<String>,
}

impl Layer {
    fn args(&self) -> InitLayerArgs {
        InitLayerArgs { init: self.full.init.clone(), digest: self.plan.digest(), excluded: self.excluded.clone() }
    }
}

/// The layer a launch may restore for INIT over BASE: the plan the store
/// keeps for INIT, when INIT still has the content it was planned from,
/// resolved (`resolve`). None without a plan, or with an edited init.
///
/// The last layer usable for INIT (`Plan::write_last`) stands in when it was
/// planned from the same init content and everything it holds is still in
/// the plan: at once, when it was resolved from this very plan, so a launch
/// reads none of the refusals that resolution went through; and while the
/// plan's own layer is not built yet, whose libraries it lacks load at
/// their first use and which the launch builds (`after_init`). After an
/// edit of the init, its configuration forms are other code, so the base
/// world boots.
pub fn layer(base: &World, init: &Path) -> Option<Layer> {
    let plan = Plan::read(&base.store, init)?;
    if !plan.current() {
        eprintln!("romacs-world: {}", json!({"event": "layer", "boot": "base", "reason": "the init changed since its plan"}));
        return None;
    }
    let last = Plan::read_last(&base.store, init).filter(|(last, _)| last.init_digest == plan.init_digest && last.within(&plan));
    let usable = |last: Plan, from: String| {
        let world = base.layered(last.recipe());
        world.ready().then(|| Layer { world, full: last.clone(), excluded: Vec::new(), plan: last, resolved_from: Some(from) })
    };
    if let Some((last, from)) = last.clone().filter(|(_, from)| *from == plan.digest()) && let Some(layer) = usable(last, from) {
        return Some(layer);
    }
    let layer = resolve(base, plan)?;
    if layer.world.ready() {
        return Some(layer);
    }
    match last.and_then(|(last, from)| usable(last, from)) {
        Some(previous) => {
            eprintln!("romacs-world: {}", json!({"event": "layer", "previous": true, "entries": previous.plan.entries.len()}));
            Some(previous)
        }
        None => Some(layer),
    }
}

/// LAYER is usable: keep its plan as its init's last usable layer's. Every
/// usable layer was made so here (`build_ahead`), so a launch writes nothing.
fn keep_last(layer: &Layer) {
    if let Err(error) = layer.plan.write_last(&layer.world.store, &layer.full.digest()) {
        eprintln!("romacs-world: {}", json!({"event": "layer", "error": format!("the last layer's plan cannot be kept: {error}")}));
    }
}

/// FULL's layer over BASE less the entries standing refusals name: each
/// refused layer's entries are left out and the rest keyed again, until a
/// key has no standing refusal. None when nothing is left, or a refusal
/// names no entry (the build refused for another reason on these inputs).
pub fn resolve(base: &World, full: Plan) -> Option<Layer> {
    let mut excluded: Vec<usize> = Vec::new();
    for _ in 0..8 {
        let plan = full.without(&excluded);
        if plan.entries.is_empty() {
            return None;
        }
        let world = base.layered(plan.recipe());
        let Some(refusals) = world.standing_refusal() else { return Some(Layer { world, full, excluded, plan, resolved_from: None }) };
        // The build replayed the trimmed plan: its entry indices are
        // positions among the entries kept.
        let kept: Vec<usize> = (0..full.entries.len()).filter(|i| !excluded.contains(i)).collect();
        let named: Vec<usize> = refusals.iter().filter(|r| r["kind"] == REFUSAL)
            .filter_map(|r| r["entry"].as_u64()).filter_map(|j| kept.get(j as usize).copied()).collect();
        if named.is_empty() {
            eprintln!("romacs-world: {}", json!({"event": "layer", "boot": "base", "reason": {"refused": refusals}}));
            return None;
        }
        excluded.extend(named);
        excluded.sort_unstable();
        excluded.dedup();
    }
    None
}

/// LAYER restored, when it is usable now; nothing is built at a launch.
pub fn restore_layer(layer: &Layer) -> Option<(Box<Vm>, Carry)> {
    match layer.world.restore_only::<Carry>() {
        Ok(restored) => {
            let key: String = layer.world.key.iter().map(|b| format!("{b:02x}")).collect();
            eprintln!("romacs-world: {}", json!({"mode": "restored", "key": key, "layer": {"entries": layer.plan.entries.len(), "excluded": layer.excluded}}));
            Some(restored)
        }
        Err(reason) => {
            eprintln!("romacs-world: {}", json!({"event": "layer", "boot": "base", "reason": reason}));
            None
        }
    }
}

/// The worlds of a launch, for what follows the user's init.
pub struct Worlds {
    pub base: World,
    pub layer: Option<Layer>,
    /// The launch restored its layer, not its base.
    pub from_layer: bool,
    pub width: u32,
    pub height: u32,
    pub floor: Option<usize>,
}

/// The user's init has run and PLAN is what it loaded (None: nothing was
/// recorded). Keep the plan when it differs from the store's in loads or
/// in the init's content, and unless this launch restored the layer of
/// these very loads, build that layer ahead of the next launch, on a
/// thread of its own.
pub fn after_init(worlds: &Worlds, plan: Option<Plan>) {
    let Some(recorded) = plan else { return };
    // An init that loads nothing has no layer.
    if recorded.entries.is_empty() {
        return;
    }
    // Restored from a layer, the init issued no autoload for a library the
    // layer holds: the layer's plan stays, with any new load added.
    let plan = match (&worlds.layer, worlds.from_layer) {
        (Some(layer), true) => layer.plan.merged(&recorded),
        _ => recorded,
    };
    // The store's plan stays when this one loads as it does: what is built
    // is the store's, whose digest a builder checks; this plan's forms may
    // print other generated names (`Entry::same`).
    let plan = match Plan::read(&worlds.base.store, &plan.init) {
        Some(kept) if !plan.supersedes(Some(&kept)) => kept,
        _ => {
            if let Err(error) = plan.write(&worlds.base.store) {
                eprintln!("romacs-world: {}", json!({"event": "layer", "error": format!("the plan cannot be kept: {error}")}));
                return;
            }
            eprintln!("romacs-world: {}", json!({"event": "layer-plan", "entries": plan.entries.len()}));
            plan
        }
    };
    // Restored the layer of these loads, or the one resolved from this plan.
    if worlds.from_layer && worlds.layer.as_ref().is_some_and(|layer| layer.plan.same_loads(&plan) || layer.resolved_from.as_ref() == Some(&plan.digest())) {
        return;
    }
    let (base, width, height, floor) = (worlds.base.clone(), worlds.width, worlds.height, worlds.floor);
    std::thread::spawn(move || build_ahead(&base, plan, width, height, floor));
}

/// The layer of FULL over BASE made usable: built here, or waited for,
/// and after a build that refused entries, built again without them, a
/// few times over. Whether a layer is usable at the end.
fn build_ahead(base: &World, full: Plan, width: u32, height: u32, floor: Option<usize>) -> bool {
    for _ in 0..4 {
        let Some(layer) = resolve(base, full.clone()) else { return false };
        if layer.world.ready() {
            keep_last(&layer);
            return true;
        }
        let key: String = layer.world.key.iter().map(|b| format!("{b:02x}")).collect();
        eprintln!("romacs-world: {}", json!({"event": "layer-build", "key": key, "entries": layer.plan.entries.len()}));
        match layer.world.prepare(|| builder_args(&layer.world, width, height, floor, Some(&layer.args()))) {
            Ok(_) => {
                eprintln!("romacs-world: {}", json!({"event": "layer-built", "key": key}));
                keep_last(&layer);
                return true;
            }
            Err(reason) => {
                eprintln!("romacs-world: {}", json!({"event": "layer-build-failed", "key": key, "reason": reason}));
                if layer.world.standing_refusal().is_none() {
                    return false;
                }
            }
        }
    }
    false
}

/// `--world-build`'s second half: INIT's layer over BASE made usable too,
/// when the store holds INIT's plan and the init is the one planned.
pub fn prepare_layer(base: &World, init: &Path, width: u32, height: u32, floor: Option<usize>) {
    match Plan::read(&base.store, init) {
        Some(plan) if plan.current() => {
            let built = build_ahead(base, plan, width, height, floor);
            eprintln!("romacs-world: {}", json!({"event": "layer-prepared", "ok": built}));
        }
        Some(_) => eprintln!("romacs-world: {}", json!({"event": "layer-prepared", "ok": false, "reason": "the init changed since its plan"})),
        None => eprintln!("romacs-world: {}", json!({"event": "layer-prepared", "ok": false, "reason": "no plan: the init has not run"})),
    }
}

/// WORLD made usable for this launch's recipe, with no window: built here,
/// or waited for, when it is missing or stale (`World::prepare`). Says how
/// it ended on stderr as a launch says how it booted.
pub fn prepare(world: &World, width: u32, height: u32, floor: Option<usize>) -> Result<(), String> {
    match world.prepare(|| builder_args(world, width, height, floor, None)) {
        Ok(manifest) => {
            eprintln!("romacs-world: {}", json!({"mode": "prepared", "key": manifest["key"]}));
            Ok(())
        }
        Err(reason) => Err(format!("world not prepared: {reason}")),
    }
}

/// A builder's arguments for WORLD's launch: what its recipe takes from the
/// command line, the store absolute, since the builder runs in the store's
/// pinned directory. Nothing else of this launch reaches it.
fn builder_args(world: &World, width: u32, height: u32, floor: Option<usize>, layer: Option<&InitLayerArgs>) -> Vec<std::ffi::OsString> {
    let mut args: Vec<std::ffi::OsString> = vec!["--loadup".into(), "--world-store".into(), world.store.clone().into(),
                                                 "--geometry".into(), format!("{width}x{height}").into()];
    if let Some(floor) = floor {
        args.extend(["--gc-floor".into(), floor.to_string().into()]);
    }
    if let Some(layer) = layer {
        args.push("--init-layer".into());
        args.extend(layer.words());
    }
    args
}

/// The read roots of a build beside the store's own: the Lisp trees and
/// the fonts the layout engine measures with (a batch world's too,
/// `batch::boot`, which measures none).
pub fn build_roots(dir: &std::path::Path) -> Vec<PathBuf> {
    let mut roots = vec![dir.to_owned()];
    roots.extend(startup::runtime_of(dir).map(|root| root.join("lisp/romacs")));
    // The bundled layout, which the bundled autoloads' scan reads.
    roots.extend(startup::runtime_of(dir).map(|root| root.join("lisp/bundled.json")));
    roots.extend(startup::bundled_load_path(dir).unwrap_or_default());
    roots.extend(crate::fonts::icon_font_dirs());
    roots.extend(["/usr/share/fonts", "/etc/fonts", "/nix/store"].map(PathBuf::from));
    if let Some(home) = romacs_vm::host::getenv("HOME").map(PathBuf::from) {
        roots.extend([home.join(".local/share/fonts"), home.join(".fonts"), home.join(".cache/fontconfig")]);
    }
    roots
}

/// The builder: the launch's steps up to the world's end, with PLAN's
/// libraries for a layer, headless, on the evaluator's stack, then publish.
fn build(world: &World, width: u32, height: u32, floor: Option<usize>, plan: Option<(Plan, Vec<Entry>)>) -> Result<(), String> {
    let world = world.clone();
    crate::spawn_evaluator(move || -> Result<(), String> {
        // The fonts load before the build is confined.
        let engine = std::rc::Rc::new(std::cell::RefCell::new(romacs_layout::LayoutEngine::new()?));
        let dir = startup::lisp_root(&startup::lisp_dir())?;
        let mut roots = build_roots(&dir);
        roots.extend(plan.iter().flat_map(|(plan, _)| plan.directories()));
        world.begin_build(&world.build_roots(roots)).map_err(|e| e.to_string())?;
        let len = world.build(|| build_world(width, height, floor, &dir, engine, plan.as_ref()))?;
        eprintln!("romacs-world: {}", json!({"event": "world-build", "ok": true, "image_bytes": len}));
        Ok(())
    })?.join().map_err(|_| "world builder panicked".to_owned())?
}

/// The build's steps once it records: up to the chain's end, then PLAN's
/// libraries for a layer. The world's VM, its carry and its events (the
/// plan replayed, with the entries refused), for `World::build` to publish.
fn build_world(width: u32, height: u32, floor: Option<usize>, dir: &std::path::Path,
               engine: std::rc::Rc<std::cell::RefCell<romacs_layout::LayoutEngine>>, plan: Option<&(Plan, Vec<Entry>)>) -> Result<(Vm, Carry, serde_json::Value), String> {
    let mut vm = Vm::new();
    vm.world_begin();
    vm.set_layout_host(Some(Box::new(crate::measure::Host { engine: engine.clone() })));
    // A live launch attaches its terminal service here; the world keeps
    // what that declares, and the restored launch attaches the service.
    vm.declare_terminal_errors()?;
    vm.set_echo_area_host(true);
    if let Some(floor) = floor { vm.set_collection_floor(floor); }
    let (line_height, column_width) = romacs_layout::frame_metrics(&vm.display_default_face()?)?;
    vm.set_frame_geometry(width, height, line_height, column_width)?;
    vm.eval_source(INTERPROGRAM)?;
    let font = launch_font()?;
    let loadup = startup::initialize_chain(&mut vm, startup::Launch::Loadup, font.as_ref().map(|(s, _)| s.as_str()), dir)?;
    startup::install_session_definitions(&mut vm, startup::Launch::Loadup, dir)?;
    let events = match plan {
        Some((plan, deferred)) => {
            let refused = vm.world_layer_load(plan, deferred);
            eprintln!("romacs-world: {}", json!({"event": "layer-load", "entries": plan.entries.len(), "refused": refused}));
            json!({"init-layer": plan.json(), "refused": refused})
        }
        None => json!({}),
    };
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
    Ok((vm, Carry { loadup }, events))
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

    /// The bundled autoloads' scan reads the layout beside lisp/romacs, which a
    /// confined build reads only inside its roots.
    #[test]
    fn a_build_reads_the_bundled_trees_and_their_layout() {
        let roots = build_roots(std::path::Path::new("/r/lisp/gnu"));
        for path in ["/r/lisp/gnu", "/r/lisp/romacs", "/r/lisp/bundled.json"] {
            assert!(roots.contains(&PathBuf::from(path)), "{path}: {roots:?}");
        }
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
        let args = builder_args(&world, 800, 600, Some(5000), None);
        let store = world.store.clone().into_os_string();
        assert_eq!(args, ["--loadup", "--world-store", store.to_str().unwrap(), "--geometry", "800x600", "--gc-floor", "5000"]
            .map(std::ffi::OsString::from));
        let layer = InitLayerArgs { init: PathBuf::from("/h/init.el"), digest: "abc".into(), excluded: vec![2, 5] };
        let args = builder_args(&world, 800, 600, None, Some(&layer));
        assert_eq!(&args[5..], ["--init-layer", "/h/init.el", "abc", "2,5"].map(std::ffi::OsString::from));
        let parsed = InitLayerArgs::parse("/h/init.el", "abc", "2,5").unwrap();
        assert_eq!((parsed.init.as_path(), parsed.digest.as_str(), parsed.excluded.as_slice()), (Path::new("/h/init.el"), "abc", &[2, 5][..]));
        assert!(InitLayerArgs::parse("/h/init.el", "abc", "-").unwrap().excluded.is_empty());
        assert!(InitLayerArgs::parse("/h/init.el", "abc", "x").is_err());
        let _ = std::fs::remove_dir_all(&scratch);
    }
}
