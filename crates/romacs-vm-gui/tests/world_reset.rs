//! `--world-reset`: the store's failure records forgotten, or with `=all`
//! the store itself, before any display is needed.

/// A store in a private cache, with a world's files, a failure record and
/// a refused manifest of another world, and a file no world owns.
fn store(name: &str) -> (std::path::PathBuf, std::path::PathBuf) {
    let home = std::env::temp_dir().join(format!("romacs-world-reset-{}-{name}", std::process::id()));
    let _ = std::fs::remove_dir_all(&home);
    let store = home.join("cache/romacs/worlds");
    std::fs::create_dir_all(store.join("ids")).unwrap();
    let (kept, refused) = ("a".repeat(64), "b".repeat(64));
    for suffix in ["rwi", "lock", "log"] {
        std::fs::write(store.join(format!("{kept}.{suffix}")), "x").unwrap();
    }
    std::fs::write(store.join(format!("{kept}.json")), r#"{"refusals": []}"#).unwrap();
    std::fs::write(store.join(format!("{kept}.failed")), r#"{"attempts": 2, "at": 0}"#).unwrap();
    std::fs::write(store.join(format!("{refused}.json")), r#"{"refusals": [{"kind": "capture"}]}"#).unwrap();
    std::fs::write(store.join("ids/1-2-3"), "00").unwrap();
    std::fs::write(store.join("notes.txt"), "mine").unwrap();
    (home, store)
}

fn reset(home: &std::path::Path, arg: &str) -> String {
    let output = std::process::Command::new(env!("CARGO_BIN_EXE_romacs-vm-gui"))
        .arg(arg).env("HOME", home).env("XDG_CACHE_HOME", home.join("cache"))
        .env("WAYLAND_DISPLAY", home.join("no-such-socket")).output().unwrap();
    assert!(output.status.success(), "{output:?}");
    String::from_utf8(output.stderr).unwrap()
}

fn names(store: &std::path::Path) -> Vec<String> {
    let mut names: Vec<String> = std::fs::read_dir(store).unwrap().map(|e| e.unwrap().file_name().into_string().unwrap()).collect();
    names.sort();
    names
}

#[test]
fn world_reset_forgets_failures_and_refusals_and_keeps_worlds() {
    let (home, store) = store("records");
    let said = reset(&home, "--world-reset");
    assert!(said.contains(r#""event":"reset""#) && said.contains(r#""removed":2"#), "{said}");
    let kept = "a".repeat(64);
    assert_eq!(names(&store), [format!("{kept}.json"), format!("{kept}.lock"), format!("{kept}.log"),
                               format!("{kept}.rwi"), "ids".into(), "notes.txt".into()]);
    let _ = std::fs::remove_dir_all(&home);
}

#[test]
fn world_reset_all_removes_the_stores_own_files_only() {
    let (home, store) = store("all");
    reset(&home, "--world-reset=all");
    assert_eq!(names(&store), ["notes.txt"]);
    let _ = std::fs::remove_dir_all(&home);
}

#[test]
fn help_names_the_world_options() {
    let output = std::process::Command::new(env!("CARGO_BIN_EXE_romacs-vm-gui")).arg("--help").output().unwrap();
    let text = String::from_utf8(output.stdout).unwrap();
    for option in ["--world-store", "--world-reset", "--world-reset=all", "ROMACS_WORLD=off"] {
        assert!(text.contains(option), "{option} in {text}");
    }
}
