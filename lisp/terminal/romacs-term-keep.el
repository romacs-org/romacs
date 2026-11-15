;;; romacs-term-keep.el --- Terminals that outlive the session  -*- lexical-binding: t -*-
;; SPDX-License-Identifier: GPL-3.0-or-later
;; Maintainer: Thanos Apollo <public@thanosapollo.org>

;;; Commentary:
;; A terminal is a helper process of its own (the pty, the shell, the
;; emulator).  Here a session keeps that process in its keeper
;; (romacs-keeper.el), so the terminal outlives the session:
;;
;; - `romacs-restart' brings each terminal buffer back attached to its
;;   running terminal (a desktop handler, which warm-restart.el calls);
;; - a session that starts after one that crashed finds the terminals it
;;   left and brings them back too (`romacs-term-adopt-orphans').
;;
;; romacs-term.el itself runs in every session, also where GNU's Lisp is
;; not loaded, and knows of this only through `romacs-term-open-function'.

;;; Code:

(require 'romacs-keeper)

(defvar romacs-term-session)
(defvar romacs-term-handle)
(defvar romacs-term-open-function)
(declare-function romacs-term--prepare "romacs-term")
(declare-function romacs-term--bound "romacs-term")
(declare-function romacs--term-create "terminal.rs")
(declare-function romacs--term-adopt "terminal.rs")

(defvar romacs-keeper-enabled)
(defvar romacs-keeper-attachment)
(defvar desktop-save-buffer)
(defvar desktop-buffer-mode-handlers)
(defvar warm-restart-passive)
(defvar warm-restart-activate-functions)
(declare-function romacs-keeper-attached-p "romacs-keeper")
(declare-function romacs-keeper-attach "romacs-keeper")
(declare-function romacs-keeper-spawn "romacs-keeper")
(declare-function romacs-keeper-claim "romacs-keeper")
(declare-function romacs-keeper-close "romacs-keeper")
(declare-function romacs-keeper-handles "romacs-keeper")

(defun romacs-term--keeper ()
  "Non-nil when this session keeps its terminals in a keeper, attached now.
Nil only when `romacs-keeper-enabled' is off.  A keeper that cannot be
reached signals `romacs-keeper-unreachable': a terminal never goes
without a keeper the user did not turn off."
  (require 'romacs-keeper)
  (and romacs-keeper-enabled
       (or (romacs-keeper-attached-p) (romacs-keeper-attach))))

(defun romacs-term--say-error (text)
  "Show TEXT in the echo area as an error that did not stop a command."
  (message "%s" (propertize text 'face 'error)))

(declare-function romacs-keeper-events "romacs-keeper")

(defvar romacs-term-keep--watch nil
  "The timer that looks for terminals whose helper process has died.")

(defvar romacs-term-keep--dead nil
  "Handles of terminals whose helper has ended, still to be let go.")

(defun romacs-term-keep--release-dead ()
  "Let go of every terminal whose helper process has ended.
The keeper says when a helper ends.  Its shell may still be alive on the
pty the keeper holds, with no emulator and no session that could show
it: the handle is closed, which hangs the shell up, and the buffer
keeps its last screen as a terminal that has exited.  A close that
fails is tried again at the next call."
  (when (romacs-keeper-attached-p)
    (dolist (event (ignore-errors (romacs-keeper-events)))
      (let ((row (cdr event)))
        (when (and (eq (car event) 'status)
                   (equal (plist-get row :kind) "terminal")
                   (memq (car (plist-get row :status)) '(exited signaled)))
          (dolist (buffer (buffer-list))
            (when (eql (buffer-local-value 'romacs-term-handle buffer) (plist-get row :handle))
              ;; No longer a terminal a restart could bring back.
              (with-current-buffer buffer
                (setq romacs-term-handle nil)
                (kill-local-variable 'desktop-save-buffer))))
          (push (plist-get row :handle) romacs-term-keep--dead))))
    (when romacs-term-keep--dead
      ;; Only a listing that succeeded says a handle is gone: a keeper
      ;; that does not answer now is asked again at the next call.
      (let ((listed (condition-case nil
                        (cons t (mapcar (lambda (row) (plist-get row :handle)) (romacs-keeper-handles)))
                      (error nil))))
        (when listed
          (dolist (handle romacs-term-keep--dead)
            (when (or (not (memq handle (cdr listed)))
                      (ignore-errors (romacs-keeper-close handle) t))
              (setq romacs-term-keep--dead (delq handle romacs-term-keep--dead)))))))))

;;;###autoload
(defun romacs-term-keep-open (shell directory)
  "Start SHELL in DIRECTORY as the current buffer's terminal; its session.
With a keeper the terminal's helper process is the keeper's child and
outlives this session; `romacs-term-handle' names it."
  (if (not (romacs-term--keeper))
      (romacs--term-create shell directory)
    (let* ((host (expand-file-name invocation-name invocation-directory))
           (row (romacs-keeper-spawn "terminal" 1 (list host "--terminal-helper" shell)
                                     :environment process-environment
                                     :directory (directory-file-name (expand-file-name directory))
                                     ;; The keeper makes the pty and keeps its
                                     ;; master: the pty outlives the helper.
                                     :descriptors '((0 . null) (1 . null) (2 . null) (3 . socket) (4 . pty))
                                     :metadata (encode-coding-string (buffer-name) 'utf-8)))
           (handle (plist-get row :handle))
           (bound nil))
      (unwind-protect
          (prog1 (romacs--term-adopt handle)
            (unless romacs-term-keep--watch
              (setq romacs-term-keep--watch (run-with-timer 1 1 #'romacs-term-keep--release-dead)))
            (setq romacs-term-handle handle bound t)
            (setq-local desktop-save-buffer #'romacs-term--desktop-save))
        (unless bound (ignore-errors (romacs-keeper-close handle)))))))

(defun romacs-term--desktop-save (_dirname)
  "What a later session needs to find this terminal again."
  (and romacs-term-handle
       (let ((window (get-buffer-window (current-buffer) t)))
         (list :handle romacs-term-handle
               :incarnation (plist-get romacs-keeper-attachment :incarnation)
               :session (emacs-pid)
               :directory default-directory
               :columns (if window (window-body-width window) 80)
               :rows (if window (window-body-height window) 24)))))

(defun romacs-term--ended (why &optional face)
  "Leave the current buffer as a terminal that did not come back, saying WHY.
In FACE, by default `warning'."
  (let ((inhibit-read-only t))
    (erase-buffer)
    (insert (propertize (format "This terminal did not come back: %s.\n" why) 'face (or face 'warning)))
    (set-buffer-modified-p nil)))

(defvar romacs-term-reattach-seconds nil
  "How long each terminal took to come back, newest first: (BUFFER . SECONDS).")

(defun romacs-term--reattach (misc)
  "Attach the current buffer to the terminal MISC names, if it still runs.
The keeper must be the one that held it, and its helper process alive;
else the buffer says what became of it.  A buffer that already has its
terminal is left as it is."
  (unless romacs-term-session
    (let ((started (float-time)))
      (romacs-term--reattach-1 misc)
      (push (cons (buffer-name) (- (float-time) started)) romacs-term-reattach-seconds))))

(defun romacs-term--reattach-1 (misc)
  "Do `romacs-term--reattach' for a buffer with no terminal yet."
  (let* ((handle (plist-get misc :handle))
         (unreachable nil)
         (keeper (and (condition-case err (romacs-term--keeper)
                        (romacs-keeper-unreachable (setq unreachable (error-message-string err)) nil))
                      (equal (plist-get romacs-keeper-attachment :incarnation) (plist-get misc :incarnation))))
         (row nil))
    (when keeper
      (dolist (candidate (romacs-keeper-handles))
        (when (eql (plist-get candidate :handle) handle) (setq row candidate))))
    (cond
     (unreachable
      (romacs-term--ended unreachable 'error)
      (romacs-term--say-error (format "Terminal %s did not come back: %s" (buffer-name) unreachable)))
     ((not keeper) (romacs-term--ended "the keeper that held it is gone"))
     ((not row) (romacs-term--ended "its keeper no longer holds it"))
     ((not (equal (plist-get row :status) '(running)))
      ;; Its shell may still run on the pty the keeper holds, where
      ;; nothing can reach it: letting the handle go hangs it up.
      (ignore-errors (romacs-keeper-close handle))
      (romacs-term--ended (format "its helper process ended (%s)"
                                  (mapconcat (lambda (part) (format "%s" part)) (plist-get row :status) " "))))
     (t
      (romacs-keeper-claim handle (plist-get misc :session))
      (setq romacs-term-session
            (condition-case nil (romacs--term-adopt handle)
              ;; Not shown in a window yet: its last size, until it is.
              (error (romacs--term-adopt handle (plist-get misc :columns) (plist-get misc :rows)))))
      (setq romacs-term-handle handle)
      (unless romacs-term-keep--watch
        (setq romacs-term-keep--watch (run-with-timer 1 1 #'romacs-term-keep--release-dead)))
      (setq-local desktop-save-buffer #'romacs-term--desktop-save)
      (romacs-term--bound)))))

(defvar romacs-restart-command)
(defvar warm-restart-blocker-functions)
(declare-function romacs--term-helper-version "terminal.rs")

(defvar romacs-term--successor-versions nil
  "(PROGRAM MODIFIED LOW . HIGH): the helper versions PROGRAM's sessions read.")

(declare-function romacs-restart--command "warm-restart")

(defun romacs-term--successor-versions ()
  "The helper versions the build a restart starts reads, as (LOW . HIGH).
Nil when that build does not say within two seconds, as one older than
this rule does not.  The program is the one `romacs-restart' would start
from the selected window's buffer, where the command runs."
  (let* ((program (with-current-buffer (window-buffer (selected-window))
                    (car (car (romacs-restart--command)))))
         (modified (file-attribute-modification-time (file-attributes program))))
    (unless (and (equal (car romacs-term--successor-versions) program)
                 (equal (cadr romacs-term--successor-versions) modified))
      (setq romacs-term--successor-versions
            (cons program
                  (cons modified
                        (with-temp-buffer
                          ;; Bounded: a program that ignores the option
                          ;; must not hold the session.
                          (and (file-executable-p program)
                               (executable-find "timeout")
                               (eql 0 (ignore-errors
                                        (call-process "timeout" nil t nil "--kill-after=1" "2"
                                                      program "--terminal-versions")))
                               (progn (goto-char (point-min))
                                      (looking-at "\\([0-9]+\\) \\([0-9]+\\)$"))
                               (cons (string-to-number (match-string 1))
                                     (string-to-number (match-string 2)))))))))
    (cddr romacs-term--successor-versions)))

;;;###autoload
(defun romacs-term--restart-blocker ()
  "Why the current terminal buffer cannot be handed to the next build, or nil.
A terminal's helper process speaks the version it was started with for
as long as the terminal lives.  A restart into a build that cannot read
it would leave the terminal running and out of reach, so it is refused
here, while this session still serves it."
  (when (and romacs-term-handle romacs-term-session)
    (let ((speaks (romacs--term-helper-version romacs-term-session))
          (reads (romacs-term--successor-versions)))
      (when (and speaks reads (not (<= (car reads) speaks (cdr reads))))
        (format "terminal %s speaks helper version %d, and the build being started reads %d-%d; close it first"
                (buffer-name) speaks (car reads) (cdr reads))))))

;;;###autoload
(with-eval-after-load 'warm-restart
  (add-hook 'warm-restart-blocker-functions #'romacs-term--restart-blocker))

;;;###autoload
(defun romacs-term-desktop-restore (_file name misc)
  "The terminal buffer NAME of MISC, attached again to its running terminal."
  (let ((buffer (get-buffer-create name)))
    (with-current-buffer buffer
      (romacs-term--prepare (or (plist-get misc :directory) default-directory)))
    (let ((attach (lambda ()
                    (when (buffer-live-p buffer)
                      (with-current-buffer buffer (romacs-term--reattach misc))))))
      (if (bound-and-true-p warm-restart-passive)
          (setq warm-restart-activate-functions (append warm-restart-activate-functions (list attach)))
        (funcall attach)))
    buffer))

(declare-function romacs-keeper-socket "romacs-keeper")

;;;###autoload
(defun romacs-term-adopt-orphans ()
  "Bring back the terminals a session that is gone left running.
A session that crashed, or was killed, leaves its terminals in its
keeper with their programs running.  Each comes back in a buffer of the
name it had.  Return those buffers.  Does nothing when no keeper runs
for this session.  One that runs and cannot be reached signals
`romacs-keeper-unreachable'."
  (interactive)
  (require 'romacs-keeper)
  (let ((buffers nil))
    (when (and romacs-keeper-enabled
               (or (romacs-keeper-attached-p) (file-exists-p (romacs-keeper-socket)))
               (romacs-term--keeper))
      (dolist (row (romacs-keeper-handles))
        ;; A helper that died while no session had it: its shell may live
        ;; on the kept pty.  Nobody is told of it again, so it goes here.
        (when (and (equal (plist-get row :kind) "terminal")
                   (not (plist-get row :owner-alive))
                   (memq (car (plist-get row :status)) '(exited signaled)))
          (ignore-errors (romacs-keeper-close (plist-get row :handle))))
        (let ((name (decode-coding-string (plist-get row :metadata) 'utf-8)))
          (when (and (equal (plist-get row :kind) "terminal")
                     (not (plist-get row :owner-alive))
                     (equal (plist-get row :status) '(running))
                     (not (string-empty-p name))
                     (not (and (get-buffer name) (buffer-local-value 'romacs-term-session (get-buffer name)))))
            (with-current-buffer (get-buffer-create name)
              (romacs-term--prepare default-directory)
              (romacs-term--reattach (list :handle (plist-get row :handle)
                                           :incarnation (plist-get romacs-keeper-attachment :incarnation)
                                           :columns 80 :rows 24))
              (push (current-buffer) buffers)))))
      (when buffers
        (message "Terminals brought back: %s" (mapconcat #'buffer-name buffers ", "))))
    (nreverse buffers)))

;;;###autoload
(defun romacs-term--adopt-saying-why ()
  "Do `romacs-term-adopt-orphans', and say so when it fails."
  (condition-case err (romacs-term-adopt-orphans)
    (error (romacs-term--say-error (format "Terminals a gone session left are not back: %s"
                                           (error-message-string err))))))

;; A session that starts where a keeper already runs looks, at the end of
;; its startup, for terminals nobody owns.  An image restored after that
;; finds its terminal buffers already attached and leaves them so.
;;;###autoload
(progn
  (defun romacs-term--adopt-at-startup ()
    "Once started, bring back terminals a gone session left."
    (unless noninteractive
      (run-at-time 0 nil #'romacs-term--adopt-saying-why)))
  (add-hook 'after-init-hook #'romacs-term--adopt-at-startup))

;; In every session from its start: a restored one must find the handler
;; before this library is loaded.
;;;###autoload
(with-eval-after-load 'desktop
  (add-to-list 'desktop-buffer-mode-handlers '(romacs-term-mode . romacs-term-desktop-restore)))

;;;###autoload
(setq romacs-term-open-function #'romacs-term-keep-open)

(provide 'romacs-term-keep)
;;; romacs-term-keep.el ends here
