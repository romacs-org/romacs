//! Romacs's floating menus (rlisp/romacs-popup.el) on the real --loadup
//! chain with a window system: every kind takes its look and motion from
//! one table, a key grid lays groups out in bands that fit, which-key's
//! popup is a `panel', an eval chip a `cursor', and the host fades a
//! `fade' layer without sliding it.
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
use romacs_vm::{CommandEvent, Vm};

fn snapshot(vm: &mut Vm, _: bool, width: u32, height: u32) -> Result<romacs_layout_input::FrameSnapshot, String> {
    vm.frame_snapshot(width, height)
}

type Engine = std::rc::Rc<std::cell::RefCell<romacs_layout2::LayoutEngine>>;

fn printed(vm: &mut Vm, source: &str) -> String {
    vm.eval_source(source).and_then(|v| v.display()).unwrap_or_else(|e| format!("VM-ERROR {e}"))
}

fn paint(vm: &mut Vm, engine: &Engine) {
    let snapshot = vm.frame_snapshot(1280, 800).unwrap();
    let (_, ends) = engine.borrow_mut().layout_frame(&snapshot, 1, None).unwrap();
    vm.display_apply_layout(&ends).unwrap();
}

fn keys(vm: &mut Vm, engine: &Engine, codes: &[i64]) {
    let (mut commands, mut presentation) = (command::Commands::default(), presentation::Presentation::new());
    for &code in codes {
        paint(vm, engine);
        presentation.command(vm, &mut commands, CommandEvent::Character(code)).unwrap();
    }
    paint(vm, engine);
}

#[test]
fn menus_share_one_look_and_motion() {
    let report = std::thread::Builder::new().stack_size(64 << 20).spawn(move || {
        let mut vm = Vm::new();
        vm.set_frame_geometry(1280, 800, 20, 10).unwrap();
        vm.set_display_host(Some(Box::new(fonts::Display::new().unwrap())));
        startup::initialize_loadup(&mut vm, startup::Launch::Loadup, None).unwrap();
        let engine: Engine = std::rc::Rc::new(std::cell::RefCell::new(romacs_layout2::LayoutEngine::new().unwrap()));
        vm.set_layout_host(Some(Box::new(measure::Host { engine: engine.clone() })));
        vm.set_echo_area_host(true);
        let mut report = vec![];
        // A key grid: blocks aligned on their widest key, side by side while
        // they fit, the next band below; faces on keys, titles, descriptions.
        report.push(printed(&mut vm, "(let ((g (romacs-popup-key-grid
                  '((\"Navigate\" (\"n\" . \"Next\") (\"RET\" . \"Open\")) (\"Chats\" (\"c\" . \"New chat\")) (\"Tools\" (\"b\" . \"Browse\")))
                  24)))
            (list (string-to-multibyte (substring-no-properties g)) (get-text-property 0 'face g)
                  (get-text-property (string-search \"RET\" g) 'face g) (get-text-property (string-search \"Open\" g) 'face g)))"));
        // which-key's custom popup is a `panel': glass, slide, the bottom.
        report.push(printed(&mut vm, "(progn (setq which-key--buffer (get-buffer-create \" *which-key*\"))
            (with-current-buffer which-key--buffer (insert \"C-x f  find-file\"))
            (romacs-popup-which-key-show '(1 . 40))
            (let ((f (romacs-layer--frame-of which-key--buffer)))
              (prog1 (list (frame-visible-p f) (romacs-popup-panel-p which-key--buffer)
                           (frame-parameter f 'romacs-popup-kind) (frame-parameter f 'romacs-layer-radius)
                           (frame-parameter f 'child-frame-border-width) (frame-parameter f 'romacs-layer-motion)
                           (frame-parameter f 'romacs-layer-edge)
                           (= (+ (cdr (frame-position f)) (frame-pixel-height f)) (romacs-chrome-bottom)))
                (romacs-popup-which-key-hide))))"));
        report.push(printed(&mut vm, "(romacs-popup-panel-p which-key--buffer)"));
        // A which-key page as wide as it is told it may be fits its panel's
        // window without continuing, glass or not.
        report.push(printed(&mut vm, "(mapcar (lambda (glass)
              (let* ((romacs-layer-glass glass) (columns (cdr (romacs-popup-which-key-max-dimensions nil))))
                (with-current-buffer which-key--buffer (erase-buffer) (insert (make-string columns ?k)))
                (romacs-popup-which-key-show '(1 . 0))
                (prog1 (>= (window-max-chars-per-line (frame-root-window (romacs-layer--frame-of which-key--buffer))) columns)
                  (romacs-popup-which-key-hide))))
            '(t nil))"));
        // A panel showing or hiding runs `romacs-popup-panel-functions'.
        report.push(printed(&mut vm, "(let (seen) (let ((romacs-popup-panel-functions (list (lambda () (push (romacs-popup-panel-p which-key--buffer) seen)))))
            (romacs-popup-which-key-show '(1 . 40)) (romacs-popup-which-key-hide)) (nreverse seen))"));
        // An eval chip is a `cursor' menu that fades without sliding: the
        // host's snapshot has its animation reduced to fades.
        vm.eval_source("(progn (setq romacs-layer-animation t) (switch-to-buffer (get-buffer-create \"menu.el\")) (emacs-lisp-mode) (insert \"(+ 1 2)\"))").unwrap();
        keys(&mut vm, &engine, &[24, 5]);
        report.push(printed(&mut vm, "(let ((f (romacs-layer--frame-of (romacs-eval-chip--buffer))))
            (list (frame-visible-p f) (frame-parameter f 'romacs-popup-kind) (frame-parameter f 'romacs-layer-radius)
                  (frame-parameter f 'child-frame-border-width) (frame-parameter f 'romacs-layer-motion)))"));
        let children = vm.frame_snapshot(1280, 800).unwrap().children;
        report.push(format!("{:?}", children.iter().map(|c| c.animation).collect::<Vec<_>>()));
        // With glass off a menu is GNU's square child frame.
        report.push(printed(&mut vm, "(let ((romacs-layer-glass nil) (b (get-buffer-create \" square\")))
            (with-current-buffer b (insert \"x\"))
            (let ((f (romacs-popup-show 'palette b)))
              (prog1 (list (frame-parameter f 'romacs-layer-radius) (frame-parameter f 'child-frame-border-width))
                (kill-buffer b))))"));
        // Toasts stack from the selected window's top-right corner, inside its
        // pane, and follow the selection; `frame' puts them in the frame's corner.
        vm.eval_source("(progn (delete-other-windows) (switch-to-buffer \"menu.el\") (split-window-right) (romacs-layer-toast \"hello\"))").unwrap();
        paint(&mut vm, &engine);
        let toast = "(let* ((f (romacs-layer--frame-of (car (car romacs-layer--toasts))))
                   (e (window-edges (selected-window) t nil t))
                   (right (+ (car (frame-position f)) (frame-pixel-width f))))
              (list (frame-visible-p f) (< (nth 0 e) (car (frame-position f))) (<= right (nth 2 e)) (> right (- (nth 2 e) 40))
                    (< (nth 1 e) (cdr (frame-position f)) (+ (nth 1 e) 40))))";
        report.push(printed(&mut vm, toast));
        // The selection changing without a key (a timer's `select-window'):
        // redisplay's window-change functions move the toasts along.
        vm.eval_source("(progn (select-window (next-window)) (romacs--run-window-change-functions))").unwrap();
        paint(&mut vm, &engine);
        report.push(printed(&mut vm, toast));
        // A narrow pane keeps its toast inside it; one too narrow for a
        // toast sends it to the frame's corner.
        vm.eval_source("(progn (delete-other-windows) (split-window-right -34) (other-window 1) (romacs-layer-toast-clear)
            (romacs-layer-toast (make-string 120 ?x)) (romacs--run-window-change-functions))").unwrap();
        paint(&mut vm, &engine);
        report.push(printed(&mut vm, toast));
        vm.eval_source("(progn (delete-other-windows) (split-window-right -14) (other-window 1) (romacs-layer-toast-clear)
            (romacs-layer-toast \"narrow\"))").unwrap();
        paint(&mut vm, &engine);
        report.push(printed(&mut vm, "(let ((f (romacs-layer--frame-of (car (car romacs-layer--toasts)))))
            (= (+ (car (frame-position f)) (frame-pixel-width f) 12 (frame-internal-border-width)) (frame-native-width)))"));
        vm.eval_source("(progn (delete-other-windows) (split-window-right) (romacs-layer-toast-clear) (romacs-layer-toast \"hello\"))").unwrap();
        paint(&mut vm, &engine);
        report.push(printed(&mut vm, "(let ((romacs-layer-toast-position 'frame)) (romacs-layer--toast-stack)
            (let ((f (romacs-layer--frame-of (car (car romacs-layer--toasts)))))
              (prog1 (= (+ (car (frame-position f)) (frame-pixel-width f) 12 (frame-internal-border-width)) (frame-native-width))
                (romacs-layer-toast-clear))))"));
        report.push(printed(&mut vm, "(list (memq #'romacs-layer--toast-follow window-selection-change-functions) romacs-layer--toasts)"));
        // The error of a command at point is a chip there, not a toast;
        // *Messages* logs it as GNU; a quit is not a chip.
        vm.eval_source("(progn (delete-other-windows) (goto-char (point-max)))").unwrap();
        keys(&mut vm, &engine, &[14]);
        report.push(printed(&mut vm, "(list (and (frame-visible-p (romacs-layer--frame (romacs-eval-chip--buffer)))
                  (with-current-buffer (romacs-eval-chip--buffer) (list (buffer-string) (get-text-property 1 'face))))
              (mapcar (lambda (toast) (nth 1 toast)) romacs-layer--toasts)
              (with-current-buffer \"*Messages*\" (save-excursion (goto-char (point-max)) (forward-line -1)
                (buffer-substring-no-properties (point) (line-end-position)))))"));
        keys(&mut vm, &engine, &[2]);
        // Long and deep error data: claimed as the reporter prints it, with
        // `print-length' and `print-level' 10, so still a chip.
        vm.eval_source("(progn (defun chip-long () (interactive) (aref (number-sequence 1 30) 0))
            (defun chip-deep () (interactive) (aref '(((((((((((((x))))))))))))) 0))
            (local-set-key (kbd \"C-c l\") 'chip-long) (local-set-key (kbd \"C-c d\") 'chip-deep))").unwrap();
        for k in ['l', 'd'] {
            keys(&mut vm, &engine, &[3, k as i64]);
            report.push(printed(&mut vm, "(list (and (frame-visible-p (romacs-layer--frame (romacs-eval-chip--buffer))) t)
                (mapcar (lambda (toast) (nth 1 toast)) romacs-layer--toasts))"));
            keys(&mut vm, &engine, &[2]);
        }
        // A process filter's error, which `command-error-function' reports
        // too, is not the command's: a toast, not a chip.
        vm.eval_source("(progn (defun chip-filter () (interactive)
              (let ((p (make-process :name \"chip-filter\" :command '(\"printf\" \"x\") :connection-type 'pipe
                                     :filter (lambda (_p _s) (error \"Filter boom\")))))
                (while (process-live-p p) (accept-process-output p 0.05))
                (accept-process-output nil 0.05)))
            (local-set-key (kbd \"C-c f\") 'chip-filter))").unwrap();
        keys(&mut vm, &engine, &[3, 'f' as i64]);
        report.push(printed(&mut vm, "(list (and (frame-visible-p (romacs-layer--frame (romacs-eval-chip--buffer))) t)
            (mapcar (lambda (toast) (nth 1 toast)) romacs-layer--toasts))"));
        keys(&mut vm, &engine, &[7]);
        report.push(printed(&mut vm, "(list (frame-visible-p (romacs-layer--frame (romacs-eval-chip--buffer))) (romacs-eval-chip--live-claim))"));
        report
    }).unwrap().join().unwrap();
    for line in &report { println!("{line}"); }
    assert_eq!(report, [
        "(\"Navigate    Chats\\nn    Next   c  New chat\\nRET  Open\\n\\nTools\\nb  Browse\" romacs-popup-title romacs-popup-key romacs-popup-description)",
        "(t t panel 8 10 slide bottom t)",
        "nil",
        "(t t)",
        "(t nil)",
        "(t cursor 6 4 fade)",
        "[Reduced]",
        "(nil 1)",
        "(t t t t t)",
        "(t t t t t)",
        "(t t t t t)",
        "t",
        "t",
        "(nil nil)",
        "((\"⇒ End of buffer\" romacs-eval-chip-error) nil \"line-move-visual: End of buffer\")",
        "(t nil)",
        "(t nil)",
        "(nil (\"error in process filter: Filter boom\"))",
        "(nil nil)",
    ]);
}
