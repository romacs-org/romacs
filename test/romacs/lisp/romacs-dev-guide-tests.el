;;; romacs-dev-guide-tests.el --- The guide answers from the machine  -*- lexical-binding: t -*-
;; SPDX-License-Identifier: GPL-3.0-or-later
;; Maintainer: Thanos Apollo <public@thanosapollo.org>

;;; Commentary:
;; The guide over functions these tests define: what it says of one, how it
;; finds them by words, and what it lists when asked nothing.

;;; Code:

(require 'ert)
(require 'romacs-dev-guide)

(defun romacs-dev-guide-tests--quokka (first &optional second)
  "Feed the quokka FIRST, a wombat fact.
SECOND is for later."
  (list first second))

(defun romacs-dev-guide-tests--helper (thing)
  "Return THING for a quokka and a numbat alike."
  thing)

(ert-deftest romacs-dev-guide-tests-describe-function ()
  "A function's name gives its arguments, kind and docstring."
  (should (equal (romacs-dev-guide "romacs-dev-guide-tests--quokka")
                 (concat "romacs-dev-guide-tests--quokka (first &optional second): function, test/romacs/lisp/romacs-dev-guide-tests.el\n"
                         "  Feed the quokka FIRST, a wombat fact.\n"
                         "  SECOND is for later."))))

(ert-deftest romacs-dev-guide-tests-describe-primitive ()
  "A primitive is said to be one, with the Rust file that defines it, and
its effect class where its definition declares one."
  (let ((lines (split-string (romacs-dev-guide "subr-arity") "\n")))
    (should (string-match-p "\\`subr-arity (subr): primitive.*crates/romacs-vm/src/prims/subr\\.rs\\'" (car lines)))
    (when (fboundp 'romacs--subr-effect)
      (should (string-match-p "primitive, pure" (car lines)))
      (should (equal (nth 1 lines) "  pure: depends on its arguments alone")))))

(ert-deftest romacs-dev-guide-tests-callers-are-told ()
  "What the owner of a better path put on a function is part of what it is."
  (put 'romacs-dev-guide-tests--helper 'romacs-lint-hint "the machine serves this another way")
  (unwind-protect
      (progn
        (should (string-match-p "^  callers are told: the machine serves this another way$"
                                (romacs-dev-guide "romacs-dev-guide-tests--helper")))
        (should (string-match-p "^  romacs-dev-guide-tests--helper: the machine serves this another way$"
                                (romacs-dev-guide ""))))
    (put 'romacs-dev-guide-tests--helper 'romacs-lint-hint nil)))

(ert-deftest romacs-dev-guide-tests-search ()
  "Words find the functions that have them all, in a name or a docstring;
a function with more of them in its name comes first."
  (let ((mine '(romacs-dev-guide-tests--helper romacs-dev-guide-tests--quokka car)))
    (should (equal (romacs-dev-guide-search "quokka wombat" nil mine)
                   (concat "romacs-dev-guide-tests--quokka (function): Feed the quokka FIRST, a wombat fact.\n"
                           "1 function; ask for one by name for its arguments and docstring")))
    (should (equal (split-string (romacs-dev-guide-search "Quokka" nil mine) "\n")
                   '("romacs-dev-guide-tests--quokka (function): Feed the quokka FIRST, a wombat fact."
                     "romacs-dev-guide-tests--helper (function): Return THING for a quokka and a numbat alike."
                     "2 functions; ask for one by name for its arguments and docstring")))
    (should (equal (romacs-dev-guide-search "quokka" 1 mine)
                   (concat "romacs-dev-guide-tests--quokka (function): Feed the quokka FIRST, a wombat fact.\n"
                           "... 1 more: add a word, or ask for one by name")))
    (should (equal (romacs-dev-guide-search "quokka zzyzzyva" nil mine)
                   "nothing here has quokka and zzyzzyva in its name or docstring"))))

(ert-deftest romacs-dev-guide-tests-search-in-steps ()
  "A search taken a step at a time finds what one taken whole does."
  (let* ((mine '(romacs-dev-guide-tests--helper romacs-dev-guide-tests--quokka car cdr cons))
         (search (romacs-dev-guide-search-start "quokka" mine))
         (steps 0))
    (should-not (romacs-dev-guide-search-done-p search))
    (while (not (romacs-dev-guide-search-done-p search))
      (setq search (romacs-dev-guide-search-step search 0) steps (1+ steps)))
    (should (> steps 1))
    (should (equal (romacs-dev-guide-search-result search) (romacs-dev-guide-search "quokka" nil mine)))))

(ert-deftest romacs-dev-guide-tests-obsolete ()
  "An obsolete function says so, and names a replacement only when it has one."
  (defalias 'romacs-dev-guide-tests--old #'identity)
  (unwind-protect
      (progn
        (make-obsolete 'romacs-dev-guide-tests--old nil "0.1")
        (should (string-match-p "^  obsolete since 0\\.1$" (romacs-dev-guide "romacs-dev-guide-tests--old")))
        (make-obsolete 'romacs-dev-guide-tests--old 'identity "0.2")
        (should (string-match-p "^  obsolete since 0\\.2: use `identity'$" (romacs-dev-guide "romacs-dev-guide-tests--old"))))
    (put 'romacs-dev-guide-tests--old 'byte-obsolete-info nil)
    (fmakunbound 'romacs-dev-guide-tests--old)))

(ert-deftest romacs-dev-guide-tests-what-searches ()
  "Only a search of docstrings is slow enough to leave the main VM."
  (should (romacs-dev-guide-search-p "quokka wombat"))
  (should (romacs-dev-guide-search-p "no-such-function-here"))
  (should-not (romacs-dev-guide-search-p "car"))
  (should-not (romacs-dev-guide-search-p "  ")))

(provide 'romacs-dev-guide-tests)
;;; romacs-dev-guide-tests.el ends here
