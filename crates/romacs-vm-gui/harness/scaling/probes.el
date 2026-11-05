;;; probes.el --- scaling probes: one workload at size N and k*N  -*- lexical-binding: t -*-

;; Maintainer: Thanos Apollo <public@thanosapollo.org>

;;; Commentary:
;; scaling_live.py runs each probe of `scaling-probes' in a fresh session
;; at N and at K*N and compares the work by the evaluating thread's
;; instruction count (`romacs-instructions'): its empirical exponent,
;; log(work(K*N) / work(N)) / log(K), must stay within the probe's
;; budget.  A probe holds a growing collection of one subsystem, at sizes
;; real sessions reach.
;;
;; A probe is (scaling-probe NAME :n N [:k K] [:budget B] [:known TICKET]
;; [:gui t] [:steps R] :setup FORM :body FORM):
;;   :setup  runs uncounted with `n' bound; its value is `it' in :body.
;;   :body   the counted work at size `n'.  A batch probe's body runs once
;;           with collection held off, so where a collection lands does
;;           not decide the count; a :gui probe's body is one step, run
;;           :steps R times from timers, each laid out by the host before
;;           the next, `i' bound to the step.
;;   :budget the largest exponent allowed (default 1.25: n log n).
;;   :known  a ticket naming a known super-linear cost: the probe still
;;           runs and reports, but over budget it is debt, not a failure.
;;   :gap    a GUI probe's seconds between steps (default 0.1).
;;   :stack  also report how much native stack the body keeps
;;           (`romacs-native-stack-room' at `scaling-stack-mark' calls).
;;
;; Batch: romacs -Q --batch -l probes.el --eval '(scaling-run (quote NAME) N)'
;; prints one line, SCALING <json>.  `scaling-list' prints the probes.

;;; Code:

(require 'cl-lib)

(defvar scaling-probes nil
  "The probes, newest first: (NAME . PLIST).")

(defmacro scaling-probe (name &rest plist)
  "Define probe NAME from PLIST, its :setup and :body forms made functions."
  (let ((setup (plist-get plist :setup)) (body (plist-get plist :body)))
    `(setf (alist-get ',name scaling-probes)
           (list ,@(cl-loop for (key value) on plist by #'cddr
                            unless (memq key '(:setup :body)) append (list key value))
                 :setup (lambda (n) (ignore n) ,setup)
                 :body (lambda (n it &optional i) (ignore n it i) ,body)))))

(defvar scaling-stack-marks nil
  "`romacs-native-stack-room' at each `scaling-stack-mark' of a body.")

(defun scaling-stack-mark ()
  "Record the native stack room here, for a :stack probe."
  (push (romacs-native-stack-room) scaling-stack-marks))

(defun scaling--names (prefix n)
  "N strings PREFIX-0 ... PREFIX-(N-1)."
  (let (names) (dotimes (i n) (push (format "%s-%d" prefix i) names)) (nreverse names)))

(defun scaling--lines-buffer (name n)
  "A buffer NAME of N numbered lines of prose."
  (with-current-buffer (get-buffer-create name)
    (erase-buffer)
    (dotimes (i n) (insert (format "line %d of some ordinary prose text, about sixty bytes\n" i)))
    (goto-char (point-min))
    (current-buffer)))

(defun scaling--file (n form-of)
  "A temporary .el file of N top-level forms, (FORM-OF I) the I-th."
  (let ((file (make-temp-file "scaling-" nil ".el")))
    (with-temp-file file
      (insert ";;; -*- lexical-binding: t -*-\n")
      (dotimes (i n) (prin1 (funcall form-of i) (current-buffer)) (insert "\n")))
    file))

(defun scaling--defun-form (i)
  "The I-th generated form: a defun with a body worth expanding, and every
thousandth a stack mark."
  (if (= (% i 1000) 999)
      '(scaling-stack-mark)
    `(defun ,(intern (format "scaling-f%d" i)) (x)
       (let ((a x) (b ,i))
         (setq a (if (> a 0) (* b 2) (- b 1)))
         (list a ,(format "s%d" i) '(q . ,i))))))

(defun scaling--candidates (n)
  "N file-like completion candidates."
  (let (names)
    (dotimes (i n)
      (push (format "src/module-%d/%s-%d.el" (% i 37) (nth (% i 5) '("abbrev" "buffer" "cabinet" "dired" "editor")) i)
            names))
    (nreverse names)))

;;;; Faces

(scaling-probe faces-defface :n 200
  :known "ticket faces-defface: custom-add-to-group is GNU's member + nconc over the whole group"
  :setup (mapcar #'intern (scaling--names "scaling-face" n))
  :body (dolist (face it) (custom-declare-face face '((t :weight bold)) "A scaling face." :group 'scaling)))

(scaling-probe faces-set-attribute :n 200
  :setup (let ((faces (mapcar #'intern (scaling--names "scaling-face" n))))
           (dolist (face faces) (custom-declare-face face '((t)) "A scaling face." :group 'scaling))
           faces)
  :body (let ((k 0))
          (dolist (face it)
            (set-face-attribute face nil :foreground (if (cl-oddp k) "red" "blue") :weight 'bold)
            (setq k (1+ k)))))

;; A theme application over N more package faces: loading and enabling
;; modus-operandi in a -Q window, ~1.3 G instructions, most of it loading
;; the theme file; nearly flat in the faces, which only the root frame
;; recomputes (VM2-CHILD-FRAME).
(scaling-probe faces-theme :gui t :n 50 :steps 1
  :setup (let ((faces (mapcar #'intern (scaling--names "scaling-face" n))))
           (dolist (face faces)
             (custom-declare-face face '((t :inherit font-lock-keyword-face)) "A package face." :group 'scaling))
           faces)
  :body (load-theme 'modus-operandi t))

(scaling-probe faces-edit-read :gui t :n 100 :steps 1
  :setup (let ((faces (mapcar #'intern (scaling--names "scaling-face" n))))
           (dolist (face faces) (custom-declare-face face '((t)) "A scaling face." :group 'scaling))
           faces)
  :body (dolist (face it)
          (set-face-attribute face nil :foreground "red")
          (face-attribute face :foreground)))

;;;; Buffers, markers, overlays, text properties, undo

(scaling-probe buffers :n 300
  :known "ticket buffers: creating or killing a buffer copies and rebuilds the whole buffer order list"
  :setup (scaling--names "scaling-buffer" n)
  :body (progn
          (dolist (name it) (get-buffer-create name))
          (dolist (name it) (get-buffer name))
          (length (buffer-list))
          (dolist (name it) (kill-buffer name))))

(scaling-probe markers :n 2000
  :setup (with-current-buffer (scaling--lines-buffer " scaling-markers" 200) (current-buffer))
  :body (with-current-buffer it
          (let ((size (buffer-size)) markers)
            (dotimes (i n) (push (copy-marker (1+ (% (* i 7919) size))) markers))
            (dotimes (i 100) (goto-char (1+ (% (* i 104729) (buffer-size)))) (insert "x"))
            (dolist (m markers) (marker-position m))
            (dolist (m markers) (set-marker m nil)))))

(scaling-probe overlays :n 1000
  :setup (scaling--lines-buffer " scaling-overlays" n)
  :body (with-current-buffer it
          (let (overlays)
            (goto-char (point-min))
            (dotimes (_ n)
              (let ((o (make-overlay (point) (line-end-position))))
                (overlay-put o 'face 'bold)
                (push o overlays))
              (forward-line 1))
            (goto-char (point-min))
            (dotimes (_ n)
              (overlays-at (point))
              (overlays-in (point) (line-end-position))
              (forward-line 1))
            (dolist (o overlays) (move-overlay o (overlay-start o) (1+ (overlay-start o))))
            (mapc #'delete-overlay overlays))))

;; N edits of a buffer holding N evaporating overlays (as a notmuch
;; search holds one per result): each edit asks only the overlays at the
;; span it leaves whether they emptied.  Half the edits empty an overlay,
;; which is deleted, as GNU's are; they delete its span by the overlay's
;; own positions, so no text is read between edits.
(scaling-probe evaporating-overlays :n 2000
  :setup (with-current-buffer (scaling--lines-buffer " scaling-evaporate" n)
           (goto-char (point-min))
           (let (overlays)
             (dotimes (i n)
               (let ((o (make-overlay (point) (line-end-position))))
                 (overlay-put o 'evaporate t)
                 (when (cl-evenp i) (push o overlays)))
               (forward-line 1))
             (cons (current-buffer) overlays)))
  :body (with-current-buffer (car it)
          (goto-char (point-max))
          (dotimes (_ (/ n 2))
            (insert-before-markers "xy")
            (delete-region (- (point) 2) (point)))
          (dolist (o (cdr it))
            (delete-region (overlay-start o) (overlay-end o)))))

(scaling-probe text-properties :n 2000
  :setup (scaling--lines-buffer " scaling-props" n)
  :body (with-current-buffer it
          (goto-char (point-min))
          (dotimes (i n)
            (put-text-property (point) (line-end-position) 'face (if (cl-oddp i) 'bold 'italic))
            (forward-line 1))
          (goto-char (point-min))
          (dotimes (_ n) (get-text-property (point) 'face) (forward-line 1))
          (let ((pos (point-min)))
            (while (setq pos (next-single-property-change pos 'face))))))

;; The buffer walked by next-property-change, as motion over invisible
;; text and redisplay do, where one key reads one value under many stamps
;; (each line's `fontified' put on its own, adjacent, as jit-lock puts its
;; chunks) and another changes every 50 lines.
(scaling-probe property-change-walk :n 20000
  :setup (with-current-buffer (scaling--lines-buffer " scaling-walk" n)
           (goto-char (point-min))
           (dotimes (i n)
             (when (zerop (% i 50)) (put-text-property (point) (line-end-position) 'face 'bold))
             (put-text-property (point) (progn (forward-line 1) (point)) 'fontified t))
           (current-buffer))
  :body (with-current-buffer it
          (let ((pos (point-min)))
            (while (setq pos (next-property-change pos))))
          (let ((pos (point-max)))
            (while (setq pos (previous-property-change pos))))))

(scaling-probe undo :n 2000
  :setup (with-current-buffer (get-buffer-create " scaling-undo")
           (buffer-enable-undo)
           (current-buffer))
  :body (with-current-buffer it
          (dotimes (_ n) (insert "x") (undo-boundary))
          (undo-start)
          (undo-more n)))

;;;; Lambda lists

;; N `(lambda ...)' lists, each called five times: one compile per list,
;; then hits (`Vm::lambda_list_closure'); the cache holds what was called
;; since the last collection.
(scaling-probe lambda-lists :n 200
  :setup (let (functions)
           (dotimes (i n) (push (list 'lambda '(x) (list '+ 'x i)) functions))
           functions)
  :body (dotimes (_ 5) (dolist (f it) (funcall f 1))))

;;;; Tree-sitter

;; A Rust buffer of N lines parsed by the bundled grammar, then twenty
;; edits, each forcing a reparse and a capture over its line: the parse is
;; linear in the text, an edit's reparse and capture are not in N.
(scaling-probe treesit-parse-edit :n 2000
  :setup (with-current-buffer (get-buffer-create " scaling-treesit")
           (dotimes (i n)
             (insert (format "fn f%d(x: u32) -> u32 { let y = x + %d; y * 2 }\n" i i)))
           (current-buffer))
  :body (with-current-buffer it
          (let ((parser (treesit-parser-create 'rust)))
            (treesit-parser-root-node parser)
            (goto-char (/ (point-max) 2))
            (dotimes (_ 20)
              (insert "x")
              (treesit-query-capture (treesit-parser-root-node parser) "(identifier) @i"
                                     (line-beginning-position) (line-end-position))))))

;;;; Windows

(scaling-probe windows :n 6
  :setup (progn
           (setq window-min-height 1 window-min-width 2)
           (delete-other-windows)
           nil)
  :body (progn
          (dotimes (i n)
            (split-window (get-largest-window) nil (if (cl-oddp i) 'right 'below)))
          (let ((count 0)) (walk-windows (lambda (_) (setq count (1+ count)))) count)
          (dolist (w (window-list)) (ignore-errors (window-resize w 1 nil)))
          (balance-windows)
          (delete-other-windows)))

;;;; Symbols, hash tables, features

(scaling-probe obarray-intern :n 20000
  :setup (scaling--names "scaling-symbol" n)
  :body (progn (dolist (name it) (intern name)) (dolist (name it) (intern-soft name))))

(scaling-probe hash-equal :n 20000
  :setup (scaling--names "scaling-key" n)
  :body (let ((table (make-hash-table :test #'equal)))
          (dolist (key it) (puthash key t table))
          (dolist (key it) (gethash key table))
          (dolist (key it) (remhash key table))))

(scaling-probe hash-eq :n 50000
  :setup nil
  :body (let ((table (make-hash-table :test #'eq)))
          (dotimes (i n) (puthash i i table))
          (dotimes (i n) (gethash i table))
          (dotimes (i n) (remhash i table))))

(scaling-probe features :n 300
  :known "ticket features: featurep and provide are GNU's memq over the features list"
  :setup (mapcar #'intern (scaling--names "scaling-feature" n))
  :body (dolist (feature it) (provide feature) (featurep feature) (require feature)))

(scaling-probe load-files :n 50
  :setup (mapcar (lambda (i) (scaling--file 1 (lambda (_) `(provide ',(intern (format "scaling-file-%d" i))))))
                 (number-sequence 1 n))
  :body (dolist (file it) (load file nil t)))

;;;; Loading and evaluating many forms

(scaling-probe load-forms :n 12500 :stack t
  :setup (scaling--file n #'scaling--defun-form)
  :body (load it nil t))

(scaling-probe eval-buffer-forms :n 12500 :stack t
  :setup (let ((file (scaling--file n #'scaling--defun-form)))
           (with-current-buffer (get-buffer-create " scaling-eval-buffer")
             (insert-file-contents file)
             (current-buffer)))
  :body (eval-buffer it))

;;;; Completion

(defmacro scaling--completion-probe (name style &optional library)
  `(scaling-probe ,name :n 2000
     :setup (progn (require 'minibuffer) ,@(and library `((require ',library))) (scaling--candidates n))
     :body (let ((completion-styles '(,style)) (completion-ignore-case nil))
             (completion-all-completions "ed" it nil 2)
             (completion-try-completion "ed" it nil 2))))

(scaling--completion-probe completion-basic basic)
(scaling--completion-probe completion-partial partial-completion)
(scaling--completion-probe completion-substring substring)
(scaling--completion-probe completion-flex flex)
(scaling--completion-probe completion-initials initials)
(scaling--completion-probe completion-orderless orderless romacs-orderless)
(scaling--completion-probe completion-fzf fzf romacs-fzf)

;;;; Processes and timers

(scaling-probe process-output :n 200000
  :setup (get-buffer-create " scaling-process")
  :body (let ((process (make-process :name "scaling" :buffer it :command (list "seq" (number-to-string n))
                                     :connection-type 'pipe :noquery t)))
          (while (process-live-p process) (accept-process-output process 1))
          (accept-process-output process 0)
          (with-current-buffer it (count-lines (point-min) (point-max)))))

(scaling-probe timers :n 200
  :known "ticket timers: timer--activate walks the sorted timer-list, ~12k instructions a step"
  :setup nil
  :body (let (timers)
          (dotimes (i n) (push (run-with-timer (+ 3600 i) nil #'ignore) timers))
          (dotimes (_ 20) (sit-for 0))
          (mapc #'cancel-timer timers)))

;;;; Mode line (rlisp/romacs-mode-line.el)

(defvar scaling-mode-line-hook nil)

(scaling-probe mode-line-segments :n 500
  ;; Segments defined while the mode is on, each on a shared hook and on
  ;; a hook of its own, so both registries grow; then both hooks a
  ;; refresh runs on.
  :setup (progn (require 'romacs-mode-line) (romacs-mode-line-mode 1) nil)
  :body (progn
          (dotimes (i n)
            (let ((own (intern (format "scaling-mode-line-hook-%d" i))))
              (set own nil)
              (eval `(romacs-mode-line-define-segment ,(intern (format "scaling-%d" i)) "A probe."
                       :compute (lambda () nil) :hooks (scaling-mode-line-hook ,own))
                    t)))
          (run-hooks 'scaling-mode-line-hook)
          (run-hooks 'after-change-major-mode-hook)))

;; The status bar's marks and paint (VM2-STATUS-BAR): N pill segments,
;; each with an icon state, its own pill colour and its own pill text
;; colour, laid out after each step's change.
(scaling-probe mode-line-bar-segments :gui t :n 40 :steps 20
  :setup (progn
           (require 'romacs-mode-line)
           (romacs-mode-line-mode 1)
           (dotimes (i n)
             (let ((face (intern (format "bar-probe-face-%d" i))))
               (make-face face)
               (set-face-attribute face nil :background (format "#%02x3040" (% (* 5 i) 256))
                                   :foreground (format "#e0%02xa0" (% (* 3 i) 256)))
               (eval `(romacs-mode-line-define-segment ,(intern (format "bar-probe-%d" i)) "A probe."
                        :pill ,face :pill-text ,face
                        :format (" " (:propertize "%*" romacs-icon ((?* . modified) (?- . saved))) " "))
                     t))
             (push (intern (format "bar-probe-%d" i)) romacs-mode-line-right))
           (romacs-mode-line-rebuild)
           (switch-to-buffer (scaling--lines-buffer "scaling-bar" 50)))
  :body (with-current-buffer it (insert "x")))

;; N pills on one pill face, each a different character in its own text
;; colour, all shown (the right side shows whole): the painter's
;; recoloured faces and their character bindings.
(scaling-probe mode-line-bar-pill-text :gui t :n 12 :steps 20
  :setup (progn
           (require 'romacs-mode-line)
           (romacs-mode-line-mode 1)
           (setq romacs-mode-line-right nil)
           (dotimes (i n)
             (let ((face (intern (format "bar-text-face-%d" i))))
               (make-face face)
               (set-face-attribute face nil :foreground (format "#e0%02x%02x" (% (* 7 i) 256) (% (* 11 i) 256)))
               (eval `(romacs-mode-line-define-segment ,(intern (format "bar-text-%d" i)) "A probe."
                        :pill romacs-mode-line-pill :pill-text ,face
                        :format ,(format " %c " (+ ?α i)))
                     t)
               (push (intern (format "bar-text-%d" i)) romacs-mode-line-right)))
           (romacs-mode-line-rebuild)
           (switch-to-buffer (scaling--lines-buffer "scaling-bar-text" 50)))
  :body (with-current-buffer it (insert "x")))

;; N characters the bar draws as an icon an alist of N entries names,
;; the icon's entry at the end of a registry grown by N: the registry and
;; each piece's alist are indexed once, not walked per character.
(scaling-probe mode-line-bar-icons :gui t :n 20 :steps 20
  :setup (progn
           (require 'romacs-icons)
           (setq romacs-icons--font t romacs-icons-source 'romacs romacs-icons-ascii nil)
           (setq romacs-icons-alist
                 (append romacs-icons-alist
                         (cl-loop for j below n collect (list (intern (format "probe-icon-%d" j)) #xFE001))))
           ;; The string's own mapping grows too: N characters, each mapped
           ;; near its end.
           (let ((last (intern (format "probe-icon-%d" (1- n))))
                 (text (apply #'string (cl-loop for j below n collect (+ #x100 j)))))
             (setq-default mode-line-format
                           `((:propertize ,text romacs-icon
                                          ,(cl-loop for j below n collect (cons (+ #x100 j) last))))))
           (switch-to-buffer (scaling--lines-buffer "scaling-bar-icons" 50)))
  :body (with-current-buffer it (insert "x")))

;; Text showing an icon at N pixel sizes, laid out each step: the font
;; resolver's per-icon answers (`resolve_sole'), one per size, are found,
;; not resolved again.
(scaling-probe icon-sizes :gui t :n 20 :steps 20
  :setup (let ((buffer (scaling--lines-buffer "scaling-icon-sizes" 10)))
           (with-current-buffer buffer
             (goto-char (point-min))
             (dotimes (i n)
               (insert (propertize (string #xFE001) 'face (list :height (+ 1.0 (* 0.01 i)))) " ")))
           (switch-to-buffer buffer)
           buffer)
  :body (with-current-buffer it (goto-char (point-max)) (insert "x")))

;; N windows, each showing its own buffer and drawing its own line
;; (VM2-MODE-LINE), a key typed in one: the others' lines are reused,
;; not expanded again.
(scaling-probe mode-line-windows :gui t :n 2 :steps 20
  :setup (progn
           (require 'romacs-mode-line)
           (romacs-mode-line-mode 1)
           (delete-other-windows)
           (let ((main (scaling--lines-buffer "scaling-windows" 20)))
             (switch-to-buffer main)
             (save-selected-window
               (dotimes (i (1- n))
                 (select-window (split-window (get-largest-window) nil (quote below)))
                 (switch-to-buffer (scaling--lines-buffer (format "scaling-window-%d" i) 5))))
             (balance-windows)
             main))
  :body (with-current-buffer it (insert "x")))

;; N side windows whose buffers set their own mode line, each folded into
;; the status bar as a context segment (VM2-STATUS-BAR), a key typed in
;; the selected window.
(scaling-probe mode-line-contexts :gui t :n 2 :steps 20
  :setup (progn
           (delete-other-windows)
           (let ((main (scaling--lines-buffer "scaling-contexts" 20)))
             (switch-to-buffer main)
             (dotimes (i n)
               (let ((buffer (scaling--lines-buffer (format "scaling-context-%d" i) 2)))
                 (with-current-buffer buffer (setq-local mode-line-format (list (format "ctx %d " i) "%b")))
                 (display-buffer-in-side-window buffer `((side . right) (slot . ,i) (window-width . 12)))))
             main))
  :body (with-current-buffer it (insert "x")))

;; N side windows whose own lines are each wider than the bar, so every
;; one is dropped when the bar fits (VM2-STATUS-BAR): fitting and folding
;; stay linear in what they drop.
(scaling-probe mode-line-contexts-saturated :gui t :n 3 :steps 20
  :setup (progn
           (delete-other-windows)
           (let ((main (scaling--lines-buffer "scaling-saturated" 20)))
             (switch-to-buffer main)
             (dotimes (i n)
               (let ((buffer (scaling--lines-buffer (format "scaling-saturated-%d" i) 2)))
                 (with-current-buffer buffer (setq-local mode-line-format (list (make-string 200 (+ ?a (% i 26))))))
                 (display-buffer-in-side-window buffer `((side . ,(if (cl-oddp i) 'left 'right)) (slot . ,i) (window-width . 10)))))
             main))
  :body (with-current-buffer it (insert "x")))

;; `format-mode-line' of N pieces under one `:propertize' whose shared
;; keymap has N bindings: the map is made once for the string, not once
;; per piece (VM2-G1B).
(scaling-probe mode-line-shared-properties :n 200
  :setup (let ((map (list 'keymap)))
           (dotimes (i n) (push (cons (+ 1000 i) 'ignore) (cdr map)))
           (list :propertize (let (pieces) (dotimes (i n) (push (propertize "a" 'help-echo (number-to-string i)) pieces)) pieces)
                 'local-map map))
  :body (let* ((noninteractive nil) (s (format-mode-line it)))
          (unless (get-text-property 0 'local-map s) (error "The shared keymap is not projected"))
          s))

;; `face-font' for N faces each asking a distinct font (size, weight,
;; slant), then the same N again: the display's per-request font caches
;; grow by one entry per request and answer the repeats from the cache.
(scaling-probe face-font-styles :gui t :n 20 :steps 1
  :setup (let (faces)
           (dotimes (i n)
             (let ((face (intern (format "scaling-font-%d" i))))
               (make-face face)
               (set-face-attribute face nil :height (+ 100 (* 2 (/ i 4))) :weight (if (cl-oddp i) 'bold 'normal)
                                   :slant (if (zerop (% (/ i 2) 2)) 'normal 'italic))
               (push face faces)))
           (cons (switch-to-buffer (get-buffer-create "scaling-font")) faces))
  ;; The step paints (a character typed in the shown buffer), as a GUI
  ;; probe's must.
  :body (progn (dotimes (_ 2) (dolist (face (cdr it)) (face-font face)))
               (with-current-buffer (car it) (insert "x"))))

;;;; Redisplay (GUI: the host lays out each step's change)

(scaling-probe redisplay-lines :gui t :n 2000 :steps 20 :budget 0.5
  :setup (let ((buffer (scaling--lines-buffer "scaling-lines" n)))
           (with-current-buffer buffer
             (goto-char (point-min))
             (dotimes (i n)
               (put-text-property (point) (line-end-position) 'face (if (cl-oddp i) 'bold 'font-lock-keyword-face))
               (forward-line 1))
             (goto-char (/ (point-max) 2)))
           (switch-to-buffer buffer)
           buffer)
  :body (with-current-buffer it (forward-line 1) (insert "x")))

(scaling-probe redisplay-windows :gui t :n 3 :steps 20 :gap 0.4
  :setup (let ((buffer (scaling--lines-buffer "scaling-windows" 200)))
           (setq window-min-height 1 window-min-width 2)
           (switch-to-buffer buffer)
           (delete-other-windows)
           (dotimes (i n) (split-window (get-largest-window) nil (if (cl-oddp i) 'right 'below)))
           (balance-windows)
           buffer)
  :body (with-current-buffer it (goto-char (point-min)) (forward-line i) (insert "x")))

(scaling-probe redisplay-buffers :gui t :n 100 :steps 20 :budget 0.5
  :setup (progn
           (dolist (name (scaling--names "scaling-chrome" n))
             (with-current-buffer (get-buffer-create name) (insert name)))
           (switch-to-buffer (get-buffer-create "scaling-chrome-shown"))
           (current-buffer))
  :body (with-current-buffer it (insert "x")))

;;;; Server clients and agent sessions (romacs-server.el, romacs-agent*.el)

(defun scaling--server ()
  "romacs-server.el's waiter index with its GNU advice, and no socket."
  (require 'romacs-server)
  (advice-add 'server-buffer-done :before #'romacs-server--buffer-done)
  (advice-add 'server-done :around #'romacs-server--done))

;; One native client waiting on N files, each finished with server-done,
;; last first: each costs its own waiters, never the files left.  The
;; index alone (bury-buffer, which every server-done runs as GNU's does,
;; left out) and the whole command.
(scaling-probe server-waiters-index :n 200
  :setup (progn (scaling--server)
                (mapcar (lambda (name) (get-buffer-create name)) (scaling--names " scaling-wait" n)))
  :body (cl-letf (((symbol-function 'romacs-mcp-reply) #'ignore)
                  ((symbol-function 'bury-buffer) #'ignore))
          (romacs-server--wait (list :probe t) 'scaling-peer it)
          (dolist (buffer (reverse it)) (with-current-buffer buffer (server-done)))))

(scaling-probe server-waiters :n 200
  :known "ticket buffers: bury-buffer, which server-done runs per finished buffer, rebuilds the buffer order list"
  :setup (progn (scaling--server)
                (mapcar (lambda (name) (get-buffer-create name)) (scaling--names " scaling-wait" n)))
  :body (cl-letf (((symbol-function 'romacs-mcp-reply) #'ignore))
          (romacs-server--wait (list :probe t) 'scaling-peer it)
          (dolist (buffer (reverse it)) (with-current-buffer buffer (server-done)))))

;; N requests held on one connection, then the connection lost.
(scaling-probe server-peer :n 200
  :setup (progn (scaling--server)
                (mapcar (lambda (name) (get-buffer-create name)) (scaling--names " scaling-peer" n)))
  :body (progn
          (dolist (buffer it) (romacs-server--wait (list :probe t) 'scaling-one-peer (list buffer)))
          (romacs-server--peer-closed 'scaling-one-peer)))

(defun scaling--agent ()
  "romacs-agent.el with a private state directory."
  (setenv "XDG_STATE_HOME" (make-temp-file "scaling-state-" t))
  (require 'romacs-agent))

;; One event to each of N sessions.
(scaling-probe agent-dispatch :n 200
  :setup (progn (scaling--agent)
                (clrhash romacs-agent--sessions)
                (mapcar (lambda (id) (let ((s (romacs-agent--make :id id :harness 'claude :title id)))
                                       (puthash id s romacs-agent--sessions) s))
                        (scaling--names "scaling-session" n)))
  :body (let ((record (json-parse-string "{\"seq\": 1, \"out\": {\"type\": \"result\", \"total_cost_usd\": 0}}"
                                         :object-type 'hash-table :null-object :null :false-object :false)))
          (dolist (session it) (romacs-agent--record (romacs-agent-get (romacs-agent-id session)) record))))

;; The same with 50 more harness drivers defined ahead of Claude Code's: a
;; record reaches its session's driver through the session, never a
;; registry lookup.
(scaling-probe agent-dispatch-drivers :n 200
  :setup (progn (scaling--agent)
                (setq romacs-agent--drivers
                      (append (cl-loop for i below 50
                                       collect (list (intern (format "scaling-driver-%d" i))
                                                     :lifetime 'persistent
                                                     :config-variable (format "SCALING_DRIVER_%d_HOME" i)))
                              (cl-remove-if (lambda (d) (string-prefix-p "scaling-driver-" (symbol-name (car d))))
                                            romacs-agent--drivers)))
                (clrhash romacs-agent--sessions)
                (mapcar (lambda (id) (let ((s (romacs-agent--make :id id :harness 'claude :title id)))
                                       (puthash id s romacs-agent--sessions) s))
                        (scaling--names "scaling-session" n)))
  :body (let ((record (json-parse-string "{\"seq\": 1, \"out\": {\"type\": \"result\", \"total_cost_usd\": 0}}"
                                         :object-type 'hash-table :null-object :null :false-object :false)))
          (dolist (session it) (romacs-agent--record (romacs-agent-get (romacs-agent-id session)) record))))

;; N calls recorded in a session's call log (each an appended line) and
;; each looked up.
(scaling-probe agent-calls :n 200
  :setup (progn (scaling--agent)
                (make-directory (romacs-agent-tools--dir "scaling-calls") t)
                (scaling--names "relay:call" n))
  :body (progn
          (dolist (key it) (romacs-agent-tools--note "scaling-calls" key (romacs-mcp--object "state" "done")))
          (dolist (key it) (gethash key (romacs-agent-tools--table "scaling-calls")))))

;; N queued calls, each withdrawn by its relay.
(scaling-probe agent-withdraw :n 64
  :setup (progn (require 'romacs-mcp) (number-sequence 0 (1- n)))
  :body (let ((romacs-mcp--queue nil) (romacs-mcp--queued (make-hash-table :test #'equal)))
          (dolist (i it)
            (let ((request (list :peer 'scaling-peer :message (romacs-mcp--object "id" i) :cancelled nil)))
              (push request romacs-mcp--queue)
              (puthash (cons 'scaling-peer i) request romacs-mcp--queued)))
          (cl-letf (((symbol-function 'romacs-mcp--send) #'ignore))
            (dolist (i it) (romacs-mcp--withdraw 'scaling-peer "w" i)))))

;; N delegated tasks, each indexed by its client request id and found again.
(scaling-probe agent-requests :n 200
  :setup (progn (scaling--agent)
                (clrhash romacs-agent--requests)
                (mapcar (lambda (id) (romacs-agent--make :id id :harness 'codex :title id
                                                        :options (list :parent "scaling-parent" :request id)))
                        (scaling--names "scaling-request" n)))
  :body (progn
          (dolist (session it)
            (puthash (romacs-agent-id session) session romacs-agent--sessions)
            (romacs-agent--index session))
          (dolist (session it) (romacs-agent-requested "scaling-parent" (romacs-agent-id session)))))

;; N children of one parent each ending a turn while the parent's turn
;; runs: each wake is queued behind it once (the real send and queue).
(scaling-probe agent-wake :n 200
  :setup (progn (scaling--agent)
                (clrhash romacs-agent--sessions)
                (clrhash romacs-agent-tools--answered)
                (let ((parent (romacs-agent--make :id "scaling-parent" :harness 'codex :title "parent"
                                                  :state 'running)))
                  (puthash "scaling-parent" parent romacs-agent--sessions))
                (mapcar (lambda (id)
                          (let ((child (romacs-agent--make :id id :harness 'codex :title id :done-seq 1
                                                           :ends (list (list 1 "ok" nil))
                                                           :dir (expand-file-name id (romacs-agent--state-root))
                                                           :options (list :parent "scaling-parent"))))
                            (make-directory (romacs-agent-dir child) t)
                            (puthash id child romacs-agent--sessions)
                            child))
                        (scaling--names "scaling-child" n)))
  :body (dolist (child it) (romacs-agent-tools--wake child)))

;; One child with N turn ends its parent has not had: one wake gives each,
;; in order, behind the parent's running turn.
(scaling-probe agent-wake-ends :n 200
  :setup (progn (scaling--agent)
                (clrhash romacs-agent--sessions)
                (clrhash romacs-agent-tools--answered)
                (puthash "scaling-parent" (romacs-agent--make :id "scaling-parent" :harness 'codex :title "parent"
                                                              :state 'running)
                         romacs-agent--sessions)
                (let ((child (romacs-agent--make :id "scaling-ends" :harness 'codex :title "ends"
                                                 :dir (expand-file-name "scaling-ends" (romacs-agent--state-root))
                                                 :options (list :parent "scaling-parent")
                                                 :ends (let (ends) (dotimes (i n) (push (list (1+ i) "ok" nil) ends)) ends))))
                  (make-directory (romacs-agent-dir child) t)
                  (puthash "scaling-ends" child romacs-agent--sessions)
                  child))
  :body (romacs-agent-tools--wake it))

;; One child ending N turns, one after another, behind its parent's running
;; turn: each wake hands on only the new end.
(scaling-probe agent-wake-successive :n 200
  :setup (progn (scaling--agent)
                (clrhash romacs-agent--sessions)
                (clrhash romacs-agent-tools--answered)
                (puthash "scaling-parent" (romacs-agent--make :id "scaling-parent" :harness 'codex :title "parent"
                                                              :state 'running)
                         romacs-agent--sessions)
                (clrhash romacs-agent-tools--scheduled)
                (let ((child (romacs-agent--make :id "scaling-turns" :harness 'codex :title "turns"
                                                 :dir (expand-file-name "scaling-turns" (romacs-agent--state-root))
                                                 :options (list :parent "scaling-parent"))))
                  (make-directory (romacs-agent-dir child) t)
                  (puthash "scaling-turns" child romacs-agent--sessions)
                  child))
  :body (dotimes (i n)
          (romacs-agent--raise it (list :type 'done :text "ok" :seq (1+ i) :last t))
          (romacs-agent-tools--wake it)))

;; N deferred calls (delegations waiting) withdrawn by their relay, then
;; retired by the next poll.
(scaling-probe agent-withdraw-deferred :n 64
  :setup (progn (require 'romacs-mcp) (number-sequence 0 (1- n)))
  :body (let ((romacs-mcp--deferred nil) (romacs-mcp--deferred-index (make-hash-table :test #'equal))
              (romacs-mcp--deferred-timer t))
          (dolist (i it)
            (romacs-mcp--defer-request (list :peer 'scaling-peer :cancelled nil) i #'ignore nil))
          (cl-letf (((symbol-function 'romacs-mcp--send) #'ignore))
            (dolist (i it) (romacs-mcp--withdraw 'scaling-peer "w" i))
            (romacs-mcp--poll-deferred))
          (unless (null romacs-mcp--deferred) (error "Withdrawn calls were not retired"))))

;; N one-shot questions pending, each session's turn end reaching only its
;; own handler.
(scaling-probe agent-asks :n 200
  :setup (progn (scaling--agent)
                (clrhash romacs-agent--asks)
                (mapcar (lambda (id)
                          (let ((session (romacs-agent--make :id id :harness 'claude :title id :state 'running)))
                            (puthash id (lambda (_event) (remhash id romacs-agent--asks)) romacs-agent--asks)
                            session))
                        (scaling--names "scaling-ask" n)))
  :body (progn
          (dolist (session it)
            (romacs-agent--raise session (list :type 'done :text "ok" :seq 1 :last t)))
          (unless (zerop (hash-table-count romacs-agent--asks)) (error "Asks left unanswered"))))

;; N gnosis cursors and plans handed out as tokens (the newest 256 kept),
;; each looked up again.
(scaling-probe agent-gnosis-tokens :n 400
  :setup (progn (scaling--agent) (require 'romacs-agent-gnosis)
                (clrhash romacs-agent-gnosis--tokens) (fillarray romacs-agent-gnosis--ring nil)
                (number-sequence 1 n))
  :body (dolist (i it)
          (romacs-agent-gnosis--value-of (romacs-agent-gnosis--token (list :after i)))))

;; N messages sent to a per-turn session while its turn runs.
(scaling-probe agent-queue :n 2000
  :setup (progn (scaling--agent)
                (romacs-agent--make :id "scaling-queue" :harness 'codex :title "queue" :state 'running))
  :body (dotimes (i n) (romacs-agent-send it "queued")))

;; A chat of N tool calls, started then each answered.
(scaling-probe agent-chat-tools :n 200
  :setup (progn (scaling--agent)
                (let ((session (romacs-agent--make :id "scaling-chat" :harness 'claude :title "scaling chat"
                                                   :dir temporary-file-directory)))
                  (romacs-agent-show session)
                  session))
  :body (progn
          (dotimes (i n)
            (romacs-agent-ui--render it (list :type 'tool :state 'started :id (format "t%d" i) :name "Bash"
                                              :seq (1+ i) :last t)))
          (dotimes (i n)
            (romacs-agent-ui--render it (list :type 'tool :state 'completed :id (format "t%d" i) :output "ok"
                                              :seq (+ 1 n i) :last t)))))

;; The home page over N sessions, half of them delegated children.
(scaling-probe agent-home :n 200
  :setup (progn (scaling--agent)
                (clrhash romacs-agent--sessions)
                (dotimes (i n)
                  (let ((s (romacs-agent--make :id (format "%08d-home" i) :harness 'claude :provider "claude"
                                               :title (format "session %d" i) :cwd (format "/tmp/project-%d" (% i 7))
                                               :last-time (+ 1e9 i) :state 'idle :turns 1
                                               :options (and (cl-oddp i) (list :parent (format "%08d-home" (1- i)))))))
                    (puthash (romacs-agent-id s) s romacs-agent--sessions)))
                (with-current-buffer (get-buffer-create "*romacs-agents*") (romacs-agents-mode) (current-buffer)))
  :body (with-current-buffer it (romacs-agent-ui--home)))

;; The home page over N projects of one session each, and over N roots in one project.
(defun scaling--agent-sessions (n project)
  (scaling--agent)
  (clrhash romacs-agent--sessions)
  (dotimes (i n)
    (let ((s (romacs-agent--make :id (format "%08d-proj" i) :harness 'claude :provider "claude"
                                 :title (format "session %d" i) :cwd (funcall project i)
                                 :last-time (+ 1e9 i) :state 'idle :turns 1)))
      (puthash (romacs-agent-id s) s romacs-agent--sessions)))
  (with-current-buffer (get-buffer-create "*romacs-agents*") (romacs-agents-mode) (current-buffer)))

(scaling-probe agent-home-projects :n 200
  :setup (scaling--agent-sessions n (lambda (i) (format "/tmp/project-%d" i)))
  :body (with-current-buffer it (romacs-agent-ui--home)))

(scaling-probe agent-home-roots :n 200
  :setup (scaling--agent-sessions n (lambda (_) "/tmp/one-project"))
  :body (with-current-buffer it (romacs-agent-ui--home)))

;; One event to each of N sessions with the home page shown: its refresh is
;; scheduled, never rebuilt per event.
(scaling-probe agent-dispatch-home :n 200
  :setup (progn (scaling--agent-sessions n (lambda (i) (format "/tmp/project-%d" (% i 5))))
                (hash-table-values romacs-agent--sessions))
  :body (let ((record (json-parse-string "{\"seq\": 1, \"out\": {\"type\": \"result\", \"total_cost_usd\": 0}}"
                                         :object-type 'hash-table :null-object :null :false-object :false)))
          ;; As if the page were shown (batch has no visible window).
          (cl-letf (((symbol-function 'get-buffer-window) (lambda (&rest _) (selected-window))))
            (dolist (session it) (romacs-agent--record session record)))
          (when (timerp romacs-agents--refresh) (cancel-timer romacs-agents--refresh) (setq romacs-agents--refresh nil))))

;; The harnesses' history: N Claude Code transcripts in one configuration
;; directory, listed (stat only) once described, and listed and described anew.
(defun scaling--history (n)
  "A Claude Code configuration directory of N transcripts over 7 projects."
  (scaling--agent)
  (require 'romacs-agent-history)
  (let ((dir (make-temp-file "scaling-claude-" t)))
    (dotimes (i n)
      (let ((project (expand-file-name (format "projects/-tmp-project-%d" (% i 7)) dir)))
        (make-directory project t)
        (with-temp-file (expand-file-name (format "%08x-0000-4000-8000-%012x.jsonl" i i) project)
          (insert (format "{\"type\":\"user\",\"cwd\":\"/tmp/project-%d\",\"message\":{\"role\":\"user\",\"content\":\"task %d\"}}\n"
                          (% i 7) i)))))
    (setq romacs-agent-history-limit (max n 1)
          romacs-agent-history--descriptions (make-hash-table :test #'equal))
    dir))

(defmacro scaling--with-history-place (dir &rest body)
  "BODY with DIR as the only place history is kept."
  `(cl-letf (((symbol-function 'romacs-agent-history--places) (lambda () (list (list ,dir 'claude "claude" ,dir)))))
     ,@body))

(scaling-probe agent-history-relist :n 200
  :setup (let ((dir (scaling--history n))) (scaling--with-history-place dir (romacs-agent-history-list)) dir)
  :body (scaling--with-history-place it (romacs-agent-history-list)))

(scaling-probe agent-history-describe :n 100
  :setup (scaling--history n)
  :body (scaling--with-history-place it
          (clrhash romacs-agent-history--descriptions)
          (romacs-agent-history-list)))

;; N harness drivers defined, then each defined again, oldest first (the
;; farthest from the registry's head): each by its name's cell, never by
;; a walk of the registry.
(scaling-probe agent-define-harness :n 2000
  :setup (progn (scaling--agent) (mapcar #'intern (scaling--names "scaling-rdriver" n)))
  :body (progn
          (dolist (name it) (romacs-agent-define-harness name :lifetime 'persistent))
          (dolist (name it) (romacs-agent-define-harness name :lifetime 'per-turn))))

;; A harness's environment, with N harness drivers defined (each a
;; configuration variable and one it sets for its children) and N other
;; variables in the editor's: one membership test per variable, linear.
(scaling-probe agent-environment :n 500
  :setup (progn (scaling--agent)
                (dotimes (i n)
                  (romacs-agent-define-harness (intern (format "scaling-edriver-%d" i)) :lifetime 'persistent
                                               :config-variable (format "SCALING_EDRIVER_%d_HOME" i)
                                               :scrub (list (format "SCALING_EDRIVER_%d_CHILD" i))))
                (append (cl-loop for i below n collect (format "SCALING_KEPT_%d=value" i))
                        (cl-loop for i below n collect (format "SCALING_EDRIVER_%d_CHILD=parent" i))
                        process-environment))
  :body (let ((process-environment it)) (dotimes (_ 5) (romacs-agent--harness-environment))))

;; The transcripts no session continues, among 200 listed and 200
;; sessions, of a harness defined before N others: each row's identity
;; finds its harness by name, never by a walk of the registry.
(scaling-probe agent-history-items-drivers :n 400 :budget 0.1
  :setup (progn (scaling--agent)
                (require 'romacs-agent-history)
                ;; Defined first: the N after it stand between it and the head.
                (romacs-agent-define-harness 'scaling-hlast :lifetime 'persistent :config-variable "SCALING_HLAST_HOME"
                                             :config-default "/scaling/home")
                (dotimes (i n)
                  (romacs-agent-define-harness (intern (format "scaling-hdriver-%d" i)) :lifetime 'persistent
                                               :config-variable (format "SCALING_HDRIVER_%d_HOME" i)))
                (clrhash romacs-agent--sessions)
                (dotimes (i 200)
                  (let ((id (format "%08x-0000-4000-8000-%012x" i i)))
                    (puthash id (romacs-agent--make :id id :harness 'scaling-hlast :title id :harness-session id)
                             romacs-agent--sessions)))
                (setq romacs-agent-history--listing
                      (cl-loop for i below 200
                               collect (list :harness 'scaling-hlast :id (format "%08x-0000-4000-8000-%012x" (* 2 i) i)
                                             :path (format "/scaling/%d.jsonl" i) :mtime i :config-dir nil)))
                nil)
  :body (dotimes (_ 5) (romacs-agent-history-items)))

;; A tool result of N characters, Unicode text: its check is native, so
;; one character costs no Lisp call.
(scaling-probe mcp-result-text :n 20000
  :setup (progn (require 'romacs-mcp) (make-string n ?é))
  :body (dotimes (_ 20) (romacs-mcp--result it nil t)))

;; The usage log's work per command (a caught signal's backtrace, its
;; error record, its slow count) with N kinds already in its slow, traced
;; and captured tables and N timers pending: a fixed 200 commands, so any
;; scan of them shows.
(scaling-probe usage-log-command :n 2000 :budget 0.5
  :setup (progn (require 'romacs-usage-log)
                (define-error 'scaling-usage-error "Scaling usage error")
                (setq romacs-usage-log-file (make-temp-file "scaling-usage-" nil ".jsonl"))
                (dotimes (i n)
                  (puthash (format "command-%d fundamental-mode" i) (vector 1 60 60 0) romacs-usage-log--slow)
                  (puthash (format "command-%d error" i) t romacs-usage-log--traced)
                  (puthash (format "command-%d scaling-usage-error" i) 0 romacs-usage-log--captured))
                (dotimes (_ n) (run-at-time 3600 nil #'ignore))
                (mapcar #'intern (scaling--names "scaling-usage" 200)))
  :body (let ((signal-hook-function #'romacs-usage-log--signal) (romacs-usage-log-slow-loop-ms 0))
          (setq romacs-usage-log--pending nil)
          (dolist (command it)
            (let ((kind (format "%s scaling-usage-error" command)))
              (remhash kind romacs-usage-log--traced)
              (remhash kind romacs-usage-log--captured))
            (let ((real-this-command command) (this-command command))
              (romacs-usage-log--pre-command)
              (condition-case data (signal 'scaling-usage-error nil)
                (error (romacs-usage-log--command-error data nil nil)))
              (romacs-usage-log--post-command)
              (romacs-usage-log--settled)))))

;; A server's start prunes the session records: N kept because their
;; processes live (pid 1), each looked at once.
(scaling-probe usage-log-prune :n 500
  :setup (let ((root (make-temp-file "scaling-sessions-" t)))
           (require 'romacs-usage-log)
           (dotimes (i n) (make-directory (expand-file-name (format "20200101T%06d-1" i) root)))
           root)
  :body (romacs-usage-log--prune it 50))

;; The endpoint sweep over N sessions, N routes of sessions gone, N ended
;; editors' sockets and N earlier builds' directories: one pass each.
(defvar romacs-mcp--socket)
(scaling-probe agent-endpoint-sweep :n 200
  :setup (let* ((runtime (make-temp-file "scaling-runtime-" t))
                (agents (expand-file-name "romacs/agents/" runtime)))
           (scaling--agent)
           (setenv "XDG_RUNTIME_DIR" runtime)
           (clrhash romacs-agent--sessions)
           (dolist (directory '("route/" "editor/"))
             (make-directory (expand-file-name directory agents) t)
             (set-file-modes (expand-file-name directory agents) #o700))
           (dotimes (i n)
             (let* ((dir (make-temp-file "scaling-session-" t))
                    (s (romacs-agent--make :id (format "%08d-sweep" i) :harness 'claude :dir dir)))
               (with-temp-file (expand-file-name "mcp.json" dir) (insert "{}"))
               (puthash (romacs-agent-id s) s romacs-agent--sessions))
             (make-symbolic-link "/nonexistent" (format "%sroute/gone%d.sock" agents i))
             (make-symbolic-link "/nonexistent" (format "%seditor/%d.sock" agents (+ 5000000 i)))
             (let ((legacy (format "%s/romacs/session-%d" runtime (+ 6000000 i))))
               (make-directory legacy t)
               (set-file-modes legacy #o700)))
           runtime)
  :body (let ((romacs-mcp--socket nil)) (romacs-agent--sweep)))

;; The history's places over N provider instances.
(defvar romacs-agent-providers)
(scaling-probe agent-history-places :n 1000
  :setup (progn (scaling--agent) (require 'romacs-agent-history)
                (cl-loop for i below n collect (cons (format "claude-%d" i) (format "/tmp/claude-config-%d" i))))
  :body (let ((romacs-agent-providers it)) (romacs-agent-history--places)))

;; The home page over N transcripts beside a session per project.
(scaling-probe agent-home-history :n 200
  :setup (let ((dir (scaling--history n)))
           (scaling--with-history-place dir (romacs-agent-history-list))
           (clrhash romacs-agent--sessions)
           (with-current-buffer (get-buffer-create "*romacs-agents*") (romacs-agents-mode) (current-buffer)))
  :body (with-current-buffer it (romacs-agent-ui--home)))

;; A transcript of N turns (a prompt, a tool call and its result, a reply), viewed.
(scaling-probe agent-history-view :n 50
  :setup (let ((file (make-temp-file "scaling-transcript-" nil ".jsonl")))
           (scaling--agent)
           (require 'romacs-agent-history)
           (romacs-agent-ui--helpers)
           (with-temp-file file
             (dotimes (i n)
               (insert (format "{\"type\":\"user\",\"cwd\":\"/tmp\",\"timestamp\":\"2026-11-03T10:00:00Z\",\"message\":{\"role\":\"user\",\"content\":\"prompt %d\"}}\n" i)
                       (format "{\"message\":{\"id\":\"a%d\",\"type\":\"message\",\"role\":\"assistant\",\"content\":[{\"type\":\"tool_use\",\"id\":\"t%d\",\"name\":\"Bash\",\"input\":{\"command\":\"ls\"}}]},\"type\":\"assistant\"}\n" i i)
                       (format "{\"type\":\"user\",\"message\":{\"role\":\"user\",\"content\":[{\"type\":\"tool_result\",\"tool_use_id\":\"t%d\",\"content\":\"a.txt\"}]}}\n" i)
                       (format "{\"message\":{\"id\":\"b%d\",\"type\":\"message\",\"role\":\"assistant\",\"content\":[{\"type\":\"text\",\"text\":\"One **file**.\"}]},\"type\":\"assistant\",\"timestamp\":\"2026-11-03T10:00:05Z\"}\n" i))))
           (list :path file :id "scaling-view" :harness 'claude :provider "claude" :cwd "/tmp" :title "view"))
  :body (kill-buffer (romacs-agent-history-open it)))

;; A chat of N characters over a fixed 100 turns.
(defun scaling--big-chat (n)
  (scaling--agent)
  (let ((session (romacs-agent--make :id "scaling-big-chat" :harness 'claude :title "big"
                                     :dir temporary-file-directory))
        (reply (concat (make-string (max 1 (/ n 100)) ?x) "\n")))
    (romacs-agent-show session)
    (dotimes (i 100)
      (let ((seq (* 10 i)) (id (format "m%d" i)))
        (romacs-agent-ui--render session (list :type 'input :text "a prompt" :seq (+ seq 1) :last t))
        (romacs-agent-ui--render session (list :type 'delta :text reply :message id :seq (+ seq 2) :last t))
        (romacs-agent-ui--render session (list :type 'done :duration 1.0 :seq (+ seq 3) :last t))))
    (romacs-agent-buffer session)))

;; Composer updates with N messages queued: the count is kept, not taken.
(scaling-probe agent-chat-queue :n 2000
  :setup (let ((session (progn (scaling--agent) (romacs-agent--make :id "scaling-queue-chat" :harness 'claude :title "queue"
                                            :dir temporary-file-directory :state 'running))))
           (romacs-agent-show session)
           (dotimes (i n) (romacs-agent-send-later session (format "message %d" i)))
           (romacs-agent-buffer session))
  :body (with-current-buffer it (dotimes (_ 200) (romacs-agent-ui--composer-update))))

;; M-p through a history of N prompts, then M-n back: each step one cell.
(scaling-probe agent-chat-history :n 2000
  :setup (let ((session (progn (scaling--agent) (romacs-agent--make :id "scaling-history-chat" :harness 'claude :title "history"
                                            :dir temporary-file-directory))))
           (romacs-agent-show session)
           (with-current-buffer (romacs-agent-buffer session)
             (setq romacs-agent-ui--history (scaling--names "prompt" n))
             (goto-char (point-max))
             (current-buffer)))
  :body (with-current-buffer it
          (dotimes (_ n) (romacs-agent-chat-history-previous))
          (dotimes (_ n) (romacs-agent-chat-history-next))))

;; A resize of that chat refits its 100 rules: flat in the transcript's size.
(scaling-probe agent-chat-refit :n 100000 :k 10
  :setup (scaling--big-chat n)
  :body (with-current-buffer it
          (dolist (width '(61 62 63 64 65))
            (cl-letf (((symbol-function 'romacs-agent-ui--rule-width) (lambda () width)))
              (romacs-agent-ui--refit-rules)))))

;; A resize of a chat of N turns refits its N rules once each: linear.
(scaling-probe agent-chat-refit-rules :n 200
  :setup (progn (scaling--agent)
                (let ((session (romacs-agent--make :id "scaling-rules-chat" :harness 'claude :title "rules"
                                                   :dir temporary-file-directory)))
                  (romacs-agent-show session)
                  (dotimes (i n)
                    (romacs-agent-ui--render session (list :type 'input :text "p" :seq (+ (* 10 i) 1) :last t))
                    (romacs-agent-ui--render session (list :type 'done :duration 1.0 :seq (+ (* 10 i) 2) :last t)))
                  (romacs-agent-buffer session)))
  :body (with-current-buffer it
          (dolist (width '(61 62 60 63 59))
            (cl-letf (((symbol-function 'romacs-agent-ui--rule-width) (lambda () width)))
              (romacs-agent-ui--refit-rules)))))

;; Keys typed into that chat's composer (with the post-command hook): flat.
(scaling-probe agent-chat-typing :n 100000 :k 10
  :setup (with-current-buffer (scaling--big-chat n) (goto-char (point-max)) (current-buffer))
  :body (with-current-buffer it
          (dotimes (_ 50)
            (let ((last-command-event ?a) (this-command 'romacs-agent-chat-self-insert))
              (call-interactively #'romacs-agent-chat-self-insert)
              (run-hooks 'post-command-hook)))
          (dotimes (_ 10)
            (let ((this-command 'romacs-agent-chat-backward-kill-word))
              (call-interactively #'romacs-agent-chat-backward-kill-word)
              (run-hooks 'post-command-hook)))))

;; A chat of N whole turns: the prompt, a streamed reply, its Markdown, the footer.
(scaling-probe agent-chat-turns :n 100
  :setup (progn (scaling--agent)
                (let ((session (romacs-agent--make :id "scaling-turns-chat" :harness 'claude :title "turns"
                                                   :dir temporary-file-directory)))
                  (romacs-agent-show session)
                  session))
  :body (dotimes (i n)
          (let ((seq (* 10 i)) (id (format "m%d" i)))
            (romacs-agent-ui--render it (list :type 'input :text "a prompt" :seq (+ seq 1) :last t))
            (dotimes (k 4)
              (romacs-agent-ui--render it (list :type 'delta :text "some **reply** text " :message id :seq (+ seq 2 k) :last t)))
            (romacs-agent-ui--render it (list :type 'message-end :message id :seq (+ seq 7) :last t))
            (romacs-agent-ui--render it (list :type 'done :seq (+ seq 8) :last t :duration 1.0)))))

;;;; Session images (rlisp/warm-restart.el)

(defun scaling--session (n)
  "N text buffers, and a private image directory."
  (require 'warm-restart)
  (setq warm-restart-directory (make-temp-file "scaling-images-" t))
  (dolist (name (scaling--names "scaling-session" n))
    (with-current-buffer (get-buffer-create name) (text-mode) (insert name "\nsecond line\n"))))

;; An image of a session of N buffers written.
(scaling-probe session-image-write :n 200
  :setup (progn (scaling--session n) (expand-file-name "w.image" warm-restart-directory))
  :body (warm-restart-write nil it))

;; An image of N buffers restored into a session without them.
(scaling-probe session-image-restore :n 200
  :setup (progn (scaling--session n)
                (let ((file (expand-file-name "r.image" warm-restart-directory)))
                  (warm-restart-write nil file)
                  (dolist (name (scaling--names "scaling-session" n)) (kill-buffer name))
                  file))
  :body (warm-restart-restore-image it))

;; An image of N file buffers, every other one with unsaved text,
;; restored into a session without them.
(scaling-probe session-image-restore-files :n 100
  :setup (let ((dir (make-temp-file "scaling-files-" t)) (create-lockfiles nil))
           (scaling--session 0)
           (dolist (name (scaling--names "scaling-file" n))
             (let ((file (expand-file-name name dir)))
               (with-temp-file file (insert name "\n"))
               (with-current-buffer (find-file-noselect file)
                 (when (cl-evenp (length (buffer-list))) (insert "unsaved\n")))))
           (let ((image (expand-file-name "f.image" warm-restart-directory)))
             (warm-restart-write nil image)
             (dolist (buffer (buffer-list))
               (when (buffer-file-name buffer)
                 (with-current-buffer buffer (set-buffer-modified-p nil))
                 (kill-buffer buffer)))
             image))
  :body (warm-restart-restore-image it))

;; N Hermes chats restored through Hermes's own desktop handler beside N
;; text buffers, then attached at activation (offline: no dashboard).
(defun scaling--offline-hermes (function)
  "Call FUNCTION with Hermes's dashboard transport off."
  (cl-letf (((symbol-function 'hermes-chat--dashboard-default-transport-p) #'ignore))
    (funcall function)))

(scaling-probe session-image-restore-mixed :n 50
  :setup (progn
           (scaling--session n)
           (add-to-list 'load-path (expand-file-name "hermes" (file-name-directory (locate-library "warm-restart"))))
           (require 'hermes-chat)
           (scaling--offline-hermes
            (lambda ()
              (let ((warm-restart-passive t) (warm-restart-activate-functions nil))
                (dolist (name (scaling--names "scaling-chat" n))
                  (hermes-chat-desktop-restore nil name (list :version 1 :session-id name :title name))))
              (setq hermes-chat--warm-restart-attaches nil)))
           (let ((file (expand-file-name "m.image" warm-restart-directory)))
             (warm-restart-write nil file)
             (dolist (buffer (buffer-list))
               (when (string-match-p "\\`scaling-\\(chat\\|session\\)" (buffer-name buffer))
                 (with-current-buffer buffer (set-buffer-modified-p nil))
                 (let ((kill-buffer-query-functions nil)) (kill-buffer buffer))))
             file))
  :body (scaling--offline-hermes (lambda () (warm-restart-restore-image it))))

;; The restore's lock pass over N modified file buffers.
(scaling-probe session-image-locks :n 100
  :setup (let ((dir (make-temp-file "scaling-locks-" t)))
           (require 'warm-restart)
           (mapcar (lambda (name)
                     (let ((file (expand-file-name name dir)))
                       (with-temp-file file (insert "text\n"))
                       (with-current-buffer (let ((create-lockfiles nil)) (find-file-noselect file))
                         (let ((create-lockfiles nil)) (insert "edit"))
                         (current-buffer))))
                   (scaling--names "scaling-lock" n)))
  :body (warm-restart--lock it))

;; The same pass when another editor holds every one of those locks.
(scaling-probe session-image-locks-contended :n 100
  :setup (let ((dir (make-temp-file "scaling-contended-" t)))
           (require 'warm-restart)
           (mapcar (lambda (name)
                     (let ((file (expand-file-name name dir)))
                       (with-temp-file file (insert "text\n"))
                       (make-symbolic-link "someone@elsewhere.1:0" (expand-file-name (concat ".#" name) dir))
                       (with-current-buffer (let ((create-lockfiles nil)) (find-file-noselect file))
                         (let ((create-lockfiles nil)) (insert "edit"))
                         (current-buffer))))
                   (scaling--names "scaling-contended" n)))
  :body (warm-restart--lock it))

;; Cursor motion in a truncated window, which places point's line for its
;; hscroll each redisplay: N faces defined, then N distinct faces on
;; point's line.  Setup returns the buffer: a step's timer runs in whatever
;; buffer is current then.
(scaling-probe hscroll-faces-defined :gui t :n 500 :steps 40
  :setup (progn (dotimes (i n) (custom-declare-face (intern (format "scaling-hs-%d" i)) '((t :weight bold)) "x" :group 'scaling))
                (switch-to-buffer (get-buffer-create "hs")) (erase-buffer)
                (insert (make-string 400 ?x) "\n") (setq-local truncate-lines t) (goto-char 10)
                (current-buffer))
  :body (with-current-buffer it (if (cl-evenp i) (forward-char 1) (backward-char 1))))

(scaling-probe hscroll-faces-on-line :gui t :n 500 :steps 40
  :setup (progn (switch-to-buffer (get-buffer-create "hs")) (erase-buffer)
                (dotimes (i n) (let ((face (intern (format "scaling-hs-%d" i))))
                                 (custom-declare-face face '((t :weight bold)) "x" :group 'scaling)
                                 (insert (propertize "x" 'face face))))
                (insert "\n") (setq-local truncate-lines t) (goto-char 10)
                (current-buffer))
  :body (with-current-buffer it (if (cl-evenp i) (forward-char 1) (backward-char 1))))

;; Motion and measurement in one long wrapped line of N words, around its
;; middle: a step's rows are laid out from the line's start, so the work
;; is linear in the line; a rescan per row would make it quadratic.
(defun scaling--wrapped-line (n word-wrap)
  "Show one line of N words, wrapped at words when WORD-WRAP; return its buffer."
  (switch-to-buffer (get-buffer-create "scaling-wrapped")) (erase-buffer)
  (dotimes (i n) (insert (format "word%d " i)))
  (insert "\n")
  (setq-local truncate-lines nil)
  (if word-wrap (visual-line-mode 1) (setq-local word-wrap nil))
  (goto-char (/ (point-max) 2))
  (current-buffer))

(defun scaling--wrapped-step (i)
  "Move vertically from the middle, to the visual line's ends, and measure."
  (goto-char (/ (point-max) 2))
  (vertical-motion (if (cl-evenp i) 3 -3))
  (end-of-visual-line)
  (beginning-of-visual-line)
  (window-text-pixel-size nil (line-beginning-position) (point)))

(scaling-probe wrapped-line-words :gui t :n 1000 :steps 20
  :setup (scaling--wrapped-line n t)
  :body (with-current-buffer it (scaling--wrapped-step i)))

(scaling-probe wrapped-line-continued :gui t :n 1000 :steps 20
  :setup (scaling--wrapped-line n nil)
  :body (with-current-buffer it (scaling--wrapped-step i)))

;;;; Running

(defun scaling--probe (name)
  (or (alist-get name scaling-probes) (error "No scaling probe %s" name)))

(defun scaling--print (plist)
  "Print PLIST as one SCALING line of JSON."
  (princ (format "SCALING %s\n" (json-encode plist))))

(defun scaling-list ()
  "Print each probe's parameters, one SCALING line each."
  (require 'json)
  (dolist (probe (reverse scaling-probes))
    (let ((plist (cdr probe)))
      (scaling--print (list :name (symbol-name (car probe)) :n (plist-get plist :n)
                            :k (or (plist-get plist :k) 4) :budget (or (plist-get plist :budget) 1.25)
                            :known (or (plist-get plist :known) nil) :gui (if (plist-get plist :gui) t :json-false)
                            :steps (or (plist-get plist :steps) 1) :stack (if (plist-get plist :stack) t :json-false))))))

(defun scaling--perf (command)
  "Send COMMAND to the `perf record --control' fifo SCALING_PERF_CTL names, if any.
scaling_live.py --record profiles a probe's body alone this way."
  (let ((fifo (getenv "SCALING_PERF_CTL")))
    (when fifo
      (call-process "sh" nil nil nil "-c" (format "printf '%s\\n' > %s" command (shell-quote-argument fifo)))
      (sleep-for 0.1))))

(defun scaling-run (name n)
  "Run batch probe NAME at size N: print its counted instructions."
  (require 'json)
  (let* ((plist (scaling--probe name))
         (it (funcall (plist-get plist :setup) n))
         (body (plist-get plist :body))
         (start nil) (end nil))
    (garbage-collect)
    (setq scaling-stack-marks nil)
    (scaling--perf "enable")
    (let ((gc-cons-threshold most-positive-fixnum) (gc-cons-percentage 1.0))
      (setq start (romacs-instructions))
      (funcall body n it)
      (setq end (romacs-instructions)))
    (scaling--perf "disable")
    (scaling--print (list :name (symbol-name name) :n n
                          :instructions (if (and start end) (- end start) nil)
                          :stack (vconcat (nreverse scaling-stack-marks))))))

(defun scaling--mark (plist)
  "Append PLIST as a JSON line to stderr, among the host's paint diagnostics."
  (write-region (concat (json-encode plist) "\n") nil "/dev/stderr" t 'silent))

(defun scaling-gui-start (name n out)
  "Run GUI probe NAME at size N in this session, then exit.
Once the session is idle, set the probe up; then run its steps from a chain
of timers, the host laying out each step's change as the command loop
returns; count the instructions from before the first step to after the
last step's layout; write (:name :n :instructions) to OUT as JSON and kill
Emacs.  Stderr marks the start, each step and the end among the paint
diagnostics, so the harness can check a paint was submitted after each
step, before the next step or the count's end."
  (require 'json)
  (let* ((plist (scaling--probe name))
         (steps (or (plist-get plist :steps) 1))
         (body (plist-get plist :body))
         (gap (or (plist-get plist :gap) 0.1)) it start)
    (setq blink-cursor-mode nil)
    (run-with-idle-timer
     0.5 nil
     (lambda ()
       (setq it (funcall (plist-get plist :setup) n))
       (garbage-collect)
       (run-with-timer
        0.5 nil
        (lambda ()
          (scaling--mark (list :scaling "start"))
          (setq start (romacs-instructions))
          (let ((step nil) (i 0))
            (setq step (lambda ()
                         (if (< i steps)
                             (progn (scaling--mark (list :scaling "step" :i i))
                                    (funcall body n it i) (setq i (1+ i)) (run-with-timer gap nil step))
                           (let ((end (romacs-instructions)))
                             (scaling--mark (list :scaling "end"))
                             (with-temp-file out
                               (insert (json-encode (list :name (symbol-name name) :n n
                                                          :instructions (and start end (- end start))))))
                             (kill-emacs 0)))))
            (funcall step))))))))

(provide 'scaling-probes)
;;; probes.el ends here
