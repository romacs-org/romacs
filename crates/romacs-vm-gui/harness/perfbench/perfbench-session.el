;;; perfbench-session.el --- perfbench.py's session rows, the same Lisp in every engine  -*- lexical-binding: t -*-

;;; Commentary:
;; A real command in a real session, for what a person feels: the time from
;; its key to its result, and how long the main VM kept the session from
;; running anything.  `perfbench-session-commands' names each command's
;; steps.  A repeating timer ticks for the whole session: a tick cannot run
;; while the main VM is busy, so the longest gap between ticks is how long
;; it blocked.  A command that only waits, as on a worker or a process,
;; leaves the ticks running however long its result takes.  A result counts
;; once it stands, the command has ended or reached a prompt, and it is on
;; the screen: the last redisplay is inside both figures.

;;; Code:

(require 'cl-lib)

(defvar perfbench-session-repo nil "A private clone of the Emacs tree, with src/xdisp.c changed.")
(defvar perfbench-session-directory nil "The 5,000 files.")
(defvar perfbench-session-agenda nil "An Org file of scheduled items.")
(defconst perfbench-session-commit "45b0f7699df36b3849edc3bdef37d4cb9e75b9ea")

(defun perfbench-session--file (name) (expand-file-name name perfbench-session-repo))

(defun perfbench-session--filled (buffer &optional text)
  "Whether BUFFER has text, TEXT among it if given, and no process is filling it."
  (let ((buffer (get-buffer buffer)))
    (and buffer (> (buffer-size buffer) 0) (not (get-buffer-process buffer))
         (or (null text)
             (with-current-buffer buffer
               (save-excursion (goto-char (point-min)) (search-forward text nil t)))))))

(defun perfbench-session--lines (buffer)
  (with-current-buffer buffer (count-lines (point-min) (point-max))))

(defun perfbench-session--xdisp ()
  (find-file (perfbench-session--file "src/xdisp.c"))
  (goto-char (point-min)))

(defun perfbench-session--kill (&rest buffers)
  (dolist (buffer buffers)
    (when (get-buffer buffer)
      (when (get-buffer-process buffer)
        (set-process-query-on-exit-flag (get-buffer-process buffer) nil))
      (with-current-buffer buffer (set-buffer-modified-p nil))
      (kill-buffer buffer))))

(defconst perfbench-session-commands
  `((find-file
     :run ,(lambda () (find-file (perfbench-session--file "src/xdisp.c")))
     :ready ,(lambda () (equal (buffer-name (window-buffer (selected-window))) "xdisp.c"))
     :check ,(lambda () (buffer-size (get-buffer "xdisp.c")))
     :cleanup ,(lambda () (perfbench-session--kill "xdisp.c")))
    (dired
     :run ,(lambda () (dired perfbench-session-directory))
     :ready ,(lambda () (derived-mode-p 'dired-mode))
     :check ,(lambda () (with-current-buffer (window-buffer (selected-window)) (count-lines (point-min) (point-max))))
     :cleanup ,(lambda () (kill-buffer (window-buffer (selected-window)))))
    (occur
     :setup ,#'perfbench-session--xdisp
     :run ,(lambda () (occur "glyph_row"))
     :ready ,(lambda () (perfbench-session--filled "*Occur*"))
     :check ,(lambda () (perfbench-session--lines "*Occur*"))
     :cleanup ,(lambda () (perfbench-session--kill "*Occur*" "xdisp.c")))
    (project-find-regexp
     :setup ,(lambda () (require 'project) (require 'xref)
               (setq project-list-file (expand-file-name "projects.eld" temporary-file-directory)))
     :run ,(lambda () (let ((default-directory (file-name-as-directory perfbench-session-repo)))
                        (project-find-regexp "\\<make_fixnum\\>")))
     :ready ,(lambda () (perfbench-session--filled "*xref*"))
     :check ,(lambda () (perfbench-session--lines "*xref*"))
     :cleanup ,(lambda () (perfbench-session--kill "*xref*")))
    (project-find-file
     :setup ,(lambda () (require 'project)
               (setq project-list-file (expand-file-name "projects.eld" temporary-file-directory)))
     :run ,(lambda () (let ((default-directory (file-name-as-directory perfbench-session-repo)))
                        (project-find-file)))
     :ready ,(lambda () (minibufferp (window-buffer (selected-window))))
     :check ,(lambda () 1)
     :cleanup ,#'ignore)
    (vc-log
     :setup ,(lambda () (require 'vc) (require 'vc-git))
     :run ,(lambda () (let ((default-directory (file-name-as-directory perfbench-session-repo)))
                        (vc-print-log-internal 'Git (list default-directory) perfbench-session-commit t 2000)))
     :ready ,(lambda () (perfbench-session--filled "*vc-change-log*"))
     :check ,(lambda () (perfbench-session--lines "*vc-change-log*"))
     :cleanup ,(lambda () (perfbench-session--kill "*vc-change-log*")))
    (vc-diff
     :setup ,(lambda () (require 'vc) (perfbench-session--xdisp))
     :run ,(lambda () (vc-diff))
     :ready ,(lambda () (perfbench-session--filled "*vc-diff*" "perfbench"))
     :check ,(lambda () (perfbench-session--lines "*vc-diff*"))
     :cleanup ,(lambda () (perfbench-session--kill "*vc-diff*" "xdisp.c")))
    (vc-dir
     :setup ,(lambda () (require 'vc-dir))
     :run ,(lambda () (vc-dir (file-name-as-directory perfbench-session-repo)))
     :ready ,(lambda () (let ((buffer (window-buffer (selected-window))))
                          (with-current-buffer buffer
                            (and (derived-mode-p 'vc-dir-mode) (not (vc-dir-busy))
                                 (save-excursion (goto-char (point-min)) (search-forward "xdisp.c" nil t))))))
     :check ,(lambda () 1)
     :cleanup ,(lambda () (perfbench-session--kill (window-buffer (selected-window)))))
    (shell
     :setup ,(lambda () (require 'shell) (setq shell-file-name "/bin/sh" explicit-shell-file-name "/bin/sh"))
     :run ,(lambda () (shell)
             (process-send-string (get-buffer-process "*shell*") "echo perfbench$((1+1))\n"))
     :ready ,(lambda () (let ((buffer (get-buffer "*shell*")))
                          (and buffer (with-current-buffer buffer
                                        (save-excursion (goto-char (point-min)) (search-forward "perfbench2" nil t))))))
     :check ,(lambda () 1)
     :cleanup ,(lambda () (perfbench-session--kill "*shell*")))
    (org-agenda
     :setup ,(lambda () (require 'org-agenda) (setq org-agenda-files (list perfbench-session-agenda)))
     :run ,(lambda () (org-agenda-list nil "2026-01-01" 'month))
     :ready ,(lambda () (perfbench-session--filled "*Org Agenda*"))
     :check ,(lambda () (perfbench-session--lines "*Org Agenda*"))
     :cleanup ,(lambda () (perfbench-session--kill "*Org Agenda*") (org-release-buffers org-agenda-new-buffers))))
  "Each session command: its :setup, untimed; :run, bound to F5 and timed
from that key; :ready, true once its result stands; :check, what it made,
compared across engines; :cleanup, untimed.")

(defun perfbench-session-text ()
  "A fresh edit buffer of 600 lines, its point at the start."
  (perfbench-session--kill "perfbench-edit")
  (switch-to-buffer (get-buffer-create "perfbench-edit"))
  (dotimes (line 600) (insert (format "line %d of some ordinary prose text, about sixty bytes long\n" line)))
  (goto-char (point-min))
  (setq buffer-undo-list nil))

(defun perfbench-session-kills ()
  "The edit buffer with something to yank."
  (perfbench-session-text)
  (kill-new "line of some ordinary prose text, about sixty bytes long\n"))

(defun perfbench-session-undos ()
  "The edit buffer with forty changes to undo."
  (perfbench-session-text)
  (dotimes (_ 40) (insert "line\n") (undo-boundary)))

(defun perfbench-session-org ()
  "ORG-NEWS in Org mode, at its first heading."
  (perfbench-session--kill "ORG-NEWS")
  (find-file (perfbench-session--file "etc/ORG-NEWS"))
  (org-mode)
  (goto-char (point-min))
  (outline-next-heading))

(defun perfbench-session-visible ()
  "How many of the next 4,000 characters show, for what Org folding did."
  (let ((shown 0))
    (dotimes (offset (min 4000 (- (point-max) (point))))
      (unless (invisible-p (+ (point) offset)) (setq shown (1+ shown))))
    shown))

(defvar perfbench-session--state nil "The armed command's steps.")
(defvar perfbench-session--file nil "Where to write its result.")
(defvar perfbench-session--started nil "When its key came.")
(defvar perfbench-session--ended nil "Whether its command has ended.")
(defvar perfbench-session--last nil "The last tick.")
(defvar perfbench-session-worst-gap 0.0 "The longest gap between ticks since the last reset.")

(defun perfbench-session--finish ()
  "Record the armed command, its result on the screen: this redisplay counts."
  (redisplay t)
  (let ((now (float-time))
        (inhibit-message t))
    (setq perfbench-session-worst-gap (max perfbench-session-worst-gap (- now perfbench-session--last))
          perfbench-session--last now)
    (write-region (format "(%S %S)\n" (- now perfbench-session--started) perfbench-session-worst-gap)
                  nil perfbench-session--file nil 'silent))
  (setq perfbench-session--state nil perfbench-session--started nil)
  ;; After the harness stops counting, so the probe's polling is not counted.
  (run-with-timer 0.6 nil #'perfbench-gui-resume))

(defun perfbench-session--tick ()
  (let ((now (float-time)))
    (when perfbench-session--last
      (setq perfbench-session-worst-gap (max perfbench-session-worst-gap (- now perfbench-session--last))))
    (setq perfbench-session--last now)
    (when (and perfbench-session--state perfbench-session--started
               (or perfbench-session--ended (> (minibuffer-depth) 0))
               (ignore-errors (funcall (plist-get perfbench-session--state :ready))))
      (perfbench-session--finish))))

(defun perfbench-session--begin ()
  (when (and perfbench-session--state (not perfbench-session--started))
    (setq perfbench-session--started (float-time)
          perfbench-session--ended nil
          perfbench-session--last perfbench-session--started
          perfbench-session-worst-gap 0.0)))

(defun perfbench-session--end ()
  (when perfbench-session--started (setq perfbench-session--ended t)))

(defun perfbench-session-reset ()
  "Start a new longest gap."
  (setq perfbench-session--last (float-time) perfbench-session-worst-gap 0.0)
  t)

(defun perfbench-session-arm (name file)
  "Run NAME's setup and bind F5 to it; its result goes to FILE."
  (let ((steps (alist-get name perfbench-session-commands)))
    (when-let* ((setup (plist-get steps :setup))) (funcall setup))
    (global-set-key [f5] (lambda () (interactive) (funcall (plist-get steps :run))))
    (setq perfbench-session--state steps perfbench-session--file file perfbench-session--started nil))
  t)

(defun perfbench-session-finish (name)
  "NAME's check, then its cleanup."
  (let* ((steps (alist-get name perfbench-session-commands))
         (check (funcall (plist-get steps :check))))
    (funcall (plist-get steps :cleanup))
    (perfbench-session--clear)
    check))

(defun perfbench-session--clear ()
  "Disarm, and put the windows back, for the next row."
  (setq perfbench-session--state nil perfbench-session--started nil)
  (delete-other-windows)
  (switch-to-buffer "*scratch*"))

(defun perfbench-session-abort (name)
  "After a failed row: NAME's cleanup if it has one, whatever state it left."
  (ignore-errors (funcall (plist-get (alist-get name perfbench-session-commands) :cleanup)))
  (perfbench-session--clear)
  t)

(add-hook 'pre-command-hook #'perfbench-session--begin)
(add-hook 'post-command-hook #'perfbench-session--end)
(run-at-time 0 0.005 #'perfbench-session--tick)

;;; perfbench-session.el ends here
