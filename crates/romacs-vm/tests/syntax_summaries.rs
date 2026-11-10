//! `parse-partial-sexp' over texts long enough that the syntax summaries
//! serve it (crates/romacs-vm/src/syntax_index.rs), against pinned GNU:
//! from `point-min' and from the states earlier calls returned, as
//! `syntax-ppss' resumes, before and after edits that keep or flip the
//! state below them. The tables cover two-character comments split across
//! leaves, comment styles, nesting, fences, escaped comment ends, a
//! character that both opens a list and starts a comment, nesting past
//! GNU's 100 levels, closes past the outermost level and multibyte text.
use romacs_vm::Vm;

/// `syn' selects a new table of SPECS; `build' inserts SNIPPET COUNT times,
/// numbered; `probe' parses from 1 to positions across the buffer;
/// `chain' resumes from each previous state in thirds; `edits' makes the
/// same edits in both and probes after each: (POS . STRING) inserts,
/// (POS . N) deletes N characters, (POS put LEN SYNTAX) gives LEN characters
/// a `syntax-table' property, or removes it where SYNTAX is nil.
const PRELUDE: &str = r#"(progn
 (defalias 'syn (lambda (&rest specs) (let ((st (make-char-table 'syntax-table))) (set-char-table-parent st (standard-syntax-table)) (while specs (modify-syntax-entry (car specs) (car (cdr specs)) st) (setq specs (cdr (cdr specs)))) (set-syntax-table st) st)))
 (defalias 'build (lambda (snippet count) (let ((i 0)) (while (< i count) (insert snippet (number-to-string i) "\n") (setq i (1+ i))))))
 (defalias 'probe (lambda () (let ((r nil) (p 1) (n (point-max))) (while (< p n) (setq r (cons (parse-partial-sexp 1 p) r)) (setq p (+ p 2753))) (nreverse (cons (list (parse-partial-sexp 1 n) (point)) r)))))
 (defalias 'chain (lambda () (let* ((n (point-max)) (a (/ n 3)) (b (* 2 a)) (s1 (parse-partial-sexp 1 a)) (s2 (parse-partial-sexp a b nil nil s1)) (s3 (parse-partial-sexp b n nil nil s2))) (list s1 s2 s3 (parse-partial-sexp 7 n nil nil (parse-partial-sexp 1 7))))))
 (defalias 'edits (lambda (insertions) (let ((r (list (probe) (chain)))) (while insertions (goto-char (car (car insertions))) (let ((op (cdr (car insertions)))) (cond ((stringp op) (insert op)) ((integerp op) (delete-char op)) ((nth 2 op) (put-text-property (point) (+ (point) (nth 1 op)) 'syntax-table (string-to-syntax (nth 2 op)))) (t (remove-text-properties (point) (+ (point) (nth 1 op)) '(syntax-table nil))))) (setq r (cons (list (probe) (chain)) r)) (setq insertions (cdr insertions))) (nreverse r))))
 EXPR)"#;

/// One expression per line.
const CASES: &str = r##"
(with-temp-buffer (syn ?/ ". 124b" ?* ". 23" ?\n "> b" ?\" "\"" ?' "\"" ?\\ "\\" ?{ "(}" ?} "){") (build "int f(int a) { /* c (*/ return a + 'x' + \"s(\\\"\"; } // d {\n/* x\n * y\n */ g({ \"multi\nline\" });\n" 900) (edits '((100 . "x") (5000 . "/*") (5002 . -2) (50 . "\"") (51 . -1) (20000 . "\\") (30000 . -300))))
(with-temp-buffer (syn ?/ ". 124b" ?* ". 23" ?\n "> b" ?\" "\"" ?\\ "\\") (setq comment-end-can-be-escaped t) (build "a /* b \\*/ c */ d // e \\\n f\n\"g\\\" h\" (i [j]) \\\\ k\n" 1500) (edits '((300 . "\\") (9000 . "/") (12000 . -5))))
(with-temp-buffer (syn ?\; "<" ?\n ">" ?# "' 14" ?| "\" 23bn" ?? "_ p" ?' "'" ?` "'" ?, "'") (build "(defun f (x) \"doc (\" #| a #| nested ( |# b |# '(x ?\\( ,x `y) ; c (\n  (g x))\n" 1200) (edits '((10 . "#|") (12 . -2) (4000 . "(") (40000 . ";"))))
(with-temp-buffer (syn ?\( "()1n" ?\) ")(4n" ?* ". 23n" ?{ "<" ?} ">" ?' "\"") (build "begin (* a (* b *) c *) f(x, (y)) { d } 'e(' (*) g *) h\n" 1500) (edits '((200 . "(*") (202 . -2) (7000 . "*)"))))
(with-temp-buffer (syn ?| "|" ?! "!" ?\( "()" ?\) ")(") (build "a |b ( c| !d ) e! (f |g) h| i\n" 2500) (edits '((60 . "|") (61 . -1) (25000 . "!"))))
(with-temp-buffer (syn ?\( "()" ?\) ")(") (build "a) b)) (c (d) e\n" 4000) (edits '((5 . ")") (30000 . "(") (40 . -3))))
(with-temp-buffer (syn ?\( "()" ?\) ")(" ?\; "<" ?\n ">") (insert (make-string 150 ?\() "\n") (build "(a ; ) (\n b)\n" 3000) (insert (make-string 160 ?\)) "\n") (build "x (y) z\n" 1000) (edits '((10 . "(") (80 . ")") (2000 . -1))))
(with-temp-buffer (syn ?« "(»" ?» ")«" ?\; "<" ?\n ">") (build "(αβ «γ ; δ» (ε) «ζ\n η» \"θ(\") ι ; κ\n" 2000) (edits '((3 . "λ") (20000 . "«") (50000 . -2))))
(with-temp-buffer (syn ?/ ". 124b" ?* ". 23" ?\n "> b" ?\" "\"" ?< "." ?> ".") (build "template<int N> f(a<b>) { /* x */ \"s\" } // c\nx = a < b > c;\n" 1200) (set (make-local-variable 'parse-sexp-lookup-properties) t) (setq syntax-propertize--done most-positive-fixnum) (goto-char 1) (while (re-search-forward "[<>]" nil t) (put-text-property (1- (point)) (point) 'syntax-table (string-to-syntax (if (eq (char-before) ?<) "(>" ")<")))) (edits '((100 . "<") (9000 . "/*") (9002 . -2) (20000 put 1 "\"") (25000 put 3000 nil) (300 put 1 "<") (40000 put 2 "\"") (60000 . -40))))
"##;

fn run(source: &str) -> (String, Result<String, String>) {
    let binary = std::env::var("ROMACS_GNU_ORACLE").expect("pinned GNU oracle");
    let output = std::process::Command::new(&binary).args(["-Q", "--batch", "--eval", &format!("(prin1 {source})")]).output().unwrap();
    assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
    let mut vm = Vm::new();
    vm.install_lisp_prelude().unwrap();
    (String::from_utf8(output.stdout).unwrap(), vm.eval_source(source).and_then(romacs_vm::Local::display).map_err(|e| format!("{e:?}")))
}

#[test]
fn served_parses_match_pinned_gnu() {
    let mut failures = Vec::new();
    let cases: Vec<&str> = CASES.lines().filter(|line| !line.is_empty()).collect();
    for expression in &cases {
        let source = PRELUDE.replace("EXPR", expression);
        let (expected, actual) = run(&source);
        if actual.as_deref() != Ok(expected.as_str()) {
            let first = expected.chars().zip(actual.as_deref().unwrap_or("").chars()).take_while(|(a, b)| a == b).count();
            failures.push(format!("{expression}\n  first difference at {first}:\n  GNU    …{}\n  Romacs …{}", &expected[first.saturating_sub(80)..(first + 200).min(expected.len())],
                actual.as_deref().map_or_else(Clone::clone, |a| a[first.saturating_sub(80)..(first + 200).min(a.len())].to_owned())));
        }
    }
    assert!(failures.is_empty(), "{} of {} cases differ:\n{}", failures.len(), cases.len(), failures.join("\n"));
}

/// The summaries served those parses, and checked against the scanner
/// they agree; with them inhibited, the scanner alone gives the same.
#[test]
fn summaries_serve_the_long_parses() {
    // `edits' also returns how many parses its buffer's summaries served.
    const COUNTED: &str = "(let ((edits (symbol-function 'edits))) (defalias 'edits (lambda (insertions) (let ((value (funcall edits insertions))) (list value (plist-get (romacs-syntax-summaries-stats) :served))))))";
    for expression in CASES.lines().filter(|line| !line.is_empty()) {
        let run = |setting: &str| -> (String, i64) {
            let source = PRELUDE.replace("EXPR", &format!("(progn {COUNTED} {setting} {expression})"));
            let mut vm = Vm::new();
            vm.install_lisp_prelude().unwrap();
            let shown = vm.eval_source(&source).and_then(romacs_vm::Local::display).unwrap_or_else(|e| panic!("{expression}: {e:?}"));
            let (value, served) = shown.trim_end_matches(')').rsplit_once(' ').expect("(VALUE SERVED)");
            (value.to_owned(), served.parse().expect("a count"))
        };
        let (checked, served) = run("(setq romacs-syntax-summaries-check t)");
        let (plain, none) = run("(setq romacs-inhibit-syntax-summaries t)");
        assert!(served > 20, "{expression}: {served} served");
        assert_eq!(none, 0, "{expression}");
        assert!(checked == plain, "{expression}: served and plain parses differ");
    }
}

/// Two tables holding one sub-char-table: a write through either changes
/// both, so summaries read under the other are not used again.
#[test]
fn a_write_through_a_shared_node_is_seen() {
    let source = r##"(let* ((leaf (car (read-from-string (let ((s "#^^[3 0") (i 0)) (while (< i 128) (setq s (concat s " (2)") i (1+ i))) (concat s "]")))))
            (a (make-char-table 'syntax-table))
            (b (make-char-table 'syntax-table)))
       (set-char-table-range a '(0 . 127) leaf)
       (set-char-table-range b '(0 . 127) leaf)
       (with-temp-buffer
         (set-syntax-table a)
         (insert (make-string 15000 ?x) "y" (make-string 15000 ?x))
         (parse-partial-sexp 1 (point-max))
         (aset b ?y '(4 . 41))
         (list (car (parse-partial-sexp 1 (point-max)))
               (progn (setq romacs-inhibit-syntax-summaries t)
                      (prog1 (car (parse-partial-sexp 1 (point-max))) (setq romacs-inhibit-syntax-summaries nil)))
               (plist-get (romacs-syntax-summaries-stats) :served))))"##;
    let mut vm = Vm::new();
    vm.install_lisp_prelude().unwrap();
    let shown = vm.eval_source(source).and_then(romacs_vm::Local::display).unwrap();
    assert_eq!(shown, "(1 1 2)", "depth served, depth scanned, parses served");
}

/// What `parse-partial-sexp' gives over the current buffer with summaries
/// and then with them inhibited, after FORM ran: "(SERVED PLAIN)".
fn served_and_plain(setup: &str, form: &str) -> String {
    let source = format!(r#"(progn
       (defalias 'rep (lambda (piece n) (let ((s "") (i 0)) (while (< i n) (setq s (concat s " " piece) i (1+ i))) s)))
       (let ((table {setup}))
         (with-temp-buffer
           (set-syntax-table table)
           (insert (make-string 15000 ?x) "y" (make-string 15000 ?x))
           (parse-partial-sexp 1 (point-max))
           {form}
           (list (car (parse-partial-sexp 1 (point-max)))
                 (progn (setq romacs-inhibit-syntax-summaries t)
                        (prog1 (car (parse-partial-sexp 1 (point-max))) (setq romacs-inhibit-syntax-summaries nil)))))))"#);
    let mut vm = Vm::new();
    vm.install_lisp_prelude().unwrap();
    vm.eval_source(&source).and_then(romacs_vm::Local::display).unwrap()
}

/// Two literals holding one detached ASCII node (not reached from their
/// roots), the reader's own reference collected: a write through either is
/// a write into a shared node.
#[test]
fn a_write_through_a_shared_detached_ascii_node_is_seen() {
    let setup = r#"(let ((tables (car (read-from-string (concat "(#^[nil nil syntax-table #1=#^^[3 0" (rep "(2)" 128) "]" (rep "(2)" 64)
                                                         "] #^[nil nil syntax-table #1#" (rep "(2)" 64) "])")))))
                      (setq other (car (cdr tables)))
                      (garbage-collect)
                      (car tables))"#;
    assert_eq!(served_and_plain(setup, "(aset other ?y '(4 . 41))"), "(1 1)");
}

/// `optimize-char-table' reads ASCII from the root again, which changes a
/// literal whose detached ASCII node said otherwise.
#[test]
fn an_optimized_table_is_read_again() {
    let setup = r##"(car (read-from-string (concat "#^[nil nil syntax-table #^^[3 0" (rep "(2)" 121) " (4 . 41)" (rep "(2)" 6) "]" (rep "(2)" 64) "]")))"##;
    assert_eq!(served_and_plain(setup, "(optimize-char-table table)"), "(0 0)");
}

/// Long parses that meet text not yet propertized are the scanner's,
/// stopping for `internal--syntax-propertize' where GNU's does: the calls,
/// what they change before the stop, and the values are GNU's.
#[test]
fn propertizing_parses_match_pinned_gnu() {
    let expression = r#"(with-temp-buffer
       (syn ?/ ". 124b" ?* ". 23" ?\n "> b" ?\" "\"")
       (build "f(a) { /* (b */ \"c(\" } // d (\n" 900)
       (set (make-local-variable 'parse-sexp-lookup-properties) t)
       (let ((log nil))
         (fset 'internal--syntax-propertize
               (lambda (pos) (setq log (cons (list pos syntax-propertize--done (point)) log))
                 (setq syntax-propertize--done (min (point-max) (+ pos 5000)))))
         (setq syntax-propertize--done 10)
         (list (parse-partial-sexp 1 (point-max)) (point) (nreverse log)
               (progn (setq log nil syntax-propertize--done 20000)
                      (list (parse-partial-sexp 2000 (point-max)) (point) (nreverse log)))
               (progn (setq log nil syntax-propertize--done (point-max))
                      (goto-char 100) (insert "x")
                      (list (parse-partial-sexp 1 (point-max)) (point) (nreverse log))))))"#;
    let source = PRELUDE.replace("EXPR", expression);
    let (expected, actual) = run(&source);
    assert_eq!(actual.as_deref(), Ok(expected.as_str()));
    // A propertization that changes syntax the scan already read, before
    // `syntax-propertize--done', as `syntax-propertize-wholelines' extends
    // a region back to its line's start.
    let rewritten = r#"(with-temp-buffer
       (insert (make-string 9998 ?x) "<<" (make-string 20000 ?x))
       (set (make-local-variable 'parse-sexp-lookup-properties) t)
       (fset 'internal--syntax-propertize
             (lambda (_pos) (put-text-property 9999 10001 'syntax-table (string-to-syntax "("))
               (setq syntax-propertize--done (point-max))))
       (setq syntax-propertize--done 10000)
       (list (parse-partial-sexp 1 (point-max)) (point)))"#;
    let (expected, actual) = run(&PRELUDE.replace("EXPR", rewritten));
    assert_eq!(actual.as_deref(), Ok(expected.as_str()));
}
