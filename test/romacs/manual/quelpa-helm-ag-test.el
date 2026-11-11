;;; quelpa-helm-ag-test.el --- Install and verify helm-ag via use-package :quelpa -*- lexical-binding: t -*-

(let ((quelpa-build-verbose-p nil))
  (use-package helm-ag
    :quelpa (helm-ag :fetcher github :repo "emacsattic/helm-ag"
                     :files ("*.el") :depends nil)))

(message "TEST: use-package helm-ag completed")

(if (fboundp 'helm-ag)
    (message "TEST PASS: helm-ag is fboundp")
  (message "TEST FAIL: helm-ag is not fboundp"))

(if (fboundp 'helm-do-ag)
    (message "TEST PASS: helm-do-ag is fboundp")
  (message "TEST FAIL: helm-do-ag is not fboundp"))

(if (boundp 'helm-ag-base-command)
    (message "TEST PASS: helm-ag-base-command = %S" helm-ag-base-command)
  (message "TEST FAIL: helm-ag-base-command not bound"))

(if (featurep 'helm-ag)
    (message "TEST PASS: helm-ag feature provided")
  (message "TEST FAIL: helm-ag feature not provided"))

(if (package-installed-p 'helm-ag)
    (message "TEST PASS: helm-ag package installed")
  (message "TEST FAIL: helm-ag package not installed"))

(message "TEST: done")

(kill-emacs 0)
