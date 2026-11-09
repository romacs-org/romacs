//! A batch process with no world to restore, as the ERT runner starts them, is
//! no builder: nothing it allocates becomes an image, so it times its collections
//! (`arena::heap_is_image`) as a session does.
use std::process::Command;

#[test]
fn a_process_booted_from_source_times_its_collections() {
    let home = std::env::temp_dir().join(format!("romacs-batch-from-source-{}", std::process::id()));
    std::fs::create_dir_all(&home).unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_romacs-vm-gui"))
        .args(["-Q", "--batch", "--eval", "(progn (garbage-collect) (princ (list (> gc-elapsed 0.0))))"])
        .env_clear().env("PATH", "/usr/bin:/bin").env("HOME", &home)
        .env("XDG_CONFIG_HOME", home.join("config")).env("XDG_CACHE_HOME", home.join("cache"))
        .env("TMPDIR", &home).env("LANG", "C.UTF-8").env("ROMACS_WORLD", "off")
        .envs(["ROMACS_RUNTIME_ROOT", "ROMACS_LISP_DIR", "LD_LIBRARY_PATH"].iter()
              .filter_map(|name| Some((name, std::env::var_os(name)?))))
        .output().unwrap();
    std::fs::remove_dir_all(&home).unwrap();
    assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
    assert_eq!(String::from_utf8_lossy(&output.stdout), "(t)");
}
