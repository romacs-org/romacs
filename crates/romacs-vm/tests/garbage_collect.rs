use romacs_vm::Vm;

#[test]
fn garbage_collect_is_callable_and_really_collects() {
    let mut vm = Vm::new();
    assert_eq!(vm.heap_stats().live_objects, 0);
    vm.set_collection_floor(1_000_000);
    vm.eval_source("(let ((i 0)) (while (< i 100) (cons i nil) (setq i (1+ i))))")
        .unwrap();
    let before = vm.heap_stats();
    let result = vm.eval_source("(funcall 'garbage-collect)");
    assert!(result.is_ok(), "GC_CALLABLE: {result:?}");
    drop(result);
    let after = vm.heap_stats();
    assert!(after.collections > before.collections, "GC_PERFORMED");
    assert!(after.reclaimed > before.reclaimed, "GC_RECLAIMED");
    assert_eq!(
        vm.eval_source("(interactive-form 'garbage-collect)")
            .unwrap()
            .display()
            .unwrap(),
        "(interactive \"\")"
    );
}

#[test]
fn garbage_collect_rows_and_absent_pool_are_honest() {
    let mut vm = Vm::new();
    vm.install_lisp_prelude().unwrap();
    let result = vm.eval_source("(mapcar 'car (garbage-collect))");
    assert!(result.is_ok(), "GC_ROWS: {result:?}");
    assert_eq!(
        result.unwrap().display().unwrap(),
        "(conses symbols strings string-bytes vectors vector-slots floats buffers)",
        "GC_NO_FABRICATED_INTERVAL_POOL"
    );
    assert_eq!(vm.eval_source("(mapcar (lambda (row) (list (car row) (length row) (mapcar 'integerp (cdr row)))) (garbage-collect))").unwrap().display().unwrap(), "((conses 4 (t t t)) (symbols 4 (t t t)) (strings 4 (t t t)) (string-bytes 3 (t t)) (vectors 3 (t t)) (vector-slots 4 (t t t)) (floats 4 (t t t)) (buffers 3 (t t)))", "GC_EXACT_ROW_SHAPE_AND_INTEGER_TYPES");
    assert_eq!(
        vm.eval_source("(condition-case e (garbage-collect 1) (error e))")
            .unwrap()
            .display()
            .unwrap(),
        "(wrong-number-of-arguments #<subr garbage-collect> 1)",
        "GC_EXACT_ARITY"
    );
}

#[test]
fn garbage_collect_forwards_live_call_state() {
    let mut vm = Vm::new();
    vm.set_collection_floor(1_000_000);
    vm.eval_source("(let ((i 0)) (while (< i 100) (cons i nil) (setq i (1+ i))))")
        .unwrap();
    let result =
        vm.eval_source("(let ((a (list 11 22)) (b (vector 33 44))) (garbage-collect) (list a b))");
    assert!(result.is_ok(), "GC_LIVE_CALL: {result:?}");
    assert_eq!(result.unwrap().display().unwrap(), "((11 22) [33 44])");
}

/// GNU's inhibited `garbage-collect' returns nil (alloc.c:6044). Romacs
/// inhibits where a native frame holds untraced Values (DIVERGENCES
/// VM2-GC-INHIBITED): a watcher a primitive's write runs, and macro
/// expansion inside an open compiler builder. A pacing floor does not
/// inhibit an explicit collection.
#[test]
fn garbage_collect_obeys_existing_inhibition() {
    let mut vm = Vm::new();
    // Counts collections, so no forced ones (`ROMACS_GC_STRESS`).
    vm.set_gc_stress(0);
    vm.set_collection_floor(usize::MAX);
    let eval = |vm: &mut Vm, source: &str| vm.eval_source(source).unwrap().display().unwrap();
    assert_ne!(eval(&mut vm, "(garbage-collect)"), "nil", "GC_PACING_IS_NOT_INHIBITION");
    eval(&mut vm, "(defvar gi-v nil)");
    eval(&mut vm, "(add-variable-watcher 'gi-v (lambda (&rest _) (setq gi-r (garbage-collect))))");
    eval(&mut vm, "(defmacro gi-m () (list 'quote (garbage-collect)))");
    let before = vm.heap_stats().collections;
    assert_eq!(eval(&mut vm, "(progn (set 'gi-v 1) gi-r)"), "nil", "GC_INHIBITED_RESULT");
    eval(&mut vm, "(defun gi-f () (gi-m))");
    assert_eq!(eval(&mut vm, "(gi-f)"), "nil", "GC_INHIBITED_RESULT");
    assert_eq!(vm.heap_stats().collections, before, "GC_INHIBITED_COUNT");
    // The same watcher collects from a compiled `setq', whose engine suspends.
    assert_ne!(eval(&mut vm, "(progn (setq gi-v 2) gi-r)"), "nil", "GC_WATCHED_SETQ");
}

fn churn(vm: &mut Vm, conses: u32) -> u64 {
    let before = vm.heap_stats().collections;
    vm.eval_source(&format!("(let ((i 0)) (while (< i {conses}) (cons i nil) (setq i (1+ i))))")).unwrap();
    vm.heap_stats().collections - before
}

/// GNU's pacing variables (alloc.c:5740-5809) decide when allocation
/// collects, read live: a binding of either takes effect at once.
#[test]
fn gc_cons_threshold_and_percentage_pace_collection() {
    let mut vm = Vm::new();
    vm.set_gc_stress(0);
    let eval = |vm: &mut Vm, source: &str| vm.eval_source(source).unwrap().display().unwrap();
    assert_eq!(eval(&mut vm, "(list gc-cons-threshold gc-cons-percentage)"), "(800000 0.1)", "GNU's defaults");
    // 160000 bytes is 10000 objects; nil drops the proportional term.
    vm.eval_source("(setq gc-cons-threshold 160000 gc-cons-percentage nil)").unwrap();
    let plain = churn(&mut vm, 200_000);
    assert!(plain >= 10, "a 10000-object threshold collects: {plain}");
    let before = vm.heap_stats().collections;
    let restored = eval(&mut vm, "(progn (let ((gc-cons-threshold most-positive-fixnum)) (let ((i 0)) (while (< i 200000) (cons i nil) (setq i (1+ i))))) gc-cons-threshold)");
    assert_eq!(restored, "160000", "the binding is undone");
    // Only the request boundary after the binding ends may collect, where
    // the same churn collected ten times above.
    assert!(vm.heap_stats().collections <= before + 1, "a raised threshold defers collection");
    // A proportional term above the threshold paces instead, in proportion:
    // 0.12 waits 1.2 traced heaps of a live heap made big enough to matter.
    vm.eval_source("(setq gc-cons-percentage 0.12 gr-live (make-list 40000 nil))").unwrap();
    let proportional = churn(&mut vm, 2_000_000);
    vm.eval_source("(setq gc-cons-percentage 0.5)").unwrap();
    let raised = churn(&mut vm, 2_000_000);
    assert!(raised * 3 <= proportional, "a raised percentage defers collection: {raised} vs {proportional}");
    // Absurd settings are held at the ceiling: they defer, and it still
    // happens, the same however large.
    let mut deferred = vec![];
    for p in ["1.8", "2.0", "1e9"] {
        vm.eval_source(&format!("(setq gc-cons-percentage {p})")).unwrap();
        deferred.push(churn(&mut vm, 2_000_000));
    }
    vm.eval_source("(setq gc-cons-percentage 0.1)").unwrap();
    assert!(deferred.iter().all(|&n| n >= 1), "the ceiling still collects: {deferred:?}");
    // The same ceiling, give or take where each churn starts in a cycle.
    assert!(deferred.iter().max().unwrap() - deferred.iter().min().unwrap() <= 1, "the ceiling holds: {deferred:?}");
    // Non-integer thresholds are refused, as GNU's DEFVAR_INT is.
    assert_eq!(eval(&mut vm, "(condition-case e (setq gc-cons-threshold 1.5) (error (car e)))"), "wrong-type-argument");
    // An embedder's floor replaces both variables.
    vm.set_collection_floor(1);
    assert!(churn(&mut vm, 200_000) >= 2, "a floor of 1 collects as the heap doubles");
}

// generator.el's `iter-defun' iterators close themselves through a
// finalizer (Eshell's `for' and subcommands are iterators).
#[test]
fn finalizers_run_once_after_the_collection_that_finds_them_unreachable() {
    let setup = "(progn (defvar fin-log nil) \
        (fset 'fin-make (lambda (tag) (make-finalizer (lambda () (setq fin-log (cons tag fin-log)))) nil)) \
        (fin-make 0) (fin-make 1) (fin-make 2) \
        (defvar fin-kept (make-finalizer (lambda () (setq fin-log (cons 'kept fin-log))))) \
        (fset 'fin-boom (lambda () (make-finalizer (lambda () (setq fin-log (cons 'boom fin-log)) (error \"boom\"))) nil)) \
        (fin-boom) t)";
    let check = "(list (progn (garbage-collect) (sort (mapcar (lambda (x) (format \"%s\" x)) fin-log) 'string-lessp)) \
              (progn (setq fin-log nil) (garbage-collect) fin-log) \
              (type-of fin-kept) (prin1-to-string fin-kept) \
              (condition-case e (make-finalizer 1) (error e)))";
    let expected = "((\"0\" \"1\" \"2\" \"boom\") nil finalizer \"#<finalizer>\" (wrong-type-argument functionp 1))";
    let oracle = std::env::var("ROMACS_GNU_ORACLE").expect("pinned GNU oracle");
    let output = std::process::Command::new(oracle)
        .args(["-Q", "--batch", "--eval", setup, "--eval", &format!("(prin1 {check})")]).output().unwrap();
    assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
    assert_eq!(String::from_utf8(output.stdout).unwrap(), expected, "pinned GNU");
    let mut vm = Vm::new();
    vm.install_lisp_prelude().unwrap();
    vm.eval_source(setup).unwrap();
    let printed = vm.eval_source(&format!("(prin1-to-string {check})")).unwrap().display().unwrap();
    assert_eq!(printed, serde_json::to_string(expected).unwrap(), "candidate");
}

/// garbage_collect ends with its finalizers and `post-gc-hook' whatever
/// asked for it: an automatic collection in a running loop too, the hook
/// seeing `gc-elapsed' already counted, and a `throw' leaves through it.
#[test]
fn automatic_collections_run_finalizers_and_post_gc_hook() {
    let setup = "(progn (defvar hook-runs 0) (defvar hook-elapsed nil) (defvar fin-ran nil) \
        (fset 'hook-note (lambda () (setq hook-runs (1+ hook-runs) hook-elapsed gc-elapsed))) \
        (fset 'fin-make (lambda () (make-finalizer (lambda () (setq fin-ran t))) nil)) \
        (fset 'hook-churn (lambda (n) (let ((i 0)) (while (< i n) (cons i nil) (setq i (1+ i)))))) t)";
    let check = "(let ((done gcs-done) (elapsed gc-elapsed)) \
          (fin-make) \
          (setq post-gc-hook '(hook-note)) \
          (hook-churn 2000000) \
          (setq post-gc-hook nil) \
          (let ((ran hook-runs) (grown (- gcs-done done))) \
            (list (> ran 0) (<= (- grown 1) ran) (<= ran grown) fin-ran (> hook-elapsed elapsed) \
                  (catch 'out \
                    (setq post-gc-hook (list (lambda () (throw 'out 'thrown)))) \
                    (unwind-protect (while t (cons 1 nil)) \
                      (setq post-gc-hook nil))))))";
    let expected = "(t t t t t thrown)";
    let oracle = std::env::var("ROMACS_GNU_ORACLE").expect("pinned GNU oracle");
    let output = std::process::Command::new(oracle)
        .args(["-Q", "--batch", "--eval", setup, "--eval", &format!("(prin1 {check})")]).output().unwrap();
    assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
    assert_eq!(String::from_utf8(output.stdout).unwrap(), expected, "pinned GNU");
    let mut vm = Vm::new();
    vm.install_lisp_prelude().unwrap();
    vm.eval_source(setup).unwrap();
    let printed = vm.eval_source(&format!("(prin1-to-string {check})")).unwrap().display().unwrap();
    assert_eq!(printed, serde_json::to_string(expected).unwrap(), "candidate");
}

/// A run of finalizers belongs to the collection that doomed them: a poll
/// that collected nothing, inside a long one, starts no other, whichever way
/// the run began, so each ends before the next starts, and the hook comes
/// last. (A collection inside one does run its own, as GNU's.) A `throw' out
/// of one leaves the rest doomed for the next run, as GNU's list does.
#[test]
fn a_long_finalizer_keeps_its_run_in_order() {
    let setup = "(progn (defvar fin-log nil) \
        (fset 'fin-spin (lambda (n) (let ((i 0)) (while (< i n) (setq i (1+ i)))))) \
        (fset 'fin-note (lambda (tag) (setq fin-log (cons tag fin-log)))) \
        (fset 'fin-sort (lambda (log) (sort (mapcar 'symbol-name log) 'string-lessp))) \
        (fset 'fin-whole (lambda () (eq (car (cdr (memq 'a-end fin-log))) 'a-begin))) t)";
    // Each in a top-level form of its own: a register of the running one
    // keeps what it held alive (VM2-GC-FINALIZER).
    let long = "(progn \
        (make-finalizer (lambda () (fin-note 'a-begin) (fin-spin 300000) (fin-note 'a-end))) \
        (make-finalizer (lambda () (fin-note 'b))) nil)";
    let automatic = "(progn (setq post-gc-hook '((lambda () (fin-note 'hook)))) \
          (let ((i 0)) (while (and (< i 100000000) (not (memq 'hook fin-log))) (cons i nil) (setq i (1+ i)))) \
          (setq post-gc-hook nil) \
          (prog1 (list (eq (car fin-log) 'hook) (fin-sort fin-log) (fin-whole)) \
            (setq fin-log nil)))";
    let explicit = "(progn (setq post-gc-hook '((lambda () (fin-note 'hook)))) \
          (garbage-collect) \
          (setq post-gc-hook nil) \
          (prog1 (list (eq (car fin-log) 'hook) (fin-sort fin-log) (fin-whole)) \
            (setq fin-log nil)))";
    let throwing = "(progn \
        (make-finalizer (lambda () (fin-note 'x))) \
        (make-finalizer (lambda () (throw 'out 'left))) \
        (make-finalizer (lambda () (fin-note 'y))) nil)";
    let thrown = "(let ((caught (catch 'out (garbage-collect) 'none))) \
          (garbage-collect) (list caught (fin-sort fin-log)))";
    let ran = "(t (\"a-begin\" \"a-end\" \"b\" \"hook\") t)";
    let expected = [ran, ran, "(left (\"x\" \"y\"))"];
    let oracle = std::env::var("ROMACS_GNU_ORACLE").expect("pinned GNU oracle");
    let printed = |form: &str| format!("(prin1 {form})");
    let output = std::process::Command::new(oracle)
        .args(["-Q", "--batch", "--eval", setup, "--eval", long, "--eval", &printed(automatic), "--eval", long,
               "--eval", &printed(explicit), "--eval", throwing, "--eval", &printed(thrown)]).output().unwrap();
    assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
    assert_eq!(String::from_utf8(output.stdout).unwrap(), expected.concat(), "pinned GNU");
    let mut vm = Vm::new();
    vm.install_lisp_prelude().unwrap();
    vm.eval_source(setup).unwrap();
    let mut value = |vm: &mut Vm, form: &str| vm.eval_source(&format!("(prin1-to-string {form})")).unwrap().display().unwrap();
    vm.eval_source(long).unwrap();
    assert_eq!(value(&mut vm, automatic), serde_json::to_string(expected[0]).unwrap(), "automatic");
    vm.eval_source(long).unwrap();
    assert_eq!(value(&mut vm, explicit), serde_json::to_string(expected[1]).unwrap(), "explicit");
    vm.eval_source(throwing).unwrap();
    assert_eq!(value(&mut vm, thrown), serde_json::to_string(expected[2]).unwrap(), "thrown");
}

/// A collection that ends a level or two under `max-lisp-eval-depth' still
/// finishes: the run borrows its room, the hook comes back, and the limit is
/// restored.
#[test]
fn a_collection_finishes_near_the_depth_limit() {
    let mut vm = Vm::new();
    vm.install_lisp_prelude().unwrap();
    let eval = |vm: &mut Vm, source: &str| vm.eval_source(source).unwrap().display().unwrap();
    let setup = "(progn (defvar deep-runs 0) (defvar deep-limit 0) \
        (fset 'deep-note (lambda () (setq deep-runs (1+ deep-runs)))) \
        (fset 'deep-probe (lambda (n) (setq deep-limit n) (1+ (deep-probe (1+ n))))) \
        (fset 'deep-churn (lambda () (setq post-gc-hook '(deep-note)) \
          (let ((i 0)) (while (and (= deep-runs 0) (< i 100000000)) (cons i nil) (setq i (1+ i)))) \
          (setq post-gc-hook nil) t)) \
        (fset 'deep-call (lambda (n) (if (> n 0) (progn (deep-call (1- n))) (deep-churn)))) \
        (setq max-lisp-eval-depth 400) t)";
    eval(&mut vm, setup);
    assert_eq!(eval(&mut vm, "(condition-case nil (deep-probe 0) (excessive-lisp-nesting 'limit))"), "limit", "the calibration");
    assert_eq!(eval(&mut vm, "(condition-case nil (deep-call deep-limit) (excessive-lisp-nesting 'limit))"), "limit", "one more would not fit");
    assert_eq!(eval(&mut vm, "(setq deep-runs 0)"), "0");
    let reserve = eval(&mut vm, "lisp-eval-depth-reserve");
    assert_eq!(eval(&mut vm, "(deep-call (- deep-limit 2))"), "t", "the program below sees no error");
    assert_eq!(eval(&mut vm, "(> deep-runs 0)"), "t", "the hook ran, there");
    assert_eq!(eval(&mut vm, "post-gc-hook"), "nil", "and came back to be removed by the program, not for failing");
    assert_eq!(eval(&mut vm, "max-lisp-eval-depth"), "400", "the limit is restored");
    assert_eq!(eval(&mut vm, "lisp-eval-depth-reserve"), reserve, "and the reserve");
}

/// A collection finishes wherever it ran, a cleanup included: a `throw' from
/// the hook leaves the cleanup there, as it does in GNU.
#[test]
fn a_throw_from_the_hook_leaves_a_cleanup() {
    let form = "(progn (defvar cl-done nil) \
        (list (catch 'out \
                (setq post-gc-hook (list (lambda () (setq post-gc-hook nil) (throw 'out 'hook-threw)))) \
                (unwind-protect (throw 'out 'payload) \
                  (garbage-collect) \
                  (setq cl-done t))) \
              cl-done))";
    let expected = "(hook-threw nil)";
    let oracle = std::env::var("ROMACS_GNU_ORACLE").expect("pinned GNU oracle");
    let output = std::process::Command::new(oracle)
        .args(["-Q", "--batch", "--eval", &format!("(prin1 {form})")]).output().unwrap();
    assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
    assert_eq!(String::from_utf8(output.stdout).unwrap(), expected, "pinned GNU");
    let mut vm = Vm::new();
    vm.install_lisp_prelude().unwrap();
    assert_eq!(vm.eval_source(form).unwrap().display().unwrap(), expected, "candidate");
}

/// What pinned GNU and a fresh candidate print for CHECK after FORMS, each
/// form at top level of its own.
fn gnu_and_candidate(forms: &[&str], check: &str) -> (String, String) {
    let oracle = std::env::var("ROMACS_GNU_ORACLE").expect("pinned GNU oracle");
    let printed = format!("(prin1 {check})");
    let mut args = vec!["-Q", "--batch"];
    for form in forms.iter().chain([&printed.as_str()]) {
        args.extend(["--eval", form]);
    }
    let output = std::process::Command::new(oracle).args(args).output().unwrap();
    assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
    let mut vm = Vm::new();
    vm.install_lisp_prelude().unwrap();
    for form in forms {
        vm.eval_source(form).unwrap();
    }
    let candidate = vm.eval_source(check).unwrap().display().unwrap();
    (String::from_utf8(output.stdout).unwrap(), candidate)
}

/// A collection inside `post-gc-hook' is an ordinary one (VM2-POST-GC):
/// `garbage-collect' there collects and answers its rows, where GNU's
/// inhibition answers nil; allocation there collects too; and the hook runs
/// once for the collection that started it, the collections inside being
/// its own. (GNU's runs again: what its first run allocated uncollected
/// starts the next collection as the hook ends.)
#[test]
fn a_collection_inside_post_gc_hook_is_an_ordinary_one() {
    let setup = "(progn (defvar hk-runs 0) (defvar hk-inner nil) (defvar hk-churned nil) \
        (fset 'hk-churn (lambda () (let ((done gcs-done) (i 0)) \
          (while (< i 3000000) (cons i nil) (setq i (1+ i))) (- gcs-done done)))) t)";
    let check = "(let ((done gcs-done)) \
          (setq post-gc-hook (list (lambda () (setq hk-runs (1+ hk-runs) \
                                                    hk-inner (consp (garbage-collect)) \
                                                    hk-churned (> (hk-churn) 0))))) \
          (garbage-collect) \
          (setq post-gc-hook nil) \
          (list hk-inner hk-churned hk-runs (> (- gcs-done done) 2)))";
    let (gnu, candidate) = gnu_and_candidate(&[setup], check);
    assert!(gnu.starts_with("(nil nil "), "pinned GNU inhibits collection in the hook: {gnu}");
    assert_eq!(candidate, "(t t 1 t)", "the candidate collects there, and runs the hook once");
}

/// A finalizer that a collection inside `post-gc-hook' dooms runs in that
/// collection's own run, inside the hook, which still runs once; GNU's
/// inhibited `garbage-collect' there dooms nothing, so it runs later.
#[test]
fn a_finalizer_doomed_inside_the_hook_runs_there() {
    let setup = "(progn (defvar nh-log nil) (defvar nh-runs 0) (defvar nh-kept nil) t)";
    // Made in a top-level form of its own, and kept only in NH-KEPT, which
    // the hook drops: no register of the running evaluation holds it
    // (VM2-GC-FINALIZER).
    let made = "(progn (setq nh-kept (make-finalizer (lambda () (setq nh-log (cons (cons 'fin nh-runs) nh-log))))) nil)";
    let check = "(progn \
          (setq post-gc-hook (list (lambda () (setq nh-runs (1+ nh-runs)) \
                                     (when (= nh-runs 1) \
                                       (setq nh-kept nil) (garbage-collect) (setq nh-log (cons 'after-gc nh-log)))))) \
          (garbage-collect) \
          (setq post-gc-hook nil) \
          (list nh-runs (reverse nh-log)))";
    let (gnu, candidate) = gnu_and_candidate(&[setup, made], check);
    // The finalizer logs the hook run it fell in: 1 is inside the first.
    assert_eq!(gnu, "(1 (after-gc))", "pinned GNU runs no finalizer inside the hook");
    assert_eq!(candidate, "(1 ((fin . 1) after-gc))", "candidate");
}

/// A collection inside a run of finalizers, here a finalizer's own
/// `garbage-collect', runs as an ordinary one, but its `post-gc-hook' is the
/// run's, which runs once after the finalizers; GNU runs it again inside the
/// finalizer first.
#[test]
fn a_collection_in_a_finalizer_leaves_the_hook_to_its_run() {
    let setup = "(progn (defvar nf-log nil) (fset 'nf-note (lambda (x) (setq nf-log (cons x nf-log)))) t)";
    // A top-level form of its own: a register of the running one keeps what
    // it held alive (VM2-GC-FINALIZER).
    let doom = "(progn (make-finalizer (lambda () (nf-note (if (consp (garbage-collect)) 'fin-gc 'fin-nil)))) nil)";
    let check = "(progn (setq post-gc-hook '((lambda () (nf-note 'hook)))) \
          (garbage-collect) \
          (setq post-gc-hook nil) \
          (reverse nf-log))";
    let (gnu, candidate) = gnu_and_candidate(&[setup, doom], check);
    assert_eq!(gnu, "(hook fin-gc hook)", "pinned GNU");
    assert_eq!(candidate, "(fin-gc hook)", "candidate");
}

/// Finalizers and `post-gc-hook' end an automatic collection inside
/// whatever code allocated, which keeps its match data and current buffer
/// (VM2-POST-GC), as a process filter's caller does; GNU's hook overwrites
/// both.
#[test]
fn post_gc_work_keeps_the_match_data_and_buffer_it_interrupted() {
    let setup = "(progn (set-buffer (get-buffer-create \"md-subject\")) (insert \"abc\") (goto-char 1) \
        (re-search-forward \"b\") \
        (defvar md-finalized nil) \
        (make-finalizer (lambda () (setq md-finalized t) (string-match \"z\" \"xz\") (set-buffer (get-buffer-create \" md-other\")))) \
        (setq post-gc-hook (list (lambda () (string-match \"q\" \"aq\") (set-buffer (get-buffer-create \" md-other\"))))) \
        nil)";
    let automatic = "(let ((buffer (current-buffer)) (data (match-data)) (done gcs-done) (i 0)) \
          (while (< i 3000000) (cons i nil) (setq i (1+ i))) \
          (setq post-gc-hook nil) \
          (list (> gcs-done done) md-finalized (eq buffer (current-buffer)) (equal data (match-data))))";
    // And after an explicit `garbage-collect', whose caller GNU's hook
    // leaves in its buffer with its match data.
    let explicit = "(progn (set-buffer (get-buffer-create \"md-subject\")) (goto-char 1) (re-search-forward \"b\") \
          (setq post-gc-hook (list (lambda () (string-match \"q\" \"aq\") (set-buffer (get-buffer-create \" md-other\"))))) \
          (let ((buffer (current-buffer)) (data (match-data))) \
            (garbage-collect) \
            (setq post-gc-hook nil) \
            (list (eq buffer (current-buffer)) (equal data (match-data)))))";
    let (gnu, candidate) = gnu_and_candidate(&[setup], &format!("(list {automatic} {explicit})"));
    assert_eq!(gnu, "((t t nil nil) (nil nil))", "pinned GNU");
    assert_eq!(candidate, "((t t t t) (t t))", "candidate");
}

/// `gcs-done' and `gc-elapsed' count every collection, `garbage-collect'
/// and automatic ones (alloc.c:6004); `garbage-collect-maybe' collects only
/// past 1/FACTOR of the automatic trigger's allocation (alloc.c:6110).
/// benchmark.el's `benchmark-run' reads both counters; Lisp that stores a
/// non-float in `gc-elapsed' stops its updates, as GNU's.
#[test]
fn collection_counters_and_garbage_collect_maybe() {
    let check = "(let* ((g gcs-done) (e gc-elapsed) (stats (garbage-collect)) (g1 gcs-done) (e1 gc-elapsed) \
                       (none (garbage-collect-maybe 1)) (g2 gcs-done)) \
                  (let ((i 0)) (while (< i 200000) (cons 1 2) (setq i (1+ i)))) \
                  (let ((some (garbage-collect-maybe 1000000)) (g3 gcs-done)) \
                    (list (consp stats) (- g1 g) (floatp e1) (>= e1 e) none (- g2 g1) some (> g3 g2) \
                          (condition-case err (garbage-collect-maybe -1) (error err)) \
                          (garbage-collect-maybe 0) \
                          (progn (setq gc-elapsed 'off) (garbage-collect) gc-elapsed))))";
    let expected = "(t 1 t t nil 0 t t (wrong-type-argument wholenump -1) nil off)";
    let oracle = std::env::var("ROMACS_GNU_ORACLE").expect("pinned GNU oracle");
    let output = std::process::Command::new(oracle)
        .args(["-Q", "--batch", "--eval", &format!("(prin1 {check})")]).output().unwrap();
    assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
    assert_eq!(String::from_utf8(output.stdout).unwrap(), expected, "pinned GNU");
    let mut vm = Vm::new();
    vm.set_gc_stress(0);
    vm.install_lisp_prelude().unwrap();
    let printed = vm.eval_source(&format!("(prin1-to-string {check})")).unwrap().display().unwrap();
    assert_eq!(printed, serde_json::to_string(expected).unwrap(), "candidate");
}
