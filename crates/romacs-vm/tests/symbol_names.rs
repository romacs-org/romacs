//! Symbol names holding raw-byte characters or the characters VM2's
//! symbol-key escape uses (U+FDD0, Plane 15; crates/romacs-vm/src/symkey.rs)
//! behave as pinned GNU 32.0.50's: looked up and uninterned by symbol or
//! name, found by mapatoms, read back from their printed form and completed
//! over; and printed text that holds those characters, in buffer names,
//! strings, markers and symbols, comes out unchanged.
use romacs_vm::Vm;

fn vm() -> Vm {
    let mut vm = Vm::new();
    vm.install_lisp_prelude().unwrap();
    vm
}

const SETUP: &str = "(progn (setq n (string #xfdd0 #xf0000 #xf0078)) (setq s (intern n)) (setq raw (intern (string ?r #x3fffff))))";

#[test]
fn escape_range_and_raw_symbols_intern_and_unintern_as_gnu() {
    let mut vm = vm();
    vm.eval_source(SETUP).unwrap();
    let form = "(list (eq (intern-soft raw) raw) (eq (intern-soft s) s) (eq (intern-soft n) s)
      (eq (car (read-from-string (prin1-to-string s))) s) (let (found) (mapatoms (lambda (x) (if (eq x s) (setq found t)))) found)
      (eq (car (car (read-from-string (concat \"(\" n \")\")))) s) (eq (car (read-from-string (prin1-to-string raw))) raw)
      (append (try-completion (string ?r) (list (symbol-name raw))) nil)
      (unintern raw nil) (intern-soft (string ?r #x3fffff)) (unintern s nil) (intern-soft n))";
    assert_eq!(vm.eval_source(form).unwrap().display().unwrap(), "(t t t t t t t (114 4194303) t nil t nil)");
}

#[test]
fn completion_over_obarrays_offers_decoded_names_as_gnu() {
    let mut vm = vm();
    let form = "(progn (intern (string #xfdd0 #xf0000 ?q)) (intern (string ?z ?z ?r #x3fffff ?k))
      (list (mapcar (lambda (c) (append c nil)) (all-completions (string #xfdd0) obarray)) (append (try-completion (string ?z ?z ?r) obarray) nil)
            (mapcar (lambda (c) (append c nil)) (all-completions (string ?z ?z ?r) obarray))
            (let ((ob (obarray-make))) (intern (string ?y #x3fffff) ob)
              (list (append (try-completion \"y\" ob) nil) (eq (intern-soft (string ?y #x3fffff) ob) (intern (string ?y #x3fffff) ob))
                    (append (symbol-name (intern-soft (string ?y #x3fffff) ob)) nil)))))";
    assert_eq!(vm.eval_source(form).unwrap().display().unwrap(),
        "(((64976 983040 113)) (122 122 114 4194303 107) ((122 122 114 4194303 107)) ((121 4194303) t (121 4194303)))");
}

#[test]
fn printed_text_keeps_escape_range_characters_as_gnu() {
    let mut vm = vm();
    vm.eval_source(SETUP).unwrap();
    vm.eval_source("(progn (setq b (get-buffer-create n)) (set-buffer b) (insert \"ab\") (setq m (copy-marker 2)))").unwrap();
    let form = "(list (append (prin1-to-string b) nil) (append (format \"%s\" b) nil) (append (prin1-to-string n) nil)
      (append (prin1-to-string m) nil) (append (format \"%S\" m) nil) (append (prin1-to-string s) nil)
      (append (prin1-to-string (list s raw b)) nil) (append (prin1-to-string b t) nil))";
    assert_eq!(vm.eval_source(form).unwrap().display().unwrap(),
        "((35 60 98 117 102 102 101 114 32 64976 983040 983160 62) (64976 983040 983160) (34 64976 983040 983160 34) \
         (35 60 109 97 114 107 101 114 32 97 116 32 50 32 105 110 32 64976 983040 983160 62) \
         (35 60 109 97 114 107 101 114 32 97 116 32 50 32 105 110 32 64976 983040 983160 62) (64976 983040 983160) \
         (40 64976 983040 983160 32 114 4194303 32 35 60 98 117 102 102 101 114 32 64976 983040 983160 62 41) (64976 983040 983160))");
}
