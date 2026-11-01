//! A resize before any key reaches the command loop still runs
//! `window-size-change-functions', once, from the repaint's redisplay: the
//! session installs window.c's change functions (rlisp/vm/window-change.el)
//! itself rather than waiting for the command loop's first key.
#[path = "../src/loadup.rs"] mod loadup;
#[path = "../src/eval_last_sexp.rs"] mod eval_last_sexp;
#[path = "../src/startup.rs"] mod startup;
#[path = "../src/command.rs"] mod command;
#[path = "../src/presentation.rs"] mod presentation;
#[path = "../src/native.rs"] mod native;
#[path = "../src/echo.rs"] mod echo;
use romacs_vm::Vm;

#[allow(dead_code)]
fn snapshot(vm: &mut Vm, _: bool, width: u32, height: u32) -> Result<romacs_layout_input::FrameSnapshot, String> {
    vm.frame_snapshot(width, height)
}

#[test]
fn a_resize_before_the_first_key_runs_the_size_functions_once() {
    let calls = std::thread::Builder::new().stack_size(64 << 20).spawn(|| {
        let mut vm = Vm::new();
        vm.set_frame_geometry(1200, 800, 20, 10).unwrap();
        let started = startup::initialize_loadup(&mut vm, startup::Launch::Loadup, None).unwrap();
        startup::finish_loadup(&mut vm, &started).unwrap();
        vm.frame_snapshot(1200, 800).unwrap();
        vm.eval_source("(progn (defvar resize-log nil) \
                        (add-hook 'window-size-change-functions (lambda (f) (push (framep f) resize-log))))").unwrap();
        vm.set_frame_geometry(1000, 600, 20, 10).unwrap();
        vm.frame_snapshot(1000, 600).unwrap();
        vm.frame_snapshot(1000, 600).unwrap();
        vm.eval_source("(prin1-to-string resize-log)").unwrap().display().unwrap()
    }).unwrap().join().unwrap();
    assert_eq!(calls, "\"(t)\"");
}
