//! Romacs's floating menus (lisp/romacs/romacs-popup.el) on the real --loadup
//! chain with a window system: every kind takes its look and motion from
//! one table, a key grid lays groups out in bands that fit, which-key's
//! popup is a `panel', an eval chip a `cursor', and the host fades a
//! `fade' layer without sliding it.
use romacs_vm_gui::{command, fonts, measure, presentation, startup};
use romacs_vm::{CommandEvent, Vm};

const META: i64 = 1 << 27;

type Engine = std::rc::Rc<std::cell::RefCell<romacs_layout::LayoutEngine>>;

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
        let engine: Engine = std::rc::Rc::new(std::cell::RefCell::new(romacs_layout::LayoutEngine::new().unwrap()));
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
        // keymap-popup's menus are `panel's by default here: one taller than
        // the panel pages with C-M-v, keeps its viewport when it refreshes,
        // and leaves no layer behind once its buffer dies, a mode change first.
        vm.eval_source("(progn (delete-other-windows) (switch-to-buffer \"menu.el\")
            (keymap-popup-define chip-small-map \"Small\" :group \"Few\" \"a\" (\"One\" ignore) \"b\" (\"Two\" ignore))
            (eval `(keymap-popup-define chip-big-map \"Big\" :group \"All\"
                     \"0\" (\"Submenu\" :keymap chip-small-map)
                     \"C-M-v\" (\"Shadowed by paging\" ignore)
                     ,@(apply #'append (mapcar (lambda (c) (list (string c) (list (format \"Entry %c\" c) 'ignore)))
                                               (string-to-list \"abcdefgijklmnoprstuvwxyzABCDEFGIJKLMNOPRSTUVWXYZ\"))))
                  t)
            (defvar chip-layers (hash-table-count romacs-layer--frames))
            (local-set-key (kbd \"C-c k\") (lambda () (interactive) (keymap-popup chip-big-map))))").unwrap();
        keys(&mut vm, &engine, &[3, 'k' as i64]);
        let popup = "(let* ((buf (keymap-popup--popup-buffer)) (f (and buf (romacs-layer--frame-of buf))))
            (list (and f (frame-visible-p f) t) (and f (frame-parameter f 'romacs-popup-kind))
                  (and buf (with-current-buffer buf (line-number-at-pos (window-start (frame-root-window f)))))
                  (and f (< (frame-text-lines f) (with-current-buffer buf (count-lines (point-min) (point-max)))))))";
        report.push(printed(&mut vm, "(eq keymap-popup-backend 'keymap-popup-backend-layer)"));
        report.push(printed(&mut vm, popup));
        keys(&mut vm, &engine, &[META | 22]);
        report.push(printed(&mut vm, popup));
        report.push(printed(&mut vm, &format!("(progn (keymap-popup--refresh (keymap-popup--popup-buffer)) {popup})")));
        keys(&mut vm, &engine, &[META | ('V' as i64 & 31) | (1 << 25)]);
        report.push(printed(&mut vm, popup));
        // A source action under a paging control is not advertised.
        report.push(printed(&mut vm, "(with-current-buffer (keymap-popup--popup-buffer) (and (string-search \"Shadowed\" (buffer-string)) t))"));
        // Paging stops at the last full page and at the top.
        let page_bounds = "(let* ((buf (keymap-popup--popup-buffer)) (w (frame-root-window (romacs-layer--frame-of buf))))
            (with-current-buffer buf (list (line-number-at-pos (window-start w))
                                           (= (line-number-at-pos (window-start w)) (max 1 (- (count-lines (point-min) (point-max)) (window-body-height w) -1))))))";
        keys(&mut vm, &engine, &[META | 22, META | 22, META | 22, META | 22, META | 22]);
        report.push(printed(&mut vm, page_bounds));
        keys(&mut vm, &engine, &[META | ('V' as i64 & 31) | (1 << 25); 5]);
        report.push(printed(&mut vm, page_bounds));
        // A submenu opened from a paged menu starts at its top, and so does
        // its parent again on Back; paging a menu that fits does nothing.
        keys(&mut vm, &engine, &[META | 22, '0' as i64]);
        report.push(printed(&mut vm, popup));
        keys(&mut vm, &engine, &[META | 22]);
        report.push(printed(&mut vm, page_bounds));
        keys(&mut vm, &engine, &['q' as i64]);
        report.push(printed(&mut vm, popup));
        keys(&mut vm, &engine, &['q' as i64]);
        report.push(printed(&mut vm, "(list (keymap-popup--popup-buffer) (= (hash-table-count romacs-layer--frames) chip-layers))"));
        keys(&mut vm, &engine, &[3, 'k' as i64]);
        report.push(printed(&mut vm, "(let ((buf (keymap-popup--popup-buffer)))
            (with-current-buffer buf (text-mode)) (kill-buffer buf)
            (= (hash-table-count romacs-layer--frames) chip-layers))"));
        // Hints: the innermost registered map of `overriding-terminal-local-map'
        // is the target; an inner map's exit gives the outer back; an inhibited
        // or ignored map, or any while a panel shows, has none.
        report.push(printed(&mut vm, "(let* ((outer (define-keymap \"a\" #'forward-char)) (inner (define-keymap \"b\" #'backward-char))
                 (exit-outer (set-transient-map outer t)) (exit-inner (set-transient-map inner t))
                 (r (list (eq (romacs-popup--hint-target) inner))))
            (funcall exit-inner) (push (eq (romacs-popup--hint-target) outer) r)
            (let ((romacs-popup-hints-inhibit t)) (setq exit-inner (set-transient-map (define-keymap \"c\" #'ignore) t)))
            (push (eq (romacs-popup--hint-target) outer) r) (funcall exit-inner)
            (setq exit-inner (set-transient-map universal-argument-map t)) (push (eq (romacs-popup--hint-target) outer) r) (funcall exit-inner)
            (romacs-popup-which-key-show '(1 . 40)) (push (romacs-popup--hint-target) r) (romacs-popup-which-key-hide)
            (push (eq (romacs-popup--hint-target) outer) r)
            (funcall exit-outer) (push (romacs-popup--hint-target) r)
            (nreverse r))"));
        // Labels: one traversal; a closure by its doc's first line, a prefix as
        // +..., a range split around the keys a child rebinds, remaps left out.
        report.push(printed(&mut vm, "(let* ((parent (make-keymap)) (child (make-sparse-keymap)))
            (define-key parent [remap undo] #'ignore)
            (set-char-table-range (nth 1 parent) '(?a . ?e) #'self-insert-command)
            (set-keymap-parent child parent)
            (define-key child \"c\" nil) (define-key child \"x\" (lambda () \"Do the thing.\nMore.\" (interactive)))
            (define-key child \"p\" (make-sparse-keymap)) (define-key child \"w\" #'windmove-left)
            (mapcar (lambda (e) (cons (car e) (string-to-multibyte (cdr e)))) (romacs-popup--hint-entries child)))"));
        // A hint shows the map's keys at the selected window's bottom left; its
        // MESSAGE's toast is retired, and the MESSAGE claimed while it shows.
        vm.eval_source("(defvar chip-hint-hidden nil)").unwrap();
        report.push(printed(&mut vm, "(let* ((map (define-keymap \"<left>\" #'indent-rigidly-left \"<right>\" #'indent-rigidly-right))
                 (exit (set-transient-map map t nil \"Indent with %k\")))
            ;; Its toast shaped narrower than the message: still retired.
            (let ((romacs-layer-toast-width 140)) (romacs-layer-toast (current-message)))
            (romacs-popup--hints-show map)
            (let* ((buf (romacs-popup--hints-buffer)) (f (romacs-layer--frame-of buf)) (e (window-edges nil t nil t)))
              (prog1 (list (frame-parameter f 'romacs-popup-kind) (frame-visible-p f)
                           (with-current-buffer buf (buffer-string))
                           (< (nth 0 e) (car (frame-position f))) (< (+ (cdr (frame-position f)) (frame-pixel-height f)) (nth 3 e))
                           (mapcar (lambda (toast) (nth 1 toast)) romacs-layer--toasts)
                           (romacs-popup--hints-claim (current-message)))
                (funcall exit) (romacs-popup--hints-reconcile)
                (push (frame-visible-p f) chip-hint-hidden))))"));
        report.push(printed(&mut vm, "chip-hint-hidden"));
        // A hint too long for the window is cut where the font paints the
        // `...', and its frame holds the painted row plus the reserved column.
        report.push(printed(&mut vm, "(let* ((map (make-sparse-keymap)))
            (dotimes (i 40) (define-key map (string (+ ?a i)) #'indent-rigidly-right))
            (romacs-popup--hints-show map)
            (let* ((buf (romacs-popup--hints-buffer)) (f (romacs-layer--frame-of buf))
                   (text (with-current-buffer buf (buffer-string)))
                   (painted (romacs-layer--toast-width text)))
              (prog1 (list (string-suffix-p \"...\" text)
                           (<= painted (- (window-body-width (selected-window)) 4))
                           (>= (window-body-width (frame-root-window f)) (1+ painted))
                           (romacs-layer--toast-width \"...\"))
                (romacs-popup--hints-hide))))"));
        report.push(printed(&mut vm, "(let ((map (define-keymap \"o\" #'other-window \"O\" #'ignore)) r)
            (let ((romacs-popup-hints-mode nil)) (romacs-popup-repeat-echo map)
              (push (and romacs-mode-line-repeat (string-match-p \"o O\" romacs-mode-line-repeat) \"\") r))
            (romacs-popup-repeat-echo nil) (romacs-popup-repeat-echo map) (push romacs-mode-line-repeat r)
            (nreverse r))"));
        // A range binds what the first map in the traversal binds it to:
        // a child's range over its parent's key, overlapping inherited
        // ranges once, and single keys amid function keys.
        report.push(printed(&mut vm, "(let* ((parent (define-keymap \"c\" #'forward-char)) (child (make-keymap))
                 (grand (make-keymap)) (mixed (make-sparse-keymap)) (base (make-keymap)))
            (set-char-table-range (nth 1 child) '(?a . ?e) #'backward-char)
            (set-char-table-range (nth 1 grand) '(?c . ?h) #'ignore)
            (set-keymap-parent parent grand) (set-keymap-parent child parent)
            (set-char-table-range (nth 1 base) '(?a . ?e) #'undo)
            (define-key mixed \"d\" #'forward-char) (define-key mixed [f1] #'ignore)
            (define-key mixed \"b\" #'backward-char) (define-key mixed [f2] #'ignore)
            (set-keymap-parent mixed base)
            (mapcar (lambda (map) (mapcar (lambda (e) (cons (car e) (string-to-multibyte (cdr e)))) (romacs-popup--hint-entries map)))
                    (list child mixed)))"));
        // The hints follow `overriding-terminal-local-map' set directly
        // (isearch replaces and restores it), a toggle of the mode, the
        // pause's own timer, and the selected window.
        report.push(printed(&mut vm, "(let* ((map (define-keymap \"o\" #'other-window)) (exit (set-transient-map map t))
                 (f (romacs-layer--frame-of (romacs-popup--hints-buffer))) r)
            (romacs-popup--hints-show map)
            (setq romacs-popup--hints-scheduled nil)
            (let ((saved overriding-terminal-local-map))
              (setq overriding-terminal-local-map (define-keymap \"s\" #'ignore))
              (push romacs-popup--hints-scheduled r) (romacs-popup--hints-reconcile)
              (push (list (romacs-popup--hint-target) romacs-popup--hints-shown (frame-visible-p f) romacs-popup--hints-pending) r)
              (setq overriding-terminal-local-map saved)
              (push romacs-popup--hints-scheduled r) (romacs-popup--hints-reconcile)
              (push (eq (car romacs-popup--hints-pending) map) r))
            (romacs-popup-hints-mode -1) (push romacs-popup--hints-pending r)
            (romacs-popup-hints-mode 1) (push (eq (car romacs-popup--hints-pending) map) r)
            (romacs-popup--hints-cancel) (romacs-popup--hints-due map)
            (push (list (eq romacs-popup--hints-shown map) romacs-popup--hints-pending) r)
            (let ((other (split-window-right)))
              (select-window other) (romacs-popup--hints-reconcile)
              (push (list (eq (car romacs-popup--hints-place) other)
                          (>= (car (frame-position f)) (nth 0 (window-edges other t nil t)))
                          (and (memq #'romacs-popup--hints-follow window-selection-change-functions) t))
                    r)
              (delete-window other))
            (funcall exit) (romacs-popup--hints-reconcile)
            (push (list romacs-popup--hints-shown (memq #'romacs-popup--hints-follow window-selection-change-functions)) r)
            (nreverse r))"));
        // A panel's buffer killed leaves the panels.
        report.push(printed(&mut vm, "(let ((buffer (get-buffer-create \" chip-panel\")))
            (with-current-buffer buffer (insert \"x\"))
            (romacs-popup-show 'panel buffer :lines 1)
            (list (gethash buffer romacs-popup--panels) (progn (kill-buffer buffer) (gethash buffer romacs-popup--panels))))"));
        // `read-key' binds the map around each read: with no hints shown or
        // due and registered maps kept, those bindings schedule nothing.
        report.push(printed(&mut vm, "(let ((map (define-keymap \"z\" #'ignore)))
            (funcall (set-transient-map map)) (romacs-popup--hints-reconcile)
            (setq romacs-popup--hints-scheduled nil)
            (dotimes (_ 5) (let ((overriding-terminal-local-map (make-sparse-keymap))) (ignore)))
            (list (and (gethash map romacs-popup--hint-maps) t) romacs-popup--hints-shown romacs-popup--hints-pending romacs-popup--hints-scheduled))"));
        // repeat-mode turned on takes Romacs's echo over GNU's message, and
        // leaves a user's own.
        report.push(printed(&mut vm, "(let (r)
            (setq repeat-echo-function #'repeat-echo-message) (repeat-mode 1)
            (push repeat-echo-function r) (repeat-mode -1)
            (setq repeat-echo-function #'ignore) (repeat-mode 1)
            (push repeat-echo-function r) (repeat-mode -1)
            (setq repeat-echo-function #'repeat-echo-message)
            (nreverse r))"));
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
        "t",
        "(t panel 1 t)",
        "(t panel 20 t)",
        "(t panel 20 t)",
        "(t panel 1 t)",
        "nil",
        "(20 t)",
        "(1 nil)",
        "(t panel 1 nil)",
        "(1 t)",
        "(t panel 1 t)",
        "(nil t)",
        "t",
        "(t t t t nil t nil)",
        "((\"w\" . \"left\") (\"p\" . \"+...\") (\"x\" . \"Do the thing.\") (\"a..b\" . \"self-insert-command\") (\"d..e\" . \"self-insert-command\"))",
        "(hints t \"<right> indent-rigidly-right   <left> indent-rigidly-left\" t t nil t)",
        "(nil)",
        "(t t t 3)",
        // The status bar's repeat segment: the keys without hints, nothing with them.
        "(\"\" nil)",
        "(((\"a..e\" . \"backward-char\") (\"f..h\" . \"ignore\")) ((\"<f2>\" . \"ignore\") (\"b\" . \"backward-char\") (\"<f1>\" . \"ignore\") (\"d\" . \"forward-char\") (\"a\" . \"undo\") (\"c\" . \"undo\") (\"e\" . \"undo\")))",
        "(t (nil nil nil nil) t t nil t (t nil) (t t t) (nil nil))",
        "(t nil)",
        "(t nil nil nil)",
        "(romacs-popup-repeat-echo ignore)",
    ]);
}

/// A host keyboard holding these keys, then nothing.
struct HostKeys(std::collections::VecDeque<CommandEvent>);
impl romacs_vm::HostInput for HostKeys {
    fn input_pending(&mut self) -> bool { !self.0.is_empty() }
    fn redisplay(&mut self, _: romacs_vm::HostFrame) -> Result<Vec<romacs_vm::WindowEnd>, String> { Ok(vec![]) }
    fn next_event(&mut self, _: Option<std::time::Duration>) -> Result<romacs_vm::HostEvent, String> {
        self.0.pop_front().map(romacs_vm::HostEvent::Key).ok_or_else(|| "keys exhausted".into())
    }
}

/// The key-hint card (popup-layers.md §19.2): which-key's automatic hints
/// for a pending prefix as a content-sized card at the selected pane's
/// bottom right, its bindings resolved as `key-binding' resolves them,
/// labelled through which-key's replacements, paged with the prefix's
/// help key inside the read.
#[test]
fn key_hint_cards() {
    let report = std::thread::Builder::new().stack_size(64 << 20).spawn(move || {
        let mut vm = Vm::new();
        vm.set_frame_geometry(1280, 800, 20, 10).unwrap();
        vm.set_display_host(Some(Box::new(fonts::Display::new().unwrap())));
        startup::initialize_loadup(&mut vm, startup::Launch::Loadup, None).unwrap();
        let engine: Engine = std::rc::Rc::new(std::cell::RefCell::new(romacs_layout::LayoutEngine::new().unwrap()));
        vm.set_layout_host(Some(Box::new(measure::Host { engine: engine.clone() })));
        vm.set_echo_area_host(true);
        vm.eval_source("(progn (require 'which-key)
            (setq which-key-popup-type 'custom which-key-custom-show-popup-function #'romacs-popup-which-key-show
                  which-key-custom-hide-popup-function #'romacs-popup-which-key-hide
                  which-key-custom-popup-max-dimensions-function #'romacs-popup-which-key-max-dimensions)
            (which-key-mode 1) (switch-to-buffer (get-buffer-create \"card.txt\")) (insert \"text\\n\"))").unwrap();
        let mut report = vec![];
        // Precedence: within the local map a child's nil hides its parent's
        // binding, which falls through to the global one; `ignore' wins and
        // hides; prefixes first, then a mode's commands, then global ones.
        report.push(printed(&mut vm, "(let ((global (make-sparse-keymap)) (parent (make-sparse-keymap)) (local (make-sparse-keymap))
                  (sub (make-sparse-keymap)) (old (current-global-map)))
            (define-key global [f12 ?a] #'forward-char) (define-key global [f12 ?b] #'backward-char)
            (define-key global [f12 ?c] #'next-line) (define-key sub \"x\" #'forward-word) (define-key global [f12 ?p] sub)
            (define-key parent [f12 ?b] #'kill-line) (define-key parent [f12 ?d] #'undo)
            (set-keymap-parent local parent) (define-key local [f12 ?b] nil) (define-key local [f12 ?c] #'ignore)
            (unwind-protect
                (with-temp-buffer
                  (use-global-map global) (use-local-map local)
                  (mapcar (lambda (e) (list (string-to-multibyte (car e)) (string-to-multibyte (substring-no-properties (nth 1 e))) (nth 2 e) (nth 3 e)))
                          (romacs-popup--card-entries [f12])))
              (use-global-map old)))"));
        // Labels: a menu-item's name, a which-key replacement verbatim, one
        // that hides, GNU's own prefix map by name, a name humanised.
        report.push(printed(&mut vm, "(let ((map (make-sparse-keymap)) (old (current-global-map))
                  (which-key-replacement-alist (cons '((nil . \"\\\\`yank\\\\'\") . (lambda (_) nil)) which-key-replacement-alist)))
            (define-key map [f11 ?m] '(menu-item \"Do the menu thing\" forward-word))
            (define-key map [f11 ?r] #'kill-ring-save) (define-key map [f11 ?s] #'yank)
            (define-key map [f11 ?4] ctl-x-4-map) (define-key map [f11 ?w] 'card-thing-command)
            (which-key-add-key-based-replacements \"<f11> r\" \"copy\")
            (unwind-protect
                (with-temp-buffer
                  (use-local-map map)
                  (mapcar (lambda (e) (cons (string-to-multibyte (car e)) (string-to-multibyte (substring-no-properties (nth 1 e))))) (romacs-popup--card-entries [f11])))
              (use-global-map old)))"));
        // Pages: one when they fit; else pages whose last row says how many
        // more and the help key; none when a page holds no entry.
        report.push(printed(&mut vm, "(let ((entries (mapcar (lambda (i) (list (format \"%c\" (+ ?a i)) (format \"command-%d\" i) nil 'global i)) (number-sequence 0 29))))
            (list (length (romacs-popup--card-pages (seq-take entries 4) 5 40))
                  (let ((pages (romacs-popup--card-pages entries 5 40)))
                    (list (length pages) (substring-no-properties (car (last (split-string (car pages) \"\\n\"))))))
                  (romacs-popup--card-pages entries 1 10)))"));
        // GNU's prefixes named: an ESC map `meta', the anonymous C-x 8 map by
        // its key, a map by its variable, GNU's own by name.
        report.push(printed(&mut vm, "(let ((entries (romacs-popup--card-entries [24])))
            (mapcar (lambda (k) (cons k (string-to-multibyte (substring-no-properties (nth 1 (assoc k entries)))))) '(\"ESC\" \"8\" \"v\" \"4\" \"r\")))"));
        // The card's window is wider than its widest row, so no row meets
        // the truncation glyph; no label is cut where the columns fit.
        report.push(printed(&mut vm, "(progn (setq which-key--automatic-display t) (romacs-popup--card-record [24] nil) (romacs-popup--card-render [24] 0)
            (let* ((w (frame-root-window (romacs-layer--frame-of which-key--buffer)))
                   (lines (with-current-buffer which-key--buffer (split-string (buffer-substring-no-properties (point-min) (point-max)) \"\\n\"))))
              (prog1 (list (>= (window-max-chars-per-line w) (seq-max (mapcar #'string-width lines)))
                           (seq-some (lambda (l) (and (string-search \"...\" l) (not (string-search \"more\" l)))) lines))
                (romacs-popup-which-key-hide))))"));
        // which-key's automatic hint for C-x: the card, titled C-x, at the
        // selected pane's bottom right 12 px in, which-key told the prefix
        // shows; hidden by the next key's read, its state kept.
        report.push(printed(&mut vm, "(progn (setq which-key--automatic-display t)
            (which-key--create-buffer-and-show (kbd \"C-x\"))
            (let* ((f (romacs-layer--frame-of which-key--buffer)) (e (window-edges (selected-window) t nil t)) (pos (frame-position f)))
              (list (frame-parameter f 'romacs-popup-kind) (frame-visible-p f)
                    (string-to-multibyte (with-current-buffer which-key--buffer (buffer-substring-no-properties (point-min) (line-end-position))))
                    (equal (which-key--pages-prefix which-key--pages-obj) (kbd \"C-x\"))
                    (and (memq #'romacs-popup--card-hide romacs--key-read-functions) t)
                    (= (+ (car pos) (frame-pixel-width f)) (- (nth 2 e) 12)) (>= (car pos) (+ (nth 0 e) 12))
                    (<= (+ (cdr pos) (frame-pixel-height f)) (- (nth 3 e) 12))
                    (> (length (nth 2 romacs-popup--card)) 1))))"));
        report.push(printed(&mut vm, "(progn (run-hook-with-args 'romacs--key-read-functions ?a)
            (list (frame-visible-p (romacs-layer--frame-of which-key--buffer)) (car romacs-popup--card)
                  (memq #'romacs-popup--card-hide romacs--key-read-functions)))"));
        // The help key pages it, wrapping; which-key's hide ends it.
        report.push(printed(&mut vm, "(let (pages)
            (dotimes (_ (1+ (length (nth 2 romacs-popup--card))))
              (romacs-popup--card-help [24] ?\\C-h) (push (nth 1 romacs-popup--card) pages))
            (prog1 (list (nreverse pages) (frame-visible-p (romacs-layer--frame-of which-key--buffer)))
              (romacs-popup-which-key-hide)))"));
        report.push(printed(&mut vm, "(list romacs-popup--card (frame-visible-p (romacs-layer--frame-of which-key--buffer)))"));
        // Over point at the bottom right, the card goes bottom left.
        report.push(printed(&mut vm, "(let* ((w (selected-window)) (bounds (romacs-popup--card-bounds w)) (e (window-edges w t nil t)))
            (cl-letf (((symbol-function 'posn-at-point)
                       (lambda (&rest _) (list w 1 (cons (- (nth 2 e) (nth 0 e) 30) (- (nth 3 e) (nth 1 e) 30)) 0))))
              (list (= (car (romacs-popup--card-place w bounds 200 100)) (nth 0 bounds))
                    ;; Point past a truncated row's right edge: its posn is the
                    ;; row's last column (GNU's too), which the card, inset
                    ;; within the pane, never covers at the bottom right.
                    (cl-letf (((symbol-function 'posn-at-point)
                               (lambda (&rest _) (list w 1 (cons (- (nth 2 e) (nth 0 e) 10) 40) 0))))
                      (let ((at (romacs-popup--card-place w bounds 200 (- (nth 3 bounds) (nth 1 bounds) 20))))
                        (and (= (car at) (- (nth 2 bounds) 200)) (<= (+ (car at) 200) (+ (nth 0 e) (- (nth 2 e) (nth 0 e) 10))))))
                    (= (car (cl-letf (((symbol-function 'posn-at-point) (lambda (&rest _) nil)))
                              (romacs-popup--card-place w bounds 200 100)))
                       (- (nth 2 bounds) 200)))))"));
        // The reader: an unbound help key after a prefix runs
        // `prefix-help-command' with the whole sequence (GNU's fallback) when
        // the card is not the display; with the card, the read goes on and a
        // macro recorded while paging holds no help key.
        // Host keys, read by the command loop as the GUI's are.
        let host = |vm: &mut Vm, codes: &[i64]| {
            vm.set_host_input(Some(Box::new(HostKeys(codes.iter().map(|&c| CommandEvent::Character(c)).collect()))));
            paint(vm, &engine);
            command::run(vm).unwrap();
            paint(vm, &engine);
        };
        vm.eval_source("(progn (defvar card-help-keys nil) (defvar card-took nil) (which-key-mode -1)
            (setq prefix-help-command (lambda () (interactive) (setq card-help-keys (this-command-keys-vector))))
            (advice-add 'romacs-popup--card-help :after (lambda (&rest _) (setq card-took (car romacs-popup--card)))))").unwrap();
        host(&mut vm, &[24, 8]);
        report.push(printed(&mut vm, "(list card-help-keys card-took)"));
        vm.eval_source("(which-key-mode 1)").unwrap();
        host(&mut vm, &[24, '(' as i64]);
        host(&mut vm, &[24, 8, 'h' as i64]);
        report.push(printed(&mut vm, "(list last-command card-took (frame-visible-p (romacs-layer--frame-of which-key--buffer)))"));
        host(&mut vm, &[24, ')' as i64]);
        report.push(printed(&mut vm, "(key-description last-kbd-macro)"));
        // A help key a translation produced is not the card's: GNU's
        // fallback (F6 -> C-h); a help key part-way into a translation ends
        // the sequence as GNU's does ([C-h x] -> [h]: x is the next command);
        // a shifted help key comes back shifted (C-x S-C-h).
        let named = |vm: &mut Vm, events: Vec<CommandEvent>| {
            vm.set_host_input(Some(Box::new(HostKeys(events.into()))));
            paint(vm, &engine);
            command::run(vm).unwrap();
            paint(vm, &engine);
        };
        vm.eval_source("(progn (setq card-took nil card-help-keys nil)
            (setq prefix-help-command (lambda () (interactive) (setq card-help-keys (list (this-command-keys-vector) last-command-event))))
            (define-key key-translation-map [f6] [8]))").unwrap();
        named(&mut vm, vec![CommandEvent::Character(24), CommandEvent::Named("f6".into())]);
        report.push(printed(&mut vm, "(prog1 (list card-help-keys card-took) (define-key key-translation-map [f6] nil) (setq card-took nil card-help-keys nil))"));
        vm.eval_source("(progn (define-key key-translation-map [8 ?x] [?h]) (erase-buffer))").unwrap();
        // Two commands from one keyboard: C-x C-h, then the x it left unread.
        vm.set_host_input(Some(Box::new(HostKeys(vec![CommandEvent::Character(24), CommandEvent::Character(8), CommandEvent::Character('x' as i64)].into()))));
        for _ in 0..2 { paint(&mut vm, &engine); command::run(&mut vm).unwrap(); }
        report.push(printed(&mut vm, "(prog1 (list card-help-keys card-took (buffer-string)) (define-key key-translation-map [8 ?x] nil) (setq card-took nil card-help-keys nil))"));
        // No card fits: the help key is not taken, GNU's fallback runs.
        vm.eval_source("(progn (advice-add 'romacs-popup--card-layout :override #'ignore '((name . card-no-fit)))
            (setq prefix-help-command (lambda () (interactive) (setq card-help-keys (list (this-command-keys-vector) last-command-event)))))").unwrap();
        named(&mut vm, vec![CommandEvent::Character(24), CommandEvent::Character(8)]);
        report.push(printed(&mut vm, "(prog1 (list card-help-keys card-took (car romacs-popup--card))
            (advice-remove 'romacs-popup--card-layout 'card-no-fit) (setq card-took nil card-help-keys nil))"));
        vm.eval_source("(which-key-mode -1)").unwrap();
        vm.eval_source("(setq prefix-help-command (lambda () (interactive) (setq card-help-keys (list (this-command-keys-vector) last-command-event))))").unwrap();
        named(&mut vm, vec![CommandEvent::Character(24), CommandEvent::Character(8 | (1 << 25))]);
        report.push(printed(&mut vm, "(list card-help-keys card-took
            (let ((unread-command-events (list 24 (+ 8 (ash 1 25))))) (read-key-sequence nil)))"));
        // Geometry: a page's footer is inside its rows; a key wider than the
        // room gives no layout (the frame's corner, then no card).
        report.push(printed(&mut vm, "(let ((entries (mapcar (lambda (i) (list (format \"%c\" (+ ?a i)) \"cmd\" nil 'global i)) (number-sequence 0 9))))
            (list (romacs-popup--card-pages (seq-take entries 4) 1 70)
                  (seq-max (mapcar (lambda (p) (length (split-string p \"\\n\"))) (romacs-popup--card-pages entries 2 70)))
                  (romacs-popup--card-pages (list (list \"<XF86AudioLowerVolume>\" \"lower\" nil 'global 0) (list \"a\" \"b\" nil 'global 1)) 1 10)))"));
        report
    }).unwrap().join().unwrap();
    for line in &report { println!("{line}"); }
    assert_eq!(report, [
        "((\"p\" \"prefix\" t global) (\"d\" \"undo\" nil mode) (\"a\" \"forward-char\" nil global) (\"b\" \"backward-char\" nil global))",
        "((\"4\" . \"other-window\") (\"m\" . \"Do the menu thing\") (\"r\" . \"copy\") (\"w\" . \"card-thing\"))",
        "(1 (4 \"... 22 more · C-h\") nil)",
        "((\"ESC\" . \"meta\") (\"8\" . \"char\") (\"v\" . \"vc\") (\"4\" . \"other-window\") (\"r\" . \"register\"))",
        "(t nil)",
        "(card t \"C-x\" t t t t t t)",
        "(nil [24] nil)",
        "((1 2 0 1) t)",
        "(nil nil)",
        "(t t t)",
        // which-key off: GNU's fallback runs `prefix-help-command' with the
        // whole sequence; the card takes nothing.
        "([24 8] nil)",
        // The card takes the help key: the read goes on for C-x, C-x h runs,
        // the card is gone, and the macro holds no help key.
        "(mark-whole-buffer [24] nil)",
        "\"C-x h\"",
        "(([24 8] 8) nil)",
        "(([24 8] 8) nil \"x\")",
        "(([24 8] 8) nil nil)",
        // GNU 32 (a keyboard macro, then `read-key-sequence'): the
        // command's keys [24 8], its event and the sequence shifted.
        "(([24 8] 33554440) nil [24 33554440])",
        "(nil 2 nil)",
    ]);
}
