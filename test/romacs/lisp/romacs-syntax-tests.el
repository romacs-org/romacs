;;; romacs-syntax-tests.el --- syntax-ppss over the syntax summaries -*- lexical-binding: t -*-

;; SPDX-License-Identifier: GPL-3.0-or-later
;; Maintainer: Thanos Apollo <public@thanosapollo.org>

;;; Code:

(require 'ert)

(ert-deftest romacs-syntax-ppss-long-parses-are-served ()
  "Far from its cached states, syntax.el's `syntax-ppss' parses through the
summaries, before and after edits that flip the state below them, and
agrees with the scanner's parse from `point-min' in every element GNU
documents as reliable."
  (with-temp-buffer
    (dotimes (i 1500)
      (insert (format "int f%d(int a) { /* c ( */ return a + \"s(\" + %d; } // d {\n" i i)))
    (c-mode)
    (let ((served (plist-get (romacs-syntax-summaries-stats) :served)))
      (dolist (edit '(nil 100 5000 40000))
        (when edit
          (goto-char edit)
          (insert "\"x"))
        (dolist (pos (list (point-max) 30000 60000 (/ (point-max) 2)))
          (let ((ppss (syntax-ppss pos))
                (plain (let ((romacs-inhibit-syntax-summaries t))
                         (save-excursion (parse-partial-sexp (point-min) pos)))))
            (should (= (point) pos))
            ;; Elements 2 and 6 depend on syntax.el's cache, as GNU documents.
            (dolist (i '(0 1 3 4 5 7 8 9 10))
              (should (equal (nth i ppss) (nth i plain)))))))
      (should (> (plist-get (romacs-syntax-summaries-stats) :served) served)))))

;;; romacs-syntax-tests.el ends here
