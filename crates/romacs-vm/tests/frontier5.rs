//! The five measured warm-chain stops (`make-vector`, `vconcat`, `reverse`,
//! `propertize`, `secure-hash-algorithms`), asserted by VALUE.
//!
//! The primh corpora (`tools/primh/frontier5-specs`) pin only the CLASS of a
//! divergent row, so a pinned row keeps passing if a wrong value changes.
//! Every expected string below is GNU's own printed answer, recorded live from
//! `/usr/bin/emacs` (GNU 32.0.50, revision b28750b822dc) in both the
//! interpreted and byte-compiled regimes, which agreed on every row
//! (`tools/primh/logs/frontier5-gnu-values.txt`). Rows whose GNU answer cannot
//! be printed by VM2 (string text properties, `#<subr>`) are asserted through
//! `text-properties-at`/`car` instead of the printer.
use romacs_vm::Vm;

/// GNU's `primh` wrapper, evaluated in a fresh VM collecting at every
/// safepoint. Returns the printed `(ok VALUE)` / `(sig CONDITION)`.
fn gnu(source: &str) -> String {
    let mut vm = Vm::new();
    vm.set_collection_floor(1);
    let wrapped = format!("(prin1-to-string (condition-case e (list 'ok {source}) (error (list 'sig e))))");
    let printed = vm.eval_source(&wrapped).and_then(|v| v.display()).unwrap_or_else(|e| format!("VM-ERROR {e}"));
    serde_json::from_str::<String>(&printed).unwrap_or(printed)
}

fn check(rows: &[(&str, &str)]) {
    let bad: Vec<String> = rows
        .iter()
        .filter_map(|(source, expected)| {
            let actual = gnu(source);
            (actual != *expected).then(|| format!("{source}\n  GNU {expected}\n  VM2 {actual}"))
        })
        .collect();
    assert!(bad.is_empty(), "{} of {} rows differ from GNU:\n{}", bad.len(), rows.len(), bad.join("\n"));
}

#[test]
fn make_vector_matches_gnu_values() {
    check(&[
        ("(make-vector 3 'x)", "(ok [x x x])"),
        ("(make-vector 0 'x)", "(ok [])"),
        ("(length (make-vector 5 nil))", "(ok 5)"),
        ("(let ((v (make-vector 2 (list 1)))) (eq (aref v 0) (aref v 1)))", "(ok t)"),
        ("(let* ((a (make-vector 2 0)) (b (make-vector 2 0))) (aset a 0 9) (list a b (eq a b)))", "(ok ([9 0] [0 0] nil))"),
        ("(make-vector -1 nil)", "(sig (wrong-type-argument wholenump -1))"),
        ("(make-vector 1.5 nil)", "(sig (wrong-type-argument wholenump 1.5))"),
        ("(make-vector 'a nil)", "(sig (wrong-type-argument wholenump a))"),
        ("(car (condition-case e (make-vector most-positive-fixnum nil) (error e)))", "(ok error)"),
        ("(funcall #'make-vector 1 2)", "(ok [2])"),
    ]);
}

/// alloc.c:3536 `make_vector` -> `allocate_vectorlike` -> `memory_full`,
/// which signals `memory-signal-data`. VM2 has no such variable and signals
/// alloc.c:7508's initializer text; GNU's startup.el:1738 rewrites it to the
/// user's key binding (DIVERGENCES VM2-FR5-2).
#[test]
fn make_vector_impossible_length_signals_memory_full_text() {
    check(&[(
        "(condition-case e (make-vector most-positive-fixnum nil) (error e))",
        "(ok (error \"Memory exhausted--use M-x save-some-buffers then exit and restart Emacs\"))",
    )]);
}

#[test]
fn vconcat_matches_gnu_values() {
    check(&[
        ("(vconcat)", "(ok [])"),
        ("(vconcat '(1 2) [3] \"aλ\" nil)", "(ok [1 2 3 97 955])"),
        ("(vconcat (unibyte-string 255 65))", "(ok [255 65])"),
        ("(vconcat (string-to-multibyte (unibyte-string 255)))", "(ok [4194303])"),
        ("(vconcat '(1 . 2))", "(sig (wrong-type-argument listp 2))"),
        ("(vconcat '(1 2 . 3) 5)", "(sig (wrong-type-argument listp 3))"),
        ("(vconcat 5 '(1 . 2))", "(sig (wrong-type-argument sequencep 5))"),
        ("(vconcat [1] 'sym)", "(sig (wrong-type-argument sequencep sym))"),
        ("(let ((l (list 1 2))) (setcdr (cdr l) l) (condition-case e (vconcat l) (circular-list (eq (car (cdr e)) l))))", "(ok t)"),
        ("(let ((l (list 0 1 2))) (setcdr (cdr (cdr l)) (cdr l)) (condition-case e (vconcat [9] l) (circular-list (list (eq (car (cdr e)) l) (car (car (cdr e)))))))", "(ok (nil 2))"),
        ("(let ((v (vector 1))) (eq v (vconcat v)))", "(ok nil)"),
        ("(let* ((x (list 1)) (v (vconcat (list x)))) (eq (aref v 0) x))", "(ok t)"),
        ("(apply #'vconcat '((1) (2)))", "(ok [1 2])"),
    ]);
}

#[test]
fn reverse_matches_gnu_values() {
    check(&[
        ("(reverse nil)", "(ok nil)"),
        ("(reverse '(1 2 3))", "(ok (3 2 1))"),
        ("(reverse [1 2 3])", "(ok [3 2 1])"),
        ("(reverse \"abc\")", "(ok \"cba\")"),
        ("(reverse \"aλ雪\")", "(ok \"雪λa\")"),
        ("(multibyte-string-p (reverse (string-to-multibyte \"abc\")))", "(ok nil)"),
        ("(multibyte-string-p (reverse \"λ\"))", "(ok t)"),
        ("(let ((r (reverse (unibyte-string 255 1)))) (list (multibyte-string-p r) (aref r 0) (aref r 1)))", "(ok (nil 1 255))"),
        ("(let ((r (reverse (string-to-multibyte (unibyte-string 255 97))))) (list (multibyte-string-p r) (aref r 0) (aref r 1)))", "(ok (t 97 4194303))"),
        ("(reverse '(1 . 2))", "(sig (wrong-type-argument listp 2))"),
        ("(reverse '(1 2 . 3))", "(sig (wrong-type-argument listp 3))"),
        ("(reverse 5)", "(sig (wrong-type-argument sequencep 5))"),
        ("(reverse 'a)", "(sig (wrong-type-argument sequencep a))"),
        ("(let ((l (list 1 2))) (setcdr (cdr l) l) (condition-case e (reverse l) (circular-list (eq (car (cdr e)) l))))", "(ok t)"),
        ("(let ((l (list 1 2))) (reverse l) l)", "(ok (1 2))"),
        // Brent's tortoise teleports at step 4096: the signal names the
        // detecting tail inside the cycle (car 2), not the list.
        ("(let ((l (list 0 1 2))) (setcdr (cdr (cdr l)) (cdr l)) (condition-case e (reverse l) (circular-list (list (eq (car (cdr e)) l) (car (car (cdr e)))))))", "(ok (nil 2))"),
        ("(let ((s (copy-sequence \"ab\"))) (put-text-property 0 2 'face 'bold s) (list (text-properties-at 0 (reverse s)) (text-properties-at 0 s)))", "(ok (nil (face bold)))"),
        ("(list (funcall #'reverse '(1 2)) (apply #'reverse '([3 4])))", "(ok ((2 1) [4 3]))"),
    ]);
}

#[test]
fn propertize_matches_gnu_values() {
    check(&[
        ("(text-properties-at 0 (propertize \"abc\" 'face 'bold))", "(ok (face bold))"),
        ("(text-properties-at 2 (propertize \"abc\" 'face 'bold))", "(ok (face bold))"),
        ("(text-properties-at 0 (propertize \"a\" 'f 1 'g 2))", "(ok (f 1 g 2))"),
        ("(text-properties-at 0 (propertize \"a\" 'f 1 'f 2))", "(ok (f 1))"),
        ("(propertize \"a\")", "(ok \"a\")"),
        ("(let ((s (copy-sequence \"a\"))) (eq s (propertize s)))", "(ok nil)"),
        ("(let ((s (copy-sequence \"ab\"))) (propertize s 'face 'bold) (text-properties-at 0 s))", "(ok nil)"),
        ("(let ((s (copy-sequence \"ab\"))) (put-text-property 0 1 'x 1 s) (list (text-properties-at 0 (propertize s 'y 2)) (text-properties-at 1 (propertize s 'y 2))))", "(ok ((y 2 x 1) (y 2)))"),
        ("(let ((s (copy-sequence \"ab\"))) (put-text-property 0 1 'x 1 s) (text-properties-at 0 (propertize s 'x 2)))", "(ok (x 2))"),
        ("(car (condition-case e (propertize \"a\" 'face) (error e)))", "(ok wrong-number-of-arguments)"),
        ("(propertize 'a 'face 1)", "(sig (wrong-type-argument stringp a))"),
        ("(let ((e (condition-case e (apply #'propertize nil) (error e)))) (list (car e) (car (cdr (cdr e)))))", "(ok (wrong-number-of-arguments 0))"),
        ("(text-properties-at 1 (propertize \"λ雪\" 'face 'bold))", "(ok (face bold))"),
        ("(let ((p (propertize (unibyte-string 255) 'face 'bold))) (list (multibyte-string-p p) (aref p 0) (text-properties-at 0 p)))", "(ok (nil 255 (face bold)))"),
        ("(let ((p (propertize (symbol-name 'foo) 'face 'bold))) (list (substring-no-properties p) (text-properties-at 0 p)))", "(ok (\"foo\" (face bold)))"),
    ]);
}

/// editfns.c:3308: an even argument count signals with the SYMBOL
/// `propertize` and the count, not the subr object.
#[test]
fn propertize_even_arity_names_the_symbol() {
    check(&[("(propertize \"a\" 'face)", "(sig (wrong-number-of-arguments propertize 2))")]);
}

#[test]
fn secure_hash_algorithms_matches_gnu_values() {
    check(&[
        ("(secure-hash-algorithms)", "(ok (md5 sha1 sha224 sha256 sha384 sha512 sha3-224 sha3-256 sha3-384 sha3-512))"),
        ("(eq (secure-hash-algorithms) (secure-hash-algorithms))", "(ok nil)"),
    ]);
}

const JUNK: &str = "(let ((i 0)) (while (< i 2000) (copy-sequence \"junk\") (setq i (1+ i))))";

/// Hold RESULT in a global across a moving collection, read it back by value,
/// then drop it and require exactly OBJECTS objects / at least BYTES bytes to be
/// released. The payload is reachable ONLY through the primitive's result.
fn retention(make: &str, read: &str, expected: &str, objects: usize, bytes: usize) {
    let mut vm = Vm::new();
    vm.eval_source(&format!("(progn {JUNK} (setq fr5-keep (condition-case nil {make} (error nil))) {JUNK} nil)")).unwrap();
    // Read back after exactly one moving collection: a stale (unforwarded)
    // index must surface HERE as wrong contents, not in a later collection.
    let before = vm.heap_stats().collections;
    vm.collect_garbage();
    assert!(vm.heap_stats().collections > before, "{make}: no collection ran");
    let shown = vm.eval_source(&format!("(condition-case e {read} (error (list 'sig e)))")).and_then(|v| v.display()).unwrap_or_else(|e| format!("VM-ERROR {e}"));
    assert_eq!(shown, expected, "{make}: contents after a moving collection");
    vm.collect_garbage();
    let held = vm.heap_stats();
    vm.eval_source("(progn (setq fr5-keep nil) nil)").unwrap();
    vm.collect_garbage();
    vm.collect_garbage();
    let released = vm.heap_stats();
    let freed = (held.live_objects - released.live_objects, held.live_bytes - released.live_bytes);
    eprintln!("{make}: held {} objects / {} bytes; released {freed:?}", held.live_objects, held.live_bytes);
    assert_eq!(freed.0, objects, "{make}: objects live only through the result");
    assert!(freed.1 >= bytes, "{make}: payload bytes live only through the result: {freed:?}");
}

#[test]
fn make_vector_result_retains_its_init_across_collection() {
    retention("(make-vector 3 (make-string 4000 113))",
              "(list (length fr5-keep) (length (aref fr5-keep 2)) (aref (aref fr5-keep 2) 3999) (eq (aref fr5-keep 0) (aref fr5-keep 2)))",
              "(3 4000 113 t)", 2, 4000);
}

#[test]
fn vconcat_result_retains_its_elements_across_collection() {
    retention("(vconcat (list (make-string 4000 113)) [7])",
              "(list (length fr5-keep) (length (aref fr5-keep 0)) (aref (aref fr5-keep 0) 0) (aref fr5-keep 1))",
              "(2 4000 113 7)", 2, 4000);
}

#[test]
fn reverse_vector_result_retains_its_elements_across_collection() {
    retention("(reverse (vector 7 (make-string 4000 113)))",
              "(list (length fr5-keep) (length (aref fr5-keep 0)) (aref (aref fr5-keep 0) 0) (aref fr5-keep 1))",
              "(2 4000 113 7)", 2, 4000);
}

#[test]
fn propertize_result_retains_property_values_across_collection() {
    retention("(propertize \"ab\" 'face (make-string 4000 113))",
              "(let ((v (get-text-property 1 'face fr5-keep))) (list (length v) (aref v 0) (substring-no-properties fr5-keep)))",
              "(4000 113 \"ab\")", 2, 4000);
}

/// VM2-FR5-3: keys VM2 cannot represent are refused loudly, never merged.
/// These are VM2's answers, deliberately not GNU's (GNU accepts any key).
#[test]
fn propertize_unrepresentable_keys_are_refused_not_merged() {
    check(&[
        ("(propertize \"a\" 7 1)", "(sig (error \"VM2 string properties need an interned symbol key\" 7))"),
        ("(propertize \"a\" \"str\" 1)", "(sig (error \"VM2 string properties need an interned symbol key\" \"str\"))"),
        // `car` is interned in every VM, so a merge would be observable here.
        ("(propertize \"a\" (make-symbol \"car\") 1)", "(sig (error \"VM2 string properties need an interned symbol key\" car))"),
        ("(text-properties-at 0 (propertize \"a\" nil 1 :kw 2))", "(ok (nil 1 :kw 2))"),
    ]);
}

/// Every primh pin in the five corpora names a `/DIVERGENCES.md` row, with
/// the same id and class, and nothing else is in the ledger.
/// The divergence ledger, one file per divergence, read as one text.
fn divergences() -> String {
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../ledger/divergences");
    let mut paths: Vec<_> = std::fs::read_dir(&dir).unwrap().map(|e| e.unwrap().path()).collect();
    paths.sort();
    paths.iter().map(|p| std::fs::read_to_string(p).unwrap()).collect::<Vec<_>>().join("\n")
}

#[test]
fn frontier5_pins_are_exactly_the_ledger() {
    let ledger: serde_json::Value = serde_json::from_str(include_str!("../tools/primh/frontier5-ledger.json")).unwrap();
    let ledger = ledger["pins"].as_object().unwrap();
    let divergences = divergences();
    let mut pinned = std::collections::BTreeMap::new();
    for evidence in [
        include_str!("../tools/primh/evidence/make-vector.json"),
        include_str!("../tools/primh/evidence/vconcat.json"),
        include_str!("../tools/primh/evidence/reverse.json"),
        include_str!("../tools/primh/evidence/propertize.json"),
        include_str!("../tools/primh/evidence/secure-hash-algorithms.json"),
    ] {
        let ev: serde_json::Value = serde_json::from_str(evidence).unwrap();
        for (id, class) in ev["pinned"].as_object().unwrap() {
            pinned.insert(id.clone(), class.as_str().unwrap().to_owned());
        }
    }
    let ledgered: std::collections::BTreeMap<_, _> =
        ledger.iter().map(|(id, v)| (id.clone(), v["class"].as_str().unwrap().to_owned())).collect();
    assert_eq!(pinned, ledgered, "pins and ledger differ");
    for (id, v) in ledger {
        let row = v["row"].as_str().unwrap();
        assert!(divergences.contains(&format!("\n| {row} |")), "{id}: ledger row {row} is not in ledger/divergences");
    }
}
