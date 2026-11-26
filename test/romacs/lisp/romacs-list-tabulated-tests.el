;;; romacs-list-tabulated-tests.el --- Tabulated lists as Romacs lists -*- lexical-binding: t; -*-
;; SPDX-License-Identifier: GPL-3.0-or-later
;; Maintainer: Thanos Apollo <public@thanosapollo.org>
(require 'ert)
(require 'tabulated-list)
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
     (tabulated-list-print)
     ,@body))

(defun romacs-list-tabulated-tests--fruit ()
  (list (list 'apple (vector "apple" "red")) (list 'lime (vector "lime" "green")) (list 'fig (vector "fig" "purple"))))

(defun romacs-list-tabulated-tests--ids ()
  (mapcar (lambda (row) (plist-get row :id)) (romacs-list-shown-rows)))

(ert-deftest romacs-list-tabulated-an-entry-s-line-is-a-row ()
  (romacs-list-tabulated-tests--with (romacs-list-tabulated-tests--fruit)
    (should (derived-mode-p 'romacs-list-mode))
    (should (equal (buffer-substring-no-properties (point-min) (point-max))
                   "  apple    red\n  fig      purple\n  lime     green\n"))
    (should (equal (romacs-list-tabulated-tests--ids) '(apple fig lime)))
    (should (eq (plist-get (romacs-list-row) :id) 'apple))
    (romacs-list-next 2)
    (should (eq (tabulated-list-get-id) 'lime))
    ;; Highlighted where a print leaves point, with no command run.
    (should hl-line-mode)
    (move-to-column 5)
    (tabulated-list-print t)
    (should (= (current-column) 5))
    (should (= (overlay-start hl-line-overlay) (line-beginning-position)))))

(ert-deftest romacs-list-tabulated-a-tagged-row-is-still-a-row ()
  (romacs-list-tabulated-tests--with (romacs-list-tabulated-tests--fruit)
    (tabulated-list-put-tag "D")
    (should (eq (plist-get (romacs-list-row) :id) 'apple))
    (tabulated-list-set-col 0 "APPLE")
    (should (eq (plist-get (romacs-list-row) :id) 'apple))
    (should (equal (romacs-list-tabulated-tests--ids) '(apple fig lime)))))

(ert-deftest romacs-list-tabulated-a-row-whose-only-column-was-set-is-a-row ()
  (with-temp-buffer
    (romacs-list-tabulated-tests-mode)
    (setq tabulated-list-format [("Name" 8 t)] tabulated-list-padding 0
          tabulated-list-entries (list (list 'apple (vector "apple")) (list 'fig (vector "fig"))))
    (tabulated-list-print)
    (tabulated-list-set-col 0 "APPLE")
    (should (eq (plist-get (romacs-list-row) :id) 'apple))
    (should (equal (romacs-list-tabulated-tests--ids) '(apple fig)))
    (romacs-list-goto 'apple)
    (should (= (point) (point-min)))
    (romacs-list-next 1)
    (should (eq (tabulated-list-get-id) 'fig))
    (romacs-list-previous 1)
    (should (eq (tabulated-list-get-id) 'apple))))

(ert-deftest romacs-list-tabulated-updating-prints-as-gnu-does ()
  (romacs-list-tabulated-tests--with (romacs-list-tabulated-tests--fruit)
    (forward-line 1)
    (tabulated-list-put-tag "D")
    (let ((marker (copy-marker (+ (point) 4))))
      (setq tabulated-list-entries
            (cons (list 'kiwi (vector "kiwi" "brown")) (assq-delete-all 'apple tabulated-list-entries)))
      (tabulated-list-print t t)
      ;; The unchanged entry keeps its line, its tag and a marker in it.
      (should (equal (buffer-substring-no-properties (point-min) (point-max))
                     "D fig      purple\n  kiwi     brown\n  lime     green\n"))
      (should (= marker (+ (point-min) 4)))
      (should (equal (romacs-list-tabulated-tests--ids) '(fig kiwi lime))))))

(ert-deftest romacs-list-tabulated-a-printer-of-its-own-prints-no-rows ()
  (romacs-list-tabulated-tests--with nil
    (setq tabulated-list-entries (romacs-list-tabulated-tests--fruit)
          tabulated-list-printer (lambda (id cols) (insert (format "%s:%s\n" id (aref cols 1)))))
    (tabulated-list-print)
    (should (equal (buffer-substring-no-properties (point-min) (point-max)) "apple:red\nfig:purple\nlime:green\n"))
    (should-not (romacs-list-shown-rows))))

;;; romacs-list-tabulated-tests.el ends here
