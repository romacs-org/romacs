;;; romacs-help-completion-tests.el --- Help's completion loads no file -*- lexical-binding: t; -*-
(require 'ert)
(require 'cl-lib)
(require 'help-fns)
(require 'romacs-compile)

(defconst romacs-help-completion-tests--function-p
  (lambda (f) (or (fboundp f) (get f 'function-documentation)))
  "describe-function's predicate.")

(defconst romacs-help-completion-tests--variable-p
  (lambda (v) (or (get v 'variable-documentation) (and (not (keywordp v)) (boundp v))))
  "describe-variable's predicate, without its buffer.")

(defmacro romacs-help-completion-tests--in-prompt (&rest body)
  "Run BODY in a buffer help's table takes for a minibuffer."
  `(with-temp-buffer
     (cl-letf (((symbol-function 'minibufferp) (lambda (&rest _) t)))
       ,@body)))

(ert-deftest romacs-help-completion-follows-help-fns ()
  "Loading help-fns.el leaves Romacs's table in place."
  (should (string-match-p "/romacs-help-completion\\.el\\'"
                          (symbol-file 'help--symbol-completion-table 'defun))))

(ert-deftest romacs-help-completion-requires-its-index-as-compiled ()
  "The compile's walk runs the file's `require' of its index as the tree's
compile does, no file loading and none visited: it finds the index
beside the file."
  (let* ((file (symbol-file 'help--symbol-completion-table 'defun))
         (form (with-temp-buffer
                 (insert-file-contents file)
                 (search-forward "(require 'romacs-help-index")
                 (goto-char (match-beginning 0))
                 (read (current-buffer))))
         (romacs-compile--emitted nil))
    (with-temp-buffer
      (let ((load-file-name nil)
            (byte-compile-current-file file))
        (romacs-compile--file-form form)))
    (should (equal romacs-compile--emitted (list (cons form byte-compile-bound-variables))))))

(ert-deftest romacs-help-completion-typing-loads-no-file ()
  "Completing a prefix `definition-prefixes' names loads no file, where
GNU's table loaded the library of the prefix."
  (should-not (featurep '5x5))
  (should (gethash "5x5-" definition-prefixes))
  (let ((history (length load-history)))
    (dolist (input '("5" "5x5-" "5x5-new"))
      (all-completions input #'help--symbol-completion-table romacs-help-completion-tests--function-p)
      (try-completion input #'help--symbol-completion-table romacs-help-completion-tests--function-p)
      (test-completion input #'help--symbol-completion-table romacs-help-completion-tests--function-p))
    (should (= (length load-history) history))
    (should-not (featurep '5x5))))

(ert-deftest romacs-help-completion-lists-once-a-prompt ()
  "A prompt runs its predicate over the obarray once, not on every key,
and lists again once a function is defined."
  (romacs-help-completion-tests--in-prompt
    (let* ((calls 0)
           (pred (lambda (f) (setq calls (1+ calls)) (fboundp f)))
           (name (concat "romacs-help-completion-tests--" "late")))
      (all-completions "car" #'help--symbol-completion-table pred)
      (should (> calls 1000))
      (setq calls 0)
      (should (member "car" (all-completions "ca" #'help--symbol-completion-table pred)))
      (should (= calls 0))
      (defalias (intern name) #'ignore)
      (unwind-protect
          (should (member name (all-completions "romacs-help-completion-tests--"
                                                #'help--symbol-completion-table pred)))
        (fmakunbound (intern name))))))

(ert-deftest romacs-help-completion-takes-an-exact-name-as-it-is-now ()
  "A prompt's kept listing still answers an exact name as the predicate
does now: one bound since is accepted, one unbound since is not."
  (romacs-help-completion-tests--in-prompt
    (let ((late (concat "romacs-help-completion-tests--" "late-var"))
          (gone (concat "romacs-help-completion-tests--" "gone-var")))
      (set (intern gone) t)
      (all-completions "" #'help--symbol-completion-table romacs-help-completion-tests--variable-p)
      (should-not (test-completion late #'help--symbol-completion-table romacs-help-completion-tests--variable-p))
      (set (intern late) t)
      (makunbound (intern gone))
      (unwind-protect
          (progn
            (should (test-completion late #'help--symbol-completion-table romacs-help-completion-tests--variable-p))
            (should-not (test-completion gone #'help--symbol-completion-table romacs-help-completion-tests--variable-p)))
        (makunbound (intern late))))))

(ert-deftest romacs-help-completion-accepts-what-it-lists ()
  "An exact name the prompt lists is accepted: the symbol nil, and a name
differing in case alone under `completion-ignore-case'."
  (romacs-help-completion-tests--in-prompt
    (should (member "nil" (all-completions "ni" #'help--symbol-completion-table
                                           romacs-help-completion-tests--variable-p)))
    (should (test-completion "nil" #'help--symbol-completion-table romacs-help-completion-tests--variable-p))
    (let ((completion-ignore-case t))
      (should (test-completion "CAR" #'help--symbol-completion-table romacs-help-completion-tests--function-p)))
    (let ((completion-ignore-case nil))
      (should-not (test-completion "CAR" #'help--symbol-completion-table romacs-help-completion-tests--function-p)))))

;;; romacs-help-completion-tests.el ends here
