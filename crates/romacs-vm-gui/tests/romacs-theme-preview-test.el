;;; romacs-theme-preview-test.el --- Showing a theme before loading it -*- lexical-binding: t -*-
;; SPDX-License-Identifier: GPL-3.0-or-later
;; Maintainer: Thanos Apollo <public@thanosapollo.org>

;; rlisp/romacs-custom.el's preview in a `--loadup' session, which starts in
;; the stock theme.  The keys of a real selection:
;; harness/theme_preview_live.py.

(require 'romacs-theme)
(require 'romacs-mode-line)

(defun romacs-preview-test--state ()
  "What a preview must leave as it found."
  (list custom-enabled-themes romacs-theme--stock
        (frame-parameter nil 'background-mode)
        (mapcar (lambda (face) (list face (face-attribute face :foreground) (face-attribute face :background)))
                '(default mode-line cursor region font-lock-keyword-face romacs-mode-line-mark))
        (mapcar (lambda (face) (mapcar #'car (get face 'theme-face))) '(default mode-line))))

(defun romacs-preview-test--file (directory name body)
  "Write the theme NAME with BODY into DIRECTORY."
  (with-temp-file (expand-file-name (format "%s-theme.el" name) directory)
    (insert (format "(deftheme %s \"A test.\")\n%s\n(provide-theme '%s)\n" name body name))))

(defmacro romacs-preview-test--themes (bindings &rest body)
  "BODY with a theme directory in the first of BINDINGS, the themes
enabled now back after it."
  (declare (indent 1))
  `(let* ((,(car bindings) (make-temp-file "romacs-preview" t))
          (custom-theme-load-path (cons ,(car bindings) custom-theme-load-path))
          (romacs-preview-test--enabled custom-enabled-themes)
          (romacs-preview-test--stock romacs-theme--stock))
     (unwind-protect (progn ,@body)
       (mapc #'disable-theme custom-enabled-themes)
       (mapc #'enable-theme (reverse romacs-preview-test--enabled))
       (setq romacs-theme--stock romacs-preview-test--stock)
       (delete-directory ,(car bindings) t))))

(romacs-deftest romacs-theme-preview-leaves-the-session-as-it-found-it ()
  (let ((before (romacs-preview-test--state)))
    (romacs-theme-preview--session
      (romacs-theme-preview--show 'catppuccin-latte)
      (romacs-should (memq 'catppuccin-latte custom-enabled-themes))
      (romacs-should (eq (frame-parameter nil 'background-mode) 'light))
      (romacs-should-not (equal (face-attribute 'default :background)
                                (nth 2 (assq 'default (nth 3 before)))))
      (romacs-theme-preview--show 'tokyonight-night)
      (romacs-should (memq 'tokyonight-night custom-enabled-themes))
      (romacs-should-not (memq 'catppuccin-latte custom-enabled-themes)))
    (romacs-should (equal (romacs-preview-test--state) before))))

(romacs-deftest romacs-theme-preview-shows-what-load-theme-gives ()
  ;; The stock theme gives way to a colour scheme in a preview as it does
  ;; to `load-theme', on the second candidate as on the first.
  (romacs-theme-preview--session
    (romacs-theme-preview--show 'tokyonight-night)
    (let ((shown custom-enabled-themes))
      (romacs-theme-preview--show 'catppuccin-latte)
      (romacs-theme-preview--show 'tokyonight-night)
      (romacs-should (equal custom-enabled-themes shown))))
  (let ((stock romacs-theme--stock) (themes custom-enabled-themes))
    (unwind-protect
        (progn
          (load-theme 'tokyonight-night t)
          (let ((loaded custom-enabled-themes))
            (mapc #'disable-theme custom-enabled-themes)
            (mapc #'enable-theme (reverse themes))
            (setq romacs-theme--stock stock)
            (romacs-theme-preview--session
              (romacs-theme-preview--show 'tokyonight-night)
              (romacs-should (equal custom-enabled-themes loaded)))))
      (mapc #'disable-theme custom-enabled-themes)
      (mapc #'enable-theme (reverse themes))
      (setq romacs-theme--stock stock))))

(romacs-deftest romacs-theme-preview-keeps-a-frame-s-local-edit ()
  (let ((frame (selected-frame)))
    (set-face-attribute 'region frame :underline t)
    (unwind-protect
        (progn
          (romacs-theme-preview--session
            (romacs-theme-preview--show 'catppuccin-latte)
            (romacs-should-not (eq (face-attribute 'region :underline frame) t)))
          (romacs-should (eq (face-attribute 'region :underline frame) t)))
      (set-face-attribute 'region frame :underline 'unspecified))))

(romacs-deftest romacs-theme-preview-tells-the-hooks-of-an-empty-stack ()
  ;; With no theme under the preview nothing is enabled at its end, and
  ;; what the enable hooks derive must still come back.
  (romacs-preview-test--themes (directory)
    (mapc #'disable-theme custom-enabled-themes)
    (let* ((state (lambda () (list (face-background 'romacs-mode-line-pill)
                                   (get 'romacs-mode-line-pill 'face-defface-spec)
                                   (get 'romacs-mode-line-pill 'face-modified))))
           (pills (funcall state)))
      (romacs-theme-preview--session (romacs-theme-preview--show 'catppuccin-latte))
      (romacs-should (equal (funcall state) pills)))))

(romacs-deftest romacs-theme-preview-ends-with-the-hooks-told ()
  ;; A hook that derives something from the faces has seen the last
  ;; candidate; with no theme under the preview, nothing else would run it.
  (romacs-preview-test--themes (directory)
    (mapc #'disable-theme custom-enabled-themes)
    (let* ((seen nil)
           (hook (lambda (&rest _) (setq seen (face-background 'default))))
           (background (face-background 'default))
           (settings (get 'user 'theme-settings))
           (stack (get 'default 'theme-face)))
      (add-hook 'enable-theme-functions hook)
      (unwind-protect
          (progn
            (romacs-theme-preview--session
              (romacs-theme-preview--show 'tokyonight-night)
              (romacs-should-not (equal seen background)))
            (romacs-should (equal seen background))
            ;; What they are told of is what stays: a local edit included.
            (set-face-attribute 'default (selected-frame) :background "#123456")
            (romacs-theme-preview--session
              (romacs-theme-preview--show 'tokyonight-night))
            (romacs-should (equal seen "#123456"))
            (romacs-should (equal (face-background 'default) "#123456"))
            ;; So is a local edit over a face `user' sets.
            (custom-theme-set-faces 'user '(default ((t :background "#334455"))))
            (set-face-attribute 'default (selected-frame) :background "#123456")
            (romacs-theme-preview--session
              (romacs-theme-preview--show 'tokyonight-night))
            (romacs-should (equal seen "#123456"))
            (romacs-should (equal (face-background 'default) "#123456")))
        (remove-hook 'enable-theme-functions hook)
        (put 'user 'theme-settings settings)
        (put 'default 'theme-face stack)
        (face-spec-recalc 'default (selected-frame))
        (set-face-attribute 'default (selected-frame) :background background)))))

(romacs-deftest romacs-theme-preview-never-reads-a-theme-load-theme-would-ask-about ()
  (romacs-preview-test--themes (directory)
    (let ((custom-safe-themes '(default))
          (before (romacs-preview-test--state)))
      (defvar romacs-preview-test-ran nil)
      (setq romacs-preview-test-ran nil)
      (romacs-preview-test--file directory 'romacs-preview-unsafe "(setq romacs-preview-test-ran t)")
      (romacs-theme-preview--session
        (romacs-theme-preview--show 'romacs-preview-unsafe)
        (romacs-should-not romacs-preview-test-ran)
        (romacs-should-not (memq 'romacs-preview-unsafe custom-enabled-themes)))
      (romacs-should (equal (romacs-preview-test--state) before))
      ;; Trusted by its hash, as `load-theme' trusts it, it is read.
      (let ((custom-safe-themes
             (list (with-temp-buffer
                     (insert-file-contents (expand-file-name "romacs-preview-unsafe-theme.el" directory))
                     (secure-hash 'sha256 (current-buffer))))))
        (romacs-theme-preview--session
          (romacs-theme-preview--show 'romacs-preview-unsafe)
          (romacs-should romacs-preview-test-ran)
          (romacs-should (memq 'romacs-preview-unsafe custom-enabled-themes)))))))

(romacs-deftest romacs-theme-preview-reads-the-file-it-approved ()
  ;; The hash approves one file; a sibling `load' would prefer must not
  ;; run in its place.
  (romacs-preview-test--themes (directory)
    (defvar romacs-preview-test-file nil)
    (setq romacs-preview-test-file nil)
    (let ((approved (expand-file-name "romacs.preview-theme.elc" directory)))
      (with-temp-file approved
        (insert "(deftheme romacs.preview \"Approved.\")\n(setq romacs-preview-test-file 'approved)\n(provide-theme 'romacs.preview)\n"))
      (with-temp-file (expand-file-name "romacs.preview-theme" directory)
        (insert "(setq romacs-preview-test-file 'sibling)\n"))
      (let ((custom-safe-themes
             (list (with-temp-buffer (insert-file-contents approved) (secure-hash 'sha256 (current-buffer))))))
        (romacs-theme-preview--session
          (romacs-theme-preview--show 'romacs.preview))
        (romacs-should (eq romacs-preview-test-file 'approved))))))

(romacs-deftest romacs-theme-preview-loads-a-candidate-over-the-first-themes ()
  ;; A theme file may read the session: what it reads is the themes the
  ;; preview began with, not the candidate before it.
  (romacs-preview-test--themes (directory)
    (let ((custom-safe-themes t) (background (face-background 'default)))
      (romacs-preview-test--file
       directory 'romacs-preview-reader
       "(custom-theme-set-faces 'romacs-preview-reader `(highlight ((t :background ,(face-background 'default)))))")
      (put 'romacs-preview-reader 'theme-settings nil)
      (romacs-theme-preview--session
        (romacs-theme-preview--show 'tokyonight-night)
        (romacs-theme-preview--show 'romacs-preview-reader)
        (romacs-should (equal (face-background 'highlight) background))))))

(romacs-deftest romacs-theme-preview-takes-back-a-candidate-that-failed ()
  ;; `enable-theme' stops at an invalid face with what came before it in
  ;; place, and the theme not recorded as enabled.
  (romacs-preview-test--themes (directory)
    (let ((custom-safe-themes t) (before (romacs-preview-test--state)))
      (romacs-preview-test--file
       directory 'romacs-preview-broken
       "(custom-theme-set-faces 'romacs-preview-broken '(highlight ((t :foreground \"#123456\"))) '(region ((t :weight nonsense))) '(shadow ((t :foreground \"#654321\"))))")
      (romacs-theme-preview--session
        (romacs-theme-preview--show 'romacs-preview-broken)
        (romacs-should-not (memq 'romacs-preview-broken custom-enabled-themes)))
      (dolist (face '(highlight shadow))
        (romacs-should-not (assq 'romacs-preview-broken (get face 'theme-face))))
      (romacs-should (equal (romacs-preview-test--state) before)))))

(romacs-deftest romacs-theme-preview-restores-past-a-failing-setter ()
  ;; A setter that cannot restore its variable stops `disable-theme'; the
  ;; variables after it still come back, whichever order they fall in.
  (romacs-preview-test--themes (directory)
    (defvar romacs-preview-test-first nil)
    (defvar romacs-preview-test-second nil)
    (let ((custom-safe-themes t)
          (setter (lambda (symbol value)
                    (when (eq value 'standard) (error "Cannot restore %s" symbol))
                    (set-default symbol value))))
      (dolist (case '((romacs-preview-test-first . romacs-preview-test-second)
                      (romacs-preview-test-second . romacs-preview-test-first)))
        (set-default (car case) 'standard)
        (set-default (cdr case) 'standard)
        (put (car case) 'standard-value '('standard))
        (put (cdr case) 'standard-value '('standard))
        (put (car case) 'custom-set setter)
        (put (cdr case) 'custom-set nil)
        (let ((theme (intern (format "romacs-preview-%s" (car case)))))
          (romacs-preview-test--file
           directory theme
           (format "(custom-theme-set-variables '%s '(romacs-preview-test-first 'themed) '(romacs-preview-test-second 'themed))" theme))
          (romacs-theme-preview--session
            (romacs-theme-preview--show theme)
            (romacs-should (eq (symbol-value (cdr case)) 'themed)))
          (romacs-should (eq (symbol-value (cdr case)) 'standard))
          (romacs-should-not (get (cdr case) 'theme-value)))
        (put (car case) 'custom-set nil)))))

(romacs-deftest romacs-theme-preview-of-a-refused-theme-replays-nothing ()
  ;; Nothing was applied, so `user''s settings are not applied again over
  ;; a value that has changed since they were.
  (romacs-preview-test--themes (directory)
    (defvar romacs-preview-test-custom 'saved)
    (let ((custom-safe-themes '(default)) (settings (get 'user 'theme-settings)))
      (custom-theme-set-variables 'user '(romacs-preview-test-custom 'saved))
      (unwind-protect
          (progn
            (setq romacs-preview-test-custom 'changed)
            (romacs-preview-test--file directory 'romacs-preview-refused "(ignore)")
            (romacs-theme-preview--session
              (romacs-theme-preview--show 'romacs-preview-refused))
            (romacs-should (eq romacs-preview-test-custom 'changed)))
        (put 'user 'theme-settings settings)
        (put 'romacs-preview-test-custom 'theme-value nil)))))

(romacs-deftest romacs-theme-preview-finishes-past-a-failing-hook ()
  (let* ((before (romacs-preview-test--state))
         (hook (lambda (&rest _) (error "A hook failed"))))
    (romacs-theme-preview--session
      (romacs-theme-preview--show 'tokyonight-night)
      (add-hook 'disable-theme-functions hook))
    (remove-hook 'disable-theme-functions hook)
    (romacs-should (equal (romacs-preview-test--state) before))))

(romacs-deftest romacs-theme-preview-runs-the-hooks-and-setters-of-each-candidate ()
  (romacs-preview-test--themes (directory)
    (let* ((custom-safe-themes t)
           (enabled nil)
           (hook (lambda (theme) (push theme enabled))))
      (defvar romacs-preview-test-variable 'standard)
      (romacs-preview-test--file directory 'romacs-preview-variable
                                 "(custom-theme-set-variables 'romacs-preview-variable '(romacs-preview-test-variable 'themed))")
      (add-hook 'enable-theme-functions hook)
      (unwind-protect
          (romacs-theme-preview--session
            (romacs-theme-preview--show 'romacs-preview-variable)
            (romacs-should (memq 'romacs-preview-variable enabled))
            (romacs-should (eq romacs-preview-test-variable 'themed))
            (romacs-theme-preview--show nil)
            (romacs-should (eq romacs-preview-test-variable 'standard)))
        (remove-hook 'enable-theme-functions hook)))))

(romacs-deftest romacs-theme-preview-keeps-load-theme-s-prompt-as-a-command ()
  (romacs-should (equal (interactive-form 'load-theme) '(interactive (romacs-theme-preview-read))))
  (let ((romacs-theme-preview nil) (asked nil))
    (cl-letf (((symbol-function 'completing-read)
               (lambda (prompt names &rest _) (setq asked (list prompt names)) "tokyonight-night")))
      (romacs-should (equal (romacs-theme-preview-read) '(tokyonight-night nil nil))))
    (romacs-should (equal (car asked) "Load custom theme: "))
    (romacs-should (member "tokyonight-night" (cadr asked)))))

;;; romacs-theme-preview-test.el ends here
