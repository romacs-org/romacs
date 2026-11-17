//! This build as an installation with its batch world, for tests whose
//! `--batch` sessions only need GNU's Lisp loaded: each restores that world
//! (batch.rs `installed_world`) instead of booting the chain from source,
//! about 0.5 G instructions a session rather than 17 G.
//!
//! The installation is laid out as tools/package-release lays one out, in
//! the target's test directory: the executable in `bin/` (a hard link to
//! this build's), the runtime tree in `share/romacs` (this checkout's,
//! linked) and `romacs-vm-gui.worlds` beside the executable. The first test
//! process to ask lays it out, under a lock every other one waits on, and
//! lays it out again for a rebuilt executable or another checkout. Each
//! test process then prepares the world (`--world-build --batch`), which
//! builds it once, under the world's own lock, and after that finds it
//! current, or builds it again when its Lisp changed. The build's own
//! executable keeps no world beside it, so the tests of a user's own store
//! see none.
use std::os::unix::fs::MetadataExt;
use std::path::{Path, PathBuf};

/// The installed executable, its batch world prepared.
pub fn romacs() -> &'static str {
    static INSTALLED: std::sync::OnceLock<String> = std::sync::OnceLock::new();
    INSTALLED.get_or_init(|| install().into_os_string().into_string().unwrap())
}

fn install() -> PathBuf {
    let root = Path::new(env!("CARGO_TARGET_TMPDIR")).join("installed");
    let exe = lay_out(&root);
    let home = root.join("home");
    let output = std::process::Command::new(&exe)
        .arg("--world-store").arg(exe.with_file_name("romacs-vm-gui.worlds")).args(["--world-build", "--batch"])
        .current_dir(&home).env("HOME", &home).env("XDG_CONFIG_HOME", home.join("config"))
        .env("XDG_CACHE_HOME", home.join("cache")).env("XDG_DATA_HOME", home.join("data"))
        .env("XDG_STATE_HOME", home.join("state")).env_remove("ROMACS_WORLD")
        .env_remove("WAYLAND_DISPLAY").env_remove("DISPLAY")
        .output().unwrap();
    assert!(output.status.success(), "the test installation's batch world: {}", String::from_utf8_lossy(&output.stderr));
    exe
}

/// The installation in ROOT for this build and checkout, laid out under
/// ROOT's lock: its executable.
fn lay_out(root: &Path) -> PathBuf {
    std::fs::create_dir_all(root.join("home")).unwrap();
    let lock = std::fs::File::create(root.join("lock")).unwrap();
    lock.lock().unwrap();
    let source = Path::new(env!("CARGO_BIN_EXE_romacs-vm-gui"));
    let (bin, exe, linked) = (root.join("bin"), root.join("bin/romacs-vm-gui"), root.join("linked"));
    let identity = identity(source);
    if !exe.is_file() || std::fs::read_to_string(&linked).ok().as_deref() != Some(identity.as_str()) {
        let _ = std::fs::remove_dir_all(&bin);
        std::fs::create_dir_all(&bin).unwrap();
        std::fs::hard_link(source, &exe).or_else(|_| std::fs::copy(source, &exe).map(drop)).unwrap();
        std::fs::write(&linked, &identity).unwrap();
    }
    let (share, tree) = (root.join("share/romacs"), Path::new(env!("CARGO_MANIFEST_DIR")).join("../.."));
    if std::fs::read_link(&share).ok().as_deref() != Some(tree.as_path()) {
        let _ = std::fs::remove_file(&share);
        std::fs::create_dir_all(root.join("share")).unwrap();
        std::os::unix::fs::symlink(&tree, &share).unwrap();
    }
    exe
}

/// What tells this build's executable from a rebuilt one.
fn identity(exe: &Path) -> String {
    let meta = std::fs::metadata(exe).unwrap();
    format!("{} {} {} {}.{}", meta.dev(), meta.ino(), meta.len(), meta.mtime(), meta.mtime_nsec())
}
