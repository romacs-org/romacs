//! Evaluation results as a chip after the evaluated sexp
//! (rlisp/romacs-eval-chip.el) on the real --loadup chain with a window
//! system: each evaluation command's printed value or error shows once, in
//! the chip, never also as a toast; the echo area's message and *Messages*
//! stay GNU's; the next command that moves point or edits hides it; and
//! toasts follow the echo area, fading when the next command clears it.
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
#[path = "../src/measure.rs"]
mod measure;
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

const LINE: u32 = 20;

struct Keys(VecDeque<HostEvent>);
impl HostInput for Keys {
    fn input_pending(&mut self) -> bool { false }
    fn redisplay(&mut self, _: HostFrame) -> Result<Vec<romacs_vm::WindowEnd>, String> { Ok(vec![]) }
    fn next_event(&mut self, _: Option<std::time::Duration>) -> Result<HostEvent, String> {
        self.0.pop_front().ok_or_else(|| "eval chip host script exhausted".into())
    }
}

/// Attach a keyboard that types TEXT.
fn attach(vm: &mut Vm, text: &str) {
    let keys = text.chars().map(|c| HostEvent::Key(CommandEvent::Character(c as i64))).collect();
    vm.set_host_input(Some(Box::new(Keys(keys))));
}

fn printed(vm: &mut Vm, source: &str) -> String {
    vm.eval_source(source).and_then(|v| v.display()).unwrap_or_else(|e| format!("VM-ERROR {e}"))
}

type Engine = std::rc::Rc<std::cell::RefCell<romacs_layout2::LayoutEngine>>;

/// Lay the frame out as the GUI does, so positions are measured.
fn paint(vm: &mut Vm, engine: &Engine) {
    let snapshot = vm.frame_snapshot(1280, 800).unwrap();
    let (_, ends) = engine.borrow_mut().layout_frame(&snapshot, 1, None).unwrap();
    vm.display_apply_layout(&ends).unwrap();
}

/// Run the command of KEYS as the GUI's outer loop does, the frame laid
/// out before each key.
fn keys(vm: &mut Vm, engine: &Engine, codes: &[i64]) {
    let (mut commands, mut presentation) = (command::Commands::default(), presentation::Presentation::new());
    for &code in codes {
        paint(vm, engine);
        presentation.command(vm, &mut commands, CommandEvent::Character(code)).unwrap();
    }
    paint(vm, engine);
}

const META: i64 = 1 << 27;

/// What shows after a command: the chip's text and face (nil when hidden),
/// the toasts' texts, and the echo area's message.
const STATE: &str = "(list (and (frame-visible-p (romacs-layer--frame (romacs-eval-chip--buffer)))
                         (with-current-buffer (romacs-eval-chip--buffer)
                           (list (buffer-substring-no-properties (point-min) (point-max)) (get-text-property (point-min) 'face))))
                    (mapcar (lambda (toast) (nth 1 toast)) romacs-layer--toasts)
                    (and (current-message) (substring-no-properties (current-message))))";

#[test]
fn eval_results_show_once_in_a_chip_after_the_sexp() {
    let report = std::thread::Builder::new().stack_size(64 << 20).spawn(move || {
        let mut vm = Vm::new();
        vm.set_frame_geometry(1280, 800, LINE, 10).unwrap();
        vm.set_display_host(Some(Box::new(fonts::Display::new().unwrap())));
        startup::initialize_loadup(&mut vm, startup::Launch::Loadup, None).unwrap();
        let engine: Engine = std::rc::Rc::new(std::cell::RefCell::new(romacs_layout2::LayoutEngine::new().unwrap()));
        vm.set_layout_host(Some(Box::new(measure::Host { engine: engine.clone() })));
        // The GUI renders an echo area, where printing to t goes.
        vm.set_echo_area_host(true);
        let mut report = vec![printed(&mut vm, "(list romacs-complete-mode romacs-eval-chip-mode
            (eval (car (get 'romacs-eval-chip-mode 'standard-value)) t))")];
        vm.eval_source("(progn (switch-to-buffer (get-buffer-create \"chip.el\")) (emacs-lisp-mode)
            (insert \"(+ 1 2)\") (local-set-key (kbd \"C-c p\") 'pp-eval-last-sexp))").unwrap();
        // C-x C-e: the value as GNU prints it, in the chip, after the sexp; no
        // toast; the echo area's message and *Messages* are GNU's.
        keys(&mut vm, &engine, &[24, 5]);
        report.push(printed(&mut vm, STATE));
        report.push(printed(&mut vm, "(let* ((f (romacs-layer--frame (romacs-eval-chip--buffer)))
                 (xy (posn-x-y (posn-at-point (point))))
                 (edges (window-edges nil t nil t)))
            (list (> (car (frame-position f)) (+ (nth 0 edges) (car xy)))
                  (with-current-buffer \"*Messages*\" (save-excursion (goto-char (point-max)) (forward-line -1)
                    (buffer-substring-no-properties (point) (line-end-position))))))"));
        // A command that moves point hides it, and clears the echo area.
        keys(&mut vm, &engine, &[2]);
        report.push(printed(&mut vm, STATE));
        // An error: a tinted chip, the command loop's report claimed. (With
        // GNU's `eval-expression-debug-on-error' t the debugger takes it
        // first, as in GNU.)
        vm.eval_source("(progn (setq eval-expression-debug-on-error nil) (goto-char (point-max)) (insert \"\n(car 1)\"))").unwrap();
        keys(&mut vm, &engine, &[24, 5]);
        report.push(printed(&mut vm, STATE));
        // An edit hides it.
        keys(&mut vm, &engine, &[' ' as i64]);
        report.push(printed(&mut vm, STATE));
        // C-u C-x C-e inserts the value: no chip.
        vm.eval_source("(progn (goto-char (point-max)) (insert \"\n(* 2 3)\"))").unwrap();
        keys(&mut vm, &engine, &[21, 24, 5]);
        report.push(printed(&mut vm, &format!("(list (buffer-substring-no-properties (line-beginning-position) (point)) {STATE})")));
        // M-- C-x C-e prints, as GNU reads the argument.
        vm.eval_source("(progn (goto-char (point-max)) (insert \"\n(* 3 3)\"))").unwrap();
        keys(&mut vm, &engine, &[META | '-' as i64, 24, 5]);
        report.push(printed(&mut vm, STATE));
        keys(&mut vm, &engine, &[2]);
        // A raw byte in an error (its escape in the buffer, so the window
        // holds no raw byte): the chip claims the echo area's printable form.
        vm.eval_source("(progn (goto-char (point-max)) (insert \"\n(error \\\"raw \\\\377\\\")\"))").unwrap();
        keys(&mut vm, &engine, &[24, 5]);
        report.push(printed(&mut vm, "(list (and (frame-visible-p (romacs-layer--frame (romacs-eval-chip--buffer))) t) (length romacs-layer--toasts))"));
        keys(&mut vm, &engine, &[2]);
        // An evaluation that inserts before its sexp: the chip follows the sexp.
        vm.eval_source("(progn (goto-char (point-max)) (insert \"\n(progn (save-excursion (goto-char (point-min)) (insert \\\";; moved\\n\\\")) 5)\"))").unwrap();
        keys(&mut vm, &engine, &[24, 5]);
        report.push(printed(&mut vm, "(let* ((f (romacs-layer--frame (romacs-eval-chip--buffer)))
                 (xy (posn-x-y (posn-at-point (point)))) (edges (window-edges nil t nil t)))
            (list (and (frame-visible-p f) t)
                  (= (+ (cdr (frame-position f)) (frame-parameter f 'child-frame-border-width)) (+ (nth 1 edges) (cdr xy)))))"));
        keys(&mut vm, &engine, &[2]);
        // A message during the evaluation is its own toast; the value is the chip.
        vm.eval_source("(progn (goto-char (point-max)) (insert \"\n(progn (message \\\"working\\\") (sit-for 0) 42)\"))").unwrap();
        keys(&mut vm, &engine, &[24, 5]);
        report.push(printed(&mut vm, STATE));
        keys(&mut vm, &engine, &[2]);
        // C-M-x: the defun's value after the defun.
        vm.eval_source("(progn (goto-char (point-max)) (insert \"\n(defun chip-probe () 1)\") (beginning-of-defun))").unwrap();
        keys(&mut vm, &engine, &[META | 24]);
        report.push(printed(&mut vm, STATE));
        keys(&mut vm, &engine, &[6]);
        // M-:, the value after point in the selected window.
        attach(&mut vm, "(* 6 7)\r");
        keys(&mut vm, &engine, &[META | ':' as i64]);
        report.push(printed(&mut vm, STATE));
        keys(&mut vm, &engine, &[2]);
        // pp: a short value in the chip; a long one in its buffer, no chip.
        vm.eval_source("(progn (goto-char (point-max)) (insert \"\n'(1 2)\"))").unwrap();
        keys(&mut vm, &engine, &[3, 'p' as i64]);
        report.push(printed(&mut vm, STATE));
        keys(&mut vm, &engine, &[2]);
        vm.eval_source("(progn (goto-char (point-max)) (insert \"\n(number-sequence 1 400)\"))").unwrap();
        keys(&mut vm, &engine, &[3, 'p' as i64]);
        report.push(printed(&mut vm, "(list (car (list (and (frame-visible-p (romacs-layer--frame (romacs-eval-chip--buffer))) t)))
            (and (get-buffer-window \"*Pp Eval Output*\") t))"));
        vm.eval_source("(progn (delete-other-windows) (switch-to-buffer \"chip.el\"))").unwrap();
        // pp: an inner buffer display in the form does not decide; the last does.
        vm.eval_source("(progn (goto-char (point-max)) (insert \"\n(progn (pp-display-expression (number-sequence 1 400) \\\"*inner*\\\") 42)\"))").unwrap();
        keys(&mut vm, &engine, &[3, 'p' as i64]);
        report.push(printed(&mut vm, STATE));
        vm.eval_source("(progn (delete-other-windows) (switch-to-buffer \"chip.el\"))").unwrap();
        keys(&mut vm, &engine, &[2]);
        // pp's buffer branch whose `temp-buffer-show-hook' leaves a message: no chip.
        vm.eval_source("(progn (add-hook 'temp-buffer-show-hook (lambda () (message \"q to quit\")))
            (goto-char (point-max)) (insert \"\n(number-sequence 1 400)\"))").unwrap();
        keys(&mut vm, &engine, &[3, 'p' as i64]);
        report.push(printed(&mut vm, "(list (and (frame-visible-p (romacs-layer--frame (romacs-eval-chip--buffer))) t)
            (mapcar (lambda (toast) (nth 1 toast)) romacs-layer--toasts))"));
        vm.eval_source("(progn (setq temp-buffer-show-hook nil) (delete-other-windows) (switch-to-buffer \"chip.el\"))").unwrap();
        // With `edebug-all-defs' a prefix evaluates without instrumenting: a chip.
        vm.eval_source("(progn (require 'edebug) (setq edebug-all-defs t) (goto-char (point-max)) (insert \"\n(defun chip-edebug () 2)\") (beginning-of-defun))").unwrap();
        keys(&mut vm, &engine, &[21, META | 24]);
        report.push(printed(&mut vm, STATE));
        keys(&mut vm, &engine, &[6]);
        // Without a prefix it instruments: \"Edebug: NAME\" is a toast.
        vm.eval_source("(beginning-of-defun)").unwrap();
        keys(&mut vm, &engine, &[META | 24]);
        report.push(printed(&mut vm, STATE));
        vm.eval_source("(setq edebug-all-defs nil)").unwrap();
        keys(&mut vm, &engine, &[6]);
        // A long value is cut to the chip's width.
        vm.eval_source("(progn (goto-char (point-max)) (insert \"\n(make-string 200 ?x)\"))").unwrap();
        keys(&mut vm, &engine, &[24, 5]);
        report.push(printed(&mut vm, "(with-current-buffer (romacs-eval-chip--buffer) (list (string-width (buffer-string)) (string-suffix-p \"…\" (buffer-string))))"));
        keys(&mut vm, &engine, &[2]);
        // The claim is the chip's own text, once: another message (a timer's,
        // one synced during the evaluation) is a toast, and the value's
        // display after it is still claimed; a second display is a toast.
        report.push(printed(&mut vm, "(let ((romacs-layer-toast-max 3))
            (romacs-eval-chip--show \"7\" (cons (selected-window) (copy-marker (point))) nil)
            (romacs-eval-chip--arm \"7\" 'value)
            (dolist (m '(\"other\" \"7\" \"7\")) (romacs-complete--message m))
            (prog1 (mapcar (lambda (toast) (nth 1 toast)) romacs-layer--toasts) (romacs-complete--message nil) (romacs-eval-chip-hide)))"));
        // The error reaches `command-error-function' and *Messages* as without
        // the chip: the same data, context and signalling function.
        vm.eval_source("(progn (defvar chip-errors nil)
            (let ((orig command-error-function))
              (setq command-error-function (lambda (data context signal) (push (list data context signal) chip-errors)
                                             (funcall orig data context signal))))
            (goto-char (point-max)) (insert \"\n(car 2)\"))").unwrap();
        keys(&mut vm, &engine, &[24, 5]);
        vm.eval_source("(romacs-eval-chip-mode -1)").unwrap();
        keys(&mut vm, &engine, &[24, 5]);
        vm.eval_source("(romacs-eval-chip-mode 1)").unwrap();
        report.push(printed(&mut vm, "(list (length chip-errors) (equal (nth 0 chip-errors) (nth 1 chip-errors))
            (with-current-buffer \"*Messages*\" (save-excursion (goto-char (point-max)) (forward-line -1)
              (buffer-substring-no-properties (point) (line-end-position)))))"));
        // A report suppressed: no chip, no toast, and the next command
        // releases the pending claim.
        vm.eval_source("(setq chip-saved-cef command-error-function command-error-function #'ignore)").unwrap();
        keys(&mut vm, &engine, &[24, 5]);
        report.push(printed(&mut vm, &format!("(list {STATE} (and romacs-eval-chip--claim (nth 2 romacs-eval-chip--claim)))")));
        keys(&mut vm, &engine, &[2]);
        report.push(printed(&mut vm, "(prog1 (list (romacs-eval-chip--live-claim) nil) (setq command-error-function chip-saved-cef))"));
        keys(&mut vm, &engine, &[6]);
        // An error an outer handler catches: GNU reports nothing; neither
        // does the chip, and the debugger is not entered.
        vm.eval_source("(progn (defvar chip-debugged nil)
            (defun chip-caught () (interactive)
              (let ((debug-on-error t) (debugger (lambda (&rest _) (setq chip-debugged t))))
                (condition-case nil (eval-last-sexp nil) (error 'caught))))
            (local-set-key (kbd \"C-c c\") 'chip-caught))").unwrap();
        keys(&mut vm, &engine, &[3, 'c' as i64]);
        report.push(printed(&mut vm, &format!("(list {STATE} chip-debugged)")));
        keys(&mut vm, &engine, &[2]);
        // A scroll hides the chip; its timer is the fallback.
        vm.eval_source("(progn (goto-char (point-max)) (insert \"\n(+ 4 4)\")
            (local-set-key (kbd \"C-c w\") (lambda () (interactive) (set-window-start nil 2))))").unwrap();
        keys(&mut vm, &engine, &[24, 5]);
        report.push(printed(&mut vm, "(let ((timer (nth 5 romacs-eval-chip--shown))) (list (timerp timer) (timer--function timer)))"));
        keys(&mut vm, &engine, &[3, 'w' as i64]);
        report.push(printed(&mut vm, STATE));
        vm.eval_source("(set-window-start nil 1)").unwrap();
        // A minibuffer read inside the evaluation: the value's chip once it returns.
        vm.eval_source("(progn (goto-char (point-max)) (insert \"\n(read-string \\\"Say: \\\")\"))").unwrap();
        attach(&mut vm, "hi\r");
        keys(&mut vm, &engine, &[24, 5]);
        report.push(printed(&mut vm, STATE));
        keys(&mut vm, &engine, &[2]);
        // A layer shown, then asked for a position not shown, hides.
        report.push(printed(&mut vm, "(let ((b (get-buffer-create \" chip-layer\")))
            (with-current-buffer b (insert \"layer\"))
            (romacs-layer-show b :position 'center)
            (prog1 (list (frame-visible-p (romacs-layer--frame-of b))
                         (progn (set-window-start nil (save-excursion (goto-char (point-min)) (forward-line 1) (point)))
                                (romacs-layer-show b :position (list 'at (selected-window) (point-min))))
                         (frame-visible-p (romacs-layer--frame-of b)))
              (set-window-start nil (point-min)) (kill-buffer b)))"));
        // Toasts follow the echo area: the next command clears them.
        vm.eval_source("(defun chip-say () (interactive) (message \"hello\"))").unwrap();
        vm.eval_source("(local-set-key (kbd \"C-c s\") 'chip-say)").unwrap();
        keys(&mut vm, &engine, &[3, 's' as i64]);
        report.push(printed(&mut vm, STATE));
        keys(&mut vm, &engine, &[6]);
        report.push(printed(&mut vm, STATE));
        report
    }).unwrap().join().unwrap();
    for line in &report { println!("{line}"); }
    assert_eq!(report, [
        "(t t t)",
        "((\"⇒ 3 (#o3, #x3, ?\\\\C-c)\" romacs-eval-chip) nil \"3 (#o3, #x3, ?\\\\C-c)\")",
        "(t \"3 (#o3, #x3, ?\\\\C-c)\")",
        "(nil nil nil)",
        "((\"⇒ Wrong type argument: listp, 1\" romacs-eval-chip-error) nil \"Wrong type argument: listp, 1\")",
        "(nil nil nil)",
        "(\"(* 2 3)6\" (nil nil nil))",
        "((\"⇒ 9 (#o11, #x9, ?\\\\C-i)\" romacs-eval-chip) nil \"9 (#o11, #x9, ?\\\\C-i)\")",
        "(t 0)",
        "(t t)",
        "((\"⇒ 42 (#o52, #x2a, ?*)\" romacs-eval-chip) nil \"42 (#o52, #x2a, ?*)\")",
        "((\"⇒ chip-probe\" romacs-eval-chip) nil \"chip-probe\")",
        "((\"⇒ 42 (#o52, #x2a, ?*)\" romacs-eval-chip) nil \"42 (#o52, #x2a, ?*)\")",
        "((\"⇒ (1 2)\" romacs-eval-chip) nil \"(1 2)\")",
        "(nil t)",
        "((\"⇒ 42\" romacs-eval-chip) nil \"42\")",
        "(nil (\"q to quit\"))",
        "((\"⇒ chip-edebug\" romacs-eval-chip) nil \"chip-edebug\")",
        "(nil (\"Edebug: chip-edebug\") \"Edebug: chip-edebug\")",
        "(62 t)",
        "(\"7\" \"other\")",
        "(2 t \"eval: Wrong type argument: listp, 2 [2 times]\")",
        "((nil nil nil) error)",
        "(nil nil)",
        "((nil nil nil) nil)",
        "(t romacs-eval-chip-hide)",
        "(nil nil nil)",
        "((\"⇒ \\\"hi\\\"\" romacs-eval-chip) nil \"\\\"hi\\\"\")",
        "(t nil nil)",
        "(nil (\"hello\") \"hello\")",
        "(nil nil nil)",
    ]);
}
