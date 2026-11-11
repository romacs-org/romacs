;;; markdown-romacs-tests.el --- Bundled Markdown face declarations -*- lexical-binding: t; -*-

;; Copyright (C) 2026 Thanos Apollo

;; This program is free software; you can redistribute it and/or modify
;; it under the terms of the GNU General Public License as published by
;; the Free Software Foundation, either version 3 of the License, or
;; (at your option) any later version.

(require 'ert)
(require 'markdown-mode)

(ert-deftest markdown-romacs-header-face-declarations ()
  (dotimes (index 6)
    (let ((face (intern (format "markdown-header-face-%d" (1+ index)))))
      (should (equal (get face 'face-defface-spec)
                     '((t (:inherit markdown-header-face :height 1.0)))))
      (should (string-prefix-p (format "Face for level %d headers." (1+ index))
                               (documentation-property face 'face-documentation))))))

(ert-deftest markdown-romacs-header-face-scaling ()
  (let* ((faces (mapcar (lambda (level)
                          (intern (format "markdown-header-face-%d" level)))
                        '(1 2 3 4 5 6)))
         (original (mapcar (lambda (face) (face-attribute face :height nil)) faces))
         (scales '(2.0 1.7 1.4 1.1 1.0 1.0)))
    (unwind-protect
        (progn
          (markdown-update-header-faces t scales)
          (should (equal (mapcar (lambda (face) (face-attribute face :height nil)) faces)
                         scales))
          (markdown-update-header-faces nil)
          (should (equal (mapcar (lambda (face) (face-attribute face :height nil)) faces)
                         '(1.0 1.0 1.0 1.0 1.0 1.0))))
      (cl-mapc (lambda (face height) (set-face-attribute face nil :height height))
               faces original))))

(provide 'markdown-romacs-tests)
;;; markdown-romacs-tests.el ends here
