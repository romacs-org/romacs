;;; romacs-list-tabulated-tests.el --- Tabulated lists drawn as lists -*- lexical-binding: t; -*-
;; SPDX-License-Identifier: GPL-3.0-or-later
;; Maintainer: Thanos Apollo <public@thanosapollo.org>
(require 'ert)
(require 'tabulated-list)
(require 'romacs-banner)
(defvar hl-line-mode)
(defvar hl-line-overlay)

(define-derived-mode romacs-list-tabulated-tests-mode tabulated-list-mode "Fruit"
  "A tabulated list for the tests."
  (setq tabulated-list-format [("Name" 8 t) ("Colour" 6 t)]
        tabulated-list-padding 2
        tabulated-list-sort-key '("Name"))
  (tabulated-list-init-header))

(defmacro romacs-list-tabulated-tests--with (entries &rest body)
  "BODY in a printed test list of ENTRIES."
  (declare (indent 1))
  `(with-temp-buffer
     (romacs-list-tabulated-tests-mode)
     (setq tabulated-list-entries ,entries)
     (unwind-protect (progn (tabulated-list-print) ,@body)
       (romacs-list--stop))))

(defconst romacs-list-tabulated-tests--fruit
  '((apple ["apple" "red"]) (lime ["lime" "green"]) (fig ["fig" "purple"])))

(ert-deftest romacs-list-tabulated-a-list-is-a-romacs-list-with-gnu-s-text ()
  (romacs-list-tabulated-tests--with (copy-sequence romacs-list-tabulated-tests--fruit)
    (should (derived-mode-p 'romacs-list-mode))
    (should (equal (buffer-substring-no-properties (point-min) (point-max))
                   "  apple    red\n  fig      purple\n  lime     green\n"))
    (should (= (point) (point-min)))
    (should (eq (tabulated-list-get-id) 'apple))
    (should (eq (plist-get (romacs-list-row) :id) 'apple))
    (should hl-line-mode)
    ;; Highlighted where point went, with no command run.
    (should (= (overlay-start hl-line-overlay) (point)))
    ;; Kept: the row and the column point was at.
    (forward-line 2)
    (move-to-column 5)
    (tabulated-list-print t)
    (should (eq (tabulated-list-get-id) 'lime))
    (should (= (current-column) 5))
    (should (= (overlay-start hl-line-overlay) (line-beginning-position)))
    ;; Not kept: the start.
    (tabulated-list-print)
    (should (= (point) (point-min)))))

(ert-deftest romacs-list-tabulated-updating-keeps-each-shown-entry-s-tag ()
  (romacs-list-tabulated-tests--with (copy-sequence romacs-list-tabulated-tests--fruit)
    (forward-line 1)
    (tabulated-list-put-tag "D")
    (setq tabulated-list-entries (cons '(kiwi ["kiwi" "brown"]) (assq-delete-all 'apple tabulated-list-entries)))
    (tabulated-list-print t t)
    (should (equal (buffer-substring-no-properties (point-min) (point-max))
                   "D fig      purple\n  kiwi     brown\n  lime     green\n"))
    (should (eq (tabulated-list-get-id) 'fig))
    ;; Printed anew: the tag goes.
    (tabulated-list-print t)
    (should (string-prefix-p "  fig" (buffer-substring-no-properties (point-min) (point-max))))))

(ert-deftest romacs-list-tabulated-a-list-shows-a-banner-when-asked ()
  (let ((romacs-list-banners '((romacs-list-tabulated-tests-mode . wave))))
    (romacs-list-tabulated-tests--with (copy-sequence romacs-list-tabulated-tests--fruit)
      (let ((lines (split-string (buffer-substring-no-properties (point-min) (point-max)) "\n")))
        (should (= (length (car lines)) (romacs-list-columns (current-buffer))))
        (should (equal (cdr lines) '("  apple    red" "  fig      purple" "  lime     green" ""))))
      ;; Drawn anew: on its first row, past the banner.
      (should (eq (tabulated-list-get-id) 'apple))
      (tabulated-list-put-tag "D" t)
      (should (eq (tabulated-list-get-id) 'fig)))))

(ert-deftest romacs-list-tabulated-what-romacs-list-cannot-draw-prints-as-gnu ()
  ;; Groups, a fake header, a printer of its own: GNU's loop.
  (romacs-list-tabulated-tests--with nil
    (setq tabulated-list-groups '(("Red" (apple ["apple" "red"])) ("Green" (lime ["lime" "green"]))))
    (tabulated-list-print)
    (should (equal (buffer-substring-no-properties (point-min) (point-max))
                   "Red\n  apple    red\nGreen\n  lime     green\n"))
    (should-not (romacs-list-shown-rows))
    (setq tabulated-list-groups nil
          tabulated-list-entries (copy-sequence romacs-list-tabulated-tests--fruit)
          tabulated-list-printer (lambda (id cols) (insert (format "%s:%s\n" id (aref cols 1)))))
    (tabulated-list-print)
    (should (equal (buffer-substring-no-properties (point-min) (point-max)) "apple:red\nfig:purple\nlime:green\n"))))

;;; romacs-list-tabulated-tests.el ends here
