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
                 '(("@@ -1 +1 @@\n" hunk) ("-" removed-sign) ("old\n" removed) ("+" added-sign) ("new\n" added)))))

(ert-deftest romacs-lexical-signs-are-legible-on-their-band ()
  (require 'diff-mode)
  (cl-letf (((symbol-function 'face-background)
             (lambda (face &rest _) (pcase face ('diff-removed "#3a1010") ('diff-added "#103a10") (_ "#121212"))))
            ((symbol-function 'face-foreground)
             (lambda (face &rest _) (pcase face ('diff-indicator-removed "#5a2020") ('diff-indicator-added "#206020") (_ "#bebebe")))))
    (romacs-lexical--derive)
    (dolist (pair '((romacs-lexical-removed-sign . "#3a1010") (romacs-lexical-added-sign . "#103a10")))
      (should (>= (romacs-color-contrast (plist-get (cdr (car (get (car pair) 'face-defface-spec))) :foreground) (cdr pair))
                  4.4))))
  (romacs-lexical--derive))

(ert-deftest romacs-lexical-hunk-is-a-band-on-every-theme ()
  (require 'diff-mode)
  ;; A theme whose diff header has no background of its own still bands
  ;; the hunk: the text's background toward its colour.
  (cl-letf (((symbol-function 'face-background)
             (lambda (face &rest _) (pcase face ('default "#eff1f5") ('diff-header nil))))
            ((symbol-function 'face-foreground) (lambda (&rest _) "#4c4f69")))
    (romacs-lexical--derive)
    (let ((band (plist-get (cdr (car (get 'romacs-lexical-hunk 'face-defface-spec))) :background)))
      (should band)
      (should (>= (romacs-color-contrast band "#eff1f5") 1.15))))
  ;; On black, its own near-black left aside: a band that shows.
  (cl-letf (((symbol-function 'face-background)
             (lambda (face &rest _) (pcase face ('default "#000000") ('diff-header "#050505"))))
            ((symbol-function 'face-foreground) (lambda (&rest _) "#e0e0e0")))
    (romacs-lexical--derive)
    (should (>= (romacs-color-contrast (plist-get (cdr (car (get 'romacs-lexical-hunk 'face-defface-spec))) :background) "#000000") 1.15)))
  ;; Its own, where it stands apart.
  (cl-letf (((symbol-function 'face-background)
             (lambda (face &rest _) (pcase face ('default "#121212") ('diff-header "#2a2a2a"))))
            ((symbol-function 'face-foreground) (lambda (&rest _) "#bebebe")))
    (romacs-lexical--derive)
    (should (equal (plist-get (cdr (car (get 'romacs-lexical-hunk 'face-defface-spec))) :background) "#2a2a2a")))
  (romacs-lexical--derive))

(ert-deftest romacs-lexical-colours-only-a-known-language ()
  (with-temp-buffer
    (insert "\"s\"")
    (should-not (romacs-lexical-colour 'no-such-language (point-min) (point-max)))
    (should-not (get-text-property 1 'face))
    (should (romacs-lexical-colour 'nix (point-min) (point-max)))
    (should (memq 'font-lock-string-face (ensure-list (get-text-property 1 'face))))))

(provide 'romacs-lexical-tests)
;;; romacs-lexical-tests.el ends here
