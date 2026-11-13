;;; romacs-commands-tests.el --- M-x over the kept command set -*- lexical-binding: t; -*-
(require 'ert)
(require 'romacs-commands)

(defun romacs-commands-tests--keys (strings)
  "M-x's candidates for each of STRINGS, as its table lists them on
successive keys, and as simple.el's table over the obarray lists them."
  (let* ((buffer (current-buffer))
         (pred (romacs-commands--predicate buffer))
         (table (romacs-commands--table pred buffer)))
    (mapcar (lambda (string)
              (list (all-completions string table pred)
                    ;; Any other predicate gets simple.el's table.
                    (all-completions string table (lambda (sym) (funcall pred sym)))))
            strings)))

(defun romacs-commands-tests--offered-p (name)
  "Whether M-x offers command NAME now, kept answers and all."
  (let* ((buffer (current-buffer))
         (pred (romacs-commands--predicate buffer)))
    (and (member name (all-completions "mx-test-" (romacs-commands--table pred buffer) pred))
         t)))

(defmacro romacs-commands-tests--with (&rest body)
  "Run BODY with M-x's default predicate and its definitions undone."
  `(let ((read-extended-command-predicate #'command-completion-default-include-p)
         (romacs-commands--kept nil))
     (unwind-protect (progn ,@body)
       (mapatoms (lambda (sym)
                   (when (string-prefix-p "mx-test-" (symbol-name sym))
                     (fmakunbound sym)
                     (setplist sym nil)))))))

(ert-deftest romacs-commands-lists-what-the-obarray-lists ()
  (romacs-commands-tests--with
   (with-temp-buffer
     (emacs-lisp-mode)
     (dolist (both (romacs-commands-tests--keys '("" "f" "fi" "find-f" "emacs-lisp-" "zzz")))
       (should (equal (car both) (cadr both)))))))

(ert-deftest romacs-commands-follow-each-definition-between-keys ()
  (romacs-commands-tests--with
   (should-not (romacs-commands-tests--offered-p "mx-test-a"))
   (defun mx-test-a () (interactive))
   (should (romacs-commands-tests--offered-p "mx-test-a"))
   (defalias 'mx-test-b 'mx-test-a)
   (should (romacs-commands-tests--offered-p "mx-test-b"))
   (fset 'mx-test-a (lambda () 1))
   (should-not (romacs-commands-tests--offered-p "mx-test-a"))
   (should-not (romacs-commands-tests--offered-p "mx-test-b"))
   (autoload 'mx-test-c "mx-test-none" nil t)
   (should (romacs-commands-tests--offered-p "mx-test-c"))
   (fmakunbound 'mx-test-c)
   (should-not (romacs-commands-tests--offered-p "mx-test-c"))
   (defun mx-test-d () 1)
   (advice-add 'mx-test-d :around
               (lambda (f &rest args) (interactive) (apply f args))
               '((name . mx-test-advice)))
   (should (romacs-commands-tests--offered-p "mx-test-d"))
   (defun mx-test-e () (interactive))
   (should (romacs-commands-tests--offered-p "mx-test-e"))
   (unintern "mx-test-e" obarray)
   (should-not (romacs-commands-tests--offered-p "mx-test-e"))))

(ert-deftest romacs-commands-follow-properties-and-modes ()
  (romacs-commands-tests--with
   (defun mx-test-a () (interactive))
   (should (romacs-commands-tests--offered-p "mx-test-a"))
   (put 'mx-test-a 'completion-predicate #'ignore)
   (should-not (romacs-commands-tests--offered-p "mx-test-a"))
   (defun mx-test-org () (interactive nil org-mode))
   (with-temp-buffer
     (should-not (romacs-commands-tests--offered-p "mx-test-org"))
     (setq major-mode 'org-mode)
     (should (romacs-commands-tests--offered-p "mx-test-org")))))

(define-minor-mode romacs-commands-tests--minor-mode "A local minor mode for M-x's tests.")

(ert-deftest romacs-commands-follow-a-minor-mode-turned-off ()
  (romacs-commands-tests--with
   (defun mx-test-in-minor () (interactive nil romacs-commands-tests--minor-mode))
   (with-temp-buffer
     (romacs-commands-tests--minor-mode 1)
     (push 'romacs-commands-tests--other-mode local-minor-modes)
     (should (romacs-commands-tests--offered-p "mx-test-in-minor"))
     ;; Not the list's head: turning it off edits the list in place.
     (romacs-commands-tests--minor-mode -1)
     (should-not (romacs-commands-tests--offered-p "mx-test-in-minor")))))

(ert-deftest romacs-commands-ask-a-command-s-own-predicate-on-each-key ()
  (romacs-commands-tests--with
   (let ((offered t))
     (defun mx-test-a () (interactive))
     (put 'mx-test-a 'completion-predicate (lambda (_ _) offered))
     (should (romacs-commands-tests--offered-p "mx-test-a"))
     (setq offered nil)
     (should-not (romacs-commands-tests--offered-p "mx-test-a")))))

(ert-deftest romacs-commands-ask-any-other-predicate-on-each-key ()
  (romacs-commands-tests--with
   (let* ((offered t)
          (read-extended-command-predicate (lambda (_ _) offered)))
     (defun mx-test-a () (interactive))
     (should (romacs-commands-tests--offered-p "mx-test-a"))
     (setq offered nil)
     (should-not (romacs-commands-tests--offered-p "mx-test-a")))))

(ert-deftest romacs-commands-offer-an-obsolete-command-typed-in-full ()
  (romacs-commands-tests--with
   (defun mx-test-old () (interactive))
   (put 'mx-test-old 'byte-obsolete-info (list nil nil "1.1"))
   (let* ((buffer (current-buffer))
          (pred (romacs-commands--predicate buffer))
          (table (romacs-commands--table pred buffer)))
     (should-not (all-completions "mx-test-ol" table pred))
     (should (equal (all-completions "mx-test-old" table pred)
                    '("mx-test-old"))))))

;;; romacs-commands-tests.el ends here
