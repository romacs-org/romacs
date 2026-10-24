use romacs_vm::{Fixed, Vm};

const BOUNDS: [i64; 10] = [
    -(1 << 61),
    -(1 << 61) + 1,
    -(1 << 60) - 1,
    -(1 << 60),
    -1,
    0,
    (1 << 60) - 1,
    1 << 60,
    (1 << 61) - 2,
    (1 << 61) - 1,
];

#[test]
fn widened_fixnums_are_injective_and_roundtrip() {
    let values: Vec<_> = BOUNDS.iter().map(|&n| Fixed::integer(n)).collect();
    assert!(
        values.iter().all(Result::is_ok),
        "S109 all GNU fixnums must be immediate"
    );
    for (i, v) in values.iter().enumerate() {
        let v = v.as_ref().unwrap();
        assert_eq!(v.as_integer(), Ok(BOUNDS[i]), "S109 decode boundary");
        for (j, w) in values.iter().enumerate() {
            assert_eq!(v == w.as_ref().unwrap(), i == j, "S109 no aliases");
        }
        let mut vm = Vm::new();
        let source = BOUNDS[i].to_string();
        let actual = vm.eval_source(&source).and_then(|v| v.display());
        assert_eq!(actual, Ok(source.clone()), "S109 reader/printer boundary");
        let printed = actual.unwrap();
        let reread = vm.read_source(&printed).unwrap().unwrap().0;
        assert_eq!(
            reread, *v,
            "S109 printed integer reads to identical immediate"
        );
        assert_eq!(
            reread.as_integer(),
            Ok(BOUNDS[i]),
            "S109 read-back boundary"
        );
    }
    for n in [-(1 << 61) - 1, 1 << 61, i64::MIN, i64::MAX] {
        assert!(Fixed::integer(n).is_err(), "S109 rejects bignum boundary");
    }
}

#[test]
fn widened_fast_arithmetic_and_identity() {
    for (source, expected) in [
        ("(1+ 1152921504606846975)", "1152921504606846976"),
        ("(1- -1152921504606846976)", "-1152921504606846977"),
        ("(+ 1152921504606846976 1)", "1152921504606846977"),
        ("(* 1152921504606846976 1)", "1152921504606846976"),
        ("(% -2305843009213693952 3)", "-2"),
        ("(< -2305843009213693952 2305843009213693951)", "t"),
        ("(eq (+ 1152921504606846976 1) 1152921504606846977)", "t"),
        ("(eq (- -1152921504606846976 1) -1152921504606846977)", "t"),
    ] {
        let mut vm = Vm::new();
        let actual = vm
            .compile_source(source)
            .and_then(|entry| vm.execute(entry))
            .and_then(|v| v.display());
        assert_eq!(actual, Ok(expected.into()), "S109 fast arithmetic {source}");
    }
}

#[test]
fn fixnum_constants_and_predicates() {
    let mut vm = Vm::new();
    let actual = vm
        .eval_source("(list most-positive-fixnum most-negative-fixnum)")
        .and_then(|v| v.display());
    assert_eq!(
        actual,
        Ok("(2305843009213693951 -2305843009213693952)".into()),
        "S109 GNU fixnum constants"
    );
    for n in BOUNDS {
        let source = format!("(list (integerp {n}) (fixnump {n}) (bignump {n}))");
        let actual = vm.eval_source(&source).and_then(|v| v.display());
        assert_eq!(
            actual,
            Ok("(t t nil)".into()),
            "S109 integer predicates {n}"
        );
    }
    for source in ["nil", "t", "1.0", "\"1\"", "'(1)", "[1]"] {
        let source = format!("(list (integerp {source}) (fixnump {source}) (bignump {source}))");
        let actual = vm.eval_source(&source).and_then(|v| v.display());
        assert_eq!(
            actual,
            Ok("(nil nil nil)".into()),
            "S109 noninteger predicates"
        );
    }
    for name in ["most-positive-fixnum", "most-negative-fixnum"] {
        for body in [
            format!("(setq {name} 0)"),
            format!("(let (({name} 0)) {name})"),
        ] {
            let source = format!("(condition-case nil {body} (setting-constant 'constant))");
            let actual = vm.eval_source(&source).and_then(|v| v.display());
            assert_eq!(
                actual,
                Ok("constant".into()),
                "S109 fixnum bounds are constants"
            );
        }
    }
}

#[test]
fn widened_weak_keys_survive_real_compaction() {
    for test in ["eq", "eql", "equal"] {
        for n in [-(1i64 << 61), -(1 << 60) - 1, 1 << 60, (1 << 61) - 1] {
            let mut vm = Vm::new();
            vm.read_source("(\"dead\")").unwrap();
            let source = format!(
                "(setq s109-table (make-hash-table :test '{test} :weakness 'key)) (puthash (+ {n} 0) (cons 42 nil) s109-table) s109-table"
            );
            let table = vm.eval_source(&source);
            assert!(
                table.is_ok(),
                "S109 weak table accepts new immediate key: {table:?}"
            );
            let table = table.unwrap();
            let table_bits = format!("{table:?}");
            let table = table.root();
            let lookup = format!("(gethash (+ {n} 0) s109-table)");
            let old_value_identity = vm
                .eval_source(&lookup)
                .map(|value| format!("{value:?}"))
                .unwrap();
            vm.clear_source_cache();
            vm.collect_garbage();
            assert_ne!(format!("{:?}", table.get(&vm).unwrap()), table_bits, "S109 actual table relocation");
            assert!(vm.heap_stats().reclaimed > 0, "S109 actual reclamation");
            vm.collect_garbage();
            let source = format!(
                "(list (hash-table-count s109-table) (car (gethash (+ {n} 0) s109-table)))"
            );
            let actual = vm.eval_source(&source).and_then(|v| v.display());
            assert_eq!(
                actual,
                Ok("(1 42)".into()),
                "S109 weak immediate retention and lookup after move {test} {n}"
            );
            let new_value_identity = vm
                .eval_source(&lookup)
                .map(|value| format!("{value:?}"))
                .unwrap();
            assert_ne!(
                old_value_identity, new_value_identity,
                "S109 weak value actually moved"
            );
        }
    }
}

#[test]
fn widened_captured_cells_and_literal_roots_survive_collection() {
    let mut vm = Vm::new();
    vm.set_collection_floor(1);
    let result = vm.eval_source("(defun s109-make (x) (lambda () (setq x (+ x 1)))) (setq s109-dead (s109-make 0)) (setq s109-dead nil) (setq s109-live (s109-make 1152921504606846976)) (let ((i 0)) (while (< i 25000) (cons i nil) (setq i (1+ i)))) (funcall s109-live) (funcall s109-live)");
    assert_eq!(
        result.and_then(|v| v.display()),
        Ok("1152921504606846978".into()),
        "S109 relocated cell and wide immediate"
    );
    assert!(
        vm.heap_stats().collections > 1,
        "S109 automatic collections ran"
    );
    assert!(vm.heap_stats().reclaimed > 0, "S109 garbage was reclaimed");
    vm.clear_source_cache();
    vm.collect_garbage();
    let result = vm
        .eval_source("(funcall s109-live)")
        .and_then(|v| v.display());
    assert_eq!(
        result,
        Ok("1152921504606846979".into()),
        "S109 idle closure cell survived forwarding"
    );
}

#[test]
fn widened_code_image_roundtrip() {
    let mut vm = Vm::new();
    let entry =
        vm.compile_source("(list -2305843009213693952 1152921504606846976 2305843009213693951)");
    assert!(
        entry.is_ok(),
        "S109 image compiles wide constants: {entry:?}"
    );
    let store = std::env::temp_dir().join(format!("s109-fixnums-{}", std::process::id()));
    std::fs::create_dir(&store).unwrap();
    let key = vm
        .publish_code_image(&store, entry.unwrap(), [109; 32])
        .unwrap();
    // The private content-addressed store is never overwritten.
    let image = unsafe { romacs_vm::ImageRegion::open(&store, &key) }.unwrap();
    let (mut restored, entry) = Vm::from_code_image(image, [109; 32]).unwrap();
    restored.collect_garbage();
    let result = restored.execute(entry).and_then(|v| v.display());
    assert_eq!(
        result,
        Ok("(-2305843009213693952 1152921504606846976 2305843009213693951)".into()),
        "S109 image integer tags"
    );
    std::fs::remove_dir_all(store).unwrap();
}
