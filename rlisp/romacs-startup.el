;;; romacs-startup.el --- VM2 launchable Rum workspace -*- lexical-binding: t -*-

;; SPDX-License-Identifier: GPL-3.0-or-later

;;; Commentary:

;; Palette port from Omarchy Rum emacs.el and colors.toml; no Modus runtime dependency.
;; Host helpers keep VM2's own `window-child': GNU's window.el (loaded by
;; loadup.el) replaces it with Lisp over C primitives VM2 lacks
;; (`window-top-child'), and the GUI's selection and window commands must
;; not depend on how far window.el got.

;;; Code:

(fset 'romacs-gui--window-child (symbol-function 'window-child))
(defface default '((t (:foreground "#d4c5a0" :background "#000000" :family "Iosevka"))) "Rum parchment on black.")
(defface cursor '((t (:background "#9099d9"))) "Rum indigo cursor.")
(defface mode-line '((t (:foreground "#d4c5a0" :background "#50269a"))) "Rum active mode line.")
(defface mode-line-inactive '((t (:foreground "#756b59" :background "#141414"))) "Rum inactive mode line.")
(defface region '((t (:foreground "#d4c5a0" :background "#3e4451"))) "Rum selection.")
(defface fringe '((t (:foreground "#000000" :background "#000000"))) "Unobtrusive fringe.")
(defface highlight '((t (:foreground "#d4c5a0" :background "#7030af"))) "Rum highlight.")
(defface minibuffer-prompt '((t (:foreground "#d2b580"))) "Rum prompt.")
(defface font-lock-keyword-face '((t (:foreground "#b395db"))) "Rum keyword.")
(defface font-lock-string-face '((t (:foreground "#aacb75"))) "Rum string.")
(defface font-lock-comment-face '((t (:foreground "#756b59"))) "Rum comment.")
(defface font-lock-function-name-face '((t (:foreground "#c28fea"))) "Rum function name.")
(defface font-lock-variable-name-face '((t (:foreground "#00a6a6"))) "Rum variable.")
(defface font-lock-constant-face '((t (:foreground "#72b6e4"))) "Rum constant.")
(defface font-lock-type-face '((t (:foreground "#00b8ad"))) "Rum type.")
(defface font-lock-builtin-face '((t (:foreground "#d0a3e8"))) "Rum builtin.")
(defface font-lock-warning-face '((t (:foreground "#e89e30"))) "Rum font lock warning.")
(defface link '((t (:foreground "#4cc9f0"))) "Rum link.")
(defface error '((t (:foreground "#c97054"))) "Rum error.")
(defface warning '((t (:foreground "#e89e30"))) "Rum warning.")
(defface success '((t (:foreground "#00b8ad"))) "Rum success.")
(defface romacs-comment '((t (:foreground "#756b59"))) "Rum dim annotation.")
(defface romacs-code '((t (:foreground "#aacb75"))) "Rum olive source.")
(defface romacs-result '((t (:foreground "#72b6e4"))) "Rum evaluation result.")
(setq romacs-gui-quit-requested nil)
;; Pending confirmation consumes future GUI keys, not read-string's macro input.
(defvar romacs-gui-pending nil)
(defvar romacs-gui-visits nil)
(defvar romacs-gui-preserve-mode-line nil)
(defvar romacs-term--closing nil)
(defun romacs-gui-running-terminal-p ()
  (and (fboundp 'romacs-term-running-p) (romacs-term-running-p)))
(defun romacs-gui-dirty-p ()
  (and buffer-file-name (buffer-modified-p)))
(defun romacs-gui-echo (text)
  (let ((old (current-buffer)) (old-point (point)) (inhibit-read-only t))
    (unwind-protect
        (progn (set-buffer (window-buffer (minibuffer-window)))
               (erase-buffer) (insert text))
      (set-buffer old) (goto-char old-point))))
(defun romacs-gui-confirm (action)
  (when romacs-gui-pending (romacs-gui-answer-no))
  (setq romacs-gui-pending (list action (current-buffer) (if (fboundp 'current-local-map) (current-local-map) nil)))
  (let ((map (make-sparse-keymap)))
    (define-key map [121] 'romacs-gui-answer-yes)
    (define-key map [110] 'romacs-gui-answer-no)
    (define-key map [7] 'romacs-gui-answer-no)
    (use-local-map map))
  (romacs-gui-echo
   (if (eq action 'save)
       "File changed on disk; overwrite it? (y/n; C-g cancels)"
     (if (and (fboundp 'romacs-term-any-running-p) (romacs-term-any-running-p))
         "Running terminals exist; close processes and discard changes? (y/n; C-g cancels)"
       "Modified file buffers exist; discard unsaved changes? (y/n; C-g cancels)"))))
(defun romacs-gui-answer-no ()
  (interactive)
  (when romacs-gui-pending
    (let ((pending romacs-gui-pending))
      (setq romacs-gui-pending nil)
      (when (buffer-live-p (car (cdr pending)))
        (with-current-buffer (car (cdr pending))
          (use-local-map (car (cdr (cdr pending))))))))
  (romacs-gui-echo "Cancelled"))
(defun romacs-gui-answer-yes ()
  (interactive)
  (let ((pending romacs-gui-pending))
    (romacs-gui-answer-no)
    (when pending
      (if (eq (car pending) 'save)
          (with-current-buffer (car (cdr pending))
            (romacs-gui-write-visited-buffer))
        (if (eq (car pending) 'quit)
          (setq romacs-gui-quit-requested t)
        (let ((romacs-term--closing t))
          (romacs-gui-kill-confirmed (car (cdr pending)))))))))
(defun romacs-gui-live-visits ()
  "Remove dead buffers from the GUI visit registry."
  (let ((buffers romacs-gui-visits) (live nil))
    (while buffers
      (when (buffer-live-p (car buffers))
        (setq live (append live (list (car buffers)))))
      (setq buffers (cdr buffers)))
    (setq romacs-gui-visits live)))
(defun romacs-gui-kill-confirmed (buffer)
  "Kill captured BUFFER if live, honoring native query vetoes."
  (unwind-protect
      (when (buffer-live-p buffer)
        (unless (kill-buffer buffer)
          (romacs-gui-echo "Buffer retained: kill vetoed"))
        (unless (buffer-live-p buffer)
          (romacs-gui-echo "Buffer killed")))
    (romacs-gui-live-visits)))
(defun romacs-gui-quit ()
  (interactive)
  (let ((buffers (romacs-gui-live-visits)) (dirty nil))
    (while buffers
      (with-current-buffer (car buffers)
        (when (and buffer-file-name (romacs-gui-dirty-p)) (setq dirty t)))
      (setq buffers (cdr buffers)))
    (if (or dirty (and (fboundp 'romacs-term-any-running-p) (romacs-term-any-running-p)))
        (romacs-gui-confirm 'quit)
      (setq romacs-gui-quit-requested t))))
(defun romacs-gui-kill-buffer ()
  (interactive)
  (if (or (and buffer-file-name (romacs-gui-dirty-p)) (romacs-gui-running-terminal-p))
      (romacs-gui-confirm 'kill)
    (romacs-gui-kill-confirmed (current-buffer))))
(defun romacs-gui-root ()
  (let ((w (selected-window)))
    (while (window-parent w) (setq w (window-parent w)))
    w))
(defun romacs-gui-leaves (w)
  (if (window-live-p w) (list w)
    (let ((child (romacs-gui--window-child w)) (leaves nil))
      (while child
        (setq leaves (append leaves (romacs-gui-leaves child)))
        (setq child (window-next-sibling child)))
      leaves)))
(defun romacs-gui-other-window ()
  (interactive)
  (let* ((leaves (romacs-gui-leaves (romacs-gui-root)))
         (tail (cdr (memq (selected-window) leaves))))
    (select-window (if tail (car tail) (car leaves)))))
(defun romacs-gui-split (horizontal)
  (let* ((w (selected-window))
         (size (if horizontal (window-pixel-width w) (window-pixel-height w)))
         (half (/ size 2)))
    (set-window-new-pixel w (- size half))
    (split-window-internal w half horizontal 0.5)))
(defun romacs-gui-split-below () (interactive) (romacs-gui-split nil))
(defun romacs-gui-split-right () (interactive) (romacs-gui-split t))
;; Prepare every node along the deletion axis, redistributing the removed
;; pixels into the first surviving sibling. Internal nodes need preparation too.
(defun romacs-gui-prepare-delete (w size horizontal omit)
  (set-window-new-pixel w size)
  (let ((child (romacs-gui--window-child w)))
    (when child
      (let ((same (eq horizontal (< (window-pixel-width child) (window-pixel-width w))))
            (sum 0) (children nil))
        (while child
          (unless (eq child omit)
            (setq children (append children (list child)))
            (setq sum (+ sum (if horizontal (window-pixel-width child) (window-pixel-height child)))))
          (setq child (window-next-sibling child)))
        (let ((delta (- size sum)))
          (while children
            (setq child (car children))
            (romacs-gui-prepare-delete child
              (if same (+ (if horizontal (window-pixel-width child) (window-pixel-height child)) delta) size)
              horizontal omit)
            (setq delta 0 children (cdr children))))))))
(defun romacs-gui-delete (w)
  (let ((parent (window-parent w)))
    ;; `signal', not `error': after loadup `error' is GNU's subr.el, which
    ;; needs the C `format-message' VM2 lacks.
    (unless parent (signal 'error (list "Cannot delete the only window")))
    (let* ((child (romacs-gui--window-child parent))
           (horizontal (< (window-pixel-width child) (window-pixel-width parent)))
           (root (romacs-gui-root)))
      (romacs-gui-prepare-delete root
        (if horizontal (window-pixel-width root) (window-pixel-height root)) horizontal w)
      (delete-window-internal w))))
(defun romacs-gui-delete-window () (interactive) (romacs-gui-delete (selected-window)))
(defun romacs-gui-delete-other-windows ()
  (interactive)
  (let ((keep (selected-window)) (leaves (romacs-gui-leaves (romacs-gui-root))))
    (while leaves
      (unless (eq (car leaves) keep) (romacs-gui-delete (car leaves)))
      (setq leaves (cdr leaves)))
    (select-window keep)))
;; scroll-conservatively is a C variable (xdisp.c); Romacs prefers scrolling
;; only enough to reveal point.
(setq scroll-conservatively 101)
(defvar last-command nil)
(defvar romacs-gui-goal-column nil)
(defun romacs-gui-vertical-motion (lines)
  (let ((origin (point)))
    (beginning-of-line)
    (unless (memq last-command '(romacs-gui-next-line romacs-gui-previous-line))
      (setq romacs-gui-goal-column (- origin (point))))
    (if (< lines 0)
        (when (= (point) (point-min))
          (goto-char origin)
          (signal 'beginning-of-buffer nil))
      (let ((start (point)))
        (end-of-line)
        (when (= (point) (point-max))
          (goto-char origin)
          (signal 'end-of-buffer nil))
        (goto-char start)))
    (unless (= (forward-line lines) 0)
      (signal (if (< lines 0) 'beginning-of-buffer 'end-of-buffer) nil))
    (let ((start (point)))
      (end-of-line)
      (goto-char (min (+ start romacs-gui-goal-column) (point))))))
(defun romacs-gui-next-line () (interactive) (romacs-gui-vertical-motion 1))
(defun romacs-gui-previous-line () (interactive) (romacs-gui-vertical-motion -1))
(defun romacs-gui-line (text face)
  (let ((start (point)))
    (insert text "\n")
    (put-text-property start (point) 'face face)))
(defvar romacs-gui-visit-errors nil)
(defun romacs-gui-visit-file (file name)
  "Report failed native lookup/adoption without aborting the session."
  (condition-case err
      (romacs-gui--visit-file file name)
    (error
     (romacs-gui--visit-error file err)
     (set-window-buffer (selected-window) (current-buffer))
     (current-buffer))))
(defun romacs-gui--visit-error (file err)
  "Record ERR visiting FILE in *romacs visit errors*, made current."
  (setq romacs-gui-visit-errors (cons (cons file err) romacs-gui-visit-errors))
  (set-buffer (get-buffer-create "*romacs visit errors*"))
  (goto-char (point-max))
  (insert (format "romacs could not visit %s:\n  %S\n" file err))
  (current-buffer))
(defvar romacs-gui-displayable-buffers nil
  "The buffers --loadup's file arguments visited, the last one first:
startup.el `command-line-1's `displayable-buffers', which
`romacs-gui-loadup-command-line-1' displays.")
(defun romacs-gui-visit-file-noselect (file name)
  "Visit FILE, named NAME, as startup.el's `process-file-arg' does: the
buffer goes onto `romacs-gui-displayable-buffers' and becomes current,
and no window shows it yet.  A file the visitor refuses gives the
*romacs visit errors* buffer in its place."
  (let ((buffer (condition-case err
                    (romacs-find-file-noselect file name)
                  (error (romacs-gui--visit-error file err)))))
    (setq romacs-gui-displayable-buffers (cons buffer romacs-gui-displayable-buffers))
    (set-buffer buffer)))
(defun romacs-gui--visit-file (file name)
  "Select the shared file visitor's result, preserving native visit identity."
  (let ((buffer (romacs-find-file-noselect file name)))
    (romacs-select-buffer buffer)))
(defun romacs-gui-write-visited-buffer ()
  "Write after supersession verification or explicit overwrite consent."
  (write-region nil nil buffer-file-name nil t)
  ;; As GNU's basic-save-buffer-1 and -2: keep the coding the save used.
  (if buffer-file-coding-system-explicit
      (setcar buffer-file-coding-system-explicit last-coding-system-used))
  (setq buffer-file-coding-system last-coding-system-used)
  (romacs-gui-echo "Saved"))
(defun romacs-gui-save-buffer ()
  "Save modified visited text; ask before overwriting a superseded file."
  (interactive)
  (unless buffer-file-name
    (signal 'error (list "romacs-gui-save-buffer: this buffer is not visiting a file")))
  (when (or (romacs-gui-dirty-p) (not (file-attributes buffer-file-name 'integer)))
    (if (verify-visited-file-modtime (current-buffer))
        (romacs-gui-write-visited-buffer)
      (romacs-gui-confirm 'save))))
