;;; romacs-gui-command-line-tests.el --- The command line's rest after the init  -*- lexical-binding: t -*-
;; SPDX-License-Identifier: GPL-3.0-or-later
;; Maintainer: Thanos Apollo <public@thanosapollo.org>

;; lisp/session/romacs-gui-command-line.el, in batch: the buffers
;; `romacs-gui-loadup-command-line-1' shows and what it returns, the
;; startup hooks and frame settings, the startup screen's buffer and
;; window.  The GUI's whole start: the startup, world and layout_journey
;; harnesses.

(require 'ert)
(require 'cl-lib)
(require 'romacs-gui-command-line)

;; lisp/session/romacs-startup.el's, which a batch session does not load.
(defvar romacs-gui-displayable-buffers nil)

(defmacro romacs-gui-command-line-tests--with (&rest body)
  "BODY in one window, with the command line's inputs as a session
without a startup screen has them; `noticed' counts the frame settings
applied, `ran' collects the startup hooks that ran."
  (declare (indent 0))
  `(save-window-excursion
     (delete-other-windows)
     (let* ((created nil)
            (romacs-gui-displayable-buffers nil)
            (initial-buffer-choice nil) (initial-scratch-message nil)
            (inhibit-startup-screen t) (inhibit-startup-buffer-menu nil) (inhibit-startup-hooks nil)
            (inhibit-x-resources nil)
            (noticed 0) (ran nil)
            (emacs-startup-hook (list (lambda () (push 'emacs-startup ran))))
            (term-setup-hook (list (lambda () (push 'term-setup ran))))
            (window-setup-hook (list (lambda () (push 'window-setup ran)))))
       (cl-letf (((symbol-function 'romacs-gui-loadup-notice-frame-settings)
                  (lambda () (cl-incf noticed) (push 'frame-settings ran))))
         (unwind-protect (progn ,@body)
           (dolist (buffer created) (when (buffer-live-p buffer) (kill-buffer buffer))))))))

(defun romacs-gui-command-line-tests--buffers (&rest names)
  "Fresh buffers named NAMES, as the file arguments' visits leave them."
  (mapcar (lambda (name) (let ((buffer (get-buffer-create name)))
                           (with-current-buffer buffer (erase-buffer))
                           buffer))
          names))

(defun romacs-gui-command-line-tests--shown ()
  "The buffers the windows show, the selected window's first."
  (mapcar #'window-buffer (window-list nil 'no-minibuf (selected-window))))

(ert-deftest romacs-gui-command-line-shows-the-visited-buffers ()
  (romacs-gui-command-line-tests--with
    (let ((buffers (romacs-gui-command-line-tests--buffers " a" " b" " c")))
      (setq created buffers)
      ;; One: shown alone.
      (setq romacs-gui-displayable-buffers (list (nth 0 buffers)))
      (should (null (romacs-gui-loadup-command-line-1)))
      (should (null romacs-gui-displayable-buffers))
      (should (equal (romacs-gui-command-line-tests--shown) (list (nth 0 buffers))))
      ;; Two: beside each other, the first selected.
      (setq romacs-gui-displayable-buffers (list (nth 0 buffers) (nth 1 buffers)))
      (romacs-gui-loadup-command-line-1)
      (should (equal (romacs-gui-command-line-tests--shown) (list (nth 0 buffers) (nth 1 buffers))))
      ;; Three: the first and the second shown, the third visited before
      ;; the second in its window, the buffer menu listed in an interactive
      ;; session.
      (delete-other-windows)
      (setq romacs-gui-displayable-buffers buffers)
      (let ((noninteractive nil) (listed nil))
        (cl-letf (((symbol-function 'list-buffers) (lambda (&rest _) (setq listed t))))
          (romacs-gui-loadup-command-line-1))
        (should listed))
      (should (equal (romacs-gui-command-line-tests--shown) (list (nth 0 buffers) (nth 1 buffers)))))))

(ert-deftest romacs-gui-command-line-takes-the-initial-buffer-choice ()
  (romacs-gui-command-line-tests--with
    (let* ((buffers (romacs-gui-command-line-tests--buffers " a" " chosen"))
           (initial-buffer-choice (lambda () (nth 1 buffers))))
      (setq created buffers romacs-gui-displayable-buffers (list (nth 0 buffers)))
      ;; Its buffer comes first, ahead of the visited ones.
      (should (null (romacs-gui-loadup-command-line-1)))
      (should (equal (romacs-gui-command-line-tests--shown) (list (nth 1 buffers) (nth 0 buffers))))
      (let ((initial-buffer-choice (lambda () 'gone)))
        (should (string-match-p "not a live buffer"
                                (cadr (should-error (romacs-gui-loadup-command-line-1))))))
      (let ((initial-buffer-choice 7))
        (should (string-match-p "must be a string, a function, or t"
                                (cadr (should-error (romacs-gui-loadup-command-line-1)))))))))

(ert-deftest romacs-gui-command-line-fills-an-empty-scratch ()
  (romacs-gui-command-line-tests--with
    (let ((scratch (get-buffer "*scratch*")) (initial-scratch-message ";; \\[find-file]\n"))
      (when scratch (kill-buffer scratch))
      (let ((initial-buffer-choice t))
        (romacs-gui-loadup-command-line-1))
      (with-current-buffer "*scratch*"
        (should (equal (buffer-string) (substitute-command-keys ";; \\[find-file]\n")))
        (should-not (buffer-modified-p)))
      (should (eq (window-buffer) (get-buffer "*scratch*")))
      ;; A scratch with text keeps it.
      (with-current-buffer "*scratch*" (erase-buffer) (insert "kept"))
      (romacs-gui-loadup-command-line-1)
      (should (equal (with-current-buffer "*scratch*" (buffer-string)) "kept")))))

(ert-deftest romacs-gui-command-line-a-startup-screen-runs-the-startup-hooks ()
  (romacs-gui-command-line-tests--with
    (let ((inhibit-startup-screen nil) (noninteractive nil) (buffers (romacs-gui-command-line-tests--buffers " a")))
      (setq created buffers)
      (should (eq (romacs-gui-loadup-command-line-1) 'full))
      (should (equal (reverse ran) '(emacs-startup term-setup frame-settings window-setup)))
      (should (= noticed 1))
      (should inhibit-startup-hooks)
      ;; Beside a visited buffer, the concise screen.
      (setq romacs-gui-displayable-buffers buffers inhibit-startup-hooks nil)
      (should (eq (romacs-gui-loadup-command-line-1) 'concise))
      ;; No startup screen: no hooks here; normal-top-level runs them.
      (setq ran nil noticed 0 inhibit-startup-hooks nil)
      (let ((inhibit-startup-screen t)) (should (null (romacs-gui-loadup-command-line-1))))
      (should (null ran))
      (should (= noticed 0)))))

(ert-deftest romacs-gui-command-line-normal-top-level-runs-what-is-left ()
  (romacs-gui-command-line-tests--with
    (romacs-gui-loadup-normal-top-level)
    (should (equal (reverse ran) '(emacs-startup term-setup frame-settings window-setup)))
    (should (= noticed 1))
    ;; Hooks the startup screen ran do not run again; the frame settings do.
    (setq ran nil)
    (let ((inhibit-startup-hooks t)) (romacs-gui-loadup-normal-top-level))
    (should (equal ran '(frame-settings)))
    (should (= noticed 2))))

(ert-deftest romacs-gui-command-line-the-startup-screen-s-buffer-and-window ()
  ;; Without the animated logo, which a batch session has not loaded.
  (skip-unless (not (fboundp 'romacs-landing)))
  (romacs-gui-command-line-tests--with
    (let* ((name " *romacs-gui-command-line-test*")
           (command-line-default-directory "/tmp/")
           (buffer (romacs-gui-loadup-startup-buffer name)))
      (setq created (list buffer))
      (with-current-buffer buffer
        (should (equal (buffer-string) "Romacs\n"))
        (should buffer-read-only)
        (should (equal default-directory "/tmp/"))
        (should (eq (current-local-map) splash-screen-keymap)))
      (should (eq (romacs-gui-loadup-startup-buffer name) buffer))
      ;; Full: in the selected window.
      (romacs-gui-loadup-startup-screen buffer nil)
      (should (eq (window-buffer) buffer))
      ;; Concise: in another window, the selected one kept.
      (let ((other (car (romacs-gui-command-line-tests--buffers " other"))))
        (push other created)
        (switch-to-buffer other)
        (romacs-gui-loadup-startup-screen buffer t)
        (should (eq (window-buffer) other))
        (should (get-buffer-window buffer))))))

;;; romacs-gui-command-line-tests.el ends here
