//! The buffer list's order (buffer_order.rs), linked through an arena the VM
//! owns: against pinned GNU after each step that changes it; the lead step
//! `romacs--lead-buffer-list`, as its comment states; across a moving
//! collection; and through a world image.
#[path = "support/gnu_lisp.rs"]
mod gnu_lisp;
#[path = "support/gnu_batch.rs"]
mod gnu_batch;
use gnu_lisp::gnu_lisp;
use romacs_vm::world::store::World;
use std::path::Path;

/// The names of the `bo-` buffers in `buffer-list`'s order: only those, as
/// each engine starts with buffers of its own (GNU batch has `*Messages*`,
/// Romacs batch none, romacs#350).
const NAMES: &str = r#"(lambda () (delq nil (mapcar (lambda (b) (let ((n (buffer-name b))) (and n (string-match "\\`bo-" n) n))) (buffer-list))))"#;

fn printed(mut vm: romacs_vm::Vm, source: &str) -> String {
    vm.eval_source(&format!("(prin1-to-string {source})")).and_then(romacs_vm::Local::display)
        .map_or_else(|e| format!("error: {e}"), |s| serde_json::from_str::<String>(&s).unwrap_or(s))
}
fn in_thread(f: impl FnOnce() -> String + Send + 'static) -> String {
    std::thread::Builder::new().stack_size(256 << 20).spawn(f).unwrap().join().unwrap()
}

/// Create a, b, c; `switch-to-buffer` b; `bury-buffer` a; kill c; create d;
/// `select-window` on the window once it shows a; rename b; a buffer made
/// and killed inside a `with-temp-buffer`. The order after every step.
#[test]
fn buffer_list_order_matches_pinned_gnu() {
    let source = format!(r#"(let* ((names {NAMES}) (log nil) (step (lambda () (push (funcall names) log))))
      (let ((a (get-buffer-create "bo-a")) (b (get-buffer-create "bo-b")) (c (get-buffer-create "bo-c")))
        (funcall step)
        (switch-to-buffer b) (funcall step)
        (bury-buffer a) (funcall step)
        (kill-buffer c) (funcall step)
        (get-buffer-create "bo-d") (funcall step)
        (set-window-buffer nil a) (select-window (selected-window)) (funcall step)
        (with-current-buffer b (rename-buffer "bo-b2")) (funcall step)
        (with-temp-buffer (let ((x (get-buffer-create "bo-inner"))) (funcall step) (kill-buffer x)) (funcall step))
        (funcall step)
        (nreverse log)))"#);
    let expected = gnu_batch::outputs(&[format!("(prin1 {source})")]).remove(0);
    let actual = in_thread(move || printed(gnu_lisp(&[]), &source));
    assert_eq!(actual, expected);
}

/// `romacs--lead-buffer-list`: BUFFERS first, each once and in their order,
/// a dead one passed over; then the others in their order.
#[test]
fn lead_buffer_list_puts_its_buffers_first() {
    let source = format!(r#"(let* ((names {NAMES})
                                   (a (get-buffer-create "bo-a")) (b (get-buffer-create "bo-b"))
                                   (c (get-buffer-create "bo-c")) (d (get-buffer-create "bo-d"))
                                   (dead (get-buffer-create "bo-dead")))
      (kill-buffer dead)
      (romacs--lead-buffer-list (list c dead a c))
      (funcall names))"#);
    assert_eq!(in_thread(move || printed(gnu_lisp(&[]), &source)), r#"("bo-c" "bo-a" "bo-b" "bo-d")"#);
}

/// The order through collections that move every object (forced: every
/// collection compacts, one every 50 allocations), with buffers made, buried,
/// recorded and killed in between. The collector's verifier runs at each,
/// checking that every linked buffer is live and knows its node.
#[test]
fn buffer_order_survives_moving_collections() {
    let mut vm = romacs_vm::Vm::new();
    vm.install_lisp_prelude().unwrap();
    vm.set_gc_moving(true);
    vm.set_gc_stress(50);
    vm.set_gc_verify(true);
    let source = format!(r#"(let* ((names {NAMES}) (log nil)
             (churn (lambda () (dotimes (_ 300) (make-string 10 ?x)) (garbage-collect))))
      (dotimes (i 6) (get-buffer-create (format "bo-%d" i)))
      (funcall churn) (push (funcall names) log)
      (bury-buffer-internal (get-buffer "bo-1")) (kill-buffer "bo-3")
      (funcall churn) (push (funcall names) log)
      (get-buffer-create "bo-6") (romacs--lead-buffer-list (list (get-buffer "bo-4")))
      (funcall churn) (push (funcall names) log)
      (nreverse log))"#);
    assert_eq!(printed(vm, &source),
        r#"(("bo-0" "bo-1" "bo-2" "bo-3" "bo-4" "bo-5") ("bo-0" "bo-2" "bo-4" "bo-5" "bo-1") ("bo-4" "bo-0" "bo-2" "bo-5" "bo-1" "bo-6"))"#);
}

/// The builder carries the index it freed. In the GNU target the restored
/// VM must reuse that exact index on its first birth, before any new release.
#[test]
fn buffer_order_survives_a_world_image() {
    fn prepare(vm: &mut romacs_vm::Vm) -> Result<u32, String> {
        let buffer = vm.eval_source(r#"(progn
            (dolist (n '("bo-w1" "bo-w2" "bo-w3" "bo-w4")) (get-buffer-create n))
            (bury-buffer-internal (get-buffer "bo-w1"))
            (get-buffer "bo-w3"))"#)?;
        let freed = buffer.buffer_order_node().expect("live buffer has an order node");
        vm.eval_source(r#"(kill-buffer "bo-w3")"#)?;
        Ok(freed)
    }

    let actual = in_thread(|| {
        let (mut vm, freed) = if env!("CARGO_CRATE_NAME") == "gnu" {
            let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
            let store = Path::new(env!("CARGO_TARGET_TMPDIR")).join("buffer-order-worlds");
            World::for_test(store, &[format!("buffer order in {}", root.display())], || {
                romacs_vm::host::end_at_dump_point();
                let mut vm = gnu_lisp::booted(&root, true);
                let freed = prepare(&mut vm)?;
                Ok((vm, freed))
            }).expect("GNU target must restore the test world; run it with nextest")
        } else {
            // The standalone test target boots from source.
            let mut vm = gnu_lisp(&[]);
            let freed = prepare(&mut vm).unwrap();
            (vm, freed)
        };
        vm.set_gc_verify(true);
        vm.collect_garbage();
        let names = format!("(funcall {NAMES})");
        assert_eq!(vm.eval_source(&names).unwrap().display().unwrap(), r#"("bo-w2" "bo-w4" "bo-w1")"#);
        let born = vm.eval_source(r#"(get-buffer-create "bo-w5")"#).unwrap();
        assert_eq!(born.buffer_order_node(), Some(freed), "birth uses the slot the builder released");
        printed(vm, &format!(r#"(progn (bury-buffer-internal (get-buffer "bo-w2")) (kill-buffer "bo-w4")
                                       (garbage-collect) {names})"#))
    });
    assert_eq!(actual, r#"("bo-w1" "bo-w5" "bo-w2")"#);
}

/// Burying a killed buffer leaves the order as it was: a dead buffer joins
/// no list, so nothing keeps it.
#[test]
fn burying_a_killed_buffer_changes_nothing() {
    let mut vm = romacs_vm::Vm::new();
    vm.install_lisp_prelude().unwrap();
    let source = format!(r#"(let* ((names {NAMES}) (a (get-buffer-create "bo-a")) (dead (get-buffer-create "bo-dead")))
      (get-buffer-create "bo-b")
      (kill-buffer dead)
      (bury-buffer-internal dead)
      (list (funcall names) (memq dead (buffer-list)) (buffer-live-p dead)))"#);
    assert_eq!(printed(vm, &source), r#"(("bo-a" "bo-b") nil nil)"#);
}

/// A name slot (`#:vm2-buffer:NAME`) is a symbol Lisp can set: cleared, it no
/// longer holds its buffer, and the order must. Under collections that move
/// every object and the verifier, the buffer stays live and in `buffer-list`
/// (the lane's probe, K4 review).
#[test]
fn buffer_order_holds_a_buffer_whose_name_slot_lisp_cleared() {
    let mut vm = romacs_vm::Vm::new();
    vm.install_lisp_prelude().unwrap();
    vm.set_gc_moving(true);
    vm.set_gc_stress(50);
    vm.set_gc_verify(true);
    // Each step is its own top-level evaluation, which leaves nothing in a
    // register, and none binds bo-z: once its name slot is cleared, only the
    // order can keep it, so the mark of the order is all that saves it.
    for step in [r#"(progn (get-buffer-create "bo-z") (get-buffer-create "bo-y") nil)"#,
                 r##"(progn (set (intern-soft "#:vm2-buffer:bo-z") nil) nil)"##,
                 "(progn (dotimes (_ 300) (make-string 10 ?x)) (garbage-collect) (dotimes (_ 300) (make-string 10 ?x)) (garbage-collect) nil)"] {
        vm.eval_source(step).unwrap();
    }
    let source = format!(r#"(let ((names {NAMES}) (tail (buffer-list)) z)
      (while tail (if (equal (buffer-name (car tail)) "bo-z") (setq z (car tail))) (setq tail (cdr tail)))
      (list (funcall names) (buffer-live-p z) (buffer-name z)))"#);
    assert_eq!(printed(vm, &source), r#"(("bo-z" "bo-y") t "bo-z")"#);
}
