;;; romacs-eval-chip.el --- Evaluation results as a chip after the sexp  -*- lexical-binding: t -*-
;; SPDX-License-Identifier: GPL-3.0-or-later
;; Maintainer: Thanos Apollo <public@thanosapollo.org>

;;; Commentary:
;; `romacs-eval-chip-mode' shows the value an evaluation command prints in
;; the echo area as a small glass layer right after the evaluated sexp,
;; in the manner of CIDER's inline results, and not also as a toast.  The
;; chip is a layer (romacs-layer.el), so the buffer's text never shifts.
;; What GNU prints, `current-message' and *Messages* are unchanged: the
;; chip takes the printed text at the command's return, and its echo
;; display is claimed once (`romacs-layer-echo-claim-functions').  An error
;; shows as a tinted chip.  The next command that moves point, edits,
;; scrolls or selects another window hides it, else a timer.
;; DIVERGENCES.md VM2-ECHO-LAYER.

;;; Code:

(eval-when-compile (require 'cl-lib))
(require 'subr-x)
(require 'romacs-layer)

(defgroup romacs-eval-chip nil
  "Evaluation results shown after the evaluated sexp."
  :group 'romacs-layer)

(defcustom romacs-eval-chip-timeout 2
  "Seconds an eval chip shows unless a command hides it first."
  :type 'number)

(defcustom romacs-eval-chip-max-width 60
  "Most columns an eval chip's text takes; a longer value is cut with …."
  :type 'natnum)

(defface romacs-eval-chip '((t :inherit romacs-layer-glass))
  "An eval chip's text and background.")

(defface romacs-eval-chip-error '((t :inherit (error romacs-eval-chip)))
  "An eval chip showing an error.")

(defconst romacs-eval-chip-commands
  '((eval-last-sexp . romacs-eval-chip--last-sexp-p)
    (eval-defun . romacs-eval-chip--defun-p)
    (eval-expression . romacs-eval-chip--expression-p)
    (pp-eval-last-sexp . romacs-eval-chip--pp-p))
  "Each evaluation command and its rule: a function of the command's
arguments, non-nil when the command printed its value to the echo area
last (gnu-lisp/progmodes/elisp-mode.el, gnu-lisp/emacs-lisp/pp.el).")

(defvar romacs-eval-chip--pp-echo nil
  "How `pp-display-expression' showed its value: `echo', `buffer' or nil.")

(defvar romacs-eval-chip--running nil
  "The `recursion-depth' of the evaluation command whose chip is pending:
a command it calls again (`eval-defun' with Edebug) shows none of its own.")

(defvar edebug-all-defs)

(defun romacs-eval-chip--last-sexp-p (arg)
  "Whether `eval-last-sexp' with ARG prints its value to the echo area:
GNU's own reading of the argument (`eval-expression-get-print-arguments'),
where nil and `-' print and others insert."
  (not (car (eval-expression-get-print-arguments arg))))

(defun romacs-eval-chip--defun-p (edebug-it)
  "Whether `eval-defun' with EDEBUG-IT evaluates without instrumenting:
`edebug-all-defs' inverts the argument (elisp-mode.el, edebug.el
`edebug--eval-defun'); an instrumented defun prints \"Edebug: NAME\"."
  (eq (not edebug-it) (not (bound-and-true-p edebug-all-defs))))

(defun romacs-eval-chip--expression-p (_exp &optional insert-value &rest _)
  "Whether `eval-expression' with INSERT-VALUE, already read by
`eval-expression-get-print-arguments', prints to the echo area."
  (null insert-value))

(defun romacs-eval-chip--pp-p (arg)
  "Whether `pp-eval-last-sexp' with ARG showed its value as a message:
ARG nil (else it inserts), and `pp-display-expression' took its echo
branch, not its buffer's."
  (and (null arg) (eq romacs-eval-chip--pp-echo 'echo)))

(defvar romacs-eval-chip--claim nil
  "The echo text a chip claims once: (TEXT DEPTH KIND ANCHOR).  KIND
`value': the chip shows already; `error': the command loop's report of
an evaluation's error, ending with TEXT, shows the chip after ANCHOR.")

(defvar romacs-eval-chip--shown nil
  "The shown chip's watch: (BUFFER POINT TICK WINDOW START TIMER).")

(defun romacs-eval-chip--buffer ()
  "The chip's buffer."
  (or (get-buffer " *romacs-eval-chip*")
      (with-current-buffer (get-buffer-create " *romacs-eval-chip*")
        (setq-local mode-line-format nil header-line-format nil cursor-type nil truncate-lines t)
        (current-buffer))))

(defun romacs-eval-chip--text (text window)
  "TEXT's first line, cut to the chip's width in WINDOW."
  (let* ((line (car (split-string text "\n")))
         (room (max 20 (min romacs-eval-chip-max-width (- (window-body-width window) 4)))))
    (concat "⇒ " (if (or (> (string-width line) room) (string-match-p "\n" text))
                     (concat (truncate-string-to-width line (1- room)) "…")
                   line))))

(defun romacs-eval-chip--columns (chip window)
  "Columns of WINDOW's frame that hold CHIP, measured in pixels where the
display can (a glyph such as ⇒ may be wider than a column), and one more
for the column a window's last keeps for its continuation glyph."
  (let* ((cw (frame-char-width (window-frame window)))
         (pixels (or (ignore-errors (string-pixel-width chip)) (* cw (string-width chip)))))
    (1+ (ceiling pixels (float cw)))))

(defun romacs-eval-chip--anchor (command)
  "Where COMMAND's chip goes: (WINDOW . MARKER), at the end of the defun
for `eval-defun', else at point; the marker follows text the evaluation
inserts before it."
  (cons (selected-window)
        (copy-marker (if (eq command 'eval-defun)
                         (or (ignore-errors (save-excursion (end-of-defun) (skip-chars-backward " \t\n") (point))) (point))
                       (point)))))

(defun romacs-eval-chip--release (anchor)
  "Let ANCHOR's marker point nowhere."
  (when (markerp (cdr anchor)) (set-marker (cdr anchor) nil)))

(defun romacs-eval-chip-hide ()
  "Hide the eval chip, which fades."
  (when romacs-eval-chip--shown
    (cancel-timer (nth 5 romacs-eval-chip--shown))
    (setq romacs-eval-chip--shown nil)
    (remove-hook 'post-command-hook #'romacs-eval-chip--watch)
    (romacs-layer-hide (romacs-eval-chip--buffer))))

(defun romacs-eval-chip--printable (text)
  "TEXT as the echo area hands it to layers (echo.rs `romacs--echo-printable')."
  (if (fboundp 'romacs--echo-printable) (romacs--echo-printable text) text))

(defun romacs-eval-chip--show (text anchor error)
  "Show TEXT in a chip after ANCHOR, (WINDOW . MARKER), tinted when
ERROR.  Non-nil when shown; nothing when the position is not shown."
  (romacs-eval-chip-hide)
  (let ((window (car anchor)) (buffer (romacs-eval-chip--buffer)))
    (when (and (window-live-p window) (stringp text) (not (equal text ""))
               (marker-buffer (cdr anchor)) (eq (window-buffer window) (marker-buffer (cdr anchor))))
      (let ((chip (romacs-eval-chip--text text window))
            (face (if error 'romacs-eval-chip-error 'romacs-eval-chip)))
        (with-current-buffer buffer
          (let ((inhibit-read-only t)) (erase-buffer) (insert (propertize chip 'face face)) (goto-char (point-min))))
        (modify-frame-parameters (romacs-layer--frame buffer)
                                 (romacs-layer-glass-parameters romacs-layer-glass face 4 6))
        (set-window-hscroll (frame-root-window (romacs-layer--frame buffer)) 0)
        (when (romacs-layer-show buffer :position (list 'at window (marker-position (cdr anchor)))
                                 :columns (romacs-eval-chip--columns chip window) :lines 1)
          (setq romacs-eval-chip--shown
                (list (window-buffer window) (window-point window)
                      (buffer-modified-tick (window-buffer window)) window (window-start window)
                      (run-at-time romacs-eval-chip-timeout nil #'romacs-eval-chip-hide)))
          (add-hook 'post-command-hook #'romacs-eval-chip--watch)
          t)))))

(defun romacs-eval-chip--watch ()
  "Hide the chip once a command moved point, edited, scrolled or chose
another window."
  (pcase romacs-eval-chip--shown
    (`(,buffer ,point ,tick ,window ,start ,_)
     (unless (and (window-live-p window) (eq (selected-window) window)
                  (eq (window-buffer window) buffer) (= (window-point window) point)
                  (= (buffer-modified-tick buffer) tick) (= (window-start window) start))
       (romacs-eval-chip-hide)))))

(defun romacs-eval-chip--claim-echo (text)
  "Claim TEXT, the echo area's new message, when it is a chip's: the
shown value's, or the report of a pending error, which shows its chip."
  (pcase romacs-eval-chip--claim
    (`(,want ,depth ,kind ,anchor)
     (when (and (= depth (recursion-depth))
                (if (eq kind 'error) (string-suffix-p want text) (equal want text)))
       (romacs-eval-chip--disarm t)
       (or (eq kind 'value) (romacs-eval-chip--try want anchor t))))))

(defun romacs-eval-chip--disarm (&optional always)
  "Forget the chip's claim, ALWAYS or as a command starts at its depth or
out, and release a pending error's anchor."
  (pcase romacs-eval-chip--claim
    (`(,_ ,depth ,_ ,anchor)
     (when (or always (<= (recursion-depth) depth))
       (setq romacs-eval-chip--claim nil)
       ;; A claimed error shows its chip first, from the anchor's position.
       (unless always (romacs-eval-chip--release anchor))))))

(defun romacs-eval-chip--try (text anchor error)
  "Show TEXT's chip after ANCHOR, tinted when ERROR; non-nil when shown.
Where it cannot show, TEXT stays the echo area's, a toast."
  (prog1 (condition-case nil (romacs-eval-chip--show text anchor error) (error (romacs-eval-chip-hide) nil))
    (when error (romacs-eval-chip--release anchor))))

(defun romacs-eval-chip--run (command orig args)
  "Run ORIG, the evaluation COMMAND, with ARGS; show the value it printed
in a chip, and arm one for its error.  The error is not caught: it
reaches the debugger and the caller as without the chip (a `handler-bind'
handler only notes it), and shows as a chip only if the command loop
reports it."
  (if (eql romacs-eval-chip--running (recursion-depth))
      (apply orig args)
    (let ((anchor (romacs-eval-chip--anchor command))
          (romacs-eval-chip--running (recursion-depth))
          (romacs-eval-chip--pp-echo nil)
          error)
      (unwind-protect
          (prog1 (handler-bind ((error (lambda (err) (setq error (error-message-string err)))))
                   (apply orig args))
            (setq error nil)
            (when (apply (cdr (assq command romacs-eval-chip-commands)) args)
              (let ((text (current-message)))
                (when (romacs-eval-chip--try text anchor nil)
                  (setq romacs-eval-chip--claim
                        (list (romacs-eval-chip--printable text) (recursion-depth) 'value nil))))))
        (if error
            (setq romacs-eval-chip--claim
                  (list (romacs-eval-chip--printable error) (recursion-depth) 'error anchor))
          ;; The chip is placed once; its watch is point's, not the marker's.
          (romacs-eval-chip--release anchor))))))

(defun romacs-eval-chip--pp-around (orig &rest args)
  "Run ORIG, `pp-display-expression', with ARGS, noting how this call
showed its value.  Its echo branch messages its output buffer's one line
(pp.el); its buffer branch shows the buffer, and any message it leaves
\(a `temp-buffer-show-hook''s) is not that line.  The last call decides."
  (let ((before (current-message)))
    (prog1 (apply orig args)
      (let ((out (get-buffer (nth 1 args))) (message (current-message)))
        (setq romacs-eval-chip--pp-echo
              (and out message (not (eq message before))
                   (equal (substring-no-properties message)
                          (with-current-buffer out (string-trim-right (buffer-substring-no-properties (point-min) (point-max)))))
                   'echo))))))

;;;###autoload
(define-minor-mode romacs-eval-chip-mode
  "Show evaluation results in a chip after the evaluated sexp.
`eval-last-sexp', `eval-defun', `pp-eval-last-sexp' and
`eval-expression' then show the value they print in the echo area, or
their error, in a small layer after the sexp, not as a toast; the value
is still the echo area's message and logged in *Messages*."
  :global t :group 'romacs-eval-chip
  (dolist (entry romacs-eval-chip-commands)
    (let ((command (car entry)))
      (if romacs-eval-chip-mode
          (advice-add command :around (lambda (orig &rest args) (romacs-eval-chip--run command orig args))
                      '((name . romacs-eval-chip)))
        (advice-remove command 'romacs-eval-chip))))
  (if romacs-eval-chip-mode
      (progn (advice-add 'pp-display-expression :around #'romacs-eval-chip--pp-around)
             (add-hook 'romacs-layer-echo-claim-functions #'romacs-eval-chip--claim-echo)
             (add-hook 'pre-command-hook #'romacs-eval-chip--disarm))
    (advice-remove 'pp-display-expression #'romacs-eval-chip--pp-around)
    (remove-hook 'romacs-layer-echo-claim-functions #'romacs-eval-chip--claim-echo)
    (remove-hook 'pre-command-hook #'romacs-eval-chip--disarm)
    (romacs-eval-chip-hide)))

(provide 'romacs-eval-chip)
;;; romacs-eval-chip.el ends here
