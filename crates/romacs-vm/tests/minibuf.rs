//! read-from-minibuffer / read-string (prims/minibuf.rs) beyond the recorded
//! corpus (tools/primh/minibuf-specs): what the corpus cannot state because it
//! needs the VM's own collector, or the GNU-recorded facts that must keep
//! holding under a collection forced in the middle of the prompt.
//!
//! Every expected value here was taken from live GNU 32.0.50 in batch with
//! `executing-kbd-macro' t (ert-x.el `ert-simulate-keys'); the same sources
//! are probes in the recorded corpus.
use romacs_vm::Vm;

/// The printed value, or the VM's error, so a missing primitive fails at the
/// assertion that names the expected GNU answer.
fn eval(vm: &mut Vm, source: &str) -> String {
    match vm.eval_source(source) {
        Ok(value) => value.display().unwrap(),
        Err(e) => format!("VM-ERROR {e}"),
    }
}

fn keys(events: &str, body: &str) -> String {
    format!("(let ((unread-command-events (append {events} nil)) (executing-kbd-macro t)) {body})")
}

/// GNU: RET returns the typed text; empty input returns "" even with a
/// DEFAULT-VALUE (minibuf.c:1337-1340), read-string returns DEFAULT-VALUE
/// itself (minibuf.c:1457, `eq'), READ returns a datum, C-g signals
/// `minibuffer-quit' (a `quit', not an `error').
#[test]
fn prompt_shape_matches_gnu() {
    let mut vm = Vm::new();
    vm.set_collection_floor(1);
    for (source, gnu) in [
        (keys("\"abc\\r\"", "(read-from-minibuffer \"P: \")"), "\"abc\""),
        (keys("\"\\r\"", "(read-from-minibuffer \"P: \" nil nil nil nil \"dflt\")"), "\"\""),
        (keys("\"\\r\"", "(let ((d (copy-sequence \"dd\"))) (eq d (read-string \"P: \" nil nil d)))"), "t"),
        (keys("\"\\r\"", "(read-string \"P: \" nil nil '(5 6))"), "5"),
        (keys("\"(a b)\\r\"", "(read-from-minibuffer \"P: \" nil nil t)"), "(a b)"),
        (keys("\"\\r\"", "(read-from-minibuffer \"P: \" nil nil t nil \"(d)\")"), "(d)"),
        (keys("\"Z\\r\"", "(read-from-minibuffer \"P: \" '(\"init\" . 3))"), "\"inZit\""),
        (keys("\"Z\\r\"", "(read-from-minibuffer \"P: \" \"init\")"), "\"initZ\""),
        (keys("\"ab\\C-g\"", "(condition-case q (read-from-minibuffer \"P: \") (error (list 'error q)) (quit (list 'quit q)))"), "(quit (minibuffer-quit))"),
        ("(condition-case e (read-from-minibuffer \"P: \") (error e))".into(), "(end-of-file \"Error reading from stdin\")"),
        // read_minibuf_unwind (minibuf.c:1111) erases the level's buffer.
        (keys("\"abc\\r\"", "(list (read-from-minibuffer \"P: \" \"in\") (with-current-buffer (get-buffer-create \" *Minibuf-1*\") (buffer-size)))"), "(\"inabc\" 0)"),
        // minibuf.c:1100: properties survive only with minibuffer-allow-text-properties.
        (keys("\"\\r\"", "(let ((i (copy-sequence \"x\")) (minibuffer-allow-text-properties t)) (put-text-property 0 1 'q 1 i) (let ((r (read-from-minibuffer \"P: \" i))) (list (substring-no-properties r) (get-text-property 0 'q r))))"), "(\"x\" 1)"),
        (keys("\"\\r\"", "(let ((i (copy-sequence \"x\"))) (put-text-property 0 1 'q 1 i) (let ((r (read-from-minibuffer \"P: \" i))) (list (substring-no-properties r) (get-text-property 0 'q r))))"), "(\"x\" nil)"),
    ] {
        assert_eq!(eval(&mut vm, &source), gnu, "{source}");
    }
}

/// A command typed at the prompt allocates until the collector runs while the
/// primitive is suspended in its T2 continuation. PROMPT, INITIAL-CONTENTS and
/// DEFAULT-VALUE wait only in traced registers; their identity (`eq') and
/// bytes must survive. The same request with a command that does not allocate
/// counts the request-boundary collections, so a larger count proves at least
/// one collection happened inside the prompt (`garbage-collect' itself does
/// not collect mid-request in VM2, so the churn allocates instead).
#[test]
fn collection_during_prompt_keeps_arguments_and_bytes() {
    let source = keys(
        "[?a f7 f7 ?b ?\\r f7 f7 ?\\r]",
        "(let ((p (copy-sequence \"Prompt: \")) (i (copy-sequence \"in\")) (d (copy-sequence \"(dflt)\"))) \
           (let ((r1 (read-from-minibuffer p i romacs-map nil nil d)) \
                 (r2 (let ((minibuffer-local-map romacs-map)) (read-string p nil nil d)))) \
             (list r1 p i d (eq r2 d) r2)))",
    );
    let mut run = |command: &str| {
        let mut vm = Vm::new();
        vm.set_collection_floor(1);
        eval(&mut vm, &format!("(defun romacs-churn () (interactive) {command})"));
        eval(&mut vm, "(setq romacs-map (make-sparse-keymap)) (set-keymap-parent romacs-map minibuffer-local-map) (define-key romacs-map [f7] 'romacs-churn)");
        let before = vm.heap_stats().collections;
        let result = eval(&mut vm, &source);
        (result, vm.heap_stats().collections - before)
    };
    let (quiet, boundary) = run("nil");
    let (result, collections) =
        run("(let ((i 0)) (while (< i 5000) (list i (make-string 3 ?z)) (setq i (1+ i))))");
    eprintln!("collections: {boundary} without allocation, {collections} with");
    assert!(collections > boundary, "no collection inside the prompts: {boundary} vs {collections}");
    assert_eq!(quiet, result);
    assert_eq!(result, "(\"inab\" \"Prompt: \" \"in\" \"(dflt)\" t \"(dflt)\")");
}

/// While the prompt is active the minibuffer window is selected and shows a
/// buffer holding PROMPT then INITIAL; afterwards the caller's buffer and
/// window are back and the minibuffer is empty again (read_minibuf_unwind).
#[test]
fn prompt_is_displayed_in_the_minibuffer_window() {
    let mut vm = Vm::new();
    vm.set_collection_floor(1);
    eval(&mut vm, "(defun romacs-spy () (interactive) (setq romacs-spy-v (list (buffer-substring-no-properties (point-min) (point-max)) (point) (eq (selected-window) (minibuffer-window)) (eq (current-buffer) (window-buffer (minibuffer-window))) (get-text-property 1 'field) (get-text-property 1 'read-only))))");
    eval(&mut vm, "(setq romacs-map (make-sparse-keymap)) (set-keymap-parent romacs-map minibuffer-local-map) (define-key romacs-map [f5] 'romacs-spy)");
    let result = eval(&mut vm, &keys(
        "[?a f5 ?\\r]",
        "(let ((b (current-buffer)) (w (selected-window))) \
           (list (read-from-minibuffer \"P: \" \"in\" romacs-map) romacs-spy-v (eq b (current-buffer)) (eq w (selected-window)) \
                 (with-current-buffer (window-buffer (minibuffer-window)) (buffer-size))))",
    ));
    assert_eq!(result, "(\"ina\" (\"P: ina\" 7 t t t t) t t 0)");
}

/// Review finding (Sol, 1bcb3487): a command at the prompt that deletes the
/// calling window made the unwind signal before the active level was reset,
/// so every later prompt was refused as recursive. GNU 32.0.50, same keys
/// with `split-window'/`delete-window': the prompt returns "", the default is
/// unbound again and the next prompt works: ("" nil ""). VM2's
/// `delete-window-internal' is used because GNU's window.el is not loaded.
#[test]
fn deleting_the_calling_window_does_not_poison_later_prompts() {
    let mut vm = Vm::new();
    vm.set_collection_floor(1);
    let result = eval(&mut vm, "(progn \
        (setq a (selected-window)) (set-window-new-pixel a 230) \
        (setq b (split-window-internal a 230 nil 0.5)) \
        (set-window-new-pixel (window-parent a) 460) (set-window-new-pixel b 460) \
        (defun del-caller () (interactive) (delete-window-internal a)) \
        (setq m (make-sparse-keymap)) (set-keymap-parent m minibuffer-local-map) (define-key m [f8] 'del-caller) \
        (let ((executing-kbd-macro t) (unread-command-events '(f8 13))) \
          (list (condition-case e (read-from-minibuffer \"P: \" nil m nil nil \"inner\") (error e)) \
                minibuffer-default \
                (condition-case e (read-string \"Next: \") (error e)))))");
    assert_eq!(result, "(\"\" nil \"\")");
}

/// The first prompt of a fresh VM, as the first and only top-level form, must
/// reach the Lisp half. That form compiles to a function only a few registers
/// wide, so the VM's register width is still that narrow when the install
/// request (`eval' of minibuf.el, nine saved registers) is pushed; the T2
/// engine refused it as `S0 call depth exceeded' at depth 1, left the Lisp
/// half uninstalled, and every later bare prompt failed the same way. Forms
/// wrapped in a wider `let' or a `condition-case' widened the registers first
/// and hid it. GNU: batch with no macro signals end-of-file (minibuf.c:659);
/// an empty macro returns "".
#[test]
fn first_prompt_on_a_fresh_vm_reaches_the_lisp_half() {
    for (source, expected) in [
        ("(read-from-minibuffer \"P: \")", "VM-ERROR (end-of-file \"Error reading from stdin\")"),
        ("(read-string \"P: \")", "VM-ERROR (end-of-file \"Error reading from stdin\")"),
        ("(let ((executing-kbd-macro t)) (read-from-minibuffer \"P: \"))", "\"\""),
        ("(let ((executing-kbd-macro t)) (read-string \"P: \"))", "\"\""),
    ] {
        let mut vm = Vm::new();
        assert_eq!(eval(&mut vm, source), expected, "first form of a fresh VM: {source}");
        assert_eq!(eval(&mut vm, "(fboundp 'romacs--read-minibuf)"), "t", "installed by: {source}");
        assert_eq!(eval(&mut vm, source), expected, "second form: {source}");
    }
}

/// The same widening one frame down: the install request is pushed from a
/// narrow function while its top-level caller is suspended, so the widening
/// rebases a live frame. (The register stack keeps its capacity across runs,
/// so this does not force the stack to relocate; the rebase mirrors
/// `force_lazy`'s.)
#[test]
fn first_prompt_widening_with_a_suspended_caller() {
    let mut vm = Vm::new();
    eval(&mut vm, "(setq executing-kbd-macro t)");
    eval(&mut vm, "(defalias 'romacs-w1 (lambda (a) (read-string a)))");
    assert_eq!(eval(&mut vm, "(romacs-w1 \"P: \")"), "\"\"");
    assert_eq!(eval(&mut vm, "(list \"x\" (romacs-w1 \"P: \") \"y\")"), "(\"x\" \"\" \"y\")");
}

/// GNU `string_to_object` (minibuf.c:269) reads the answer with
/// `read-from-string`: a `\N{NAME}` calls `char-from-name` (here a stand-in
/// with GNU's answer for BULLET), and trailing text other than whitespace
/// signals.
#[test]
fn a_read_answer_may_call_lisp_and_refuses_trailing_text() {
    let mut vm = Vm::new();
    eval(&mut vm, r#"(defun char-from-name (name &optional _) (and (equal name "BULLET") 8226))"#);
    for (source, gnu) in [
        (keys(r#""?\\N{BULLET}\r""#, r#"(read-from-minibuffer "P: " nil nil t)"#), "8226"),
        (keys(r#""(a)  \r""#, r#"(read-from-minibuffer "P: " nil nil t)"#), "(a)"),
        (keys(r#""1 2\r""#, r#"(condition-case e (read-from-minibuffer "P: " nil nil t) (error e))"#),
         r#"(invalid-read-syntax "Trailing garbage following expression")"#),
    ] {
        assert_eq!(eval(&mut vm, &source), gnu, "{source}");
    }
}
