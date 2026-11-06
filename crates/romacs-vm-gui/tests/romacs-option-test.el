;;; romacs-option-test.el --- User options as data -*- lexical-binding: t -*-
;; SPDX-License-Identifier: GPL-3.0-or-later
;; Maintainer: Thanos Apollo <public@thanosapollo.org>

;; rlisp/romacs-option.el in a `--loadup' session
;; (gnu_completion.rs `romacs_option_lisp_tests_pass').

(require 'romacs-option)

(defconst romacs-option-test--verdicts
  '((kill-ring-max 60 t) (kill-ring-max "x" nil) (kill-ring-max 1.5 nil) (kill-ring-max nil nil)
    (ses-initial-column-width 7 t) (ses-initial-column-width 2.5 t) (ses-initial-column-width -1 nil)
    (message-required-mail-headers (From Subject) t) (message-required-mail-headers "From" nil)
    (backup-by-copying t t) (backup-by-copying 7 nil)
    (save-interprogram-paste-before-kill t t) (save-interprogram-paste-before-kill 100 t)
    (save-interprogram-paste-before-kill "x" t)
    (after-save-hook (ignore) t) (after-save-hook "x" nil)
    (version-control t t) (version-control never t) (version-control sometimes t)
    (sql-postgres-login-params (user server) t) (sql-postgres-login-params 7 nil)
    (auto-save-file-name-transforms (("a" "b" t)) t) (auto-save-file-name-transforms 7 nil)
    (c-default-style "gnu" t) (c-default-style ((c-mode . "gnu")) t) (c-default-style 7 nil))
  "What pinned GNU's `:match' says of each (OPTION VALUE): own matchers, lazy
types, choices and hooks.")

(defvar romacs-option-test--counter 0)

(defmacro romacs-option-test--with-option (name type &rest body)
  "Run BODY with NAME, a fresh option of TYPE and value 1, set back after."
  (declare (indent 2))
  `(let ((,name (intern (format "romacs-option-test-%d" (cl-incf romacs-option-test--counter)))))
     (custom-declare-variable ,name 1 "A test option.\nMore." :type ,type :group 'romacs-option-test)
     ,@body))

(romacs-deftest romacs-option-check-gives-gnus-verdicts ()
  (dolist (library '(ses message cc-vars files sql)) (require library))
  (dolist (case romacs-option-test--verdicts)
    (romacs-should (eq (nth 2 case) (romacs-option-check (car case) (nth 1 case))))))

(romacs-deftest romacs-option-with-no-type-takes-anything ()
  (let ((option (intern (format "romacs-option-test-%d" (cl-incf romacs-option-test--counter)))))
    (custom-declare-variable option 1 "No type." :group 'romacs-option-test)
    (romacs-should (romacs-option-check option "anything"))))

(romacs-deftest romacs-option-set-does-what-customize-set-variable-does ()
  (romacs-option-test--with-option ours 'integer
    (romacs-option-test--with-option theirs 'integer
      (let ((outcome (romacs-option-set ours 5)))
        (customize-set-variable theirs 5)
        (romacs-should (equal (plist-get outcome :previous) "1"))
        (romacs-should (equal (plist-get outcome :value) "5"))
        (romacs-should (eq (plist-get outcome :state) 'set))
        (romacs-should (= (symbol-value ours) 5))
        (dolist (property '(customized-value theme-value saved-value))
          (romacs-should (equal (get ours property) (get theirs property))))))))

(romacs-deftest romacs-option-set-runs-the-setter ()
  (let ((option (intern (format "romacs-option-test-%d" (cl-incf romacs-option-test--counter))))
        (calls nil))
    (custom-declare-variable option 1 "A test option." :type 'integer :group 'romacs-option-test
                             :set (lambda (symbol value) (push value calls) (set-default symbol (* 2 value))))
    (setq calls nil)
    (let ((outcome (romacs-option-set option 4)))
      (romacs-should (equal calls '(4)))
      (romacs-should (equal (plist-get outcome :value) "8")))))

(romacs-deftest romacs-option-a-refused-value-changes-nothing ()
  (romacs-option-test--with-option option 'integer
    (let ((before (copy-sequence (symbol-plist option))))
      (dolist (write '(romacs-option-set romacs-option-save))
        (romacs-should-error (funcall write option "text") :type 'user-error))
      (romacs-should (= (symbol-value option) 1))
      (romacs-should (equal (symbol-plist option) before)))))

(romacs-deftest romacs-option-save-saves-beside-the-init-or-refuses ()
  (romacs-option-test--with-option option 'integer
    (let* ((dir (file-name-as-directory (make-temp-file "romacs-option" t)))
           (user-emacs-directory dir) (custom-file nil)
           (user-init-file (expand-file-name "init.el" dir)))
      (unwind-protect
          (progn
            (write-region "; mine\n" nil user-init-file nil 'silent)
            (romacs-should (eq (plist-get (romacs-option-save option 7) :state) 'saved))
            (romacs-should (string-search (format "(%s 7)" option)
                                          (with-temp-buffer (insert-file-contents (expand-file-name "custom.el" dir))
                                                            (buffer-string))))
            (romacs-should (equal (with-temp-buffer (insert-file-contents user-init-file) (buffer-string)) "; mine\n"))
            (let ((user-init-file nil))
              (romacs-should-error (romacs-option-save option 9) :type 'user-error))
            (romacs-should (= (symbol-value option) 7)))
        (delete-directory dir t)))))

(romacs-deftest romacs-option-describes-what-custom-keeps ()
  (romacs-option-test--with-option option '(choice integer (const nil))
    (let ((record (romacs-option-describe option)))
      (romacs-should (eq (plist-get record :kind) 'variable))
      (romacs-should (equal (plist-get record :type) "(choice integer (const nil))"))
      (romacs-should (equal (plist-get record :value) "1"))
      (romacs-should (eq (plist-get record :state) 'standard))
      (romacs-should (equal (plist-get record :groups) '(romacs-option-test))))))

(romacs-deftest romacs-option-aliases-read-the-base-option ()
  (romacs-option-test--with-option option 'integer
    (let ((alias (intern (format "%s-alias" option))))
      (defvaralias alias option)
      (romacs-should (eq (plist-get (romacs-option-describe alias) :name) option))
      (romacs-should (eq (plist-get (romacs-option-set alias 3) :option) option))
      (romacs-should (= (symbol-value option) 3)))))

(romacs-deftest romacs-option-faces-are-read-not-written ()
  (let ((record (romacs-option-describe 'Buffer-menu-buffer)))
    (romacs-should (eq (plist-get record :kind) 'face))
    (romacs-should (eq (plist-get record :name) 'buffer-menu-buffer)))
  (romacs-should-error (romacs-option-set 'bold '((t :weight bold))) :type 'user-error)
  (romacs-should (assq 'bold (romacs-option-search "bold" :kind 'face))))

(romacs-deftest romacs-option-an-autoloaded-option-loads-to-be-set ()
  (let* ((dir (file-name-as-directory (make-temp-file "romacs-option" t)))
         (load-path (cons dir load-path))
         (library (format "romacs-option-test-lib-%d" (cl-incf romacs-option-test--counter)))
         (option (intern (concat library "-value"))))
    (unwind-protect
        (progn
          (write-region (format "(defcustom %s 1 \"Late.\" :type 'integer :group 'romacs-option-test)\n(provide '%s)\n"
                                option library)
                        nil (expand-file-name (concat library ".el") dir) nil 'silent)
          (custom-autoload option library)
          (romacs-should (null (plist-get (romacs-option-describe option) :loaded)))
          (romacs-should-not (assq option (romacs-option-search library)))
          (romacs-should-error (romacs-option-set option "text") :type 'user-error)
          (romacs-should (eq (plist-get (romacs-option-set option 2) :state) 'set))
          (romacs-should (assq option (romacs-option-search library))))
      (delete-directory dir t))))

(romacs-deftest romacs-option-an-unknown-name-is-refused ()
  (romacs-should-error (romacs-option-describe 'romacs-option-no-such) :type 'user-error)
  (romacs-should-error (romacs-option-set 'romacs-option-no-such 1) :type 'user-error))

(romacs-deftest romacs-option-search-follows-what-is-declared ()
  (romacs-option-test--with-option option 'integer
    (let ((name (symbol-name option)) (late (intern (format "%s-late" option))))
      (romacs-should (assq option (romacs-option-search name)))
      ;; A membership added with nothing declared or loaded.
      (romacs-should-not (romacs-option-search name :group 'romacs-option-other-group))
      (custom-add-to-group 'romacs-option-other-group option 'custom-variable)
      (romacs-should (assq option (romacs-option-search name :group 'romacs-option-other-group)))
      (romacs-should (memq 'romacs-option-other-group (plist-get (romacs-option-describe option) :groups)))
      (custom-declare-variable late 1 "Later." :type 'integer :group 'romacs-option-test)
      (romacs-should (assq late (romacs-option-search name))))))

(romacs-deftest romacs-option-search-words-docs-and-limit ()
  (romacs-option-test--with-option option 'integer
    (let ((rows (romacs-option-search (format "%s test" option))))
      (romacs-should (equal (cdr (assq option rows)) '(variable "A test option."))))
    (romacs-should-not (romacs-option-search (format "%s scaffolding" option)))
    (romacs-should (assq option (romacs-option-search (format "%s more" option) :docs t)))
    (romacs-should (= 1 (length (romacs-option-search "e" :limit 1))))))

(romacs-deftest romacs-option-search-finds-aliases-and-ignores-case ()
  (romacs-option-test--with-option option 'integer
    (let ((alias (intern (format "%s-alias" option))) (upper (intern (format "%s-UPPER" option))))
      (defvaralias alias option)
      (custom-declare-variable upper 1 "Mixed case." :type 'integer :group 'romacs-option-test)
      (romacs-should (assq alias (romacs-option-search (symbol-name alias))))
      (romacs-should (assq upper (romacs-option-search (symbol-name upper)))))))

(romacs-deftest romacs-option-a-group-search-keeps-kinds-and-order ()
  (romacs-option-test--with-option option 'integer
    (custom-declare-face 'romacs-option-test-face '((t)) "A face." :group 'romacs-option-group-test)
    (custom-add-to-group 'romacs-option-group-test option 'custom-variable)
    (romacs-should (equal (mapcar #'car (romacs-option-search "" :group 'romacs-option-group-test))
                          (list option 'romacs-option-test-face)))
    (romacs-should (equal (mapcar #'car (romacs-option-search "" :group 'romacs-option-group-test :kind 'face))
                          '(romacs-option-test-face)))))

(romacs-deftest romacs-option-printing-comes-back-or-says-nothing ()
  (romacs-option-test--with-option option 'sexp
    (romacs-should (equal (read (plist-get (romacs-option-set option '(a "b\nc" 1.5)) :value))
                          '(a "b\nc" 1.5)))
    ;; Circular, read back through `print-circle'.
    (let ((circular (list 'a)))
      (setcdr circular circular)
      (let ((printed (plist-get (romacs-option-set option circular) :value)))
        (romacs-should (eq (car (read printed)) 'a))))
    ;; An object with no printed form, and text too long to carry: never a
    ;; fragment, and the previous value was printed before the change.
    (set-default option 1)
    (let ((outcome (romacs-option-set option (current-buffer))))
      (romacs-should (equal (plist-get outcome :previous) "1"))
      (romacs-should-not (plist-get outcome :value)))
    (let ((outcome (romacs-option-set option (make-string 20000 ?x))))
      (romacs-should-not (plist-get outcome :previous))
      (romacs-should-not (plist-get outcome :value)))
    (romacs-should-not (plist-get (romacs-option-describe option) :value))
    (set-default option 1)))

(romacs-deftest romacs-option-standard-is-the-value-not-its-form ()
  (let ((option (intern (format "romacs-option-test-%d" (cl-incf romacs-option-test--counter)))))
    (custom-declare-variable option ''(a b) "Quoted." :type '(repeat symbol) :group 'romacs-option-test)
    (romacs-should (equal (plist-get (romacs-option-describe option) :standard) "(a b)"))))

(romacs-deftest romacs-option-save-says-when-it-lost-its-file ()
  ;; Saving `custom-file' itself as nil, with the default linked to the init.
  (let* ((dir (file-name-as-directory (make-temp-file "romacs-option" t)))
         (user-emacs-directory dir)
         (user-init-file (expand-file-name "init.el" dir))
         (custom-file (expand-file-name "mine.el" dir)))
    (unwind-protect
        (progn
          (write-region "; mine\n" nil user-init-file nil 'silent)
          (make-symbolic-link user-init-file (expand-file-name "custom.el" dir))
          (romacs-should-error (romacs-option-save 'custom-file nil) :type 'user-error)
          (romacs-should (equal (with-temp-buffer (insert-file-contents user-init-file) (buffer-string))
                                "; mine\n")))
      (delete-directory dir t))))

(romacs-deftest romacs-option-search-leaves-out-the-obsolete ()
  (romacs-option-test--with-option option 'integer
    (make-obsolete-variable option nil "1.0")
    (romacs-should-not (assq option (romacs-option-search (symbol-name option))))))
;;; romacs-option-test.el ends here
