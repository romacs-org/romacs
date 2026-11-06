;;; romacs-theme-preview-test.el --- Showing a theme before loading it -*- lexical-binding: t -*-
;; SPDX-License-Identifier: GPL-3.0-or-later
;; Maintainer: Thanos Apollo <public@thanosapollo.org>

;; rlisp/romacs-custom.el's preview in a `--loadup' session, which starts in
;; the stock theme.  The keys of a real selection:
;; harness/theme_preview_live.py.

(require 'romacs-theme)

(defun romacs-preview-test--state ()
  "What a preview must leave as it found."
  (list custom-enabled-themes romacs-theme--stock
        (frame-parameter nil 'background-mode)
        (mapcar (lambda (face) (list face (face-attribute face :foreground) (face-attribute face :background)))
                '(default mode-line cursor region font-lock-keyword-face romacs-mode-line-mark))
        (mapcar (lambda (face) (mapcar #'car (get face 'theme-face))) '(default mode-line))))

(defmacro romacs-preview-test--with (&rest body)
  "BODY as `romacs-theme-preview-read' runs a selection."
  `(let* ((romacs-theme-preview--base custom-enabled-themes)
          (romacs-theme--stock romacs-theme--stock)
          (romacs-theme-preview--stock romacs-theme--stock))
     ,@body))

(defun romacs-preview-test--file (directory name body)
  "Write the theme NAME with BODY into DIRECTORY."
  (with-temp-file (expand-file-name (format "%s-theme.el" name) directory)
    (insert (format "(deftheme %s \"A test.\")\n%s\n(provide-theme '%s)\n" name body name))))

(romacs-deftest romacs-theme-preview-leaves-the-session-as-it-found-it ()
  (let ((before (romacs-preview-test--state)))
    (romacs-preview-test--with
     (romacs-theme-preview--show 'modus-operandi)
     (romacs-should (memq 'modus-operandi custom-enabled-themes))
     (romacs-should (eq (frame-parameter nil 'background-mode) 'light))
     (romacs-should-not (equal (face-attribute 'default :background)
                               (nth 2 (assq 'default (nth 3 before)))))
     (romacs-theme-preview--show 'tango-dark)
     (romacs-should (memq 'tango-dark custom-enabled-themes))
     (romacs-should-not (memq 'modus-operandi custom-enabled-themes))
     (romacs-theme-preview--show nil))
    (romacs-should (equal (romacs-preview-test--state) before))))

(romacs-deftest romacs-theme-preview-shows-what-load-theme-gives ()
  ;; The stock theme gives way to a colour scheme in a preview as it does
  ;; to `load-theme', on the second candidate as on the first.
  (romacs-preview-test--with
   (romacs-theme-preview--show 'tango-dark)
   (let ((shown custom-enabled-themes))
     (romacs-theme-preview--show 'modus-operandi)
     (romacs-theme-preview--show 'tango-dark)
     (romacs-should (equal custom-enabled-themes shown)))
   (romacs-theme-preview--show nil))
  (let ((stock romacs-theme--stock) (themes custom-enabled-themes))
    (unwind-protect
        (progn
          (load-theme 'tango-dark t)
          (let ((loaded custom-enabled-themes))
            (mapc #'disable-theme custom-enabled-themes)
            (mapc #'enable-theme (reverse themes))
            (setq romacs-theme--stock stock)
            (romacs-preview-test--with
             (romacs-theme-preview--show 'tango-dark)
             (romacs-should (equal custom-enabled-themes loaded))
             (romacs-theme-preview--show nil))))
      (mapc #'disable-theme custom-enabled-themes)
      (mapc #'enable-theme (reverse themes))
      (setq romacs-theme--stock stock))))

(romacs-deftest romacs-theme-preview-never-reads-a-theme-load-theme-would-ask-about ()
  (let* ((directory (make-temp-file "romacs-preview" t))
         (custom-theme-load-path (cons directory custom-theme-load-path))
         (custom-safe-themes '(default))
         (before (romacs-preview-test--state)))
    (defvar romacs-preview-test-ran nil)
    (setq romacs-preview-test-ran nil)
    (romacs-preview-test--file directory 'romacs-preview-unsafe "(setq romacs-preview-test-ran t)")
    (unwind-protect
        (progn
          (romacs-should-not (romacs-theme-preview-safe-p 'romacs-preview-unsafe))
          (romacs-preview-test--with (romacs-theme-preview--show 'romacs-preview-unsafe))
          (romacs-should-not romacs-preview-test-ran)
          (romacs-should (equal (romacs-preview-test--state) before))
          ;; Trusted by its hash, as `load-theme' trusts it, it is read.
          (let ((custom-safe-themes
                 (list (with-temp-buffer
                         (insert-file-contents (expand-file-name "romacs-preview-unsafe-theme.el" directory))
                         (secure-hash 'sha256 (current-buffer))))))
            (romacs-should (romacs-theme-preview-safe-p 'romacs-preview-unsafe))
            (romacs-preview-test--with
             (romacs-theme-preview--show 'romacs-preview-unsafe)
             (romacs-should romacs-preview-test-ran)
             (romacs-theme-preview--show nil))))
      (delete-directory directory t))))

(romacs-deftest romacs-theme-preview-runs-the-hooks-and-setters-of-each-candidate ()
  (let* ((directory (make-temp-file "romacs-preview" t))
         (custom-theme-load-path (cons directory custom-theme-load-path))
         (custom-safe-themes t)
         (enabled nil)
         (hook (lambda (theme) (push theme enabled))))
    (defvar romacs-preview-test-variable 'standard)
    (romacs-preview-test--file directory 'romacs-preview-variable
                               "(custom-theme-set-variables 'romacs-preview-variable '(romacs-preview-test-variable 'themed))")
    (add-hook 'enable-theme-functions hook)
    (unwind-protect
        (romacs-preview-test--with
         (romacs-theme-preview--show 'romacs-preview-variable)
         (romacs-should (memq 'romacs-preview-variable enabled))
         (romacs-should (eq romacs-preview-test-variable 'themed))
         (romacs-theme-preview--show nil)
         (romacs-should (eq romacs-preview-test-variable 'standard)))
      (remove-hook 'enable-theme-functions hook)
      (delete-directory directory t))))

(romacs-deftest romacs-theme-preview-keeps-load-theme-s-prompt-as-a-command ()
  (romacs-should (equal (interactive-form 'load-theme) '(interactive (romacs-theme-preview-read))))
  (let ((romacs-theme-preview nil) (asked nil))
    (cl-letf (((symbol-function 'completing-read)
               (lambda (prompt names &rest _) (setq asked (list prompt names)) "tango-dark")))
      (romacs-should (equal (romacs-theme-preview-read) '(tango-dark nil nil))))
    (romacs-should (equal (car asked) "Load custom theme: "))
    (romacs-should (member "tango-dark" (cadr asked)))))

;;; romacs-theme-preview-test.el ends here
