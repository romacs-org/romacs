//! Last-sexp evaluation bridge; the VM owns reading, execution, and printing.
use romacs_vm::Vm;

/// The bridge's commands, defined with VM2's own `defun` (before GNU's
/// byte-run.el replaces it). Its C-x C-e command is a romacs stand-in for GNU's
/// `eval-last-sexp` (elisp-mode.el) and is NAMED as one, so a command reply
/// can never pass it off as GNU's. C-u uses the shared command prefix carrier.
/// Keys are bound by `startup::STAND_INS`.
pub fn define(vm: &mut Vm) -> Result<(), String> {
    vm.eval_source(
        r#"
        (setq romacs-eval-request nil romacs-eval-buffer nil romacs-eval-point nil)
        (defun romacs-universal-argument (arg)
          "Accumulate a universal prefix; repeated C-u multiplies it by four."
          (interactive "P")
          (setq this-command last-command real-this-command real-last-command
                prefix-arg (list (if (consp arg) (* 4 (car arg)) 4))))
        (defun romacs-gui-eval-last-sexp (arg) (interactive "P")
          (setq romacs-eval-buffer (current-buffer)
                romacs-eval-point (point)
                romacs-eval-request (if arg 2 1)))
    "#,
    )?;
    Ok(())
}

/// Define the bridge and bind C-x C-e and C-u to it (tests; the GUI binds
/// through `startup::STAND_INS`).
#[cfg(test)]
pub fn install(vm: &mut Vm) -> Result<(), String> {
    define(vm)?;
    vm.eval_source("(global-set-key [24 5] 'romacs-gui-eval-last-sexp) (global-set-key [21] 'romacs-universal-argument)")?;
    Ok(())
}

/// Run after native command dispatch. Lisp slots keep request objects GC-rooted.
pub fn after_command(vm: &mut Vm) -> Result<(), String> {
    // Not installed under --gnu-lisp: GNU's defun cannot make commands yet.
    let request = vm.eval_source("(and (boundp 'romacs-eval-request) romacs-eval-request)")?;
    let request = request.display()?;
    if request == "nil" { return Ok(()); }
    vm.eval_source("(setq romacs-eval-request nil)")?;
    let result = (|| -> Result<String, Failure> {
        // Not with-current-buffer: after loadup that is GNU's subr.el macro over
        // the special form save-current-buffer, which VM2 does not have yet.
        let value = vm.eval_source("(let ((old (current-buffer))) (unwind-protect (progn (set-buffer romacs-eval-buffer) (buffer-substring-no-properties (point-min) romacs-eval-point)) (set-buffer old)))")?;
        // display prints strings as JSON; decode only this Rust-owned copy.
        let source: String =
            serde_json::from_str(&value.display()?).map_err(|error| error.to_string())?;
        let source = preceding_sexp(vm, &source)?;
        // A Lisp signal is reported the way GNU's command loop reports it:
        // error-message-string of the error object (print.c
        // print_error_message), with no prefix. The outcome stays in a traced
        // Lisp slot; each piece is converted to owned text immediately.
        vm.eval_source(&format!(
            "(setq romacs-eval-outcome (condition-case romacs-eval-error (cons t {source}) \
             (error (cons nil (error-message-string romacs-eval-error)))))"
        ))?;
        let ok = vm.eval_source("(car romacs-eval-outcome)")? != romacs_vm::Fixed::NIL;
        let value = vm.eval_source("(cdr romacs-eval-outcome)")?;
        let printed = value.display()?;
        vm.eval_source("(setq romacs-eval-outcome nil)")?;
        if ok { return Ok(printed); }
        let message: String = serde_json::from_str(&printed).map_err(|error| error.to_string())?;
        Err(Failure::Lisp(message))
    })();
    match result {
        Ok(printed) if request == "2" => {
            let insertion = vm.eval_source(&format!(
                "(let ((old (current-buffer))) (unwind-protect (progn (set-buffer romacs-eval-buffer) (goto-char romacs-eval-point) (insert {})) (set-buffer old)))",
                serde_json::to_string(&printed).map_err(|error| error.to_string())?
            ));
            if let Err(error) = insertion {
                echo(vm, &format!("Error: {error}"))?;
            }
        }
        Ok(printed) => echo(vm, &printed)?,
        Err(Failure::Lisp(message)) => echo(vm, &message)?,
        Err(Failure::Bridge(error)) => echo(vm, &format!("Error: {error}"))?,
    }
    vm.eval_source("(setq romacs-eval-request nil romacs-eval-buffer nil romacs-eval-point nil)")?;
    Ok(())
}

/// A Lisp signal carries GNU's message text; a bridge failure (no complete
/// sexp, an internal VM error) keeps the bridge's own "Error: " report.
enum Failure { Lisp(String), Bridge(String) }
impl From<String> for Failure { fn from(error: String) -> Self { Failure::Bridge(error) } }

fn preceding_sexp(vm: &mut Vm, source: &str) -> Result<String, String> {
    let source = source.trim_end_matches(|c: char| c.is_ascii_whitespace());
    // Try balanced suffixes in source order: the first whole datum includes any
    // quote prefix and outer list. Never execute a probing reader result, nor
    // keep its unrooted Value across the next read/evaluation.
    for (start, _) in source.char_indices() {
        let end = vm
            .read_source(&source[start..])
            .ok()
            .flatten()
            .map(|(_, end)| end);
        if end == Some(source.len() - start) {
            return Ok(source[start..].into());
        }
    }
    Err("No complete sexp before point".into())
}

fn echo(vm: &mut Vm, text: &str) -> Result<(), String> {
    // JSON's escapes (including four-digit Unicode controls) are accepted by
    // VM2's reader. Never select the minibuffer window just to paint its buffer.
    let text = serde_json::to_string(text).map_err(|error| error.to_string())?;
    vm.eval_source(&format!(
        r#"
        (let ((old-buffer (current-buffer)) (old-point (point))
              (inhibit-read-only t))
          (unwind-protect
              (progn (set-buffer (window-buffer (minibuffer-window)))
                     (erase-buffer) (insert {text}))
            (set-buffer old-buffer) (goto-char old-point)))
    "#
    ))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn text(vm: &mut Vm, source: &str) -> String {
        let value = vm.eval_source(source).unwrap();
        value.display().unwrap()
    }
    fn setup() -> Vm {
        let mut vm = Vm::new();
        vm.set_frame_geometry(1200, 800, 20, 10).unwrap();
        vm.set_collection_floor(1);
        install(&mut vm).unwrap();
        vm.eval_source("(switch-to-buffer \"*eval-test*\")")
            .unwrap();
        vm
    }
    fn request(vm: &mut Vm, source: &str, insert: bool) {
        vm.eval_source(&format!(
            "(erase-buffer) (insert {})",
            serde_json::to_string(source).unwrap()
        ))
        .unwrap();
        let mut commands = crate::command::Commands::default();
        if insert { commands.dispatch(vm, romacs_vm::CommandEvent::Character(21)).unwrap(); }
        commands.dispatch(vm, romacs_vm::CommandEvent::Character(24)).unwrap();
        commands.dispatch(vm, romacs_vm::CommandEvent::Character(5)).unwrap();
        after_command(vm).unwrap();
    }
    fn echo(vm: &mut Vm) -> String {
        let printed = text(
            vm,
            "(with-current-buffer (window-buffer (minibuffer-window)) (buffer-string))",
        );
        serde_json::from_str(&printed).unwrap()
    }
    #[test]
    fn installs_global_interactive_binding() {
        let mut vm = setup();
        assert_eq!(text(&mut vm, "(key-binding [24 5])"), "romacs-gui-eval-last-sexp");
        vm.eval_source("(call-interactively (key-binding [24 5]))")
            .unwrap();
        assert_eq!(text(&mut vm, "romacs-eval-request"), "1");
    }
    #[test]
    fn real_reader_execution_printer_and_echo_preserve_editor_state() {
        let mut vm = setup();
        for (source, expected) in [
            ("(+ 20 22)", "42"),
            ("\"a\\\"b\\\\c\"", "\"a\\\"b\\\\c\""),
            ("'(1 \"two\" nil)", "(1 \"two\" nil)"),
            ("nil", "nil"),
            ("(substring \"κόσμε\" 1 4)", "\"όσμ\""),
            ("(setq should-not-run 99) (+ 20 22)", "42"),
            ("garbage\n'((1 2) \"x)y\")", "((1 2) \"x)y\")"),
        ] {
            vm.eval_source("(setq should-not-run nil saved-window (selected-window) saved-buffer (current-buffer))").unwrap();
            request(&mut vm, source, false);
            assert_eq!(echo(&mut vm), expected, "{source}");
            assert_eq!(
                text(
                    &mut vm,
                    "(list (eq saved-window (selected-window)) (eq saved-buffer (current-buffer)) (= (point) (point-max)) should-not-run)"
                ),
                "(t t t nil)"
            );
        }
    }
    #[test]
    fn point_selects_final_form_not_whole_buffer_or_future_text() {
        let mut vm = setup();
        vm.eval_source("(insert \"(+ 1 2) (+ 20 22) (+ 3 4)\") (goto-char 18) (call-interactively (key-binding [24 5]))").unwrap();
        after_command(&mut vm).unwrap();
        assert_eq!(echo(&mut vm), "42");
        assert_eq!(text(&mut vm, "(point)"), "18");
    }
    #[test]
    fn prefix_inserts_printed_value_and_is_consumed() {
        let mut vm = setup();
        request(&mut vm, "(+ 20 22)", true);
        assert_eq!(text(&mut vm, "(buffer-string)"), "\"(+ 20 22)42\"");
        request(&mut vm, "(+ 1 2)", false);
        assert_eq!(echo(&mut vm), "3");
        assert_eq!(text(&mut vm, "(buffer-string)"), "\"(+ 1 2)\"");
    }
    #[test]
    fn errors_echo_and_next_command_still_evaluates() {
        let mut vm = setup();
        // GNU's own text for each signal: evidence/usable/gnu-eval-last-sexp-errors.txt
        for (source, gnu) in [
            ("(/ 1 0)", "Arithmetic error"),
            ("(car 1)", "Wrong type argument: listp, 1"),
            ("(error \"Boom\")", "Boom"),
            ("(undefined-fn 1)", "Symbol’s function definition is void: undefined-fn"),
            ("undefined-var", "Symbol’s value as variable is void: undefined-var"),
        ] {
            request(&mut vm, source, false);
            assert_eq!(echo(&mut vm), gnu, "{source}");
        }
        request(&mut vm, "(+ 1 2))", false);
        assert_eq!(echo(&mut vm), "Error: No complete sexp before point");
        request(&mut vm, "(+ 20 22)", false);
        assert_eq!(echo(&mut vm), "42");
    }
    #[test]
    fn prefix_cancels_on_an_unrelated_command() {
        let mut vm = setup();
        vm.command_event(romacs_vm::CommandEvent::Character(21))
            .unwrap();
        after_command(&mut vm).unwrap();
        vm.command_event(romacs_vm::CommandEvent::Character(97))
            .unwrap();
        after_command(&mut vm).unwrap();
        request(&mut vm, "(+ 20 22)", false);
        assert_eq!(echo(&mut vm), "42");
        assert_eq!(text(&mut vm, "(buffer-string)"), "\"(+ 20 22)\"");
    }
    #[test]
    fn embedding_roundtrips_json_control_escapes() {
        let mut vm = setup();
        let literal = serde_json::to_string("\0\u{8}	\n\u{c}\r\u{1f}λ").unwrap();
        request(&mut vm, &literal, false);
        let expected = literal;
        assert_eq!(echo(&mut vm), expected);
    }
}
