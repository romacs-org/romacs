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
