#[path = "../src/command.rs"] mod command;
#[path = "../src/echo.rs"] mod echo;
#[path = "../src/eval_last_sexp.rs"] mod eval_last_sexp;
#[path = "../src/loadup.rs"] mod loadup;
#[path = "../src/native.rs"] mod native;
#[path = "../src/presentation.rs"] mod presentation;
#[path = "../src/startup.rs"] mod startup;
use romacs_vm::Vm;
fn snapshot(vm: &mut Vm, _: bool, width: u32, height: u32) -> Result<romacs_layout_input::FrameSnapshot, String> { vm.frame_snapshot(width, height) }

fn setup(height: u32) -> Vm {
    let mut vm = Vm::new();
    vm.set_frame_geometry(1000, height, 20, 10).unwrap();
    startup::initialize_files(&mut vm, &[]).unwrap();
    vm.eval_source("(romacs-gui-delete-other-windows) (setq anchor (selected-window) tree (get-buffer-create \"tree\") message-buffer (get-buffer-create \"message\")) (switch-to-buffer tree) (insert \"tree content\") (goto-char 4)").unwrap();
    vm
}
fn printed(vm: &mut Vm, source: &str) -> String {
    vm.eval_source(source).unwrap_or_else(|e| panic!("{source}: {e}")).display().unwrap()
}

#[test]
fn new_pair_returns_lower_window_with_quarter_anchor_and_preserves_selection() {
    let mut vm = setup(700);
    assert_eq!(printed(&mut vm, "(progn (setq original-height (window-pixel-height anchor) pair (romacs-display-buffer-paired message-buffer anchor nil 0.25)) (list (eq anchor (selected-window)) (eq (current-buffer) tree) (eq (window-buffer anchor) tree) (eq (window-buffer pair) message-buffer) (eq pair (window-next-sibling anchor)) (window-point anchor) (window-pixel-height anchor) (window-pixel-height pair) (= original-height (+ (window-pixel-height anchor) (window-pixel-height pair)))))"), "(t t t t t 4 170 510 t)");
}

#[test]
fn reuse_keeps_geometry_and_does_not_select_the_pair() {
    let mut vm = setup(700);
    assert_eq!(printed(&mut vm, "(progn (setq pair (romacs-display-buffer-paired message-buffer anchor nil 0.25) replacement (get-buffer-create \"replacement\")) (list (eq pair (romacs-display-buffer-paired replacement anchor pair 0.25)) (length (romacs-gui-leaves (romacs-gui-root)))) (list (eq pair (romacs-display-buffer-paired message-buffer anchor pair 0.25)) (length (romacs-gui-leaves (romacs-gui-root)))) (list (eq anchor (selected-window)) (window-pixel-height anchor) (window-pixel-height pair) (eq (window-buffer pair) message-buffer) (length (romacs-gui-leaves (romacs-gui-root)))))"), "(t 170 510 t 2)");
}

#[test]
fn small_anchor_falls_back_and_minimum_clamps_quarter_split() {
    let mut vm = setup(200);
    assert_eq!(printed(&mut vm, "(list (eq anchor (romacs-display-buffer-paired message-buffer anchor nil 0.25)) (eq anchor (selected-window)) (length (romacs-gui-leaves (romacs-gui-root))) (eq (window-buffer anchor) message-buffer))"), "(t t 1 t)");
    let mut vm = setup(240);
    assert_eq!(printed(&mut vm, "(progn (setq pair (romacs-display-buffer-paired message-buffer anchor nil 0.25)) (list (window-pixel-height anchor) (window-pixel-height pair)))"), "(100 120)");
}

#[test]
fn close_removes_exact_pair_restores_anchor_and_refuses_stale_or_sole() {
    let mut vm = setup(700);
    assert_eq!(printed(&mut vm, "(progn (setq pair (romacs-display-buffer-paired message-buffer anchor nil 0.25)) (select-window pair) (list (romacs-delete-paired-window pair) (window-live-p pair) (eq anchor (selected-window)) (window-pixel-height anchor) (window-point anchor) (romacs-delete-paired-window pair) (romacs-delete-paired-window anchor) (romacs-delete-paired-window nil) (romacs-delete-paired-window 42)))"), "(t nil t 680 4 nil nil nil nil)");
}

#[test]
fn invalid_pairs_are_not_reused_and_invalid_anchors_are_rejected() {
    let mut vm = setup(700);
    assert_eq!(printed(&mut vm, "(progn (setq pair (romacs-display-buffer-paired message-buffer anchor anchor 0.25)) (romacs-delete-paired-window pair) (setq new-pair (romacs-display-buffer-paired message-buffer anchor pair 0.25)) (list (not (eq pair new-pair)) (window-live-p new-pair) (condition-case nil (romacs-display-buffer-paired message-buffer pair nil 0.25) (error 'invalid)) (romacs-delete-paired-window (window-parent anchor))))"), "(t t invalid nil)");
}

#[test]
fn unselected_anchor_preserves_other_windows_and_pair_close_stays_local() {
    let mut vm = setup(700);
    assert_eq!(printed(&mut vm, "(progn (setq foreign (romacs-gui-split nil)) (select-window foreign) (switch-to-buffer (get-buffer-create \"foreign\")) (insert \"untouched\") (goto-char 6) (setq foreign-buffer (current-buffer) foreign-height (window-pixel-height foreign) foreign-width (window-pixel-width foreign)) (setq pair (romacs-display-buffer-paired message-buffer anchor nil 0.25)) (list (eq foreign (selected-window)) (eq foreign-buffer (current-buffer)) (window-point foreign) (= foreign-height (window-pixel-height foreign)) (= foreign-width (window-pixel-width foreign)) (eq foreign-buffer (window-buffer foreign)) (length (romacs-gui-leaves (romacs-gui-root))) (romacs-delete-paired-window pair) (= foreign-height (window-pixel-height foreign)) (eq foreign (selected-window)) (window-point foreign) (window-point anchor) (window-pixel-height anchor)))"), "(t t 6 t t t 3 t t t 6 4 340)");
}

#[test]
fn horizontal_foreign_window_survives_pair_creation_and_close() {
    let mut vm = setup(700);
    assert_eq!(printed(&mut vm, "(progn (setq foreign (romacs-gui-split t) width (window-pixel-width foreign)) (setq pair (romacs-display-buffer-paired message-buffer anchor nil 0.25)) (list (= width (window-pixel-width foreign)) (window-pixel-height foreign) (romacs-delete-paired-window pair) (= width (window-pixel-width foreign)) (window-pixel-height foreign) (window-pixel-height anchor)))"), "(t 680 t t 680 680)");
}
