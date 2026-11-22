;;; romacs-bundled-autoloads-tests.el --- What the runtime scan registers -*- lexical-binding: t; -*-
(require 'ert)
(require 'romacs-bundled-autoloads)

(defun romacs-bundled-autoloads-tests--prefixes (library)
  "The `register-definition-prefixes' forms the scan makes of LIBRARY."
  (let ((file (locate-library (concat library ".el") t)))
    (should file)
    (seq-filter (lambda (form) (eq (car-safe form) 'register-definition-prefixes))
                (romacs-bundled-autoloads--forms (file-name-directory file) (list file)))))

(ert-deftest romacs-bundled-autoloads-one-regime-files-register-no-prefixes ()
  "Help's completion loads the files whose prefixes match what is typed:
C-h f dired would load the workspace's dired over GNU's, C-h v
buffer-name-history its minibuffer readers, C-h v
default-file-name-coding-system its coding state.  Files only one
regime loads register no prefixes; a file of both still does."
  (dolist (library '("romacs-dired" "romacs-coding" "romacs-workspace-completion" "romacs-ert-batch"))
    (should (equal (list library (romacs-bundled-autoloads-tests--prefixes library)) (list library nil))))
  (should (romacs-bundled-autoloads-tests--prefixes "romacs-find-file")))

;;; romacs-bundled-autoloads-tests.el ends here
