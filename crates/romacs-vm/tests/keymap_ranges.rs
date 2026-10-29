use romacs_vm::Vm;

/// store_in_keymap (keymap.c:730) with a (FROM . TO) character range, the
/// key map-keymap hands its function for a char-table run: the keymap's
/// char-table takes the range, alist bindings inside it are set or removed,
/// a keymap without a char-table gains one, and lookups and prefix keys use
/// the range's EVENT_HEAD. A range that is not the last event is refused
/// (GNU's refusal reaches message_with_string with the cons, which signals
/// an accidental wrong-type-argument stringp; VM2 signals the message).
#[test]
fn character_ranges_store_as_gnus_store_in_keymap() {
    let oracle = std::env::var("ROMACS_GNU_ORACLE").expect("pinned GNU oracle");
    let expression = r#"(list
 (let ((m (make-sparse-keymap))) (define-key m [(?a . ?c)] 'x) (list (lookup-key m "a") (lookup-key m "c") (lookup-key m "d") (char-table-p (car (cdr m))) (length m)))
 (let ((m (make-keymap))) (define-key m [(?a . ?c)] 'x) (list (lookup-key m "b") (length m) (aref (car (cdr m)) ?c)))
 (let ((m (make-sparse-keymap))) (define-key m "b" 'old) (define-key m [(?a . ?c)] 'new) (list (lookup-key m "b") (lookup-key m "a") (length m)))
 (let ((m (make-sparse-keymap))) (define-key m "b" 'old) (define-key m [(?b . ?b)] 'new) (list (lookup-key m "b") (length m)))
 (let ((m (make-keymap))) (define-key m [(?a . ?c)] nil) (list (aref (car (cdr m)) ?a) (lookup-key m "a")))
 (let ((m (make-keymap))) (define-key m [(?a . ?c)] 'x) (define-key m [(?a . ?b)] nil t) (list (aref (car (cdr m)) ?a) (aref (car (cdr m)) ?c)))
 (let ((m (make-sparse-keymap))) (define-key m "a" 'x) (define-key m "b" 'y) (define-key m "d" 'z) (define-key m [(?a . ?b)] nil t) m)
 (let ((src (make-keymap)) (m (make-keymap))) (define-key src [(?0 . ?9)] 'digit) (map-keymap (lambda (k b) (define-key m (vector k) b)) src) (list (lookup-key m "5") (lookup-key m "a")))
 (let ((m (make-sparse-keymap))) (define-key m [(?a . ?c)] 'x) (lookup-key m [(?b . ?c)]))
 (condition-case e (define-key (make-sparse-keymap) [(?a . foo)] 'x) (error e)))"#;
    let output = std::process::Command::new(oracle)
        .args(["-Q", "--batch", "--eval", &format!("(princ (prin1-to-string {expression}))")])
        .stdin(std::process::Stdio::null())
        .output().unwrap();
    assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
    let gnu = String::from_utf8(output.stdout).unwrap();
    let mut vm = Vm::new();
    vm.install_lisp_prelude().unwrap();
    let mine = vm.eval_source(&format!("(prin1-to-string {expression})")).unwrap().display().unwrap();
    assert_eq!(mine, format!("{gnu:?}"));
    let prefix = "(condition-case e (define-key (make-sparse-keymap) [(?a . ?c) ?x] 'x) (error (car e)))";
    assert_eq!(vm.eval_source(prefix).unwrap().display().unwrap(), "error");
}
