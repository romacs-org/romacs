//! Attachments' Lisp (lisp/romacs/romacs-attach.el, attachments.md stage A1) in a
//! `--loadup' session with a window system: items sniffed without decoding,
//! plans, the pure list operations, the tray's cached string and batch
//! insertion, edit mode, drafts reconciliation and preparation through the
//! helper (tests/romacs-attach-test.el).
use romacs_vm_gui::{fonts, startup};
use romacs_vm::Vm;

const LINE: u32 = 20;

fn printed(vm: &mut Vm, source: &str) -> String {
    vm.eval_source(source).and_then(|v| v.display()).unwrap_or_else(|e| format!("VM-ERROR {e}"))
}

/// The helper as the workspace built it beside this test, if it did.
fn helper() -> Option<std::path::PathBuf> {
    let deps = std::env::current_exe().ok()?.parent()?.to_path_buf();
    [deps.join("romacs-attach-tool"), deps.parent()?.join("romacs-attach-tool")].into_iter().find(|p| p.is_file())
}

#[test]
fn romacs_attach_lisp_tests_pass() {
    let summary = std::thread::Builder::new().stack_size(64 << 20).spawn(move || {
        let mut vm = Vm::new();
        vm.set_frame_geometry(1280, 800, LINE, 10).unwrap();
        vm.set_display_host(Some(Box::new(fonts::Display::new().unwrap())));
        startup::initialize_loadup(&mut vm, startup::Launch::Loadup, None).unwrap();
        assert!(vm.open_display());
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
        let tool = helper().map_or("nil".to_owned(), |p| format!("{:?}", p.display().to_string()));
        printed(&mut vm, &format!("(setq romacs-attach-test-helper {tool})"));
        for file in ["lisp/romacs/romacs-test.el", "crates/romacs-vm-gui/tests/romacs-attach-test.el"] {
            vm.load_file(&root.join(file)).unwrap_or_else(|e| panic!("{file}: {e}"));
        }
        printed(&mut vm, "(let ((s (romacs-test-run t))) (format \"%S\" s))")
    }).unwrap().join().unwrap();
    assert!(summary.contains(":failed 0") && summary.contains(":errors 0"), "{summary}");
}
