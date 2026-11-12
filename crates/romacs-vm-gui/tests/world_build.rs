//! `--world-build`: the worlds a live `--loadup` launch and a `--batch`
//! session would boot from, made usable with no window, so the user's first
//! launch and first batch session restore; a second call finds them and
//! builds nothing. Before `--batch`, it prepares the batch world alone, as
//! an installer does beside the executable (src/batch.rs). The batch world
//! is the GUI's companion: one the installation serves is not built again,
//! and one that cannot be built is a warning, not a failed build.

fn build(home: &std::path::Path) -> (bool, String) {
    let output = std::process::Command::new(env!("CARGO_BIN_EXE_romacs-vm-gui"))
        .args(["--loadup", "--world-build"]).env("HOME", home).env("XDG_CACHE_HOME", home.join("cache"))
        .env_remove("WAYLAND_DISPLAY").env_remove("DISPLAY").output().unwrap();
    (output.status.success(), String::from_utf8_lossy(&output.stderr).into_owned())
}

#[test]
fn world_build_prepares_the_store_once_and_needs_no_display() {
    let home = std::env::temp_dir().join(format!("romacs-world-build-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&home);
    std::fs::create_dir_all(&home).unwrap();
    let (ok, said) = build(&home);
    assert!(ok && said.contains(r#""mode":"prepared""#) && said.contains(r#""launch":"batch""#), "{said}");
    let store = home.join("cache/romacs/worlds");
    let worlds: Vec<_> = std::fs::read_dir(&store).unwrap().map(|e| e.unwrap().path())
        .filter(|p| p.extension().is_some_and(|x| x == "rwi")).collect();
    assert_eq!(worlds.len(), 2, "{worlds:?}");
    assert!(worlds.iter().all(|w| w.with_extension("json").is_file()));
    let images: Vec<_> = worlds.iter().map(|w| std::fs::metadata(w).unwrap().modified().unwrap()).collect();
    let (ok, said) = build(&home);
    assert!(ok && said.contains(r#""mode":"prepared""#), "{said}");
    let again: Vec<_> = worlds.iter().map(|w| std::fs::metadata(w).unwrap().modified().unwrap()).collect();
    assert_eq!(again, images, "a world was built again");
    // The batch world gone and its last build failed just now, so none is
    // due yet (`World::retry_in`): the GUI's world stands, the batch world
    // is a warning, the layer step still runs.
    let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_secs();
    for manifest in worlds.iter().map(|w| w.with_extension("json")) {
        if std::fs::read_to_string(&manifest).unwrap().contains("launch batch") {
            std::fs::remove_file(manifest.with_extension("rwi")).unwrap();
            std::fs::remove_file(&manifest).unwrap();
            std::fs::write(manifest.with_extension("failed"), format!(r#"{{"kind":"transient","error":"test","attempts":1,"at":{now}}}"#)).unwrap();
        }
    }
    let (ok, said) = build(&home);
    assert!(ok && said.contains(r#""event":"batch-world","ok":false"#) && said.contains("layer-prepared"), "{said}");
    let _ = std::fs::remove_dir_all(&home);
}

/// A `--batch' session's world holds the stock theme the chain enables
/// (lisp/romacs/romacs-theme.el `romacs-theme-start'): the session that prepares
/// it and one restored from it both start in `default'. The first says why
/// it booted from source under ROMACS_DIAGNOSTICS=1; the second says nothing.
#[test]
fn a_batch_world_starts_in_the_stock_theme() {
    use std::os::unix::fs::PermissionsExt;
    let home = std::env::temp_dir().join(format!("romacs-world-theme-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&home);
    let store = home.join("worlds");
    std::fs::create_dir_all(&store).unwrap();
    std::fs::set_permissions(&store, std::fs::Permissions::from_mode(0o700)).unwrap();
    let session = |diagnostics: &str| {
        let output = std::process::Command::new(env!("CARGO_BIN_EXE_romacs-vm-gui"))
            .arg("--world-store").arg(&store)
            .args(["--batch", "--eval", "(princ (format \"THEMES %S\" custom-enabled-themes))"])
            .env("HOME", &home).env("XDG_CACHE_HOME", home.join("cache")).env("ROMACS_DIAGNOSTICS", diagnostics)
            .env_remove("WAYLAND_DISPLAY").env_remove("DISPLAY").output().unwrap();
        (String::from_utf8_lossy(&output.stdout).into_owned(), String::from_utf8_lossy(&output.stderr).into_owned())
    };
    // Booting from source while the world is built says why, when asked.
    let (said, err) = session("1");
    assert!(said.contains("THEMES (default)") && err.contains(r#""mode":"source""#) && err.contains(r#""reason""#), "{said}\n{err}");
    let image = || std::fs::read_dir(&store).unwrap().filter_map(Result::ok).any(|e| e.path().extension().is_some_and(|x| x == "rwi"));
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(300);
    while !image() && std::time::Instant::now() < deadline { std::thread::sleep(std::time::Duration::from_millis(500)); }
    assert!(image(), "no batch world was built in {}", store.display());
    // Restored, and as quiet as GNU's batch by default.
    let (said, err) = session("");
    assert!(said.contains("THEMES (default)") && !err.contains("romacs-world"), "{said}\n{err}");
    let _ = std::fs::remove_dir_all(&home);
}

/// What a batch session answers about its HOME, user, locale and the
/// variables startup computes from them.
const STARTUP_STATE: &str = "(prin1 (list user-emacs-directory current-language-environment locale-coding-system
  (user-login-name) (getenv \"HOME\") project-list-file
  (mapcar (lambda (v) (cons v (and (boundp v) (symbol-value v)))) romacs-loadup-delayed-init-variables)))";

/// Every file under DIR, with its length and modification time.
fn files(dir: &std::path::Path) -> Vec<(std::path::PathBuf, u64, std::time::SystemTime)> {
    let mut found = Vec::new();
    for entry in std::fs::read_dir(dir).unwrap().map(Result::unwrap) {
        let meta = entry.metadata().unwrap();
        if meta.is_dir() { found.extend(files(&entry.path())) } else { found.push((entry.path(), meta.len(), meta.modified().unwrap())) }
    }
    found.sort();
    found
}

/// An installation's batch world (`--world-build --batch` into
/// `EXECUTABLE.worlds`), built under one HOME, user directory and locale,
/// serves a session under another as a boot from source there answers:
/// it holds no record of the builder's HOME, XDG directories, user name or
/// locale, the session only reads its store, and makes none of its own.
#[test]
fn an_installed_batch_world_serves_any_home() {
    let root = std::path::Path::new(env!("CARGO_TARGET_TMPDIR")).join(format!("installed-world-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    let (bin, a, b) = (root.join("bin"), root.join("home-a"), root.join("home-b"));
    for dir in [&bin, &a, &b] { std::fs::create_dir_all(dir).unwrap(); }
    // An installation: the executable in a directory of its own.
    let exe = bin.join("romacs-vm-gui");
    std::fs::hard_link(env!("CARGO_BIN_EXE_romacs-vm-gui"), &exe)
        .or_else(|_| std::fs::copy(env!("CARGO_BIN_EXE_romacs-vm-gui"), &exe).map(drop)).unwrap();
    let installed = bin.join("romacs-vm-gui.worlds");
    let command = |home: &std::path::Path, lang: &str| {
        let mut command = std::process::Command::new(&exe);
        // A world binds the login name and SHELL its chain reads (VM2-BATCH2):
        // both homes give the same, so the test does not pass by inheritance.
        command.current_dir(home).env("HOME", home).env("LANG", lang).env("XDG_CACHE_HOME", home.join("cache"))
            .env("USER", "builder").env("LOGNAME", "builder").env("NAME", "A Builder").env("SHELL", "/bin/sh")
            .env_remove("XDG_CONFIG_HOME").env_remove("LC_ALL").env_remove("LC_CTYPE")
            .env_remove("LC_MESSAGES").env_remove("LANGUAGE").env_remove("ROMACS_WORLD")
            .env_remove("WAYLAND_DISPLAY").env_remove("DISPLAY");
        command
    };
    let output = command(&a, "en_US.UTF-8").env("XDG_CONFIG_HOME", a.join("config"))
        .arg("--world-store").arg(&installed).args(["--world-build", "--batch"]).output().unwrap();
    let said = String::from_utf8_lossy(&output.stderr);
    assert!(output.status.success() && said.contains(r#""launch":"batch""#), "{said}");
    let manifests: Vec<_> = files(&installed).into_iter().map(|(path, ..)| path)
        .filter(|path| path.extension().is_some_and(|x| x == "json")).collect();
    assert_eq!(manifests.len(), 1, "{manifests:?}");
    let manifest: serde_json::Value = serde_json::from_slice(&std::fs::read(&manifests[0]).unwrap()).unwrap();
    let personal = |name: &str| ["HOME", "USER", "LANG", "LANGUAGE"].contains(&name) || name.starts_with("LC_") || name.starts_with("XDG_");
    for record in manifest["records"].as_array().unwrap() {
        let text = record.to_string();
        assert!(!text.contains(a.to_str().unwrap()), "a record names the builder's HOME: {text}");
        assert!(record["kind"] != "env" || !record["name"].as_str().is_some_and(personal), "a record binds the builder's environment: {text}");
    }
    let before = files(&installed);
    let session = |world: bool| {
        let mut command = command(&b, "C.UTF-8");
        command.env("ROMACS_DIAGNOSTICS", "1").args(["-Q", "--batch", "--eval", STARTUP_STATE]);
        if !world { command.env("ROMACS_WORLD", "off"); }
        let output = command.output().unwrap();
        assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
        (String::from_utf8_lossy(&output.stdout).into_owned(), String::from_utf8_lossy(&output.stderr).into_owned())
    };
    let (restored, said) = session(true);
    assert!(said.contains(r#""installed":true"#), "the installed world was not restored: {said}");
    let (source, _) = session(false);
    assert!(restored.contains(b.to_str().unwrap()), "{restored}");
    assert_eq!(restored, source);
    assert_eq!(files(&installed), before, "the session wrote in the installation's store");
    let made = b.join("cache/romacs/worlds");
    assert!(!made.exists(), "the session made a store: {:?}", files(&b.join("cache")));
    // `--world-build` for this executable builds the GUI's world and finds
    // the installation's batch world: no second one.
    let c = root.join("home-c");
    std::fs::create_dir_all(&c).unwrap();
    let output = command(&c, "C.UTF-8").args(["--loadup", "--world-build"]).output().unwrap();
    let said = String::from_utf8_lossy(&output.stderr);
    assert!(output.status.success() && said.contains(r#""skipped""#) && !said.contains(r#""launch":"batch""#), "{said}");
    let built: Vec<_> = files(&c.join("cache/romacs/worlds")).into_iter().filter(|(path, ..)| path.extension().is_some_and(|x| x == "rwi")).collect();
    assert_eq!(built.len(), 1, "{built:?}");
    let _ = std::fs::remove_dir_all(&root);
}
