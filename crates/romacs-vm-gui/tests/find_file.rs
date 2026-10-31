#[path = "../src/command.rs"]
mod command;
#[path = "../src/echo.rs"]
mod echo;
#[path = "../src/eval_last_sexp.rs"]
mod eval_last_sexp;
#[path = "../src/loadup.rs"]
mod loadup;
#[path = "../src/native.rs"]
mod native;
#[path = "../src/presentation.rs"]
mod presentation;
#[path = "../src/startup.rs"]
mod startup;
use romacs_vm::{CommandEvent, HostEvent, HostFrame, HostInput, Vm};
use std::{cell::RefCell, collections::VecDeque, path::PathBuf, rc::Rc};

fn snapshot(
    vm: &mut Vm,
    _: bool,
    width: u32,
    height: u32,
) -> Result<romacs_layout_input::FrameSnapshot, String> {
    vm.frame_snapshot(width, height)
}
fn printed(vm: &mut Vm, source: &str) -> String {
    match vm.eval_source(source) {
        Ok(value) => value.display().unwrap(),
        Err(error) => format!("VM-ERROR {error}"),
    }
}
fn quoted(value: &str) -> String {
    serde_json::to_string(value).unwrap()
}
struct Fixture {
    root: PathBuf,
}
impl Fixture {
    fn new(name: &str) -> Self {
        Self::in_directory(std::env::temp_dir(), name)
    }
    fn in_directory(directory: PathBuf, name: &str) -> Self {
        let root = directory.join(format!("romacs-find-file-{}-{name}", std::process::id()));
        std::fs::create_dir_all(&root).unwrap();
        Self { root }
    }
    fn vm(&self) -> Vm {
        let mut vm = Vm::new();
        vm.set_frame_geometry(1000, 700, 20, 10).unwrap();
        startup::initialize_files(&mut vm, &[]).unwrap();
        vm.eval_source(&format!(
            "(setq default-directory {})",
            quoted(&format!("{}/", self.root.display()))
        ))
        .unwrap();
        vm
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.root);
    }
}
struct Keys {
    keys: VecDeque<HostEvent>,
    seen: Rc<RefCell<Vec<String>>>,
}
impl HostInput for Keys {
    fn input_pending(&mut self) -> bool {
        false
    }
    fn redisplay(&mut self, frame: HostFrame) -> Result<Vec<romacs_vm::WindowEnd>, String> {
        self.seen.borrow_mut().push(
            frame
                .frame
                .windows
                .last()
                .unwrap()
                .snapshot
                .text
                .utf8
                .clone(),
        );
        Ok(vec![])
    }
    fn next_event(&mut self, _: Option<std::time::Duration>) -> Result<HostEvent, String> {
        self.keys
            .pop_front()
            .ok_or_else(|| "find-file host script exhausted".into())
    }
}
fn attach(vm: &mut Vm, text: &str) -> Rc<RefCell<Vec<String>>> {
    let seen = Rc::new(RefCell::new(vec![]));
    vm.set_host_input(Some(Box::new(Keys {
        keys: text
            .chars()
            .map(|c| HostEvent::Key(CommandEvent::Character(c as i64)))
            .collect(),
        seen: seen.clone(),
    })));
    seen
}

#[test]
fn find_file_public_api_and_relative_unicode_visit() {
    let fixture = Fixture::new("api");
    std::fs::write(fixture.root.join("notes λ.txt"), "hello λ\n").unwrap();
    let mut vm = fixture.vm();
    assert_eq!(
        printed(
            &mut vm,
            "(list (fboundp 'find-file) (fboundp 'find-file-noselect) (key-binding [24 6]))"
        ),
        "(t t find-file)",
        "FIND_FILE_PUBLIC_CALLABLE_AND_BINDING"
    );
    assert_eq!(
        printed(
            &mut vm,
            "(progn (find-file \"./notes λ.txt\") (buffer-string))"
        ),
        "\"hello λ\\n\"",
        "FIND_FILE_RELATIVE_UNICODE_BYTES"
    );
    assert_eq!(
        printed(&mut vm, "buffer-file-name"),
        quoted(fixture.root.join("notes λ.txt").to_str().unwrap())
    );
    assert_eq!(
        printed(&mut vm, "default-directory"),
        quoted(&format!("{}/", fixture.root.display()))
    );
    assert_eq!(
        printed(
            &mut vm,
            "(condition-case e (find-file 42) (wrong-type-argument e))"
        ),
        "(wrong-type-argument stringp 42)",
        "FIND_FILE_TYPED_FILENAME"
    );
}

#[test]
fn find_file_new_file_noselect_and_dirty_identity() {
    let fixture = Fixture::new("identity");
    let mut vm = fixture.vm();
    assert_eq!(
        printed(
            &mut vm,
            "(let ((old (current-buffer)) (window (window-buffer (selected-window)))) (setq fresh (find-file-noselect \"new file λ.txt\")) (list (eq old (current-buffer)) (eq window (window-buffer (selected-window)))))"
        ),
        "(t t)",
        "FIND_FILE_NOSELECT_PRESERVES_SELECTION"
    );
    assert!(
        !fixture.root.join("new file λ.txt").exists(),
        "FIND_FILE_DELAYED_CREATION"
    );
    assert_eq!(
        printed(
            &mut vm,
            "(progn (find-file \"new file λ.txt\") (insert \"unsaved λ\") (setq saved-point (point)) (find-file \"./new file λ.txt\") (list (eq fresh (current-buffer)) (buffer-string) (= saved-point (point)) (buffer-modified-p)))"
        ),
        "(t \"unsaved λ\" t t)",
        "FIND_FILE_REVISIT_PRESERVES_DIRTY_IDENTITY_AND_POINT"
    );
    assert!(!fixture.root.join("new file λ.txt").exists());
    assert!(!printed(&mut vm, "(romacs-gui-save-buffer)").starts_with("VM-ERROR"));
    assert_eq!(
        std::fs::read_to_string(fixture.root.join("new file λ.txt")).unwrap(),
        "unsaved λ"
    );
}

#[test]
fn find_file_interactive_completion_and_abort() {
    let fixture = Fixture::new("interactive");
    std::fs::write(fixture.root.join("completion λ.txt"), "visited\n").unwrap();
    let mut vm = fixture.vm();
    let seen = attach(&mut vm, "comp\t\r");
    let mut commands = command::Commands::default();
    let mut presentation = presentation::Presentation::new();
    let mut outcome = None;
    for code in [24, 6] {
        outcome = presentation
            .command(&mut vm, &mut commands, CommandEvent::Character(code))
            .unwrap();
    }
    assert!(
        outcome.as_ref().is_some_and(|o| o.signal.is_none()),
        "FIND_FILE_REAL_CXCF_DISPATCH: {outcome:?}"
    );
    assert_eq!(
        printed(&mut vm, "(buffer-string)"),
        "\"visited\\n\"",
        "FIND_FILE_TAB_VISITS_COMPLETE_UNICODE_PATH"
    );
    assert!(
        seen.borrow()
            .iter()
            .any(|s| s == &format!("Find file: {}/completion λ.txt", fixture.root.display())),
        "FIND_FILE_RENDERED_PROMPT_AND_COMPLETION: {:?}",
        seen.borrow()
    );
    vm.eval_source("(insert \"dirty\") (setq before-buffer (current-buffer) before-point (point) before-window (selected-window))").unwrap();
    attach(&mut vm, "absent\u{7}");
    assert_eq!(
        printed(
            &mut vm,
            "(condition-case e (call-interactively 'find-file) (quit 'cancelled))"
        ),
        "cancelled",
        "FIND_FILE_CANCEL_SIGNALS_QUIT"
    );
    assert_eq!(
        printed(
            &mut vm,
            "(list (eq before-buffer (current-buffer)) (= before-point (point)) (eq before-window (selected-window)) (buffer-string) (buffer-modified-p))"
        ),
        "(t t t \"dirtyvisited\\n\" t)",
        "FIND_FILE_ABORT_PRESERVES_DOCUMENT_POINT_SELECTION"
    );
    assert!(!fixture.root.join("absent").exists());
    assert_eq!(
        std::fs::read_to_string(fixture.root.join("completion λ.txt")).unwrap(),
        "visited\n"
    );
}

#[test]
fn find_file_tilde_errors_and_hooks() {
    let home = std::env::var("HOME").unwrap();
    let fixture = Fixture::in_directory(PathBuf::from(&home), "errors");
    let mut vm = fixture.vm();
    let relative = fixture.root.strip_prefix(&home).unwrap();
    let filename = format!("~/{}/tilde.txt", relative.display());
    std::fs::write(fixture.root.join("tilde.txt"), "home").unwrap();
    assert_eq!(
        printed(
            &mut vm,
            &format!("(progn (find-file {}) (buffer-string))", quoted(&filename))
        ),
        "\"home\"",
        "FIND_FILE_TILDE"
    );
    vm.eval_source("(setq good (current-buffer) good-point (point))")
        .unwrap();
    // A file that is not UTF-8 is visited, its byte a raw-byte character
    // (VM2-RAWTEXT1), as GNU visits it.
    std::fs::write(fixture.root.join("bad.txt"), [0xff]).unwrap();
    assert_eq!(
        printed(
            &mut vm,
            "(progn (find-file \"bad.txt\") (list (char-after 1) (buffer-size) (eq (romacs-find-buffer-visiting \"bad.txt\") (current-buffer))))"
        ),
        "(4194303 1 t)",
        "FIND_FILE_RAW_BYTE"
    );
    assert_eq!(
        printed(
            &mut vm,
            "(progn (kill-buffer) (switch-to-buffer good) (list (= good-point (point)) (romacs-find-buffer-visiting \"bad.txt\")))"
        ),
        "(t nil)",
        "FIND_FILE_RAW_BYTE_KILLED"
    );
    for path in ["/ssh:host:/file", "*.txt", "/:special"] {
        assert_eq!(
            printed(
                &mut vm,
                &format!(
                    "(condition-case e (find-file {}) (error 'refused))",
                    quoted(path)
                )
            ),
            "refused",
            "FIND_FILE_UNSUPPORTED_EXPLICIT: {path}"
        );
    }
    assert_eq!(
        printed(
            &mut vm,
            "(progn (setq hook-count 0 find-file-hook (list (lambda () (setq hook-count (1+ hook-count))))) (find-file \"hook.txt\") (find-file \"hook.txt\") hook-count)"
        ),
        "1",
        "FIND_FILE_HOOK_ONCE_FOR_NEW_VISIT"
    );
}

#[test]
fn find_file_history_nested_completion_and_reader_policy() {
    let fixture = Fixture::new("history");
    std::fs::create_dir(fixture.root.join("sub dir")).unwrap();
    std::fs::write(fixture.root.join("sub dir/notes λ.txt"), "nested").unwrap();
    let mut vm = fixture.vm();
    let seen = attach(&mut vm, "sub\tno\t\r");
    assert_eq!(
        printed(
            &mut vm,
            "(progn (call-interactively 'find-file) (buffer-string))"
        ),
        "\"nested\"",
        "FIND_FILE_NESTED_DIRECTORY_COMPLETION"
    );
    assert!(seen.borrow().iter().any(|s| s.ends_with("/sub dir/")));
    assert_eq!(
        printed(&mut vm, "(length file-name-history)"),
        "1",
        "FIND_FILE_HISTORY_RECORDED"
    );
    let history = printed(&mut vm, "file-name-history");
    attach(&mut vm, "\u{1b}p\r");
    assert_eq!(
        printed(
            &mut vm,
            "(progn (call-interactively 'find-file) (buffer-string))"
        ),
        "\"nested\"",
        "FIND_FILE_HISTORY_RECALLED"
    );
    assert_eq!(
        printed(&mut vm, "file-name-history"),
        history,
        "FIND_FILE_HISTORY_DEDUPLICATED"
    );
    attach(&mut vm, "zz\u{7}");
    assert_eq!(
        printed(
            &mut vm,
            "(condition-case e (call-interactively 'find-file) (quit 'cancelled))"
        ),
        "cancelled"
    );
    assert_eq!(
        printed(&mut vm, "file-name-history"),
        history,
        "FIND_FILE_ABORT_DOES_NOT_ADD_HISTORY"
    );
    attach(&mut vm, "\r");
    assert_eq!(
        printed(
            &mut vm,
            "(read-file-name \"File: \" nil nil t \"notes λ.txt\")"
        ),
        quoted(fixture.root.join("sub dir/notes λ.txt").to_str().unwrap()),
        "FIND_FILE_READER_MUSTMATCH"
    );
    assert_eq!(
        printed(
            &mut vm,
            "(let ((read-file-name-function (lambda (&rest args) \"custom\"))) (read-file-name \"File: \"))"
        ),
        "\"custom\"",
        "FIND_FILE_READER_POLICY_EXTENSION"
    );
}

#[test]
fn find_file_mode_lifecycle_and_nonlocal_cleanup() {
    let fixture = Fixture::new("lifecycle");
    let mut vm = fixture.vm();
    assert_eq!(
        printed(
            &mut vm,
            "(progn (defun custom-mode () (setq major-mode 'custom-mode) (setq mode-calls (1+ mode-calls))) (setq mode-calls 0 auto-mode-alist '((\"\\\\.custom\\\\'\" . custom-mode))) (find-file \"first.custom\") (find-file \"first.custom\") (list major-mode mode-calls))"
        ),
        "(custom-mode 1)",
        "FIND_FILE_FILENAME_MODE_ONCE"
    );
    vm.eval_source("(setq good (current-buffer) good-point (point))")
        .unwrap();
    for (handler, expression, expected) in [
        (
            "(lambda () (signal 'error '(\"hook error\")))",
            "(condition-case e (find-file \"error.txt\") (error 'refused))",
            "refused",
        ),
        (
            "(lambda () (signal 'quit nil))",
            "(condition-case e (find-file \"error.txt\") (quit 'cancelled))",
            "cancelled",
        ),
        (
            "(lambda () (throw 'escape 'escaped))",
            "(catch 'escape (find-file \"error.txt\"))",
            "escaped",
        ),
    ] {
        vm.eval_source(&format!("(setq find-file-hook (list {handler}))"))
            .unwrap();
        assert_eq!(
            printed(&mut vm, expression),
            expected,
            "FIND_FILE_HOOK_EXIT"
        );
        assert_eq!(
            printed(
                &mut vm,
                "(list (eq good (current-buffer)) (= good-point (point)) (romacs-find-buffer-visiting \"error.txt\"))"
            ),
            "(t t nil)",
            "FIND_FILE_NONLOCAL_EXIT_REMOVES_FALSE_VISIT"
        );
    }
    vm.eval_source("(setq find-file-hook nil)").unwrap();
    assert_eq!(
        printed(
            &mut vm,
            "(condition-case e (find-file \"absent/child.txt\") (file-missing 'missing-parent))"
        ),
        "missing-parent",
        "FIND_FILE_MISSING_PARENT_REFUSED"
    );
    assert!(!fixture.root.join("absent").exists());
    vm.eval_source("(setq file-name-handler-alist '((\"handled\" . forbidden-handler)))")
        .unwrap();
    assert_eq!(
        printed(
            &mut vm,
            "(condition-case e (find-file \"handled.txt\") (file-error 'handler-refused))"
        ),
        "handler-refused",
        "FIND_FILE_HANDLER_REFUSED"
    );
    assert!(!fixture.root.join("handled.txt").exists());
}

#[test]
fn switch_buffer_native_entry_completion_history_cancel_and_programmatic() {
    let fixture = Fixture::new("buffers");
    std::fs::write(fixture.root.join("alpha λ.txt"), "Alpha\n").unwrap();
    std::fs::write(fixture.root.join("beta.txt"), "Beta\n").unwrap();
    let mut vm = fixture.vm();
    assert_eq!(
        printed(&mut vm, "(key-binding [24 98])"),
        "switch-to-buffer",
        "BUFFER_SWITCH_PUBLIC_BINDING"
    );
    vm.eval_source("(find-file \"alpha λ.txt\") (goto-char 3) (insert \"dirty\") (setq alpha (current-buffer) alpha-point (point)) (find-file \"beta.txt\") (goto-char 2) (setq beta (current-buffer) beta-point (point))").unwrap();
    let seen = attach(&mut vm, "alpha\t\r");
    let mut commands = command::Commands::default();
    let mut presentation = presentation::Presentation::new();
    let mut outcome = None;
    for code in [24, 98] {
        outcome = presentation
            .command(&mut vm, &mut commands, CommandEvent::Character(code))
            .unwrap();
    }
    assert!(
        outcome.as_ref().is_some_and(|o| o.signal.is_none()),
        "BUFFER_SWITCH_NATIVE_ENTRY: {outcome:?}"
    );
    assert_eq!(
        printed(
            &mut vm,
            "(list (eq alpha (current-buffer)) (eq alpha (window-buffer (selected-window))) (= alpha-point (point)) (buffer-string) (buffer-modified-p))"
        ),
        "(t t t \"Aldirtypha\\n\" t)",
        "BUFFER_SWITCH_PRESERVES_DIRTY_POINT_AND_IDENTITY"
    );
    assert!(
        seen.borrow().iter().any(|s| s.ends_with("alpha λ.txt")),
        "BUFFER_SWITCH_COMPLETED_NAME"
    );
    assert_eq!(
        printed(&mut vm, "(length buffer-name-history)"),
        "1",
        "BUFFER_SWITCH_HISTORY_RECORDED"
    );
    assert_eq!(
        printed(
            &mut vm,
            "(progn (switch-to-buffer beta) (list (eq beta (window-buffer (selected-window))) (= beta-point (point)) (buffer-modified-p)))"
        ),
        "(t t nil)",
        "BUFFER_SWITCH_PROGRAMMATIC_SHARED_DISPLAY_ENGINE"
    );
    attach(&mut vm, "alpha\u{7}");
    assert_eq!(
        printed(
            &mut vm,
            "(condition-case e (call-interactively 'switch-to-buffer) (quit 'cancelled))"
        ),
        "cancelled"
    );
    assert_eq!(
        printed(
            &mut vm,
            "(list (eq beta (current-buffer)) (= beta-point (point)) (length buffer-name-history))"
        ),
        "(t t 1)",
        "BUFFER_SWITCH_CANCEL_PRESERVES_STATE_AND_HISTORY"
    );
    attach(&mut vm, "\u{1b}p\r");
    assert_eq!(
        printed(
            &mut vm,
            "(progn (call-interactively 'switch-to-buffer) (eq alpha (current-buffer)))"
        ),
        "t",
        "BUFFER_SWITCH_HISTORY_RECALL"
    );
    assert_eq!(
        std::fs::read_to_string(fixture.root.join("alpha λ.txt")).unwrap(),
        "Alpha\n"
    );
}

#[test]
fn find_file_empty_new_buffer_is_created_only_by_explicit_save() {
    let fixture = Fixture::new("empty-save");
    let mut vm = fixture.vm();
    vm.eval_source("(find-file \"empty.txt\")").unwrap();
    assert!(!fixture.root.join("empty.txt").exists());
    vm.eval_source("(romacs-gui-save-buffer)").unwrap();
    assert!(
        fixture.root.join("empty.txt").is_file(),
        "FIND_FILE_EMPTY_NEW_EXPLICIT_SAVE_CREATES_FILE"
    );
    assert_eq!(std::fs::read(fixture.root.join("empty.txt")).unwrap(), b"");
}

#[test]
fn extended_command_binding_completion_and_nested_file_prompt() {
    let fixture = Fixture::new("extended-command");
    std::fs::write(fixture.root.join("nested λ.txt"), "Nested file\n").unwrap();
    let mut vm = fixture.vm();
    assert_eq!(
        printed(&mut vm, "(list (not (null (interactive-form 'execute-extended-command))) (key-binding [27 120]))"),
        "(t execute-extended-command)",
        "EXTENDED_COMMAND_PUBLIC_ENTRY"
    );
    vm.eval_source("(defun mx-live-command () (interactive) (insert \"live\"))").unwrap();
    let seen = attach(&mut vm, "mx-live-c\t\r");
    let mut commands = command::Commands::default();
    let mut presentation = presentation::Presentation::new();
    let outcome = presentation.command(&mut vm, &mut commands,
        CommandEvent::Character((1 << 27) | 'x' as i64)).unwrap().unwrap();
    assert!(outcome.signal.is_none(), "EXTENDED_COMMAND_NATIVE_DISPATCH: {outcome:?}");
    assert!(seen.borrow().iter().any(|s| s == "M-x mx-live-command"));
    assert_eq!(printed(&mut vm, "(buffer-substring-no-properties (- (point) 4) (point))"), "\"live\"");
    attach(&mut vm, "find-f\t\rnested λ.txt\r");
    assert_eq!(printed(&mut vm,
        "(progn (call-interactively 'execute-extended-command) (buffer-string))"),
        "\"Nested file\\n\"", "EXTENDED_COMMAND_NESTED_FIND_FILE");
}

#[test]
fn extended_command_cancel_history_rejection_and_prefix() {
    let fixture = Fixture::new("extended-command-policy");
    let mut vm = fixture.vm();
    vm.eval_source("(defun mx-prefix-command (arg) (interactive \"P\") (setq mx-prefix-seen arg)) (defun mx-not-interactive () (setq mx-forbidden t)) (setq mx-forbidden nil mx-prefix-seen nil) (goto-char 3) (setq mx-buffer (current-buffer) mx-point (point))").unwrap();
    assert_eq!(printed(&mut vm,
        "(progn (execute-extended-command '(4) \"mx-prefix-command\") mx-prefix-seen)"),
        "(4)", "EXTENDED_COMMAND_PREFIX_FORWARDED");
    attach(&mut vm, "mx-prefix-command\r");
    assert_eq!(printed(&mut vm,
        "(progn (call-interactively 'execute-extended-command) (car extended-command-history))"),
        "\"mx-prefix-command\"", "EXTENDED_COMMAND_HISTORY");
    attach(&mut vm, "\u{1b}p\r");
    assert_eq!(printed(&mut vm,
        "(progn (call-interactively 'execute-extended-command) (length extended-command-history))"),
        "1", "EXTENDED_COMMAND_HISTORY_RECALL");
    attach(&mut vm, "does-not-exist\u{7}");
    assert_eq!(printed(&mut vm,
        "(condition-case e (call-interactively 'execute-extended-command) (quit 'cancelled))"),
        "cancelled", "EXTENDED_COMMAND_CANCEL");
    assert_eq!(printed(&mut vm,
        "(list (eq mx-buffer (current-buffer)) (= mx-point (point)) (length extended-command-history))"),
        "(t t 1)", "EXTENDED_COMMAND_CANCEL_PRESERVES_STATE");
    for name in ["mx-not-interactive", "mx-no-such-command"] {
        assert_eq!(printed(&mut vm, &format!(
            "(condition-case e (execute-extended-command nil {}) (error 'refused))", quoted(name))),
            "refused", "EXTENDED_COMMAND_REJECTS_NONCOMMAND");
    }
    assert_eq!(printed(&mut vm, "mx-forbidden"), "nil");
}

/// An installed package's command is an interactive autoload: M-x offers
/// and accepts it by name without loading its file, and running it loads the
/// file. A non-interactive autoload stays refused, still unloaded.
#[test]
fn extended_command_runs_interactive_autoloads() {
    let fixture = Fixture::new("extended-command-autoload");
    let library = fixture.root.join("mx-package.el");
    std::fs::write(&library, "(setq mx-package-loads (1+ mx-package-loads))\n\
        (defun mx-autoloaded () (interactive) (setq mx-autoloaded-ran t))\n\
        (defun mx-auto-plain () (setq mx-forbidden t))\n").unwrap();
    let mut vm = fixture.vm();
    vm.eval_source(&format!("(setq mx-package-loads 0 mx-autoloaded-ran nil mx-forbidden nil)
        (autoload 'mx-autoloaded {0} nil t) (autoload 'mx-auto-plain {0})", quoted(&library.to_string_lossy()))).unwrap();
    assert_eq!(printed(&mut vm,
        "(list (and (member \"mx-autoloaded\" (romacs-command-names)) t) (member \"mx-auto-plain\" (romacs-command-names)) mx-package-loads)"),
        "(t nil 0)", "EXTENDED_COMMAND_OFFERS_AUTOLOAD_UNLOADED");
    assert_eq!(printed(&mut vm,
        "(condition-case e (execute-extended-command nil \"mx-auto-plain\") (error 'refused))"),
        "refused", "EXTENDED_COMMAND_REJECTS_NONINTERACTIVE_AUTOLOAD");
    attach(&mut vm, "mx-autol\t\r");
    assert_eq!(printed(&mut vm,
        "(progn (call-interactively 'execute-extended-command) (list mx-autoloaded-ran mx-package-loads mx-forbidden))"),
        "(t 1 nil)", "EXTENDED_COMMAND_LOADS_AUTOLOAD_ON_CALL");
    // GNU's M-x runs a named keyboard macro through command-execute.
    assert_eq!(printed(&mut vm,
        "(progn (fset 'mx-macro \"xy\") (erase-buffer) (execute-extended-command nil \"mx-macro\") (buffer-string))"),
        "\"xy\"", "EXTENDED_COMMAND_RUNS_NAMED_KEYBOARD_MACRO");
}

#[test]
fn extended_command_escape_entry_and_errors_keep_editor_usable() {
    let fixture = Fixture::new("extended-command-errors");
    let mut vm = fixture.vm();
    vm.eval_source("(defun mx-raises () (interactive) (error \"Command failed: 100%%\")) (defun mx-noncommand () (error \"must not run\")) (setq mx-before (buffer-string) mx-point (point))").unwrap();
    let mut commands = command::Commands::default();
    let mut presentation = presentation::Presentation::new();
    attach(&mut vm, "mx-raises\r");
    assert!(presentation.command(&mut vm, &mut commands, CommandEvent::Character(27)).unwrap().is_none());
    let failed = presentation.command(&mut vm, &mut commands, CommandEvent::Character(120)).unwrap().unwrap();
    assert_eq!(failed.signal.as_ref().unwrap().condition, "error", "EXTENDED_COMMAND_PRESERVES_ERROR");
    assert_eq!(echo::text(&mut vm).unwrap(), "Command failed: 100%", "EXTENDED_COMMAND_ERROR_MESSAGE");
    attach(&mut vm, "mx-noncommand\r");
    let rejected = presentation.command(&mut vm, &mut commands,
        CommandEvent::Character((1 << 27) | 'x' as i64)).unwrap().unwrap();
    assert_eq!(rejected.signal.as_ref().unwrap().condition, "user-error", "EXTENDED_COMMAND_REJECTS_TYPED_NONCOMMAND");
    attach(&mut vm, "find-file\u{7}");
    let cancelled = presentation.command(&mut vm, &mut commands,
        CommandEvent::Character((1 << 27) | 'x' as i64)).unwrap().unwrap();
    assert!(cancelled.signal.is_some(), "EXTENDED_COMMAND_QUIT_REACHES_COMMAND_LOOP");
    assert_eq!(printed(&mut vm, "(list (equal mx-before (buffer-string)) (= mx-point (point)))"), "(t t)");
    presentation.command(&mut vm, &mut commands, CommandEvent::Character('!' as i64)).unwrap();
    assert_eq!(printed(&mut vm, "(buffer-substring-no-properties (- (point) 1) (point))"), "\"!\"", "EXTENDED_COMMAND_ERROR_RECOVERY");
}

#[test]
fn extended_command_preserves_prompted_command_identity() {
    for spec in ["(list (read-string \"Argument: \"))", "(list (read-from-minibuffer \"Argument: \"))"] {
        let fixture = Fixture::new(if spec.contains("read-string") { "mx-identity-string" } else { "mx-identity-minibuf" });
        let mut vm = fixture.vm();
        vm.eval_source(&format!(
            "(defun mx-identity (arg) (interactive {spec}) (setq mx-identities (list this-command real-this-command last-command arg) this-command 'mx-repeat-sentinel)) (setq last-command 'prior-command real-last-command 'prior-real)"
        )).unwrap();
        attach(&mut vm, "mx-identity\ranswer\r");
        let mut commands = command::Commands::default();
        let mut presentation = presentation::Presentation::new();
        let outcome = presentation.command(&mut vm, &mut commands, CommandEvent::Character((1 << 27) | 120)).unwrap().unwrap();
        assert!(outcome.signal.is_none(), "MX_PROMPTED_IDENTITY_EXECUTION: {outcome:?}");
        assert_eq!(printed(&mut vm, "mx-identities"), "(mx-identity mx-identity prior-command \"answer\")", "MX_ARGUMENT_PREPARATION_RESTORES_IDENTITIES");
        assert_eq!(printed(&mut vm, "(list last-command real-last-command)"), "(mx-repeat-sentinel mx-identity)", "MX_COMMAND_LOOP_RECORDS_RESULTING_IDENTITIES");
    }
}

#[test]
fn extended_command_nested_raw_prompt_does_not_blanket_restore_identity() {
    let fixture = Fixture::new("mx-nested-identity");
    let mut vm = fixture.vm();
    vm.eval_source("(defun mx-inner-reader () (interactive) (let ((enable-recursive-minibuffers t)) (read-string \"Inner: \")) (setq mx-inner-identities (list this-command real-this-command last-command real-last-command))) (define-key minibuffer-local-map [1] 'mx-inner-reader) (defun mx-outer-reader (arg) (interactive (list (read-string \"Outer: \"))) (setq mx-outer-identities (list this-command real-this-command last-command))) (setq last-command 'prior-command)").unwrap();
    attach(&mut vm, "mx-outer-reader\r\u{1}inner\router\r");
    let mut commands = command::Commands::default();
    let mut presentation = presentation::Presentation::new();
    let outcome = presentation.command(&mut vm, &mut commands, CommandEvent::Character((1 << 27) | 120)).unwrap().unwrap();
    assert!(outcome.signal.is_none(), "MX_NESTED_IDENTITY_EXECUTION: {outcome:?}");
    assert_eq!(printed(&mut vm, "mx-inner-identities"), "(exit-minibuffer exit-minibuffer self-insert-command self-insert-command)", "MX_RAW_PROMPT_NOT_BLANKET_RESTORED");
    assert_eq!(printed(&mut vm, "mx-outer-identities"), "(mx-outer-reader mx-outer-reader prior-command)", "MX_OUTER_INTERACTIVE_BOUNDARY_RESTORES");
}

#[test]
fn dired_escapes_its_directory_header_without_changing_visit_paths() {
    let fixture = Fixture::new("dired-header");
    let directory = fixture.root.join("dir\n\t\\λ");
    std::fs::create_dir(&directory).unwrap();
    std::fs::write(directory.join("entry.txt"), "Header path sentinel").unwrap();
    let mut vm = fixture.vm();
    vm.eval_source(&format!("(dired {}) (goto-char (point-min))", quoted(directory.to_str().unwrap()))).unwrap();
    let prefix = format!("  {}/dir\\n\\t\\\\λ:", fixture.root.display());
    assert_eq!(printed(&mut vm, &format!("(buffer-substring-no-properties (point-min) (+ (point-min) {}))", prefix.chars().count())), quoted(&prefix), "DIRED_DIRECTORY_HEADER_IS_ESCAPED");
    assert_eq!(printed(&mut vm, "(progn (dired-goto-file \"entry.txt\") (dired-find-file) (buffer-string))"), "\"Header path sentinel\"", "DIRED_ESCAPED_DIRECTORY_EXACT_VISIT");
}

#[test]
fn dired_long_rows_use_gnu_metadata_columns_and_filename_faces() {
    let fixture = Fixture::new("dired-long");
    std::fs::write(fixture.root.join("ordinary.txt"), "sentinel\n").unwrap();
    std::fs::create_dir(fixture.root.join("child")).unwrap();
    std::os::unix::fs::symlink("ordinary.txt", fixture.root.join("link")).unwrap();
    let mut vm = fixture.vm();
    assert_eq!(printed(&mut vm, "(progn (dired default-directory) (goto-char (point-min)) (looking-at (concat \"  \" (substring dired-directory 0 -1) \": (\")))"), "t", "DIRED_GNU_HEADER");
    assert_eq!(printed(&mut vm, "(progn (dired-goto-file \"ordinary.txt\") (list (get-text-property (point) 'dired-filename) (looking-at \"ordinary.txt\") (save-excursion (beginning-of-line) (looking-at \"  -rw\"))))"), "(t t t)", "DIRED_GNU_LONG_ROW");
    assert_eq!(printed(&mut vm, "(progn (dired-goto-file \"child\") (list (get-text-property (point) 'face) (looking-at \"child$\")))"), "(dired-directory t)", "DIRED_GNU_DIRECTORY_FACE_NO_SUFFIX");
    assert_eq!(printed(&mut vm, "(progn (dired-goto-file \"link\") (list (get-text-property (point) 'face) (looking-at \"link -> ordinary.txt$\")))"), "(dired-symlink t)", "DIRED_GNU_SYMLINK_ROW");
    assert_eq!(printed(&mut vm, "(list (dired-goto-file \".\") (dired-goto-file \"..\"))"), "(t t)", "DIRED_GNU_DOT_ROWS");
}

#[test]
fn dired_reuses_directory_buffers_and_preserves_file_visits() {
    let fixture = Fixture::new("dired");
    let file = fixture.root.join("a λ.txt");
    std::fs::write(&file, "File sentinel\n").unwrap();
    std::fs::create_dir(fixture.root.join("child")).unwrap();
    let mut vm = fixture.vm();
    vm.eval_source("(find-file \"a λ.txt\") (goto-char 3) (insert \"dirty\") (setq dired-entry (current-buffer) dired-entry-point (point))").unwrap();
    assert_eq!(printed(&mut vm, "(list (fboundp 'dired) (key-binding [24 100]))"),
        "(t dired)", "DIRED_PUBLIC_ENTRY");
    assert_eq!(printed(&mut vm,
        "(progn (setq directory-view (dired-noselect default-directory)) (list (eq dired-entry (current-buffer)) (with-current-buffer directory-view (list major-mode buffer-read-only buffer-file-name (buffer-modified-p)))))"),
        "(t (dired-mode t nil nil))", "DIRED_NOSELECT_READONLY_NONFILE");
    assert_eq!(printed(&mut vm,
        "(progn (find-file default-directory) (list (eq directory-view (current-buffer)) (eq directory-view (dired-noselect default-directory))))"),
        "(t t)", "DIRED_FIND_FILE_DELEGATES_AND_REUSES");
    assert_eq!(printed(&mut vm, &format!("(list (equal dired-directory {0}) (equal default-directory {0}))",
        quoted(&format!("{}/", fixture.root.display())))), "(t t)", "DIRED_DIRECTORY_CONTEXT");
    assert_eq!(printed(&mut vm, "(list (dired-goto-file \"a λ.txt\") romacs-dired-rows)"),
        printed(&mut vm, "(list t romacs-dired-rows)"), "DIRED_LOCATES_EXACT_ROW");
    assert_eq!(printed(&mut vm,
        "(progn (dired-goto-file \"a λ.txt\") (dired-find-file) (list (eq dired-entry (current-buffer)) (= dired-entry-point (point)) (buffer-string) (buffer-modified-p)))"),
        "(t t \"Fidirtyle sentinel\\n\" t)", "DIRED_OPEN_PRESERVES_DIRTY_IDENTITY");
    vm.eval_source("(dired default-directory) (dired-goto-file \"child\") (dired-find-file) (dired-up-directory) (dired-quit-window)").unwrap();
    assert_eq!(printed(&mut vm, "(list (eq dired-entry (current-buffer)) (= dired-entry-point (point)))"),
        "(t t)", "DIRED_PARENT_AND_QUIT_RESTORE_ENTRY");
    assert_eq!(std::fs::read_to_string(file).unwrap(), "File sentinel\n");
}

#[test]
fn dired_exact_filename_mapping_refresh_and_stale_entry_refusal() {
    let fixture = Fixture::new("dired-rows");
    for name in ["line\nname.txt", "tab\tname.txt", "back\\name.txt", "λ space.txt"] {
        std::fs::write(fixture.root.join(name), name).unwrap();
    }
    let mut vm = fixture.vm();
    assert_eq!(printed(&mut vm, "(progn (dired default-directory) (setq exact-directory-view (current-buffer)) major-mode)"), "dired-mode", "DIRED_OPENS");
    let mut commands = command::Commands::default();
    let mut presentation = presentation::Presentation::new();
    for name in ["line\nname.txt", "tab\tname.txt", "back\\name.txt", "λ space.txt"] {
        let path = fixture.root.join(name);
        // The exact path, whatever its representation: an ASCII name read
        // from the directory is unibyte, as GNU's directory-files leaves it.
        assert_eq!(printed(&mut vm, &format!(
            "(progn (dired-goto-file {0}) (equal (dired-get-filename) {0}))", quoted(path.to_str().unwrap()))),
            "t", "DIRED_EXACT_PATH_NOT_DISPLAY_PARSE");
        let opened = presentation.command(&mut vm, &mut commands, CommandEvent::Character(13)).unwrap().unwrap();
        assert!(opened.signal.is_none(), "DIRED_ESCAPED_NATIVE_OPEN: {opened:?}");
        assert_eq!(printed(&mut vm, &format!("(equal buffer-file-name {})", quoted(path.to_str().unwrap()))), "t", "DIRED_ESCAPED_VISIT_IDENTITY");
        assert_eq!(printed(&mut vm, "(buffer-string)"), quoted(name), "DIRED_ESCAPED_VISIT_BYTES");
        vm.eval_source("(romacs-select-buffer exact-directory-view)").unwrap();
    }
    std::fs::write(fixture.root.join("new.txt"), "new").unwrap();
    vm.eval_source("(setq selected-path (dired-get-filename)) (dired-revert)").unwrap();
    assert_eq!(printed(&mut vm, "(list (equal selected-path (dired-get-filename)) (buffer-modified-p))"),
        "(t nil)", "DIRED_REFRESH_PRESERVES_SELECTION");
    std::fs::remove_file(fixture.root.join("λ space.txt")).unwrap();
    assert_eq!(printed(&mut vm, "(condition-case e (dired-find-file) (file-missing 'stale))"),
        "stale", "DIRED_STALE_ENTRY_MUST_NOT_CREATE_FILE_VISIT");
    assert!(!fixture.root.join("λ space.txt").exists());
    vm.eval_source("(dired-revert) (dired-goto-file \"new.txt\")").unwrap();
    // A name read from the directory is unibyte, as GNU's directory-files leaves it.
    assert_eq!(printed(&mut vm, "(equal (file-name-nondirectory (dired-get-filename)) \"new.txt\")"), "t");
}

#[test]
fn dired_read_only_rejects_text_mutations_and_allows_owned_refresh() {
    let fixture = Fixture::new("dired-readonly");
    let directory = fixture.root.join("isolated");
    std::fs::create_dir(&directory).unwrap();
    std::fs::write(directory.join("file.txt"), "unchanged").unwrap();
    let mut vm = fixture.vm();
    vm.eval_source(&format!("(setq default-directory {})", quoted(&format!("{}/", directory.display())))).unwrap();
    vm.eval_source("(dired default-directory) (setq dired-before (buffer-string) dired-point (point))").unwrap();
    for edit in ["(insert \"x\")", "(delete-region (point-min) (1+ (point-min)))", "(erase-buffer)"] {
        assert_eq!(printed(&mut vm, &format!("(condition-case e {edit} (buffer-read-only 'blocked))")),
            "blocked", "DIRED_READ_ONLY_MUTATION: {edit}");
        assert_eq!(printed(&mut vm, "(list (equal dired-before (buffer-string)) (= dired-point (point)) (buffer-modified-p))"),
            "(t t nil)", "DIRED_READ_ONLY_PRESERVES_DOCUMENT");
    }
    let mut commands = command::Commands::default();
    let mut presentation = presentation::Presentation::new();
    for key in ['x' as i64, 4, 127] {
        let outcome = presentation.command(&mut vm, &mut commands, CommandEvent::Character(key)).unwrap().unwrap();
        assert_eq!(outcome.signal.as_ref().map(|s| s.condition.as_str()), Some("buffer-read-only"),
            "DIRED_READ_ONLY_NATIVE_KEY: {key}");
        assert_eq!(printed(&mut vm, "(list (equal dired-before (buffer-string)) (= dired-point (point)))"), "(t t)");
    }
    assert_eq!(printed(&mut vm,
        "(progn (dired-revert) (list (equal dired-before (buffer-string)) buffer-read-only (buffer-modified-p)))"),
        "(t t nil)", "DIRED_OWNED_REFRESH_CAN_WRITE");
    assert_eq!(std::fs::read_to_string(directory.join("file.txt")).unwrap(), "unchanged");
}

#[test]
fn dired_native_keys_browse_refresh_and_return() {
    let fixture = Fixture::new("dired-keys");
    std::fs::create_dir(fixture.root.join("child")).unwrap();
    std::fs::write(fixture.root.join("entry.txt"), "Native Dired file\n").unwrap();
    let mut vm = fixture.vm();
    vm.eval_source("(goto-char 7) (setq entry-buffer (current-buffer) entry-point (point))").unwrap();
    let mut commands = command::Commands::default();
    let mut presentation = presentation::Presentation::new();
    attach(&mut vm, "\r");
    assert!(presentation.command(&mut vm, &mut commands, CommandEvent::Character(24)).unwrap().is_none());
    let opened = presentation.command(&mut vm, &mut commands, CommandEvent::Character(100)).unwrap().unwrap();
    assert!(opened.signal.is_none(), "DIRED_NATIVE_CXD: {opened:?}");
    assert_eq!(printed(&mut vm, "major-mode"), "dired-mode");
    vm.eval_source("(dired-goto-file \"child\") (setq child-row (point))").unwrap();
    for key in [110, 112] {
        let moved = presentation.command(&mut vm, &mut commands, CommandEvent::Character(key)).unwrap().unwrap();
        assert!(moved.signal.is_none(), "DIRED_NATIVE_NAVIGATION: {moved:?}");
    }
    assert_eq!(printed(&mut vm, "(= child-row (point))"), "t");
    for key in [13, 94] {
        let moved = presentation.command(&mut vm, &mut commands, CommandEvent::Character(key)).unwrap().unwrap();
        assert!(moved.signal.is_none(), "DIRED_NATIVE_CHILD_PARENT: {moved:?}");
    }
    assert_eq!(printed(&mut vm, "(equal (file-name-nondirectory (dired-get-filename)) \"child\")"), "t");
    std::fs::write(fixture.root.join("added.txt"), "added").unwrap();
    let refreshed = presentation.command(&mut vm, &mut commands, CommandEvent::Character(103)).unwrap().unwrap();
    assert!(refreshed.signal.is_none(), "DIRED_NATIVE_REFRESH: {refreshed:?}");
    assert_eq!(printed(&mut vm, "(dired-goto-file \"added.txt\")"), "t");
    let returned = presentation.command(&mut vm, &mut commands, CommandEvent::Character(113)).unwrap().unwrap();
    assert!(returned.signal.is_none());
    assert_eq!(printed(&mut vm, "(list (eq entry-buffer (current-buffer)) (= entry-point (point)))"), "(t t)");
    attach(&mut vm, "dired\r\r");
    let mx = presentation.command(&mut vm, &mut commands, CommandEvent::Character((1 << 27) | 120)).unwrap().unwrap();
    assert!(mx.signal.is_none(), "DIRED_NATIVE_MX: {mx:?}");
    assert_eq!(printed(&mut vm, "(list major-mode buffer-file-name (buffer-modified-p))"), "(dired-mode nil nil)");
}

#[cfg(unix)]
#[test]
fn dired_symlinks_failed_refresh_and_dead_quit_target() {
    use std::os::unix::fs::symlink;
    let fixture = Fixture::new("dired-safety");
    std::fs::create_dir(fixture.root.join("child")).unwrap();
    std::fs::write(fixture.root.join("entry.txt"), "Linked sentinel").unwrap();
    symlink("entry.txt", fixture.root.join("file-link")).unwrap();
    symlink("child", fixture.root.join("dir-link")).unwrap();
    symlink("missing", fixture.root.join("dangling")).unwrap();
    symlink("cycle", fixture.root.join("cycle")).unwrap();
    let mut vm = fixture.vm();
    vm.eval_source("(setq entry-buffer (current-buffer)) (dired default-directory) (setq original-dired (current-buffer))").unwrap();
    assert_eq!(printed(&mut vm, "(progn (dired-goto-file \"file-link\") (dired-find-file) (buffer-string))"), "\"Linked sentinel\"");
    vm.eval_source("(romacs-select-buffer original-dired) (dired-goto-file \"dir-link\") (dired-find-file)").unwrap();
    assert_eq!(printed(&mut vm, "major-mode"), "dired-mode");
    for name in ["dangling", "cycle"] {
        assert_eq!(printed(&mut vm, &format!("(progn (romacs-select-buffer original-dired) (dired-goto-file {}) (condition-case e (dired-find-file) (file-error 'refused)))", quoted(name))), "refused", "DIRED_LINK_REFUSAL: {name}");
    }
    vm.eval_source("(setq before-refresh (list (buffer-string) romacs-dired-rows (point)))").unwrap();
    let moved = fixture.root.with_extension("moved");
    std::fs::rename(&fixture.root, &moved).unwrap();
    let refresh = printed(&mut vm, "(condition-case e (dired-revert) (file-error 'refused))");
    std::fs::rename(&moved, &fixture.root).unwrap();
    assert_eq!(refresh, "refused", "DIRED_REFRESH_FAILURE");
    assert_eq!(printed(&mut vm, "(equal before-refresh (list (buffer-string) romacs-dired-rows (point)))"), "t", "DIRED_REFRESH_ATOMICITY");
    vm.eval_source("(let ((kill-buffer-query-functions nil) (kill-buffer-hook nil)) (kill-buffer entry-buffer)) (dired-quit-window)").unwrap();
    assert_eq!(printed(&mut vm, "(list (buffer-live-p (current-buffer)) (eq original-dired (current-buffer)))"), "(t nil)", "DIRED_DEAD_RETURN_FALLBACK");
}

#[test]
fn find_file_live_observer_controls() {
    let output = std::process::Command::new("python3")
        .args(["-m", "unittest", "-v", "test_find_file_live"])
        .current_dir(std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("harness"))
        .env("PYTHONDONTWRITEBYTECODE", "1")
        .output()
        .unwrap();
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        output.status.success() && stderr.contains("Ran 4 tests") && !stderr.contains("skipped"),
        "FIND_FILE_OBSERVER_CONTROLS: {}{}",
        String::from_utf8_lossy(&output.stdout),
        stderr
    );
}
