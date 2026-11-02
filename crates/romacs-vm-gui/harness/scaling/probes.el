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

;; A theme application over N more package faces.  Its cost is the themes
;; lane's: ~7 G instructions for modus-operandi in a -Q window, and ~16 M
;; more per defined face, linear but each far above GNU's.
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

(scaling-probe undo :n 2000
  :setup (with-current-buffer (get-buffer-create " scaling-undo")
           (buffer-enable-undo)
           (current-buffer))
  :body (with-current-buffer it
          (dotimes (_ n) (insert "x") (undo-boundary))
          (undo-start)
          (undo-more n)))

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
