//! Primitives the paired GNU/Romacs daily-journey bench found void, each
//! against pinned GNU, alone and through the command that reached it:
//! the posix- searches (ange-ftp's file-name test on every visit),
//! `ngettext' (query-replace's count, occur's header), `system-users' and
//! `system-groups' (file-name completion), `internal-event-symbol-parse-
//! modifiers' (`event-modifiers', C-h f), `accessible-keymaps' and
//! `keymap--get-keyelt' (C-h m, where-is), `print-unreadable-function'
//! (`readablep', C-h v) and `buffer-size' of a given buffer (help-fns).
use romacs_vm::Vm;
use std::path::Path;

fn romacs() -> Vm {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let mut vm = Vm::new();
    vm.eval_source("(setq noninteractive t)").unwrap();
    vm.set_frame_geometry(80, 25, 1, 1).unwrap();
    let lisp = root.join("gnu-lisp");
    let mut names: Vec<String> = std::fs::read_dir(&lisp).unwrap()
        .filter_map(|entry| entry.ok().filter(|e| e.path().is_dir()).and_then(|e| e.file_name().into_string().ok()))
        .filter(|name| !["term", "RCS", "CVS", "Old"].contains(&name.as_str()) && !name.starts_with('='))
        .collect();
    names.sort();
    let mut paths = vec![lisp.clone()];
    paths.extend(names.iter().rev().filter(|n| *n != "obsolete").map(|n| lisp.join(n)));
    if names.iter().any(|n| n == "obsolete") { paths.push(lisp.join("obsolete")); }
    paths.push(root.join("rlisp"));
    vm.set_load_path(&paths).unwrap();
    vm.install_lisp_prelude().unwrap();
    vm.load_file(&root.join("rlisp/romacs-loadup-bootstrap.el")).unwrap();
    vm
}

/// Each EXPRESSION's printed value (or signal) in pinned GNU and in a
/// Romacs on the loadup bootstrap, after SETUP in both; the mismatches.
fn mismatches(setup: &str, expressions: &[&str]) -> Vec<String> {
    let binary = std::env::var("ROMACS_GNU_ORACLE").expect("pinned GNU oracle");
    let probe = |expression: &str| format!("(prin1-to-string (condition-case e {expression} (error (list 'signal e))))");
    let gnu = format!("(progn {setup} (dolist (s (list {})) (princ s) (princ \"\\0\")))",
        expressions.iter().map(|e| probe(e)).collect::<Vec<_>>().join(" "));
    let home = std::env::temp_dir().join(format!("journey-primitives-{}", std::process::id()));
    std::fs::create_dir_all(&home).unwrap();
    let output = std::process::Command::new(&binary)
        .args(["-Q", "--batch", "--eval", &gnu])
        .env("HOME", &home)
        .output()
        .unwrap();
    assert!(output.status.success(), "GNU: {}", String::from_utf8_lossy(&output.stderr));
    let expected: Vec<String> = String::from_utf8(output.stdout).unwrap().split('\0').map(str::to_owned).collect();
    let mut vm = romacs();
    vm.eval_source(&format!("(progn {setup} nil)")).unwrap();
    let mut failures = Vec::new();
    for (expression, expected) in expressions.iter().zip(expected) {
        let actual = vm.eval_source(&probe(expression)).and_then(|v| v.display()).map(|printed| {
            serde_json::from_str::<String>(&printed).unwrap_or(printed)
        }).unwrap_or_else(|e| format!("ERR {e}"));
        if actual != expected {
            failures.push(format!("{expression}\n  GNU    {expected}\n  Romacs {actual}"));
        }
    }
    failures
}

fn assert_matches(setup: &str, expressions: &[&str]) {
    let failures = mismatches(setup, expressions);
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

#[test]
fn posix_searches_take_the_longest_match() {
    assert_matches("", &[
        r#"(list (posix-string-match "a\\|ab" "abc") (match-data) (string-match "a\\|ab" "abc") (match-data))"#,
        r#"(list (posix-string-match "\\(a*\\)\\(ab\\)*b" "aabb") (match-data))"#,
        r#"(list (posix-string-match "x\\|xy" "axyz" 1) (match-data))"#,
        r#"(progn (string-match "z" "z") (list (posix-string-match "a\\|ab" "ab" nil t) (match-data)))"#,
        r#"(let ((case-fold-search t)) (list (posix-string-match "A\\|AB" "abc") (match-data)))"#,
        r#"(posix-string-match 1 "a")"#,
        r#"(with-temp-buffer (insert "abcd") (goto-char 1)
             (list (posix-looking-at "a\\|abc") (match-data t) (looking-at "a\\|abc") (match-data t)
                   (posix-looking-at "a\\|abc" t) (point)))"#,
        r#"(with-temp-buffer (insert "xx abcd ab") (goto-char 1)
             (list (posix-search-forward "ab\\|abcd") (point) (match-data t)
                   (posix-search-forward "zz" nil t) (posix-search-forward "ab\\|abcd" nil t 1) (point)))"#,
        r#"(with-temp-buffer (insert "abcd ab") (goto-char (point-max))
             (list (posix-search-backward "a\\|ab") (point) (match-data t)
                   (posix-search-backward "c\\|cd" nil t) (point) (match-data t)))"#,
        r#"(with-temp-buffer (insert "abc") (goto-char 1) (posix-search-forward "q"))"#,
    ]);
}

/// ange-ftp, once loaded, tests every visited name with `posix-string-match'
/// from `find-file-hook'; a local file still visits as GNU's does.
#[test]
fn a_file_visits_with_ange_ftp_loaded() {
    let file = std::env::temp_dir().join(format!("journey-primitives-{}.txt", std::process::id()));
    std::fs::write(&file, "hi\n").unwrap();
    let file = file.display();
    assert_matches("(require 'ange-ftp)", &[
        "(and (memq 'ange-ftp-set-buffer-mode find-file-hook) t)",
        &format!(r#"(let ((b (find-file-noselect "{file}"))) (prog1 (with-current-buffer b (list major-mode (buffer-string))) (kill-buffer b)))"#),
        r#"(list (ange-ftp-ftp-name "/ftp@host:/x") (ange-ftp-ftp-name "/tmp/x.txt"))"#,
    ]);
}

#[test]
fn ngettext_picks_the_singular_for_one() {
    assert_matches("", &[
        r#"(list (ngettext "file" "files" 1) (ngettext "file" "files" 0) (ngettext "file" "files" 2) (ngettext "file" "files" (expt 2 70)))"#,
        r#"(ngettext "a" "b" 1.0)"#,
        r#"(ngettext 'a "b" 1)"#,
        r#"(ngettext "a" 'b 1)"#,
        r#"(let ((s "x")) (eq (ngettext s "y" 1) s))"#,
    ]);
}

/// perform-replace's "Replaced N occurrence(s)" and occur's header count
/// with `ngettext'.
#[test]
fn replace_and_occur_count_their_matches() {
    let messages = |body: &str| format!(
        r#"(progn (with-current-buffer (messages-buffer) (let ((inhibit-read-only t)) (erase-buffer)))
                  (let ((inhibit-message t)) {body})
                  (with-current-buffer (messages-buffer) (buffer-string)))"#);
    assert_matches("(require 'replace)", &[
        &messages(r#"(with-temp-buffer (insert "foo foo foo") (goto-char 1) (perform-replace "foo" "bar" nil nil nil))"#),
        &messages(r#"(with-temp-buffer (insert "foo") (goto-char 1) (perform-replace "foo" "bar" nil nil nil))"#),
        r#"(with-temp-buffer (rename-buffer "occur-src") (insert "alpha\nbeta alpha\ngamma\n")
             (let ((inhibit-message t)) (occur "alpha"))
             (with-current-buffer "*Occur*" (buffer-substring-no-properties (point-min) (line-end-position))))"#,
    ]);
}

#[test]
fn system_users_and_groups_list_the_account_database() {
    assert_matches("", &[
        "(system-users)",
        "(system-groups)",
    ]);
}

#[test]
fn event_symbols_parse_their_modifiers() {
    let symbols = ["C-M-x", "M-S-down-mouse-1", "mouse-1", "C-mouse-1", "double-mouse-3", "wheel-up", "S-wheel-down",
        "drag-mouse-2", "triple-down-mouse-1", "s-H-A-f1", "up-mouse-1", "x", "foo-bar", "C-x-y", "down",
        "C-double-mouse-1", "double-wheel-up", "nil"];
    let parsed: Vec<String> = symbols.iter().map(|s| format!(
        "(list (internal-event-symbol-parse-modifiers '{s}) (get '{s} 'event-symbol-element-mask) (get '{s} 'event-symbol-elements))"))
        .collect();
    let mut expressions: Vec<&str> = parsed.iter().map(String::as_str).collect();
    expressions.extend([
        "(internal-event-symbol-parse-modifiers 5)",
        r#"(internal-event-symbol-parse-modifiers "C-x")"#,
        "(list (event-modifiers 'C-M-x) (event-modifiers 'mouse-1) (event-basic-type 'C-M-x) (event-modifiers '(down-mouse-1 nil)))",
        "(progn (put 'jp-cached 'event-symbol-element-mask '(zz 1)) (put 'jp-cached 'event-symbol-elements '(zz up)) (internal-event-symbol-parse-modifiers 'jp-cached))",
    ]);
    assert_matches("", &expressions);
}

#[test]
fn accessible_keymaps_walks_prefixes_breadth_first() {
    let setup = r#"(setq jp-m (make-sparse-keymap) jp-sub (make-sparse-keymap) jp-subsub (make-sparse-keymap))
                   (define-key jp-m "a" #'ignore) (define-key jp-m "\C-c" jp-sub) (define-key jp-sub "x" jp-subsub)
                   (define-key jp-subsub "y" #'ignore) (define-key jp-m [f5] jp-sub)
                   (define-key jp-m [27] jp-subsub) (define-key jp-m "b" (cons "Menu" jp-sub))
                   (define-key jp-m "c" (list 'menu-item "C" jp-subsub :filter #'identity))"#;
    assert_matches(setup, &[
        "(mapcar #'car (accessible-keymaps jp-m))",
        r#"(mapcar #'car (accessible-keymaps jp-m "\C-c"))"#,
        "(mapcar #'car (accessible-keymaps jp-m [f5 ?x]))",
        r#"(accessible-keymaps jp-m "q")"#,
        "(eq (cdr (car (accessible-keymaps jp-m))) jp-m)",
        "(let ((m (make-sparse-keymap))) (define-key m \"a\" m) (mapcar #'car (accessible-keymaps m)))",
        "(let ((m (make-sparse-keymap)) (sub (make-sparse-keymap))) (define-key m [134217825] sub) (define-key sub \"b\" #'ignore) (mapcar #'car (accessible-keymaps m (string 225))))",
        "(let ((m (make-sparse-keymap)) (p (make-sparse-keymap))) (define-key p \"z\" jp-sub) (set-keymap-parent m p) (mapcar #'car (accessible-keymaps m)))",
        "(accessible-keymaps 'jp-not-a-map)",
        "(mapcar (lambda (x) (key-description (car x))) (accessible-keymaps ctl-x-map))",
        "(list (keymap--get-keyelt '(\"S\" . ignore) nil) (keymap--get-keyelt '(menu-item \"I\" ignore :filter (lambda (_) 'undo)) nil) (keymap--get-keyelt '(menu-item \"I\" ignore :filter (lambda (_) 'undo)) t) (keymap--get-keyelt '(menu-item) nil))",
        "(list (where-is-internal 'jp-cmd (list jp-m)) (where-is-internal 'ignore (list jp-m)))",
    ]);
}

#[test]
fn print_unreadable_function_answers_unreadable_objects() {
    assert_matches("", &[
        r#"(list (readablep 1) (readablep "x") (readablep '(a "b" [c])) (readablep (current-buffer)) (readablep (list 1 (make-marker))) (readablep (symbol-function 'car)))"#,
        r#"(let ((print-unreadable-function (lambda (o e) (format "<%s %s>" (type-of o) e)))) (list (prin1-to-string (list 1 (current-buffer) (make-marker))) (prin1-to-string (current-buffer) t)))"#,
        "(let ((print-unreadable-function (lambda (_o _e) nil))) (prin1-to-string (list (make-marker))))",
        "(let ((print-unreadable-function (lambda (_o _e) 'skip))) (prin1-to-string (list 1 (make-marker) 2)))",
        "(let ((print-unreadable-function 'jp-not-a-function)) (prin1-to-string (make-marker)))",
        r#"(let* ((seen nil) (print-unreadable-function (lambda (_o e) (push (list e print-unreadable-function) seen) "U"))) (list (prin1-to-string (make-marker)) seen))"#,
        r#"(let ((print-unreadable-function (lambda (_o e) (format "[%s]" e)))) (with-output-to-string (princ (list (make-marker) "s")) (prin1 (make-marker)) (print (make-marker))))"#,
        r#"(let ((print-unreadable-function (lambda (_o _e) "B"))) (with-temp-buffer (prin1 (list (make-marker)) (current-buffer)) (buffer-string)))"#,
        "(catch 'tag (let ((print-unreadable-function (lambda (_o _e) (throw 'tag 'thrown)))) (prin1-to-string (list 1 (make-marker)))))",
        r#"(let ((print-unreadable-function (lambda (_o _e) (error "boom")))) (prin1-to-string (make-marker)))"#,
        r#"(let ((print-circle t) (x (list 1)) (print-unreadable-function (lambda (_o _e) "M"))) (prin1-to-string (list x x (make-marker))))"#,
        r#"(let ((print-length 2) (print-unreadable-function (lambda (_o _e) "M"))) (prin1-to-string (list (make-marker) (make-marker) (make-marker))))"#,
        "(let* ((n 0) (print-unreadable-function (lambda (_o _e) (setq n (1+ n)) nil)) (m (make-marker))) (list (prin1-to-string (list m m)) n))",
        r#"(let ((print-unreadable-function (lambda (_o _e) "S"))) (prin1-to-string (list (make-marker) "a\nb") nil '((escape-newlines . t))))"#,
        r#"(prin1-to-string (list (make-marker)) nil (list (cons 'unreadable-function (lambda (_o _e) "O"))))"#,
        r#"(let ((print-unreadable-function (lambda (_o _e) "S"))) (prin1-to-string (list (make-marker)) nil '(t)))"#,
        "(prin1-to-string 42 nil 7)",
        "(prin1-to-string 42 nil '(garbage))",
        "(prin1-to-string 42 nil '((unknown . t)))",
        "(prin1-to-string 42 nil '((length . 1) . 3))",
    ]);
}

/// help-fns: C-h f on a primitive and C-h v on a documented variable print
/// as GNU's (`readablep' of the value, `buffer-size' of the help buffer);
/// C-h m lists the mode's keys (`keymap--get-keyelt', `accessible-keymaps'),
/// compared up to the hook text, whose filling is the docstring's own.
#[test]
fn help_describes_a_function_a_variable_and_a_mode() {
    let help = |body: &str, end: &str| format!(
        "(progn (let ((inhibit-message t)) (save-window-excursion {body}))
                (with-current-buffer (help-buffer)
                  (buffer-substring-no-properties (point-min) (progn (goto-char (point-min)) {end}))))");
    assert_matches(r#"(require 'help-fns) (defvar jp-documented '(1 "two") "A documented variable.")"#, &[
        &help("(describe-function 'buffer-size)", "(point-max)"),
        &help("(describe-variable 'jp-documented)", "(point-max)"),
        &help("(with-temp-buffer (emacs-lisp-mode) (describe-mode))", r#"(search-forward "In addition") (match-beginning 0)"#),
        "(buffer-size (get-buffer-create \" jp-sized\"))",
        "(buffer-size 'jp-not-a-buffer)",
    ]);
}

/// loadup's Snarf-documentation gives the C variables their DOC
/// docstrings: `documentation-property' reads them, help knows they are
/// C source's, and C-h v describes one as GNU does.
#[test]
fn c_variables_carry_their_doc_strings() {
    let help = |body: &str| format!(
        "(progn (let ((inhibit-message t)) (save-window-excursion {body}))
                (with-current-buffer (help-buffer) (buffer-substring-no-properties (point-min) (point-max))))");
    assert_matches("(require 'help-fns)", &[
        "(documentation-property 'fill-column 'variable-documentation t)",
        "(documentation-property 'case-fold-search 'variable-documentation)",
        "(integerp (get 'gc-cons-threshold 'variable-documentation))",
        "(find-lisp-object-file-name 'fill-column 'defvar)",
        &help("(describe-variable 'fill-column)"),
    ]);
}

/// icomplete's file-name completion of a `~USER' prefix lists
/// `system-users', and a `~/' typed in the middle of a path loads neither
/// tramp nor ange-ftp, as in GNU.
#[test]
fn file_name_completion_reaches_system_users_and_loads_no_handler() {
    let sorted = |input: &str| format!(
        r#"(with-temp-buffer (insert "{input}")
             (let ((minibuffer-completion-table #'read-file-name-internal) (names nil)
                   (all (completion-all-sorted-completions)))
               (while (consp all) (setq names (cons (substring-no-properties (car all)) names) all (cdr all)))
               (list (nreverse names) all)))"#);
    assert_matches("(require 'icomplete)", &[
        &sorted("~roo"),
        "(mapcar #'substring-no-properties (completion-file-name-table \"~roo\" nil t))",
        r#"(let ((default-directory "/tmp/"))
             (dolist (p '("/tmp/~" "/tmp/~/" "/tmp/x/~/src/scratch.txt"))
               (completion-all-completions p #'read-file-name-internal nil (length p))
               (completion-try-completion p #'read-file-name-internal nil (length p)))
             (list (featurep 'ange-ftp) (featurep 'tramp)))"#,
    ]);
}

/// eshell's esh-util loads ange-ftp in VM2 (DIVERGENCES.md VM2-EWCLOAD1);
/// a local file still visits as GNU's does.
#[test]
fn a_file_visits_after_eshell_loads() {
    let file = std::env::temp_dir().join(format!("journey-eshell-{}.txt", std::process::id()));
    std::fs::write(&file, "hi\n").unwrap();
    assert_matches("(require 'esh-util)", &[
        &format!(r#"(let ((b (find-file-noselect "{}"))) (prog1 (with-current-buffer b (list major-mode (buffer-string))) (kill-buffer b)))"#, file.display()),
    ]);
}

/// `eval-buffer' and `eval-region' (lread.c readevalloop): dialect from the
/// cookie, a top-level `(defvar NAME)' special for the later forms, eager
/// macro expansion, PRINTFLAG streams, narrowing, errors, READ-FUNCTION,
/// `load-history', and `M-x eval-buffer'; `load' alongside.
#[test]
fn eval_buffer_and_region_evaluate_as_gnu() {
    let dir = std::env::temp_dir().join(format!("journey-eval-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let file = dir.join("ev1.el");
    std::fs::write(&file, ";;; -*- lexical-binding: t -*-\n(defvar ev-dyn)\n(defun ev-read-dyn () ev-dyn)\n(defun ev-f (x) (* x 2))\n\
        (setq ev-r (let ((ev-dyn 5)) (ev-read-dyn)))\n(defmacro ev-m () 41)\n(setq ev-m (1+ (ev-m)))\n\
        (setq ev-lex (let ((y 3)) (funcall (lambda () y))))\n").unwrap();
    let file = file.display().to_string();
    assert_matches("", &[
        "(list (commandp 'eval-buffer) (commandp 'eval-region) (interactive-form 'eval-buffer) (interactive-form 'eval-region))",
        &format!(r#"(with-current-buffer (find-file-noselect "{file}") (list (eval-buffer) ev-r (ev-f 4) ev-m ev-lex (point) (assoc "{file}" load-history)))"#),
        r#"(with-temp-buffer (insert ";; -*- lexical-binding: nil -*-\n(setq ev-d (let ((z 1)) (boundp 'z)))") (eval-buffer) ev-d)"#,
        r#"(with-temp-buffer (insert "(+ 1 2) \"s\" ; c\n(list 'a)") (list (with-output-to-string (eval-buffer nil standard-output)) (car values)))"#,
        r#"(with-temp-buffer (insert "(princ \"hidden\") 7") (with-output-to-string (eval-buffer)))"#,
        r#"(progn (with-current-buffer (get-buffer-create "ev-named") (erase-buffer) (insert "(setq ev-n 9)")) (list (eval-buffer "ev-named") ev-n))"#,
        r#"(eval-buffer "jp-no-such-buffer")"#,
        r#"(with-temp-buffer (insert "(setq ev-a 1) (setq ev-b 2)") (setq ev-a 0 ev-b 0) (narrow-to-region 1 14) (eval-buffer) (list ev-a ev-b (point)))"#,
        r#"(with-temp-buffer (insert "(setq ev-e 1) (car 1) (setq ev-e 2)") (list (condition-case e (eval-buffer) (error e)) ev-e (point)))"#,
        r#"(with-temp-buffer (insert "(setq ev-x 1) (setq ev-y 2) (setq ev-z 3)") (setq ev-x 0 ev-y 0 ev-z 0) (goto-char 5) (list (eval-region 1 28) ev-x ev-y ev-z (point)))"#,
        r#"(with-temp-buffer (insert "(* 6 7) (list 1)") (with-output-to-string (eval-region (point-min) (point-max) standard-output)))"#,
        r#"(with-temp-buffer (insert "(a) (b) (c)") (let (seen) (eval-region 1 (point-max) nil (lambda (s) (let ((f (read s))) (push f seen) 1))) (nreverse seen)))"#,
        r#"(with-temp-buffer (insert "(setq ev-i 1) (car") (condition-case e (eval-region 1 (point-max)) (error (list (car e) ev-i))))"#,
        r#"(with-temp-buffer (insert ";; -*- lexical-binding: t -*-\n(setq ev-cmd (+ 1 2))") (setq ev-cmd nil) (call-interactively 'eval-buffer) ev-cmd)"#,
        r#"(with-temp-buffer (insert "(setq ev-reg 4)") (set-mark 1) (goto-char (point-max)) (call-interactively 'eval-region) ev-reg)"#,
        r#"(with-temp-buffer (setq-local lexical-binding nil) (insert "(setq ev-c (let ((z 7)) (boundp 'z)))") (eval-buffer) ev-c)"#,
        r#"(with-temp-buffer (insert "(save-excursion (goto-char (point-max)) (delete-char -1)) (setq ev-s 5)  ") (setq ev-s 0) (eval-region 1 (point-max)) ev-s)"#,
        &format!(r#"(progn (setq ev-r nil) (load "{file}" nil t) ev-r)"#),
    ]);
}

/// Every command GNU defines in C, as Romacs defines it (in Rust or as a
/// Lisp stand-in), with GNU's `interactive-form'; M-f through its binding.
#[test]
fn gnu_c_commands_have_gnu_interactive_forms() {
    let binary = std::env::var("ROMACS_GNU_ORACLE").expect("pinned GNU oracle");
    let listing = "(let (names) (mapatoms (lambda (s) (if (and (fboundp s) (subr-primitive-p (symbol-function s)) (commandp s)) (push s names)))) \
                   (dolist (s (sort names #'string<)) (princ s) (princ \"\\n\")))";
    let output = std::process::Command::new(&binary).args(["-Q", "--batch", "--eval", listing]).output().unwrap();
    assert!(output.status.success(), "GNU: {}", String::from_utf8_lossy(&output.stderr));
    let names: Vec<String> = String::from_utf8(output.stdout).unwrap().lines().map(str::to_owned).collect();
    assert!(names.len() > 90, "GNU lists its C commands");
    let mut vm = romacs();
    // `signal-process' is Lisp in rlisp/vm/subprocess.el, without GNU's spec yet.
    let defined: Vec<String> = names.into_iter().filter(|name| name != "signal-process" && vm
        .eval_source(&format!("(fboundp '{name})")).and_then(|v| v.display()).as_deref() == Ok("t")).collect();
    let forms: Vec<String> = defined.iter().map(|name| format!("(interactive-form '{name})")).collect();
    let mut expressions: Vec<&str> = forms.iter().map(String::as_str).collect();
    let motion = r#"(with-temp-buffer (insert "alpha beta gamma") (goto-char 1)
                      (let ((command (key-binding (kbd "M-f")))) (call-interactively command)
                        (let ((current-prefix-arg 2)) (call-interactively command)) (list command (point))))"#;
    expressions.push(motion);
    assert_matches("", &expressions);
}
