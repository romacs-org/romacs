//! Forward `scan-lists' and `scan-sexps' over texts long enough that the
//! syntax index folds them (crates/romacs-vm/src/syntax_index.rs), against
//! pinned GNU: the reviewers' probes for counts, depths and floors with a
//! leaf cut (every 1,024 characters of a fresh text) at the position each
//! is about, stops for `internal--syntax-propertize' below and at the
//! frontier, a table held in a `syntax-table' property, and counts over
//! many long lists, before and after edits. Each case also asserts, from
//! `romacs-syntax-summaries-stats', that the index folded: a missing fast
//! path fails on the counts, never on time.
use romacs_vm::Vm;

/// `fill-to' inserts PIECE (one character) until point is POS; `try' gives
/// a scan's value or its error; `measured' F's value, recording in
/// `romacs--folded' what that call alone did to the buffer's index, as
/// (EXCURSION-FOLDS EXACT-FOLDS REUSED-CHARACTERS LEAF-SCANS), all 0 in
/// GNU; `twice' a scan's value cold, then warm and measured; `table' a new
/// syntax table, `under' F's value under a table.
const PRELUDE: &str = r#"(progn
 (defalias 'fill-to (lambda (pos piece) (while (< (point) pos) (insert piece))))
 (defalias 'try (lambda (f) (condition-case e (funcall f) (scan-error (cons 'error e)))))
 (defalias 'counts (lambda () (if (fboundp 'romacs-syntax-summaries-stats)
   (let ((s (romacs-syntax-summaries-stats)))
     (list (or (cdr (assq 'excursion (plist-get s :folds))) 0) (or (cdr (assq 'exact (plist-get s :folds))) 0)
           (or (plist-get s :reused) 0) (or (plist-get s :leaf-scans) 0)))
   (list 0 0 0 0))))
 (defalias 'measured (lambda (f) (let* ((before (counts)) (value (funcall 'try f)) (after (counts)) (r nil))
   (while after (setq r (cons (- (car after) (car before)) r) after (cdr after) before (cdr before)))
   (setq romacs--folded (nreverse r))
   value)))
 (defalias 'twice (lambda (f) (list (funcall 'try f) (funcall 'measured f))))
 (defalias 'table (lambda (&optional parent) (let ((st (make-char-table 'syntax-table))) (set-char-table-parent st (or parent (standard-syntax-table))) st)))
 (defalias 'under (lambda (table f) (let ((old (syntax-table))) (set-syntax-table table) (unwind-protect (funcall f) (set-syntax-table old)))))
 (setq romacs--folded (list 0 0 0 0))
 (with-temp-buffer EXPR))"#;

/// A case: its name, the body of a `with-temp-buffer', and what its
/// measured call must do in Romacs: the least number of spans of each kind
/// (excursion, exact) it folds, the least number of characters it folds on
/// summaries made before it, and whether it is warm: it summarizes at most
/// the leaf it starts in, which a scan's first 1,024 steps read without
/// the index, so that a scan stopped for Lisp there makes it after.
type Case = (&'static str, &'static str, ([u64; 2], u64, bool));

const CASES: &[Case] = &[
    // (a): up one level from depth -1, the opener the first character of a leaf.
    ("open at depth -1", r#"(fill-to 4097 "x") (insert "(a)") (fill-to 9000 "x")
       (twice (lambda () (scan-lists 1 1 -1)))"#, ([0, 1], 3000, true)),
    // (a): out of a list from depth 1, the closer the first character of a leaf.
    ("close at depth 1", r#"(insert "(") (fill-to 4097 "x") (insert ")") (fill-to 9000 "x")
       (twice (lambda () (scan-lists 2 1 1)))"#, ([1, 0], 3000, true)),
    // ((: depth -2 rises to 0 at the second opener, a cut between the two.
    ("two openers from depth -2", r#"(fill-to 4096 "x") (insert "((") (fill-to 9000 "x")
       (twice (lambda () (scan-lists 1 1 -2)))"#, ([0, 1], 3000, true)),
    // ($x): math is nothing to a list scan and a level to a sexp scan; a cut between ( and $.
    ("math after a cut", r#"(set-syntax-table (table)) (modify-syntax-entry ?$ "$")
       (fill-to 4096 " ") (insert "($") (fill-to 9000 "x") (insert ")") (fill-to 12000 " ")
       (list (twice (lambda () (scan-lists 1 1 0))) (twice (lambda () (scan-sexps 1 1)))
             (twice (lambda () (scan-sexps 4096 1))))"#, ([1, 0], 4000, true)),
    // A stop at every position, GNU's 2 to 14.
    ("a stop per position", r#"(insert "(abcdefghij) following")
       (set (make-local-variable 'parse-sexp-lookup-properties) t)
       (let ((log nil))
         (fset 'internal--syntax-propertize (lambda (pos) (setq log (cons pos log)) (setq syntax-propertize--done pos)))
         (setq syntax-propertize--done 1)
         (list (scan-sexps 1 1) (nreverse log)))"#, ([0, 0], 0, false)),
    // Warm summaries, then the frontier lowered with no edit: the stops are
    // GNU's, and the text below each new frontier folds.
    ("stops below a lowered frontier", r#"(insert "(") (dotimes (i 400) (insert (format "(g%d \"s(\" 'x [y]) " i))) (insert ") following")
       (set (make-local-variable 'parse-sexp-lookup-properties) t)
       (setq syntax-propertize--done most-positive-fixnum)
       (let ((log nil) (warm (list (scan-sexps 1 1) (scan-lists 1 1 0))))
         (fset 'internal--syntax-propertize
               (lambda (pos) (setq log (cons pos log)) (setq syntax-propertize--done (min (point-max) (+ pos 2500)))))
         (setq syntax-propertize--done 1)
         (list warm (measured (lambda () (scan-sexps 1 1))) (nreverse log)
               (progn (setq log nil syntax-propertize--done 300) (list (scan-lists 1 1 0) (nreverse log)))))"#, ([1, 0], 2000, true)),
    // The closer the first character of a leaf, frontiers 2,000 apart.
    ("a closer at a cut under stops", r#"(insert "(") (fill-to 6145 "x") (insert ")") (fill-to 9000 " ")
       (set (make-local-variable 'parse-sexp-lookup-properties) t)
       (setq syntax-propertize--done most-positive-fixnum)
       (let ((log nil) (warm (scan-sexps 1 1)))
         (fset 'internal--syntax-propertize
               (lambda (pos) (setq log (cons pos log)) (setq syntax-propertize--done (min (point-max) (+ pos 2000)))))
         (setq syntax-propertize--done 1)
         (list warm (measured (lambda () (scan-sexps 1 1))) (nreverse log)))"#, ([1, 0], 2000, true)),
    // The second callback makes an opener the scan already entered
    // punctuation: that scan keeps what it read, the next reads it anew.
    ("a callback rewrites an opener read", r#"(insert "(") (dotimes (i 400) (insert (format "(g%d \"s(\" 'x [y]) " i))) (insert ") following")
       (set (make-local-variable 'parse-sexp-lookup-properties) t)
       (setq syntax-propertize--done most-positive-fixnum)
       (let ((log nil) (n 0) (warm (scan-sexps 1 1)))
         (fset 'internal--syntax-propertize
               (lambda (pos) (setq log (cons pos log) n (1+ n))
                 (when (= n 2) (put-text-property 2 3 'syntax-table (string-to-syntax ".")))
                 (setq syntax-propertize--done (min (point-max) (+ pos 2500)))))
         (setq syntax-propertize--done 1)
         (list warm (measured (lambda () (scan-sexps 1 1))) (nreverse log)
               (progn (setq syntax-propertize--done most-positive-fixnum) (list (scan-sexps 1 1) (scan-sexps 1 1)))))"#, ([1, 0], 2000, true)),
    // A table held in a property, written after the summaries were made,
    // the opener inside a leaf.
    ("a table in a property", r#"(fill-to 4500 " ") (insert "()") (fill-to 9000 " ")
       (set (make-local-variable 'parse-sexp-lookup-properties) t)
       (setq syntax-propertize--done most-positive-fixnum)
       (let ((copy (copy-syntax-table)))
         (put-text-property 4500 4502 'syntax-table copy)
         (list (twice (lambda () (scan-sexps 1 (point-max))))
               (progn (modify-syntax-entry ?\( "." copy) (twice (lambda () (scan-sexps 1 (point-max)))))))"#, ([0, 1], 3000, true)),
    // So too through the parent of the table the property holds.
    ("a table in a property, its parent written", r#"(fill-to 4500 " ") (insert "()") (fill-to 9000 " ")
       (set (make-local-variable 'parse-sexp-lookup-properties) t)
       (setq syntax-propertize--done most-positive-fixnum)
       (let* ((parent (copy-syntax-table)) (child (table parent)))
         (put-text-property 4500 4502 'syntax-table child)
         (list (twice (lambda () (scan-sexps 1 (point-max))))
               (progn (modify-syntax-entry ?\( "." parent) (twice (lambda () (scan-sexps 1 (point-max)))))))"#, ([0, 1], 3000, true)),
    // Counts over forty lists of about 3,000 characters, then edits.
    ("counts over long lists", r#"(dotimes (i 40) (insert (format "(defun f%d (a)\n  \"Doc (%d).\"\n" i i))
         (dotimes (j 110) (insert (format "  (list a 'b%d \"c)\" [d])\n" j))) (insert "  a)\n"))
       (let ((scans (lambda () (mapcar (lambda (args) (try (lambda () (apply (car args) (cdr args)))))
                                       '((scan-sexps 1 5) (scan-sexps 1 20) (scan-sexps 1 39) (scan-sexps 1 40) (scan-sexps 1 41)
                                         (scan-lists 1 20 0) (scan-lists 1 41 0) (scan-sexps 3001 3) (scan-lists 3001 7 1))))))
         (list (funcall scans) (measured scans)
               (progn (goto-char 50000) (insert "x") (funcall scans))
               (progn (goto-char 70000) (insert "(") (funcall scans))
               (progn (goto-char 70000) (delete-char 1) (goto-char 9000) (insert ")") (funcall scans))))"#, ([1, 1], 50000, true)),
    // A COUNT 1 match many leaves on, as electric-pair and cc-mode ask:
    // from the outer opener, from an inner one, as sexp and as list.
    ("a far match", r#"(insert "(") (dotimes (i 3000) (insert (format "(f%d a \"b)\" 'c) ; d (\n" i))) (insert ")\n")
       (let ((scans (lambda () (mapcar (lambda (args) (try (lambda () (apply (car args) (cdr args)))))
                                       '((scan-sexps 1 1) (scan-lists 1 1 0) (scan-sexps 1001 1) (scan-lists 1001 1 0) (scan-lists 2 1 1))))))
         (list (funcall scans) (measured scans)
               (progn (goto-char 40000) (insert "x") (funcall scans))
               (progn (goto-char 40000) (insert "(") (funcall scans))
               (progn (goto-char 40000) (delete-char 1) (funcall scans))))"#, ([1, 0], 50000, true)),
    // Two tables by turns over one text.
    ("two tables by turns", r#"(insert "{\n") (dotimes (i 2000) (insert (format "  if (a_%d) { b('x', \"y\"); } /* c */\n" i))) (insert "}\n")
       (let ((a (table)) (b (table)))
         (modify-syntax-entry ?' "\"" a) (modify-syntax-entry ?_ "w" b) (modify-syntax-entry ?/ ". 14" a) (modify-syntax-entry ?* ". 23" a)
         (set-syntax-table a)
         (let ((r nil)) (dotimes (_ 3) (setq r (cons (list (scan-lists 1 1 0) (under b (lambda () (measured (lambda () (scan-lists 1 1 0)))))) r))) r))"#, ([1, 0], 50000, true)),
];

fn gnu(source: &str) -> String {
    let binary = std::env::var("ROMACS_GNU_ORACLE").expect("pinned GNU oracle");
    let output = std::process::Command::new(&binary).args(["-Q", "--batch", "--eval", &format!("(prin1 {source})")]).output().unwrap();
    assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
    String::from_utf8(output.stdout).unwrap()
}

/// SOURCE's value in Romacs, and the spans its buffer's index folded.
fn romacs(source: &str) -> (Result<String, String>, String) {
    let mut vm = Vm::new();
    vm.install_lisp_prelude().unwrap();
    let value = vm.eval_source(source).and_then(romacs_vm::Local::display).map_err(|e| format!("{e:?}"));
    let folded = vm.eval_source("romacs--folded").and_then(romacs_vm::Local::display).unwrap();
    (value, folded)
}

#[test]
fn forward_list_scans_match_pinned_gnu_and_fold() {
    let _alone = alone();
    let mut failures = Vec::new();
    for (name, body, (least, reused, warm)) in CASES {
        let source = PRELUDE.replace("EXPR", body);
        let expected = gnu(&source);
        let (actual, folded) = romacs(&source);
        if actual.as_deref() != Ok(expected.as_str()) {
            failures.push(format!("{name}: values differ\n  GNU    {expected}\n  Romacs {actual:?}"));
        }
        let counts: Vec<u64> = folded.trim_matches(['(', ')']).split(' ').filter_map(|n| n.parse().ok()).collect();
        if counts.len() != 4 || counts[0] < least[0] || counts[1] < least[1] || counts[2] < *reused || (*warm && counts[3] > 1) {
            failures.push(format!("{name}: the measured call did (excursion exact reused leaf-scans) {folded}, wanted at least {least:?}, {reused} reused{}",
                                  if *warm { ", at most one leaf scan" } else { "" }));
        }
    }
    assert!(failures.is_empty(), "{} failures:\n{}", failures.len(), failures.join("\n"));
}

/// This file's tests one at a time: each reads the counts of a buffer's
/// index, which a char-table write moves, and every test writes tables.
/// Its own target (tests/all/standalone.txt) keeps other tests' writes
/// out, so a count that moves here is the fixture's own.
fn alone() -> std::sync::MutexGuard<'static, ()> {
    static ALONE: std::sync::Mutex<()> = std::sync::Mutex::new(());
    ALONE.lock().unwrap_or_else(std::sync::PoisonError::into_inner)
}

/// What EXPRESSION, run in a fresh Romacs, shows: a list whose last
/// element is a buffer size and whose others are counter deltas, none of
/// which may have a context lose its summaries (`alone').
fn shown(expression: &str) -> String {
    let mut vm = Vm::new();
    vm.install_lisp_prelude().unwrap();
    let shown = vm.eval_source(expression).and_then(romacs_vm::Local::display).unwrap_or_else(|e| panic!("{expression}: {e:?}"));
    assert!(deltas(&shown).iter().all(|delta| delta[6] == 0), "a context lost its summaries: {shown}");
    shown
}

/// `stat' gives a count of the buffer's index, `delta' what F's call did to
/// each of `counters', as a list.
const COUNTERS: &str = r#"(progn
 (defalias 'table (lambda () (let ((st (make-char-table 'syntax-table))) (set-char-table-parent st (standard-syntax-table)) st)))
 (defalias 'under (lambda (table f) (let ((old (syntax-table))) (set-syntax-table table) (unwind-protect (funcall f) (set-syntax-table old)))))
 (defalias 'stat (lambda (key &optional sub) (let ((v (plist-get (romacs-syntax-summaries-stats) key))) (or (if sub (cdr (assq sub v)) v) 0))))
 (defalias 'counters (lambda () (list (stat :list-served) (stat :folds 'excursion) (stat :folds 'exact) (stat :skipped) (stat :reused)
                                      (stat :leaf-scans) (stat :context-invalidations))))
 (defalias 'delta (lambda (f) (let ((before (counters))) (funcall f)
   (let ((after (counters)) (r nil))
     (while after (setq r (cons (- (car after) (car before)) r) after (cdr after) before (cdr before)))
     (nreverse r)))))
 EXPR)"#;

/// Counter deltas as numbers: served, excursion folds, exact folds,
/// characters skipped, of those on summaries made before the call, leaves
/// summarized, contexts invalidated.
fn deltas(shown: &str) -> Vec<Vec<i64>> {
    shown.split(")").filter_map(|group| {
        let numbers: Vec<i64> = group.trim_matches(['(', ' ']).split(' ').filter_map(|n| n.parse().ok()).collect();
        (numbers.len() == 7).then_some(numbers)
    }).collect()
}

/// The buffer size SHOWN ends with.
fn size(shown: &str) -> i64 {
    shown.trim_end_matches(')').rsplit(' ').next().unwrap().parse().unwrap()
}

/// A warm scan with a count past the end, which the index served before
/// S1, stays served: folded, nearly all its text on summaries made before
/// it, nothing read again for a summary.
#[test]
fn a_warm_unbounded_scan_stays_served() {
    let _alone = alone();
    let expression = COUNTERS.replace("EXPR", r#"(with-temp-buffer
       (dotimes (i 2500) (insert (format "(defun f%d (a) \"s(\" (list a 'b))\n" i)))
       (scan-sexps 1 (point-max))
       (list (delta (lambda () (scan-sexps 1 (point-max)))) (buffer-size)))"#);
    let shown = shown(&expression);
    let [warm] = &deltas(&shown)[..] else { panic!("{shown}") };
    let size = size(&shown);
    assert!(warm[0] == 1 && warm[2] >= 1 && warm[4] >= size - 4096 && warm[5] == 0,
            "served, excursion and exact folds, skipped, reused, leaves summarized: {warm:?} of {size} characters");
}

/// cc-mode's statement-block scan, `(scan-lists BRACE 1 0)' over about
/// 800,000 characters, by turns under two tables: warm, each folds nearly
/// all of it on kept summaries and summarizes nothing, the other table's
/// summaries kept.
#[test]
fn a_block_scan_under_two_tables_is_served_warm() {
    let _alone = alone();
    let expression = COUNTERS.replace("EXPR", &format!(r#"(with-temp-buffer {C_BLOCK}
       (let ((a (table)) (b (table)))
         (modify-syntax-entry ?/ ". 124b" a) (modify-syntax-entry ?* ". 23" a) (modify-syntax-entry ?\n "> b" a)
         (modify-syntax-entry ?_ "w" b)
         (set-syntax-table a)
         (setq parse-sexp-ignore-comments t)
         (let ((rounds nil))
           (dotimes (_ 3)
             (setq rounds (cons (delta (lambda () (scan-lists 9 1 0))) rounds))
             (setq rounds (cons (delta (lambda () (under b (lambda () (scan-lists 9 1 0))))) rounds)))
           (list (nreverse rounds) (buffer-size)))))"#));
    let shown = shown(&expression);
    let rounds = deltas(&shown);
    assert_eq!(rounds.len(), 6, "{shown}");
    let size = size(&shown);
    assert!(size > 750_000, "{size}");
    for (i, round) in rounds.iter().enumerate().skip(2) {
        assert!(round[1] >= 1 && round[4] >= size - 4096 && round[5] == 0,
                "round {i} (table {}): served, excursion and exact folds, skipped, reused, leaves summarized: {round:?}", if i % 2 == 0 { 'a' } else { 'b' });
    }
}

/// An 800,000-character C block, the statement-block scan's text.
const C_BLOCK: &str = r#"(insert "int f() {\n") (dotimes (i 24000) (insert (format "  if (a_%d) { b(\"x\"); } /* c */\n" i))) (insert "}\n")"#;

/// After `()' is typed at the top, the warm scans of the whole text read
/// again only the edited leaf and the leaves beside it, and fold the rest
/// on a few composed nodes, not leaf by leaf: the block scan of one
/// 800,000-character function, and a scan with a count past the end over
/// 600 top-level functions of 40 lines, whose leaves the scan mostly
/// enters at depth 1 or more inside a function and now and then at depth 0
/// between two (keys-cts' no-match scans over xdisp.c).
#[test]
fn an_edit_at_the_top_rebuilds_only_its_path() {
    let _alone = alone();
    let functions = r#"(dotimes (i 600) (insert (format "static int f%d (int a)\n{\n" i))
                        (dotimes (j 40) (insert (format "  if (a_%d) { b(\"x\"); } /* c */\n" j))) (insert "}\n\n"))"#;
    for (text, scan) in [(functions, "(scan-sexps 1 (point-max))"), (C_BLOCK, "(scan-lists 9 1 0)")] {
        let expression = COUNTERS.replace("EXPR", &format!(r#"(with-temp-buffer {text}
           (let ((a (table)))
             (modify-syntax-entry ?/ ". 124b" a) (modify-syntax-entry ?* ". 23" a) (modify-syntax-entry ?\n "> b" a)
             (set-syntax-table a)
             (setq parse-sexp-ignore-comments t)
             {scan} {scan} {scan}
             (goto-char 1) (insert "()")
             (let ((edited (delta (lambda () {scan}))))
               (list edited (delta (lambda () {scan})) (buffer-size)))))"#, scan = scan.replace("(scan-lists 9 1 0)", "(scan-lists (if (eq (char-after 1) ?\\() 11 9) 1 0)")));
        let shown = shown(&expression);
        let rounds = deltas(&shown);
        assert_eq!(rounds.len(), 2, "{shown}");
        let size = size(&shown);
        let [edited, warm] = &rounds[..] else { unreachable!() };
        assert!(edited[5] <= 3 && edited[1] + edited[2] <= 64 && edited[4] >= size - 8192,
                "{scan} after the edit: served, excursion and exact folds, skipped, reused, leaves summarized: {edited:?} of {size}");
        assert!(warm[5] == 0 && warm[1] + warm[2] <= 64 && warm[4] >= size - 4096,
                "{scan} again: {warm:?} of {size}");
    }
}
