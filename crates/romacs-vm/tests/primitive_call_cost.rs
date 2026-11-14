//! What a call from compiled Lisp into a primitive costs, by the evaluating
//! thread's instruction count (`romacs-instructions`): a loop making the
//! call, less the same loop without it, at two lengths. A machine without
//! the counter fails these tests: they never pass unmeasured.
//!
//! Each test runs alone (`alone`), in an executable of its own
//! (`all/standalone.txt`): a syntax table's flat view keys on the
//! char-table writes of the whole process, so another test's write renews
//! it in the middle of a measurement.
use romacs_vm::Vm;

/// Held for a whole test, so no other test of this file writes a char table
/// while it measures.
fn alone() -> std::sync::MutexGuard<'static, ()> {
    static ALONE: std::sync::Mutex<()> = std::sync::Mutex::new(());
    ALONE.lock().unwrap_or_else(std::sync::PoisonError::into_inner)
}

fn eval(vm: &mut Vm, source: &str) -> String {
    vm.eval_source(source).and_then(romacs_vm::Local::display).unwrap_or_else(|e| panic!("{source}: {e}"))
}

/// Instructions one evaluation of BODY costs over one of `i`, in a loop
/// binding `i` (the counter) and `s` (the last value).
fn per_call(vm: &mut Vm, body: &str) -> f64 {
    per_call_in(vm, "progn", body)
}

/// `per_call` with the loop inside the form WRAP opens, as
/// `with-temp-buffer (insert ...)`.
fn per_call_in(vm: &mut Vm, wrap: &str, body: &str) -> f64 {
    let mut run = |body: &str, n: usize| -> f64 {
        let form = format!(
            "({wrap} (let ((f (lambda (n) (let ((i 0) (s 0)) (while (< i n) (setq s {body}) (setq i (1+ i))) s)))) \
               (funcall f 10) (let ((a (romacs-instructions))) (funcall f {n}) (and a (- (romacs-instructions) a)))))"
        );
        let text = eval(vm, &form);
        text.parse().unwrap_or_else(|_| panic!("no instruction counter (romacs-instructions gave {text})"))
    };
    let with = (run(body, 3000) - run(body, 1000)) / 2000.0;
    let without = (run("i", 3000) - run("i", 1000)) / 2000.0;
    with - without
}

/// The bitwise primitives fold fixnums in machine words, as GNU's
/// `arith_driver` does; a bignum per operand cost several hundred
/// instructions a call.
#[test]
fn bitwise_on_fixnums_stays_in_machine_words() {
    let _alone = alone();
    let mut vm = Vm::new();
    vm.install_lisp_prelude().unwrap();
    for body in ["(logand i 1023)", "(logior i 1 2)", "(logxor i 5)", "(lognot i)", "(ash i 2)", "(ash i -1)", "(logcount i)"] {
        let cost = per_call(&mut vm, body);
        // 290-350 with the fold; 770-990 through bignums.
        assert!(cost < 450.0, "{body} costs {cost:.0} instructions a call");
    }
    assert_eq!(
        eval(&mut vm, "(list (logand 12 10) (logand) (logior 12 3) (logxor 12 10) (lognot 5) (ash 5 2) (ash -5 -1) (ash 1 70) (logcount -1) (logand most-negative-fixnum -1))"),
        "(8 -1 15 6 -6 20 -3 1180591620717411303424 0 -2305843009213693952)"
    );
    // The edges of the fold: the first bignum, a shift past the word, the
    // widest left shift of a fixnum, a marker, and the complement's end.
    assert_eq!(
        eval(&mut vm, "(list (ash 1 61) (ash -1 -64) (ash most-negative-fixnum 1) (with-temp-buffer (logand (point-marker))) (lognot most-negative-fixnum))"),
        "(2305843009213693952 -1 -4611686018427387904 1 2305843009213693951)"
    );
}

/// A search whose pattern is already compiled finds it by the pattern
/// string's own bytes, builds no case translation, and reads its variables
/// by slot. Each of those was paid on every call.
#[test]
fn a_cached_pattern_costs_no_copy_and_no_translation_table() {
    let _alone = alone();
    let mut vm = Vm::new();
    vm.install_lisp_prelude().unwrap();
    let buffer = "with-temp-buffer (dotimes (_ 200) (insert \"  (defun example (arg) ;; comment here\\n\")) (goto-char 400)";
    // 3,124 and 4,063 with variables read where they were found and
    // positions kept in bytes; 4,056 and 4,545 with the syntax memo; 4,525
    // and 5,004 before it; 5,299 and 6,066 before either.
    for (body, bound) in [("(looking-at \"[ \\t]*(def\")", 3500.0), ("(string-match \"b+\" \"aabbcc\")", 4400.0)] {
        let cost = per_call_in(&mut vm, buffer, body);
        assert!(cost < bound, "{body} costs {cost:.0} instructions a call");
    }
    assert_eq!(
        eval(&mut vm, "(with-temp-buffer (insert \"  (Defun x)\") (goto-char 1) (list (looking-at \"[ \\t]*(def\") (let ((case-fold-search nil)) (looking-at \"[ \\t]*(def\")) (string-match \"B+\" \"aabbcc\") (let ((case-fold-search nil)) (string-match \"B+\" \"aabbcc\")) (condition-case e (string-match 1 \"a\") (error e)) (search-forward \"(D\" nil t)))"),
        "(t nil 2 nil (wrong-type-argument stringp 1) 5)"
    );
}

/// What a search learns of its syntax table lasts between searches: that
/// the table is one whose parents do not loop, and its entries for ASCII.
/// Every way the table can change must still be seen by the next search:
/// `modify-syntax-entry`, a plain `aset`, a change to the entry cons
/// itself, a parent's entry, another parent, a collection, and another
/// table for the buffer.
#[test]
fn the_syntax_memo_follows_every_change_to_a_table() {
    let _alone = alone();
    let mut vm = Vm::new();
    vm.install_lisp_prelude().unwrap();
    let buffer = "with-temp-buffer (dotimes (_ 200) (insert \"  (defun example (arg) ;; comment here\\n\")) (goto-char 5)";
    // 2,179 walking bytes from point; 2,898 with the memo; 3,720 before.
    let cost = per_call_in(&mut vm, buffer, "(progn (goto-char 5) (skip-syntax-forward \"^w\"))");
    assert!(cost < 2500.0, "skip-syntax-forward costs {cost:.0} instructions a call");
    // Pinned GNU's answers to the same form.
    assert_eq!(
        eval(&mut vm, "(with-temp-buffer (insert \"a-b c\") (let* ((r nil) (note (lambda () (setq r (cons (list (progn (goto-char 1) (skip-syntax-forward \"w\") (point)) (progn (goto-char 1) (and (looking-at \"\\\\sw+\") (match-end 0)))) r)))) (table (lambda (parent) (let ((tb (make-char-table 'syntax-table nil))) (set-char-table-parent tb parent) tb))) (parent (funcall table (standard-syntax-table))) (child (funcall table parent)) (other (funcall table (standard-syntax-table))) (old (syntax-table))) (set-syntax-table child) (funcall note) (modify-syntax-entry ?- \"w\" child) (funcall note) (aset child ?- (cons 2 nil)) (funcall note) (setcar (aref child ?-) 1) (funcall note) (aset child ?- nil) (modify-syntax-entry ?- \"w\" parent) (funcall note) (set-char-table-parent child other) (funcall note) (garbage-collect) (funcall note) (set-syntax-table parent) (funcall note) (set-syntax-table old) (funcall note) (nreverse r)))"),
        "((2 2) (4 4) (4 4) (2 2) (4 4) (2 2) (2 2) (4 4) (2 2))"
    );
}

/// A syntax scan that calls no Lisp reads its variables where they were
/// found, keeps its registers on the stack and records nothing for a
/// rerun: what each call paid besides the scan.
#[test]
fn a_scan_pays_little_besides_its_text() {
    let _alone = alone();
    let mut vm = Vm::new();
    vm.install_lisp_prelude().unwrap();
    let buffer = "with-temp-buffer (dotimes (_ 200) (insert \"  (defun example (arg) ;; comment here\\n\")) (goto-char 5)";
    // 3,914 and 4,034 here; in the full binary 11,487 and 11,730 a call
    // before, 4,342 and 4,461 after (pinned GNU 633 and 642).
    for (body, bound) in [("(progn (goto-char 24) (forward-comment 1))", 4600.0), ("(scan-sexps 16 1)", 4700.0)] {
        let cost = per_call_in(&mut vm, buffer, body);
        assert!(cost < bound, "{body} costs {cost:.0} instructions a call");
    }
}

/// A search reads the variable itself, not whatever its name is interned
/// as now: uninterning `case-fold-search` and setting a new symbol of
/// that name leaves searches folding case, as in GNU, where the search
/// reads a C variable. Looking the name up on every search followed the
/// new symbol, and panicked while there was none.
#[test]
fn a_search_reads_its_variables_not_their_names() {
    let _alone = alone();
    let mut vm = Vm::new();
    vm.install_lisp_prelude().unwrap();
    assert_eq!(eval(&mut vm, "(string-match \"A\" \"a\")"), "0");
    // Pinned GNU's answers to the same forms.
    assert_eq!(
        eval(&mut vm, "(progn (unintern \"case-fold-search\" obarray) (list (string-match \"A\" \"a\") (progn (set (intern \"case-fold-search\") nil) (string-match \"A\" \"a\"))))"),
        "(0 0)"
    );
}

/// `proper-list-p` walks a list reading each cons once, with the cycle
/// check `safe-length` has: the guard a mapping loop in place pays per
/// element (lisp/romacs/romacs-compile.el).
#[test]
fn proper_list_p_reads_each_cons_once() {
    let _alone = alone();
    let mut vm = Vm::new();
    vm.install_lisp_prelude().unwrap();
    let mut cost = |n: usize| -> f64 {
        let form = format!("(let ((l (make-list {n} 1))) (proper-list-p l) \
            (let ((a (romacs-instructions))) (dotimes (_ 20) (proper-list-p l)) (and a (- (romacs-instructions) a))))");
        let text = eval(&mut vm, &form);
        text.parse().unwrap_or_else(|_| panic!("no instruction counter (romacs-instructions gave {text})"))
    };
    let per_element = (cost(20_000) - cost(2_000)) / (20.0 * 18_000.0);
    // 38 when written; 198 when each cons was read twice through calls the walk
    // could not inline. Pinned GNU's is 14.
    assert!(per_element < 50.0, "proper-list-p costs {per_element:.1} instructions an element");
    assert_eq!(eval(&mut vm, "(list (proper-list-p nil) (proper-list-p '(1 2 3)) (proper-list-p '(1 . 2)) (proper-list-p 5) \
        (let ((c (list 1 2 3))) (setcdr (cdr (cdr c)) c) (proper-list-p c)) \
        (let ((c (list 1 2 3))) (setcdr (cdr (cdr c)) (cdr c)) (proper-list-p c)) \
        (safe-length '(1 2 . 3)) (let ((c (list 1 2))) (setcdr (cdr c) c) (safe-length c)))"),
        "(0 3 nil nil nil nil 2 2)");
    // Where the tortoise moves shows in the counts on a cycle: GNU's for a
    // self-cycle after one cons, and for a ring just past the first move.
    assert_eq!(eval(&mut vm, "(list (let ((c (list 0 1))) (setcdr (cdr c) (cdr c)) (safe-length c)) \
        (let ((c (make-list 4097 1))) (setcdr (nthcdr 4096 c) c) (safe-length c)))"),
        "(4097 12289)");
}
