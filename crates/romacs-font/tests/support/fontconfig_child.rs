//! A test that owns Fontconfig runs again in a child process whose
//! environment holds `FONTCONFIG_FILE` from its start: Fontconfig reads it
//! once, when it starts, and no thread of the test process sees its own
//! environment change.

use std::path::{Path, PathBuf};
use std::process::Command;

/// Names, in the child's environment, the root its parent prepared.
const ROOT: &str = "ROMACS_FONT_TEST_ROOT";

/// In the child, the root its parent prepared; None in the parent.
pub fn root() -> Option<PathBuf> {
    std::env::var_os(ROOT).map(PathBuf::from)
}

/// Run TEST, this executable's test of that exact name, again in a child
/// whose Fontconfig reads CONFIG and whose `root` is ROOT. Panics unless
/// the child ran that one test and it passed.
pub fn run(test: &str, root: &Path, config: &Path) {
    let output = Command::new(std::env::current_exe().unwrap())
        .args(["--exact", test, "--nocapture"])
        .env("FONTCONFIG_FILE", config)
        .env(ROOT, root)
        .output()
        .unwrap();
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(output.status.success() && stdout.contains("test result: ok. 1 passed"),
            "the child running {test}: {stdout}{}", String::from_utf8_lossy(&output.stderr));
}
