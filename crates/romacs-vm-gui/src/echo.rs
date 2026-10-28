//! Echo area: the command loop's error report and its clearing.
//!
//! GNU does both in C (keyboard.c `command-error-default-function`, which
//! prints with print.c `print_error_message`; and command_loop_1 clearing the
//! echo area when the next command starts), so this is host code, not Lisp.
//! The message text itself comes from the VM's `error-message-string`.
use romacs_vm::{CommandSignal, Vm};

/// Show `text` in the echo area: the minibuffer window's buffer. Never selects
/// the minibuffer window, and restores the current buffer and its point.
pub fn show(vm: &mut Vm, message: &str) -> Result<(), String> {
    let text = serde_json::to_string(message).map_err(|error| error.to_string())?;
    vm.eval_source(&format!(
        "(let ((old-buffer (current-buffer)) (old-point (point)) (inhibit-read-only t))
           (unwind-protect
               (progn (set-buffer (window-buffer (minibuffer-window)))
                      (erase-buffer) (insert {text}))
             (set-buffer old-buffer) (goto-char old-point)))"
    ))?;
    vm.set_echo_area_message(Some(message));
    Ok(())
}

/// Show the message the command left (`message`, or printing to `t`), as
/// GNU's echo area shows it once the command returns. Nothing when it left none.
pub fn show_current_message(vm: &mut Vm) -> Result<(), String> {
    current_message(vm, false)
}

/// The echo area after timers changed the message: shown, or cleared by
/// `(message nil)`.
pub fn sync_current_message(vm: &mut Vm) -> Result<(), String> {
    current_message(vm, true)
}

fn current_message(vm: &mut Vm, clear_when_none: bool) -> Result<(), String> {
    // As GNU's echo area, a multibyte buffer, holds it: ASCII messages
    // (`format''s and `set_echo_area_message''s) are unibyte strings, which
    // the VM prints as byte vectors.
    let value = vm.eval_source("(let ((message (current-message)))
                                  (and message (string-to-multibyte (substring-no-properties message))))")?;
    if value.is_nil() { return if clear_when_none { clear(vm) } else { Ok(()) }; }
    let printed = value.display()?;
    let message: String = serde_json::from_str(&printed).unwrap_or(printed);
    show(vm, &message)
}

/// Clear a message left by the previous command.
pub fn clear(vm: &mut Vm) -> Result<(), String> {
    vm.eval_source(
        "(let ((old-buffer (current-buffer)) (old-point (point)) (inhibit-read-only t))
           (unwind-protect
               (progn (set-buffer (window-buffer (minibuffer-window))) (erase-buffer))
             (set-buffer old-buffer) (goto-char old-point)))",
    )?;
    vm.set_echo_area_message(None);
    Ok(())
}

/// The echo area's current text.
#[cfg(test)]
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

/// GNU's text for a signal the command loop caught, from `error-message-string`
/// over the error object rebuilt from the VM's printed condition and data.
/// Unreadable data (`#<buffer x>`) cannot be rebuilt; it falls back to the
/// printed pair rather than inventing a message.
pub fn signal_message(vm: &mut Vm, signal: &CommandSignal) -> Result<String, String> {
    let rebuilt = format!("(error-message-string (quote ({} . {})))", signal.condition, signal.data);
    match vm.eval_source(&rebuilt) {
        Ok(value) => serde_json::from_str(&value.display()?).map_err(|error| error.to_string()),
        Err(_) => Ok(format!("{}: {}", signal.condition, signal.data)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use romacs_vm::CommandEvent;

    fn workspace() -> (Vm, crate::command::Commands, crate::presentation::Presentation) {
        let mut vm = Vm::new();
        vm.set_collection_floor(1);
        vm.set_frame_geometry(1200, 800, 20, 10).unwrap();
        crate::startup::initialize(&mut vm).unwrap();
        crate::eval_last_sexp::install(&mut vm).unwrap();
        (vm, Default::default(), crate::presentation::Presentation::new())
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
        let mut vm = Vm::new();
        vm.set_collection_floor(1);
        vm.set_frame_geometry(1200, 800, 20, 10).unwrap();
        let started = crate::startup::initialize_loadup(&mut vm, crate::startup::Launch::GnuOnly, None).unwrap();
        let loadup = started.loadup.as_ref().unwrap();
        assert!(loadup.rows.iter().any(|r| r.site.name == "emacs-lisp/backquote"
            && matches!(r.outcome, crate::loadup::Outcome::Complete { .. })), "{}", loadup.table());
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
        let shown = text(&mut vm).unwrap();
        assert_eq!(shown, signal_message(&mut vm, &signal).unwrap());
        presentation.command(&mut vm, &mut commands, CommandEvent::Character('!' as i64)).unwrap();
        assert_eq!(text(&mut vm).unwrap(), "");
        let state = vm.eval_source("(buffer-string)").unwrap();
        assert_eq!(state.display().unwrap(), "\"xaΔλ!\"");
    }
}
