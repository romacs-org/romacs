;;; definition-files-test.el --- where help and xref find definitions  -*- lexical-binding: t -*-
;; SPDX-License-Identifier: GPL-3.0-or-later

;; ERT tests tests/definition_files.rs runs in Romacs's batch session and in
;; pinned GNU's: what Lisp defines, help-fns.el's `find-lisp-object-file-name'
;; finds in the same library in both, and a primitive or a variable the
;; implementation declares has a definition there too (GNU's C, Romacs's
;; Rust) rather than a void `build-files'.

(require 'ert)
(require 'help-fns)

(defun definition-files-test--library (object type)
  "The library defining OBJECT, without its directory or extensions."
  (file-name-base (file-name-sans-extension (find-lisp-object-file-name object type))))

(ert-deftest definition-files-lisp-functions-go-where-gnu-goes ()
  (should (equal (mapcar (lambda (f) (definition-files-test--library f (symbol-function f)))
                         '(when string-trim describe-function find-file xref-find-definitions push))
                 '("subr" "subr" "help-fns" "files" "xref" "subr"))))

(ert-deftest definition-files-lisp-variables-go-where-gnu-goes ()
  (require 'find-func)
  (require 'xref)
  (should (equal (mapcar (lambda (v) (definition-files-test--library v 'defvar))
                         '(help-window-select find-function-regexp xref-prompt-for-identifier kill-ring-max fill-nobreak-predicate))
                 '("help" "find-func" "xref" "simple" "fill"))))

(ert-deftest definition-files-built-ins-have-a-definition ()
  (should (boundp 'build-files))
  (should (find-lisp-object-file-name 'car (symbol-function 'car)))
  (should (find-lisp-object-file-name 'read-string (symbol-function 'read-string)))
  (should (find-lisp-object-file-name 'fill-column 'defvar)))
