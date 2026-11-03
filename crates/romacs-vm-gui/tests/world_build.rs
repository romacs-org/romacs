//! `--world-build`: the world a live `--loadup` launch would boot from, made
//! usable with no window, so the user's first launch restores; a second
//! call finds it and builds nothing. A `--batch` session's world is not
//! built here: the first session in the store prepares it (src/batch.rs).

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
    assert!(ok && said.contains(r#""mode":"prepared""#) && !said.contains(r#""launch":"batch""#), "{said}");
    let store = home.join("cache/romacs/worlds");
    let worlds: Vec<_> = std::fs::read_dir(&store).unwrap().map(|e| e.unwrap().path())
        .filter(|p| p.extension().is_some_and(|x| x == "rwi")).collect();
    assert_eq!(worlds.len(), 1, "{worlds:?}");
    assert!(worlds.iter().all(|w| w.with_extension("json").is_file()));
    let images: Vec<_> = worlds.iter().map(|w| std::fs::metadata(w).unwrap().modified().unwrap()).collect();
    let (ok, said) = build(&home);
    assert!(ok && said.contains(r#""mode":"prepared""#), "{said}");
    let again: Vec<_> = worlds.iter().map(|w| std::fs::metadata(w).unwrap().modified().unwrap()).collect();
    assert_eq!(again, images, "a world was built again");
    let _ = std::fs::remove_dir_all(&home);
}

/// A `--batch' session's world holds the stock theme the chain enables
/// (rlisp/romacs-theme.el `romacs-theme-start'): the session that prepares
/// it and one restored from it both start in `default'.
#[test]
fn a_batch_world_starts_in_the_stock_theme() {
    use std::os::unix::fs::PermissionsExt;
    let home = std::env::temp_dir().join(format!("romacs-world-theme-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&home);
    let store = home.join("worlds");
    std::fs::create_dir_all(&store).unwrap();
    std::fs::set_permissions(&store, std::fs::Permissions::from_mode(0o700)).unwrap();
    let session = || {
        let output = std::process::Command::new(env!("CARGO_BIN_EXE_romacs-vm-gui"))
            .arg("--world-store").arg(&store)
            .args(["--batch", "--eval", "(princ (format \"THEMES %S\" custom-enabled-themes))"])
            .env("HOME", &home).env("XDG_CACHE_HOME", home.join("cache"))
            .env_remove("WAYLAND_DISPLAY").env_remove("DISPLAY").output().unwrap();
        (String::from_utf8_lossy(&output.stdout).into_owned(), String::from_utf8_lossy(&output.stderr).into_owned())
    };
    let (said, _) = session();
    assert!(said.contains("THEMES (default)"), "{said}");
    let image = || std::fs::read_dir(&store).unwrap().filter_map(Result::ok).any(|e| e.path().extension().is_some_and(|x| x == "rwi"));
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(300);
    while !image() && std::time::Instant::now() < deadline { std::thread::sleep(std::time::Duration::from_millis(500)); }
    assert!(image(), "no batch world was built in {}", store.display());
    let (said, err) = session();
    assert!(said.contains("THEMES (default)") && !err.contains(r#""mode":"source""#), "{said}\n{err}");
    let _ = std::fs::remove_dir_all(&home);
}
