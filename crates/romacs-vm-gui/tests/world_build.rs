//! `--world-build`: the world a live `--loadup` launch would boot from, made
//! usable with no window, so the user's first launch restores; a second
//! call finds it and builds nothing.

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
    assert!(ok && said.contains(r#""mode":"prepared""#), "{said}");
    let store = home.join("cache/romacs/worlds");
    let worlds: Vec<_> = std::fs::read_dir(&store).unwrap().map(|e| e.unwrap().path())
        .filter(|p| p.extension().is_some_and(|x| x == "rwi")).collect();
    assert_eq!(worlds.len(), 1, "{worlds:?}");
    assert!(worlds[0].with_extension("json").is_file());
    let image = std::fs::metadata(&worlds[0]).unwrap().modified().unwrap();
    let (ok, said) = build(&home);
    assert!(ok && said.contains(r#""mode":"prepared""#), "{said}");
    assert_eq!(std::fs::metadata(&worlds[0]).unwrap().modified().unwrap(), image, "the world was built again");
    let _ = std::fs::remove_dir_all(&home);
}
