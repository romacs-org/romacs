use romacs_vm::Vm;

fn show(vm: &mut Vm, source: &str) -> String {
    match vm.eval_source(source).and_then(|v| v.display()) {
        Ok(s) => s,
        Err(e) => format!("ERROR {e}"),
    }
}

#[test]
fn mapc_returns_original_sequence_and_observes_callback_mutation() {
    let mut vm = Vm::new();
    vm.install_lisp_prelude().unwrap();
    for (source, expected) in [
        ("(list (fboundp 'mapc) (functionp #'mapc) (funcall #'mapc #'identity nil))", "(t t nil)"),
        ("(let ((s (list 1 2 3)) calls) (list (eq s (mapc (lambda (x) (setq calls (cons x calls)) (cons x x)) s)) calls))", "(t (3 2 1))"),
        ("(let ((s (vector 1 2 3)) calls) (list (eq s (mapc (lambda (x) (setq calls (cons x calls)) (aset s 2 'z)) s)) calls))", "(t (z 2 1))"),
        ("(let ((s (list 1 2 3)) calls) (mapc (lambda (x) (setq calls (cons x calls)) (setcdr s nil)) s) calls)", "(1)"),
        ("(let ((s (list 1 2)) calls) (mapc (lambda (x) (setq calls (cons x calls)) (setcdr (cdr s) (list 9))) s) calls)", "(2 1)"),
        ("(let (calls) (mapc (lambda (x) (setq calls (cons x calls))) (string ?a ?λ ?雪)) calls)", "(38634 955 97)"),
        ("(let (calls) (mapc (lambda (x) (setq calls (cons x calls))) (unibyte-string 0 128 255)) calls)", "(255 128 0)"),
        ("(mapc 'absent-mapc-callback [])", "[]"),
        ("(condition-case e (mapc #'identity '(1 . 2)) (error e))", "(wrong-type-argument listp 2)"),
        ("(condition-case e (mapc #'identity 7) (error e))", "(wrong-type-argument sequencep 7)"),
        // GNU's datum is `#<subr mapc>`; the prelude's is its arity (VM2-MAPCAR1).
        ("(condition-case e (apply #'mapc '(identity)) (error e))", "(wrong-number-of-arguments (2 . 2) 1)"),
        ("(catch 'done (mapc (lambda (x) (if (eq x 2) (throw 'done x))) '(1 2 3)))", "2"),
        ("(condition-case e (mapc (lambda (x) (signal 'error (list x))) '(1 2)) (error e))", "(error 1)"),
    ] {
        assert_eq!(show(&mut vm, source), expected, "mapc GNU observable behavior: {source}");
    }
}

#[test]
fn mapc_callback_collects_and_releases_objects_and_bytes() {
    let mut vm = Vm::new();
    vm.install_lisp_prelude().unwrap();
    vm.set_collection_floor(1);
    // Identical source and symbol inventory on every iteration: compiler constants
    // and newly interned names cannot masquerade as a callback retention leak.
    const SOURCE: &str = "(let ((s (vector (make-string 4000 ?a) (make-string 4000 ?b) (make-string 4000 ?c))) (calls 0)) (list (eq s (mapc (lambda (x) (setq calls (1+ calls)) (let ((i 0)) (while (< i 12000) (cons x (make-string 80 ?z)) (setq i (1+ i))))) s)) calls (aref (aref s 2) 3999)))";
    assert_eq!(show(&mut vm, SOURCE), "(t 3 99)", "callback roots survive moving collection");
    vm.collect_garbage();
    vm.collect_garbage();
    let baseline = vm.heap_stats();
    for _ in 0..4 {
        let before = vm.heap_stats().collections;
        assert_eq!(show(&mut vm, SOURCE), "(t 3 99)", "callback roots survive moving collection");
        assert!(vm.heap_stats().collections >= before + 3, "each callback actually collects");
        vm.collect_garbage();
        vm.collect_garbage();
        let after = vm.heap_stats();
        eprintln!("mapc retention: {}/{} -> {}/{}", baseline.live_objects, baseline.live_bytes, after.live_objects, after.live_bytes);
        assert_eq!((after.live_objects, after.live_bytes), (baseline.live_objects, baseline.live_bytes), "mapc callback state must release objects AND bytes");
    }
}
