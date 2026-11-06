;;; romacs-saved-settings-test.el --- Saved settings stay out of the init -*- lexical-binding: t -*-
;; SPDX-License-Identifier: GPL-3.0-or-later
;; Maintainer: Thanos Apollo <public@thanosapollo.org>

;; rlisp/romacs-saved-settings.el in a `--loadup' session, with GNU's cus-edit
;; (gnu_completion.rs `romacs_saved_settings_lisp_tests_pass').  The journey through
;; a real session: harness/options_live.py.

(require 'cus-edit)
(require 'novice)

(defconst romacs-saved-settings-test--init ";; hand written\n(setq fill-column 72)\n")

(defun romacs-saved-settings-test--text (file)
  "FILE's text, or nil if it does not exist."
  (when (file-exists-p file)
    (with-temp-buffer (insert-file-contents file) (buffer-string))))

(defmacro romacs-saved-settings-test--with (spec &rest body)
  "Run BODY in a fresh directory with an init, bound as SPEC's (DIR INIT DEFAULT).
Saved buffers go with it; `custom-file' starts nil."
  (declare (indent 1))
  (let ((dir (nth 0 spec)) (init (nth 1 spec)) (default (nth 2 spec)))
    `(let* ((,dir (file-name-as-directory (make-temp-file "romacs-saved-settings" t)))
            (,init (expand-file-name "init.el" ,dir))
            (,default (expand-file-name "custom.el" ,dir))
            (user-emacs-directory ,dir)
            (user-init-file ,init)
            (custom-file nil)
            (init-file-had-error nil))
       (write-region romacs-saved-settings-test--init nil ,init nil 'silent)
       (unwind-protect (progn ,@body)
         (dolist (buffer (buffer-list))
           (when (string-prefix-p ,dir (or (buffer-file-name buffer) ""))
             (with-current-buffer buffer (set-buffer-modified-p nil))
             (kill-buffer buffer)))
         (delete-directory ,dir t)))))

(romacs-deftest romacs-saved-settings-a-nil-custom-file-saves-beside-the-init ()
  ;; GNU's answer here is the init.
  (romacs-saved-settings-test--with (dir init default)
    (romacs-should (equal (custom-file) default))
    (customize-save-variable 'kill-ring-max 123)
    (romacs-should (equal (romacs-saved-settings-test--text init) romacs-saved-settings-test--init))
    (romacs-should (string-search "kill-ring-max 123" (romacs-saved-settings-test--text default)))))

(romacs-deftest romacs-saved-settings-an-absent-init-stays-absent ()
  (romacs-saved-settings-test--with (dir init default)
    (delete-file init)
    (romacs-should (equal (custom-file) default))
    (customize-save-variable 'kill-ring-max 124)
    (disable-command 'forward-word)
    (romacs-should-not (file-exists-p init))
    (romacs-should (string-search "'forward-word 'disabled t" (romacs-saved-settings-test--text default)))))

(romacs-deftest romacs-saved-settings-naming-the-init-saves-beside-it ()
  ;; By its own name, and through a link: neither reaches the init.
  (romacs-saved-settings-test--with (dir init default)
    (let ((link (expand-file-name "link.el" dir)))
      (make-symbolic-link init link)
      (dolist (name (list init link))
        (let ((custom-file name))
          (romacs-should (equal (custom-file) default))))
      (let ((custom-file link))
        (customize-save-variable 'kill-ring-max 125))
      (romacs-should (equal (romacs-saved-settings-test--text init) romacs-saved-settings-test--init))
      (romacs-should (string-search "kill-ring-max 125" (romacs-saved-settings-test--text default))))))

(romacs-deftest romacs-saved-settings-dots-past-a-link-are-read-as-a-writer-reads-them ()
  ;; `file-truename' takes this through the link to another directory;
  ;; `find-file-noselect' expands it first, to the init.
  (romacs-saved-settings-test--with (dir init default)
    (let ((link (expand-file-name "link" dir)) (elsewhere (make-temp-file "romacs-saved-settings-other" t)))
      (unwind-protect
          (let ((child (expand-file-name "child" elsewhere)))
            (make-directory child)
            (make-symbolic-link child link)
            (let ((custom-file (concat link "/../init.el")))
              (romacs-should (equal (custom-file) default))
              (customize-save-variable 'kill-ring-max 129)
              (disable-command 'forward-word))
            (romacs-should (equal (romacs-saved-settings-test--text init) romacs-saved-settings-test--init))
            (put 'forward-word 'disabled nil))
        (delete-directory elsewhere t)))))

(romacs-deftest romacs-saved-settings-a-hard-link-to-the-init-is-the-init ()
  ;; With the init visited, `find-file-noselect' would hand back its buffer.
  (romacs-saved-settings-test--with (dir init default)
    (let ((hard (expand-file-name "hard.el" dir)))
      (add-name-to-file init hard)
      (find-file-noselect init)
      (let ((custom-file hard))
        (romacs-should (equal (custom-file) default))
        (disable-command 'forward-word))
      (romacs-should (equal (romacs-saved-settings-test--text init) romacs-saved-settings-test--init))
      (romacs-should (string-search "'forward-word 'disabled t" (romacs-saved-settings-test--text default)))
      (put 'forward-word 'disabled nil))))

(romacs-deftest romacs-saved-settings-a-default-hard-linked-to-the-init-is-refused ()
  (romacs-saved-settings-test--with (dir init default)
    (add-name-to-file init default)
    (find-file-noselect init)
    (romacs-should-not (custom-file t))
    (disable-command 'forward-word)
    (romacs-should (equal (romacs-saved-settings-test--text init) romacs-saved-settings-test--init))
    (put 'forward-word 'disabled nil)))

(romacs-deftest romacs-saved-settings-start-does-not-run-the-init-through-a-link ()
  (romacs-saved-settings-test--with (dir init default)
    (defvar romacs-saved-settings-test--runs 0)
    (setq romacs-saved-settings-test--runs 0)
    (write-region "(setq romacs-saved-settings-test--runs (1+ romacs-saved-settings-test--runs))\n" nil init nil 'silent)
    (make-symbolic-link init default)
    (romacs-saved-settings-start)
    (romacs-should (= romacs-saved-settings-test--runs 0))))

(romacs-deftest romacs-saved-settings-a-chosen-file-is-kept ()
  (romacs-saved-settings-test--with (dir init default)
    (let ((custom-file (expand-file-name "mine.el" dir)))
      (romacs-should (equal (custom-file) custom-file))
      (customize-save-variable 'kill-ring-max 126)
      (romacs-should (string-search "kill-ring-max 126" (romacs-saved-settings-test--text custom-file)))
      (romacs-should-not (file-exists-p default)))))

(romacs-deftest romacs-saved-settings-a-default-linked-to-the-init-is-refused ()
  (romacs-saved-settings-test--with (dir init default)
    (make-symbolic-link init default)
    (romacs-should-not (custom-file t))
    (romacs-should-error (custom-file) :type 'user-error)
    ;; The session has it, as GNU's `-q' does; nothing is written.
    (customize-save-variable 'kill-ring-max 127)
    (disable-command 'forward-word)
    (romacs-should (equal (romacs-saved-settings-test--text init) romacs-saved-settings-test--init))
    (romacs-should (eq (get 'forward-word 'disabled) t))
    (put 'forward-word 'disabled nil)))

(romacs-deftest romacs-saved-settings-a-quick-session-writes-nothing ()
  (romacs-saved-settings-test--with (dir init default)
    (let ((user-init-file nil) (process-environment (cons (concat "HOME=" dir) process-environment)))
      (romacs-should-not (custom-file t))
      (disable-command 'forward-word)
      (romacs-should (eq (get 'forward-word 'disabled) t))
      (romacs-should-not (file-exists-p default))
      (romacs-should-not (file-exists-p (expand-file-name ".emacs" dir)))
      (put 'forward-word 'disabled nil))))

(romacs-deftest romacs-saved-settings-disable-command-writes-the-literal-name ()
  ;; GNU substitutes the name it opens: this one would become the init.
  (romacs-saved-settings-test--with (dir init default)
    (let* ((process-environment (cons (concat "ROMACS_CUSTOM_TEST=" init) process-environment))
           (custom-file (concat dir "$ROMACS_CUSTOM_TEST")))
      (disable-command 'forward-word)
      (romacs-should (equal (romacs-saved-settings-test--text init) romacs-saved-settings-test--init))
      (romacs-should (string-search "'forward-word 'disabled t" (romacs-saved-settings-test--text custom-file)))
      (put 'forward-word 'disabled nil))))

(romacs-deftest romacs-saved-settings-commands-keep-one-line-through-custom-saves ()
  (romacs-saved-settings-test--with (dir init default)
    (disable-command 'forward-word)
    (enable-command 'forward-word)
    (customize-save-variable 'kill-ring-max 128)
    (let ((text (romacs-saved-settings-test--text default)))
      (romacs-should (string-search "(put 'forward-word 'disabled nil)" text))
      (romacs-should-not (string-search "'forward-word 'disabled t" text))
      (romacs-should (string-search "kill-ring-max 128" text)))))

(romacs-deftest romacs-saved-settings-an-invalid-command-is-refused-without-a-file ()
  (romacs-saved-settings-test--with (dir init default)
    (let ((user-init-file nil))
      (romacs-should-error (disable-command 'romacs-saved-settings-no-such))
      (romacs-should-not (get 'romacs-saved-settings-no-such 'disabled)))))

(romacs-deftest romacs-saved-settings-start-loads-the-default-and-keeps-a-choice ()
  (romacs-saved-settings-test--with (dir init default)
    (write-region "(setq romacs-saved-settings-test--loaded 'default)\n" nil default nil 'silent)
    (defvar romacs-saved-settings-test--loaded nil)
    (romacs-saved-settings-start)
    (romacs-should (equal custom-file default))
    (romacs-should (eq romacs-saved-settings-test--loaded 'default))
    (let ((custom-file (expand-file-name "mine.el" dir)))
      (write-region "(setq romacs-saved-settings-test--loaded 'mine)\n" nil custom-file nil 'silent)
      (romacs-saved-settings-start)
      (romacs-should (eq romacs-saved-settings-test--loaded 'mine)))))

(romacs-deftest romacs-saved-settings-a-broken-file-does-not-stop-the-session ()
  (romacs-saved-settings-test--with (dir init default)
    (write-region "(error \"broken settings\")\n" nil default nil 'silent)
    (romacs-saved-settings-start)
    (romacs-should (equal custom-file default))))
;;; romacs-saved-settings-test.el ends here
