//! Process isolation for the compile store and generated O7m inputs.
#[path = "gnu_lisp.rs"]
mod gnu_lisp;
#[path = "own_test.rs"]
mod own_test;

pub use gnu_lisp::gnu_lisp;

use romacs_vm::{Vm, compiled_lisp};
use std::{fs, path::Path, process::Command};

macro_rules! in_private_store {
    ($test:ident, $run:expr) => {
        self::support::in_private_store(module_path!(), stringify!($test), $run)
    };
}

pub fn in_private_store(module: &str, test: &str, run: impl FnOnce(&Path)) {
    const MARKER: &str = "ROMACS_O7M_CODEC_CHILD";
    if let Some(dir) = std::env::var_os(MARKER) {
        run(Path::new(&dir));
        return;
    }
    let marker = std::env::temp_dir().join("romacs-o7m-codec-XXXXXX");
    let (dir, file) = romacs_vm::host::temp_file(&marker).unwrap();
    drop(file);
    fs::remove_file(&dir).unwrap();
    fs::create_dir(&dir).unwrap();
    let output = Command::new(std::env::current_exe().unwrap())
        .args(["--exact", &own_test::name(module, test), "--nocapture"])
        .env(MARKER, &dir)
        .output()
        .unwrap();
    fs::remove_dir_all(dir).unwrap();
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("running 1 test"), "{stdout}");
    assert!(
        output.status.success(),
        "{stdout}\n{}",
        String::from_utf8_lossy(&output.stderr)
    );
}

pub fn configure_store(store: &Path) {
    compiled_lisp::configure(
        compiled_lisp::Config {
            compiler: vec![],
            environment: vec![],
            inherited: &[],
            defer_misses: false,
            store: store.to_owned(),
            identity: Box::new(|| Some("o7m-test-world".into())),
            runtime: vec![],
            root: None,
        },
        None,
    );
}

pub fn eval(vm: &mut Vm, form: &str) -> String {
    vm.eval_source(form).and_then(|v| v.display()).unwrap()
}
