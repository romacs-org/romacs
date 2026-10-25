;;; completion-test.el --- Native completion contracts -*- lexical-binding: t -*-
;; SPDX-License-Identifier: GPL-3.0-or-later

(romacs-deftest completion-prefix-and-exact-results ()
  (romacs-should (equal (try-completion "al" '("alpha" "alpine" "beta")) "alp"))
  (romacs-should (eq (try-completion "alpha" '("alpha" "beta")) t))
  (romacs-should (equal (all-completions "al" '("alpha" "alpine" "beta"))
                        '("alpha" "alpine"))))

(romacs-deftest completion-predicates-survive-collection ()
  (let ((calls nil))
    (romacs-should
     (equal (all-completions "a" '("ab" "ac")
                             (lambda (entry)
                               (garbage-collect)
                               (setq calls (cons entry calls))
                               t))
            '("ab" "ac")))
    (romacs-should (equal calls '("ac" "ab")))))

(provide 'completion-test)
