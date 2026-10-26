//! C primitives that GNU's preloaded Lisp (custom, faces, frame, startup,
//! mule-conf, mule-cmds, url) reaches at load time, compared with the pinned
//! GNU oracle in batch. Each expression is self-contained primitive Lisp.
use romacs_vm::Vm;

fn oracle(source: &str) -> String {
    let binary = std::env::var("ROMACS_GNU_ORACLE").expect("pinned GNU oracle");
    let output = std::process::Command::new(binary).args(["-Q", "--batch", "--eval", &format!("(princ {source})")])
        .output().unwrap();
    assert!(output.status.success(), "{source}: {}", String::from_utf8_lossy(&output.stderr));
    String::from_utf8(output.stdout).unwrap()
}

fn romacs(vm: &mut Vm, source: &str) -> String {
    let printed = vm.eval_source(source).and_then(|v| v.display()).unwrap_or_else(|e| panic!("{source}: {e}"));
    serde_json::from_str(&printed).unwrap_or(printed)
}

fn compare(expressions: &[&str]) {
    let mut failures = Vec::new();
    for expression in expressions {
        let source = format!("(prin1-to-string (condition-case err {expression} (error (list 'signal err))))");
        let mut vm = Vm::new();
        // GNU batch: the echo area stays empty and faces are not probed.
        vm.eval_source("(setq noninteractive t)").unwrap();
        let (expected, actual) = (oracle(&source), romacs(&mut vm, &source));
        if expected != actual { failures.push(format!("{expression}\n  GNU    {expected}\n  Romacs {actual}")); }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

#[test]
fn bitwise_shift_and_complement_match_gnu() {
    compare(&[
        "(list (ash 1 70) (ash -5 -1) (ash 5 -100) (ash -5 -100) (ash 0 100000) (ash 7 0))",
        "(ash 1 100000)", "(ash 'x 1)", "(ash 1 1.0)",
        "(list (logxor) (logxor 5) (logxor 5 3) (logxor -1 (ash 1 80)) (lognot 0) (lognot (ash 1 70)))",
        "(lognot 1.5)", "(logxor 1 'a)",
    ]);
}

#[test]
fn assoc_string_matches_gnu() {
    compare(&[
        "(assoc-string \"Foo\" (list \"x\" (cons 'foo 1)) t)",
        "(assoc-string 'b (list \"a\" \"b\"))", "(assoc-string \"ab\" (list \"a\"))",
        "(assoc-string 1 nil)", "(assoc-string 1 (list \"a\"))", "(assoc-string \"a\" (list 1 'a))",
        "(assoc-string \"É\" (list \"é\") t)",
    ]);
}

#[test]
fn user_identity_matches_gnu() {
    compare(&[
        "(list (user-login-name) (user-real-login-name) (user-uid) (user-real-uid))",
        "(list user-login-name user-real-login-name (stringp user-full-name))",
        "(list (user-login-name 0) (user-full-name \"root\") (user-login-name 999999) (user-full-name 999999))",
        "(user-full-name 'x)",
    ]);
}

#[test]
fn file_predicates_match_gnu() {
    compare(&[
        "(list (file-exists-p \"/etc/passwd\") (file-exists-p \"/nonexistent\") (file-readable-p \"/etc/passwd\"))",
        "(list (file-executable-p \"/bin/sh\") (file-executable-p \"/etc/passwd\") (file-executable-p \"/etc/\"))",
        "(list (file-regular-p \"/etc/passwd\") (file-regular-p \"/etc/\") (file-regular-p \"/etc/passwd/\"))",
        "(file-exists-p 7)",
    ]);
}

#[test]
fn hooks_until_success_and_failure_match_gnu() {
    compare(&[
        "(progn (setq h1 (list (lambda (x) nil) (lambda (x) (* x 2)) (lambda (x) 99))) (list (run-hook-with-args-until-success 'h1 4) (run-hook-with-args-until-failure 'h1 4)))",
        "(progn (setq h2 nil) (list (run-hook-with-args-until-success 'h2) (run-hook-with-args-until-failure 'h2)))",
        "(progn (setq h3 (lambda (&rest a) a)) (run-hook-with-args-until-success 'h3 1 2))",
    ]);
}

#[test]
fn funcall_is_a_callable_subr() {
    compare(&[
        "(apply #'funcall (list #'+ 1 2))", "(apply #'funcall (list (lambda () 1)))",
        "(funcall #'funcall #'list 1 2)", "(funcall #'funcall 'no-such-function)",
    ]);
}

#[test]
fn backtrace_frame_internal_matches_gnu() {
    compare(&[
        "(backtrace-frame--internal (lambda (evald f args flags) (list evald f (length args) (nth 1 args))) 0 'backtrace-frame--internal)",
        "(backtrace-frame--internal #'list 100000 nil)",
        "(backtrace-frame--internal #'list -1 nil)",
    ]);
}

#[test]
fn frame_parameters_and_display_queries_match_gnu() {
    compare(&[
        "(frame-parameter nil 'name)", "(frame-parameter nil 'no-such)", "(frame-parameter nil \"name\")",
        "(progn (modify-frame-parameters nil '((a . 1) (b . 2) (a . 3))) (list (frame-parameter nil 'a) (frame-parameter nil 'b)))",
        "(modify-frame-parameters nil '(x))", "(modify-frame-parameters nil 'x)",
        "(list (window-system) (current-message) (display-supports-face-attributes-p '(:weight bold)))",
        "(list (internal-char-font nil 8230) (internal-char-font nil ?a))", "(internal-char-font nil -1)",
        "(list (internal-show-cursor-p) (progn (internal-show-cursor nil nil) (internal-show-cursor-p)) (progn (internal-show-cursor nil t) (internal-show-cursor-p)))",
        "(internal-show-cursor 'x t)",
        "(list (progn (set-keyboard-coding-system-internal 'undecided) (keyboard-coding-system)) (progn (set-keyboard-coding-system-internal nil) (keyboard-coding-system)))",
        "(progn (set-terminal-coding-system-internal 'undecided) (terminal-coding-system))",
    ]);
}

#[test]
fn documentation_reads_primitive_docstrings_from_doc() {
    compare(&[
        // simple.el's function-documentation answers a subr with its DOC
        // position; substitute-command-keys (help.el) is skipped with RAW.
        "(progn (defalias 'function-documentation #'internal-subr-documentation) (list (documentation 'car t) (documentation 'cons t)))",
        "(numberp (internal-subr-documentation (symbol-function 'car)))", "(internal-subr-documentation 'car)",
        "(progn (defalias 'function-documentation #'internal-subr-documentation) (documentation 'no-such-function-anywhere t))",
        "(documentation-property 'x 'y)",
        "(progn (put 'x 'y \"plain\") (documentation-property 'x 'y t))",
        "(progn (put 'x 'z '(concat \"a\" \"b\")) (documentation-property 'x 'z t))",
    ]);
}

#[test]
fn charsetp_and_unification_match_gnu() {
    compare(&[
        "(list (charsetp 'ascii) (charsetp 'unicode) (charsetp 'no-such) (charsetp \"ascii\"))",
        "(unify-charset 'no-such)", "(unify-charset 'ascii)",
    ]);
}

#[test]
fn coding_system_priority_follows_gnu_categories() {
    // A bare VM defines only syms_of_coding's two systems; GNU's dump has all
    // of mule-conf.el. Compare what both define the same way.
    compare(&[
        "(progn (set-coding-system-priority 'no-conversion) (car (coding-system-priority-list)))",
        "(progn (set-coding-system-priority 'undecided 'no-conversion) (list (coding-system-priority-list t) coding-category-raw-text))",
        "(set-coding-system-priority 'no-such-coding)",
        "(progn (set-safe-terminal-coding-system-internal 'undecided) 'ok)", "(set-safe-terminal-coding-system-internal 7)",
    ]);
}

#[test]
fn charset_priority_moves_named_charsets_first() {
    compare(&[
        "(progn (set-charset-priority 'unicode) (charset-priority-list t))",
        "(progn (set-charset-priority 'ascii 'unicode) (let ((l (charset-priority-list))) (list (nth 0 l) (nth 1 l))))",
        "(set-charset-priority 'no-such)",
    ]);
}

#[test]
fn variable_watchers_follow_gnu_registration() {
    compare(&[
        "(progn (defvar ww 1) (add-variable-watcher 'ww #'set-buffer-redisplay) (add-variable-watcher 'ww #'set-buffer-redisplay) (setq ww 2) (let ((ww 3)) ww) (list ww (get-variable-watchers 'ww) (get 'ww 'watchers)))",
        "(progn (defvar wx 1) (add-variable-watcher 'wx #'set-buffer-redisplay) (remove-variable-watcher 'wx #'set-buffer-redisplay) (list (get-variable-watchers 'wx) (get 'wx 'watchers)))",
        "(get-variable-watchers 'never-watched)",
    ]);
}

#[test]
fn lisp_variable_watchers_are_refused_explicitly() {
    let mut vm = Vm::new();
    let refused = romacs(&mut vm, "(condition-case e (add-variable-watcher 'wy (lambda (&rest _) nil)) (error (car (cdr e))))");
    assert_eq!(refused, "VM2 add-variable-watcher with a Lisp watch function is unsupported");
    assert_eq!(romacs(&mut vm, "(get 'wy 'watchers)"), "nil", "a refused watcher is not recorded");
}

#[test]
fn face_attribute_setter_validates_like_gnu() {
    let setup = "(progn (internal-make-lisp-face 'tf) (internal-make-lisp-face 'tf (selected-frame)) (internal-make-lisp-face 'tg) (internal-make-lisp-face 'tg (selected-frame)))";
    let cases = [
        "(internal-set-lisp-face-attribute 'tf :foreground \"red\" nil)",
        "(progn (internal-set-lisp-face-attribute 'tf :foreground \"red\" nil) (internal-get-lisp-face-attribute 'tf :foreground (selected-frame)))",
        "(progn (internal-set-lisp-face-attribute 'tf :foreground nil nil) (internal-get-lisp-face-attribute 'tf :foreground (selected-frame)))",
        "(internal-set-lisp-face-attribute 'tf :foreground \"\" nil)", "(internal-set-lisp-face-attribute 'tf :foreground 7 nil)",
        "(progn (internal-set-lisp-face-attribute 'tf :weight 'BOLD nil) (internal-get-lisp-face-attribute 'tf :weight (selected-frame)))",
        "(internal-set-lisp-face-attribute 'tf :weight 'heavyish nil)", "(internal-set-lisp-face-attribute 'tf :slant \"italic\" nil)",
        "(internal-set-lisp-face-attribute 'tf :height 0 nil)", "(internal-set-lisp-face-attribute 'tf :height 1.5 nil)",
        "(internal-set-lisp-face-attribute 'tf :height 0.01 nil)", "(internal-set-lisp-face-attribute 'default :height 1.5 nil)",
        "(internal-set-lisp-face-attribute 'tf :underline '(:style wave :color \"red\") nil)",
        "(internal-set-lisp-face-attribute 'tf :underline '(:style zigzag) nil)", "(internal-set-lisp-face-attribute 'tf :underline 'foo nil)",
        "(progn (internal-set-lisp-face-attribute 'tf :box t nil) (internal-get-lisp-face-attribute 'tf :box (selected-frame)))",
        "(internal-set-lisp-face-attribute 'tf :box 0 nil)", "(internal-set-lisp-face-attribute 'tf :box '(:line-width (2 . 2) :style flat-button) nil)",
        "(internal-set-lisp-face-attribute 'tf :box '(:line-width 0) nil)", "(internal-set-lisp-face-attribute 'tf :overline 'blue nil)",
        "(internal-set-lisp-face-attribute 'tf :overline 7 nil)", "(internal-set-lisp-face-attribute 'tf :inverse-video 'x nil)",
        "(internal-set-lisp-face-attribute 'tf :extend 7 nil)",
        "(progn (internal-set-lisp-face-attribute 'tf :bold nil nil) (internal-get-lisp-face-attribute 'tf :weight (selected-frame)))",
        "(progn (internal-set-lisp-face-attribute 'tf :italic t nil) (internal-get-lisp-face-attribute 'tf :slant (selected-frame)))",
        "(internal-set-lisp-face-attribute 'tf :inherit 'tf nil)", "(internal-set-lisp-face-attribute 'tf :inherit '(tg 7) nil)",
        "(progn (internal-set-lisp-face-attribute 'tg :inherit 'tf nil) (internal-set-lisp-face-attribute 'tf :inherit 'tg nil))",
        "(internal-set-lisp-face-attribute 'tf :stipple '(2 2 \"ab\") nil)", "(internal-set-lisp-face-attribute 'tf :stipple 7 nil)",
        "(internal-set-lisp-face-attribute 'tf :no-such 1 nil)", "(internal-set-lisp-face-attribute 'no-such-face :foreground \"red\" t)",
        "(progn (internal-set-lisp-face-attribute 'tf :foreground 'unspecified t) (internal-get-lisp-face-attribute 'tf :foreground t))",
        "(internal-set-lisp-face-attribute \"tf\" :foreground \"red\" nil)",
    ];
    let wrapped: Vec<String> = cases.iter().map(|c| format!("(progn {setup} {c})")).collect();
    let refs: Vec<&str> = wrapped.iter().map(String::as_str).collect();
    compare(&refs);
}

#[test]
fn doc_file_is_the_pinned_gnu_build_artifact() {
    // etc/DOC is GNU's own make-docfile output for the pinned build; its
    // offsets are what internal-subr-documentation reports.
    let oracle = std::env::var("ROMACS_GNU_ORACLE").expect("pinned GNU oracle");
    let installed = std::path::Path::new(&oracle).parent().unwrap().join("../share/emacs/32.0.50/etc/DOC");
    let ours = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../etc/DOC");
    assert_eq!(std::fs::read(ours).unwrap(), std::fs::read(installed).unwrap(), "etc/DOC differs from the pinned GNU build");
}

#[test]
fn review_regressions_match_gnu() {
    compare(&[
        // A bad argument leaves the charset ordering untouched.
        "(progn (condition-case nil (set-charset-priority 'unicode 'no-such) (error nil)) (and (memq 'unicode (charset-priority-list)) t))",
        // A watcher's own signal reaches the writer: set, let, unlet, set-default, kill-local-variable.
        "(progn (defvar wv1 0) (add-variable-watcher 'wv1 #'user-uid) (condition-case e (setq wv1 1) (wrong-number-of-arguments 'set-caught)))",
        "(progn (defvar wv2 0) (add-variable-watcher 'wv2 #'user-uid) (condition-case e (let ((wv2 1)) 'body) (wrong-number-of-arguments 'let-caught)))",
        "(progn (defvar wv3 0) (condition-case e (let ((wv3 1)) (add-variable-watcher 'wv3 #'user-uid) 'body) (wrong-number-of-arguments 'unlet-caught)))",
        "(progn (defvar wv4 0) (add-variable-watcher 'wv4 #'user-uid) (condition-case e (set-default 'wv4 1) (wrong-number-of-arguments 'default-caught)))",
        "(progn (defvar wv5 0) (with-temp-buffer (set (make-local-variable 'wv5) 1) (add-variable-watcher 'wv5 #'user-uid) (condition-case e (kill-local-variable 'wv5) (wrong-number-of-arguments 'kill-caught))))",
        // An evaluated documentation property is the documentation itself.
        "(progn (put 'xd 'yd '(+ 1 2)) (documentation-property 'xd 'yd t))",
        "(progn (put 'xd 'zd '(quote 42)) (documentation-property 'xd 'zd))",
        // The cycle check stops at a list member that is not a face.
        "(progn (internal-make-lisp-face 'ta) (internal-make-lisp-face 'tb) (internal-set-lisp-face-attribute 'tb :inherit 'ta t) (internal-set-lisp-face-attribute 'ta :inherit '(no-such-face tb) t))",
        // Frames without a window system report GNU's computed parameters.
        "(list (frame-parameter nil 'font) (frame-parameter nil 'foreground-color) (frame-parameter nil 'background-color) (frame-parameter nil 'modeline) (frame-parameter nil 'unsplittable) (frame-parameter nil 'tab-bar-lines) (frame-parameter nil 'visibility) (frame-parameter nil 'no-accept-focus) (frame-parameter nil 'buried-buffer-list))",
    ]);
}

#[test]
fn copy_keymap_matches_gnu() {
    compare(&[
        // Sub-keymaps and menu items are fresh; commands and symbol prefixes are shared.
        "(let* ((sub (list 'keymap (cons 1 'cmd))) (m (list 'keymap (cons 2 sub) (cons 3 (list 'menu-item \"Name\" sub :enable t)) (cons 4 (cons \"Str\" (cons \"Help\" sub))) (cons 5 'Control-X-prefix))) (c (copy-keymap m))) (list (equal c m) (eq (cdr (assq 2 c)) sub) (eq (nth 3 (cdr (assq 3 c))) sub) (eq (cdr (cdr (cdr (assq 4 c)))) sub) (eq (cdr (assq 5 c)) 'Control-X-prefix)))",
        "(let* ((m (make-keymap)) (sub (list 'keymap))) (aset (nth 1 m) ?a sub) (let ((c (copy-keymap m))) (list (eq (nth 1 c) (nth 1 m)) (equal (aref (nth 1 c) ?a) sub) (eq (aref (nth 1 c) ?a) sub))))",
        "(let* ((parent (list 'keymap)) (m (list 'keymap (cons 1 'a) 'keymap))) (setcdr (cdr (cdr m)) (cdr parent)) (let ((c (copy-keymap m))) (length c)))",
        "(copy-keymap 'not-a-keymap)", "(copy-keymap 7)",
        "(let ((v (vector nil (list 'keymap)))) (let ((c (copy-keymap (list 'keymap v)))) (list (eq (nth 1 c) v) (equal (nth 1 c) v))))",
    ]);
}

#[test]
fn review_two_regressions_match_gnu() {
    compare(&[
        // A char-table default that is a sub-keymap is copied.
        "(let* ((m (make-keymap)) (sub (list 'keymap (cons 1 'old)))) (set-char-table-range (nth 1 m) nil sub) (let ((c (copy-keymap m))) (setcdr (car (cdr (aref (nth 1 c) ?z))) 'new) (cdr (car (cdr sub)))))",
        // C specials bind dynamically even before their symbols are interned.
        "(let ((inhibit-message t)) (symbol-value 'inhibit-message))",
        // set and makunbound pass a watcher's own signal through.
        "(progn (defvar wv6 0) (add-variable-watcher 'wv6 #'user-uid) (condition-case e (set 'wv6 1) (wrong-number-of-arguments 'set-caught)))",
        "(progn (defvar wv7 0) (add-variable-watcher 'wv7 #'user-uid) (condition-case e (makunbound 'wv7) (wrong-number-of-arguments 'unbound-caught)))",
        // kill-local-variable of a plain variable does not notify.
        "(progn (defvar wv8 0) (add-variable-watcher 'wv8 #'user-uid) (kill-local-variable 'wv8))",
    ]);
}

#[test]
fn c_specials_bind_dynamically_in_already_read_forms() {
    // JSON forms are compiled before any reader interns their symbols.
    let mut vm = Vm::new();
    let form = romacs_vm::Form::from_json(serde_json::json!(
        ["let", [["inhibit-message", "t"]], ["symbol-value", ["quote", "inhibit-message"]]])).unwrap();
    let id = vm.compile(&form).unwrap();
    assert_eq!(vm.execute(id).unwrap().display().unwrap(), "t");
}

#[test]
fn review_three_regressions_match_gnu() {
    compare(&[
        // Automatically buffer-local C variables stay per buffer.
        "(list (with-temp-buffer (setq syntax-propertize--done 100)) (with-temp-buffer syntax-propertize--done) (local-variable-if-set-p 'fontification-functions))",
        // Bindings that share one prefix map stay independent after copying.
        "(let* ((sub (list 'keymap (cons 1 'old))) (m (make-keymap))) (aset (nth 1 m) ?a sub) (aset (nth 1 m) ?c sub) (let ((c (copy-keymap m))) (setcdr (car (cdr (aref (nth 1 c) ?a))) 'new) (list (cdr (car (cdr (aref (nth 1 c) ?c)))) (cdr (car (cdr sub))))))",
        // An out-of-range high limb never names another account.
        "(user-login-name (cons (ash 1 120) 0))",
    ]);
    let mut vm = Vm::new();
    let form = romacs_vm::Form::from_json(serde_json::json!(
        ["let", [["exec-path", ["quote", ["chosen"]]]], ["symbol-value", ["quote", "exec-path"]]])).unwrap();
    let id = vm.compile(&form).unwrap();
    assert_eq!(vm.execute(id).unwrap().display().unwrap(), "(chosen)");
}

#[test]
fn category_tables_match_gnu() {
    compare(&[
        "(let ((tb (make-category-table))) (list (category-table-p tb) (category-table-p (make-char-table 'x)) (char-table-subtype tb) (length (char-table-extra-slot tb 0)) (get-unused-category tb)))",
        "(let ((tb (make-category-table))) (define-category ?a \"alpha\" tb) (list (category-docstring ?a tb) (get-unused-category tb) (condition-case e (define-category ?a \"again\" tb) (error e))))",
        "(let ((tb (make-category-table))) (define-category ?a \"alpha\" tb) (define-category ?b \"beta\" tb) (modify-category-entry '(?x . ?z) ?a tb) (modify-category-entry ?y ?b tb) (with-temp-buffer (set-category-table tb) (list (category-set-mnemonics (char-category-set ?x)) (category-set-mnemonics (char-category-set ?y)) (category-set-mnemonics (char-category-set ?w)) (eq (char-category-set ?x) (char-category-set ?z)))))",
        "(let ((tb (make-category-table))) (define-category ?a \"alpha\" tb) (modify-category-entry '(#x4e00 . #x9fff) ?a tb) (modify-category-entry #x5000 ?a tb t) (with-temp-buffer (set-category-table tb) (list (category-set-mnemonics (char-category-set #x4e00)) (category-set-mnemonics (char-category-set #x5000)) (category-set-mnemonics (char-category-set #x9fff)) (category-set-mnemonics (char-category-set #xa000)))))",
        "(let ((tb (make-category-table))) (list (condition-case e (modify-category-entry ?x ?q tb) (error e)) (condition-case e (define-category 7 \"x\" tb) (error e)) (condition-case e (category-docstring ?a 'nope) (error e))))",
        "(list (category-set-mnemonics (make-category-set \"cab\")) (condition-case e (make-category-set \"\\x7f\") (error e)) (condition-case e (category-set-mnemonics (make-bool-vector 10 t)) (error e)))",
        "(let* ((tb (make-category-table)) (cp (progn (define-category ?a \"alpha\" tb) (modify-category-entry ?x ?a tb) (copy-category-table tb)))) (modify-category-entry ?y ?a cp) (list (with-temp-buffer (set-category-table tb) (category-set-mnemonics (char-category-set ?y))) (with-temp-buffer (set-category-table cp) (list (category-set-mnemonics (char-category-set ?x)) (category-set-mnemonics (char-category-set ?y)))) (eq (char-table-extra-slot tb 0) (char-table-extra-slot cp 0))))",
        "(category-table-p (make-char-table (make-symbol \"category-table\")))",
        "(let ((tb (make-category-table))) (with-temp-buffer (set-category-table tb) (list (eq (category-table) tb) (with-temp-buffer (eq (category-table) (standard-category-table))))))",
    ]);
}

// Self-defined charsets of every method, so GNU -Q and a bare VM agree on
// the inputs: MAP from a vector (duplicate characters and codes), a unified
// OFFSET charset, a SUBSET and a SUPERSET.
const CHARSETS: &str = "(progn (define-charset-internal 'rt-map 1 [32 127 0 0 0 0 0 0] nil nil nil nil nil nil nil nil nil [33 #x100 40 #x200 41 #x200 50 #x100 60 #x201] nil nil nil nil) (define-charset-internal 'rt-uni 2 [33 126 33 126 0 0 0 0] nil nil nil nil nil nil nil nil #x200000 nil nil nil [#x2121 #x3000 #x2122 #x3001 #x2221 #x3001 #x2123 #x3002] nil) (unify-charset 'rt-uni) (define-charset-internal 'rt-sub 1 [33 126 0 0 0 0 0 0] nil nil nil nil nil nil nil nil nil nil '(rt-map 40 60 -7) nil nil nil) (define-charset-internal 'rt-sup 1 [0 255 0 0 0 0 0 0] nil nil nil nil nil nil nil nil nil nil nil '((rt-map . 0) (ascii . 128)) nil nil))";

#[test]
fn charset_code_conversion_matches_gnu() {
    let cases = [
        "(list (decode-char 'rt-map 33) (decode-char 'rt-map 41) (decode-char 'rt-map 34) (decode-char 'rt-map 200) (encode-char #x200 'rt-map) (encode-char #x100 'rt-map) (encode-char ?A 'rt-map))",
        "(list (decode-char 'rt-uni #x2121) (decode-char 'rt-uni #x2123) (decode-char 'rt-uni #x2124) (decode-char 'rt-uni #x2020) (encode-char #x3001 'rt-uni) (encode-char #x3002 'rt-uni) (encode-char #x3003 'rt-uni) (encode-char (decode-char 'rt-uni #x2130) 'rt-uni))",
        "(list (decode-char 'rt-sub 33) (decode-char 'rt-sub 34) (encode-char #x200 'rt-sub) (encode-char #x201 'rt-sub) (encode-char #x100 'rt-sub))",
        "(list (decode-char 'rt-sup 41) (decode-char 'rt-sup 193) (encode-char #x200 'rt-sup) (encode-char ?A 'rt-sup) (decode-char 'rt-sup 5))",
        "(list (decode-char 'ascii 65) (decode-char 'ascii 200) (decode-char 'unicode '(1 . 2)) (encode-char #x10ffff 'unicode) (encode-char #x110000 'unicode))",
        "(list (condition-case e (decode-char 'nope 1) (error e)) (condition-case e (decode-char 'ascii -1) (error e)) (condition-case e (encode-char -1 'ascii) (error e)) (condition-case e (encode-char ?a \"ascii\") (error e)))",
    ];
    let cases: Vec<String> = cases.iter().map(|c| format!("(progn {CHARSETS} {c})")).collect();
    compare(&cases.iter().map(String::as_str).collect::<Vec<_>>());
}

// One reused cons per encoder or deunifier walk; a fresh one per OFFSET range.
#[test]
fn map_charset_chars_matches_gnu() {
    let walk = "(lambda (cs &optional from to) (let (calls) (map-charset-chars (lambda (r a) (push (list (car r) (cdr r) a) calls)) cs 'arg from to) (nreverse calls)))";
    let cases = [
        "(list (funcall walk 'rt-map) (funcall walk 'rt-map 35 45) (funcall walk 'rt-uni) (funcall walk 'rt-uni #x2122 #x2221))",
        "(list (funcall walk 'rt-sub) (funcall walk 'rt-sup) (funcall walk 'ascii) (funcall walk 'unicode 65 70))",
        "(let (seen) (map-charset-chars (lambda (r _) (push r seen)) 'rt-map) (list (length seen) (eq (car seen) (car (cdr seen))) (car seen)))",
        "(list (condition-case e (map-charset-chars #'ignore 'ascii nil -1) (error e)) (condition-case e (map-charset-chars #'ignore 'nope) (error e)))",
    ];
    let cases: Vec<String> = cases.iter().map(|c| format!("(progn {CHARSETS} (let ((walk {walk})) {c}))")).collect();
    compare(&cases.iter().map(String::as_str).collect::<Vec<_>>());
}

// A uniprop table built by hand: a run-length block (decoded through extra
// slot 4) and a simple block, read, written and mapped.
const UNIPROP: &str = "(progn (put 'char-code-property-table 'char-table-extra-slots 5) (let ((tb (make-char-table 'char-code-property-table))) (set-char-table-range tb '(128 . 255) (string 2 5 (+ 128 3) 7 9 (+ 128 120))) (set-char-table-range tb '(256 . 383) (string 1 4 6 0 8)) (set-char-table-extra-slot tb 1 0) (set-char-table-extra-slot tb 2 1) (set-char-table-extra-slot tb 4 [z0 z1 z2 z3 z4 z5 z6 z7 z8 z9]) tb))";

#[test]
fn unicode_property_tables_match_gnu() {
    let cases = [
        "(list (aref tb 128) (aref tb 131) (aref tb 135) (aref tb 255) (aref tb 259) (aref tb 260) (aref tb 261) (aref tb 262))",
        "(list (get-unicode-property-internal tb 130) (get-unicode-property-internal tb 131) (get-unicode-property-internal tb 260))",
        "(progn (put-unicode-property-internal tb 129 'z2) (list (aref tb 128) (aref tb 129) (get-unicode-property-internal tb 129) (aref tb 130)))",
        "(let (l) (map-char-table (lambda (k v) (push (cons k v) l)) tb) l)",
        "(list (condition-case e (put-unicode-property-internal tb 129 'zz) (error e)) (condition-case e (get-unicode-property-internal (make-char-table 'x) ?a) (error e)) (condition-case e (get-unicode-property-internal 'x ?a) (error e)) (condition-case e (get-unicode-property-internal tb -1) (error e)))",
        "(progn (set-char-table-extra-slot tb 2 2) (list (put-unicode-property-internal tb 140 4242) (get-unicode-property-internal tb 140) (char-table-extra-slot tb 4) (condition-case e (put-unicode-property-internal tb 141 1.5) (error e))))",
        "(progn (set-char-table-extra-slot tb 2 0) (list (put-unicode-property-internal tb 142 ?x) (aref tb 142) (condition-case e (put-unicode-property-internal tb 143 'q) (error e))))",
        "(let ((char-code-property-alist (list (cons 'p tb) (cons 'q \"rt-no-such-uni.el\")))) (list (eq (unicode-property-table-internal 'p) tb) (unicode-property-table-internal 'q) (unicode-property-table-internal 'r)))",
    ];
    let cases: Vec<String> = cases.iter().map(|c| format!("(let ((tb {UNIPROP})) {c})")).collect();
    compare(&cases.iter().map(String::as_str).collect::<Vec<_>>());
}
