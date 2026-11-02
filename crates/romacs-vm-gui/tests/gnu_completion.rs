//! GNU's minibuffer completion in the `--loadup` GUI, through the GUI's
//! keyboard (`romacs_vm::HostInput`) on the real chain: Romacs's completion
//! defaults are the options' standard values (with no window system,
//! fido-vertical, not `romacs-complete-mode'); M-x, C-x C-f and C-x b read
//! with GNU's `execute-extended-command`, `read-file-name` and `read-buffer`;
//! icomplete shows the candidates in the mini-window as an overlay string,
//! the mini-window grows to hold them and shrinks back when the minibuffer
//! exits; GNU's `find-file-noselect' visits. The GUI delivers Return as C-j
//! (DIVERGENCES VM2-HOSTIN1).
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
use romacs_vm::{CommandEvent, HostEvent, HostFrame, HostInput, Vm};
use std::{cell::RefCell, collections::VecDeque, rc::Rc};

fn snapshot(vm: &mut Vm, _: bool, width: u32, height: u32) -> Result<romacs_layout_input::FrameSnapshot, String> {
    vm.frame_snapshot(width, height)
}

const LINE: u32 = 20;

/// Each prompt redisplay's mini-window: its text and height.
type Frames = Rc<RefCell<Vec<(String, u32)>>>;

struct Keys {
    keys: VecDeque<HostEvent>,
    frames: Frames,
    /// Keys read so far, and after how many the rest count as typed ahead.
    read: usize,
    ahead_after: usize,
}
impl HostInput for Keys {
    fn input_pending(&mut self) -> bool {
        self.read > self.ahead_after && !self.keys.is_empty()
    }
    fn redisplay(&mut self, frame: HostFrame) -> Result<Vec<romacs_vm::WindowEnd>, String> {
        let mini = &frame.frame.windows.last().unwrap().snapshot;
        self.frames.borrow_mut().push((mini.text.utf8.clone(), mini.height));
        Ok(vec![])
    }
    fn next_event(&mut self, _: Option<std::time::Duration>) -> Result<HostEvent, String> {
        self.read += 1;
        match self.keys.pop_front() {
            Some(key) => Ok(key),
            // A typed-ahead script quits whatever still reads, so a wrong
            // command fails the test instead of waiting forever.
            None if self.ahead_after != usize::MAX => Ok(HostEvent::Key(CommandEvent::Character(7))),
            None => Err("completion host script exhausted".into()),
        }
    }
}

/// Attach a keyboard that types TEXT, Return as the GUI sends it (C-j).
fn attach(vm: &mut Vm, text: &str) -> Frames {
    attach_typing_ahead(vm, text, usize::MAX)
}

/// `attach', the keys after the first AFTER pending while each is read,
/// as when typing outruns the commands.
fn attach_typing_ahead(vm: &mut Vm, text: &str, after: usize) -> Frames {
    let frames = Frames::default();
    let keys = text.chars().map(|c| HostEvent::Key(CommandEvent::Character(c as i64))).collect();
    vm.set_host_input(Some(Box::new(Keys { keys, frames: frames.clone(), read: 0, ahead_after: after })));
    frames
}

fn printed(vm: &mut Vm, source: &str) -> String {
    vm.eval_source(source).and_then(|v| v.display()).unwrap_or_else(|e| format!("VM-ERROR {e}"))
}

fn quoted(text: &str) -> String {
    serde_json::to_string(text).unwrap()
}

/// Run the command of KEYS as the GUI's outer loop does.
fn keys(vm: &mut Vm, codes: &[i64]) -> Option<romacs_vm::CommandOutcome> {
    let (mut commands, mut presentation) = (command::Commands::default(), presentation::Presentation::new());
    let mut outcome = None;
    for &code in codes {
        outcome = presentation.command(vm, &mut commands, CommandEvent::Character(code)).unwrap();
    }
    outcome
}

/// The tallest mini-window a prompt showed, and the texts it showed.
fn shown(frames: &Frames) -> (u32, Vec<String>) {
    let frames = frames.borrow();
    (frames.iter().map(|f| f.1).max().unwrap_or(0), frames.iter().map(|f| f.0.clone()).collect())
}

fn mini_height(vm: &mut Vm) -> u32 {
    vm.frame_snapshot(1280, 800).unwrap().windows.last().unwrap().snapshot.height
}

#[test]
fn gnu_completion_reads_commands_files_and_buffers_through_icomplete() {
    let root = std::env::temp_dir().join(format!("romacs-gnu-completion-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(root.join("work/sub")).unwrap();
    std::fs::create_dir_all(root.join("home/config")).unwrap();
    std::fs::write(root.join("work/sub/notes.txt"), "notes\n").unwrap();
    for dir in ["a", "b"] {
        std::fs::create_dir_all(root.join("work/uq").join(dir)).unwrap();
        std::fs::write(root.join("work/uq").join(dir).join("notes.org"), "* note\n").unwrap();
    }
    std::fs::write(root.join("work/uq/raw.txt"), "raw\n").unwrap();
    for name in ["zq-a-b.txt", "xzqab.txt", "zqab.txt"] {
        std::fs::write(root.join("work").join(name), "zq\n").unwrap();
    }
    let work = format!("{}/", root.join("work").display());
    let home = root.join("home");
    let report = std::thread::Builder::new().stack_size(64 << 20).spawn(move || {
        let mut vm = Vm::new();
        vm.set_frame_geometry(1280, 800, LINE, 10).unwrap();
        // The session's user directory is a private one, never the user's own.
        vm.eval_source(&format!("(setq process-environment (append (list {} {}) process-environment))",
            quoted(&format!("HOME={}", home.display())),
            quoted(&format!("XDG_CONFIG_HOME={}", home.join("config").display())))).unwrap();
        startup::initialize_loadup(&mut vm, startup::Launch::Loadup, None).unwrap();
        let mut report = vec![];
        // Romacs's defaults are standard values, set through their :set.
        report.push(printed(&mut vm, "(list completion-styles completion-ignore-case completions-format completions-sort
            completion-category-overrides icomplete-compute-delay
            (equal (eval (car (get 'completions-sort 'standard-value)) t) completions-sort)
            fido-vertical-mode fido-mode icomplete-mode (eval (car (get 'fido-vertical-mode 'standard-value)) t)
            (lookup-key icomplete-fido-mode-map (kbd \"TAB\")) resize-mini-windows)"));
        // The styles are standard values, which Custom returns to.
        report.push(printed(&mut vm, "(let ((completion-styles nil)) (custom-reevaluate-setting 'completion-styles) completion-styles)"));
        // Fido completes with fzf: its first candidate is fzf's best.
        vm.eval_source("(progn (dolist (c '(xzqab zq-alpha-beta zqab-x)) (defalias c (lambda () (interactive) (setq fzf-probe-ran c))))
            (add-hook 'minibuffer-setup-hook (lambda () (add-hook 'post-command-hook
              (lambda () (setq fzf-probe (list (completion--styles (completion--field-metadata (minibuffer-prompt-end))) (let ((l (completion-all-sorted-completions))) (list (nth 0 l) (nth 1 l) (nth 2 l))))))
              90 t))))").unwrap();
        attach(&mut vm, "zqab\n");
        let outcome = keys(&mut vm, &[(1 << 27) | 'x' as i64]);
        report.push(format!("{:?}", outcome.and_then(|o| o.signal).map(|s| s.condition)));
        report.push(printed(&mut vm, "(prin1-to-string (list fzf-probe-ran fzf-probe))"));

        // M-x: the vertical list, TAB takes the selected command, Return runs it.
        vm.eval_source("(defun gc-probe-command () (interactive) (setq gc-probe-ran 'ran))").unwrap();
        let frames = attach(&mut vm, "gc-probe\t\n");
        let outcome = keys(&mut vm, &[(1 << 27) | 'x' as i64]);
        let (tallest, texts) = shown(&frames);
        report.push(format!("{:?}", outcome.and_then(|o| o.signal).map(|s| s.condition)));
        report.push(printed(&mut vm, "(list (bound-and-true-p gc-probe-ran) (car extended-command-history))"));
        report.push(format!("{} {}", tallest > LINE, mini_height(&mut vm) == LINE));
        report.push(texts.iter().any(|t| t.contains("M-x gc-probe \ngc-probe-command")).to_string());
        report.push(texts.iter().any(|t| t.contains("M-x gc-probe-command \n")).to_string());

        // C-x C-f: partial completion of s/n to sub/notes.txt, then visit.
        vm.eval_source(&format!("(setq default-directory {})", quoted(&work))).unwrap();
        let frames = attach(&mut vm, "s/n\t\n");
        let outcome = keys(&mut vm, &[24, 6]);
        let (tallest, texts) = shown(&frames);
        report.push(format!("{:?}", outcome.and_then(|o| o.signal).map(|s| s.condition)));
        report.push(printed(&mut vm, &format!("(list (file-relative-name buffer-file-name {}) (buffer-string))", quoted(&work))));
        report.push(format!("{} {}", tallest > LINE, mini_height(&mut vm) == LINE));
        report.push(texts.iter().any(|t| t.contains(&format!("Find file: {work}s/n \nsub/notes.txt"))).to_string());
        // GNU's files.el visits: uniquify names, a visited file is found
        // again, and closing the window asks before losing an edit to it,
        // a literal visit's too.
        report.push(printed(&mut vm, &format!("(let* ((default-directory {}) (a (find-file-noselect \"uq/a/notes.org\")) (b (find-file-noselect \"uq/b/notes.org\")))
            (list (buffer-name a) (buffer-name b) (eq a (find-file-noselect \"uq/b/../a/notes.org\"))
                  (mapcar (lambda (buffer)
                            (with-current-buffer buffer (insert \"edit\"))
                            (setq romacs-gui-quit-requested nil)
                            (with-current-buffer \"*scratch*\" (romacs-gui-quit))
                            (prog1 (list romacs-gui-quit-requested (car romacs-gui-pending))
                              (romacs-gui-answer-no)
                              (with-current-buffer buffer (set-buffer-modified-p nil))))
                          (list a (find-file-noselect \"uq/raw.txt\" nil t)))))", quoted(&work))));

        // C-x C-f: an fzf-only query, an exact term, visits fzf's best match.
        vm.eval_source(&format!("(setq default-directory {})", quoted(&work))).unwrap();
        attach(&mut vm, "'qab\n");
        let outcome = keys(&mut vm, &[24, 6]);
        report.push(format!("{:?}", outcome.and_then(|o| o.signal).map(|s| s.condition)));
        report.push(printed(&mut vm, "(list (buffer-name) (car fzf-probe))"));

        // C-x b: the default in the prompt, completion over the buffers.
        vm.eval_source("(get-buffer-create \"alpha-kt-buffer\")").unwrap();
        let frames = attach(&mut vm, "alpha-kt\t\n");
        let outcome = keys(&mut vm, &[24, 98]);
        let (_, texts) = shown(&frames);
        report.push(format!("{:?}", outcome.and_then(|o| o.signal).map(|s| s.condition)));
        report.push(printed(&mut vm, "(list (buffer-name (window-buffer (selected-window))) (car buffer-name-history))"));
        report.push(texts.iter().any(|t| t.contains("Switch to buffer (default ") && t.contains("alpha-kt-buffer")).to_string());
        report
    }).unwrap().join().unwrap();
    let _ = std::fs::remove_dir_all(&root);
    assert_eq!(report, [
        "((fzf basic partial-completion emacs22) t one-column historical ((file (styles fzf partial-completion)) (project-file (styles fzf substring))) 0.15 t t t t t icomplete-force-complete grow-only)",
        "(fzf basic partial-completion emacs22)",
        "None", r#""(zqab-x ((fzf basic partial-completion emacs22) (\"zqab-x\" \"zq-alpha-beta\" \"xzqab\")))""#,
        "None", "(ran \"gc-probe-command\")", "true true", "true", "true",
        "None", r#"("sub/notes.txt" "notes\n")"#, "true true", "true",
        r#"("notes.org<a>" "notes.org<b>" t ((nil quit) (nil quit)))"#,
        "None", "(\"zqab.txt\" (fzf partial-completion basic emacs22))",
        "None", "(\"alpha-kt-buffer\" \"alpha-kt-buffer\")", "true",
    ]);
}

/// On a window system the defaults show the minibuffer in romacs-complete's
/// layer (DIVERGENCES VM2-COMPLETE-DEFAULT), and Return takes the
/// candidate of the input typed, even when typing outran the candidate
/// updates after its first key: Return, `romacs-complete-directory-enter',
/// once exited with a candidate computed for that first key.
#[test]
fn romacs_complete_is_the_default_on_a_window_system_and_return_takes_the_typed_input() {
    let report = std::thread::Builder::new().stack_size(64 << 20).spawn(move || {
        let mut vm = Vm::new();
        vm.set_frame_geometry(1280, 800, LINE, 10).unwrap();
        vm.set_display_host(Some(Box::new(fonts::Display::new().unwrap())));
        startup::initialize_loadup(&mut vm, startup::Launch::Loadup, None).unwrap();
        let mut report = vec![printed(&mut vm, "(list window-system romacs-complete-mode
            (eval (car (get 'romacs-complete-mode 'standard-value)) t) fido-vertical-mode icomplete-mode
            (frame-live-p (romacs--minibuffer-host)))")];
        vm.eval_source("(defun rc-ahead-target () (interactive) (setq rc-ahead-ran 'ran))").unwrap();
        attach_typing_ahead(&mut vm, "rc-ahead-target\r", 1);
        let outcome = keys(&mut vm, &[(1 << 27) | 'x' as i64]);
        report.push(format!("{:?}", outcome.and_then(|o| o.signal).map(|s| s.condition)));
        report.push(printed(&mut vm, "(list (bound-and-true-p rc-ahead-ran) (car extended-command-history))"));
        // The panel floats centred, glass: rounded, lit, padded, its own background.
        report.push(printed(&mut vm, "(let ((f romacs-complete--layer))
            (list romacs-complete-position (frame-parameter f 'romacs-layer-radius) (frame-parameter f 'romacs-layer-shadow)
                  (frame-parameter f 'romacs-layer-glow) (frame-parameter f 'child-frame-border-width)
                  (stringp (frame-parameter f 'background-color)) (frame-parameter f 'romacs-layer-edge)
                  (pcase-let ((`(,l ,_ ,w ,_) (romacs-complete--placement f 3)))
                    (= (+ l (/ w 2)) (/ (- (frame-text-width) (* 2 (frame-parameter f 'child-frame-border-width))) 2)))))"));
        // Each position moves the same panel, glass, centred and as wide,
        // and nothing else: `bottom' ends a line above the mode line of the
        // window the minibuffer was entered from, at the panel's tallest.
        report.push(printed(&mut vm, "(let ((f romacs-complete--layer))
            (cl-letf (((symbol-function 'minibuffer-selected-window) #'selected-window))
              (list (romacs-complete--mode-line-top (frame-parent f))
                    (mapcar (lambda (position)
                              (let ((romacs-complete-position position))
                                (romacs-complete--frame)
                                (cons position (append (romacs-complete--placement f 3)
                                                       (list (frame-parameter f 'romacs-layer-radius)
                                                             (frame-parameter f 'romacs-layer-edge))))))
                            '(top center bottom)))))"));
        // At `bottom' over a short selected window the panel shows only
        // the rows that fit above its mode line.
        report.push(printed(&mut vm, "(let ((f romacs-complete--layer) (romacs-complete-position 'bottom))
            (cl-letf (((symbol-function 'minibuffer-selected-window) #'selected-window))
              (split-window nil 10)
              (prog1 (list (romacs-complete--mode-line-top (frame-parent f)) (romacs-complete--rows)
                           (cadr (romacs-complete--placement f (romacs-complete--rows))))
                (delete-other-windows))))"));
        // A prompt that wraps, at `bottom' over a short selected window:
        // the rows, the panel and paging all fit the room above its mode
        // line once the prompt wrapped.
        vm.eval_source("(defun rc-pick () (interactive)
            (minibuffer-with-setup-hook
                (lambda () (add-hook 'post-command-hook
                             (lambda () (let ((f romacs-complete--layer))
                                          (push (list (romacs-complete--rows) romacs-complete--index (window-pixel-height (minibuffer-window))
                                                      (+ (cdr (frame-position f)) (frame-pixel-height f))
                                                      (romacs-complete--mode-line-top (frame-parent f)))
                                                rc-pick-seen)))
                             90 t))
              (setq rc-pick-result (completing-read (make-string 70 ?p) (mapcar (lambda (i) (format \"cand-%02d\" i)) (number-sequence 1 30))))))
            (setq romacs-complete-position 'bottom rc-pick-seen nil) (split-window nil 10)").unwrap();
        attach(&mut vm, "rc-pick\n\u{16}\n");
        keys(&mut vm, &[(1 << 27) | 'x' as i64]);
        report.push(printed(&mut vm, "(prog1 (list rc-pick-result (car rc-pick-seen) (length rc-pick-seen)
                                                  (seq-every-p (lambda (r) (<= (nth 3 r) (- (nth 4 r) 20))) rc-pick-seen))
                                          (delete-other-windows) (setq romacs-complete-position 'top))"));
        // Messages stack as toasts in the top-right corner, newest on top, at
        // most three; the same message again extends the newest.
        report.push(printed(&mut vm, "(progn (dolist (m '(\"one\" \"two\" \"three\" \"four\" \"four\")) (romacs-layer-toast m))
            (list (mapcar (lambda (toast) (nth 1 toast)) romacs-layer--toasts)
                  (let ((tops (mapcar (lambda (toast) (cdr (frame-position (romacs-layer--frame (car toast))))) romacs-layer--toasts)))
                    (equal tops (sort (copy-sequence tops) #'<)))
                  (let ((f (romacs-layer--frame (caar romacs-layer--toasts))))
                    (list (frame-parameter f 'romacs-layer-edge)
                          (= (+ (car (frame-position f)) (frame-pixel-width f) 12) (frame-text-width))))
                  (current-message)))"));
        // Only the characters fzf matched carry the match face; a file shows
        // its name, then its directory, muted.
        report.push(printed(&mut vm, "(let* ((completion-styles '(fzf)) (completion-lazy-hilit nil)
                (s (romacs-complete--dim-directory
                    (romacs-complete--accent (copy-sequence (car (completion-all-completions \"band\" '(\"docs/band.rs\") nil 4))))))
                (with (lambda (face) (seq-filter (lambda (i) (memq face (ensure-list (get-text-property i 'face s))))
                                                 (number-sequence 0 (1- (length s)))))))
            (list (string-to-multibyte (substring-no-properties s)) (funcall with 'romacs-complete-match) (funcall with 'romacs-complete-directory)))"));
        // A panel made again is glass again; with glass off a toast is square.
        report.push(printed(&mut vm, "(progn (romacs-complete-mode -1) (romacs-complete-mode 1)
            (list (frame-parameter romacs-complete--layer 'romacs-layer-radius)
                  (let ((romacs-layer-glass nil)) (romacs-layer-toast \"square\")
                    (let ((f (romacs-layer--frame (caar romacs-layer--toasts))))
                      (list (frame-parameter f 'romacs-layer-radius) (frame-parameter f 'child-frame-border-width))))))"));
        // The panel is 42% of the room, at least 60 columns, never wider
        // than the room: a narrow frame or a large font is not clipped.
        report.push(printed(&mut vm, "(mapcar (lambda (room-column) (apply #'romacs-complete--panel-width room-column))
            '((1260 10) (2000 10) (480 10) (1260 30)))"));
        vm.set_frame_geometry(500, 800, LINE, 10).unwrap();
        report.push(printed(&mut vm, "(let ((f romacs-complete--layer))
            (pcase-let ((`(,l ,_ ,w ,_) (romacs-complete--placement f 3)))
              (list l (+ l w (* 2 (frame-parameter f 'child-frame-border-width))) (frame-text-width))))"));
        // Input that wraps at the panel's width grows it before redisplay:
        // at `bottom' the panel rises by the input's new lines and still
        // ends at least a line above the mode line.
        // (This frame's panel is 48 columns, under `truncate-partial-width-windows'.)
        report.push(printed(&mut vm, "(let ((f romacs-complete--layer) (romacs-complete-position 'bottom)
                  (truncate-partial-width-windows nil))
            (cl-letf (((symbol-function 'minibuffer-selected-window) #'selected-window))
              (with-current-buffer (window-buffer (minibuffer-window)) (insert (make-string 300 ?x)))
              (romacs-complete--show (romacs-complete--candidates-buffer) 3)
              (list (window-pixel-height (minibuffer-window)) (frame-text-height f) (cdr (frame-position f))
                    (<= (+ (cdr (frame-position f)) (frame-pixel-height f)) (- (romacs-complete--mode-line-top (frame-parent f)) 20)))))"));
        report
    }).unwrap().join().unwrap();
    assert_eq!(report, ["(pgtk t t nil nil pgtk)", "None", r#"(ran "rc-ahead-target")"#,
                        r##"(top 8 t "#d48a20" 10 t top t)"##,
                        "(780 ((top 322 88 600 80 8 top) (center 322 260 600 80 8 top) (bottom 322 480 600 80 8 top)))",
                        "(180 6 0)", "(\"cand-06\" (5 5 40 160 180) 2 t)",
                        "((\"four\" \"three\" \"two\") t (top t) nil)",
                        "(\"band.rs  docs/\" (0 1 2 3) (9 10 11 12 13))",
                        "(8 (nil 1))", "(600 840 480 1260)", "(0 484 484)", "(140 200 360 t)"]);
}
