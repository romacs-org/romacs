;;; romacs-workspace-completion-tests.el --- The workspace's readers stay its own -*- lexical-binding: t; -*-
(require 'ert)
(require 'cl-lib)

(defun romacs-workspace-completion-tests--readers ()
  "The readers of command, file and buffer names in place now."
  (list (symbol-function 'execute-extended-command) (symbol-function 'read-extended-command)
        (symbol-function 'read-file-name) read-buffer-function read-file-name-function))

(defun romacs-workspace-completion-tests--gnus-p ()
  "Whether GNU's readers are in place: simple.el's M-x, minibuffer.el's
file reader and minibuf.c's buffer reader."
  (and (string-suffix-p "/simple.el" (symbol-file 'execute-extended-command 'defun))
       (string-suffix-p "/simple.el" (symbol-file 'read-extended-command 'defun))
       (string-suffix-p "/minibuffer.el" (symbol-file 'read-file-name 'defun))
       (eq read-file-name-function 'read-file-name-default)
       (null read-buffer-function)))

(ert-deftest romacs-workspace-completion-load-keeps-gnus-readers ()
  "Loading the workspace's readers where GNU's minibuffer.el is, as C-h f
does completing a `romacs-' name, leaves GNU's M-x, C-x b and C-x C-f."
  (should (featurep 'minibuffer))
  (should (romacs-workspace-completion-tests--gnus-p))
  (let ((before (romacs-workspace-completion-tests--readers))
        (doc (get 'romacs-complete--candidates 'variable-documentation)))
    (load "romacs-workspace-completion" nil t)
    (should (romacs-workspace-completion-tests--gnus-p))
    (should (equal (romacs-workspace-completion-tests--readers) before))
    (should (equal (get 'romacs-complete--candidates 'variable-documentation) doc))))

(ert-deftest romacs-workspace-completion-load-installs-without-gnus-minibuffer ()
  "Where GNU's minibuffer.el is not loaded, the workspace regime, loading
the file puts its readers in place."
  (cl-letf (((symbol-function 'execute-extended-command) nil)
            ((symbol-function 'read-extended-command) nil)
            ((symbol-function 'read-file-name) nil))
    (let ((read-buffer-function nil) (features (remq 'minibuffer features)) (load-history load-history))
      (load "romacs-workspace-completion" nil t)
      (should (equal (butlast (romacs-workspace-completion-tests--readers))
                     (list #'romacs-execute-extended-command #'romacs-read-extended-command
                           #'romacs-workspace-read-file-name #'romacs-read-buffer))))))

;;; romacs-workspace-completion-tests.el ends here
