//! Romacs's interactive defaults (rlisp/romacs-defaults.el, DIVERGENCES
//! VM2-OOB-DEFAULTS) on the real --loadup chain with a window system: a
//! session turns them on before the user's init, as standard values, the
//! modes the first frame does not need before its first command (whose
//! prompt has the saved history) unless the init toggled them: typing
//! pairs delimiters, and C-x o o inserts o as in GNU (repeat-mode stays
//! off); `romacs-defaults-mode' off
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
#[path = "../src/grammars.rs"]
mod grammars;
#[path = "../src/loadup.rs"]
mod loadup;
#[path = "../src/native.rs"]
mod native;
#[path = "../src/presentation.rs"]
mod presentation;
#[path = "../src/startup.rs"]
mod startup;
use romacs_vm::{CommandEvent, HostEvent, HostFrame, HostInput, Vm};
use std::collections::VecDeque;

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

/// The keys a prompt reads, as the GUI delivers them.
struct Keys(VecDeque<HostEvent>);
impl HostInput for Keys {
    fn input_pending(&mut self) -> bool { false }
    fn redisplay(&mut self, _: HostFrame) -> Result<Vec<romacs_vm::WindowEnd>, String> { Ok(vec![]) }
    fn next_event(&mut self, _: Option<std::time::Duration>) -> Result<HostEvent, String> {
        self.0.pop_front().ok_or_else(|| "oob input exhausted".into())
    }
}

/// A --loadup session on a window system in a private HOME, whose saved
/// minibuffer history holds one command and recent files one file, up to
/// the user's init, started
/// with -Q when QUICK; and its HOME.
fn session(name: &str, quick: bool) -> (Vm, std::path::PathBuf) {
    let home = std::env::temp_dir().join(format!("romacs-oob-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&home);
    std::fs::create_dir_all(home.join("config/romacs")).unwrap();
    std::fs::write(home.join("config/romacs/history"), "(setq extended-command-history '(\"oob-history-command\"))\n").unwrap();
    std::fs::write(home.join("config/romacs/recentf.eld"), "(setq recentf-list '(\"/oob/kept.txt\"))\n").unwrap();
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
        // The init turns one of the modes the first frame does not need
        // off; none of them is on before the first command.
        vm.eval_source("(global-auto-revert-mode -1)").unwrap();
        // A file the command line visits before the first command.
        let visited = home.join("visited.txt");
        std::fs::write(&visited, "visited\n").unwrap();
        vm.eval_source(&format!("(find-file-noselect {})", quoted(&visited.display().to_string()))).unwrap();
        report.push(printed(&mut vm, "(list savehist-mode recentf-mode repeat-mode global-auto-revert-mode which-key-mode
            (mapcar #'car (reverse romacs-defaults--pending)))"));
        // The first command, M-x, already reads the saved history: M-p RET
        // runs the command it holds.
        vm.eval_source("(defun oob-history-command () (interactive) (setq oob-history-ran t))").unwrap();
        vm.set_host_input(Some(Box::new(Keys(VecDeque::from([
            HostEvent::Key(CommandEvent::Character((1 << 27) | 'p' as i64)),
            HostEvent::Key(CommandEvent::Character(10)),
        ])))));
        let (mut commands, mut presentation) = (command::Commands::default(), presentation::Presentation::new());
        presentation.command(&mut vm, &mut commands, CommandEvent::Character((1 << 27) | 'x' as i64)).unwrap();
        report.push(printed(&mut vm, &format!("(list (bound-and-true-p oob-history-ran) savehist-mode romacs-defaults--pending
            (and (member (recentf-expand-file-name {}) recentf-list) t))", quoted(&visited.display().to_string()))));
        report.push(printed(&mut vm, SEEN));
        // Every setting is its option's standard value, and set, but the
        // mode the init turned off.
        report.push(printed(&mut vm, &format!("(delq nil (mapcar (lambda (s) (unless (and (nth 1 s) (nth 2 s)) (car s))) {SETTINGS}))")));
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
        "(nil t nil nil t (savehist-mode global-auto-revert-mode omarchy-mode))",
        "(t t nil t)",
        "(t t t t nil nil t t custom t t command-completion-default-include-p t prefix)",
        "(global-auto-revert-mode)",
        "(t t t t t t)",
        "(\"(a)\" 3)",
        "(nil \"o\" nil)",
        "(nil nil nil nil nil nil nil nil side-window nil mine nil nil nil)",
        "((kill-do-not-save-duplicates . mine))",
        "(\"(a\" 3)",
        "(nil \"o\" nil)",
    ]);
}

/// An init that turns the modes off keeps what they keep: the saved recent
/// files, and the `echo-keystrokes' which-key found.
#[test]
fn an_init_turning_the_modes_off_keeps_their_state() {
    let report = std::thread::Builder::new().stack_size(64 << 20).spawn(move || {
        let (mut vm, _) = session("init-off", false);
        vm.eval_source("(recentf-mode -1) (which-key-mode -1) (setopt savehist-mode nil)").unwrap();
        let mut report = vec![printed(&mut vm, "(list (with-temp-buffer (insert-file-contents recentf-save-file)
            (and (search-forward \"/oob/kept.txt\" nil t) t))
            (and (numberp echo-keystrokes) (equal echo-keystrokes which-key--echo-keystrokes-backup))
            recentf-mode which-key-mode)")];
        typed(&mut vm, "a");
        report.push(printed(&mut vm, "(list savehist-mode repeat-mode global-auto-revert-mode romacs-defaults--pending)"));
        report
    }).unwrap().join().unwrap();
    assert_eq!(report, ["(t t nil nil)", "(nil nil t nil)"]);
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
            // Turned on and off again before the first command: the
            // modes it would have turned on later stay off.
            printed(&mut vm, "(progn (romacs-defaults-mode 1) (romacs-defaults-mode -1) nil)"),
            pair(&mut vm, "quick"),
            printed(&mut vm, "(list savehist-mode recentf-mode repeat-mode global-auto-revert-mode which-key-mode romacs-defaults--pending)"),
        ]
    }).unwrap().join().unwrap();
    assert_eq!(report, [
        "(nil nil)",
        "(nil nil nil nil nil nil nil nil side-window nil nil nil nil nil)",
        "nil",
        "nil",
        "t",
        "nil",
        "(\"(a\" 3)",
        "(nil nil nil nil nil nil)",
    ]);
}

/// Programming buffers number their lines relative to point, the current
/// line by its own number; text, Org and special buffers do not.  The
/// switch takes only its own hook away: a user's `prog-mode-hook' entry
/// stays, the numbers it turned on go, and the option is GNU's again.
#[test]
fn programming_buffers_number_their_lines_relatively() {
    let report = std::thread::Builder::new().stack_size(64 << 20).spawn(move || {
        let (mut vm, _home) = session("numbers", false);
        let numbers = "(mapcar (lambda (mode)
            (with-current-buffer (get-buffer-create (format \"n-%s\" mode)) (funcall mode)
              (list mode display-line-numbers)))
            '(emacs-lisp-mode js-mode text-mode org-mode special-mode))";
        let mut report = vec![printed(&mut vm, "(list display-line-numbers-type display-line-numbers-current-absolute
            (and (memq 'romacs-defaults--line-numbers prog-mode-hook) t))")];
        report.push(printed(&mut vm, numbers));
        vm.eval_source("(add-hook 'prog-mode-hook #'oob-user-hook) (defun oob-user-hook () (setq-local oob-user-ran t))
            (romacs-defaults-mode -1)").unwrap();
        report.push(printed(&mut vm, "(list display-line-numbers-type (memq 'romacs-defaults--line-numbers prog-mode-hook)
            (and (memq 'oob-user-hook prog-mode-hook) t) (with-current-buffer \"n-emacs-lisp-mode\" display-line-numbers))"));
        report.push(printed(&mut vm, "(with-current-buffer (get-buffer-create \"n-after\") (emacs-lisp-mode)
            (list display-line-numbers (bound-and-true-p oob-user-ran)))"));
        report
    }).unwrap().join().unwrap();
    assert_eq!(report, [
        "(relative t t)",
        "((emacs-lisp-mode relative) (js-mode relative) (text-mode nil) (org-mode nil) (special-mode nil))",
        "(t nil t nil)",
        "(nil t)",
    ]);
}

/// `use-package' forms show in a Lisp buffer's imenu under "Packages"
/// (`use-package-enable-imenu-support'); the switch takes the entry away.
#[test]
fn use_package_forms_are_in_imenu() {
    let report = std::thread::Builder::new().stack_size(64 << 20).spawn(move || {
        let (mut vm, _home) = session("imenu", false);
        let packages = "(progn (require 'lisp-mode)
            (list use-package-enable-imenu-support
                  (and (assoc \"Packages\" lisp-imenu-generic-expression) t)))";
        let mut report = vec![printed(&mut vm, packages)];
        vm.eval_source("(romacs-defaults-mode -1)").unwrap();
        report.push(printed(&mut vm, packages));
        report
    }).unwrap().join().unwrap();
    assert_eq!(report, ["(t t)", "(nil nil)"]);
}

/// Files of the languages whose grammars Romacs ships open in their
/// tree-sitter modes (`treesit-enabled-modes'); the switch gives the regex
/// modes back, except where GNU's own `*-ts-mode-maybe' picks the ts mode
/// whenever its grammar is available (Rust).
#[test]
fn files_of_bundled_languages_open_in_their_ts_modes() {
    let report = std::thread::Builder::new().stack_size(64 << 20).spawn(move || {
        grammars::register();
        let (mut vm, home) = session("treesit", false);
        let files: Vec<String> = ["x.rs", "x.py", "x.c", "x.json"].iter().map(|name| {
            let path = home.join(name);
            std::fs::write(&path, "\n").unwrap();
            quoted(&path.display().to_string())
        }).collect();
        let visit = format!("(mapcar (lambda (file) (with-current-buffer (find-file-noselect file)
              (prog1 (list (file-name-extension file) major-mode (mapcar #'treesit-parser-language (treesit-parser-list)))
                (kill-buffer))))
            (list {}))", files.join(" "));
        let mut report = vec![printed(&mut vm, &visit)];
        vm.eval_source("(romacs-defaults-mode -1)").unwrap();
        report.push(printed(&mut vm, "(list treesit-enabled-modes (rassq 'python-ts-mode major-mode-remap-alist))"));
        report.push(printed(&mut vm, &visit));
        report
    }).unwrap().join().unwrap();
    assert_eq!(report, [
        r#"(("rs" rust-ts-mode (rust)) ("py" python-ts-mode (python)) ("c" c-ts-mode (c)) ("json" json-ts-mode (json)))"#,
        "(nil nil)",
        r#"(("rs" rust-ts-mode (rust)) ("py" python-mode nil) ("c" c-mode nil) ("json" js-json-mode nil))"#,
    ]);
}
