//! Executable scripts for tests that run them while other tests run in
//! parallel.  Exec of a file some process holds open for writing fails
//! with `ETXTBSY`, and a sibling test that forks while this process writes a
//! script keeps a copy of the writing descriptor until its child execs:
//! closing it here, `O_CLOEXEC` or a rename (the same inode) leaves that
//! window open.  `install` writes the executable in a process of its own,
//! so no descriptor of this process ever has it open for writing.

use std::path::Path;

/// Writes TEXT to PATH as a script with mode 0755, replacing any file there.
pub fn write_script(path: &Path, text: &[u8]) {
    let source = path.with_extension("script-source");
    std::fs::write(&source, text).expect("write script source");
    let status = std::process::Command::new("install")
        .args(["-m", "0755"])
        .arg(&source)
        .arg(path)
        .status()
        .expect("run install");
    assert!(status.success(), "install {} failed: {status}", path.display());
    std::fs::remove_file(&source).expect("remove script source");
}
