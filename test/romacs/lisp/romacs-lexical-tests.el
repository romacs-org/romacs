;;; romacs-lexical-tests.el --- Code coloured by its lexical classes -*- lexical-binding: t; -*-
;; SPDX-License-Identifier: GPL-3.0-or-later
(require 'ert)
(require 'romacs-lexical)

(defun romacs-lexical-tests--spans (language text)
  "TEXT's spans lexed as LANGUAGE: each its text and class."
  (with-temp-buffer
    (insert text)
    (mapcar (lambda (span) (list (buffer-substring (car span) (cadr span)) (nth 2 span)))
            (romacs-lexical-spans (romacs-lexical-family language) (point-min) (point-max)))))

(ert-deftest romacs-lexical-lisp-classes ()
  (should (equal (romacs-lexical-tests--spans 'emacs-lisp "(message \"a \\\" b\" ?\\n :key 42 utf8) ; done")
                 '(("message" head) ("\"a \\\" b\"" string) ("?\\n" char) (":key" keyword) ("42" number) ("; done" comment)))))

(ert-deftest romacs-lexical-lisp-forms-are-not-calls ()
  (should (equal (romacs-lexical-tests--spans 'emacs-lisp "(defun f (x) (when-let* ((y x)) (g y)))")
                 ;; A definition's name, its arguments no call; a lexer
                 ;; cannot tell a binding list from a call.
                 '(("defun" form) ("f" name) ("when-let*" form) ("y" head) ("g" head))))
  (should (equal (romacs-lexical-tests--spans 'emacs-lisp "(cl-defun g (a &optional b))")
                 '(("cl-defun" form) ("g" name) ("&optional" keyword)))))

(ert-deftest romacs-lexical-slash-and-hash-comments ()
  (should (equal (romacs-lexical-tests--spans 'zig "x = 0x1f; // c\n/* b * c */ \"s\"")
                 '(("0x1f" number) ("// c" comment) ("/* b * c */" comment) ("\"s\"" string))))
  ;; A slash that starts no comment is code.
  (should (equal (romacs-lexical-tests--spans 'zig "a / b") nil))
  (should (equal (romacs-lexical-tests--spans 'nix "{ x = 1; # c\n y = 'z'; }")
                 '(("1" number) ("# c" comment) ("'z'" string)))))

(ert-deftest romacs-lexical-diff-lines ()
  (should (equal (romacs-lexical-tests--spans 'diff "@@ -1 +1 @@\n-old\n+new\n same\n")
                 ;; Each a band: through its newline.
                 '(("@@ -1 +1 @@\n" hunk) ("-old\n" removed) ("+new\n" added)))))

(ert-deftest romacs-lexical-colours-only-a-known-language ()
  (with-temp-buffer
    (insert "\"s\"")
    (should-not (romacs-lexical-colour 'no-such-language (point-min) (point-max)))
    (should-not (get-text-property 1 'face))
    (should (romacs-lexical-colour 'nix (point-min) (point-max)))
    (should (memq 'font-lock-string-face (ensure-list (get-text-property 1 'face))))))

(provide 'romacs-lexical-tests)
;;; romacs-lexical-tests.el ends here
