//! What reading a case table or a syntax table costs, and that every change
//! of a table's contents is read: a primitive that reads a character after
//! another descends no table's nodes for each.
use super::*;

fn vm() -> Vm {
    let mut vm = Vm::new();
    vm.install_lisp_prelude().unwrap();
    vm
}

/// SOURCE's printed value, and how many characters it read alone by
/// descending a table's nodes and how many rows it read, in a run after a
/// first (which makes what is made on first use) during which no other
/// thread wrote a char table. The count of writes is the process's: a
/// write by another test's VM drops this one's views, which costs their
/// rows again and, between the making of a view and its first read, a
/// call's characters read alone.
fn counted(vm: &mut Vm, source: &str) -> (String, [usize; 2]) {
    let run = |vm: &mut Vm| {
        let before = (reads(), writes(), writes_here());
        let value = vm.eval_source_raw(source).unwrap_or_else(|error| panic!("{source}: {error}"));
        let after = reads();
        let foreign = writes() - before.1 != u64::try_from(writes_here() - before.2).unwrap();
        ((vm.display(value).unwrap(), [after[0] - before.0[0], after[1] - before.0[1]]), foreign)
    };
    run(vm);
    loop {
        let (counted, foreign) = run(vm);
        if !foreign { return counted; }
        std::thread::yield_now();
    }
}

/// SOURCE's printed value.
fn value(vm: &mut Vm, source: &str) -> String {
    let value = vm.eval_source_raw(source).unwrap_or_else(|error| panic!("{source}: {error}"));
    vm.display(value).unwrap()
}

/// SOURCE gives EXPECTED and reads no more than `STRAY` characters alone.
fn within(vm: &mut Vm, source: &str, expected: &str) {
    let (value, [alone, rows]) = counted(vm, source);
    assert_eq!(value, expected, "{source}");
    assert!(alone <= STRAY, "{source}: {alone} characters read alone by descending table nodes ({rows} rows read)");
}

/// The characters a run may read alone: a margin, since a run no other
/// thread disturbed reads none alone.
const STRAY: usize = 32;

#[test]
fn a_case_insensitive_compare_descends_no_table_per_character() {
    let mut vm = vm();
    within(&mut vm, "(let ((a (make-string 5000 ?a)) (b (make-string 5000 ?A)))
                      (list (compare-strings a nil nil b nil nil t) (compare-strings a nil nil b nil nil)
                            (compare-strings (concat a \"x\") nil nil (concat b \"Y\") nil nil t)
                            (compare-strings a 10 20 b 30 41 t)))",
           "(t 1 -5001 -11)");
    // Greek, Cyrillic and a raw byte: pages, read once each.
    within(&mut vm, "(let ((a (make-string 5000 ?α)) (b (make-string 5000 ?Α)) (c (make-string 5000 ?я)) (d (make-string 5000 ?Я)))
                      (list (compare-strings a nil nil b nil nil t) (compare-strings c nil nil d nil nil t)
                            (compare-strings (concat a \"\\377\") nil nil (concat b \"\\376\") nil nil t)))",
           "(t t 5001)");
    within(&mut vm, "(let ((keys nil)) (dotimes (i 500) (push (cons (format \"Key-%d\" i) i) keys))
                      (list (cdr (assoc-string \"KEY-7\" keys t)) (assoc-string \"KEY-7\" keys)))",
           "(7 nil)");
}

#[test]
fn case_conversion_descends_no_table_per_character() {
    let mut vm = vm();
    within(&mut vm, "(let ((s (concat (make-string 3000 ?a) (make-string 3000 ?Ω) \"ǅß\")))
                      (list (length (upcase s)) (substring (upcase s) -4) (substring (downcase (upcase s)) 2998 3002)
                            (substring (downcase (upcase s)) -4)))",
           "(6003 \"ΩǄSS\" \"aaωω\" \"ωǆss\")");
    within(&mut vm, "(let ((n 0)) (dotimes (i 5000) (setq n (+ n (upcase ?a) (downcase ?Ω)))) n)", "5170000");
}

#[test]
fn a_syntax_skip_descends_no_table_per_call() {
    let mut vm = vm();
    within(&mut vm, "(with-temp-buffer
                      (dotimes (_ 2000) (insert \"word next_one, \"))
                      (goto-char (point-min))
                      (let ((n 0)) (while (not (eobp)) (skip-syntax-forward \"^w_\") (skip-syntax-forward \"w_\") (setq n (1+ n))) n))",
           "4001");
    within(&mut vm, "(let ((n 0)) (dotimes (i 5000) (setq n (+ n (char-syntax ?a) (char-syntax ?\\()))) n)", "795000");
}

#[test]
fn a_syntax_scan_descends_no_table_per_call() {
    let mut vm = vm();
    within(&mut vm, "(with-temp-buffer
                      (dotimes (_ 2000) (insert \"(a b) \"))
                      (goto-char (point-min))
                      (let ((n 0)) (while (scan-sexps (point) 1) (goto-char (scan-sexps (point) 1)) (setq n (1+ n))) n))",
           "2000");
}

/// A table made for one use costs its row, not a descent for each character
/// read under it, and no other table's rows are read again for it.
#[test]
fn a_fresh_table_costs_its_row() {
    let mut vm = vm();
    let (value, [alone, rows]) = counted(&mut vm, "(with-temp-buffer
          (insert \"word and_more words\")
          (let ((n 0))
            (dotimes (_ 200)
              (let ((s (make-char-table 'syntax-table)))
                (set-char-table-parent s (standard-syntax-table))
                (aset s ?_ '(2))
                (set-syntax-table s)
                (goto-char 6)
                (setq n (+ n (skip-syntax-forward \"w\")))))
            n))");
    assert_eq!(value, "1600");
    // Each table's row and its parent's.
    assert!(alone <= STRAY && rows <= 200 * 8, "{alone} characters read alone and {rows} rows read for 200 fresh tables");
}

#[test]
fn a_write_to_a_table_or_its_parent_is_read() {
    let mut vm = vm();
    assert_eq!(value(&mut vm, "(with-temp-buffer
          (insert \"a.b-c\")
          (let ((parent (make-char-table 'syntax-table)) (s (make-char-table 'syntax-table)) (out nil)
                (skip (lambda () (goto-char 1) (skip-syntax-forward \"w\"))))
            (set-char-table-parent parent (standard-syntax-table))
            (set-char-table-parent s parent)
            (set-syntax-table s)
            (push (funcall skip) out)
            (aset s ?. '(2))
            (push (funcall skip) out)
            (aset parent ?- '(2))
            (push (funcall skip) out)
            (push (char-syntax ?-) out)
            (aset s ?- '(1))
            (push (funcall skip) out)
            (set-char-table-parent s (standard-syntax-table))
            (aset s ?- nil)
            (push (list (funcall skip) (char-syntax ?-) (string-match \"\\\\w+\" \"a.b-c\") (match-end 0)) out)
            (set-syntax-table (standard-syntax-table))
            (push (funcall skip) out)
            (nreverse out)))"), "(1 3 5 119 3 (3 95 0 3) 1)");
}

/// A descriptor is a cons a program may alter without writing the table
/// that holds it, and the next read sees it, as GNU's does.
#[test]
fn a_descriptor_altered_in_place_is_read() {
    let mut vm = vm();
    assert_eq!(value(&mut vm, "(with-temp-buffer
          (insert \"a.b\")
          (let ((s (make-char-table 'syntax-table)) (entry (list 1)) (out nil))
            (set-char-table-parent s (standard-syntax-table))
            (aset s ?. entry)
            (set-syntax-table s)
            (goto-char 1)
            (push (list (skip-syntax-forward \"w\") (char-syntax ?.) (string-match \"\\\\w+\" \"a.b\") (match-end 0)) out)
            (setcar entry 2)
            (goto-char 1)
            (push (list (skip-syntax-forward \"w\") (char-syntax ?.) (string-match \"\\\\w+\" \"a.b\") (match-end 0)) out)
            (push (scan-sexps 1 1) out)
            (setcar entry 1)
            (push (scan-sexps 1 1) out)
            (nreverse out)))"), "((1 46 0 1) (3 119 0 3) 4 2)");
}

#[test]
fn a_case_table_set_or_written_is_read() {
    let mut vm = vm();
    assert_eq!(value(&mut vm, "(let ((out nil) (fold (lambda (a b) (compare-strings a nil nil b nil nil t))))
          (with-temp-buffer
            (let ((d (make-char-table 'case-table)))
              (aset d ?A ?a)
              (push (list (funcall fold \"q\" \"Q\") (upcase \"aq\")) out)
              (set-case-table d)
              (push (list (funcall fold \"a\" \"A\") (funcall fold \"q\" \"Q\") (upcase \"aq\") (downcase \"AQ\")) out)
              (aset (char-table-extra-slot d 0) ?q ?Q)
              (push (list (funcall fold \"q\" \"Q\") (upcase \"aq\")) out)
              (aset d ?Q ?q)
              (push (downcase \"AQ\") out)
              ;; Another buffer keeps the standard table.
              (with-temp-buffer (push (list (funcall fold \"z\" \"Z\") (upcase \"z\")) out))
              (push (funcall fold \"z\" \"Z\") out)))
          (push (funcall fold \"z\" \"Z\") out)
          (nreverse out))"), "((t \"AQ\") (t 1 \"Aq\" \"aQ\") (t \"AQ\") \"aq\" (t \"Z\") 1 t)");
}

#[test]
fn a_unibyte_buffer_folds_bytes_as_bytes() {
    let mut vm = vm();
    assert_eq!(value(&mut vm, "(let ((fold (lambda (a b) (compare-strings a nil nil b nil nil t))))
          (list (funcall fold \"é\" \"É\") (funcall fold \"\\351\" \"\\311\") (funcall fold \"a\\351\" \"A\\351\")
                (with-temp-buffer (set-buffer-multibyte nil)
                  (list (funcall fold \"é\" \"É\") (funcall fold \"\\351\" \"\\311\") (funcall fold \"a\\351\" \"A\\351\")
                        (funcall fold \"λ\" \"Λ\") (upcase \"a\\351\") (upcase ?\\351) (upcase ?a)))))"),
        "(t 1 t (1 1 t t #u8(65 233) 233 65))");
}
