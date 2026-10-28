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
        "(with-temp-buffer (set-default 'h4 (list (lambda (x) 7) t 'cdr)) (set (make-local-variable 'h4) (list (lambda (x) nil) 'car t (lambda (x) 9))) (let (log) (list (run-hook-wrapped 'h4 (lambda (f &rest a) (setq log (cons (cons (functionp f) a) log)) nil) 1 2) (nreverse log) (run-hook-wrapped 'h4 (lambda (f x) (and (not (symbolp f)) (funcall f x))) 3) (run-hook-wrapped 'h3 (lambda (f &rest a) (apply f a)) 5) (run-hook-wrapped 'h2 #'ignore) (condition-case e (run-hook-wrapped 'h4) (error (car e))))))",
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

#[test]
fn char_table_range_and_optimize_match_gnu() {
    compare(&[
        "(list (condition-case e (char-table-range (make-char-table 'x) 5000000) (error e)) (condition-case e (char-table-range (make-char-table 'x) 'a) (error e)) (condition-case e (char-table-range (make-char-table 'x) '(a . 1)) (error e)) (condition-case e (char-table-range (make-char-table 'x) '(1 . a)) (error e)))",
        "(let ((p (make-char-table 'x)) (c (make-char-table 'x))) (set-char-table-parent c p) (aset p ?a 'pa) (list (char-table-range c ?a) (char-table-range c '(?a . ?b)) (progn (set-char-table-range c nil 'cd) (list (char-table-range c ?a) (char-table-range c '(?a . ?b)) (char-table-range c nil)))))",
        "(let ((tb (make-char-table 'x))) (set-char-table-range tb '(#x100 . #x17f) 'a) (aset tb #x105 'a) (list (prin1-to-string tb) (progn (optimize-char-table tb) (prin1-to-string tb)) (aref tb #x105)))",
        "(let ((tb (make-char-table 'x))) (dotimes (i 128) (aset tb i (list 1))) (optimize-char-table tb 'eq) (let ((a (prin1-to-string tb))) (optimize-char-table tb 'equal) (list a (prin1-to-string tb))))",
        "(condition-case e (optimize-char-table 'x) (error e))",
    ]);
}

// GNU's C initial values (character.c, xdisp.c, composite.c, coding.c);
// GNU -Q shows them only after characters.el has changed them.
#[test]
fn c_char_tables_start_as_gnus_c_values() {
    let mut vm = Vm::new();
    let source = "(list (char-table-subtype auto-fill-chars) (aref auto-fill-chars 32) (aref auto-fill-chars 10) (aref auto-fill-chars ?a) (aref char-width-table ?a) (aref char-width-table #x80) (aref char-width-table #x9f) (aref char-width-table #x3fff80) (aref printable-chars 31) (aref printable-chars ?a) (aref printable-chars 127) (aref printable-chars 160) (aref printable-chars #x3fff80) (char-table-subtype char-script-table) (char-table-extra-slot char-script-table 0) (char-table-extra-slot glyphless-char-display 0) (char-table-range composition-function-table ?a) (length translation-table-vector) (get 'translation-table 'char-table-extra-slots) unicode-category-table script-representative-chars current-iso639-language (clear-composition-cache))";
    assert_eq!(vm.eval_source(source).unwrap().display().unwrap(),
        "(auto-fill-chars t t nil 1 4 4 4 nil t nil t nil char-script-table nil empty-box nil 16 2 nil nil nil nil)");
}

#[test]
fn unicode_scalar_character_names_match_gnu() {
    compare(&[
        "(list ?\\N{U+41} \"a\\N{U+1F600}b\" (read \"?\\\\N{U+0041}\") (read \"?\\\\N{U+10FFFF}\"))",
        "(list (condition-case e (read \"?\\\\N{U+D800}\") (error (car e))) (condition-case e (read \"?\\\\N{U+110000}\") (error (car e))) (condition-case e (read \"?\\\\N{}\") (error (car e))) (condition-case e (read \"?\\\\N{U++41}\") (error (car e))) (condition-case e (read \"?\\\\NU+41\") (error (car e))))",
    ]);
}

// utf-8-emacs source holds characters above U+10FFFF (here #x1A01CA, as in
// language/ethiopic.el) and decodes an invalid byte to a raw-byte character.
#[test]
fn utf_8_emacs_sources_load_gnu_characters() {
    let dir = std::env::temp_dir().join(format!("romacs-utf8-emacs-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let file = dir.join("rt-utf8-emacs.el");
    let mut bytes = b";; -*- coding: utf-8-emacs; lexical-binding: t -*-\n(setq rt-big ?".to_vec();
    bytes.extend_from_slice(&[0xf6, 0xa0, 0x87, 0x8a]);
    bytes.extend_from_slice(b" rt-text \"a");
    // A lone continuation byte, GNU's internal 2-byte raw-byte form and a
    // surrogate encoding all decode to raw bytes, as coding.c does.
    bytes.extend_from_slice(&[0xf6, 0xa0, 0x87, 0x8a, 0x80, 0xc0, 0x80, 0xed, 0xa0, 0x80]);
    bytes.extend_from_slice(b"\")\n");
    std::fs::write(&file, bytes).unwrap();
    let dos = dir.join("rt-utf8-emacs-dos.el");
    let mut bytes = b";; -*- coding: utf-8-emacs-dos; lexical-binding: t -*-\r\n(setq rt-dos \"a\r\nb".to_vec();
    bytes.extend_from_slice(&[0xf6, 0xa0, 0x87, 0x8a]);
    bytes.extend_from_slice(b"\")\r\n");
    std::fs::write(&dos, bytes).unwrap();
    let expression = format!("(progn (load {:?} nil t t) (load {:?} nil t t) (list rt-big (append rt-text nil) (multibyte-string-p rt-text) (append rt-dos nil)))", file.to_str().unwrap(), dos.to_str().unwrap());
    compare(&[&expression]);
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn frame_and_buffer_state_queries_match_gnu() {
    compare(&[
        "(progn (defvar rt-s nil) (list (frame-or-buffer-changed-p 'rt-s) (frame-or-buffer-changed-p 'rt-s) (progn (get-buffer-create \"rt-x\") (frame-or-buffer-changed-p 'rt-s)) (frame-or-buffer-changed-p 'rt-s) (progn (get-buffer-create \" rt-hidden\") (frame-or-buffer-changed-p 'rt-s)) (progn (with-current-buffer \" rt-hidden\" (insert \"a\")) (frame-or-buffer-changed-p 'rt-s)) (progn (with-current-buffer \"rt-x\" (insert \"a\")) (frame-or-buffer-changed-p 'rt-s)) (progn (with-current-buffer \"rt-x\" (setq buffer-read-only t)) (frame-or-buffer-changed-p 'rt-s)) (frame-or-buffer-changed-p 'rt-s) (frame-or-buffer-changed-p) (frame-or-buffer-changed-p) (vectorp rt-s) (eq (aref rt-s (1- (length rt-s))) 'lambda)))",
        "(progn (defvar rt-v [1 2]) (list (condition-case e (frame-or-buffer-changed-p 3) (error e)) (frame-or-buffer-changed-p 'rt-v) (condition-case e (frame-or-buffer-changed-p 'rt-unbound-var) (error e))))",
        "(list (frame-initial-p) (frame-initial-p (selected-frame)) (frame-initial-p (frame-terminal)) (frame-initial-p 3))",
        "(list left-margin (local-variable-if-set-p 'left-margin) (with-temp-buffer (setq left-margin 4) (list left-margin (local-variable-p 'left-margin))) left-margin (condition-case e (setq left-margin 'x) (error e)))",
    ]);
}

#[test]
fn syntax_motion_matches_gnu() {
    compare(&[
        "(with-temp-buffer (insert \"  foo-bar (baz) ;; x\") (goto-char 1) (list (skip-syntax-forward \" \") (point) (skip-syntax-forward \"w_\") (point) (skip-syntax-forward \"^(\") (point) (skip-syntax-forward \"(\") (skip-syntax-backward \"^ \") (point) (skip-syntax-backward \" \" 3) (point) (skip-syntax-forward \"w\" 100) (skip-syntax-forward \"\") (point)))",
        "(list (condition-case e (skip-syntax-forward 3) (error e)) (condition-case e (with-temp-buffer (skip-syntax-forward \"w\" 'x)) (error e)) (with-temp-buffer (insert \"abc def\") (narrow-to-region 2 4) (goto-char 2) (list (skip-syntax-forward \"w\") (point) (skip-syntax-backward \"w\" 1) (point))))",
        "(with-temp-buffer (insert \"ab cd\") (put-text-property 3 4 'syntax-table (string-to-syntax \"w\")) (goto-char 1) (list (skip-syntax-forward \"w\") (point) (let ((parse-sexp-lookup-properties t)) (goto-char 1) (list (skip-syntax-forward \"w\") (point)))))",
        "(with-temp-buffer (let ((st (copy-syntax-table))) (modify-syntax-entry ?' \"'\" st) (modify-syntax-entry ?\\\\ \"\\\\\" st) (set-syntax-table st) (insert \"a ''b \\\\'c x\\\\\\\\'d\") (list (progn (goto-char 5) (backward-prefix-chars) (point)) (progn (goto-char 9) (backward-prefix-chars) (point)) (progn (goto-char 14) (backward-prefix-chars) (point)) (progn (goto-char 2) (backward-prefix-chars) (point)))))",
        "(with-temp-buffer (let ((st (copy-syntax-table))) (modify-syntax-entry ?' \"'\" st) (modify-syntax-entry ?# \"_ p\" st) (modify-syntax-entry ?/ \"/\" st) (set-syntax-table st) (insert \"z #'#q /#r\") (list (progn (goto-char 6) (backward-prefix-chars) (point)) (progn (goto-char 10) (backward-prefix-chars) (point)))))",
    ]);
}

#[test]
fn text_property_not_all_matches_gnu() {
    compare(&[
        "(with-temp-buffer (insert \"abcdef\") (put-text-property 3 5 'face 'bold) (list (text-property-not-all 1 7 'face nil) (text-property-not-all 3 7 'face 'bold) (text-property-not-all 3 5 'face 'bold) (text-property-not-all 7 1 'face nil) (text-property-not-all 2 2 'face 'x) (text-property-not-all 1 3 'q 'x) (condition-case e (text-property-not-all 0 3 'face nil) (error e)) (condition-case e (text-property-not-all 1 9 'face nil) (error e))))",
        "(let ((s (propertize \"abc\" 'p 1))) (list (text-property-not-all 0 3 'p 1 s) (text-property-not-all 0 3 'p 2 s) (condition-case e (text-property-not-all 0 4 'p 1 s) (error e)) (text-property-not-all 0 2 'x nil \"ab\") (text-property-not-all 0 2 'x 3 \"ab\")))",
        "(with-temp-buffer (insert \"abc\") (put 'rt-cat 'face 'italic) (put-text-property 1 2 'category 'rt-cat) (list (text-property-not-all 1 4 'face 'italic) (let ((default-text-properties '(face italic))) (text-property-not-all 1 4 'face 'italic)) (let ((m (copy-marker 3))) (text-property-not-all 1 m 'face nil))))",
        "(list (text-property-not-all 9 9 'face nil \"abc\") (condition-case e (text-property-not-all 0 9 'face nil \"abc\") (error e)) (let ((default-text-properties '(p 1))) (list (text-property-not-all 0 3 'p 1 \"abc\") (text-property-not-all 0 3 'p 1 (propertize \"abc\" 'q 2)))) (with-temp-buffer (insert \"abc\") (list (text-property-not-all 9 9 'face nil) (let ((default-text-properties '(p 1))) (text-property-not-all 1 3 'p 1)))) (with-temp-buffer (text-property-not-all 1 1 'p 1)) (condition-case e (text-property-not-all 0 0 'p nil 42) (error e)))",
    ]);
}

#[test]
fn indentation_matches_gnu() {
    compare(&[
        "(with-temp-buffer (list (insert-char ?x 3) (insert-char ?y) (insert-char ?z 0) (insert-char ?w -1) (buffer-string) (condition-case e (insert-char \"a\") (error e)) (condition-case e (insert-char ?a 'b) (error e))))",
        "(with-temp-buffer (insert \"  \\tfoo\") (list (current-indentation) (progn (goto-char (point-max)) (current-column)) (progn (setq tab-width 4) (current-indentation))))",
        "(with-temp-buffer (insert \"ab\") (list (indent-to 10) (buffer-string) (current-column) (let ((indent-tabs-mode nil)) (indent-to 14 2)) (buffer-string) (indent-to 3 1) (buffer-string) (condition-case e (indent-to 'x) (error e))))",
        "(with-temp-buffer (insert \"a\\tb\\nxy\") (goto-char 1) (list (move-to-column 3) (point) (move-to-column 3 t) (buffer-string) (point) (move-to-column 20) (point) (progn (goto-char (point-max)) (move-to-column 5 t)) (buffer-string) (point) (condition-case e (move-to-column -1) (error e))))",
        "(with-temp-buffer (list (preceding-char) (progn (insert \"ab\") (preceding-char)) (progn (goto-char 1) (preceding-char)) indent-tabs-mode))",
    ]);
}

#[test]
fn subst_char_in_region_matches_gnu() {
    compare(&[
        "(with-temp-buffer (insert (propertize \"abcabc\" 'face 'bold)) (let ((m (copy-marker 3))) (goto-char 4) (list (subst-char-in-region 1 7 ?b ?X) (buffer-string) (marker-position m) (point) (buffer-modified-p))))",
        "(with-temp-buffer (insert \"aaa\") (set-buffer-modified-p nil) (list (subst-char-in-region 4 1 ?a ?b t) (buffer-string) (buffer-modified-p)))",
        "(with-temp-buffer (insert \"a\") (list (condition-case e (subst-char-in-region 1 9 ?a ?b) (error e)) (subst-char-in-region 1 2 ?z ?b) (progn (setq buffer-read-only t) (condition-case e (subst-char-in-region 1 2 ?a ?b) (error (car e))))))",
        "(with-temp-buffer (insert \"a\\u00e9a\") (list (subst-char-in-region 1 4 ?\\u00e9 ?e) (buffer-string) (subst-char-in-region 1 4 ?a ?\\u00e9) (buffer-string)))",
    ]);
}

#[test]
fn text_property_removal_matches_gnu() {
    compare(&[
        "(with-temp-buffer (insert \"abcdef\") (put-text-property 1 7 'face 'bold) (put-text-property 2 4 'x 1) (list (remove-text-properties 2 3 '(face nil)) (text-properties-at 1) (remove-text-properties 2 3 '(face nil)) (remove-list-of-text-properties 1 7 '(x q)) (text-properties-at 2) (set-text-properties 1 3 '(a 1 b 2)) (text-properties-at 1) (set-text-properties 1 7 nil) (text-properties-at 4) (buffer-string)))",
        "(let ((s (propertize \"abc\" 'p 1 'q 2))) (list (remove-text-properties 0 2 '(p nil) s) s (set-text-properties 1 2 '(z 3) s) s (remove-list-of-text-properties 0 3 '(q) s) s (set-text-properties 0 3 nil s) s))",
        "(list (condition-case e (with-temp-buffer (remove-text-properties 0 5 '(p nil))) (error e)) (condition-case e (set-text-properties 0 9 nil \"ab\") (error e)) (condition-case e (remove-list-of-text-properties 0 1 'p \"ab\") (error e)))",
    ]);
}

#[test]
fn literal_search_matches_gnu() {
    compare(&[
        "(with-temp-buffer (insert \"a.b a*b a.b\") (goto-char 1) (list (search-forward \".b\") (point) (search-forward \"*\" nil t) (search-forward \"zz\" nil t) (point) (search-backward \"a.b\") (point) (search-forward \"a.b\" nil t 2) (search-backward \"b\" 3 t) (condition-case e (search-forward \"zz\") (error e)) (search-forward \"zz\" nil 1) (point) (search-backward \"a\" nil t -1) (point)))",
        "(with-temp-buffer (insert \"ABC abc\") (goto-char 1) (list (let ((case-fold-search t)) (search-forward \"abc\")) (let ((case-fold-search nil)) (goto-char 1) (search-forward \"abc\")) (match-beginning 0) (match-end 0) (let ((case-fold-search nil)) (search-backward \"AB\" nil t))))",
        "(with-temp-buffer (insert \"ab\\nb\") (list (let ((case-fold-search nil)) (goto-char 1) (list (search-forward \"b\") (match-beginning 0) (search-forward \"\\n\" nil t) (match-beginning 0))) (let ((case-fold-search t)) (goto-char 1) (list (search-forward \"B\" nil t) (match-beginning 0) (match-end 0)))))",
        "(with-temp-buffer (insert \"ABC\") (set (make-local-variable (quote case-fold-search)) nil) (goto-char 1) (list (search-forward \"abc\" nil t) (progn (goto-char 1) (re-search-forward \"abc\" nil t)) (string-match \"abc\" \"ABC\") (let ((case-fold-search t)) (goto-char 1) (search-forward \"abc\" nil t)) (with-temp-buffer (insert \"ABC\") (goto-char 1) (search-forward \"abc\" nil t))))",
        "(with-temp-buffer (insert \"abc\") (goto-char 3) (list (condition-case e (search-forward \"a\" 1) (error e)) (condition-case e (search-backward \"a\" 3) (error e)) (condition-case e (search-forward 'a) (error e))))",
    ]);
}

#[test]
fn min_and_max_accept_markers_like_gnu() {
    compare(&[
        "(with-temp-buffer (insert \"abcd\") (list (min (copy-marker 3) 4) (max (copy-marker 3) 2) (max 1 (copy-marker 2) 1.5) (min 2.0 (copy-marker 2)) (min (copy-marker 4)) (condition-case e (min 'a 1) (error e)) (condition-case e (max 1 \"x\") (error e)) (max 0.0 -0.0) (min -0.0 0.0)))",
    ]);
}

#[test]
fn char_charset_matches_gnu() {
    compare(&[
        "(progn (set-charset-priority 'unicode) (list (char-charset ?a) (char-charset ?\\u00e9) (char-charset #x3fff80) (char-charset ?\\u00e9 '(iso-8859-1)) (char-charset ?\\u0100 '(iso-8859-1)) (char-charset ?a '(eight-bit)) (condition-case e (char-charset 'x) (error e))))",
        "(progn (set-charset-priority 'iso-8859-1 'unicode) (with-temp-buffer (insert \"a\\u00e9\\u0100\") (list (charset-after 1) (charset-after 2) (charset-after 3) (charset-after 4) (charset-after) (charset-after 0) (progn (goto-char 2) (charset-after)) (condition-case e (charset-after 'x) (error e)))))",
    ]);
}

#[test]
fn legacy_buffer_primitives_accept_markers_like_gnu() {
    compare(&[
        "(with-temp-buffer (insert \"abcdef\") (list (markerp (goto-char (copy-marker 2))) (point) (buffer-substring (copy-marker 1) (copy-marker 3)) (buffer-substring-no-properties (copy-marker 3) 5) (char-after (copy-marker 2)) (progn (narrow-to-region (copy-marker 2) (copy-marker 4)) (prog1 (list (buffer-string) (point-min) (point-max)) (widen))) (progn (delete-region (copy-marker 1) (copy-marker 2)) (buffer-string)) (condition-case e (goto-char (make-marker)) (error e))))",
        "(with-temp-buffer (insert (propertize \"abc\" 'face 'bold) \"def\") (narrow-to-region 2 5) (buffer-string))",
    ]);
}

#[test]
fn cus_start_built_ins_start_as_gnus_c_values() {
    compare(&[
        "(list gc-cons-threshold report-emacs-bug-address max-mini-window-height alter-fullscreen-frames auto-save-timeout polling-period (default-value 'parse-sexp-ignore-comments) maximum-scroll-margin hourglass-delay (stringp shell-file-name))",
        "(list (default-value 'abbrev-mode) (default-value 'ctl-arrow) (default-value 'bidi-paragraph-direction) (default-value 'word-wrap) (default-value 'selective-display-ellipses) (default-value 'indicate-empty-lines) (default-value 'indicate-buffer-boundaries) (default-value 'scroll-up-aggressively) (default-value 'scroll-down-aggressively) (default-value 'cursor-type) (default-value 'line-spacing) (default-value 'cursor-in-non-selected-windows) (local-variable-if-set-p 'cursor-type) (with-temp-buffer (setq word-wrap t) (list word-wrap (local-variable-p 'word-wrap))) word-wrap)",
        "(list header-line-format (default-value 'header-line-format) (local-variable-if-set-p 'header-line-format) (local-variable-p 'header-line-format) (with-temp-buffer (setq header-line-format \"x\") (list header-line-format (local-variable-p 'header-line-format))) header-line-format (special-variable-p 'header-line-format))",
    ]);
}

#[test]
fn file_name_absolute_p_matches_gnu() {
    compare(&[
        "(list (file-name-absolute-p \"/a\") (file-name-absolute-p \"~\") (file-name-absolute-p \"~/x\") (file-name-absolute-p \"~root/x\") (file-name-absolute-p \"~root\") (file-name-absolute-p \"~rt-no-such-user/x\") (file-name-absolute-p \"a/b\") (file-name-absolute-p \"\") (condition-case e (file-name-absolute-p 'a) (error e)))",
    ]);
}

#[test]
fn command_remapping_and_keymap_lists_match_gnu() {
    compare(&[
        "(let ((m (make-sparse-keymap)) (n (make-sparse-keymap))) (define-key m [remap kill-line] 'my-kill) (define-key n (kbd \"C-c\") 'foo) (list (command-remapping 'kill-line nil m) (command-remapping 'kill-line nil (list n m)) (command-remapping 'other nil m) (command-remapping \"x\") (command-remapping nil) (command-remapping 'kill-line) (progn (use-local-map m) (command-remapping 'kill-line)) (with-temp-buffer (command-remapping 'kill-line))))",
        "(let ((n (make-sparse-keymap))) (define-key n (kbd \"C-c\") 'foo) (list (lookup-key (list n) (kbd \"C-c\")) (lookup-key nil \"a\") (lookup-key nil \"ab\") (lookup-key nil \"\") (lookup-key (list n) \"\") (condition-case e (lookup-key 3 \"a\") (error e)) (lookup-key (list n) (kbd \"C-c a\")) (lookup-key n [nil]) (condition-case e (use-local-map 3) (error e)) (with-temp-buffer (use-local-map n) (list (key-binding (kbd \"C-c\") nil t) (key-binding (kbd \"C-c\") nil nil nil)))))",
    ]);
}

#[test]
fn base64_string_coding_matches_gnu() {
    compare(&[
        "(list (base64-encode-string \"\") (base64-encode-string \"a\") (base64-encode-string \"ab\") (base64-encode-string \"abc\") (base64-encode-string (make-string 60 ?x)) (base64-encode-string (make-string 60 ?x) t) (condition-case e (base64-encode-string (string 255 0 128)) (error e)) (base64-encode-string (unibyte-string 255 0 128)) (base64-encode-string (string-to-multibyte (unibyte-string 255 0 128))) (condition-case e (base64-encode-string \"\\u00e9\") (error e)) (base64url-encode-string \"\\xff\\xfe\") (base64url-encode-string \"\\xff\\xfe\" t))",
        "(list (base64-decode-string \"YWJj\") (base64-decode-string \" YW\\nJj \") (base64-decode-string \"YQ==\") (base64-decode-string \"YWI=\") (condition-case e (base64-decode-string \"YQ\") (error e)) (base64-decode-string \"YQ\" t) (condition-case e (base64-decode-string \"Y*Q==\") (error e)) (base64-decode-string \"Y*Q==\" nil t) (base64-decode-string \"__-_\" t) (condition-case e (base64-decode-string \"YQ=x\") (error e)) (base64-decode-string \"YQ= =\") (condition-case e (base64-decode-string 3) (error e)) (multibyte-string-p (base64-decode-string \"w6k=\")) (base64-decode-string \"YWJjZA==YWI=\"))",
    ]);
}
