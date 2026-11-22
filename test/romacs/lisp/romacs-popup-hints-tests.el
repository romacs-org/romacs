;;; romacs-popup-hints-tests.el --- A transient map's keys as hints -*- lexical-binding: t; -*-
;; SPDX-License-Identifier: GPL-3.0-or-later
;; Maintainer: Thanos Apollo <public@thanosapollo.org>
(require 'ert)
(require 'romacs-popup)

(defun romacs-popup-hints-tests--next () (interactive))
(defun romacs-popup-hints-tests--quit () (interactive))

(ert-deftest romacs-popup-hints-one-entry-per-command ()
  ;; Aliases share one hint, its shortest keys first, at the place of the
  ;; command's first key; an unbound key shows nothing.
  (let ((map (define-keymap
               "q" #'romacs-popup-hints-tests--quit
               "<right>" #'romacs-popup-hints-tests--next
               "C-f" #'romacs-popup-hints-tests--next
               "n" #'romacs-popup-hints-tests--next
               "f" #'romacs-popup-hints-tests--next
               "x" nil))
        (romacs-popup-hints-strip '("romacs-popup-hints-tests--")))
    (should (equal (romacs-popup--hint-entries map)
                   '(("f/n" . "next") ("q" . "quit"))))
    (let ((romacs-popup-hints-keys-per-command 3))
      (should (equal (car (romacs-popup--hint-entries map)) '("f/n/C-f" . "next"))))))

(ert-deftest romacs-popup-hints-meta-keys-read-as-meta ()
  ;; A map's meta keys live in its ESC map: they show as M- keys, not as
  ;; a prefix.
  (let ((map (define-keymap
               "q" #'romacs-popup-hints-tests--quit
               "M-f" #'romacs-popup-hints-tests--next))
        (romacs-popup-hints-strip '("romacs-popup-hints-tests--")))
    (should (equal (romacs-popup--hint-entries map)
                   '(("M-f" . "next") ("q" . "quit"))))))

(ert-deftest romacs-popup-hints-pack-into-lines ()
  ;; Entries fill a line, then the next; past the last line allowed, it
  ;; ends in an ellipsis.
  (let ((truncate-string-ellipsis "…"))
    (should (equal (romacs-popup--hint-lines '("a 1" "b 2" "c 3") 9 2 #'string-width)
                   '("a 1   b 2" "c 3")))
    (should (equal (romacs-popup--hint-lines '("a 1" "b 2" "c 3") 9 1 #'string-width)
                   '("a 1   b …")))
    (should (equal (romacs-popup--hint-lines '("a long one") 5 2 #'string-width) '("a lo…")))
    (should (equal (romacs-popup--hint-lines nil 9 2 #'string-width) '("")))))

(provide 'romacs-popup-hints-tests)
;;; romacs-popup-hints-tests.el ends here
