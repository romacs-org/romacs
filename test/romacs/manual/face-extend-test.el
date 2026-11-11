;;; face-extend-test.el --- Test face :extend in fido-vertical-mode -*- lexical-binding: t -*-

;;; Code:

(defun face-extend-test--run ()
  "Run the face :extend test with fido-vertical-mode."
  (fido-vertical-mode 1)
  (icomplete-mode 1)
  (set-face-attribute 'icomplete-selected-match nil :extend t)
  ;; Schedule exit
  (run-at-time 60 nil (lambda () (kill-emacs 0))))

(add-hook 'emacs-startup-hook #'face-extend-test--run)

;;; face-extend-test.el ends here
