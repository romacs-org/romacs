;;; driver.el --- The editor side of tools/package-parity  -*- lexical-binding: t -*-

;; SPDX-License-Identifier: GPL-3.0-or-later

;;; Commentary:

;; Loaded by pinned GNU and by Romacs alike, in batch, with a private HOME.
;; Every result is one line, "PP " and a JSON object, which the tool reads.
;; `PP_ELPA' names the package directory the run installs into or loads from.

;;; Code:

(require 'package)
(require 'package-vc)
(require 'json)

(setq package-archives nil
      package-user-dir (getenv "PP_ELPA")
      package-vc-register-as-project nil
      ;; A dependency missing from the manifest fails the install, not a prompt.
      package-vc-allow-build-commands nil)

(defun pp-emit (object)
  "Write OBJECT, an alist, as one result line."
  (princ (format "PP %s\n" (json-encode object)) #'external-debugging-output))

(defun pp-print (value)
  "VALUE as the tool compares it: printed, bounded, without properties."
  (let ((print-length 60) (print-level 8) (print-escape-newlines t))
    (let ((text (prin1-to-string value)))
      (substring-no-properties text 0 (min (length text) 2000)))))

(defun pp-install (spec rev)
  "Install the package of package-vc SPEC at REV."
  (package-initialize)
  (let ((name (car spec))
        (start (float-time)))
    (condition-case err
        (progn
          (package-vc-install spec rev)
          (let ((desc (cadr (assq name package-alist))))
            (pp-emit `((event . "install") (ok . ,(if desc t :json-false))
                       (error . ,(unless desc "no package descriptor after install"))
                       (seconds . ,(- (float-time) start))))))
      (error
       (pp-emit `((event . "install") (ok . :json-false) (error . ,(pp-print err))
                  (seconds . ,(- (float-time) start))))))))

(defun pp-activate (loads)
  "Activate the installed packages and put LOADS first on `load-path'."
  (package-initialize)
  (dolist (directory (reverse loads))
    (add-to-list 'load-path directory)))

(defun pp--step (name thunk)
  (let ((result (condition-case err (cons 'value (funcall thunk))
                  (error (cons 'error err)))))
    (pp-emit `((event . "step") (name . ,name)
               (,(car result) . ,(pp-print (cdr result)))))))

(defmacro pp-step (name &rest body)
  "Run BODY as journey step NAME and record its value or its error."
  (declare (indent 1))
  `(pp--step ,name (lambda () ,@body)))

(defun pp-keys (keys)
  "Run KEYS, a `kbd' string, through the command loop."
  (execute-kbd-macro (kbd keys)))

(defun pp-buffer-text (&optional buffer lines)
  "The first LINES (default 20) lines of BUFFER, without properties."
  (with-current-buffer (or buffer (current-buffer))
    (save-excursion
      (goto-char (point-min))
      (forward-line (or lines 20))
      (buffer-substring-no-properties (point-min) (point)))))

(defun pp-git (directory &rest args)
  "Run git with ARGS in DIRECTORY; signal on failure."
  (let ((default-directory (file-name-as-directory directory)))
    (unless (zerop (apply #'call-process "git" nil nil nil args))
      (error "git %S failed" args))))

(defun pp-fixture-repo (name files)
  "A fresh Git repository NAME under HOME with FILES, (PATH . TEXT), committed."
  (let ((directory (expand-file-name name (getenv "HOME"))))
    (make-directory directory t)
    (pp-git directory "init" "-q" "-b" "main")
    (dolist (file files)
      (let ((path (expand-file-name (car file) directory)))
        (make-directory (file-name-directory path) t)
        (with-temp-file path (insert (cdr file)))))
    (pp-git directory "add" ".")
    (pp-git directory "commit" "-q" "-m" "Initial")
    directory))

(provide 'pp-driver)
;;; driver.el ends here
