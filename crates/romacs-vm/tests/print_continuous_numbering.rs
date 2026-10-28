//! print-circle labels, print-continuous-numbering and print-number-table
//! against pinned GNU: labels follow GNU's preprocessing order, and with
//! continuous numbering the table survives between print calls, `princ'
//! included; an override binds numbering for one call.
use romacs_vm::Vm;

const CASES: &str = r##"
(let ((print-circle t) (a (list 1)) (b (list 2))) (prin1-to-string (list a b b a)))
(let ((print-circle t) (a (list 1)) (b (list 2))) (prin1-to-string (list a (vector b a) b)))
(let ((print-circle t) (a (list 1))) (prin1-to-string (let ((x (list a a))) (setcdr (cdr x) x) x)))
(let ((print-circle t) (print-continuous-numbering t) print-number-table) (prin1-to-string (list "s" "s")))
(let ((print-circle t) (print-continuous-numbering t) print-number-table (a (list 1))) (list (prin1-to-string (list a a)) (hash-table-p print-number-table) (gethash a print-number-table)))
(let ((print-circle t) (print-continuous-numbering t) print-number-table (a (list 1)) (b (list 2))) (list (prin1-to-string (list a b a)) (prin1-to-string (list b a b)) (prin1-to-string a) (hash-table-count print-number-table) (gethash a print-number-table) (gethash b print-number-table)))
(let ((print-circle t) (print-continuous-numbering t) print-number-table (a (list 1))) (list (prin1-to-string (list a a)) (let ((print-circle nil)) (prin1-to-string (list a a))) (prin1-to-string a)))
(let ((print-circle t) (print-continuous-numbering t) print-number-table (a (list 1))) (prin1-to-string (list a a)) (let ((print-continuous-numbering nil)) (prin1-to-string (list a a))))
(let ((print-circle t) (print-continuous-numbering t) print-number-table (a (list 1))) (prin1-to-string (list a a)) (prin1-to-string (list a a) t))
(let ((print-circle t) (print-continuous-numbering t) print-number-table (a (list 1))) (prin1-to-string (list a a)) (list (prin1-to-string (list a a) nil t) (prin1-to-string (list a a) nil '((circle . t)))))
(let ((print-circle t) (v (vector)) (s "")) (prin1-to-string (list v v s s)))
(let ((a (list 1))) (list (let ((print-circle t)) (prin1-to-string (list a a)) (hash-table-count print-number-table)) (let ((print-circle t)) (prin1-to-string 1) print-number-table)))
(let ((print-circle t) (print-continuous-numbering t) (print-number-table (make-hash-table :test 'eq)) (a (list 1)) (b (list 2))) (puthash a 7 print-number-table) (puthash b "#$" print-number-table) (list (prin1-to-string (list a b (cons 0 b))) (hash-table-count print-number-table)))
(let ((print-circle t) (print-continuous-numbering t) (print-number-table (make-hash-table :test 'eq)) (a (list 1)) (b (list 2))) (puthash a -7 print-number-table) (puthash b t print-number-table) (list (prin1-to-string (list a b)) (gethash a print-number-table) (gethash b print-number-table 'none)))
(let (print-circle (print-continuous-numbering t) print-number-table (a (list 1))) (list (prin1-to-string (list a a)) print-number-table))
(let ((print-circle t) (print-continuous-numbering t) print-number-table (s (make-symbol "g")) (print-gensym t)) (list (prin1-to-string (list s)) (prin1-to-string s)))
(let ((print-circle t) (print-continuous-numbering t) print-number-table (a (list 1))) (with-temp-buffer (prin1 (list a a) (current-buffer)) (prin1 a (current-buffer)) (buffer-string)))
(let ((print-circle t) (print-continuous-numbering t) print-number-table (s "hi")) (list (prin1-to-string (list s s)) (with-temp-buffer (let ((standard-output (current-buffer))) (princ s)) (buffer-string)) (let (acc) (princ s (lambda (c) (push c acc))) (concat (nreverse acc))) (with-temp-buffer (princ s (current-buffer)) (princ "\377" (current-buffer)) (buffer-string))))
(let ((print-circle t) (print-continuous-numbering t) print-number-table (a (list 1))) (prin1-to-string (list a a)) (list (prin1-to-string (list a a) nil '(t (circle . t))) (prin1-to-string (list a a)) (hash-table-count print-number-table)))
(let ((print-circle t) (print-continuous-numbering t) print-number-table (a (list 1)) (b (list 2))) (list (prin1-to-string (list a a)) (prin1-to-string (list b b) nil t) (prin1-to-string (list b b)) (gethash a print-number-table) (gethash b print-number-table)))
(let ((print-circle t) print-number-table (a (list 1))) (list (prin1-to-string (list a a) nil '((continuous-numbering . t))) (prin1-to-string a) (hash-table-p print-number-table) (gethash a print-number-table)))
(let ((print-circle t) (print-continuous-numbering t) print-number-table (a (list 1))) (prin1-to-string (list a a)) (list (prin1-to-string (list a a) nil '((continuous-numbering))) (gethash a print-number-table)))
(let ((print-circle t) (print-continuous-numbering t) print-number-table (tb (make-hash-table :test 'eq)) (a (list 1))) (list (prin1-to-string (list a a) nil `((number-table . ,tb))) (prin1-to-string a nil `((number-table . ,tb))) (gethash a tb) print-number-table (prin1-to-string a)))
(let ((print-circle t) (print-continuous-numbering t) (tb (make-hash-table :test 'eq)) (a (list 1))) (puthash a 3 tb) (let ((print-number-table tb)) (prin1-to-string (list a a) nil '((number-table)))))
(let ((print-circle t) (print-continuous-numbering t) print-number-table (a (list 1))) (list (prin1-to-string (list a a)) (with-temp-buffer (let ((print-continuous-numbering nil)) (princ "\377" (current-buffer))) (buffer-string)) (prin1-to-string (list a a)) (gethash a print-number-table)))
(list (prin1-to-string "\377" t) (prin1-to-string (list "\377" (string-to-multibyte "\377")) t) (with-temp-buffer (princ "\377" (current-buffer)) (princ (vector "\377" "é") (current-buffer)) (buffer-string)) (let (acc) (princ "\377" (lambda (c) (push c acc))) acc))
"##;

fn oracle(expression: &str) -> String {
    let binary = std::env::var("ROMACS_GNU_ORACLE").expect("pinned GNU oracle");
    let source = format!("(prin1 (let ((print-circle nil)) (prin1-to-string {expression})))");
    let output = std::process::Command::new(&binary).args(["-Q", "--batch", "--eval", &source])
        .env("LANG", "C.UTF-8").output().unwrap();
    assert!(output.status.success(), "{expression}: {}", String::from_utf8_lossy(&output.stderr));
    String::from_utf8(output.stdout).unwrap()
}

fn romacs(expression: &str) -> String {
    let source = format!("(let ((print-circle nil)) (prin1-to-string {expression}))");
    match Vm::new().eval_source(&source).and_then(|v| v.display()) {
        Ok(text) => text,
        Err(error) => format!("VM error: {error}"),
    }
}

#[test]
fn print_number_table_matches_gnu() {
    let failures: Vec<String> = CASES.lines().filter(|line| !line.is_empty())
        .filter_map(|expression| {
            let (gnu, ours) = (oracle(expression), romacs(expression));
            (gnu != ours).then(|| format!("{expression}\n  gnu:    {gnu}\n  romacs: {ours}"))
        })
        .collect();
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}
