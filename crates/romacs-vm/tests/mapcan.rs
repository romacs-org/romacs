use romacs_vm::Vm;

fn collecting_vm() -> Vm {
    let mut vm = Vm::new();
    vm.set_collection_floor(1);
    vm
}

fn show(vm: &mut Vm, source: &str) -> String {
    let wrapped = format!("(prin1-to-string (condition-case e (list 'ok {source}) (error (list 'sig e))))");
    let printed = vm.eval_source(&wrapped).and_then(|v| v.display()).unwrap_or_else(|e| format!("VM-ERROR {e}"));
    serde_json::from_str::<String>(&printed).unwrap_or(printed)
}

const CHURN: &str = "(let ((i 0)) (while (< i 12000) (setq i (1+ i)) (cons i i)))";
const DROP: &str = "(setq mf-ballast (cdr mf-ballast))";

const CLOSURES: &str = "(let (l (i 0)) (while (< i 64) (setq l (cons (let ((k i)) (lambda () (setq k (1+ k)))) l) i (1+ i))) l)";

fn chunks(n: u64) -> String {
    format!("(let (l (k 0)) (while (< k {n}) (setq l (cons (let (c (i 0)) (while (< i 300) \
             (setq c (cons (copy-sequence \"chunk-of-ballast\") c) i (1+ i))) c) l) k (1+ k))) l)")
}

fn walk(sequence: &str, elements: u64, map: &str, check: &str, expected: &str, held_objects: usize, held_bytes: usize) {
    let setup = format!("(progn (setq mf-ballast {}) (setq mf-seq {sequence}) (setq mf-sep (copy-sequence \"-\")) (setq mf-pad {}) \
                         (setq mf-pad-fns {CLOSURES}) (setq mf-out nil) (setq mf-shown nil) nil)",
                        chunks(elements), chunks(2 * elements));
    let mut control = collecting_vm();
    control.eval_source(&setup).unwrap();
    control.eval_source("(progn (setq mf-ballast nil) nil)").unwrap();
    control.collect_garbage();
    control.collect_garbage();
    let control = control.heap_stats();

    let mut vm = collecting_vm();
    vm.eval_source(&setup).unwrap();
    vm.collect_garbage();
    let before = vm.heap_stats().collections;
    vm.eval_source(&format!("(progn (setq mf-out (condition-case e {map} (error (list 'sig e)))) \
                             (setq mf-shown (prin1-to-string (condition-case e (list 'ok {check}) (error (list 'sig e))))) nil)")).unwrap();
    let during = vm.heap_stats().collections - before;
    let shown = vm.eval_source("mf-shown").and_then(|v| v.display()).unwrap_or_else(|e| format!("VM-ERROR {e}"));
    let shown = serde_json::from_str::<String>(&shown).unwrap_or(shown);
    assert_eq!(shown, expected, "{map}: identity and contents after moving collections inside the walk");
    assert!(during >= elements, "{map}: each callback must collect; saw {during}");
    assert_eq!(show(&mut vm, "mf-ballast"), "(ok nil)", "{map}: every callback dropped one ballast chunk");
    vm.eval_source("(progn (setq mf-shown nil) nil)").unwrap();
    vm.collect_garbage();
    let held = vm.heap_stats();
    vm.eval_source("(progn (setq mf-out nil) nil)").unwrap();
    vm.collect_garbage();
    vm.collect_garbage();
    let released = vm.heap_stats();
    eprintln!("{map}: control {}/{} held {}/{} released {}/{}", control.live_objects, control.live_bytes,
              held.live_objects, held.live_bytes, released.live_objects, released.live_bytes);
    assert_eq!((released.live_objects, released.live_bytes), (control.live_objects, control.live_bytes),
               "{map}: the walk retained objects or bytes after its result was dropped");
    assert_eq!(held.live_objects - control.live_objects, held_objects, "{map}: objects owned by the result");
    assert!(held.live_bytes - control.live_bytes >= held_bytes, "{map}: bytes owned by the result");
}

const STRINGS: &str = "(list (make-string 4000 ?a) (make-string 4000 ?b) (make-string 4000 ?c))";

#[test]
fn mapcan_accumulates_across_moving_collections_inside_every_callback() {
    walk(STRINGS, 3,
         &format!("(mapcan (lambda (x) {DROP} {CHURN} (list x (copy-sequence x))) mf-seq)"),
         "(list (length mf-out) (eq (nth 0 mf-out) (nth 0 mf-seq)) (eq (nth 2 mf-out) (nth 1 mf-seq))
                (eq (nth 4 mf-out) (nth 2 mf-seq)) (eq (nth 1 mf-out) (nth 0 mf-seq))
                (equal (nth 5 mf-out) (nth 2 mf-seq)) (aref (nth 1 mf-out) 3999) (aref (nth 3 mf-out) 0)
                (aref (nth 5 mf-out) 1234))",
         "(ok (6 t t t nil t 97 98 99))", 9, 12000);
}

fn check(rows: &[(&str, &str)]) {
    let bad: Vec<String> = rows
        .iter()
        .filter_map(|(source, expected)| {
            let actual = show(&mut collecting_vm(), source);
            (actual != *expected).then(|| format!("{source}\n  GNU {expected}\n  VM2 {actual}"))
        })
        .collect();
    assert!(bad.is_empty(), "{} of {} rows differ from GNU:\n{}", bad.len(), rows.len(), bad.join("\n"));
}

#[test]
fn mapcan_matches_gnu_values() {
    check(&[
        ("(condition-case e (mapcan #'list (car (cdr (make-keymap)))) (error (list (car e) (car (cdr e)))))", "(ok (wrong-type-argument listp))"),
        ("(let* ((l (list 1 2 3)) (calls nil)) (mapcan (lambda (x) (push x calls) (setcdr l nil) (list x)) l) calls)", "(ok (1))"),
        ("(mapcan (lambda (x) x) (list (list 1) (list 2) 3))", "(ok (1 2 . 3))"),
        ("(mapcan (lambda (x) x) (list (list 1) 2 (list 3)))", "(sig (wrong-type-argument consp 2))"),
        ("(let ((a (list 1)) (b (list 2))) (mapcan (lambda (x) x) (list a b)) a)", "(ok (1 2))"),
        ("(let ((a (list 1)) (b (list 2))) (list (mapcan (lambda (x) x) (list a nil b)) a))", "(ok ((1 2) (1 2)))"),
        ("(let ((l (list (list 1) (list 2) (list 3)))) (mapcan (lambda (x) (setcdr l nil) (copy-sequence x)) l))", "(ok (1))"),
        ("(mapcan #'list \"ab\")", "(ok (97 98))"),
        ("(mapcan #'list [])", "(ok nil)"),
        ("(let ((v (vector 1 2 3))) (mapcan (lambda (x) (aset v 2 'z) (list x)) v))", "(ok (1 2 z))"),
        ("(mapcan #'list (unibyte-string 255 65))", "(ok (255 65))"),
        ("(condition-case e (apply #'mapcan '(1)) (error e))", "(ok (wrong-number-of-arguments #<subr mapcan> 1))"),
    ]);
}
#[test]
fn a_string_cursor_left_inside_a_character_is_an_error_not_a_panic() {
    for (map, init) in [("mapcan", "(string ?a ?λ ?b)"), ("mapcan", "(copy-sequence \"ab\")")] {
        let source = format!("(let ((s {init})) (condition-case e ({map} (lambda (c) (aset s 0 ?λ) (list c)) s) (error (car (cdr e)))))");
        let shown = std::panic::catch_unwind(|| show(&mut collecting_vm(), &source)).unwrap_or_else(|_| "PANIC".into());
        assert_eq!(shown, "(ok \"String's bytes changed under a mapping function\")", "{source}");
    }
}

#[test]
fn mapcan_vector_accumulates_across_moving_collections_inside_every_callback() {
    walk("(vector (make-string 4000 ?a) (make-string 4000 ?b) (make-string 4000 ?c))", 3,
         &format!("(mapcan (lambda (x) {DROP} {CHURN} (list x (copy-sequence x))) mf-seq)"),
         "(list (length mf-out) (eq (nth 0 mf-out) (aref mf-seq 0)) (eq (nth 2 mf-out) (aref mf-seq 1))
                (eq (nth 4 mf-out) (aref mf-seq 2)) (eq (nth 1 mf-out) (aref mf-seq 0))
                (equal (nth 5 mf-out) (aref mf-seq 2)) (aref (nth 1 mf-out) 3999) (aref (nth 3 mf-out) 0)
                (aref (nth 5 mf-out) 1234))",
         "(ok (6 t t t nil t 97 98 99))", 9, 12000);
}


#[test]
fn mapcan_string_survives_moving_collections_inside_every_callback() {
    walk("(concat (make-string 2 ?λ) \"x\" (make-string 2 ?雪))", 5,
         &format!("(mapcan (lambda (c) {DROP} {CHURN} (list c)) mf-seq)"),
         "(list mf-out mf-seq)",
         "(ok ((955 955 120 38634 38634) \"λλx雪雪\"))", 5, 0);
}

#[test]
fn mapcan_pins_are_exactly_the_ledger() {
    let ledger: serde_json::Value = serde_json::from_str(include_str!("../tools/primh/mapcan-ledger.json")).unwrap();
    let ledger = ledger["pins"].as_object().unwrap();
    let ev: serde_json::Value = serde_json::from_str(include_str!("../tools/primh/evidence/mapcan.json")).unwrap();
    assert_eq!(ev["gnu"]["revision"], "45b0f7699df36b3849edc3bdef37d4cb9e75b9ea");
    let pinned: std::collections::BTreeMap<_, _> = ev["pinned"].as_object().unwrap().iter()
        .map(|(id, class)| (id.clone(), class.as_str().unwrap().to_owned())).collect();
    let ledgered: std::collections::BTreeMap<_, _> = ledger.iter()
        .map(|(id, v)| (id.clone(), v["class"].as_str().unwrap().to_owned())).collect();
    assert_eq!(pinned, ledgered, "pins and ledger differ");
    for (id, v) in ledger {
        let row = v["row"].as_str().unwrap();
        assert!(include_str!("../../../DIVERGENCES.md").contains(&format!("\n| {row} |")), "{id}: missing {row}");
    }
}
