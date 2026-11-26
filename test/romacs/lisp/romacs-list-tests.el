;;; romacs-list-tests.el --- Lists of rows -*- lexical-binding: t; -*-
;; SPDX-License-Identifier: GPL-3.0-or-later
;; Maintainer: Thanos Apollo <public@thanosapollo.org>
(require 'ert)
(require 'cl-lib)
(require 'romacs-list)

;;; A list of fruit by colour: groups, a level per colour, a live column

(defvar romacs-list-tests--fruit nil "(ID COLOUR RIPE) of each fruit the test list shows.")
(defvar romacs-list-tests--drawn 0 "How many times the test list was drawn.")

(defun romacs-list-tests--rows ()
  "The test list's rows: per colour, its row then its fruit; in a colour's
level, that colour's alone."
  (let ((level romacs-list-level) (rows nil))
    (dolist (colour (delete-dups (mapcar #'cadr romacs-list-tests--fruit)))
      (when (or (null level) (equal level colour))
        (push (list :kind 'colour :id colour) rows)
        (dolist (fruit romacs-list-tests--fruit)
          (when (equal (cadr fruit) colour)
            (push (list :kind 'fruit :id (car fruit) :ripe (nth 2 fruit)) rows)))))
    (nreverse rows)))

(defun romacs-list-tests-name (row) (symbol-name (plist-get row :id)))
(defun romacs-list-tests-ripe (row) (if (plist-get row :ripe) "ripe" "raw"))
(put 'romacs-list-tests-ripe 'romacs-list-live t)

(defvar romacs-list-tests--columns '((romacs-list-tests-name . fill) (romacs-list-tests-ripe . 4)))

(defun romacs-list-tests--line (row width)
  (if (eq (plist-get row :kind) 'colour)
      (format "# %s" (plist-get row :id))
    (romacs-list-cells romacs-list-tests--columns row width "  " t)))

(defun romacs-list-tests--redraw (&optional _changed)
  (cl-incf romacs-list-tests--drawn)
  (romacs-list-render (current-buffer) (romacs-list-tests--rows) 20))

(define-derived-mode romacs-list-tests-mode romacs-list-mode "Fruit"
  "A list for the tests."
  (setq-local romacs-list-redraw-function #'romacs-list-tests--redraw
              romacs-list-line-function #'romacs-list-tests--line
              romacs-list-group-kind 'colour
              romacs-list-empty-function (lambda () "No fruit.")
              romacs-list-hint-function (lambda (width) (substring "n next  p previous  q quit" 0 (min width 26)))
              romacs-list-banner 'wave
              romacs-list-banner-faces '(success warning)
              romacs-list-live-function (lambda (rows) (cl-some (lambda (row) (eq (plist-get row :ripe) 'now)) rows))
              romacs-list-fresh-function
              (lambda (row) (plist-put (copy-sequence row) :ripe (nth 2 (assq (plist-get row :id) romacs-list-tests--fruit))))))

(defmacro romacs-list-tests--with (fruit &rest body)
  "BODY in a drawn test list of FRUIT."
  (declare (indent 1))
  `(let ((romacs-list-tests--fruit ,fruit) (romacs-list-tests--drawn 0) (romacs-list-banners nil))
     (with-temp-buffer
       (romacs-list-tests-mode)
       (unwind-protect
           (progn (funcall romacs-list-redraw-function) ,@body)
         (romacs-banner-stop (romacs-list--banner-id))))))

(defun romacs-list-tests--lines ()
  (split-string (buffer-substring-no-properties (point-min) (point-max)) "\n"))

(defun romacs-list-tests--at () (plist-get (romacs-list-row) :id))

(ert-deftest romacs-list-draws-a-banner-groups-with-rules-between-and-hints ()
  (romacs-list-tests--with '((apple red nil) (cherry red t) (lime green nil))
    (let ((lines (romacs-list-tests--lines)))
      (should (equal (car lines) (substring-no-properties (romacs-banner-still 'wave 20 1 '(success warning) romacs-list--epoch))))
      (should (equal (cdr lines)
                     '("# red" "  apple         raw " "  cherry        ripe"
                       "────────────────────" "# green" "  lime          raw "
                       "n next  p previous  "))))
    (should (equal-including-properties (buffer-substring (point-min) (save-excursion (goto-char (point-min)) (line-end-position)))
                                        (romacs-banner-still 'wave 20 1 '(success warning) romacs-list--epoch)))
    (should (equal (romacs-list-shown-rows) (romacs-list-tests--rows)))
    ;; Shown anew: point on the first row.
    (should (eq (romacs-list-tests--at) 'red))))

(ert-deftest romacs-list-says-so-with-no-rows-and-shows-no-banner-when-told ()
  (romacs-list-tests--with nil
    (should (equal (cdr (romacs-list-tests--lines)) '("No fruit." "n next  p previous  ")))
    (should-not (romacs-list-row)))
  (let ((romacs-list-tests--fruit '((apple red nil))))
    (with-temp-buffer
      (romacs-list-tests-mode)
      (let ((romacs-list-banners '((romacs-list-tests-mode))))
        (funcall romacs-list-redraw-function))
      (should (equal (car (romacs-list-tests--lines)) "# red")))))

(ert-deftest romacs-list-moves-by-row-past-rules-and-keeps-its-row ()
  (romacs-list-tests--with '((apple red nil) (cherry red t) (lime green nil) (kiwi green nil))
    (romacs-list-next 3)
    (should (eq (romacs-list-tests--at) 'green))
    (call-interactively (key-binding (kbd "C-p")))
    (should (eq (romacs-list-tests--at) 'cherry))
    (romacs-list-next 10)
    (should (eq (romacs-list-tests--at) 'kiwi))
    (romacs-list-previous 10)
    (should (eq (romacs-list-tests--at) 'red))
    ;; A fruit lands above: point stays on its row.
    (romacs-list-goto 'lime)
    (push '(fig red nil) romacs-list-tests--fruit)
    (revert-buffer)
    (should (eq (romacs-list-tests--at) 'lime))
    (should (romacs-list--find 'fig))
    ;; Its row gone: point keeps its line.
    (setq romacs-list-tests--fruit (assq-delete-all 'lime romacs-list-tests--fruit))
    (funcall romacs-list-redraw-function)
    (should (eq (romacs-list-tests--at) 'kiwi))))

(ert-deftest romacs-list-enters-a-level-and-comes-back-to-it ()
  (romacs-list-tests--with '((apple red nil) (lime green nil) (kiwi green nil))
    (romacs-list-enter 'green)
    (should (equal (mapcar (lambda (row) (plist-get row :id)) (romacs-list-shown-rows)) '(green lime kiwi)))
    ;; Entered: on its first row, its heading.
    (should (eq (romacs-list-tests--at) 'green))
    (romacs-list-up)
    (should-not romacs-list-level)
    (should (eq (romacs-list-tests--at) 'green))
    (should-error (romacs-list-up) :type 'user-error)
    ;; Drawn anew in a level: past its heading, on its first row.
    (setq romacs-list-level 'green)
    (let ((inhibit-read-only t)) (erase-buffer))
    (funcall romacs-list-redraw-function)
    (should (eq (romacs-list-tests--at) 'lime))))

(ert-deftest romacs-list-cells-fill-their-width-and-cut-from-the-end ()
  (let ((row '(:id banana :ripe t)))
    (should (equal (substring-no-properties (romacs-list-cells romacs-list-tests--columns row 20 "  ")) "  banana        ripe"))
    ;; Live: the live column says so; still: none does.
    (let ((line (romacs-list-cells romacs-list-tests--columns row 20 "  " t)))
      (should (equal (get-text-property 16 'romacs-list-live line) '(romacs-list-tests-ripe 4)))
      (should-not (get-text-property 2 'romacs-list-live line)))
    (should-not (text-property-not-all 0 20 'romacs-list-live nil (romacs-list-cells romacs-list-tests--columns row 20 "  ")))
    ;; Too narrow for every column: cut, and still.
    (let ((line (romacs-list-cells romacs-list-tests--columns row 10 "  " t)))
      (should (= (string-width line) 10))
      (should-not (text-property-not-all 0 (length line) 'romacs-list-live nil line))))
  (should (equal (romacs-list-cell "abc" 5) "abc  "))
  (should (= (string-width (romacs-list-cell "abcdefgh" 5)) 5)))

(ert-deftest romacs-list-asks-its-live-cells-again-while-it-shows ()
  (romacs-list-tests--with '((apple red nil) (cherry red t))
    (let ((before (buffer-string)))
      (setf (nth 2 (assq 'apple romacs-list-tests--fruit)) t)
      (setq romacs-list--ticker (run-at-time 3600 nil #'ignore))
      (cl-letf (((symbol-function 'romacs-list-windows) (lambda (_) (list (selected-window)))))
        (romacs-list--tick (current-buffer)))
      (should (string-search "apple         ripe" (buffer-string)))
      (should (= (length before) (length (buffer-string))))
      (should (= romacs-list-tests--drawn 1))
      (should romacs-list--ticker)
      ;; Not shown: the clock stops.
      (romacs-list--tick (current-buffer))
      (should-not romacs-list--ticker))))

(ert-deftest romacs-list-banner-moves-only-while-live-and-is-chosen-once ()
  (romacs-list-tests--with '((apple red nil))
    (should-not (romacs-animate--get (romacs-list--banner-id)))
    (setq romacs-list-tests--fruit '((apple red now)))
    (funcall romacs-list-redraw-function)
    (should (equal (plist-get (romacs-banner--element (romacs-list--banner-id)) :kind)
                   (list 'banner 'wave romacs-list--epoch '(success warning))))
    (setq romacs-list-tests--fruit '((apple red nil)))
    (funcall romacs-list-redraw-function)
    (should-not (romacs-animate--get (romacs-list--banner-id))))
  ;; Per mode, and at random: chosen as the list is drawn first, kept after.
  (let ((romacs-list-tests--fruit '((apple red nil))) (romacs-list-banners '((romacs-list-tests-mode . random))))
    (with-temp-buffer
      (romacs-list-tests-mode)
      (funcall romacs-list-redraw-function)
      (let ((pattern (romacs-list--banner-pattern)))
        (should (memq pattern romacs-list--patterns))
        (dotimes (_ 5) (funcall romacs-list-redraw-function))
        (should (eq (romacs-list--banner-pattern) pattern))))))

(ert-deftest romacs-list-window-hooks-draw-only-lists ()
  (save-window-excursion
    (romacs-list-tests--with '((apple red nil))
      (let ((list (current-buffer)))
        (set-window-buffer (selected-window) list)
        (romacs-list--shown-in (selected-window))
        (should (= romacs-list-tests--drawn 2))
        ;; As wide as it was drawn: not again.
        (cl-letf (((symbol-function 'romacs-list-columns) (lambda (_) 20)))
          (romacs-list--resized (selected-window)))
        (should (= romacs-list-tests--drawn 2))
        (cl-letf (((symbol-function 'romacs-list-columns) (lambda (_) 30)))
          (romacs-list--resized (selected-window)))
        (should (= romacs-list-tests--drawn 3))
        (with-temp-buffer
          (insert "the user's text")
          (set-window-buffer (selected-window) (current-buffer))
          (romacs-list--shown-in (selected-window))
          (romacs-list--resized (selected-window))
          (should (equal (buffer-string) "the user's text"))
          (should (= romacs-list-tests--drawn 3)))))))

;;; romacs-list-tests.el ends here
