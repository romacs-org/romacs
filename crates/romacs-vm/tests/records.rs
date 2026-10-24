use romacs_vm::{Fixed, Vm};

fn printed(vm: &mut Vm, source: &str) -> String {
    let value = vm
        .eval_source(&format!("(prin1-to-string {source})"))
        .unwrap();
    serde_json::from_str(&value.display().unwrap()).unwrap()
}

// All fixed expectations were observed with pinned GNU 45b0f7699d,
// executable SHA256 a31ba7aa12a7e4bbdda6ab9e19dab8338340f81f8100c33ac3843d10e176c8d8.

#[test]
fn record_construction() {
    assert_eq!(
        printed(
            &mut Vm::new(),
            r#"(list (record 'tag) (record nil 1 2) (make-record 'tag 3 'fill) (make-record 'tag 0 nil))"#
        ),
        r#"(#s(tag) #s(nil 1 2) #s(tag fill fill fill) #s(tag))"#
    );
}

#[test]
fn record_predicates() {
    assert_eq!(
        printed(
            &mut Vm::new(),
            r#"(list (recordp (record 'tag)) (recordp [tag]) (recordp nil) (vectorp (record 'tag)) (type-of (record 'tag 1)) (type-of (record 42)))"#
        ),
        r#"(t nil nil nil tag 42)"#
    );
}

#[test]
fn record_access_mutation() {
    assert_eq!(
        printed(
            &mut Vm::new(),
            r#"(let ((r (record 'tag 10 20))) (list (length r) (aref r 0) (aref r 2) (aset r 1 99) (aref r 1) (aset r 0 'new-tag) (type-of r)))"#
        ),
        r#"(3 tag 20 99 99 new-tag new-tag)"#
    );
}

#[test]
fn record_copy_equal() {
    assert_eq!(
        printed(
            &mut Vm::new(),
            r#"(let* ((x (list 7)) (r (record 'tag x)) (c (copy-sequence r))) (list (equal r c) (eq r c) (eq (aref r 1) (aref c 1)) (equal r [tag (7)]) (equal r (record 'other x)) (progn (aset c 1 9) (aref r 1))))"#
        ),
        r#"(t nil t nil nil (7))"#
    );
}

#[test]
fn record_printing_reading() {
    assert_eq!(
        printed(
            &mut Vm::new(),
            r#"(let* ((r (record 'tag 1 '(2 3))) (s (prin1-to-string r))) (list s (equal r (car (read-from-string s))) (recordp (car (read-from-string s))) (type-of (car (read-from-string s))) (length (car (read-from-string s)))))"#
        ),
        r##"("#s(tag 1 (2 3))" t t tag 3)"##
    );
}

#[test]
fn record_class_type() {
    assert_eq!(
        printed(
            &mut Vm::new(),
            r#"(let ((c (record 'cl-structure-class 'my-class))) (list (type-of (record c 1)) (type-of (record (record 'other 'my-class))) (type-of (record nil))))"#
        ),
        r#"(my-class my-class nil)"#
    );
}

#[test]
fn record_errors() {
    assert_eq!(
        printed(
            &mut Vm::new(),
            r#"(list (condition-case e (aref (record 'tag 1) -1) (error e)) (condition-case e (aref (record 'tag 1) 2) (error e)) (condition-case e (aset (record 'tag) 1 9) (error e)) (condition-case e (aref (record 'tag) 'bad) (error e)) (condition-case e (make-record 'tag -1 nil) (error e)) (condition-case e (make-record 'tag 'bad nil) (error e)))"#
        ),
        r#"((args-out-of-range #s(tag 1) -1) (args-out-of-range #s(tag 1) 2) (args-out-of-range #s(tag) 1) (wrong-type-argument fixnump bad) (wrong-type-argument wholenump -1) (wrong-type-argument wholenump bad))"#
    );
}

#[test]
fn record_callable() {
    assert_eq!(
        printed(
            &mut Vm::new(),
            r#"(list (fboundp 'record) (functionp #'record) (recordp (funcall #'record 'tag 1)) (recordp (funcall #'make-record 'tag 1 nil)) (funcall #'recordp (record 'tag)))"#
        ),
        r#"(t t t t t)"#
    );
}

#[test]
fn record_arity() {
    let source = "(list (condition-case e (apply #'record nil) (error (list (car e) (nth 2 e)))) (condition-case e (apply #'make-record '(tag 2)) (error (list (car e) (nth 2 e)))) (condition-case e (apply #'make-record '(tag 2 nil extra)) (error (list (car e) (nth 2 e)))) (condition-case e (apply #'recordp nil) (error (list (car e) (nth 2 e)))) (condition-case e (apply #'recordp '(nil nil)) (error (list (car e) (nth 2 e)))))";
    assert_eq!(
        printed(&mut Vm::new(), source),
        "((wrong-number-of-arguments 0) (wrong-number-of-arguments 2) (wrong-number-of-arguments 4) (wrong-number-of-arguments 0) (wrong-number-of-arguments 2))"
    );
}

#[test]
fn record_constructor_sharing() {
    assert_eq!(
        printed(
            &mut Vm::new(),
            "(let* ((x (list 7)) (r (make-record 'tag 2 x)) (q (record 'tag x x))) (list (length r) (eq (aref r 1) x) (eq (aref r 1) (aref r 2)) (eq (aref q 1) x) (eq (aref q 1) (aref q 2)) (progn (setcar x 9) (car (aref r 2)))))"
        ),
        "(3 t t t t 9)"
    );
}

#[test]
fn record_gc_traces_and_forwards_owned_slots() {
    let mut vm = Vm::new();
    let source = "(progn (list 11 12 13) (record (record 'cl-structure-class 'gc-class) (list 777) (vector (list 888))))";
    let check = "(list (type-of record-gc-root) (car (aref record-gc-root 1)) (car (aref (aref record-gc-root 2) 0)))";
    // Warm compiler constants before comparing post-release live heap sizes.
    let mut baseline = None;
    for _ in 0..32 {
        let root = vm.eval_source(source).unwrap().root();
        let collections = vm.heap_stats().collections;
        vm.collect_garbage();
        assert_eq!(vm.heap_stats().collections, collections + 1);
        vm.declare_special_rooted("record-gc-root", &root).unwrap();
        drop(root);
        assert_eq!(vm.eval_source(check).and_then(|value| value.display()), Ok("(gc-class 777 888)".into()), "RECORD_GC_FORWARD");
        vm.declare_special("record-gc-root", Fixed::NIL);
        vm.collect_garbage();
        let stats = vm.heap_stats();
        let live = (stats.live_objects, stats.live_bytes);
        if let Some(expected) = baseline {
            assert_eq!(live, expected, "record graph leaked");
        }
        baseline = Some(live);
    }
    println!("record retention: 32 cycles, live objects/bytes {:?}", baseline.unwrap());
}


// Read expectations directly from fresh GNU observations, not hand-written answers.
fn boundary_observation(name: &str) {
    let corpus: serde_json::Value = serde_json::from_str(include_str!(
        "../tools/primh/corpus/record-observations.json"
    )).unwrap();
    let row = corpus["observations"].as_array().unwrap().iter()
        .find(|row| row["name"] == name).unwrap();
    assert_eq!(printed(&mut Vm::new(), row["source"].as_str().unwrap()),
        row["expected"].as_str().unwrap(), "record boundary: {name}");
}

#[test]
fn record_max_slots() { boundary_observation("max_slots"); }
#[test]
fn record_too_many_slots() { boundary_observation("too_many_slots"); }
#[test]
fn record_short_tag_fallback() { boundary_observation("short_tag"); }
#[test]
fn record_bad_reader_syntax() { boundary_observation("bad_reader"); }

#[test]
fn record_and_reader_share_allocation_limit() {
    for count in [4095, 4096] {
        let source = format!("#s(tag{})", " nil".repeat(count - 1));
        let constructor = format!("(record 'tag{})", " nil".repeat(count - 1));
        let reader = format!("(car (read-from-string {}))", serde_json::to_string(&source).unwrap());
        for expression in [constructor, reader] {
            let observed = printed(&mut Vm::new(), &format!("(condition-case e (length {expression}) (error e))"));
            let expected = if count == 4095 { "4095" } else { "(error \"Attempt to allocate a record of 4096 slots; max is 4095\")" };
            assert_eq!(observed, expected, "RECORD_SHARED_LIMIT: {count}");
        }
    }
}
