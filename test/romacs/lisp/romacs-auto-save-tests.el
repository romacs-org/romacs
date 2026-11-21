;;; romacs-auto-save-tests.el --- do-auto-save's loop  -*- lexical-binding: t -*-
;; SPDX-License-Identifier: GPL-3.0-or-later
;; Maintainer: Thanos Apollo <public@thanosapollo.org>

;; `do-auto-save', whose loop is lisp/vm/auto-save.el's
;; `romacs--do-auto-save' over the native auto_save_1
;; (crates/romacs-vm/src/prims/autosave.rs): which buffers it saves, what
;; it writes, the auto-save list, the shrink rule, its hook and messages.

(require 'ert)
(require 'cl-lib)

(defmacro romacs-auto-save-tests--with (names &rest body)
  "BODY with a buffer visiting a file under a fresh directory `dir' for
each of NAMES, bound to it, its text \"text\" and modified, its auto-save
file named; and `auto-save-list-file-name' nil."
  (declare (indent 1))
  `(let* ((dir (make-temp-file "romacs-auto-save-" t))
          (auto-save-list-file-name nil)
          (auto-save-hook nil)
          ,@(mapcar (lambda (name)
                      `(,name (let ((buffer (generate-new-buffer ,(symbol-name name))))
                                (with-current-buffer buffer
                                  (setq buffer-file-name (expand-file-name ,(symbol-name name) dir))
                                  (insert "text")
                                  (setq buffer-auto-save-file-name
                                        (expand-file-name ,(format "#%s#" name) dir)))
                                buffer)))
                    names))
     (unwind-protect (progn ,@body)
       (dolist (buffer (list ,@names))
         (with-current-buffer buffer (set-buffer-modified-p nil))
         (kill-buffer buffer))
       (delete-directory dir t))))

(defun romacs-auto-save-tests--text (buffer)
  "The text of BUFFER's auto-save file, or nil when there is none."
  (let ((file (buffer-local-value 'buffer-auto-save-file-name buffer)))
    (and (file-exists-p file)
         (with-temp-buffer (insert-file-contents file) (buffer-string)))))

(ert-deftest romacs-auto-save-saves-each-modified-buffer ()
  (romacs-auto-save-tests--with (a b)
    (with-current-buffer b (set-buffer-modified-p nil))
    (should (null (do-auto-save t)))
    (should (equal (romacs-auto-save-tests--text a) "text"))
    (should (null (romacs-auto-save-tests--text b)))
    (with-current-buffer a
      (should (recent-auto-save-p))
      (should (= buffer-saved-size (buffer-size))))
    ;; Saved once: not again until it changes.
    (delete-file (buffer-local-value 'buffer-auto-save-file-name a))
    (do-auto-save t)
    (should (null (romacs-auto-save-tests--text a)))
    (with-current-buffer a (insert " more"))
    (do-auto-save t)
    (should (equal (romacs-auto-save-tests--text a) "text more"))))

(ert-deftest romacs-auto-save-current-only-saves-the-current-buffer ()
  (romacs-auto-save-tests--with (a b)
    (with-current-buffer b (do-auto-save t t))
    (should (null (romacs-auto-save-tests--text a)))
    (should (equal (romacs-auto-save-tests--text b) "text"))))

(ert-deftest romacs-auto-save-skips-a-buffer-auto-saving-is-off-in ()
  (romacs-auto-save-tests--with (a)
    (with-current-buffer a (setq buffer-saved-size -1))
    (do-auto-save t)
    (should (null (romacs-auto-save-tests--text a)))))

(ert-deftest romacs-auto-save-a-shrunk-buffer-is-not-saved-until-a-real-save ()
  (romacs-auto-save-tests--with (a)
    (with-current-buffer a
      (insert (make-string 6000 ?x))
      (setq buffer-saved-size (buffer-size))
      (delete-region 100 (point-max)))
    (let ((messages nil))
      (cl-letf (((symbol-function 'sleep-for) #'ignore)
                ((symbol-function 'message)
                 (lambda (format &rest args) (push (apply #'format-message format args) messages))))
        (do-auto-save))
      (should (member "Buffer a has shrunk a lot; auto save disabled in that buffer until next real save" messages)))
    (should (null (romacs-auto-save-tests--text a)))
    (should (= (buffer-local-value 'buffer-saved-size a) -1))
    ;; Unless big deletions are included.
    (with-current-buffer a (setq buffer-saved-size 6004))
    (let ((auto-save-include-big-deletions t)) (do-auto-save t))
    (should (= (length (romacs-auto-save-tests--text a)) 99))))

(ert-deftest romacs-auto-save-runs-its-hook-and-says-so ()
  (romacs-auto-save-tests--with (a)
    (let ((ran 0) (messages nil))
      (add-hook 'auto-save-hook (lambda () (setq ran (1+ ran))))
      (cl-letf (((symbol-function 'message)
                 (lambda (format &rest args) (push (and format (apply #'format-message format args)) messages)))
                ((symbol-function 'current-message) #'ignore))
        (do-auto-save))
      (should (= ran 1))
      (should (equal (reverse messages) '("Auto-saving..." "Auto-saving...done")))
      ;; Nothing to save: the hook still runs, and nothing is said.
      (setq messages nil)
      (cl-letf (((symbol-function 'message)
                 (lambda (format &rest args) (push (and format (apply #'format-message format args)) messages))))
        (do-auto-save))
      (should (= ran 2))
      (should (null messages)))))

(ert-deftest romacs-auto-save-lists-every-auto-saving-buffer ()
  (romacs-auto-save-tests--with (a b)
    (with-current-buffer b (set-buffer-modified-p nil))
    (let ((auto-save-list-file-name (expand-file-name "list/saves" dir)))
      (do-auto-save t)
      (let ((listed (with-temp-buffer (insert-file-contents auto-save-list-file-name) (buffer-string))))
        (dolist (buffer (list a b))
          (should (string-search (concat (buffer-local-value 'buffer-file-name buffer) "\n"
                                         (buffer-local-value 'buffer-auto-save-file-name buffer) "\n")
                                 listed)))))))

(ert-deftest romacs-auto-save-a-failed-save-warns-and-goes-on ()
  (romacs-auto-save-tests--with (a b)
    (with-current-buffer a
      (setq buffer-auto-save-file-name (expand-file-name "missing/#a#" dir)))
    (let ((warned nil) (messages nil))
      (cl-letf (((symbol-function 'display-warning) (lambda (type text &rest _) (push (list type text) warned)))
                ((symbol-function 'ding) #'ignore)
                ((symbol-function 'current-message) #'ignore)
                ((symbol-function 'message)
                 (lambda (format &rest args) (push (and format (apply #'format-message format args)) messages))))
        (do-auto-save))
      (should (equal (mapcar #'car warned) '(auto-save)))
      (should (string-prefix-p "Auto-saving a: " (cadr (car warned))))
      (should-not (member "Auto-saving...done" messages)))
    (should (equal (romacs-auto-save-tests--text b) "text"))))

(ert-deftest romacs-auto-save-keeps-the-current-buffer-and-a-pending-quit ()
  (romacs-auto-save-tests--with (a)
    (with-temp-buffer
      (let ((here (current-buffer)))
        (do-auto-save t)
        (should (eq (current-buffer) here))))
    (should (equal (romacs-auto-save-tests--text a) "text"))
    ;; A quit pending before it is pending after it.
    (let ((inhibit-quit t))
      (with-current-buffer a (insert " more"))
      (setq quit-flag t)
      (do-auto-save t)
      (should quit-flag)
      (setq quit-flag nil))
    (should (equal (romacs-auto-save-tests--text a) "text more"))))

;;; romacs-auto-save-tests.el ends here
