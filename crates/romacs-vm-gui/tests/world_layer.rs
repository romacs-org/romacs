//! An init's layer in the store (`romacs_vm::world::layer`, world.rs):
//! `--world-build` prepares it beside the base world once the init's plan
//! is there, builds it again when a library it loaded changed, and leaves
//! it alone when the init changed since its plan; `--world-reset=all`
//! forgets the plans too. No display is needed.
use romacs_vm::world::layer::{Entry, Kind, Plan};
use std::path::{Path, PathBuf};

struct Home {
    home: PathBuf,
    store: PathBuf,
    init: PathBuf,
    library: PathBuf,
}

impl Home {
    fn new(name: &str) -> Home {
        let home = std::env::temp_dir().join(format!("romacs-world-layer-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&home);
        let lisp = home.join("lisp");
        std::fs::create_dir_all(&lisp).unwrap();
        std::fs::create_dir_all(home.join("config/romacs")).unwrap();
        let library = lisp.join("layer-lib.el");
        std::fs::write(&library, "(defvar layer-lib-loaded 1)\n(provide 'layer-lib)\n").unwrap();
        let init = home.join("config/romacs/init.el");
        std::fs::write(&init, format!("(setq load-path (cons {:?} load-path))\n(require 'layer-lib)\n", lisp.display().to_string())).unwrap();
        Home { store: home.join("cache/romacs/worlds"), home, init, library }
    }

    fn command(&self, args: &[&str]) -> (bool, String) {
        let output = std::process::Command::new(env!("CARGO_BIN_EXE_romacs-vm-gui")).args(args)
            .env("HOME", &self.home).env("XDG_CACHE_HOME", self.home.join("cache")).env("XDG_CONFIG_HOME", self.home.join("config"))
            .env_remove("WAYLAND_DISPLAY").env_remove("DISPLAY").env_remove("ROMACS_WORLD").output().unwrap();
        (output.status.success(), String::from_utf8_lossy(&output.stderr).into_owned())
    }

    fn build(&self) -> String {
        let (ok, said) = self.command(&["--loadup", "--world-build"]);
        assert!(ok && said.contains(r#""mode":"prepared""#), "{said}");
        said
    }

    /// The plan a launch would have recorded for this init.
    fn plan(&self) {
        let entries = vec![Entry { kind: Kind::Require, name: "layer-lib".into(), file: None, noerror: false, load_path: 0 }];
        let plan = Plan::new(&self.init, entries, vec![vec![self.home.join("lisp").display().to_string()]]).unwrap();
        plan.write(&self.store).unwrap();
    }

    /// The store's images with their modification times, keyed by the
    /// world they belong to: the base, or the layer (its manifest's events
    /// hold the plan).
    fn images(&self) -> Vec<(&'static str, std::time::SystemTime)> {
        let mut images = Vec::new();
        for entry in std::fs::read_dir(&self.store).unwrap() {
            let path = entry.unwrap().path();
            if path.extension().is_some_and(|x| x == "rwi") {
                let manifest = std::fs::read_to_string(path.with_extension("json")).unwrap();
                let kind = if manifest.contains("\"init-layer\"") { "layer" } else { "base" };
                images.push((kind, std::fs::metadata(&path).unwrap().modified().unwrap()));
            }
        }
        images.sort();
        images
    }
}

fn modified(images: &[(&str, std::time::SystemTime)], kind: &str) -> Option<std::time::SystemTime> {
    images.iter().find(|(k, _)| *k == kind).map(|(_, at)| *at)
}

#[test]
fn world_build_prepares_the_init_layer_and_rebuilds_it_only_for_its_inputs() {
    let home = Home::new("build");
    let said = home.build();
    assert!(said.contains(r#""event":"layer-prepared","ok":false"#) && said.contains("no plan"), "{said}");
    assert_eq!(home.images().len(), 1);
    home.plan();
    let said = home.build();
    assert!(said.contains(r#""event":"layer-built""#) && said.contains(r#""event":"layer-prepared","ok":true"#), "{said}");
    let first = home.images();
    assert_eq!(first.iter().map(|(k, _)| *k).collect::<Vec<_>>(), ["base", "layer"], "{first:?}");
    // Nothing changed: nothing is built.
    let said = home.build();
    assert!(!said.contains(r#""event":"layer-build""#), "{said}");
    assert_eq!(home.images(), first);
    // The library changed: the layer is built again under its key, the base is not.
    std::fs::write(&home.library, "(defvar layer-lib-loaded 2)\n(provide 'layer-lib)\n").unwrap();
    let said = home.build();
    assert!(said.contains(r#""event":"layer-built""#), "{said}");
    let second = home.images();
    assert_eq!(modified(&second, "base"), modified(&first, "base"));
    assert_ne!(modified(&second, "layer"), modified(&first, "layer"));
    // The init changed since its plan: no layer is prepared from the old plan.
    std::fs::write(&home.init, "(require 'layer-lib)\n").unwrap();
    let said = home.build();
    assert!(said.contains("the init changed since its plan"), "{said}");
    assert_eq!(home.images(), second);
    // A reset forgets the plans with the worlds.
    let (ok, _) = home.command(&["--world-reset=all"]);
    assert!(ok);
    assert!(!home.store.join("plans").exists() || std::fs::read_dir(home.store.join("plans")).unwrap().next().is_none());
    assert!(home.images().is_empty());
    let _ = std::fs::remove_dir_all(&home.home);
}

#[test]
fn a_plan_is_kept_by_its_init_and_read_back_whole() {
    let home = Home::new("plan");
    std::fs::create_dir_all(&home.store).unwrap();
    home.plan();
    let plan = Plan::read(&home.store, &home.init).expect("the plan");
    assert_eq!(plan.entries.len(), 1);
    assert!(plan.current());
    assert!(Plan::read(&home.store, Path::new("/elsewhere/init.el")).is_none());
    let _ = std::fs::remove_dir_all(&home.home);
}
