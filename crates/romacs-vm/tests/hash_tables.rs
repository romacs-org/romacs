//! VM2 hash tables: live-GNU replay plus moving-collector contracts.
//!
//! The replay cases were recorded from GNU 32.0.50 by `tools/hash_oracle.py`
//! (fresh byte-compiled GNU per case). No GNU binary is needed here.
use romacs_vm::{Form, ImageRegion, Vm};
use sha2::{Digest, Sha256};
use std::path::Path;

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

fn replay(family: &str, expected_rows: usize) {
    let evidence: serde_json::Value =
        serde_json::from_str(include_str!("../evidence/hash-tables.json")).unwrap();
    assert_eq!(evidence["protocol"], "hash-tables-compiled-v1");
    assert_eq!(evidence["regime"], "GNU byte-compiled / VM2 compiled");
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let generator = std::fs::read(root.join("tools/hash_oracle.py")).unwrap();
    assert_eq!(
        evidence["generator_sha256"],
        hex(&Sha256::digest(generator))
    );
    let rows: Vec<_> = evidence["cases"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|row| row["family"] == family)
        .collect();
    assert_eq!(rows.len(), expected_rows, "{family} case count");
    let store = std::env::temp_dir().join(format!("vm2-s96-{family}-{}", std::process::id()));
    std::fs::create_dir_all(&store).unwrap();
    let mut mapped_cases = 0;
    for row in rows {
        assert_eq!(row["version"], "32.0.50");
        let source = row["source"].as_str().unwrap();
        let expected = row["expected"].as_str().unwrap().to_owned();
        let form = Form::from_json(row["transport"].clone()).unwrap();
        // Heap: dead prefix, forced compaction before and after every run.
        let mut vm = Vm::new();
        vm.set_collection_floor(1);
        vm.read_source("(garbage [1 2 3])").unwrap();
        let entry = vm.compile(&form).unwrap();
        vm.collect_garbage();
        let key = vm.publish_code_image(&store, entry, [0; 32]);
        for _ in 0..2 {
            let actual = vm.execute(entry).map(|value| value.root()).and_then(|value| {
                vm.collect_garbage();
                value.get(&vm)?.display()
            });
            assert_eq!(actual, Ok(expected.clone()), "heap: {source}");
        }
        // Mapped executable image: primitive indices must be stable ABI.
        if let Ok(key) = key {
            // SAFETY: the content-addressed store is private and never overwritten.
            let image = unsafe { ImageRegion::open(&store, &key) }.unwrap();
            let (mut mapped, entry) = Vm::from_code_image(image, [0; 32]).unwrap();
            mapped.set_collection_floor(1);
            let actual = mapped.execute(entry).map(|value| value.root()).and_then(|value| {
                mapped.collect_garbage();
                value.get(&mapped)?.display()
            });
            assert_eq!(actual, Ok(expected.clone()), "mapped: {source}");
            mapped_cases += 1;
        }
        // Source requests collect at their boundary.
        let mut source_vm = Vm::new();
        source_vm.set_collection_floor(1);
        source_vm
            .read_source(&format!("({})", "1 ".repeat(100)))
            .unwrap();
        let wrapped = format!("(condition-case e {source} (error (list 'signal e)))");
        let actual = source_vm
            .eval_source(&wrapped)
            .and_then(|value| value.display());
        assert_eq!(actual, Ok(expected.clone()), "source request: {source}");
    }
    assert!(
        mapped_cases > 0,
        "{family}: no case exercised a mapped image"
    );
    std::fs::remove_dir_all(store).unwrap();
}

#[test]
fn gnu_printed_representation() {
    replay("print", 11);
}
#[test]
fn gnu_make_hash_table_argument_order_and_errors() {
    replay("args", 17);
}
#[test]
fn gnu_hash_table_type_errors() {
    replay("type", 12);
}
#[test]
fn gnu_subr_arity() {
    replay("arity", 10);
}
#[test]
fn gnu_eq_eql_equal_tests() {
    replay("test", 27);
}
#[test]
fn gnu_slot_order_growth_and_clrhash() {
    replay("order", 10);
}
#[test]
fn gnu_mutation_during_maphash() {
    replay("mutate", 9);
}
#[test]
fn gnu_remaining_surface() {
    replay("surface", 14);
}

/// Printed result, or the error text: a failure surfaces at the caller's
/// target assertion rather than in this helper.
fn eval(vm: &mut Vm, source: &str) -> String {
    match vm.eval_source(source).and_then(|value| value.display()) {
        Ok(printed) => printed,
        Err(error) => format!("<error: {error}>"),
    }
}

/// Collect with nothing but globals live: request code and caches dropped.
fn settle(vm: &mut Vm) {
    vm.clear_source_cache();
    vm.collect_garbage();
}

#[test]
fn identity_keys_are_found_after_compaction_moves_them() {
    let mut vm = Vm::new();
    vm.set_gc_moving(true);
    eval(
        &mut vm,
        "(setq k (cons 1 2)) (setq v (vector 7)) (setq h (make-hash-table :test 'eq))",
    );
    // A dead prefix forces every surviving private object to move.
    eval(
        &mut vm,
        "(let ((i 0)) (while (< i 2000) (cons i nil) (vector i) (setq i (1+ i)))) (puthash k (list 'cons) h) (puthash v (list 'vector) h) (puthash h (list 'self) h) nil",
    );
    let k_before = format!("{:?}", vm.eval_source("k").unwrap());
    settle(&mut vm);
    let k_after = format!("{:?}", vm.eval_source("k").unwrap());
    assert_ne!(k_before, k_after, "the key must actually move");
    assert_eq!(
        eval(
            &mut vm,
            "(list (gethash k h) (gethash v h) (gethash h h) (gethash (cons 1 2) h) (hash-table-count h))"
        ),
        "((cons) (vector) (self) nil 3)"
    );
    // Insert/remove after the move must use the rebuilt index, not a stale chain.
    assert_eq!(
        eval(
            &mut vm,
            "(progn (remhash k h) (puthash k 'again h) (list (gethash k h) (gethash v h) (hash-table-count h)))"
        ),
        "(again (vector) 3)"
    );
}

#[test]
fn strong_tables_trace_and_forward_keys_and_values() {
    let mut vm = Vm::new();
    eval(&mut vm, "(setq h (make-hash-table :test 'equal))");
    eval(
        &mut vm,
        "(let ((i 0)) (while (< i 500) (cons i nil) (setq i (1+ i)))) (dotimes (i 50) (puthash (list i (copy-sequence \"key\")) (vector i (cons i i)) h)) nil",
    );
    settle(&mut vm);
    let live = vm.heap_stats().live_objects;
    assert!(
        live >= 50 * 5,
        "keys and values must be retained, got {live}"
    );
    assert_eq!(
        eval(
            &mut vm,
            "(let ((ok 0)) (dotimes (i 50) (let ((v (gethash (list i \"key\") h))) (when (and v (= (car (aref v 1)) i)) (setq ok (1+ ok))))) ok)"
        ),
        "50"
    );
    eval(&mut vm, "(setq h nil)");
    settle(&mut vm);
    assert_eq!(
        vm.heap_objects(),
        0,
        "an unreachable table releases its graph"
    );
}

#[test]
fn weak_key_entries_die_with_their_keys_only() {
    let mut vm = Vm::new();
    eval(
        &mut vm,
        "(setq h (make-hash-table :test 'eq :weakness 'key)) (setq k (list 'kept)) (puthash (list 'dead) (list 'value) h) (puthash k (list 'value) h) (puthash 'symbol (list 'immortal) h) (puthash 7 (list 'fixnum) h) nil",
    );
    assert_eq!(eval(&mut vm, "(hash-table-count h)"), "4");
    settle(&mut vm);
    assert_eq!(
        eval(
            &mut vm,
            "(list (hash-table-count h) (gethash k h) (gethash 'symbol h) (gethash 7 h))"
        ),
        "(3 (value) (immortal) (fixnum))"
    );
    eval(&mut vm, "(setq k nil)");
    settle(&mut vm);
    assert_eq!(eval(&mut vm, "(hash-table-count h)"), "2");
    // The freed slots are reused; the table stays consistent.
    assert_eq!(
        eval(
            &mut vm,
            "(progn (puthash 'x 1 h) (puthash 'y 2 h) (list (hash-table-count h) (gethash 'x h) (gethash 'y h) (gethash 7 h)))"
        ),
        "(4 1 2 (fixnum))"
    );
}

#[test]
fn every_weakness_follows_gnu_keep_rule() {
    // E1 live key/dead value, E2 dead key/live value, E3 both live,
    // E4 both dead, E5 immortal symbol key and value.
    for (weakness, kept) in [
        ("nil", "(5 (fresh) v3 sym)"),
        ("'key", "(3 (fresh) v3 sym)"),
        ("'value", "(3 nil v3 sym)"),
        ("'key-or-value", "(4 (fresh) v3 sym)"),
        ("t", "(2 nil v3 sym)"),
    ] {
        let mut vm = Vm::new();
        eval(
            &mut vm,
            &format!(
                "(setq h (make-hash-table :test 'eq :weakness {weakness}))
                 (setq k1 (list 'k1) v2 (list 'v2) k3 (list 'k3) v3 (list 'v3))
                 (puthash k1 (list 'fresh) h) (puthash (list 'k2) v2 h) (puthash k3 v3 h)
                 (puthash (list 'k4) (list 'v4) h) (puthash 'sym 'sym h) nil"
            ),
        );
        settle(&mut vm);
        assert_eq!(
            eval(
                &mut vm,
                "(list (hash-table-count h) (gethash k1 h) (car (gethash k3 h)) (gethash 'sym h))"
            ),
            kept,
            "weakness {weakness}"
        );
    }
}

#[test]
fn weak_keys_are_ephemerons_through_a_fixpoint() {
    let mut vm = Vm::new();
    // Entry B's key is reachable only through entry A's value. B occupies the
    // earlier slot, so a single sweep in slot order would wrongly drop it.
    eval(
        &mut vm,
        "(setq h (make-hash-table :test 'eq :weakness 'key)) (setq a (list 'a))
         (let ((b (list 'b))) (puthash b (list 'b-value) h) (puthash a (list b) h))
         (let ((c (list 'c))) (puthash c (list c 'self-reference) h)) nil",
    );
    settle(&mut vm);
    assert_eq!(
        eval(
            &mut vm,
            "(list (hash-table-count h) (gethash (car (gethash a h)) h))"
        ),
        "(2 (b-value))",
        "a value referencing its own key must not keep the entry alive"
    );
    eval(&mut vm, "(setq a nil)");
    settle(&mut vm);
    assert_eq!(eval(&mut vm, "(hash-table-count h)"), "0");
    eval(&mut vm, "(setq h nil)");
    settle(&mut vm);
    assert_eq!(vm.heap_objects(), 0);
}

#[test]
fn maphash_survives_collections_inside_its_body() {
    let mut vm = Vm::new();
    vm.set_collection_floor(1);
    eval(
        &mut vm,
        "(setq h (make-hash-table :test 'eq)) (setq keys nil) (dotimes (i 64) (let ((k (cons i nil))) (push k keys) (puthash k (vector i) h)))",
    );
    let before = vm.heap_stats().collections;
    assert_eq!(
        eval(
            &mut vm,
            "(let ((sum 0) (seen 0)) (maphash (lambda (k v) (dotimes (j 300) (cons j (vector j))) (when (eq (gethash k h) v) (setq seen (1+ seen))) (setq sum (+ sum (car k) (aref v 0)))) h) (list sum seen (hash-table-count h)))"
        ),
        "(4032 64 64)"
    );
    assert!(
        vm.heap_stats().collections > before,
        "the body must have collected while maphash held slot indices"
    );
}

/// Recorded divergences (GNU-DIVERGENCES.md H1-H3): each fails loudly or is
/// pinned to its exact VM2 answer, so a change in either direction is seen.
#[test]
fn divergences_are_loud_and_pinned() {
    let mut vm = Vm::new();
    // H1: GNU calls FUNCTION through an indirect maphash; VM2 cannot.
    assert_eq!(
        eval(
            &mut vm,
            "(let ((h (make-hash-table))) (puthash 'a 1 h) (condition-case e (apply #'maphash (list #'ignore h)) (error e)))"
        ),
        "(error \"VM2 cannot call maphash indirectly on a non-empty table\" ignore)"
    );
    // H2: GNU accepts a test installed by define-hash-table-test.
    assert_eq!(
        eval(
            &mut vm,
            "(progn (define-hash-table-test 'my-test #'eq #'ignore) (condition-case e (make-hash-table :test 'my-test) (error e)))"
        ),
        "(error \"VM2 does not support user-defined hash table tests\" my-test)"
    );
    // H3: GNU compares closures element-wise; VM2 requires the same lambda site.
    assert_eq!(
        eval(
            &mut vm,
            "(let ((h (make-hash-table :test 'equal)) (make (lambda () (lambda (x) x)))) (puthash (lambda (x) x) 'first h) (list (gethash (lambda (x) x) h) (progn (puthash (funcall make) 'site h) (gethash (funcall make) h))))"
        ),
        "(nil site)"
    );
}
