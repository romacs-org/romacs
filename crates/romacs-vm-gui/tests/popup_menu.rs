//! menu.c's popup menus on the GUI's `pgtk' frame, through GNU's text
//! menus (lisp/romacs/romacs-menu.el, DIVERGENCES VM2-MENU1), on the real
//! `--loadup' chain with the GUI's display and its keyboard
//! (`romacs_vm::HostInput`). tmm gives each item a shortcut key, the same
//! in pinned GNU and Romacs. Expected values: pinned GNU's text menus
//! (`emacs -nw`) for (alpha) and a pane's VALUE 2; menu.c's
//! find_and_return_menu_selection for a submenu's (sub beta); its contract
//! for cancelling (a quit, or nil after a mouse event); and pinned GNU's
//! `File > New Window Below', `split-window-below'. `popup-menu' runs the
//! chosen command and returns its value.
use romacs_vm_gui::{command, fonts, measure, presentation, startup};
use romacs_vm::{CommandEvent, HostEvent, HostFrame, HostInput, Vm};
use std::collections::VecDeque;

struct Keys(VecDeque<HostEvent>);
impl HostInput for Keys {
    fn input_pending(&mut self) -> bool { false }
    fn redisplay(&mut self, _: HostFrame) -> Result<Vec<romacs_vm::WindowEnd>, String> { Ok(vec![]) }
    fn next_event(&mut self, _: Option<std::time::Duration>) -> Result<HostEvent, String> {
        self.0.pop_front().ok_or_else(|| "menu host script exhausted".into())
    }
}

fn printed(vm: &mut Vm, source: &str) -> String {
    vm.eval_source(source).and_then(|v| v.display()).unwrap_or_else(|e| format!("VM-ERROR {e}"))
}

/// Run EVENT's command as the GUI's outer loop does, the menu reading SCRIPT.
fn command(vm: &mut Vm, event: CommandEvent, script: &[i64]) -> Option<romacs_vm::CommandSignal> {
    let keys = script.iter().map(|&c| HostEvent::Key(CommandEvent::Character(c))).collect();
    vm.set_host_input(Some(Box::new(Keys(keys))));
    let (mut commands, mut presentation) = (command::Commands::default(), presentation::Presentation::new());
    presentation.command(vm, &mut commands, event).unwrap().and_then(|o| o.signal)
}

const MENU: &str = "(let ((map (make-sparse-keymap \"Probe\")) (sub (make-sparse-keymap \"Sub\")))
  (define-key sub [beta] '(menu-item \"Beta\" (lambda () (interactive) (setq menu-probe-ran 'beta))))
  (define-key map [sub] (cons \"Sub\" sub))
  (define-key map [alpha] '(menu-item \"Alpha\" (lambda () (interactive) (setq menu-probe-ran 'alpha))))
  (define-key map [hidden] '(menu-item \"Hidden\" ignore :visible nil))
  (define-key map [disabled] '(menu-item \"Disabled\" ignore :enable nil))
  map)";

#[test]
fn popup_menus_read_gnus_text_menus_with_gnus_contract() {
    let root = std::env::temp_dir().join(format!("romacs-popup-menu-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(root.join("config")).unwrap();
    let report = std::thread::Builder::new().stack_size(64 << 20).spawn(move || {
        let mut vm = Vm::new();
        vm.set_frame_geometry(1280, 800, 20, 10).unwrap();
        vm.set_display_host(Some(Box::new(fonts::Display::new().unwrap())));
        vm.eval_source(&format!("(setq process-environment (append (list \"HOME={0}\" \"XDG_CONFIG_HOME={0}/config\") process-environment))",
                                root.display())).unwrap();
        startup::initialize_loadup(&mut vm, startup::Launch::Loadup, None).unwrap();
        // A plain buffer, whose position `popup-menu' takes for F10.
        printed(&mut vm, &format!("(progn (switch-to-buffer (get-buffer-create \"plain\")) (insert \"plain text\")
            (defvar menu-probe-map {MENU}) (defvar menu-probe-result nil) (defvar menu-probe-ran nil)
            t)"));
        // The GUI lays the frame out, so `popup-menu' measures where the
        // menu goes as it would there.
        let engine = std::rc::Rc::new(std::cell::RefCell::new(romacs_layout::LayoutEngine::new().unwrap()));
        vm.set_layout_host(Some(Box::new(measure::Host { engine: engine.clone() })));
        let paint = |vm: &mut Vm| {
            let snapshot = vm.frame_snapshot(1280, 800).unwrap();
            let (_, ends) = engine.borrow_mut().layout_frame(&snapshot, 1, None).unwrap();
            vm.display_apply_layout(&ends).unwrap();
        };
        paint(&mut vm);
        let mut report = vec![printed(&mut vm, "(list (framep (selected-frame)) (key-binding [f10]))")];
        let probe = |vm: &mut Vm, form: &str, script: &[i64]| {
            // The menu reads inside a command, as a mouse or key binding runs it.
            let source = format!("(global-set-key [f5] (lambda () (interactive)
                (setq menu-probe-result (condition-case nil {form} (quit 'quit)))))");
            printed(vm, &source);
            printed(vm, "(setq menu-probe-result 'unset menu-probe-ran nil)");
            let signal = command(vm, CommandEvent::Named("f5".into()), script);
            format!("{signal:?} {}", printed(vm, "(list menu-probe-result menu-probe-ran)"))
        };
        let (a, s, b, t, n, quit) = ('a' as i64, 's' as i64, 'b' as i64, 't' as i64, 'n' as i64, 7);
        report.push(probe(&mut vm, "(x-popup-menu t menu-probe-map)", &[a]));
        report.push(probe(&mut vm, "(x-popup-menu t menu-probe-map)", &[s, b]));
        // Back up from the submenu with `^', then the top level's Alpha.
        report.push(probe(&mut vm, "(x-popup-menu t menu-probe-map)", &[s, '^' as i64, a]));
        // The hidden item is not offered and the disabled one does nothing:
        // the items tmm lists, each with whether it does nothing.
        report.push(printed(&mut vm, "(progn (require 'tmm) (dlet ((tmm-km-list nil) (tmm-table-undef nil))
            (map-keymap (lambda (k v) (tmm-get-keymap (cons k v))) (romacs-menu--keymap menu-probe-map nil))
            (mapcar (lambda (e) (list (substring-no-properties (car e)) (eq (cddr e) 'ignore))) tmm-km-list)))"));
        report.push(probe(&mut vm, "(x-popup-menu t '(\"Title\" (\"Pane\" (\"One\" . 1) (\"Two\" . 2))))", &[t]));
        report.push(probe(&mut vm, "(x-popup-menu t menu-probe-map)", &[quit]));
        report.push(probe(&mut vm, "(x-popup-menu (list 'mouse-3 (posn-at-point)) menu-probe-map)", &[quit]));
        report.push(probe(&mut vm, "(x-popup-menu nil menu-probe-map)", &[]));
        report.push(probe(&mut vm, "(popup-menu menu-probe-map (posn-at-point))", &[s, b]));
        report.push(probe(&mut vm, "(popup-menu menu-probe-map (posn-at-point))", &[s, '^' as i64, a]));
        report.push(probe(&mut vm, "(x-popup-dialog t '(\"Really?\" (\"Yes\" . yes) nil (\"No\" . no)))", &[n]));
        // F10 on the frame: the menu bar's File menu, New Window Below.
        let signal = command(&mut vm, CommandEvent::Named("f10".into()), &['f' as i64, 'n' as i64]);
        report.push(format!("{signal:?} {}", printed(&mut vm,
            "(seq-count (lambda (w) (equal (buffer-name (window-buffer w)) \"plain\")) (window-list))")));
        let _ = std::fs::remove_dir_all(&root);
        report
    }).unwrap().join().unwrap();
    assert_eq!(report, [
        "(pgtk menu-bar-open)",
        "None ((alpha) nil)",
        "None ((sub beta) nil)",
        "None ((alpha) nil)",
        r#"(("Sub" nil) ("Alpha" nil) ("Disabled" t))"#,
        "None (2 nil)",
        "None (quit nil)",
        "None (nil nil)",
        "None (nil nil)",
        "None (beta beta)",
        "None (alpha alpha)",
        "None (no nil)",
        "None 2",
    ], "{report:#?}");
}
