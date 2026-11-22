;;; romacs-popup-hints-tests.el --- A transient map's keys as hints -*- lexical-binding: t; -*-
;; SPDX-License-Identifier: GPL-3.0-or-later
;; Maintainer: Thanos Apollo <public@thanosapollo.org>
(require 'ert)
(require 'romacs-popup)

(defun romacs-popup-hints-tests--next () (interactive))
(defun romacs-popup-hints-tests--previous () (interactive))
(defun romacs-popup-hints-tests--quit () (interactive))

(defmacro romacs-popup-hints-tests--with (&rest body)
  "BODY with the test commands' prefix stripped from their hints."
  (declare (indent 0))
  `(let ((romacs-popup-hints-strip '("romacs-popup-hints-tests--"))
         (romacs-popup-hints-keys-per-command 2))
     ,@body))

(ert-deftest romacs-popup-hints-one-entry-per-command ()
  ;; Aliases share one hint, its shortest keys first, at the place of the
  ;; command's first key; an unbound key shows nothing.
  (romacs-popup-hints-tests--with
    (let ((map (define-keymap
                 "q" #'romacs-popup-hints-tests--quit
                 "<right>" #'romacs-popup-hints-tests--next
                 "C-f" #'romacs-popup-hints-tests--next
                 "n" #'romacs-popup-hints-tests--next
                 "f" #'romacs-popup-hints-tests--next
                 "x" nil)))
      (should (equal (romacs-popup--hint-entries map)
                     '(("q" . "quit") ("n/f" . "next"))))
      (let ((romacs-popup-hints-keys-per-command 3))
        (should (equal (cadr (romacs-popup--hint-entries map)) '("n/f/C-f" . "next")))))))

(ert-deftest romacs-popup-hints-in-the-order-written ()
  ;; A sparse map stores its newest binding first: the card shows them as
  ;; written, a map's own before its parent's.
  (romacs-popup-hints-tests--with
    (let* ((parent (define-keymap "c" #'romacs-popup-hints-tests--quit))
           (map (define-keymap :parent parent
                  "a" #'romacs-popup-hints-tests--next
                  "b" #'romacs-popup-hints-tests--previous)))
      (should (equal (romacs-popup--hint-entries map)
                     '(("a" . "next") ("b" . "previous") ("c" . "quit")))))))

(ert-deftest romacs-popup-hints-walk-is-map-keymap-s-order ()
  ;; The walk that knows each binding's map visits what `map-keymap'
  ;; visits, in its order: a parent, a composed map, a full map.
  (let* ((parent (define-keymap "c" #'ignore "<f5>" #'ignore))
         (child (define-keymap :parent parent "a" #'ignore "b" #'ignore))
         (full (let ((m (make-keymap))) (define-key m "z" #'ignore) (define-key m "a" #'ignore) (define-key m [f6] #'ignore) m))
         (composed (make-composed-keymap (list child full) parent)))
    (dolist (map (list child full composed))
      (let (walked visited)
        (romacs-popup--walk-keymap map (lambda (event binding _level _full) (push (cons event binding) walked)))
        (map-keymap (lambda (event binding) (push (cons event binding) visited)) map)
        (should (equal walked visited))))))

(ert-deftest romacs-popup-hints-a-full-map-s-characters-ascend ()
  (romacs-popup-hints-tests--with
    (let ((map (make-keymap)))
      (define-key map "z" #'romacs-popup-hints-tests--quit)
      (define-key map "a" #'romacs-popup-hints-tests--next)
      (should (equal (romacs-popup--hint-entries map) '(("a" . "next") ("z" . "quit")))))))

(ert-deftest romacs-popup-hints-meta-keys-read-as-meta ()
  ;; A map's meta keys live in its ESC map: they show as M- keys, not as
  ;; a prefix, its parent's too.
  (romacs-popup-hints-tests--with
    (let ((map (define-keymap
                 "q" #'romacs-popup-hints-tests--quit
                 "M-f" #'romacs-popup-hints-tests--next)))
      (should (equal (romacs-popup--hint-entries map)
                     '(("q" . "quit") ("M-f" . "next")))))
    (let* ((parent (define-keymap "M-b" #'romacs-popup-hints-tests--previous))
           (child (define-keymap :parent parent "M-f" #'romacs-popup-hints-tests--next)))
      (should (equal (romacs-popup--hint-entries child)
                     '(("M-f" . "next") ("M-b" . "previous")))))))

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
