//! Echo area: the message a command left, and its clearing when the next
//! command starts (keyboard.c command_loop_1). A command's error reaches
//! the echo area as GNU's does, through `command-error-function' in the
//! Lisp command loop (lisp/vm/keyboard.el). Each change runs
//! `romacs-echo-area-functions' with the message, or nil once cleared,
//! where a layer shows messages (lisp/completion/romacs-complete.el).
use romacs_vm::Vm;

/// The echo area's Lisp, defined once per VM: by `install' when a session
/// starts, else by the first form that runs it. `romacs--echo-display'
/// shows a text, or nothing, in the minibuffer window's buffer, never
/// selecting that window and restoring the current buffer and its point;
/// then it runs `romacs-echo-area-functions' with the text (a failing
/// function leaves the echo area as it is).
/// `romacs--echo-shown' is the message it was current for, so a later one
/// is new to the display whoever left it: `romacs--echo-sync' shows it
/// (non-nil when it did), as the outer loop does after timers and the
/// evaluator's own waits do before they redisplay, pending input or not
/// (romacs_vm host_input.rs), as GNU's `message' redisplays the echo area
/// at once (xdisp.c message3_nolog). Not while a minibuffer
/// is active: its window shows the minibuffer, and the prompt's next key
/// clears the message unseen (ledger/divergences VM2-ECHO-LAYER).
///
/// As GNU's echo area, a multibyte buffer, holds it: ASCII messages
/// (`format''s and `set_echo_area_message''s) are unibyte strings; a raw
/// byte shows as GNU's \ooo escape, so the text is always printable. The
/// message keeps its text properties, so a key hint's `help-key-binding'
/// reaches the echo area and the layers showing it; only a message with a
/// raw byte, rebuilt for that escape, loses them.
const LISP: &str = r#"(progn
  (defvar romacs--echo-shown nil)
  (defun romacs--echo-display (text)
    (let ((old-buffer (current-buffer)) (old-point (point)) (inhibit-read-only t))
      (unwind-protect
          (progn (set-buffer (window-buffer (minibuffer-window)))
                 (erase-buffer) (and text (insert text)))
        (set-buffer old-buffer) (goto-char old-point)))
    (setq romacs--echo-shown (current-message))
    (and (boundp 'romacs-echo-area-functions) romacs-echo-area-functions
         (condition-case nil (run-hook-with-args 'romacs-echo-area-functions text) (error nil))))
  (defun romacs--echo-printable (message)
    (let ((text (string-to-multibyte message)))
      (if (catch 'raw (mapc (lambda (c) (if (> c #x10ffff) (throw 'raw t))) text) nil)
          (string-to-multibyte
           (mapconcat (lambda (c) (if (> c #x10ffff) (format "\\%o" (logand c 255)) (string c)))
                      (substring-no-properties text) ""))
        text)))
  (defun romacs--echo-sync ()
    (unless (romacs--echo-current-p)
      ;; Keep the Lisp checks when native reads cannot establish currency,
      ;; including when either public function has been advised.
      (let ((message (current-message)))
        (unless (or (eq message romacs--echo-shown) (> (minibuffer-depth) 0))
          (romacs--echo-display (and message (romacs--echo-printable message)))
          t)))))"#;

/// FORM, after LISP the first time. LISP holds strings, which keep a form
/// from the source cache (source_cache.rs): it is read once per VM, and a
/// key's clearing, with no string, is compiled once. Its body calls the
/// `current-message' and `minibuffer-depth' `Vm::echo_current' checks;
/// altering the reader (including `read-symbol-shorthands'), the evaluator
/// or core macros before this install is outside that guard's contract.
fn display(vm: &mut Vm, form: &str) -> Result<(), String> {
    if vm.eval_source(&format!("(and (fboundp 'romacs--echo-sync) (progn {form} t))"))?.is_nil() {
        vm.eval_source(LISP)?;
        vm.eval_source(form)?;
    }
    Ok(())
}

/// Define the echo area's Lisp now, before the session's own Lisp runs. A
/// wait inside a timer or the init redisplays the echo area only through
/// it (host_input.rs), and the first idle check's timers may wait before
/// the outer loop has shown anything: a message one of them leaves there
/// must be shown then, as GNU's `message' shows it.
pub fn install(vm: &mut Vm) -> Result<(), String> {
    display(vm, "nil")
}

/// Show `text` in the echo area.
pub fn show(vm: &mut Vm, message: &str) -> Result<(), String> {
    let text = serde_json::to_string(message).map_err(|error| error.to_string())?;
    vm.set_echo_area_message(Some(message));
    display(vm, &format!("(romacs--echo-display {text})"))
}

/// Show the message the command left (`message`, or printing to `t`), as
/// GNU's echo area shows it once the command returns. Nothing when it left none.
pub fn show_current_message(vm: &mut Vm) -> Result<(), String> {
    display(vm, "(let ((message (current-message))) (and message (romacs--echo-display (romacs--echo-printable message))))")
}

/// The echo area after timers changed the message: shown, or cleared by
/// `(message nil)`.
pub fn sync_current_message(vm: &mut Vm) -> Result<(), String> {
    display(vm, "(romacs--echo-sync)")
}

/// Clear a message left by the previous command.
pub fn clear(vm: &mut Vm) -> Result<(), String> {
    // keyboard.c read_char: the hook runs while the message still shows.
    if vm.echo_area_message_shown() {
        vm.eval_source("(and (fboundp 'romacs--safe-run-hooks) (romacs--safe-run-hooks 'echo-area-clear-hook))")?;
    }
    vm.set_echo_area_message(None);
    hide(vm)
}

/// Take the message off the display, leaving it current.
pub fn hide(vm: &mut Vm) -> Result<(), String> {
    display(vm, "(romacs--echo-display nil)")
}

/// The echo area's current text.
pub fn text(vm: &mut Vm) -> Result<String, String> {
    // Not with-current-buffer: under --gnu-lisp that is GNU's macro over the
    // special form save-current-buffer, which the VM does not have yet.
    let value = vm.eval_source(
        "(let ((old-buffer (current-buffer)))
           (unwind-protect
               (progn (set-buffer (window-buffer (minibuffer-window)))
                      (buffer-substring-no-properties (point-min) (point-max)))
             (set-buffer old-buffer)))",
    )?;
    serde_json::from_str(&value.display()?).map_err(|error| error.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use romacs_vm::{CommandEvent, HostEvent, HostFrame, HostInput, WindowEnd};
    use std::{cell::RefCell, collections::VecDeque, rc::Rc};

    struct EchoRead {
        pending: bool,
        keys: VecDeque<i64>,
        paints: Rc<RefCell<Vec<String>>>,
    }

    impl HostInput for EchoRead {
        fn input_pending(&mut self) -> bool { self.pending && !self.keys.is_empty() }
        fn redisplay(&mut self, frame: HostFrame<'_>) -> Result<Vec<WindowEnd>, String> {
            let echo = frame.frame.windows.iter().find(|window| window.snapshot.id == frame.minibuffer)
                .ok_or("missing minibuffer window")?;
            self.paints.borrow_mut().push(echo.snapshot.text.utf8.clone());
            Ok(Vec::new())
        }
        fn next_event(&mut self, _: Option<std::time::Duration>) -> Result<HostEvent, String> {
            self.keys.pop_front().map(|key| HostEvent::Key(CommandEvent::Character(key))).ok_or_else(|| "keys exhausted".into())
        }
    }

    fn echo_read(vm: &mut Vm, pending: bool, keys: impl Into<VecDeque<i64>>) -> Rc<RefCell<Vec<String>>> {
        install(vm).unwrap();
        let paints = Rc::new(RefCell::new(Vec::new()));
        vm.set_host_input(Some(Box::new(EchoRead { pending, keys: keys.into(), paints: Rc::clone(&paints) })));
        paints
    }

    fn gnu_echo_read() -> Vm {
        let boot = crate::test_world::Boot { echo_area: true, ..crate::test_world::Boot::LOADUP };
        let (mut vm, loadup) = crate::test_world::chain(boot);
        crate::test_home::private_home(&mut vm, "echo-read");
        crate::startup::resume_loadup(&mut vm, boot.launch, loadup).unwrap();
        vm
    }

    #[test]
    fn a_pending_key_wait_paints_a_message_left_by_a_timer() {
        std::thread::Builder::new().stack_size(64 << 20).spawn(|| {
            let mut vm = gnu_echo_read();
            let paints = echo_read(&mut vm, true, [97]);
            vm.eval_source("(setq timer-list nil timer-idle-list nil)
                            (run-with-timer 0 nil (lambda () (message \"tick\")))").unwrap();
            assert_eq!(vm.eval_source("(read-event)").unwrap().display().unwrap(), "97");
            assert_eq!(*paints.borrow(), ["tick"]);
        }).unwrap().join().unwrap();
    }

    /// Advice on a public function the echo sync calls runs on an ordinary
    /// wait and on a pending read alike, as the full path calls it: advice on
    /// `current-message', on an uninterned `current-message' whose namesake
    /// holds the primitive, and on `minibuffer-depth' while a minibuffer is
    /// counted. Each case: its setup once, then the two reads' (KEY CALLS).
    #[test]
    fn advised_echo_functions_are_dispatched_on_ordinary_and_pending_reads() {
        const ADVICE: &str = "(defvar romacs-test-calls 0)
            (defun romacs-test-virtual (&rest _) (setq romacs-test-calls (1+ romacs-test-calls)) \"virtual message\")
            (defun romacs-test-depth (&rest _) (setq romacs-test-calls (1+ romacs-test-calls)) 0)";
        let cases = [
            ("(advice-add 'current-message :around #'romacs-test-virtual)", "", ["(97 2)", "(97 3)"]),
            ("(let ((primitive (symbol-function 'current-message)))
                (advice-add 'current-message :around #'romacs-test-virtual)
                (unintern \"current-message\" obarray)
                (fset (intern \"current-message\") primitive))", "", ["(97 2)", "(97 3)"]),
            ("(advice-add 'minibuffer-depth :around #'romacs-test-depth)",
             "(setq romacs--minibuf-level 1) (message \"virtual message\") (setq romacs--echo-shown nil)", ["(97 1)", "(97 1)"]),
        ];
        for (setup, each, reads) in cases {
            std::thread::Builder::new().stack_size(64 << 20).spawn(move || {
                let mut vm = gnu_echo_read();
                install(&mut vm).unwrap();
                vm.eval_source(ADVICE).unwrap();
                vm.eval_source(setup).unwrap();
                for pending in [false, true] {
                    let paints = echo_read(&mut vm, pending, [97, 97]);
                    vm.eval_source(&format!("(setq timer-list nil timer-idle-list nil) (message nil) (setq romacs--echo-shown nil romacs-test-calls 0) {each}")).unwrap();
                    let read = "(list (romacs--read-host-event) romacs-test-calls)";
                    assert_eq!(vm.eval_source(read).unwrap().display().unwrap(), reads[0], "{setup} pending {pending}");
                    assert_eq!(*paints.borrow(), ["virtual message"], "{setup} pending {pending}");
                    assert_eq!(vm.eval_source(read).unwrap().display().unwrap(), reads[1], "{setup} pending {pending}");
                    assert_eq!(paints.borrow().len(), if pending { 1 } else { 2 }, "{setup} pending {pending}");
                }
            }).unwrap().join().unwrap();
        }
    }

    #[test]
    fn quail_guidance_is_painted_before_a_pending_continuation() {
        std::thread::Builder::new().stack_size(64 << 20).spawn(|| {
            let mut vm = gnu_echo_read();
            let paints = echo_read(&mut vm, true, [39, 101]);
            vm.eval_source("(erase-buffer)
                (setq timer-list nil timer-idle-list nil romacs-test-quail-waits 0)
                (activate-input-method \"latin-1-prefix\")
                (setq input-method-use-echo-area t)
                (advice-add 'romacs--wait-state :before
                  (lambda () (when quail-translating
                    (setq romacs-test-quail-waits (1+ romacs-test-quail-waits)))))
                (message nil)").unwrap();
            let command = crate::command::run(&mut vm).unwrap();
            assert_eq!((command.command.as_deref(), command.signal), (Some("self-insert-command"), None));
            assert_eq!(vm.eval_source("(list (buffer-string) romacs-test-quail-waits)").unwrap().display().unwrap(), "(\"é\" 2)");
            assert_eq!(paints.borrow().len(), 1);
            assert!(paints.borrow()[0].contains("AEIOUYaeiouy"), "{paints:?}");
        }).unwrap().join().unwrap();
    }

    fn workspace() -> (Vm, crate::command::Commands, crate::presentation::Presentation) {
        let mut vm = Vm::new();
        vm.set_collection_floor(1);
        vm.set_frame_geometry(1200, 800, 20, 10).unwrap();
        crate::startup::initialize(&mut vm).unwrap();
        crate::eval_last_sexp::install(&mut vm).unwrap();
        (vm, Default::default(), crate::presentation::Presentation::new())
    }

    /// A key hint's face reaches the echo area and what shows its messages.
    #[test]
    fn a_message_keeps_its_faces_to_the_echo_area() {
        let (mut vm, _, _) = workspace();
        vm.set_echo_area_host(true);
        vm.eval_source("(setq seen nil romacs-echo-area-functions (list (lambda (text) (setq seen text))))
                        (message \"%s\" (propertize \"C-c\" 'face 'help-key-binding))").unwrap();
        show_current_message(&mut vm).unwrap();
        let printed = |vm: &mut Vm, source: &str| vm.eval_source(source).unwrap().display().unwrap();
        assert_eq!(printed(&mut vm, "(list (substring-no-properties seen) (get-text-property 0 'face seen))"), "(\"C-c\" help-key-binding)");
        assert_eq!(text(&mut vm).unwrap(), "C-c");
        assert_eq!(printed(&mut vm, "(save-current-buffer (set-buffer (window-buffer (minibuffer-window))) (get-text-property 1 'face))"),
                   "help-key-binding");
    }

    /// A command wiping a message runs `echo-area-clear-hook' while the
    /// message shows; with none showing it does not run.
    #[test]
    fn clearing_a_message_runs_the_echo_area_clear_hook() {
        let (mut vm, _, _) = workspace();
        vm.set_echo_area_host(true);
        vm.eval_source("(progn (fset 'romacs--safe-run-hooks (lambda (hook) (funcall (symbol-value hook)))) (setq echo-probe nil echo-area-clear-hook (lambda () (setq echo-probe (cons (current-message) echo-probe)))) (message \"shown\"))").unwrap();
        show_current_message(&mut vm).unwrap();
        clear(&mut vm).unwrap();
        clear(&mut vm).unwrap();
        assert_eq!(vm.eval_source("echo-probe").unwrap().display().unwrap(), "(\"shown\")");
    }

    /// A message holding a raw byte, even one whose `message' signalled,
    /// shows it as GNU's echo area does, `\351x', and the session goes on.
    #[test]
    fn raw_bytes_in_the_current_message_show_as_octal_escapes() {
        let (mut vm, _, _) = workspace();
        vm.set_echo_area_host(true);
        vm.eval_source("(let ((message-log-max nil)) (message \"%s\" (unibyte-string 233 120)))").unwrap();
        show_current_message(&mut vm).unwrap();
        assert_eq!(text(&mut vm).unwrap(), "\\351x");
    }

    /// GNU 32.0.50 transcripts (`emacs -Q --batch`, error-message-string of
    /// the caught error) for the signals each command raises below.
    #[test]
    fn command_errors_show_gnus_message_and_the_session_keeps_editing() {
        let (mut vm, mut commands, mut presentation) = workspace();
        vm.eval_source("(erase-buffer) (insert \"ab\")").unwrap();
        // C-f at the end of the buffer: forward-char signals end-of-buffer.
        let outcome = presentation.command(&mut vm, &mut commands, CommandEvent::Character(6)).unwrap().unwrap();
        assert_eq!(outcome.signal.as_ref().unwrap().condition, "end-of-buffer");
        assert_eq!(text(&mut vm).unwrap(), "End of buffer");
        // The next command clears the message and still edits.
        presentation.command(&mut vm, &mut commands, CommandEvent::Character('λ' as i64)).unwrap();
        assert_eq!(text(&mut vm).unwrap(), "");
        let buffer = vm.eval_source("(buffer-string)").unwrap();
        assert_eq!(buffer.display().unwrap(), "\"abλ\"");
        // A Lisp error from a command: GNU prints the data with prin1.
        vm.eval_source("(defun romacs-test-fail () (interactive) (car 1))
                        (define-key global-map [f5] 'romacs-test-fail)").unwrap();
        presentation.command(&mut vm, &mut commands, CommandEvent::Named("f5".into())).unwrap();
        assert_eq!(text(&mut vm).unwrap(), "Wrong type argument: listp, 1");
        vm.eval_source("(defun romacs-test-fail () (interactive) (error \"Boom %d\" 7))").unwrap();
        presentation.command(&mut vm, &mut commands, CommandEvent::Named("f5".into())).unwrap();
        assert_eq!(text(&mut vm).unwrap(), "Boom 7");
        presentation.command(&mut vm, &mut commands, CommandEvent::Character('!' as i64)).unwrap();
        let buffer = vm.eval_source("(buffer-string)").unwrap();
        assert_eq!(buffer.display().unwrap(), "\"abλ!\"");
        assert_eq!(text(&mut vm).unwrap(), "");
    }

    /// Under --gnu-lisp the keys resolve through GNU's own subr.el keymaps,
    /// after GNU's loadup.el ran on the GUI evaluator's 64 MiB stack.
    #[test]
    fn gnu_subr_keymap_drives_editing_and_reports_what_is_missing() {
        std::thread::Builder::new().stack_size(64 << 20).spawn(gnu_subr_keymap_body).unwrap().join().unwrap();
    }
    fn gnu_subr_keymap_body() {
        let (mut vm, loadup) = crate::test_world::chain(crate::test_world::Boot { floor: Some(1), ..crate::test_world::Boot::GNU_ONLY });
        crate::test_home::private_home(&mut vm, "echo-keymap");
        let started = crate::startup::resume_loadup(&mut vm, crate::startup::Launch::GnuOnly, loadup).unwrap();
        let loadup = started.loadup.as_ref().unwrap();
        assert!(loadup.rows.iter().any(|r| r.site.name == "emacs-lisp/backquote"
            && matches!(r.outcome, crate::loadup::Outcome::Complete { .. })), "{}", loadup.table());
        assert!(loadup.rows.iter().flat_map(|r| &r.nested).any(|n| n.0 == "emacs-lisp/pcase.el"), "{}", loadup.table());
        assert!(started.bindings.is_empty(), "--gnu-lisp installs no stand-ins");
        let (mut commands, mut presentation) = (crate::command::Commands::default(), crate::presentation::Presentation::new());
        // GNU's global-map is subr.el's, not VM2's built-in one.
        let map = vm.eval_source("(eq (lookup-key global-map [6]) 'forward-char)").unwrap();
        assert_eq!(map.display().unwrap(), "t");
        vm.eval_source("(erase-buffer)").unwrap();
        for key in ['a' as i64, 'λ' as i64, 2, 'Δ' as i64, 1, 'x' as i64, 5] {
            let outcome = presentation.command(&mut vm, &mut commands, CommandEvent::Character(key)).unwrap().unwrap();
            assert!(outcome.signal.is_none(), "{outcome:?}");
        }
        let state = vm.eval_source("(list (buffer-string) (point))").unwrap();
        assert_eq!(state.display().unwrap(), "(\"xaΔλ\" 5)");
        let command = vm.eval_source("(not (null (interactive-form 'execute-extended-command)))").unwrap();
        assert_eq!(command.display().unwrap(), "t");
        let backend = vm.eval_source("(functionp completing-read-function)").unwrap();
        assert_eq!(backend.display().unwrap(), "t");
        let outcome = presentation.command(&mut vm, &mut commands, CommandEvent::Character((1 << 27) | 'x' as i64)).unwrap().unwrap();
        assert_eq!(outcome.command.as_deref(), Some("execute-extended-command"));
        // With GNU's minibuffer variables present, M-x reaches the minibuffer
        // read; this test attaches no keyboard, so it ends as GNU batch does.
        let signal = outcome.signal.expect("M-x reports that no input is available");
        assert_eq!((signal.condition.as_str(), signal.data.as_str()), ("end-of-file", "(\"Error reading from stdin\")"));
        // GNU 32.0.50: (error-message-string (quote (end-of-file "Error reading from stdin"))).
        assert_eq!(text(&mut vm).unwrap(), "End of file during parsing: Error reading from stdin");
        presentation.command(&mut vm, &mut commands, CommandEvent::Character('!' as i64)).unwrap();
        assert_eq!(text(&mut vm).unwrap(), "");
        let state = vm.eval_source("(buffer-string)").unwrap();
        assert_eq!(state.display().unwrap(), "\"xaΔλ!\"");
    }
}
