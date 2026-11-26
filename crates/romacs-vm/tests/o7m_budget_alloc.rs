//! O7m Sol S3. Only the seed request's native allocation traffic is measured.
#[path = "support/counting_alloc.rs"]
mod counting_alloc;
#[macro_use]
#[path = "support/o7m_codec.rs"]
mod support;

use counting_alloc::{Counting, measure, measure_with_bytes};
use romacs_vm::{Local, Vm};
use std::path::Path;
use support::{configure_store, eval, gnu_lisp};

#[global_allocator]
static ALLOCATOR: Counting = Counting;

const LIMIT: usize = 128;
const MAX_BYTES: usize = 64 * 1024;

// Sol S3. Measure both arms before asserting so a bad vector cannot mask
// premature decoding or scanning of a pending function.
#[test]
fn review_budget_checked_before_large_allocations() {
    in_private_store!(review_budget_checked_before_large_allocations, |dir| {
        let vector = vector_refusal_bytes();
        let pending = pending_refusal_bytes(dir);
        assert!(
            vector <= MAX_BYTES && pending <= MAX_BYTES,
            "limit {LIMIT}, ceiling {MAX_BYTES} bytes; vector allocated {vector}, pending body allocated {pending}"
        );
    });
}

fn vector_refusal_bytes() -> usize {
    let mut vm = Vm::new();
    vm.install_lisp_prelude().unwrap();
    eval(&mut vm, "(setq o7m-object (make-vector 8 23)) nil");
    let request = seed_request(&mut vm, "variable");
    // Verify the same precompiled request accepts a small value before
    // swapping in the wide fixture. Fixture construction is outside the meter.
    let (control, ..) = measure(|| refusal(&mut vm, request));
    assert_eq!(control, "nil");
    eval(&mut vm, "(setq o7m-object (make-vector 10000000 23)) nil");
    vm.collect_garbage();
    measured_refusal(&mut vm, request)
}

fn pending_refusal_bytes(dir: &Path) -> usize {
    let path = dir.join("oversize.el");
    let body = "(setq o7m-budget-state (1+ o7m-budget-state))\n".repeat(8192);
    std::fs::write(
        &path,
        format!(";; -*- lexical-binding: t; -*-\n(defun o7m-object ()\n{body})\n"),
    )
    .unwrap();

    let mut compiler = gnu_lisp(&[]);
    eval(&mut compiler, "(require 'romacs-compile)");
    configure_store(&dir.join("rlc"));
    compiler.compile_lisp_file(&path).unwrap();

    let mut vm = gnu_lisp(&[]);
    let path = serde_json::to_string(&path.with_extension("")).unwrap();
    eval(&mut vm, &format!("(load {path} nil t)"));
    assert_eq!(
        vm.eval_source("(symbol-function 'o7m-object)")
            .unwrap()
            .function_body_kind()
            .unwrap(),
        "pending",
        "fixture must load from the compiled artifact"
    );
    let request = seed_request(&mut vm, "function");
    vm.collect_garbage();
    measured_refusal(&mut vm, request)
}

fn seed_request(vm: &mut Vm, kind: &str) -> usize {
    vm.compile_source(&format!(
        "(nth 2 (car (cdr (romacs--seed-build '(({kind} o7m-object)) '(:incarnation 1) {LIMIT}))))"
    ))
    .unwrap()
}

fn measured_refusal(vm: &mut Vm, request: usize) -> usize {
    let (actual, _, _, _, bytes) = measure_with_bytes(|| refusal(vm, request));
    assert_eq!(actual, "too-large");
    bytes
}

fn refusal(vm: &mut Vm, request: usize) -> String {
    vm.execute(request).and_then(Local::display).unwrap()
}
