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
;;   :gap    how often, in seconds, a GUI probe looks for its step's
;;           redisplay (default 0.1).
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
;; catppuccin-latte in a -Q window, most of it loading the theme file; nearly flat in the faces, which only the root frame
;; recomputes (VM2-CHILD-FRAME).
(scaling-probe faces-theme :gui t :n 50 :steps 1
  :setup (let ((faces (mapcar #'intern (scaling--names "scaling-face" n))))
           (dolist (face faces)
             (custom-declare-face face '((t :inherit font-lock-keyword-face)) "A package face." :group 'scaling))
           faces)
  :body (load-theme 'catppuccin-latte t))

;; N Omarchy themes Romacs does not ship, made one after another, the first
;; made again: Custom's theme list and `features' grow by one each.  Sessions
;; meet tens of them.
(scaling-probe omarchy-generated-themes :n 40
  :setup (progn
           (require 'romacs-omarchy)
           (cons (mapcar #'intern (scaling--names "omarchy-scaling" n))
                 '((mode dark) (bg "#101418") (fg "#d8dee9") (red "#e06c75") (green "#98c379")
                   (yellow "#e5c07b") (blue "#61afef") (magenta "#c678dd") (cyan "#56b6c2"))))
  :body (let ((palette (cdr it)))
          (dolist (theme (car it)) (omarchy--define theme "scaling" palette))
          (omarchy--define (car (car it)) "scaling" palette)))

;; Registering N bundled themes, as a load of romacs-theme.el does: a property
;; each, no scan of the table per theme.
(scaling-probe theme-bundled-rows :n 4000
  :setup (progn
           (require 'romacs-theme)
           (mapcar (lambda (name) (list (intern name) 'dark (intern (concat name "-family"))))
                   (scaling--names "scaling-theme" n)))
  :body (romacs-theme--register it))

;; A theme's role resolution over N more palette rows: an index, not a scan
;; per name.
(scaling-probe theme-palette-rows :n 2000
  :setup (progn
           (require 'romacs-theme-roles)
           (append '((mode dark) (bg "#101418") (fg "#d8dee9") (red "#e06c75") (green "#98c379")
                     (yellow "#e5c07b") (blue "#61afef") (magenta "#c678dd") (cyan "#56b6c2"))
                   (mapcar (lambda (name) (list (intern name) "#123456")) (scaling--names "scaling-slot" n))))
  :body (romacs-theme-roles-resolve it))

;; The floor pass and its merge over N face rows, a tenth of them failing on
;; one pair: each pair is computed once, each row merged once.
(scaling-probe theme-floor-rows :n 4000
  :setup (progn
           (require 'romacs-theme-roles)
           (mapcar (lambda (name)
                     (list (intern name) (list (list t :foreground (if (string-suffix-p "1" name) "#333333" "#eeeeee")))))
                   (scaling--names "scaling-floor" n)))
  :body (let ((moves (romacs-theme-roles-floors it '((bg . "#101418")))))
          (romacs-theme--apply-delta it (mapcar (lambda (move) (list (car move) :foreground (nth 2 move))) moves))))

;; The floor pass over an `:inherit' chain N deep: each ancestor is read once.
(scaling-probe theme-inherit-depth :n 2000
  :setup (progn
           (require 'romacs-theme-roles)
           (let ((names (mapcar #'intern (scaling--names "scaling-inherit" n))) (rows nil) (parent nil))
             (dolist (name names (nreverse rows))
               (push (list name (list (if parent
                                          (list t :inherit parent :background "#101418")
                                        (list t :foreground "#eeeeee"))))
                     rows)
               (setq parent name))))
  :body (romacs-theme-roles-floors it '((bg . "#101418"))))

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

;; The same buffer parsed, then a hundred more parsers of it, each like
;; c-ts-mode's `for-each' one: a first parse copies the tree, so none costs
;; anything in N.
(scaling-probe treesit-twin-parser :n 2000 :budget 0.1
  :setup (with-current-buffer (get-buffer-create " scaling-treesit-twin")
           (dotimes (i n)
             (insert (format "fn f%d(x: u32) -> u32 { let y = x + %d; y * 2 }\n" i i)))
           (treesit-parser-root-node (treesit-parser-create 'rust))
           (current-buffer))
  :body (with-current-buffer it
          (dotimes (i 100)
            (treesit-parser-root-node
             (treesit-parser-create 'rust nil nil (intern (format "twin-%d" i)))))))

;; The same buffer's first parse on another thread: what reaches the
;; evaluating thread is the tree, so its parser and a hundred more cost
;; nothing in N.
(scaling-probe treesit-early-parse :n 2000 :budget 0.1
  :setup (with-current-buffer (get-buffer-create " scaling-treesit-early")
           (dotimes (i n)
             (insert (format "fn f%d(x: u32) -> u32 { let y = x + %d; y * 2 }\n" i i)))
           (romacs--treesit-parse-early 'rust)
           (current-buffer))
  :body (with-current-buffer it
          (treesit-parser-root-node (treesit-parser-create 'rust))
          (dotimes (i 100)
            (treesit-parser-root-node
             (treesit-parser-create 'rust nil nil (intern (format "twin-%d" i)))))))

;; Edits made while it runs, which its tree catches up with: linear in the
;; edits, nothing in the text.
(scaling-probe treesit-early-edits :n 200
  :setup (with-current-buffer (get-buffer-create " scaling-treesit-early-edits")
           (dotimes (i 2000)
             (insert (format "fn f%d(x: u32) -> u32 { let y = x + %d; y * 2 }\n" i i)))
           (romacs--treesit-parse-early 'rust)
           (current-buffer))
  :body (with-current-buffer it
          (dotimes (i n)
            (goto-char (* 40 (1+ i)))
            (insert "x"))
          (treesit-parser-root-node (treesit-parser-create 'rust))))

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

;; A header line of N columns, each its own face run then a `(space
;; :align-to ...)' (VM2-MODE-LINE-STRETCH), redisplayed after each key in
;; the buffer under it: the band's stretches resolve with the window's
;; geometry read once, not once per stretch, and the face runs once each.
(scaling-probe header-line-stretches :gui t :n 40 :steps 20
  :setup (let ((buffer (scaling--lines-buffer "scaling-header" 50)))
           (switch-to-buffer buffer)
           (setq header-line-format
                 (cl-loop for i below n append
                          (list (propertize (format "c%d" i) 'face (if (cl-oddp i) 'bold 'italic))
                                (propertize " " 'display `(space :align-to ,(* 3 (1+ i)))))))
           buffer)
  :body (with-current-buffer it (insert "x")))

;; A buffer whose `face-remapping-alist' remaps N faces, shown in the
;; status bar and its window, redisplayed after each key in it: its
;; remapped face table is realized once and found again per redisplay
;; (`display_faces_for'), for the window's text and bands and for the
;; bar's cache key (VM2-FACE-REMAP).
(scaling-probe face-remap-bands :gui t :n 32 :steps 20
  :setup (progn
           (require 'romacs-mode-line)
           (romacs-mode-line-mode 1)
           (let ((buffer (scaling--lines-buffer "scaling-remap" 50)))
             (switch-to-buffer buffer)
             (setq header-line-format "remapped")
             (setq-local face-remapping-alist
                         (cl-loop for i below n
                                  collect (let ((face (intern (format "remap-probe-face-%d" i))))
                                            (make-face face)
                                            (list face (list :foreground (format "#%02x6080" (% (* 7 i) 256)))))))
             (push '(mode-line-active (:background "#203040")) face-remapping-alist)
             buffer))
  :body (with-current-buffer it (insert "x")))

;; The same remapping in the selected window, the bar's owner, while the
;; keys go to a buffer in the other: the bar's line is unchanged, so each
;; request finds its last expansion by its key, which reads the remapping
;; again (`bar_key').
(scaling-probe face-remap-bar-unchanged :gui t :n 256 :steps 20
  :setup (progn
           (require 'romacs-mode-line)
           (romacs-mode-line-mode 1)
           (let ((other (scaling--lines-buffer "scaling-remap-other" 50)))
             (switch-to-buffer (scaling--lines-buffer "scaling-remap-still" 50))
             (setq-local face-remapping-alist
                         (cl-loop for i below n
                                  collect (let ((face (intern (format "remap-still-face-%d" i))))
                                            (make-face face)
                                            (list face (list :foreground (format "#%02x6080" (% (* 7 i) 256)))))))
             (set-window-buffer (split-window-below) other)
             other))
  :body (with-current-buffer it (insert "x")))

;; The same bar with a box, and a context window's line folded in: the
;; stretches of its box run, and the context's background swept with the
;; characters, never a walk over the segments for each one.  N stays small
;; enough to fit: a bar too full drops the context and clips the pills.
;; The body refuses a run whose bar, at its widest after the last key, has
;; lost the context, which follows every pill, so the pills are shown too.
(scaling-probe mode-line-bar-boxed :gui t :n 6 :steps 20
  :setup (progn
           (require 'romacs-mode-line)
           (romacs-mode-line-mode 1)
           (set-face-attribute 'mode-line nil :box '(:line-width 1 :color "#808080"))
           (dotimes (i n)
             (let ((face (intern (format "bar-boxed-face-%d" i))))
               (make-face face)
               (set-face-attribute face nil :background (format "#%02x3040" (% (* 5 i) 256))
                                   :foreground (format "#e0%02xa0" (% (* 3 i) 256)))
               (eval `(romacs-mode-line-define-segment ,(intern (format "bar-boxed-%d" i)) "A probe."
                        :pill ,face :pill-text ,face
                        :format (" " (:propertize "%*" romacs-icon ((?* . modified) (?- . saved))) " "))
                     t))
             (push (intern (format "bar-boxed-%d" i)) romacs-mode-line-right))
           (romacs-mode-line-rebuild)
           (let ((buffer (scaling--lines-buffer "scaling-bar-boxed" 50)))
             (switch-to-buffer buffer)
             (with-selected-window (split-window-below)
               (switch-to-buffer (get-buffer-create "scaling-bar-context"))
               (setq-local mode-line-format '("q  " "context")))
             buffer))
  :body (progn
          (with-current-buffer it (insert "x"))
          (when (= i 19)
            (let ((shown (romacs-status-bar-string)))
              (unless (string-search "context" shown)
                (scaling--mark (list :scaling "unfit" :shown shown))
                (kill-emacs 3))))))

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

;; A key grid of N entries: one tall group of N/2 and N/2 more in groups
;; of four, laid out in bands; linear, each row one `aref' per block.
(scaling-probe menu-key-grid :gui t :n 400 :steps 1
  :setup (let ((entry (lambda (i) (cons (format "k%d" i) (format "command %d" i)))))
           (cons (switch-to-buffer (get-buffer-create "scaling-grid"))
                 (cons (cons "Tall" (mapcar entry (number-sequence 1 (/ n 2))))
                       (seq-map-indexed (lambda (_ g) (cons (format "G%d" g) (mapcar entry (number-sequence 1 4))))
                                        (make-list (/ n 8) nil)))))
  :body (progn (romacs-popup-key-grid (cdr it) 160)
               (with-current-buffer (car it) (insert "x"))))

;; One `set-transient-map' activation and exit at depth 1 with N other
;; maps registered (retained by the setup): the hints' registration and
;; reconcile do the same work whatever N is.
(scaling-probe menu-hints-activate :gui t :n 100 :steps 10
  :setup (let ((maps (mapcar (lambda (_) (define-keymap "a" #'ignore)) (make-list n nil))))
           (dolist (map maps) (funcall (set-transient-map map)))
           (cons (switch-to-buffer (get-buffer-create "scaling-hints")) maps))
  :body (progn (funcall (set-transient-map (define-keymap "b" #'ignore)))
               (with-current-buffer (car it) (insert "x"))))

;; Hint entries for a map of N bindings whose sparse child rebinds or
;; unbinds half of them: one traversal, a seen-key hash, no lookup per key.
(scaling-probe menu-hints-keys :gui t :n 400 :steps 1
  :setup (let ((parent (make-sparse-keymap)) (child (make-sparse-keymap)))
           (dotimes (i n) (define-key parent (vector (+ #x1000 i)) #'ignore))
           (dotimes (i (/ n 2)) (define-key child (vector (+ #x1000 (* 2 i))) (and (cl-oddp i) #'forward-char)))
           (set-keymap-parent child parent)
           (cons (switch-to-buffer (get-buffer-create "scaling-hint-keys")) child))
  :body (progn (romacs-popup--hint-entries (cdr it))
               (with-current-buffer (car it) (insert "x"))))

;; Hint entries for a child of N character ranges over a parent of 2N
;; keys inside and beside them and a grandparent of N ranges overlapping
;; them all: each interval painted once over the sorted bounds, never a
;; scan of the keys per range.
(scaling-probe menu-hints-ranges :gui t :n 200 :steps 1
  :setup (let ((grand (make-keymap)) (parent (make-sparse-keymap)) (child (make-keymap)))
           (dotimes (i n)
             (let ((lo (+ #x1000 (* 10 i))))
               (set-char-table-range (nth 1 child) (cons lo (+ lo 3)) (if (cl-oddp i) #'forward-char #'backward-char))
               (define-key parent (vector (+ lo 1)) #'ignore)
               (define-key parent (vector (+ lo 5)) #'ignore)
               (set-char-table-range (nth 1 grand) (cons (+ lo 2) (+ lo 7)) (if (cl-oddp i) #'undo #'redo))))
           (set-keymap-parent parent grand)
           (set-keymap-parent child parent)
           (cons (switch-to-buffer (get-buffer-create "scaling-hint-ranges")) child))
  :body (progn (romacs-popup--hint-entries (cdr it))
               (with-current-buffer (car it) (insert "x"))))

;; The hint target after N panels shown and their buffers killed: a
;; killed panel leaves the registry, so the target asks about none.
(scaling-probe popup-panels-cycle :gui t :n 20 :steps 10
  :setup (progn (dotimes (i n)
                  (let ((buffer (get-buffer-create (format " scaling-panel-%d" i))))
                    (with-current-buffer buffer (insert "x"))
                    (romacs-popup-show 'panel buffer :lines 1)
                    (kill-buffer buffer)))
                (set-transient-map (define-keymap "a" #'ignore) t)
                (switch-to-buffer (get-buffer-create "scaling-panels")))
  :body (progn (romacs-popup--hint-target)
               (with-current-buffer it (insert "x"))))

;; A key-hint card for a prefix of N bindings over M active minor-mode
;; maps that bind under it and R which-key replacements: one traversal of
;; each map, one replacement pass per entry, one layout; linear in each.
(scaling-probe key-card-bindings :gui t :n 100 :steps 1
  :setup (let ((map (make-sparse-keymap)))
           (dotimes (i n) (define-key map (vector 'f12 (+ #x1000 i)) #'forward-char))
           (use-local-map map)
           (switch-to-buffer (get-buffer-create "scaling-card")))
  :body (progn (romacs-popup--card-layout [f12]) (with-current-buffer it (insert "x"))))

(scaling-probe key-card-maps :gui t :n 20 :steps 1
  :setup (progn
           (dotimes (i n)
             (let ((map (make-sparse-keymap)) (mode (intern (format "scaling-card-mode-%d" i))))
               (define-key map (vector 'f12 (+ #x1000 i)) #'forward-char)
               (set mode t)
               (push (cons mode map) minor-mode-map-alist)))
           (switch-to-buffer (get-buffer-create "scaling-card-maps")))
  :body (progn (romacs-popup--card-layout [f12]) (with-current-buffer it (insert "x"))))

(scaling-probe key-card-replacements :gui t :n 50 :steps 1
  :setup (progn
           (require 'which-key)
           (let ((map (make-sparse-keymap)))
             (dotimes (i 40) (define-key map (vector 'f12 (+ #x1000 i)) #'forward-char))
             (use-local-map map))
           (dotimes (i n) (which-key-add-key-based-replacements (format "<f11> %d" i) "scaling"))
           (switch-to-buffer (get-buffer-create "scaling-card-repl")))
  :body (progn (romacs-popup--card-layout [f12]) (with-current-buffer it (insert "x"))))

;; One more package entry written to a `mode-line-modes' of N: the
;; watcher rebuilds the extras once per write, linear in the entries,
;; and nothing per key or redisplay.
(scaling-probe mode-line-modes-writes :gui t :n 50 :steps 10
  :setup (let ((saved (default-value 'mode-line-modes)))
           (dotimes (i n) (add-to-list 'mode-line-modes (list t (format "[p%d] " i)) t))
           (cons (switch-to-buffer (get-buffer-create "scaling-modes")) saved))
  :body (progn (setq-default mode-line-modes (cons (list t (format "[w%d] " i)) (cdr (default-value 'mode-line-modes))))
               (with-current-buffer (car it) (insert "x"))))

;; One of N panels kept shown hidden and shown again: the panel registry
;; adds and removes it in O(1), the hint target stops at the first shown.
(scaling-probe popup-panels-kept :gui t :n 10 :steps 10
  :setup (let (buffers)
           (dotimes (i n)
             (let ((buffer (get-buffer-create (format " scaling-kept-panel-%d" i))))
               (with-current-buffer buffer (insert "x"))
               (romacs-popup-show 'panel buffer :lines 1)
               (push buffer buffers)))
           ;; An active map, so the hint target asks about the panels.
           (set-transient-map (define-keymap "a" #'ignore) t)
           (cons (switch-to-buffer (get-buffer-create "scaling-kept-panels")) (car (last buffers))))
  :body (progn (romacs-popup-hide (cdr it)) (romacs-popup-show 'panel (cdr it) :lines 1)
               (with-current-buffer (car it) (insert "x"))))

;; Typing at the end of N numbered lines: a label comes from the text's line
;; index, not from counting the lines before it.
(scaling-probe line-numbers-at-end :gui t :n 20000 :steps 20 :budget 0.5
  :setup (let ((buffer (switch-to-buffer (get-buffer-create "scaling-line-numbers"))))
           (dotimes (_ n) (insert "a line of the buffer\n"))
           (setq display-line-numbers t)
           (goto-char (point-max))
           buffer)
  :body (with-current-buffer it (insert "x")))

;; Clearing N toasts (the echo area's clearing at the next command) with N
;; more layers kept hidden: each toast hidden once, no restacking per
;; toast, its layer found without a walk over the kept ones.
(scaling-probe toasts-clear :gui t :n 20 :steps 1
  :setup (progn (dotimes (i n) (romacs-layer--frame (get-buffer-create (format " scaling-layer-%d" i))))
                (setq romacs-layer-toast-max n romacs-layer-toast-timeout 600)
                (dotimes (i n) (romacs-layer-toast (format "toast %d" i)))
                n)
  :body (romacs-layer-toast-clear))

;; N toasts following the selection between two windows, with N more
;; layers kept hidden: one restack of N toasts per change, for the root
;; frame only, never one per child frame's own window change.
(scaling-probe toasts-follow :gui t :n 10 :steps 10
  :setup (progn (dotimes (i n) (romacs-layer--frame (get-buffer-create (format " scaling-layer-%d" i))))
                (delete-other-windows) (split-window-right)
                (setq romacs-layer-toast-max n romacs-layer-toast-timeout 600 romacs-layer-toast-position 'window)
                (dotimes (i n) (romacs-layer-toast (format "toast %d" i)))
                n)
  :body (select-window (next-window)))

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

;; A walk through N images: each key reads the matches filtered once.
;; N keys in a chat whose draft carries N images: the tray is drawn once.
(scaling-probe agent-tray-keys :n 2000
  :setup (progn (scaling--agent)
                (require 'romacs-agent-images)
                (scaling--names "scaling-image" n))
  :body (let ((romacs-agent-images--tray-shown nil))
          (cl-letf (((symbol-function 'romacs-agent-images--tray-files) (lambda (_) it))
                    ((symbol-function 'romacs-agent-images--tray-anchor) (lambda (&rest _) '(10 . 20)))
                    ((symbol-function 'romacs-agent-images--tray-draw) #'ignore)
                    ((symbol-function 'romacs-popup-show) #'ignore))
            (dotimes (_ (length it)) (romacs-agent-images-tray-sync)))))

(scaling-probe agent-images-walk :n 2000
  :setup (progn (scaling--agent)
                (require 'romacs-agent-images)
                (romacs-agent-images--state (vconcat (scaling--names "scaling-image" n))))
  :body (let ((state it))
          (dotimes (_ (length (plist-get state :files)))
            (setq state (romacs-agent-images--step state ?\C-n)))))

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
  ;; An inactive timer: none is scheduled, and retiring the last call
  ;; cancels it as it would the real one.
  :body (let ((romacs-mcp--deferred nil) (romacs-mcp--deferred-index (make-hash-table :test #'equal))
              (romacs-mcp--deferred-timer (timer-create)))
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

;; The instances of N Lisp entries and T3's settings of N instances: one
;; read of the file, one name table, linear; read again (as after the file
;; changed) or kept from the last read.
(defun scaling--t3-instances (n)
  (scaling--agent)
  (require 'romacs-agent-t3)
  (let ((file (make-temp-file "scaling-t3-" nil ".json"))
        (instances (make-hash-table :test #'equal)))
    (dotimes (i n)
      (puthash (format "claudeAgent_%d" i)
               (romacs-mcp--object "driver" "claudeAgent" "displayName" (format "Claude %d" i)
                                   "config" (romacs-mcp--object "homePath" (format "/tmp/t3-claude-%d" i)
                                                                "launchArgs" "--chrome --x 'a b'"))
               instances))
    (with-temp-file file (insert (json-serialize (romacs-mcp--object "providerInstances" instances))))
    (cons (list 't3 :settings file)
          (cl-loop for i below n collect (cons (format "claude-lisp-%d" i) (format "/tmp/claude-config-%d" i))))))

(scaling-probe agent-instances :n 1000
  :setup (scaling--t3-instances n)
  :body (let ((romacs-agent-providers it))
          (setq romacs-agent-t3--read nil)
          (romacs-agent-instances t)))

;; The start palette of N instances, each with the harness's catalogue:
;; rows by instance, one table of shown names.
(scaling-probe agent-picker-rows :n 1000
  :setup (progn (scaling--agent)
                (cl-loop for i below n collect (list (format "claude-%d" i) :display "Claude"
                                                     :config-dir (format "/tmp/claude-config-%d" i))))
  :body (let ((romacs-agent-providers it)) (romacs-agent--picker-rows)))

;; N drivers, each a T3 driver name, and T3's settings of N instances of
;; them, read anew: each instance's harness by an index built once.
(scaling-probe agent-instances-drivers :n 500
  :setup (progn (scaling--agent)
                (require 'romacs-agent-t3)
                (dotimes (i n)
                  (romacs-agent-define-harness (intern (format "scaling-tdriver-%d" i)) :lifetime 'persistent
                                               :t3-driver (format "tdriver%d" i)
                                               :config-variable (format "SCALING_TDRIVER_%d_HOME" i)
                                               :config-default (format "/tmp/tdriver-%d" i)))
                (let ((file (make-temp-file "scaling-t3d-" nil ".json"))
                      (instances (make-hash-table :test #'equal)))
                  ;; The first defined is the farthest from the registry's head.
                  (dotimes (i n)
                    (puthash (format "t%d" i) (romacs-mcp--object "driver" (format "tdriver%d" (- n 1 i))
                                                                  "displayName" (format "T %d" i))
                             instances))
                  (with-temp-file file (insert (json-serialize (romacs-mcp--object "providerInstances" instances))))
                  (list (list 't3 :settings file))))
  :body (let ((romacs-agent-providers it))
          (setq romacs-agent-t3--read nil)
          (romacs-agent-instances t)))

(scaling-probe agent-instances-kept :n 1000
  ;; Read once here: the body resolves with the file's read kept.
  :setup (let ((providers (scaling--t3-instances n)))
           (let ((romacs-agent-providers providers)) (romacs-agent-instances t))
           providers)
  :body (let ((romacs-agent-providers it)) (romacs-agent-instances t)))

;; N entries naming no harness Romacs runs, with N drivers defined: each a
;; short reason; the harnesses there are listed once.
(scaling-probe agent-instances-invalid :n 500
  :setup (progn (scaling--agent)
                (dotimes (i n)
                  (romacs-agent-define-harness (intern (format "scaling-idriver-%d" i)) :lifetime 'persistent
                                               :config-variable (format "SCALING_IDRIVER_%d_HOME" i)))
                (cl-loop for i below n collect (list (format "nosuch-%d" i) :harness (intern (format "nosuch-%d" i)))))
  :body (let ((romacs-agent-providers it)) (romacs-agent-instances t)))

;; observe's buffer summary over N buffers and N processes, each with
;; its buffer: which has a process is a table made once, not a walk of
;; the processes per buffer.
(scaling-probe observe-buffers-processes :n 200
  :setup (progn (scaling--agent)
                (require 'romacs-mcp-session)
                (dotimes (i n)
                  (let ((buffer (get-buffer-create (format "scaling-observe-%d" i))))
                    (make-pipe-process :name (format "scaling-observe-%d" i) :buffer buffer :noquery t)))
                nil)
  :body (dotimes (_ 5) (romacs-mcp-session--buffers (romacs-mcp--object))))

;; T3's thread titles: a store of N threads read cold, read again after
;; its file changed (as its write-ahead log does), and kept (the read
;; skipped, 200 lookups).
(defun scaling--t3-store (n)
  (scaling--agent)
  (require 'romacs-agent-t3)
  (let* ((dir (make-temp-file "scaling-t3s-" t))
         (store (expand-file-name "statev2.sqlite" dir))
         (db (sqlite-open store)))
    (sqlite-execute db "create table orchestration_v2_projection_threads (thread_id text, title text)")
    (sqlite-execute db "create table orchestration_v2_projection_provider_threads (thread_id text, payload_json text, updated_at text, provider_instance_id text)")
    (sqlite-transaction db)
    (dotimes (i n)
      (sqlite-execute db "insert into orchestration_v2_projection_threads values (?, ?)" (list (format "t%d" i) (format "Title %d" i)))
      (sqlite-execute db "insert into orchestration_v2_projection_provider_threads values (?, ?, '1', 'claudeAgent')"
                      (list (format "t%d" i) (format "{\"nativeThreadRef\":{\"nativeId\":\"n%d\"}}" i))))
    (sqlite-commit db)
    (sqlite-close db)
    store))

(scaling-probe agent-t3-titles-cold :n 2000
  :setup (scaling--t3-store n)
  :body (progn (clrhash romacs-agent-t3--titles) (romacs-agent-t3-thread-titles it)))

(scaling-probe agent-t3-titles-changed :n 2000
  :setup (let ((store (scaling--t3-store n))) (romacs-agent-t3-thread-titles store) store)
  :body (progn (set-file-times it (time-add (current-time) (random 1000000)))
               (romacs-agent-t3-thread-titles it)))

(scaling-probe agent-t3-titles-kept :n 2000 :budget 0.5
  :setup (let ((store (scaling--t3-store n))) (romacs-agent-t3-thread-titles store) store)
  :body (let ((titles (romacs-agent-t3-thread-titles it)))
          (dotimes (i 200) (gethash (romacs-agent-t3-title-key "claudeAgent" (format "n%d" i)) titles))))

;; The history listed anew with N instances from T3, each a transcript of
;; its own: each row's instance, and its title, by an index.
(scaling-probe agent-history-t3-relist :n 200
  :setup (let* ((root (make-temp-file "scaling-t3h-" t))
                (settings (expand-file-name "t3/settings.json" root))
                (instances (make-hash-table :test #'equal)))
           (scaling--agent)
           (require 'romacs-agent-history)
           (make-directory (file-name-directory settings) t)
           (dotimes (i n)
             (let* ((home (expand-file-name (format "home-%d" i) root))
                    (dir (expand-file-name "projects/-work" home))
                    (id (format "%08x-0000-4000-8000-%012x" i i)))
               (make-directory dir t)
               (with-temp-file (expand-file-name (concat id ".jsonl") dir)
                 (insert (json-serialize (romacs-mcp--object "type" "user" "cwd" "/work"
                                                             "message" (romacs-mcp--object "role" "user" "content" (format "task %d" i))))
                         "\n"))
               (puthash (format "claudeAgent_%d" i)
                        (romacs-mcp--object "driver" "claudeAgent" "displayName" (format "C %d" i)
                                            "config" (romacs-mcp--object "homePath" home))
                        instances)))
           (with-temp-file settings (insert (json-serialize (romacs-mcp--object "providerInstances" instances))))
           (let ((romacs-agent-providers (list (list 't3 :settings settings)))) (romacs-agent-history-list))
           (list (list 't3 :settings settings)))
  :body (let ((romacs-agent-providers it)) (romacs-agent-history-list)))

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

;;;; Jabber: the archive catch-up's collections (rlisp/jabber/jabber-mam.el,
;;;; jabber-mam-sync.el).  A connection here is an FSM symbol with state
;;;; data and no transport: the probes count bookkeeping, not traffic.

(defvar jabber-connections)
(defvar jabber-buffer-connection)
(defvar jabber-group)
(defvar jabber-mam--dirty-peers)
(defvar jabber-mam--room-nicks)
(defvar jabber-mam-sync--accounts)
(defvar jabber-mam-sync--stored)
(defvar jabber-mam-sync--merge-queue)
(defvar jabber-mam-sync--merge-tail)
(defvar jabber-mam-sync--merge-queued)
(defvar jabber-mam-sync--live-jobs)

(defun scaling--jabber ()
  "Jabber loaded, and a connection for romeo@montague.net, made current."
  (require 'jabber)
  (let ((jc (make-symbol "scaling-jc")))
    (put jc :state :session-established)
    (put jc :state-data (list :username "romeo" :server "montague.net" :resource "scaling"
                              :password "pw" :connection-type 'network))
    (setq jabber-connections (list jc))
    jc))

;; N distinct nicknames rendered on one background, each twice: the
;; colour cache (jabber-muc.el) computes each once.  kN stays under its
;; bound (`jabber-muc-nick-colors-limit', 4096), past which it starts over.
(defvar jabber-muc--nick-colors)
(scaling-probe jabber-nick-colors :n 1000
  :setup (progn (scaling--jabber) (clrhash jabber-muc--nick-colors) (scaling--names "nick" n))
  :body (dotimes (_ 2) (dolist (nick it) (jabber-muc--nick-face nick))))

;; N rooms requested for one account's background catch-up.
(scaling-probe jabber-sync-requests :n 400
  :setup (let ((jc (scaling--jabber))) (clrhash jabber-mam-sync--accounts) (cons jc (scaling--names "room" n)))
  :body (dolist (room (cdr it)) (jabber-mam-sync-request (car it) room)))

;; N accounts, each requesting its personal archive.
(scaling-probe jabber-sync-accounts :n 400
  :setup (progn
           (scaling--jabber)
           (clrhash jabber-mam-sync--accounts)
           (mapcar (lambda (name)
                     (let ((jc (make-symbol name)))
                       (put jc :state :session-established)
                       (put jc :state-data (list :username name :server "montague.net" :resource "r"))
                       jc))
                   (scaling--names "user" n)))
  :body (dolist (jc it) (jabber-mam-sync-request jc)))

;; N accounts, each through a whole job: requested, started, finished
;; (no worker: the isolate functions answer at once), all retained.
(scaling-probe jabber-sync-lifecycle :n 300
  :setup (progn
           (scaling--jabber)
           (clrhash jabber-mam-sync--accounts)
           (setq jabber-mam-sync--live-jobs 0)
           (mapcar (lambda (name)
                     (let ((jc (make-symbol name)))
                       (put jc :state :session-established)
                       (put jc :state-data (list :username name :server "montague.net"
                                                 :resource "r" :password "pw"))
                       jc))
                   (scaling--names "user" n)))
  :body (let (callbacks)
          (cl-letf (((symbol-function 'isolate-spawn) (lambda (&rest _) 'future))
                    ((symbol-function 'isolate-on-done) (lambda (_f callback) (push callback callbacks)))
                    ((symbol-function 'isolate-await) (lambda (&rest _) '(:archives nil :peers nil)))
                    ((symbol-function 'isolate-future-isolate) (lambda (_) 'worker))
                    ((symbol-function 'isolate-live-p) #'ignore)
                    ((symbol-function 'thaw) #'identity)
                    ((symbol-function 'jabber-mam-sync--toast) #'ignore)
                    ((symbol-function 'jabber-db-share-writes) #'ignore))
            (dolist (jc it)
              (jabber-mam-sync-request jc)
              (jabber-mam-sync--start (jabber-connection-bare-jid jc)))
            (dolist (callback callbacks) (funcall callback 'future)))))

;; N finished jobs' merges, one changed peer and its open buffer each.
(scaling-probe jabber-sync-merge-repeat :n 400
  :setup (let ((jc (scaling--jabber)) summaries)
           (setq jabber-mam-sync--merge-queue nil jabber-mam-sync--merge-tail nil)
           (clrhash jabber-mam-sync--merge-queued)
           (dolist (room (scaling--names "room" n))
             (with-current-buffer (get-buffer-create (concat " *scaling-merge-" room "*"))
               (setq major-mode 'jabber-chat-mode)
               (setq-local jabber-buffer-connection jc)
               (setq-local jabber-group room)
               (jabber-buffer-registry-register 'muc (list "romeo@montague.net" room)))
             (push (list :peers (list (list "romeo@montague.net" room "groupchat" 1 1))) summaries))
           summaries)
  :body (dolist (summary it) (jabber-mam-sync--merge summary)))

;; A finished job's N changed peers against N open (hidden) chat buffers:
;; the histories are marked changed; no buffer is looked for.
(scaling-probe jabber-sync-merge :n 400
  :setup (let ((jc (scaling--jabber)) peers)
           (setq jabber-mam-sync--merge-queue nil)
           (dolist (room (scaling--names "room" n))
             (with-current-buffer (get-buffer-create (concat " *scaling-" room "*"))
               (setq major-mode 'jabber-chat-mode)
               (setq-local jabber-buffer-connection jc)
               (setq-local jabber-group room)
               (jabber-buffer-registry-register 'muc (list "romeo@montague.net" room)))
             (push (list "romeo@montague.net" room "groupchat" 1 1) peers))
           (list :peers peers))
  :body (jabber-mam-sync--merge it))

;; N archived results marking N/2 peers dirty, twice each, and their page's redraw.
(defvar jabber-mam--redraw-peers)
(defvar jabber-mam--redraw-timer)

;; Pages of a catch-up over N peers: each page marks them dirty and
;; schedules their redraw, as `jabber-mam--accept-page' does.
(scaling-probe jabber-mam-redraw-schedule :n 2000
  :setup (progn (setq jabber-mam--dirty-peers nil jabber-mam--redraw-peers nil)
                (cons (scaling--jabber) (scaling--names "peer" n)))
  :body (dotimes (_ 3)
          (dolist (peer (cdr it)) (jabber-mam--mark-dirty (car it) peer "chat"))
          (jabber-mam--schedule-redraw jabber-mam--dirty-peers)
          (when jabber-mam--redraw-timer (cancel-timer jabber-mam--redraw-timer)
                (setq jabber-mam--redraw-timer nil))))

(scaling-probe jabber-mam-dirty-peers :n 4000
  :setup (progn (setq jabber-mam--dirty-peers nil)
                (cons (scaling--jabber) (scaling--names "peer" (/ n 2))))
  :body (progn
          (dotimes (_ 2)
            (dolist (peer (cdr it)) (jabber-mam--mark-dirty (car it) peer "chat")))))

;; Our nickname looked up once per archived result, over N rooms read unjoined.
(scaling-probe jabber-mam-room-nicks :n 4000
  :setup (let ((nicks (make-hash-table :test #'equal)) (rooms (scaling--names "room" n)))
           (scaling--jabber)
           (dolist (room rooms) (puthash room "romeo" nicks))
           (setq jabber-mam--room-nicks nicks)
           (cons (car jabber-connections) rooms))
  :body (dolist (room (cdr it)) (jabber-mam--our-muc-nick-p room "romeo" (car it))))

;; A room buffer refreshed over N stored messages (`jabber-backlog-number'
;; N): what runs before the first idle tick, the database's materialization
;; of the backlog among it.  The clearing and insertion ticks run later.
(scaling-probe jabber-chat-refresh :n 300
  :setup (let* ((jc (scaling--jabber))
                (room "room@conference.montague.net"))
           (setq jabber-db-path (make-temp-file "scaling-jabber" nil ".db")
                 jabber-db--connection nil
                 jabber-backlog-number n)
           (dotimes (i n)
             (jabber-db-store-message "romeo@montague.net" room "in" "groupchat"
                                      (format "message %d %s" i (make-string 60 ?x))
                                      (+ 1700000000 i) "nick" (format "s%d" i)))
           (let ((buffer (jabber-muc-create-buffer jc room)))
             ;; The creation's own backlog load: let it finish first.
             (with-current-buffer buffer
               (cl-incf jabber-chat--backlog-generation))
             buffer))
  :body (with-current-buffer it (jabber-chat-buffer-refresh)))

;; A worker's summary of N archives and N changed peers.
(scaling-probe jabber-sync-summary :n 2000
  :setup (let ((archives nil))
           (scaling--jabber)
           (setq jabber-mam-sync--stored (make-hash-table :test #'equal))
           (dolist (room (scaling--names "room" n))
             (push (list :room room :with nil :nick "romeo") archives)
             (puthash (list "romeo@montague.net" room "groupchat") (cons 3 3) jabber-mam-sync--stored))
           (cons (list :username "romeo" :server "montague.net")
                 (cons (list :room nil :with nil :nick nil) archives)))
  :body (jabber-mam-sync--summary (car it) (cdr it) (mapcar (lambda (a) (cons a 'success)) (cdr it)) t 1.0))

;; The stanza reader (jabber-xml.el, natively romacs--xml-read-element):
;; one stanza of N children, one of N attributes; and a connection's
;; buffer of N stanzas read and deleted one at a time, as jabber-filter
;; does, which stays linear because a read looks at a window of the
;; buffer, not all that follows.
(scaling-probe jabber-xml-read-stanza :n 2000
  :setup (progn (scaling--jabber)
                (concat "<message>"
                        (mapconcat (lambda (i) (format "<b x='%d'>t &amp; u</b>" i))
                                   (number-sequence 1 n) "")
                        "</message>"))
  :body (with-temp-buffer (insert it) (jabber-xml-parse-next-stanza)))

(scaling-probe jabber-xml-read-attributes :n 2000
  :setup (progn (scaling--jabber)
                (concat "<presence"
                        (mapconcat (lambda (i) (format " a%d='%d'" i i)) (number-sequence 1 n) "")
                        "/>"))
  :body (with-temp-buffer (insert it) (jabber-xml-parse-next-stanza)))

(scaling-probe jabber-xml-read-buffer :n 2000
  :setup (progn (scaling--jabber)
                (mapconcat (lambda (i) (format "<presence from='r@m/n%d'><x xmlns='u'/></presence>" i))
                           (number-sequence 1 n) ""))
  :body (with-temp-buffer
          (insert it)
          (while (jabber-xml-parse-next-stanza)
            (delete-region (point-min) (point)))))

;;;; Images (prim/leaf/graphics.rs: the semantic image cache)

;; N distinct images, each measured, then all N again from the cache: a
;; lookup is one hash of the spec's copied plist, so the work is linear
;; in the images and a lookup never scans the cache.
(defun scaling--image-specs (n)
  "N distinct SVG image specs, a size each."
  (let (specs)
    (dotimes (i n)
      (push (list 'image :type 'svg
                  :data (format "<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"%d\" height=\"8\"/>" (1+ i)))
            specs))
    (nreverse specs)))

(defun scaling--image-buffer (n)
  "A buffer of N lines, each showing its own image beside text; return it."
  (switch-to-buffer (get-buffer-create "scaling-images")) (erase-buffer)
  (let ((i 0))
    (dolist (spec (scaling--image-specs n))
      (insert (format "line %d " i))
      (insert-image spec)
      (insert " text\n")
      (setq i (1+ i))))
  (goto-char (point-min))
  (current-buffer))

;; N lines each with its own image, shown a window at a time: a step's
;; redisplay decodes and lays out the images it shows, so the work is in
;; the window's lines, not the buffer's images.
(scaling-probe images-in-buffer :gui t :n 200 :steps 10
  :setup (progn (clear-image-cache t) (scaling--image-buffer n))
  :body (with-current-buffer it (forward-line 15) (recenter)))

(scaling-probe images :gui t :n 100 :steps 1
  :setup (progn (clear-image-cache t) (scaling--image-specs n))
  ;; The message is the step's visible change, so the host paints it.
  :body (let (size) (dotimes (_ 2) (dolist (spec it) (setq size (image-size spec t)))) (message "%S" size)))

;;;; Attachments (rlisp/romacs-attach.el)

(defun scaling--attach-files (n)
  "N small distinct PNG files in a temporary directory: their names."
  (let ((dir (make-temp-file "scaling-attach" t)) (files nil)
        (png (base64-decode-string "iVBORw0KGgoAAAANSUhEUgAAAAIAAAACCAIAAAD91JpzAAAAEElEQVR4nGP4z8AARAwQCgAf7gP9i18U1AAAAABJRU5ErkJggg==")))
    (dotimes (i n)
      (let ((file (expand-file-name (format "a%d.png" i) dir)))
        (let ((coding-system-for-write 'binary)) (write-region png nil file nil 'silent))
        (push file files)))
    (nreverse files)))

(defun scaling--attach-host (files)
  "The attach test host with FILES attached: its buffer."
  (romacs-attach-test-host)
  (romacs-attach-set-items nil)
  (erase-buffer)
  (when files (romacs-attach-files files))
  (current-buffer))

;; One batch insertion of N items: planned and rendered once, linear.
(scaling-probe attach-tray :gui t :n 100 :steps 1
  :setup (progn (require 'romacs-attach) (cons (scaling--attach-host nil) (scaling--attach-files n)))
  :body (with-current-buffer (car it) (romacs-attach-set-items nil) (romacs-attach-files (cdr it))))

;; A key typed into the draft under N chips: the cached tray string is
;; put again as it is, so the key's cost does not grow with N.
(scaling-probe attach-tray-key :gui t :n 100 :steps 20 :budget 0.5
  :setup (progn (require 'romacs-attach) (scaling--attach-host (scaling--attach-files n)))
  :body (with-current-buffer it (goto-char (point-max)) (insert "x")))

;; A draft naming N files beside N orphans: one listing, linear.
(scaling-probe attach-drafts-reconcile :n 1000 :steps 1
  :setup (progn (require 'romacs-attach)
                (let* ((named (scaling--attach-files n))
                       (dir (file-name-directory (car named))))
                  (cons dir (mapcar (lambda (file) (list :path file)) named))))
  :body (let ((dir (car it)))
          (dotimes (i n) (write-region "o" nil (expand-file-name (format "orphan%d" i) dir) nil 'silent))
          (romacs-attach-reconcile dir (cdr it))))

;;;; Collection

(defvar scaling--keeper nil
  "The cons each `gc-free-lists' round keeps above its garbage.")

(defun scaling--churn (n)
  "Garbage in every table: N conses, N/2 vectors and N/100 strings, N/10
closures over a mutable variable (each a closure and a cell)."
  (make-list n nil)
  (dotimes (_ (/ n 2)) (make-vector 1 nil))
  (dotimes (_ (/ n 100)) (make-string 16 ?x))
  (dotimes (i (/ n 10)) (let ((x i)) (lambda () (setq x (1+ x))))))

;; A zygote's validity check before each dispatch (rlisp/vm/isolate.el,
;; worker-vms design §2.4): the inherited variables compared, then the
;; files it loaded and the `load-path' directories stamped.  Linear in
;; each, and in nothing else.
(scaling-probe isolate-zygote-check :n 200
  :setup (let* ((environment (scaling--names "SCALING_VARIABLE=value" (* 10 n)))
                (directories (scaling--names "/nonexistent/scaling-lisp" n))
                (files (scaling--names "/nonexistent/scaling-library.el" n))
                (inputs (let ((process-environment environment) (load-path directories))
                          (romacs--isolate-inputs)))
                (expanded (let ((load-path directories)) (romacs--isolate-load-directories))))
           (list environment directories
                 (record 'isolate-zygote nil nil 'ready nil inputs files (romacs--isolate-stamp files)
                         expanded (romacs--isolate-stamp expanded) 0)))
  :body (let ((process-environment (nth 0 it)) (load-path (nth 1 it)))
          (dotimes (_ 10)
            (or (romacs--isolate-zygote-current-p (nth 2 it)) (error "Zygote check failed")))))

;; The project commands' record of enumeration times (rlisp/romacs-project.el):
;; one entry per project root, each decision one lookup and one stat of the
;; root's Git index.
(scaling-probe romacs-project-enumerations :n 2000
  :setup (progn (require 'romacs-project) (scaling--names "/nonexistent/scaling-project" n))
  :body (progn
          (dolist (root it) (romacs-project--record root 0.01))
          (dolist (root it) (romacs-project--enumeration-estimate root))
          (dolist (root it) (remhash root romacs-project--enumerations))))

;; A child's environment (rlisp/vm/subprocess.el romacs--environment-block):
;; N distinct names in `process-environment', each first definition kept.
(scaling-probe process-environment-block :n 2000
  :setup (cl-loop for i below n collect (format "SCALING_ENV_%d=value" i))
  :body (let ((process-environment it))
          (romacs--environment-block "/")))

;; An init layer's pending definitions (world/pending.rs): N variables
;; made pending, then each read once, which evaluates its definition.
(scaling-probe world-pending :n 2000
  :setup (let ((symbols (mapcar #'intern (scaling--names "scaling-pending" n))))
           (dolist (symbol symbols) (set-default symbol 'stale))
           (setq romacs--world-pending
                 (vconcat (mapcar (lambda (symbol) (list 'defvar symbol (list 'defvar symbol 1) nil t)) symbols)))
           symbols)
  :body (let ((index 0))
          (dolist (symbol it)
            (romacs--world-pend symbol index)
            (setq index (1+ index)))
          (dolist (symbol it) (symbol-value symbol))))

;; An init layer build's refusal index (world::layer `Deferred'): N
;; libraries whose configuration is refused, half by feature and half by
;; file, then N declared entries each asking which of them its loads
;; (three features, three nested files) hold, as the builder asks.
(scaling-probe world-layer-refusals :n 2000
  :setup (let ((names (scaling--names "scaling-refused" n)) (i 0) deferred loaded)
           (dolist (name names)
             (push (list name (and (cl-oddp i) (format "/scaling/lib/%s.el" name))) deferred)
             (push (list (list (format "%s-a" name) (format "%s-b" name) (if (cl-evenp i) name "none"))
                         (list (format "/scaling/x/%s-1.el" name) (format "/scaling/x/y/%s-2.el" name)
                               (format "/scaling/lib/%s.el" name)))
                   loaded)
             (setq i (1+ i)))
           (list deferred loaded))
  :body (romacs--world-layer-deferred-by (car it) (cadr it)))

(scaling-probe gc-free-lists :n 200000
  ;; Rounds of: garbage in every table, a cons, closure, cell and string
  ;; kept above it, a collection, then as much garbage again.  The sweep leaves free
  ;; entries in proportion to N below the kept ones and the allocation
  ;; takes them all back, so the work is linear in N; an allocation that
  ;; scanned a free list, a sweep that listed an entry twice or a table
  ;; never cut back would make it grow faster.
  :setup nil
  :body (dotimes (_ 4)
          (scaling--churn n)
          (setq scaling--keeper (let ((y 0)) (list n (lambda () (setq y (1+ y))) (make-string 16 ?y))))
          (garbage-collect)
          (scaling--churn n)))

;; Code retirement (request_code.rs, Program::retire_code): N functions
;; compiled, lowered by a call and dropped in each step; the host's idle
;; pass between steps collects at the top level, the only place request
;; code retires, and the next step takes their sites and cache slots back.
;; A session no key has reached has no `romacs--idle-collect' (the lazy
;; rlisp/vm/keyboard.el) and shows a startup message, so the probe supplies
;; the one and clears the other.  A step with no collection after the last ends the session,
;; which fails the measure.  Retirement that scanned its free lists, or
;; rebuilt tables per function, would grow faster than N.
(defvar scaling--retire-collections 0
  "`gcs-done' when the last `gc-retired-code' step ended.")

(scaling-probe gc-retired-code :gui t :n 1000 :steps 4 :gap 1.0
  :setup (progn
           (defalias 'romacs--idle-collect (lambda (&optional _in-lisp) t))
           (setq scaling--retire-collections gcs-done)
           (switch-to-buffer (get-buffer-create "*scaling-retire*")))
  :body (progn
          (when (and (> i 0) (<= gcs-done scaling--retire-collections))
            (kill-emacs 3))
          (dotimes (k n)
            (funcall (eval (list 'lambda nil (list 'list (list '+ k i) k)) t)))
          (with-current-buffer (window-buffer (selected-window))
            (goto-char (point-max))
            (insert (format "step %d\n" i)))
          (message nil)
          (setq scaling--retire-collections gcs-done)))

;; The collections a usage log reads (gc.rs `Vm::note_collection'): N
;; collections, past what the log keeps at 4N, then its read.  A record
;; costs the same however many were kept, and past the limit only a count.
(scaling-probe gc-collection-log :n 100
  :setup (romacs--take-collections)
  :body (progn (dotimes (_ n) (garbage-collect))
               (romacs--take-collections)))

;; The debugger's frame model: `backtrace-get-frames' (every frame with
;; its locals, as a *Backtrace* buffer is built) over a stack N deep with a
;; dynamic binding per frame.  One walk (`backtrace--frames-and-locals');
;; GNU's per-frame `backtrace--locals' would be quadratic.
(defvar scaling--bt-depth nil)
(defun scaling--bt-frames (n)
  (let ((scaling--bt-depth n))
    (if (> n 0) (scaling--bt-frames (1- n)) (length (backtrace-get-frames)))))
;; A snapshot of the stack (`romacs--inspect-frames') walks within a work
;; budget: the time is flat in the size of an argument, and in the depth of
;; the stack past the frames it keeps.
(defun scaling--inspect-value (_value) (romacs--inspect-frames))
(defun scaling--inspect-deep (n)
  (if (> n 0) (list (scaling--inspect-deep (1- n))) (romacs--inspect-frames)))
(scaling-probe inspect-value :n 20000 :k 8
  :setup (make-list n 'x)
  :body (dotimes (_ 200) (scaling--inspect-value it)))
(scaling-probe inspect-string :n 100000 :k 8
  :setup (make-string n ?x)
  :body (dotimes (_ 200) (scaling--inspect-value it)))
(scaling-probe inspect-stack :n 1000 :k 4
  :setup (setq max-lisp-eval-depth (max max-lisp-eval-depth (* 20 n)))
  :body (dotimes (_ 100) (scaling--inspect-deep n)))

(scaling-probe backtrace-frames :n 200 :k 4
  :setup (progn (require 'backtrace) (require 'romacs-backtrace)
                (setq max-lisp-eval-depth (max max-lisp-eval-depth (* 20 n))))
  :body (dotimes (_ 4) (scaling--bt-frames n)))

;; `save-excursion' records nested N deep, then `backtrace-eval' at the
;; outermost frame, whose unrewind swaps all N excursions and bindings and
;; back: linear in N.
(defun scaling--bt-excursions (n top)
  (let ((scaling--bt-depth n))
    (save-excursion
      (if (> n 0) (scaling--bt-excursions (1- n) top)
        (backtrace-eval '(list scaling--bt-depth (point)) top 'scaling--bt-excursions)))))
(scaling-probe backtrace-excursions :n 200 :k 4
  :setup (setq max-lisp-eval-depth (max max-lisp-eval-depth (* 20 n)))
  :body (dotimes (_ 4) (scaling--bt-excursions n n)))

;; `backtrace-debug' flags: every frame of a stack N deep flags itself and
;; clears the flag before it returns, then N calls each leave theirs set
;; (whose tokens go stale once they return): O(log N) amortized a flag.
(defun scaling--bt-flag-deep (n)
  (backtrace-debug 0 t 'scaling--bt-flag-deep)
  (prog1 (if (> n 0) (scaling--bt-flag-deep (1- n)) 0)
    (backtrace-debug 0 nil 'scaling--bt-flag-deep)))
(defun scaling--bt-flag-leave () (backtrace-debug 0 t 'scaling--bt-flag-leave) nil)
(defun scaling--bt-flag-any (n)
  ;; Flags the N frames below the leaf newest to oldest, then clears them
  ;; oldest first: insertion and removal away from the end of the set.
  (if (> n 0) (scaling--bt-flag-any (1- n))
    (let ((depth (length (backtrace-frames 'scaling--bt-flag-any))))
      (dotimes (i depth) (backtrace-debug i t 'scaling--bt-flag-any))
      (dotimes (i depth) (backtrace-debug (- depth 1 i) nil 'scaling--bt-flag-any)))))
(scaling-probe backtrace-debug-flags :n 400 :k 4
  :setup (setq max-lisp-eval-depth (max max-lisp-eval-depth (* 20 n)))
  :body (let ((debugger (lambda (&rest args) (cadr args))))
          (scaling--bt-flag-deep n)
          (scaling--bt-flag-any n)
          (dotimes (_ n) (scaling--bt-flag-leave))))

;; `backtrace-eval' over N buffer-local bindings whose locals are gone (each
;; becomes inert in place) below N live bindings: one unrewind, linear.
(defun scaling--bt-dead-inner () (backtrace-eval nil 1 'scaling--bt-dead-inner))
(defun scaling--bt-dead-outer (locals globals)
  (cl-progv locals (make-list (length locals) 1)
    (kill-all-local-variables)
    (cl-progv globals (make-list (length globals) 2)
      (scaling--bt-dead-inner))))
(scaling-probe backtrace-eval-dead-locals :n 500 :k 4
  :setup (let ((locals (mapcar #'intern (scaling--names "scaling--bt-local" n)))
               (globals (mapcar #'intern (scaling--names "scaling--bt-global" n))))
           (dolist (g globals) (set g 0))
           (list locals globals))
  :body (with-temp-buffer
          (dotimes (_ 4)
            (dolist (l (car it)) (set (make-local-variable l) 0))
            (scaling--bt-dead-outer (car it) (cadr it)))))

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
of timers, each after the host's redisplay of the step before: a paint still
being drawn holds the next one back, so no fixed time means the step was
laid out.  Count the instructions from before the first step to after the
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
          (let ((step nil) (laid-out nil) (i 0))
            (setq laid-out (lambda ()
                             (if (romacs--redisplay-since-mark) (funcall step) (run-with-timer gap nil laid-out))))
            (setq step (lambda ()
                         (if (< i steps)
                             (progn (scaling--mark (list :scaling "step" :i i))
                                    (funcall body n it i) (setq i (1+ i))
                                    (romacs--redisplay-since-mark t)
                                    (run-with-timer gap nil laid-out))
                           (let ((end (romacs-instructions)))
                             (scaling--mark (list :scaling "end"))
                             (with-temp-file out
                               (insert (json-encode (list :name (symbol-name name) :n n
                                                          :instructions (and start end (- end start))))))
                             (kill-emacs 0)))))
            (funcall step))))))))

(provide 'scaling-probes)
;;; probes.el ends here
