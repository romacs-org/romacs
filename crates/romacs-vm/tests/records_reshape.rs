//! Records found by tag and reshaped in place (`romacs-records-tagged`,
//! `romacs-records-reshape`): what redefining a struct in a live session
//! builds on. Identity, holders and weak tables keep each record; a batch
//! that refuses changes none.
use romacs_vm::Vm;

/// A VM with its Lisp prelude (`mapcar` among it).
fn vm() -> Vm {
    let mut vm = Vm::new();
    vm.install_lisp_prelude().unwrap();
    vm
}

fn eval(vm: &mut Vm, source: &str) -> String {
    let value = vm.eval_source(&format!("(prin1-to-string {source})")).unwrap_or_else(|e| panic!("{source}: {e}"));
    serde_json::from_str(&value.display().unwrap()).unwrap()
}

const SETUP: &str = r#"(progn
  (setq rr-all (let (l) (dotimes (i 50) (push (record 'rr-a i (format "s%d" i) (intern (format "c%d" i))) l)) (nreverse l)))
  (setq rr-ids (make-hash-table :test 'eq))
  (setq rr-weak (make-hash-table :test 'eq :weakness 'key))
  (dolist (r rr-all) (puthash r (aref r 1) rr-ids) (puthash r t rr-weak))
  (setq rr-vec (vconcat rr-all))
  (setq rr-closure (let ((r (nth 7 rr-all))) (lambda () r)))
  (setq rr-sub (record 'rr-b 1000 "sub" 'z 'dee))
  (setq rr-other (record 'rr-other 1 2 3))
  t)"#;

/// Allocate until a minor collection is recorded: the kinds of collection
/// seen, in order (`romacs--take-collections').
const UNTIL_MINOR: &str = r#"(let ((kinds nil) (i 0))
  (romacs--take-collections)
  (while (and (< i 400) (not (memq 'minor kinds)))
    (dotimes (_ 50000) (cons i i))
    (setq kinds (append kinds (mapcar (lambda (r) (if (consp r) (car r) r)) (cdr (romacs--take-collections)))) i (1+ i)))
  kinds)"#;

#[test]
fn a_reshape_keeps_identity_through_every_holder() {
    let mut vm = vm();
    eval(&mut vm, SETUP);
    // Old records first, so each new slot is an old entry's store of a young value.
    eval(&mut vm, "(garbage-collect)");
    // Reorder and grow (a b c) to (c e a b), each new e a fresh cons of its own record.
    assert_eq!(eval(&mut vm, r#"(let ((rs (romacs-records-tagged '(rr-a))))
        (list (length rs)
              (romacs-records-reshape rs 'rr-a [3 nil 1 2] (mapcar (lambda (r) (vector nil (list 'fresh (aref r 1)) nil nil)) rs))))"#),
        "(50 50)");
    // The young conses reachable only through their new slot survive a minor collection.
    let kinds = eval(&mut vm, UNTIL_MINOR);
    assert!(kinds.starts_with("(minor"), "the first collection after the reshape is a minor: {kinds}");
    assert_eq!(eval(&mut vm, r#"(let ((bad 0))
        (dolist (r rr-all)
          (let ((i (gethash r rr-ids)))
            (unless (and i (= (length r) 5)
                         (equal (list (aref r 0) (aref r 1) (aref r 2) (aref r 3) (aref r 4))
                                (list 'rr-a (intern (format "c%d" i)) (list 'fresh i) i (format "s%d" i))))
              (setq bad (1+ bad)))))
        (list bad (hash-table-count rr-weak) (eq (aref rr-vec 7) (funcall rr-closure)) (aref (funcall rr-closure) 3)))"#),
        "(0 50 t 7)");
    // Untouched: another tag, and the descendant (its own tag, not passed).
    assert_eq!(eval(&mut vm, "(list rr-other rr-sub)"), r#"(#s(rr-other 1 2 3) #s(rr-b 1000 "sub" z dee))"#);
}

#[test]
fn one_walk_finds_a_struct_and_its_descendants_each_reshaped_by_its_own_map() {
    let mut vm = vm();
    eval(&mut vm, SETUP);
    assert_eq!(eval(&mut vm, "(let ((rs (romacs-records-tagged '(rr-b rr-a))) (b 0)) (dolist (r rs) (when (eq (aref r 0) 'rr-b) (setq b (1+ b)))) (list (length rs) b))"),
        "(51 1)");
    // Shrink and reorder the parent's slots (b a), the child's (b a d), one shared fill.
    assert_eq!(eval(&mut vm, r#"(list (romacs-records-reshape (romacs-records-tagged '(rr-a)) 'rr-a [2 1] nil)
                                      (romacs-records-reshape (list rr-sub) 'rr-b [2 1 4] nil)
                                      (nth 3 rr-all) rr-sub)"#),
        r#"(50 1 #s(rr-a "s3" 3) #s(rr-b "sub" 1000 dee))"#);
}

#[test]
fn a_refused_batch_changes_no_record() {
    let mut vm = vm();
    eval(&mut vm, SETUP);
    // A frozen record last: the read-only signal, and the first records as they were.
    assert_eq!(eval(&mut vm, r#"(let ((fz (freeze (record 'rr-a 1 2 3))))
        (list (car (condition-case e (romacs-records-reshape (append rr-all (list fz)) 'rr-a [1] nil) (error (list 'refused e))))
              (car rr-all) (aref fz 3)))"#),
        r#"(refused #s(rr-a 0 "s0" c0) 3)"#);
    // An old index past one record's slots: out of range, nothing changed.
    assert_eq!(eval(&mut vm, r#"(list (condition-case e (romacs-records-reshape (list (car rr-all) (record 'rr-a 1)) 'rr-a [3] nil) (args-out-of-range 'range))
                                      (car rr-all))"#),
        r#"(range #s(rr-a 0 "s0" c0))"#);
    // Not a record.
    assert_eq!(eval(&mut vm, "(condition-case e (romacs-records-reshape (list [rr-a 1]) 'rr-a [1] nil) (wrong-type-argument (car (cdr e))))"),
        "recordp");
}

#[test]
fn a_reshape_keeps_the_record_slot_limit() {
    let mut vm = vm();
    eval(&mut vm, SETUP);
    // 4,095 slots, the tag included, as `make-record' allows; one more refused.
    assert_eq!(eval(&mut vm, r#"(let ((r (car rr-all)))
        (list (romacs-records-reshape (list r) 'rr-a (make-vector 4094 nil) nil) (length r)
              (condition-case e (romacs-records-reshape (list r) 'rr-a (make-vector 4095 nil) nil) (error 'refused))
              (length r)))"#),
        "(1 4095 refused 4095)");
}
