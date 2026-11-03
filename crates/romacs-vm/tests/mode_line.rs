use romacs_vm::Vm;

const SCENE: &str = r##"
(defface mode-line '((t (:background "#303030"))) "band")
(set-buffer (get-buffer-create "g1"))
(insert "Ωμέγα Hello\n")
(set-window-buffer nil (current-buffer))
"##;

#[test]
fn band_expands_displayed_buffer_not_current_buffer() {
    let mut vm = Vm::new();
    vm.eval_source(SCENE).unwrap();
    vm.eval_source(r#"(setq mode-line-format '(" %b ")) (set-buffer (get-buffer-create "other"))"#)
        .unwrap();
    let s = vm.display_snapshot(400, 300).unwrap();
    assert_eq!(
        s.mode_line.as_ref().map(|b| b.utf8.as_str()),
        Some(" g1 "),
        "displayed-buffer band"
    );
    assert_eq!(
        s.faces[s.mode_line_face.0 as usize].background,
        [48, 48, 48]
    );
    let current = vm
        .eval_source("(eq (current-buffer) (get-buffer-create \"other\"))")
        .unwrap();
    assert_eq!(current.display().unwrap(), "t", "restore current buffer");
}

#[test]
fn mutation_waits_for_paint_boundary_and_diagnostic_names_construct() {
    let mut vm = Vm::new();
    vm.eval_source(SCENE).unwrap();
    vm.eval_source(r#"(setq mode-line-format '(" %b " (:eval (progn (set-window-vscroll nil 7 t) (format "v=%S" (window-vscroll nil t))))))"#).unwrap();
    let n = vm.display_snapshot(400, 300).unwrap();
    assert_eq!(
        n.mode_line.as_ref().map(|b| b.utf8.as_str()),
        Some(" g1 v=0"),
        "mutation must not apply mid-build"
    );
    assert_eq!(n.vscroll_px, 0);
    let diagnostics = vm.take_display_diagnostics();
    assert_eq!(diagnostics.len(), 1, "one mutation diagnostic");
    assert!(diagnostics[0].contains(":eval") && diagnostics[0].contains("set-window-vscroll"));
    assert!(vm.display_apply_journal());
    let next = vm.display_snapshot(400, 300).unwrap();
    assert_eq!(next.vscroll_px, 7, "next snapshot sees committed scroll");
    assert_eq!(next.mode_line.unwrap().utf8, " g1 v=7");
    assert!(
        !vm.display_apply_journal(),
        "idempotent setter does not loop"
    );
    assert!(vm.take_display_diagnostics().is_empty());
}

#[test]
fn conditionals_select_gnus_branch_and_symbol_strings_are_literal() {
    let mut vm = Vm::new();
    vm.eval_source(SCENE).unwrap();
    vm.eval_source(r#"(setq band-symbol "%b" mode-line-format '("" band-symbol))"#)
        .unwrap();
    assert_eq!(
        vm.display_snapshot(400, 300)
            .unwrap()
            .mode_line
            .unwrap()
            .utf8,
        "%b",
        "symbol string is literal, GNU xdisp.c"
    );
    // Conditionals are GNU's now (xdisp.c:28849): an unbound symbol selects
    // the else branch -- GNU 32.0.50 formats `(missing "yes" "no")` as "no"
    // (crates/romacs-vm-gui/evidence/tierb/gnu/mode-line.txt). Before the
    // Tier B engine this asserted the staged subset's refusal instead.
    vm.eval_source(r#"(setq mode-line-format '(missing "yes" "no"))"#)
        .unwrap();
    assert_eq!(
        vm.display_snapshot(400, 300).unwrap().mode_line.unwrap().utf8,
        "no",
        "a conditional must select a branch, never concatenate both"
    );
}

#[test]
fn journal_last_write_back_to_committed_state_cancels_pending_change() {
    let mut vm = Vm::new();
    vm.eval_source(SCENE).unwrap();
    vm.eval_source(r#"(setq mode-line-format '(:eval (progn (set-window-vscroll nil 7 t) (set-window-vscroll nil 0 t) "ok")))"#).unwrap();
    assert_eq!(
        vm.display_snapshot(400, 300)
            .unwrap()
            .mode_line
            .unwrap()
            .utf8,
        "ok"
    );
    assert!(
        !vm.display_apply_journal(),
        "last journal write must win, including zero"
    );
    assert_eq!(vm.display_snapshot(400, 300).unwrap().vscroll_px, 0);
}

#[test]
fn error_construct_is_empty_and_siblings_survive_collection() {
    let mut vm = Vm::new();
    vm.set_collection_floor(1);
    vm.eval_source(SCENE).unwrap();
    vm.eval_source(r#"(setq mode-line-format '("" "a" (:eval (error "boom")) "b"))"#)
        .unwrap();
    let s = vm.display_snapshot(400, 300).unwrap();
    assert_eq!(
        s.mode_line.as_ref().map(|b| b.utf8.as_str()),
        Some("ab"),
        "error drops only its construct"
    );
    let diagnostics = vm.take_display_diagnostics();
    assert_eq!(diagnostics.len(), 1);
    assert!(
        diagnostics[0].contains("boom"),
        "real error, not undefined-function"
    );
    assert!(!diagnostics[0].contains("undefined"));
    // Force moving collections inside a successful :eval, not just setup.
    vm.eval_source(r#"(setq mode-line-format '("a" (:eval (progn (let ((i 0)) (while (< i 2000) (vector i) (setq i (1+ i)))) (copy-sequence "middle"))) "b"))"#).unwrap();
    vm.set_collection_floor(1);
    let before = vm.heap_stats().collections;
    let s = vm.display_snapshot(400, 300).unwrap();
    assert_eq!(s.mode_line.unwrap().utf8, "amiddleb");
    assert!(vm.heap_stats().collections > before);
    let alive = vm.eval_source("(+ 2 3)").unwrap();
    assert_eq!(alive.display().unwrap(), "5");
}

/// GNU's own mode-line variables hold `propertize`d strings (bindings.el
/// `mode-line-modified`, `propertized-buffer-identification`): each
/// expands as its text, a %-construct when it is an element or heads a
/// list, literally when it is a symbol's value (`display_mode_element`).
#[test]
fn propertized_strings_expand_as_their_text() {
    let mut vm = Vm::new();
    vm.eval_source(SCENE).unwrap();
    vm.eval_source(r#"(setq ml-literal (propertize "%b" 'face 'bold)
                            mode-line-format
                            (list (propertize "[%b]" 'help-echo "name")
                                  (list (propertize "%1*" 'local-map (make-sparse-keymap)) (propertize "%1+" 'help-echo "m"))
                                  '(:eval (propertize "E" 'face 'bold))
                                  'ml-literal))"#).unwrap();
    let s = vm.display_snapshot(400, 300).unwrap();
    assert_eq!(s.mode_line.as_ref().map(|b| b.utf8.as_str()), Some("[g1]**E%b"));
}

#[path = "support/gui_probe.rs"]
mod gui_probe;

/// Romacs's mode line (rlisp/romacs-mode-line.el) enters the evaluator
/// only where a region shows or a package's element asks: the evaluator
/// entries `Vm::mode_line_evals' counts for one expansion of the
/// installed format, over a file buffer, with `vc-mode' set, with a
/// region, and in `flymake-mode' with three diagnostics, where it makes
/// none: the counts are made when flymake reports.
#[test]
fn romacs_mode_line_enters_the_evaluator_only_where_due() {
    let mut vm = gui_probe::gnu_session();
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    for file in ["rlisp/romacs-layer.el", "rlisp/romacs-chrome.el", "rlisp/romacs-mode-line.el"] {
        vm.load_file(&root.join(file)).unwrap();
    }
    let eval = |vm: &mut Vm, form: &str| vm.eval_source(form).unwrap_or_else(|e| panic!("{form}: {e}")).display().unwrap();
    let dir = std::env::temp_dir().join(format!("romacs-mode-line-evals-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let file = dir.join("a.el");
    std::fs::write(&file, ";;; a.el -*- lexical-binding: t -*-\n(defun a () 1)\n(defun b () 2)\n").unwrap();
    eval(&mut vm, "(progn (setq noninteractive nil) (romacs-mode-line-mode 1))");
    eval(&mut vm, &format!("(switch-to-buffer (find-file-noselect {:?}))", file.to_str().unwrap()));
    let evals = |vm: &mut Vm, format: &str| {
        let before = vm.mode_line_evals();
        eval(vm, &format!("(format-mode-line {format})"));
        vm.mode_line_evals() - before
    };
    assert_eq!(eval(&mut vm, "(eq mode-line-format romacs-mode-line-format)"), "t");
    assert_eq!(evals(&mut vm, "mode-line-format"), 0, "a file buffer");
    eval(&mut vm, "(setq vc-mode \" Git-main\")");
    assert_eq!(evals(&mut vm, "mode-line-format"), 0, "with vc-mode");
    eval(&mut vm, "(progn (transient-mark-mode 1) (goto-char 1) (push-mark (point) t t) (forward-line 2))");
    assert_eq!(evals(&mut vm, "mode-line-format"), 1, "with a region");
    eval(&mut vm, "(deactivate-mark)");
    eval(&mut vm, "(progn (require 'flymake) (setq trusted-content :all)
        (defun mle-backend (report &rest _)
          (funcall report (list (flymake-make-diagnostic (current-buffer) 1 3 :error \"e\")
                                (flymake-make-diagnostic (current-buffer) 4 6 :warning \"w\")
                                (flymake-make-diagnostic (current-buffer) 7 9 :note \"n\"))))
        (setq-local flymake-diagnostic-functions '(mle-backend)) (flymake-mode 1) (flymake-start))");
    assert_eq!(eval(&mut vm, "(length (flymake-diagnostics))"), "3");
    let gnu = evals(&mut vm, "flymake-mode-line-counters");
    assert!(gnu > 0);
    // Romacs's diagnostics are counted when flymake reports, not in the
    // format: none of GNU's element's entries.
    assert_eq!(evals(&mut vm, "mode-line-format"), 0, "flymake's counts, refreshed on report");
    let line = eval(&mut vm, "(let ((noninteractive nil)) (substring-no-properties (format-mode-line mode-line-format)))");
    assert!(line.contains("1") && line.matches('1').count() >= 3, "{line}");
    eval(&mut vm, "(progn (flymake-mode -1) (set-buffer-modified-p nil) (kill-buffer))");
    std::fs::remove_dir_all(&dir).ok();
}

/// rlisp/romacs-mode-line.el's own tests (tests/romacs-mode-line-test.el):
/// segments refreshed by their hooks, the installed format's components,
/// and the ownership of `mode-line-format', in a session with GNU's Lisp.
#[test]
fn romacs_mode_line_lisp_tests_pass() {
    let mut vm = gui_probe::gnu_session();
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    vm.eval_source(&format!("(setq load-path (cons {:?} load-path))", root.join("rlisp").to_str().unwrap())).unwrap();
    for file in ["rlisp/romacs-test.el", "rlisp/romacs-layer.el", "rlisp/romacs-chrome.el", "rlisp/romacs-icons.el",
                 "rlisp/romacs-mode-line.el", "crates/romacs-vm/tests/romacs-mode-line-test.el"] {
        vm.load_file(&root.join(file)).unwrap_or_else(|e| panic!("{file}: {e}"));
    }
    let summary = vm.eval_source("(let ((s (romacs-test-run t))) (format \"%S\" s))").unwrap().display().unwrap();
    assert!(summary.contains(":failed 0") && summary.contains(":errors 0"), "{summary}");
    // The Lisp icon table is the font's: assets/icons/codepoints.txt.
    let table = std::fs::read_to_string(root.join("assets/icons/codepoints.txt")).unwrap();
    let font: Vec<String> = table.lines().filter(|l| !l.starts_with('#') && !l.is_empty())
        .map(|l| { let (name, point) = l.split_once(' ').unwrap(); format!("{name} {}", i64::from_str_radix(&point[2..], 16).unwrap()) }).collect();
    let font = font.iter().map(|l| format!("{l:?}")).collect::<Vec<_>>().join(" ");
    let same = vm.eval_source(&format!("(equal (mapcar (lambda (e) (format \"%s %d\" (car e) (cadr e))) romacs-icons-alist) '({font}))"))
        .unwrap().display().unwrap();
    assert_eq!(same, "t", "romacs-icons-alist and assets/icons/codepoints.txt differ");
}
