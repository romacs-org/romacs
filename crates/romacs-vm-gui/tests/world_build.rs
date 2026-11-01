//! `--world-build`: the world a live `--loadup` launch would boot from, and
//! a `--batch` session's (src/batch.rs), made usable with no window, so
//! the user's first launch and first batch session restore; a second call
//! finds them and builds nothing.

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
    let _ = std::fs::remove_dir_all(&home);
}
