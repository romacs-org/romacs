//! Romacs's interactive defaults (rlisp/romacs-defaults.el, DIVERGENCES
//! VM2-OOB-DEFAULTS) on the real --loadup chain with a window system: a
//! session turns them on before the user's init, as standard values:
//! typing pairs delimiters and C-x o o repeats; `romacs-defaults-mode' off
//! gives every option GNU's standard value back but one the init set
//! itself; -Q never turns them on.
#[path = "../src/command.rs"]
mod command;
#[path = "../src/echo.rs"]
mod echo;
#[path = "../src/eval_last_sexp.rs"]
mod eval_last_sexp;
#[path = "../src/fonts.rs"]
mod fonts;
#[path = "../src/loadup.rs"]
mod loadup;
#[path = "../src/native.rs"]
mod native;
#[path = "../src/presentation.rs"]
mod presentation;
#[path = "../src/startup.rs"]
mod startup;
use romacs_vm::{CommandEvent, Vm};

fn snapshot(vm: &mut Vm, _: bool, width: u32, height: u32) -> Result<romacs_layout_input::FrameSnapshot, String> {
    vm.frame_snapshot(width, height)
}

fn printed(vm: &mut Vm, source: &str) -> String {
    vm.eval_source(source).and_then(|v| v.display()).unwrap_or_else(|e| format!("VM-ERROR {e}"))
}

fn quoted(text: &str) -> String {
    serde_json::to_string(text).unwrap()
}

/// Type TEXT as the GUI's outer loop runs each key's command.
fn typed(vm: &mut Vm, text: &str) {
    let (mut commands, mut presentation) = (command::Commands::default(), presentation::Presentation::new());
    for c in text.chars() {
        presentation.command(vm, &mut commands, CommandEvent::Character(c as i64)).unwrap();
    }
}

/// A --loadup session on a window system in a private HOME, up to the
/// user's init, started with -Q when QUICK; and its HOME.
fn session(name: &str, quick: bool) -> (Vm, std::path::PathBuf) {
    let home = std::env::temp_dir().join(format!("romacs-oob-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&home);
    std::fs::create_dir_all(home.join("config/romacs")).unwrap();
    let mut vm = Vm::new();
    vm.set_frame_geometry(1280, 800, 20, 10).unwrap();
    vm.set_display_host(Some(Box::new(fonts::Display::new().unwrap())));
    vm.eval_source(&format!("(setq process-environment (append (list {} {}) process-environment))",
        quoted(&format!("HOME={}", home.display())),
        quoted(&format!("XDG_CONFIG_HOME={}", home.join("config").display())))).unwrap();
    startup::initialize_loadup(&mut vm, startup::Launch::Loadup, None).unwrap();
    startup::before_init(&mut vm, startup::Launch::Loadup, quick).unwrap();
    (vm, home)
}

/// Each setting's option: whether it has the setting's value, and whether
/// that is its standard value.
const SETTINGS: &str = "(mapcar (lambda (setting)
    (let ((value (eval (cadr setting) t)))
      (list (car setting) (equal (default-value (car setting)) value)
            (equal (eval (car (get (car setting) 'standard-value)) t) value))))
  romacs-defaults-settings)";

/// The options that differ from their standard value, as the value.
const NOT_STANDARD: &str = "(delq nil (mapcar (lambda (setting)
    (unless (equal (default-value (car setting)) (eval (car (get (car setting) 'standard-value)) t))
      (cons (car setting) (default-value (car setting)))))
  romacs-defaults-settings))";

/// The modes and options a user sees, by name.
const SEEN: &str = "(list electric-pair-mode savehist-mode save-place-mode recentf-mode repeat-mode
  global-auto-revert-mode column-number-mode which-key-mode which-key-popup-type
  save-interprogram-paste-before-kill kill-do-not-save-duplicates read-extended-command-predicate
  imenu-auto-rescan imenu-flatten)";

fn pair(vm: &mut Vm, name: &str) -> String {
    vm.eval_source(&format!("(switch-to-buffer (get-buffer-create {}))", quoted(name))).unwrap();
    typed(vm, "(a");
    printed(vm, "(list (buffer-string) (point))")
}

/// C-x o o in two windows: whether the first window is selected again,
/// what the buffer holds, and any post-command-hook error logged.
fn repeat(vm: &mut Vm, name: &str) -> String {
    vm.eval_source(&format!("(progn (delete-other-windows) (switch-to-buffer (get-buffer-create {}))
        (split-window) (setq oob-first (selected-window)))", quoted(name))).unwrap();
    typed(vm, "\x18oo");
    printed(vm, "(list (eq (selected-window) oob-first) (buffer-string)
        (with-current-buffer \"*Messages*\" (save-excursion (goto-char (point-min))
          (and (search-forward \"Error in post-command-hook\" nil t) t))))")
}

#[test]
fn an_interactive_session_turns_the_defaults_on_and_the_switch_turns_them_off() {
    let report = std::thread::Builder::new().stack_size(64 << 20).spawn(move || {
        let (mut vm, home) = session("on", false);
        let config = format!("{}/", home.join("config/romacs").display());
        let mut report = vec![];
        report.push(printed(&mut vm, "(list window-system romacs-defaults-mode (eval (car (get 'romacs-defaults-mode 'standard-value)) t))"));
        report.push(printed(&mut vm, SEEN));
        // Every setting is set, and is its option's standard value.
        report.push(printed(&mut vm, &format!("(seq-every-p (lambda (s) (and (nth 1 s) (nth 2 s))) {SETTINGS})")));
        // The per-user files and directories are this session's.
        report.push(printed(&mut vm, &format!("(let ((dir {}))
            (list (file-in-directory-p savehist-file dir) (file-in-directory-p recentf-save-file dir)
                  (file-in-directory-p save-place-file dir) (equal backup-directory-alist (list (cons \".\" (concat dir \"backups/\"))))
                  (equal (car (last auto-save-file-name-transforms)) (list \".*\" (concat dir \"auto-save/\") t))
                  (file-directory-p (concat dir \"auto-save/\"))))", quoted(&config))));
        report.push(pair(&mut vm, "on"));
        report.push(repeat(&mut vm, "repeat-on"));
        // The init changes one option, then turns the defaults off: that
        // option keeps its value, every other has GNU's standard value.
        vm.eval_source("(setq kill-do-not-save-duplicates 'mine) (romacs-defaults-mode -1)").unwrap();
        report.push(printed(&mut vm, SEEN));
        report.push(printed(&mut vm, NOT_STANDARD));
        report.push(pair(&mut vm, "off"));
        report.push(repeat(&mut vm, "repeat-off"));
        report
    }).unwrap().join().unwrap();
    assert_eq!(report, [
        "(pgtk t t)",
        "(t t t t t t t t custom t t command-completion-default-include-p t prefix)",
        "t",
        "(t t t t t t)",
        "(\"(a)\" 3)",
        "(t \"\" nil)",
        "(nil nil nil nil nil nil nil nil side-window nil mine nil nil nil)",
        "((kill-do-not-save-duplicates . mine))",
        "(\"(a\" 3)",
        "(nil \"o\" nil)",
    ]);
}

#[test]
fn a_quick_session_keeps_gnus_values() {
    let report = std::thread::Builder::new().stack_size(64 << 20).spawn(move || {
        let (mut vm, home) = session("quick", true);
        vec![
            printed(&mut vm, "(list init-file-user romacs-defaults-mode)"),
            printed(&mut vm, SEEN),
            printed(&mut vm, NOT_STANDARD),
            printed(&mut vm, &format!("(file-exists-p {})", quoted(&home.join("config/romacs/auto-save").display().to_string()))),
            // The libraries' per-user files are this session's, -Q or not.
            printed(&mut vm, &format!("(seq-every-p (lambda (file) (file-in-directory-p file {}))
                (list savehist-file save-place-file recentf-save-file))", quoted(&home.join("config/romacs").display().to_string()))),
            pair(&mut vm, "quick"),
        ]
    }).unwrap().join().unwrap();
    assert_eq!(report, [
        "(nil nil)",
        "(nil nil nil nil nil nil nil nil side-window nil nil nil nil nil)",
        "nil",
        "nil",
        "t",
        "(\"(a\" 3)",
    ]);
}
