use romacs_vm::Vm;
fn answer(vm: &mut Vm, source: &str) -> Result<String,String> { vm.eval_source(source).and_then(|v|v.display()) }
#[test]
fn temporary_local_values_follow_owner_lifetime_not_global_slots() {
    fn retained(iterations: usize) -> usize {
        let mut vm=Vm::new();
        vm.set_collection_floor(1);
        vm.eval_source(&format!("(dotimes (i {iterations}) (with-temp-buffer (make-local-variable 's124-cycle) (set 's124-cycle (list (current-buffer) (copy-sequence \"local-graph\")))))")).unwrap();
        let count=vm.heap_stats().collections;
        vm.collect_garbage();
        assert!(vm.heap_stats().collections>count);
        vm.heap_stats().live_objects
    }
    let small=retained(10); let large=retained(100);
    assert!(large<=small+2,"temporary local graphs grow with iterations: {small} -> {large}");
    let mut vm=Vm::new();
    vm.set_gc_moving(true);
    vm.set_collection_floor(usize::MAX);
    // The retained-owner half needs a live buffer. with-temp-buffer now kills
    // its buffer on exit, as GNU does; retain a generated buffer instead.
    let owner=vm.eval_source("(dotimes (i 80) (copy-sequence \"dead-owner-prefix\")) (let ((s124-previous (current-buffer))) (unwind-protect (progn (set-buffer (get-buffer-create (generate-new-buffer-name \"s124-live-owner\"))) (set (intern \"#:vm2-buffer:s124-live-owner\") nil) (make-local-variable 's124-cycle) (set 's124-cycle (list (current-buffer) (copy-sequence \"live\"))) (current-buffer)) (set-buffer s124-previous)))").unwrap();
    let old=format!("{owner:?}"); let owner=owner.root(); let before=vm.heap_stats().collections;
    vm.collect_garbage();
    assert!(vm.heap_stats().collections>before);
    assert_ne!(old,format!("{:?}",owner.get(&vm).unwrap()),"owner fixture must compact");
    vm.declare_special_rooted("s124-owned-buffer",&owner).unwrap();
    assert_eq!(answer(&mut vm,"(set-buffer s124-owned-buffer) (list (eq (car s124-cycle) (current-buffer)) (car (cdr s124-cycle)))"),Ok("(t \"live\")".into()));
}
#[test]
fn protected_constants_reject_all_variable_mutators() {
    let mut vm=Vm::new();
    let before=answer(&mut vm,"(list most-positive-fixnum most-negative-fixnum)").unwrap();
    for form in ["(set 'most-positive-fixnum 0)","(makunbound 'most-negative-fixnum)","(make-local-variable 'most-positive-fixnum)"] {
        assert!(vm.eval_source(form).is_err(),"accepted protected mutation: {form}");
        assert_eq!(answer(&mut vm,&format!("(condition-case e {form} (setting-constant 'protected))")),Ok("protected".into()));
        assert_eq!(answer(&mut vm,"(list most-positive-fixnum most-negative-fixnum)").unwrap(),before);
    }
}
#[test]
fn mode_establishes_local_binding_after_explicit_localization_elsewhere() {
    let mut vm=Vm::new();
    assert_eq!(answer(&mut vm,"(defvar major-mode 'fundamental-mode) (make-local-variable 'major-mode) (set-buffer (get-buffer-create \"s124-other-mode\")) (emacs-lisp-mode) (set-buffer (get-buffer-create \"s124-third-mode\")) major-mode"),Ok("fundamental-mode".into()));
    assert_eq!(answer(&mut vm,"(set-buffer (get-buffer-create \"s124-other-mode\")) major-mode"),Ok("emacs-lisp-mode".into()));
}
#[test]
fn local_if_set_detects_existing_local_binding_but_not_other_buffers() {
    let mut vm=Vm::new();
    assert_eq!(answer(&mut vm,"(local-variable-if-set-p 'truncate-lines)"),Ok("t".into()));
    assert_eq!(answer(&mut vm,"(setq s124-explicit 1) (make-local-variable 's124-explicit) (list (local-variable-if-set-p 's124-explicit) (progn (set-buffer (get-buffer-create \"s124-no-local\")) (local-variable-if-set-p 's124-explicit)))"),Ok("(t nil)".into()));
}
