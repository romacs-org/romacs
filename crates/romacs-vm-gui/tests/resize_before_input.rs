//! A resize before any key reaches the command loop still runs
//! `window-size-change-functions', once, from the repaint's redisplay: the
//! session installs window.c's change functions (rlisp/vm/window-change.el)
//! itself rather than waiting for the command loop's first key.
use romacs_vm_gui::startup;
use romacs_vm::Vm;

fn size_calls_after_an_early_resize(start: fn(&mut Vm)) -> String {
    std::thread::Builder::new().stack_size(64 << 20).spawn(move || {
        let mut vm = Vm::new();
        vm.set_frame_geometry(1200, 800, 20, 10).unwrap();
        start(&mut vm);
        vm.frame_snapshot(1200, 800).unwrap();
        vm.eval_source("(progn (defvar resize-log nil) \
                        (set-default 'window-size-change-functions (list (lambda (f) (setq resize-log (cons (framep f) resize-log))))))").unwrap();
        vm.set_frame_geometry(1000, 600, 20, 10).unwrap();
        vm.frame_snapshot(1000, 600).unwrap();
        vm.frame_snapshot(1000, 600).unwrap();
        vm.eval_source("(prin1-to-string resize-log)").unwrap().display().unwrap()
    }).unwrap().join().unwrap()
}

#[test]
fn a_resize_before_the_first_key_runs_the_size_functions_once() {
    let loadup = size_calls_after_an_early_resize(|vm| {
        let started = startup::initialize_loadup(vm, startup::Launch::Loadup, None).unwrap();
        startup::finish_loadup(vm, &started).unwrap();
    });
    assert_eq!(loadup, "\"(t)\"", "--loadup");
    let gnu_only = size_calls_after_an_early_resize(|vm| {
        startup::initialize_loadup(vm, startup::Launch::GnuOnly, None).unwrap();
    });
    assert_eq!(gnu_only, "\"(t)\"", "--gnu-lisp");
    let workspace = size_calls_after_an_early_resize(|vm| { startup::initialize(vm).unwrap(); });
    assert_eq!(workspace, "\"(t)\"", "--no-gnu-lisp");
}
