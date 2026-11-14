//! A private user for an in-process loadup: the session reads its HOME and
//! XDG directories (inits, history, agent journals) and its runtime directory
//! (the terminal keeper, the server) through `process-environment`, which
//! would otherwise be the developer's own, or a HOME every test of a check
//! slot shares.
use std::os::unix::fs::PermissionsExt;
use std::path::PathBuf;

/// Make `$TMPDIR/romacs-home-PID-THREAD-NAME` afresh and point VM's HOME,
/// XDG directories and runtime directory (mode 0700) under it, ahead of
/// any others a `getenv` would find. Returns that HOME. The thread keeps
/// tests that run side by side in one process apart, whatever NAME they
/// give; one thread runs one test at a time.
pub fn private_home(vm: &mut romacs_vm::Vm, name: &str) -> PathBuf {
    let thread = format!("{:?}", std::thread::current().id()).chars().filter(char::is_ascii_digit).collect::<String>();
    let home = std::env::temp_dir().join(format!("romacs-home-{}-{thread}-{name}", std::process::id()));
    let _ = std::fs::remove_dir_all(&home);
    let entries = [("HOME", home.clone()), ("XDG_CONFIG_HOME", home.join("config")), ("XDG_CACHE_HOME", home.join("cache")),
                   ("XDG_DATA_HOME", home.join("data")), ("XDG_STATE_HOME", home.join("state")), ("XDG_RUNTIME_DIR", home.join("run"))]
        .map(|(variable, path)| {
            std::fs::create_dir_all(&path).unwrap();
            serde_json::to_string(&format!("{variable}={}", path.display())).unwrap()
        });
    std::fs::set_permissions(home.join("run"), std::fs::Permissions::from_mode(0o700)).unwrap();
    vm.eval_source(&format!("(setq process-environment (append (list {}) process-environment))", entries.join(" ")))
        .unwrap();
    home
}
