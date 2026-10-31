//! A unibyte buffer keeps its local keymap as a text buffer does: through
//! `use-local-map', `current-local-map', `key-binding', the keymaps of the
//! text at point and `set-buffer-multibyte' either way, and
//! `kill-all-local-variables' drops it (keymap.c, buffer.c BVAR keymap).
use romacs_vm::Vm;

#[test]
fn a_unibyte_buffer_keeps_and_answers_its_local_map_as_gnu() {
    let oracle = std::env::var("ROMACS_GNU_ORACLE").expect("pinned GNU oracle");
    let expression = r#"(let ((m (make-sparse-keymap)) (other (make-sparse-keymap)) (text (make-sparse-keymap)))
  (define-key m "a" 'ka) (define-key other "b" 'kb) (define-key text "c" 'kc)
  (with-temp-buffer
    (use-local-map m)
    (set-buffer-multibyte nil)
    (list (eq (current-local-map) m)
          (key-binding "a")
          (progn (use-local-map other) (eq (current-local-map) other))
          (key-binding "b")
          (progn (insert "xy") (add-text-properties 1 3 (list 'keymap text 'local-map m)) (goto-char 2)
                 (list (where-is-internal (quote ka) nil t) (where-is-internal (quote kb) nil t)))
          (where-is-internal 'kc nil t)
          (progn (set-buffer-multibyte t) (eq (current-local-map) other))
          (progn (set-buffer-multibyte nil) (kill-all-local-variables) (current-local-map)))))"#;
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
}
