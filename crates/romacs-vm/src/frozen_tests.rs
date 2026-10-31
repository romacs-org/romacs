//! Frozen regions: graph semantics, refusals, every mutator, collection
//! with weak tables of every kind, id reuse and exhaustion, portable
//! hashing across region ids and intern orders, world-capture refusal.
use super::*;

fn vm() -> Vm {
    let mut vm = Vm::new();
    vm.install_lisp_prelude().expect("prelude");
    vm.eval_source("(progn (defvar trash nil) (defun cadr (x) (car (cdr x))) (defun cddr (x) (cdr (cdr x))) (defun caddr (x) (car (cdr (cdr x)))))").expect("helpers");
    vm
}

fn eval(vm: &mut Vm, source: &str) -> String {
    vm.eval_source(source).and_then(|v| v.display()).unwrap_or_else(|e| format!("ERROR {e}"))
}

/// SOURCE's signal as `(CONDITION . DATA)` printed, or its value.
fn caught(vm: &mut Vm, source: &str) -> String {
    eval(vm, &format!("(condition-case e {source} (error (cons 'signalled e)))"))
}

#[test]
fn immediates_freeze_to_themselves() {
    let mut vm = Vm::new();
    assert_eq!(vm.freeze(Value::NIL).ok(), Some(Value::NIL));
    assert_eq!(vm.freeze(Value::integer(7).unwrap()).ok(), Some(Value::integer(7).unwrap()));
    assert!(!Vm::frozen(Value::TRUE));
}

#[test]
fn a_frozen_graph_keeps_cycles_sharing_and_uninterned_identity() {
    let mut vm = vm();
    assert_eq!(eval(&mut vm, "(let ((l (list 1 2))) (setcdr (cdr l) l) (let ((f (freeze l))) (list (frozen-p f) (eq (cddr f) f) (car f) (cadr f))))"), "(t t 1 2)");
    assert_eq!(eval(&mut vm, "(let* ((shared (list 'a)) (f (freeze (list shared shared)))) (list (eq (car f) (cadr f)) (frozen-p (car f)) (equal f '((a) (a)))))"), "(t t t)");
    assert_eq!(eval(&mut vm, "(let* ((s (make-symbol \"uninterned-probe\")) (f (freeze (list s s)))) (list (eq (car f) (cadr f)) (eq (car f) s) (string= (symbol-name (car f)) \"uninterned-probe\") (intern-soft \"uninterned-probe\")))"), "(t nil t nil)");
    assert_eq!(eval(&mut vm, "(let ((f (freeze (list 'car :key \"text\" 1.5 (vector 1 2) #s(point 3 4))))) (list (frozen-p f) (eq (car f) 'car) (eq (cadr f) :key) (equal f (list 'car :key \"text\" 1.5 (vector 1 2) #s(point 3 4)))))"), "(t t t t)");
    assert_eq!(eval(&mut vm, "(let ((f (freeze (freeze (list 1))))) (frozen-p f))"), "t");
    // Freezing a region's root is the identity; a sub-value is copied.
    assert_eq!(eval(&mut vm, "(let* ((f (freeze (list 1 2))) (g (freeze f)) (h (freeze (cdr f)))) (list (eq f g) (eq h (cdr f)) (frozen-p h)))"), "(t nil t)");
}

#[test]
fn thaw_gives_a_private_mutable_copy() {
    let mut vm = vm();
    assert_eq!(eval(&mut vm, "(let* ((f (freeze (list 1 (vector 2 3) \"s\"))) (p (thaw f))) (setcar p 9) (aset (cadr p) 0 8) (list (frozen-p p) (frozen-p (cadr p)) p (car f) (aref (cadr f) 0)))"), "(nil nil (9 [8 3] \"s\") 1 2)");
    assert_eq!(eval(&mut vm, "(let ((l (list 1 2))) (setcdr (cdr l) l) (let ((p (thaw (freeze l)))) (list (frozen-p p) (eq (cddr p) p))))"), "(t nil)".replace("(t nil)", "(nil t)"));
    // thaw of a private value is the value.
    assert_eq!(eval(&mut vm, "(let ((l (list 1))) (eq (thaw l) l))"), "t");
    // copy-sequence of a frozen value is private.
    assert_eq!(eval(&mut vm, "(let ((f (freeze (vector 1 2)))) (list (frozen-p (copy-sequence f)) (frozen-p (copy-sequence (freeze (list 1))))))"), "(nil nil)");
}

#[test]
fn refusals_name_the_object_and_the_path() {
    let mut vm = vm();
    assert_eq!(eval(&mut vm, "(condition-case e (freeze (list 1 (vector 2 (current-buffer)))) (error (list (car e) (cadr e) (eq (caddr e) (current-buffer)) (nth 3 e))))"), "(error \"Cannot freeze\" t (cdr car (aref 1)))");
    assert_eq!(caught(&mut vm, "(freeze (lambda (x) x))").starts_with("(signalled error \"Cannot freeze\" "), true);
    assert_eq!(caught(&mut vm, "(freeze (make-hash-table :weakness 'key))").contains("Cannot freeze"), true);
    assert_eq!(caught(&mut vm, "(freeze (propertize \"s\" 'face (current-buffer)))").ends_with("((property 0)))"), true);
    assert_eq!(caught(&mut vm, "(let ((h (make-hash-table))) (puthash 'k (point-marker) h) (freeze h))").ends_with("((value 0)))"), true);
    assert_eq!(caught(&mut vm, "(freeze (symbol-function 'car))").contains("Cannot freeze"), true);
}

#[test]
fn every_mutator_refuses_with_gnus_read_only_error() {
    let mut vm = vm();
    let cases = [
        ("(setcar f 1)", "(freeze (list 1 2))"),
        ("(setcdr f 1)", "(freeze (list 1 2))"),
        ("(nreverse f)", "(freeze (list 1 2))"),
        ("(nconc f (list 3))", "(freeze (list 1 2))"),
        ("(sort f #'<)", "(freeze (list 2 1))"),
        ("(delq 2 f)", "(freeze (list 1 2))"),
        ("(aset f 0 9)", "(freeze (vector 1 2))"),
        ("(fillarray f 0)", "(freeze (vector 1 2))"),
        ("(nreverse f)", "(freeze (vector 1 2))"),
        ("(aset f 0 ?x)", "(freeze (copy-sequence \"ab\"))"),
        ("(fillarray f ?x)", "(freeze (copy-sequence \"ab\"))"),
        ("(clear-string f)", "(freeze (copy-sequence \"ab\"))"),
        ("(put-text-property 0 1 'face 'bold f)", "(freeze (copy-sequence \"ab\"))"),
        ("(set-text-properties 0 1 nil f)", "(freeze (propertize \"ab\" 'face 'bold))"),
        ("(aset f 0 t)", "(freeze (make-bool-vector 3 nil))"),
        ("(puthash 'k 1 f)", "(freeze (make-hash-table))"),
        ("(remhash 'k f)", "(let ((h (make-hash-table))) (puthash 'k 1 h) (freeze h))"),
        ("(clrhash f)", "(let ((h (make-hash-table))) (puthash 'k 1 h) (freeze h))"),
        ("(aset f ?a 1)", "(freeze (make-char-table 'test))"),
        ("(set-char-table-range f '(?a . ?b) 1)", "(freeze (make-char-table 'test))"),
        ("(set-char-table-parent f nil)", "(freeze (make-char-table 'test))"),
        ("(aset f 0 1)", "(freeze (record 'r 1))"),
    ];
    for (form, make) in cases {
        let got = caught(&mut vm, &format!("(let ((f {make})) {form})"));
        assert!(got.starts_with("(signalled error \"Attempt to modify read-only object\" "), "{form} on {make}: {got}");
    }
    // The readers of every kind still work.
    assert_eq!(eval(&mut vm, "(let ((f (freeze (list (vector 1 2) (propertize \"ab\" 'face 'bold) (make-bool-vector 3 t) (let ((h (make-hash-table :test 'equal))) (puthash \"k\" 'v h) h) (let ((c (make-char-table 'test 0))) (aset c ?a 'x) c) (record 'r 1))))) (list (aref (nth 0 f) 1) (get-text-property 0 'face (nth 1 f)) (aref (nth 2 f) 2) (gethash \"k\" (nth 3 f)) (hash-table-count (nth 3 f)) (aref (nth 4 f) ?a) (aref (nth 4 f) ?b) (aref (nth 5 f) 1) (type-of (nth 5 f)) (length (nth 0 f)) (length (nth 1 f))))"), "(2 bold t v 1 x 0 1 r 2 2)");
}

#[test]
fn writers_that_indexed_the_arena_directly_refuse_frozen_input() {
    // Sol r1 S1: define-key, intern into a vector obarray, font-put and
    // modify-syntax-entry used to index the object arena with the frozen
    // payload and panic.
    let mut vm = vm();
    for (form, make) in [
        ("(define-key f [?a] 'ignore)", "(freeze (make-keymap))"),
        ("(intern \"frozen-obarray-probe\" f)", "(freeze (make-vector 7 0))"),
        ("(set-char-table-range f ?a 1)", "(freeze (make-char-table 'test))"),
    ] {
        let got = caught(&mut vm, &format!("(let ((f {make})) {form})"));
        assert!(got.starts_with("(signalled error \"Attempt to modify read-only object\" "), "{form} on {make}: {got}");
    }
    // A syntax table is refused at freeze, as the design states.
    let syntax = caught(&mut vm, "(freeze (let ((c (make-char-table 'syntax-table))) c))");
    assert!(syntax.contains("Cannot freeze"), "{syntax}");
    // A frozen value of the wrong kind is a type error, as GNU's, never a panic.
    assert!(caught(&mut vm, "(let ((f (freeze (vector 1 2)))) (set-marker f 1))").starts_with("(signalled "));
}

#[test]
fn bool_vectors_and_category_tables_freeze_and_read() {
    // Sol r1 S8: the bit length was validated as a byte count.
    let mut vm = vm();
    assert_eq!(eval(&mut vm, "(let ((f (freeze (let ((b (make-bool-vector 128 nil))) (aset b 127 t) b)))) (list (length f) (aref f 127) (aref f 0)))"), "(128 t nil)");
    // Sol r1 S11: category-docstring and copy-category-table read a frozen table.
    assert_eq!(eval(&mut vm, "(let* ((c (make-category-table)) (f (progn (define-category ?z \"zed\" c) (freeze c)))) (list (category-docstring ?z f) (category-table-p (copy-category-table f)) (condition-case e (define-category ?y \"why\" f) (error (car (cdr e))))))"), "(\"zed\" t \"Attempt to modify read-only object\")");
}

#[test]
fn unibyte_strings_keep_their_properties_across_freeze_and_thaw() {
    // Sol r1 S9: a unibyte string's properties were dropped on decode.
    let mut vm = vm();
    assert_eq!(eval(&mut vm, "(let* ((s (propertize (unibyte-string 97 98) 'face 'bold)) (f (freeze s)) (p (thaw f))) (list (get-text-property 0 'face s) (get-text-property 0 'face f) (get-text-property 0 'face p) (multibyte-string-p f) (equal f s)))"), "(bold bold bold nil t)");
    assert_eq!(eval(&mut vm, "(let ((f (freeze (unibyte-string 200 201)))) (list (multibyte-string-p f) (aref f 0) (length f)))"), "(nil 200 2)");
}

#[test]
fn frozen_strings_insert_as_private_ones() {
    // A snapshot's text and a job's string literals reach `insert` frozen.
    let mut vm = vm();
    for text in ["(propertize \"aλb\" 'face 'bold)", "(unibyte-string 97 200)", "(make-string 100000 ?x)"] {
        let src = format!("(let ((s {text}) (b (get-buffer-create \"frozen-insert\")))
          (set-buffer b) (erase-buffer) (insert s) (setq trash (list (buffer-string) (get-text-property 1 'face)))
          (erase-buffer) (insert (freeze s) ?z) (list (equal (buffer-substring 1 (1- (point-max))) (car trash))
                                                       (equal (get-text-property 1 'face) (nth 1 trash)) (buffer-size)))");
        let expected = eval(&mut vm, &format!("(let ((s {text})) (list t t (1+ (length (if (multibyte-string-p s) s (string-to-multibyte s))))))"));
        assert_eq!(eval(&mut vm, &src), expected, "{text}");
    }
}

#[test]
fn inline_and_sealed_transports_behave_the_same() {
    let mut vm = vm();
    for size in [10usize, 100_000] {
        let src = format!("(let* ((s (make-string {size} ?a)) (f (freeze (list s 1)))) (list (frozen-p f) (frozen-p (car f)) (length (car f)) (condition-case e (aset (car f) 0 ?b) (error (cadr e))) (equal (car f) s) (eq (car f) (car f))))");
        assert_eq!(eval(&mut vm, &src), format!("(t t {size} \"Attempt to modify read-only object\" t t)"));
    }
    let regions = vm.regions.iter().flatten().map(|r| matches!(r.backing, Backing::Sealed(_))).collect::<Vec<_>>();
    assert_eq!(regions, [false, true]);
}

#[test]
fn regions_die_with_their_last_reference_and_ids_are_reused() {
    let mut vm = vm();
    assert_eq!(eval(&mut vm, "(progn (setq kept (freeze (list 'kept))) (freeze (make-string 10000 ?x)) (length (romacs--frozen-regions)))"), "2");
    vm.collect_garbage();
    assert_eq!(eval(&mut vm, "(mapcar #'car (romacs--frozen-regions))"), "(1)");
    assert_eq!(eval(&mut vm, "(progn (setq other (freeze (list 'other))) (mapcar #'car (romacs--frozen-regions)))"), "(1 2)");
    assert_eq!(eval(&mut vm, "(car kept)"), "kept");
    // GNU's keep rule per weakness: entry 1 is (frozen K . K) in region 1,
    // entry 2 is (live . frozen V) in region 2; `strong` holds K.
    for (weakness, strong, regions, count) in [
        ("key", false, "(2)", "1"), ("value", false, "nil", "0"), ("key-or-value", false, "(2)", "1"), ("key-and-value", false, "nil", "0"),
        ("key", true, "(1 2)", "2"), ("value", true, "(1)", "1"), ("key-and-value", true, "(1)", "1"),
    ] {
        let mut vm = self::vm();
        let hold = if strong { "(setq strong frozen)" } else { "nil" };
        eval(&mut vm, &format!("(progn (setq h (make-hash-table :weakness '{weakness})) (let ((frozen (freeze (list 'fk)))) (puthash frozen frozen h) {hold}) (puthash 'live (freeze (vector 'fv)) h) nil)"));
        vm.collect_garbage();
        assert_eq!(eval(&mut vm, "(mapcar #'car (romacs--frozen-regions))"), regions, "{weakness} strong={strong}");
        assert_eq!(eval(&mut vm, "(hash-table-count h)"), count, "{weakness} strong={strong}");
        if regions.contains('2') { assert_eq!(eval(&mut vm, "(aref (gethash 'live h) 0)"), "fv"); }
        if strong { assert_eq!(eval(&mut vm, "(list (car strong) (car (gethash strong h)))"), "(fk fk)"); }
    }
}

#[test]
fn a_value_weak_entry_whose_region_died_is_pruned() {
    let mut vm = vm();
    eval(&mut vm, "(progn (setq h (make-hash-table :weakness 'value)) (puthash 'k (freeze (list 'dead)) h) nil)");
    vm.collect_garbage();
    assert_eq!(eval(&mut vm, "(list (hash-table-count h) (romacs--frozen-regions))"), "(0 nil)");
}

#[test]
fn id_exhaustion_signals_instead_of_corrupting() {
    let mut vm = vm();
    let n = MAX_REGIONS - 1;
    assert_eq!(eval(&mut vm, &format!("(progn (setq all nil) (dotimes (i {n}) (push (freeze (list i)) all)) (length (romacs--frozen-regions)))")), n.to_string());
    assert_eq!(caught(&mut vm, "(freeze (list 'one-more))"), format!("(signalled error \"Too many frozen regions {n}\")"));
    // A dead register of the same run may still name a region: the
    // collection that frees it runs at the next entry.
    eval(&mut vm, "(setq all (cdr all))");
    vm.collect_garbage();
    assert_eq!(eval(&mut vm, "(frozen-p (freeze (list 'fits)))"), "t");
}

#[test]
fn a_freeze_and_discard_loop_is_bounded() {
    let mut vm = vm();
    eval(&mut vm, "(dotimes (i 400) (freeze (make-string 100000 ?a)))");
    assert!(vm.region_count() < 64, "{} regions live after the loop", vm.region_count());
}

#[test]
fn world_capture_refuses_while_a_region_is_attached() {
    let mut vm = Vm::new();
    assert_eq!(vm.world_quiescent(), Ok(()));
    let frozen = vm.eval_source_raw("(freeze (list 1))").unwrap();
    assert!(Vm::frozen(frozen));
    assert_eq!(vm.world_quiescent(), Err("world capture refused: regions is not at rest".into()));
    let entry = vm.compile(&Form::from_json(serde_json::json!(["quote", 1])).unwrap()).unwrap();
    assert!(vm.publish_code_image(std::path::Path::new("/nonexistent"), entry, [0; 32]).is_err());
}

#[test]
fn frozen_tables_hash_portably_across_region_ids_and_intern_orders() {
    let mut a = vm();
    let table = a.eval_source_raw("(let ((h (make-hash-table :test 'equal))) (puthash \"str\" 'by-string h) (puthash '(1 \"x\") 'by-list h) (puthash 'sym 'by-symbol h) (puthash 1.5 'by-float h) (puthash 7 'by-fixnum h) (let ((e (make-hash-table :test 'eq))) (puthash 'a 1 e) (puthash 2 'two e) (puthash :kw 'k e) (let ((l (make-hash-table :test 'eql))) (puthash 2.5 'f l) (puthash (expt 2 70) 'big l) (puthash 'q 'sq l) (list h e l))))").unwrap();
    let bytes = a.encode_frozen(table).ok().expect("freezable");
    for (intern_first, pad) in [(0, 0), (500, 3)] {
        let mut b = vm();
        for i in 0..intern_first { b.intern(&format!("padding-symbol-{i}")); }
        for _ in 0..pad { b.eval_source_raw("(freeze (list 'pad))").unwrap(); }
        let root = b.attach(Backing::Inline(std::sync::Arc::new(bytes.clone()))).unwrap();
        assert_eq!(region_of(root.payload() as usize), pad + 1);
        b.declare_special_raw("tables", root);
        assert_eq!(eval(&mut b, "(list (gethash \"str\" (car tables)) (gethash (list 1 \"x\") (car tables)) (gethash 'sym (car tables)) (gethash 1.5 (car tables)) (gethash 7 (car tables)) (gethash 'missing (car tables)))"), "(by-string by-list by-symbol by-float by-fixnum nil)");
        assert_eq!(eval(&mut b, "(list (gethash 'a (cadr tables)) (gethash 2 (cadr tables)) (gethash :kw (cadr tables)) (gethash 'b (cadr tables)))"), "(1 two k nil)");
        assert_eq!(eval(&mut b, "(list (gethash 2.5 (caddr tables)) (gethash (expt 2 70) (caddr tables)) (gethash 'q (caddr tables)) (gethash 2.0 (caddr tables)))"), "(f big sq nil)");
        assert_eq!(eval(&mut b, "(let (keys) (maphash (lambda (k _) (push k keys)) (car tables)) (length keys))"), "5");
    }
}

#[test]
fn frozen_values_compare_and_print_as_private_ones() {
    let mut vm = vm();
    assert_eq!(eval(&mut vm, "(let ((f (freeze (list \"a\" 1.5 (vector 'x) #s(r 1))))) (list (equal f (list \"a\" 1.5 (vector 'x) #s(r 1))) (= (sxhash-equal f) (sxhash-equal (list \"a\" 1.5 (vector 'x) #s(r 1)))) (format \"%S\" f) (prin1-to-string f)))"), "(t t \"(\\\"a\\\" 1.5 [x] #s(r 1))\" \"(\\\"a\\\" 1.5 [x] #s(r 1))\")");
    assert_eq!(eval(&mut vm, "(let ((f (freeze (list 1 2 3)))) (list (length f) (nth 2 f) (memq 2 f) (mapcar #'1+ f) (apply #'+ f) (append f nil) (delq nil (mapcar (lambda (x) (and (= 1 (% x 2)) x)) f)) (string-match \"b\" (freeze (copy-sequence \"abc\")))))").replace("(2 3)", "(2 3)"), "(3 3 (2 3) (2 3 4) 6 (1 2 3) (1 3) 1)");
}

#[test]
fn gc_stress_moves_nothing_frozen() {
    let mut vm = vm();
    vm.set_gc_stress(1);
    assert_eq!(eval(&mut vm, "(let ((f (freeze (list (vector 1 (list 2 \"s\")) 'sym)))) (dotimes (i 50) (push i trash)) (list (aref (car f) 0) (cadr (aref (car f) 1)) (cadr f) (frozen-p f)))"), "(1 \"s\" sym t)");
}

#[test]
#[ignore]
fn checkpoint_frozen_versus_private_reads() {
    // M1's checkpoint measurement: a project's file listing (ROMACS_CHECKPOINT_DIR,
    // else gnu-lisp/) frozen and private, under length, a filter and string-match.
    fn walk(dir: &std::path::Path, out: &mut Vec<String>) {
        for entry in std::fs::read_dir(dir).unwrap().flatten() {
            let path = entry.path();
            if path.is_dir() { if entry.file_name() != ".git" { walk(&path, out); } } else { out.push(path.display().to_string()); }
        }
    }
    let root = std::env::var("ROMACS_CHECKPOINT_DIR").unwrap_or_else(|_| concat!(env!("CARGO_MANIFEST_DIR"), "/../../gnu-lisp").into());
    let mut files = Vec::new();
    walk(std::path::Path::new(&root), &mut files);
    files.sort();
    let mut vm = vm();
    let literal: Vec<String> = files.iter().map(|f| format!("{f:?}")).collect();
    eval(&mut vm, &format!("(setq files (list {}))", literal.join(" ")));
    let n = eval(&mut vm, "(length files)");
    let t = std::time::Instant::now();
    eval(&mut vm, "(setq frozen (freeze files))");
    let freeze = t.elapsed();
    let t = std::time::Instant::now();
    eval(&mut vm, "(setq thawed (thaw frozen))");
    let thaw = t.elapsed();
    for (name, form) in [("length", "(length X)"), ("filter", "(length (delq nil (mapcar (lambda (f) (and (string-match \"\\\\.el\\\\'\" f) f)) X)))"),
                         ("string-match", "(let ((n 0)) (dolist (f X) (when (string-match \"progmodes\" f) (setq n (1+ n)))) n)")] {
        for var in ["files", "frozen"] {
            let src = form.replace('X', var);
            let result = eval(&mut vm, &src);
            let t = std::time::Instant::now();
            for _ in 0..10 { eval(&mut vm, &src); }
            println!("CHECKPOINT {name} over {var} ({n} files): {:?} per pass -> {result}", t.elapsed() / 10);
        }
    }
    println!("CHECKPOINT freeze {n} files: {freeze:?}; thaw: {thaw:?}; bytes {}", vm.regions.iter().flatten().map(|r| r.bytes).sum::<usize>());
}
