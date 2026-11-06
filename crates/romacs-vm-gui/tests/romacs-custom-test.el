;;; romacs-custom-test.el --- Saved settings stay out of the init -*- lexical-binding: t -*-
;; SPDX-License-Identifier: GPL-3.0-or-later
;; Maintainer: Thanos Apollo <public@thanosapollo.org>

;; rlisp/romacs-custom.el in a `--loadup' session, with GNU's cus-edit
;; (gnu_completion.rs `romacs_custom_lisp_tests_pass').  The journey through
;; a real session: harness/options_live.py.

(require 'cus-edit)
(require 'novice)

(defconst romacs-custom-test--init ";; hand written\n(setq fill-column 72)\n")

(defun romacs-custom-test--text (file)
  "FILE's text, or nil if it does not exist."
  (when (file-exists-p file)
    (with-temp-buffer (insert-file-contents file) (buffer-string))))

(defmacro romacs-custom-test--with (spec &rest body)
  "Run BODY in a fresh directory with an init, bound as SPEC's (DIR INIT DEFAULT).
Saved buffers go with it; `custom-file' starts nil."
  (declare (indent 1))
  (let ((dir (nth 0 spec)) (init (nth 1 spec)) (default (nth 2 spec)))
    `(let* ((,dir (file-name-as-directory (make-temp-file "romacs-custom" t)))
            (,init (expand-file-name "init.el" ,dir))
            (,default (expand-file-name "custom.el" ,dir))
            (user-emacs-directory ,dir)
            (user-init-file ,init)
            (custom-file nil)
            (init-file-had-error nil))
       (write-region romacs-custom-test--init nil ,init nil 'silent)
       (unwind-protect (progn ,@body)
         (dolist (buffer (buffer-list))
           (when (string-prefix-p ,dir (or (buffer-file-name buffer) ""))
             (with-current-buffer buffer (set-buffer-modified-p nil))
             (kill-buffer buffer)))
         (delete-directory ,dir t)))))

(romacs-deftest romacs-custom-a-nil-custom-file-saves-beside-the-init ()
  ;; GNU's answer here is the init.
  (romacs-custom-test--with (dir init default)
    (romacs-should (equal (custom-file) default))
    (customize-save-variable 'kill-ring-max 123)
    (romacs-should (equal (romacs-custom-test--text init) romacs-custom-test--init))
    (romacs-should (string-search "kill-ring-max 123" (romacs-custom-test--text default)))))

(romacs-deftest romacs-custom-an-absent-init-stays-absent ()
  (romacs-custom-test--with (dir init default)
    (delete-file init)
    (romacs-should (equal (custom-file) default))
    (customize-save-variable 'kill-ring-max 124)
    (disable-command 'forward-word)
    (romacs-should-not (file-exists-p init))
    (romacs-should (string-search "'forward-word 'disabled t" (romacs-custom-test--text default)))))

(romacs-deftest romacs-custom-naming-the-init-saves-beside-it ()
  ;; By its own name, and through a link: neither reaches the init.
  (romacs-custom-test--with (dir init default)
    (let ((link (expand-file-name "link.el" dir)))
      (make-symbolic-link init link)
      (dolist (name (list init link))
        (let ((custom-file name))
          (romacs-should (equal (custom-file) default))))
      (let ((custom-file link))
        (customize-save-variable 'kill-ring-max 125))
      (romacs-should (equal (romacs-custom-test--text init) romacs-custom-test--init))
      (romacs-should (string-search "kill-ring-max 125" (romacs-custom-test--text default))))))

(romacs-deftest romacs-custom-a-chosen-file-is-kept ()
  (romacs-custom-test--with (dir init default)
    (let ((custom-file (expand-file-name "mine.el" dir)))
      (romacs-should (equal (custom-file) custom-file))
      (customize-save-variable 'kill-ring-max 126)
      (romacs-should (string-search "kill-ring-max 126" (romacs-custom-test--text custom-file)))
      (romacs-should-not (file-exists-p default)))))

(romacs-deftest romacs-custom-a-default-linked-to-the-init-is-refused ()
  (romacs-custom-test--with (dir init default)
    (make-symbolic-link init default)
    (romacs-should-not (custom-file t))
    (romacs-should-error (custom-file) :type 'user-error)
    ;; The session has it, as GNU's `-q' does; nothing is written.
    (customize-save-variable 'kill-ring-max 127)
    (disable-command 'forward-word)
    (romacs-should (equal (romacs-custom-test--text init) romacs-custom-test--init))
    (romacs-should (eq (get 'forward-word 'disabled) t))
    (put 'forward-word 'disabled nil)))

(romacs-deftest romacs-custom-a-quick-session-writes-nothing ()
  (romacs-custom-test--with (dir init default)
    (let ((user-init-file nil) (process-environment (cons (concat "HOME=" dir) process-environment)))
      (romacs-should-not (custom-file t))
      (disable-command 'forward-word)
      (romacs-should (eq (get 'forward-word 'disabled) t))
      (romacs-should-not (file-exists-p default))
      (romacs-should-not (file-exists-p (expand-file-name ".emacs" dir)))
      (put 'forward-word 'disabled nil))))

(romacs-deftest romacs-custom-disable-command-writes-the-literal-name ()
  ;; GNU substitutes the name it opens: this one would become the init.
  (romacs-custom-test--with (dir init default)
    (let* ((process-environment (cons (concat "ROMACS_CUSTOM_TEST=" init) process-environment))
           (custom-file (concat dir "$ROMACS_CUSTOM_TEST")))
      (disable-command 'forward-word)
      (romacs-should (equal (romacs-custom-test--text init) romacs-custom-test--init))
      (romacs-should (string-search "'forward-word 'disabled t" (romacs-custom-test--text custom-file)))
      (put 'forward-word 'disabled nil))))

(romacs-deftest romacs-custom-commands-keep-one-line-through-custom-saves ()
  (romacs-custom-test--with (dir init default)
    (disable-command 'forward-word)
    (enable-command 'forward-word)
    (customize-save-variable 'kill-ring-max 128)
    (let ((text (romacs-custom-test--text default)))
      (romacs-should (string-search "(put 'forward-word 'disabled nil)" text))
      (romacs-should-not (string-search "'forward-word 'disabled t" text))
      (romacs-should (string-search "kill-ring-max 128" text)))))

(romacs-deftest romacs-custom-an-invalid-command-is-refused-without-a-file ()
  (romacs-custom-test--with (dir init default)
    (let ((user-init-file nil))
      (romacs-should-error (disable-command 'romacs-custom-no-such))
      (romacs-should-not (get 'romacs-custom-no-such 'disabled)))))

(romacs-deftest romacs-custom-start-loads-the-default-and-keeps-a-choice ()
  (romacs-custom-test--with (dir init default)
    (write-region "(setq romacs-custom-test--loaded 'default)\n" nil default nil 'silent)
    (defvar romacs-custom-test--loaded nil)
    (romacs-custom-start)
    (romacs-should (equal custom-file default))
    (romacs-should (eq romacs-custom-test--loaded 'default))
    (let ((custom-file (expand-file-name "mine.el" dir)))
      (write-region "(setq romacs-custom-test--loaded 'mine)\n" nil custom-file nil 'silent)
      (romacs-custom-start)
      (romacs-should (eq romacs-custom-test--loaded 'mine)))))

(romacs-deftest romacs-custom-a-broken-file-does-not-stop-the-session ()
  (romacs-custom-test--with (dir init default)
    (write-region "(error \"broken settings\")\n" nil default nil 'silent)
    (romacs-custom-start)
    (romacs-should (equal custom-file default))))
;;; romacs-custom-test.el ends here
