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
        self.plan_requiring("layer-lib", &self.home.join("lisp"));
    }

    /// A plan whose one entry requires FEATURE from DIR.
    fn plan_requiring(&self, feature: &str, dir: &Path) {
        let entries = vec![Entry { kind: Kind::Require, name: feature.into(), file: None, noerror: false, load_path: 0, declared: false, forms: Vec::new() }];
        let plan = Plan::new(&self.init, entries, vec![vec![dir.display().to_string()]]).unwrap();
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
                // A batch session's world (src/batch.rs) is neither.
                if manifest.contains("\"launch batch\"") { continue; }
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

/// cus-start, loaded again in full as Custom's `require' loads it after
/// loadup's preload, asks whether the variables startup computes are bound
/// and puts the `custom-set' startup reads: the layer holds it.
#[test]
fn an_init_layer_loads_cus_start_in_full() {
    let home = Home::new("cus-start");
    let gnu_lisp = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../lisp/gnu").canonicalize().unwrap();
    home.build();
    home.plan_requiring("cus-start", &gnu_lisp);
    let said = home.build();
    assert!(said.contains(r#""event":"layer-built""#) && said.contains(r#""event":"layer-prepared","ok":true"#), "{said}");
    assert!(!said.contains("refused"), "{said}");
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

/// A library the init loads at startup that the layer cannot hold (here
/// it signals) takes every later entry with it: the layer holds the entries
/// before it, and the init loads it and the rest in its own order.
#[test]
fn a_startup_library_left_out_ends_capture_there() {
    let home = Home::new("prefix");
    let lisp = home.home.join("lisp");
    std::fs::write(lisp.join("layer-bad.el"), "(error \"romacs-fixture: this library fails\")\n").unwrap();
    std::fs::write(lisp.join("layer-tail.el"), "(defvar layer-tail-loaded 1)\n(provide 'layer-tail)\n").unwrap();
    home.build();
    let require = |name: &str| Entry { kind: Kind::Require, name: name.into(), file: None, noerror: false, load_path: 0, declared: false, forms: Vec::new() };
    let plan = Plan::new(&home.init, vec![require("layer-lib"), require("layer-bad"), require("layer-tail")], vec![vec![lisp.display().to_string()]]).unwrap();
    plan.write(&home.store).unwrap();
    let said = home.build();
    assert!(said.contains(r#""event":"layer-prepared","ok":true"#), "{said}");
    let built: Vec<Vec<String>> = std::fs::read_dir(&home.store).unwrap().filter_map(|e| {
        let path = e.unwrap().path();
        let manifest: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(&path).ok()?).ok()?;
        let entries = manifest["events"]["init-layer"]["entries"].as_array()?.clone();
        manifest["refusals"].as_array().is_none_or(|r| r.is_empty())
            .then(|| entries.iter().map(|e| e["name"].as_str().unwrap().to_owned()).collect())
    }).collect();
    assert_eq!(built, [vec!["layer-lib".to_owned()]], "{said}");
    let _ = std::fs::remove_dir_all(&home.home);
}

/// Libraries the init only declared are built into its layer after their
/// configuration's forms. One whose configuration asks for a secret,
/// through auth-source, straight from the password store or from a plaintext
/// authinfo file, is built in too: the form that asked is kept as code for
/// the init's own call, and nothing the store holds contains the secret.
#[test]
fn an_init_layer_holds_declared_libraries_and_never_a_secret() {
    const SECRET: &str = "romacs-fixture-secret-7f3a9c";
    let home = Home::new("declared");
    let lisp = home.home.join("lisp");
    for (name, text) in [
        ("layer-cfg", "(defvar layer-cfg-map (make-sparse-keymap))\n(defvar layer-cfg-hook nil)\n(provide 'layer-cfg)\n"),
        ("layer-cmd", "(defun layer-cmd () (interactive) 'ran)\n(provide 'layer-cmd)\n"),
        ("layer-secret", "(defvar layer-secret-value nil)\n(provide 'layer-secret)\n"),
        ("layer-reader", "(defvar layer-reader-value nil)\n(provide 'layer-reader)\n"),
        ("layer-closure", "(provide 'layer-closure)\n"),
        ("layer-withheld", "(provide 'layer-withheld)\n"),
        ("layer-netrc", "(defvar layer-netrc-value nil)\n(provide 'layer-netrc)\n"),
        ("layer-timer", "(defvar layer-timer-set nil)\n(provide 'layer-timer)\n"),
        ("layer-if", "(defvar layer-if-value nil)\n(provide 'layer-if)\n"),
        ("layer-fn", "(provide 'layer-fn)\n"),
        ("layer-sym", "(defun layer-sym-config () (error \"romacs-fixture: the symbol function was called\"))\n(provide 'layer-sym)\n"),
    ] {
        std::fs::write(lisp.join(format!("{name}.el")), text).unwrap();
    }
    let pass = home.home.join(".password-store/xmpp");
    std::fs::create_dir_all(&pass).unwrap();
    let entry = pass.join("me.gpg");
    std::fs::write(&entry, format!("{SECRET}\n")).unwrap();
    let authinfo = home.home.join(".authinfo");
    std::fs::write(&authinfo, format!("machine example login me password {SECRET}\n")).unwrap();
    home.build();
    let after = |name: &str, form: String| Entry { kind: Kind::After, name: name.into(), file: None, noerror: true, load_path: 0, declared: true, forms: vec![form] };
    let entries = vec![
        Entry { kind: Kind::Require, name: "layer-lib".into(), file: None, noerror: false, load_path: 0, declared: false, forms: Vec::new() },
        after("layer-cfg", "(t progn (define-key layer-cfg-map \"a\" 'ignore) (add-hook 'layer-cfg-hook 'ignore))".into()),
        Entry { kind: Kind::Autoload, name: "layer-cmd".into(), file: None, noerror: true, load_path: 0, declared: true, forms: Vec::new() },
        after("layer-secret", "(t progn (require 'auth-source-pass) (setq layer-secret-value (auth-source-pass-get 'secret \"xmpp/me\")))".into()),
        after("layer-reader", format!("(t setq layer-reader-value (with-temp-buffer (insert-file-contents {:?}) (buffer-string)))", entry.display().to_string())),
        // A `with-eval-after-load' closure, and a form withheld after a secret reader ran.
        after("layer-closure", "(t . #<interpreted-function>)".into()),
        after("layer-withheld", "#<after a secret reader>".into()),
        // A configuration form that starts a timer: kept as code, the
        // forms beside it captured.
        after("layer-timer", "(t progn (setq layer-timer-set 'built) (run-with-timer 60 nil #'ignore))".into()),
        // A secret asked in an `if''s test: the whole `if' is kept.
        after("layer-if", "(t if (auth-source-pass-get 'secret \"xmpp/me\") (setq layer-if-value 'then) (setq layer-if-value 'else))".into()),
        // A function given to `eval-after-load' is called, as it would be.
        after("layer-fn", "(t lambda nil (error \"romacs-fixture: the function was called\"))".into()),
        // A function's symbol, as `#'layer-sym-config' gives it, is called too.
        after("layer-sym", "(t . layer-sym-config)".into()),
        // A plaintext credential file read without any reader.
        after("layer-netrc", format!("(t setq layer-netrc-value (with-temp-buffer (insert-file-contents {:?}) (buffer-string)))", authinfo.display().to_string())),
    ];
    // As an init's `load-path' is: its own directory before the installation's.
    let gnu_lisp = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../lisp/gnu").canonicalize().unwrap();
    let mut load_path = vec![lisp.display().to_string()];
    let mut dirs = vec![gnu_lisp];
    while let Some(dir) = dirs.pop() {
        load_path.push(dir.display().to_string());
        let mut subdirs: Vec<PathBuf> = std::fs::read_dir(&dir).unwrap().map(|e| e.unwrap().path()).filter(|p| p.is_dir()).collect();
        subdirs.sort();
        dirs.extend(subdirs.into_iter().rev());
    }
    let plan = Plan::new(&home.init, entries, vec![load_path]).unwrap();
    plan.write(&home.store).unwrap();
    let said = home.build();
    assert!(said.contains(r#""event":"layer-prepared","ok":true"#), "{said}");
    // The refusals name the reasons; the layer built holds the other three.
    let manifests: Vec<String> = std::fs::read_dir(&home.store).unwrap().filter_map(|e| {
        let path = e.unwrap().path();
        (path.extension().is_some_and(|x| x == "json")).then(|| std::fs::read_to_string(path).unwrap())
    }).collect();
    let why: Vec<(String, String)> = manifests.iter().filter_map(|m| serde_json::from_str::<serde_json::Value>(m).ok())
        .flat_map(|m| m["refusals"].as_array().cloned().unwrap_or_default())
        .filter(|r| r["kind"] == "init-layer")
        .map(|r| (r["name"].as_str().unwrap().to_owned(), r["why"].as_str().unwrap().to_owned())).collect();
    assert!(why.iter().any(|(name, why)| name == "layer-fn" && why.contains("the function was called")), "{why:?}");
    assert!(why.iter().any(|(name, why)| name == "layer-sym" && why.contains("the symbol function was called")), "{why:?}");
    for name in ["layer-secret", "layer-reader", "layer-netrc", "layer-timer", "layer-if"] {
        assert!(!why.iter().any(|(n, _)| n == name), "{name} was left out: {why:?}");
    }
    for name in ["layer-closure", "layer-withheld"] {
        assert!(why.iter().any(|(n, why)| n == name && why.contains("a form a layer cannot replay")), "{why:?}");
    }
    let built: Vec<serde_json::Value> = manifests.iter().filter_map(|m| serde_json::from_str::<serde_json::Value>(m).ok())
        .filter(|m| m["events"]["init-layer"].is_object() && m["refusals"].as_array().is_none_or(|r| r.is_empty()))
        .map(|m| m["events"]["init-layer"]["entries"].clone()).collect();
    let names: Vec<Vec<&str>> = built.iter().map(|entries| entries.as_array().unwrap().iter().map(|e| e["name"].as_str().unwrap()).collect()).collect();
    assert!(names.contains(&vec!["layer-lib", "layer-cfg", "layer-cmd", "layer-secret", "layer-reader", "layer-timer", "layer-if", "layer-netrc"]), "{names:?}");
    // No file the store holds contains the secret: images, manifests, plans.
    let mut files = vec![home.home.join("cache")];
    while let Some(path) = files.pop() {
        if path.is_dir() {
            files.extend(std::fs::read_dir(&path).unwrap().map(|e| e.unwrap().path()));
        } else if let Ok(bytes) = std::fs::read(&path) {
            assert!(!bytes.windows(SECRET.len()).any(|w| w == SECRET.as_bytes()), "{} holds the secret", path.display());
        }
    }
    let _ = std::fs::remove_dir_all(&home.home);
}
