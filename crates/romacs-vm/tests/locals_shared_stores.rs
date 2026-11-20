//! What a c-mode buffer's locals store into tables every buffer shares,
//! none: the declaration tables (`romacs_vm::declaration_stores`), counted over
//! one buffer's c-mode setup and over its kill, at steady state. GNU keeps
//! a buffer's locals in the buffer (`local_var_alist`), where neither
//! stores into anything but that buffer.
#[path = "support/gnu_lisp.rs"]
mod gnu_lisp;
use gnu_lisp::gnu_lisp;
use romacs_vm::{Vm, declaration_stores};

const TEXT: &str = r##""#include <stdio.h>\nint main (void) { return 0; }\n""##;

/// The declaration-table stores SOURCE makes in VM, and its value printed.
fn stores(vm: &mut Vm, source: &str) -> (u64, String) {
    let before = declaration_stores();
    let value = vm.eval_source(source).and_then(romacs_vm::Local::display).unwrap();
    (declaration_stores() - before, value)
}

#[test]
fn a_c_mode_buffers_locals_store_into_no_shared_table() {
    let (setup, kill, locals) = std::thread::Builder::new().stack_size(256 << 20).spawn(|| {
        let mut vm = gnu_lisp(&[]);
        // Warm: cc-mode loaded and one buffer set up, as every later one finds it.
        vm.eval_source(&format!("(progn (require 'cc-mode) (with-temp-buffer (insert {TEXT}) (c-mode)) t)")).unwrap();
        let (setup, locals) = stores(&mut vm, &format!(
            "(progn (setq cc-buffer (generate-new-buffer \"cc\"))
                    (with-current-buffer cc-buffer (insert {TEXT}) (c-mode))
                    (length (buffer-local-variables cc-buffer)))"));
        let (kill, _) = stores(&mut vm, "(kill-buffer cc-buffer)");
        (setup, kill, locals.parse::<u64>().unwrap())
    }).unwrap().join().unwrap();
    eprintln!("c-mode: {locals} locals, {setup} shared stores in setup, {kill} in the kill");
    // A buffer's locals live in the buffer: neither the setup nor the
    // kill stores a local into a shared table.
    assert!(locals >= 200, "c-mode's locals: {locals}");
    assert_eq!(setup, 0, "setup's shared stores for {locals} locals");
    assert_eq!(kill, 0, "kill's shared stores for {locals} locals");
}
