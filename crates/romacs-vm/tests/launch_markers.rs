//! The launch markers stay out of every child without the environment
//! changing (`romacs_vm::host::LAUNCH_MARKERS`). The host module, the one
//! place outside tests that could change it (tests/host_boundary_lint.rs
//! keeps the rest of the VM from `std::env::set_var` and `remove_var`),
//! calls neither: a change while another thread reads the environment is
//! undefined behaviour in the C library, and the VM's threads read it.
//! What a child inherits is held at run time by romacs-vm-gui's
//! tests/launch_markers.rs.
#[path = "support/own_test.rs"]
mod own_test;

use romacs_vm::host::{self, LAUNCH_MARKERS};

#[test]
fn the_host_never_changes_the_environment() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    let mut files = vec![root.join("host.rs")];
    files.extend(std::fs::read_dir(root.join("host")).unwrap().map(|entry| entry.unwrap().path()));
    for file in files {
        let source = std::fs::read_to_string(&file).unwrap();
        for call in ["set_var(", "remove_var(", "setenv(", "unsetenv(", "putenv(", "clearenv("] {
            assert!(!source.contains(call), "{} calls {call}: the environment is never changed", file.display());
        }
    }
}

#[test]
fn a_child_command_inherits_no_launch_marker() {
    let command = host::child("true");
    let removed: Vec<_> = command.get_envs().filter(|(_, value)| value.is_none()).map(|(name, _)| name.to_owned()).collect();
    for marker in LAUNCH_MARKERS {
        assert!(removed.iter().any(|name| name == marker), "{marker} reaches a child: {removed:?}");
    }
}

#[test]
fn lisp_names_the_launch_markers_the_host_does() {
    let mut vm = romacs_vm::Vm::new();
    let names = vm.eval_source("(romacs--launch-markers)").unwrap().display().unwrap();
    let expected = format!("({})", LAUNCH_MARKERS.iter().map(|m| format!("\"{m}\"")).collect::<Vec<_>>().join(" "));
    assert_eq!(names, expected);
}

/// Set in this test binary run again with every marker set.
const IN_CHILD: &str = "ROMACS_MARKER_COMMAND_CHILD";

/// A process acts on each marker it was started with once, and a child of
/// `host::command` that inherits its environment, as an isolate worker
/// does, holds none of them.
#[test]
fn an_inheriting_child_holds_no_launch_marker() {
    if std::env::var_os(IN_CHILD).is_none() {
        let test = own_test::name(module_path!(), "an_inheriting_child_holds_no_launch_marker");
        let mut rerun = std::process::Command::new(std::env::current_exe().unwrap());
        for marker in LAUNCH_MARKERS {
            rerun.env(marker, "set");
        }
        let output = rerun.args(["--exact", &test, "--nocapture", "--test-threads", "1"]).env(IN_CHILD, "1").output().unwrap();
        let stdout = String::from_utf8_lossy(&output.stdout);
        assert!(output.status.success(), "{stdout}{}", String::from_utf8_lossy(&output.stderr));
        // A name that selected nothing would pass with no test run.
        assert!(stdout.contains("test result: ok. 1 passed"), "the child ran no test: {stdout}");
        return;
    }
    // Each marker is there to take, once; the host's lookups, which a
    // world's environment facts record, never see one.
    for marker in LAUNCH_MARKERS {
        assert!(host::getenv(marker).is_none(), "a host lookup sees {marker}");
        assert!(host::has_marker(marker), "{marker} was set");
        assert_eq!(host::take_marker(marker).as_deref(), Some(std::ffi::OsStr::new("set")));
        assert!(!host::has_marker(marker) && host::take_marker(marker).is_none(), "{marker} taken twice");
    }
    let output = host::command("/usr/bin/env").unwrap().output().unwrap();
    let env = String::from_utf8(output.stdout).unwrap();
    assert!(env.lines().any(|line| line.starts_with(&format!("{IN_CHILD}="))), "the child's environment was not read: {env}");
    for marker in LAUNCH_MARKERS {
        assert!(!env.lines().any(|line| line.starts_with(&format!("{marker}="))), "{marker} reached the child: {env}");
    }
}
