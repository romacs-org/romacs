//! Tree-sitter modes fontify as pinned GNU's do, character by character:
//! a Rust, a Python, a C and a JavaScript file opened with
//! `treesit-enabled-modes' t and fontified at GNU's default
//! `treesit-font-lock-level' give the face runs pinned GNU gives with the
//! same grammars (fixtures/treesit-faces/*.faces), with the grammars this
//! executable ships (DIVERGENCES.md VM2-TREESIT-BUNDLED).
//!
//! The expected runs are pinned GNU's: build the shipped grammars as
//! libraries with `tools/lib/treesit_grammars.py so --out DIR`, then for each
//! sample run pinned GNU as
//! `TREESIT_FACES_GRAMMARS=DIR TREESIT_FACES_FILE=sample.EXT emacs -Q --batch
//! --eval "$(cat faces.form)" > sample.EXT.faces` in that directory.
use std::path::{Path, PathBuf};
use std::process::Command;

const FORM: &str = include_str!("fixtures/treesit-faces/faces.form");

/// A HOME of this test's own, made exclusively and removed when dropped,
/// whether the test passes or fails.
struct Home(PathBuf);
impl Home {
    fn new() -> Self {
        let base = std::env::temp_dir();
        let nanos = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos();
        for attempt in 0.. {
            let dir = base.join(format!("romacs-treesit-faces-{}-{nanos}-{attempt}", std::process::id()));
            match std::fs::create_dir(&dir) {
                Ok(()) => return Home(dir),
                Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
                Err(error) => panic!("{}: {error}", dir.display()),
            }
        }
        unreachable!()
    }
}
impl Drop for Home {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// The face runs this executable gives SAMPLE, as faces.form prints them.
/// Every per-user directory is HOME's, so no grammar directory or world
/// cache of the user running the test is read or written.
fn romacs_runs(sample: &Path, home: &Path) -> String {
    let output = Command::new(env!("CARGO_BIN_EXE_romacs-vm-gui"))
        .args(["-Q", "--batch", "--eval", FORM])
        .env("HOME", home)
        .env("XDG_CONFIG_HOME", home.join(".config"))
        .env("XDG_CACHE_HOME", home.join(".cache"))
        .env("XDG_DATA_HOME", home.join(".local/share"))
        .env("XDG_STATE_HOME", home.join(".local/state"))
        .env("TREESIT_FACES_FILE", sample)
        .env_remove("TREESIT_FACES_GRAMMARS")
        .output()
        .unwrap();
    assert!(output.status.success(), "{}: {}", sample.display(), String::from_utf8_lossy(&output.stderr));
    String::from_utf8(output.stdout).unwrap()
}

#[test]
fn ts_modes_fontify_each_character_as_pinned_gnu() {
    let fixtures = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/treesit-faces");
    let home = Home::new();
    let mut failures = Vec::new();
    for extension in ["rs", "py", "c", "js"] {
        let sample = fixtures.join(format!("sample.{extension}"));
        let expected = std::fs::read_to_string(fixtures.join(format!("sample.{extension}.faces"))).unwrap();
        let actual = romacs_runs(&sample, &home.0);
        if actual != expected {
            let first = actual.lines().zip(expected.lines()).position(|(a, e)| a != e)
                .unwrap_or_else(|| actual.lines().count().min(expected.lines().count()));
            failures.push(format!("sample.{extension}: run {first}: Romacs {:?}, pinned GNU {:?}",
                actual.lines().nth(first), expected.lines().nth(first)));
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}
