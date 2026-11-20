;;; romacs-popup-card-tests.el --- A key-hint card's order and rows -*- lexical-binding: t; -*-
;; SPDX-License-Identifier: GPL-3.0-or-later
;; Maintainer: Thanos Apollo <public@thanosapollo.org>
(require 'ert)
(require 'cl-lib)
(require 'romacs-popup)

(defun romacs-popup-card-tests--sorted (events)
  "EVENTS in a card's key order, as their descriptions."
  (mapcar #'single-key-description
          (sort (copy-sequence events)
                (lambda (a b) (romacs-popup--rank< (romacs-popup--event-rank a) (romacs-popup--event-rank b))))))

(ert-deftest romacs-popup-card-key-order-reads-which-key-s-keys ()
  "As `which-key-sort-order', the order reads which-key's own bindings:
a whole sequence by its last key, a range by its first; a description
naming no key goes last."
  (should (equal (mapcar #'car (sort (list (cons "C-x C-f" "") (cons "REM" "") (cons "C-x C-<left>" "") (cons "C-x a .. z" "")
                                           (cons "C-x <left>" "") (cons "C-x TAB" "") (cons "C-x M-x" "") (cons "C-x 0 .. 9" ""))
                                     #'romacs-popup-key-order))
                 '("C-x a .. z" "C-x 0 .. 9" "C-x TAB" "C-x <left>" "C-x C-f" "C-x C-<left>" "C-x M-x" "REM")))
  ;; A card's key carries its event: no parsing.
  (should (eq (romacs-popup--key-event (propertize "x" 'romacs-popup-event 'f12)) 'f12)))

(ert-deftest romacs-popup-card-a-command-remapped-to-a-function ()
  "With `which-key-compute-remaps', a command remapped to a function
object still sorts: named as which-key names a binding to one."
  (require 'which-key)
  (let ((map (make-sparse-keymap)) (old (current-global-map)) (which-key-compute-remaps t))
    (define-key map [f12 ?a] #'find-file)
    (define-key map [remap find-file] (lambda () (interactive)))
    (unwind-protect
        (progn (use-global-map map)
               (should (equal (mapcar (lambda (e) (cdr (nth 7 e))) (romacs-popup--card-entries [f12]))
                              (list (which-key--compute-binding 'find-file)))))
      (use-global-map old))))

(ert-deftest romacs-popup-card-legible-on-the-glass ()
  "A caller's colour moved until it reads on the menus' glass at AA; one
that reads already, or an unknown one, unchanged."
  (require 'romacs-theme)
  (let ((spec (get 'romacs-popup 'face-defface-spec)))
    (unwind-protect
        (progn
          (face-spec-set 'romacs-popup '((t :background "#202020")) 'face-defface-spec)
          (should (>= (romacs-color-contrast (romacs-popup-legible "#303030") "#202020") 4.5))
          (should (equal (romacs-popup-legible "#ffffff") "#ffffff"))
          (should (equal (romacs-popup-legible "no-such-colour") "no-such-colour")))
      (face-spec-set 'romacs-popup spec 'face-defface-spec))))

(defun romacs-popup-card-tests--entry (key label binding &optional prefix-p source)
  "An entry as `romacs-popup--card-entry' makes it."
  (list key label prefix-p (or source 'global) 0 binding nil (cons key label)))

(ert-deftest romacs-popup-card-keys-group-by-modifier ()
  "Unmodified keys first (letters, digits, punctuation, named keys, SPC
among them), then each modifier's together: C-SPC and C-+ sit with C-a."
  (should (equal (romacs-popup-card-tests--sorted
                  (list ?\C-a 'C-left ?+ ?\t ?b ?B ?a ?1 ?\s 'left ?\d (aref (kbd "C-SPC") 0)
                        (aref (kbd "C-+") 0) (aref (kbd "M-x") 0) (aref (kbd "C-M-x") 0) 'f10 'f2 ?\e
                        (aref (kbd "M-RET") 0) (aref (kbd "M-DEL") 0) 'C-M-left ?\C-@))
                 '("a" "b" "B" "1" "+" "DEL" "ESC" "SPC" "TAB" "<f2>" "<f10>" "<left>"
                   "C-a" "C-+" "C-SPC" "C-@" "C-<left>" "M-x" "M-DEL" "M-RET" "C-M-x" "C-M-<left>"))))

(defun romacs-popup-card-tests--keys (rows)
  "Each of ROWS as its keys: the first, then the others."
  (mapcar (lambda (e) (cons (car e) (nth 6 e))) rows))

(ert-deftest romacs-popup-card-one-command-one-row ()
  "Keys bound to one command share its first key's row, the others drawn
after its label; a different binding, label or source keeps its own row."
  (let* ((f (lambda () (interactive)))
         (g (lambda () (interactive)))
         (rows (romacs-popup--card-merge
                (list (romacs-popup-card-tests--entry "C-0" "text-scale-adjust" 'text-scale-adjust)
                      (romacs-popup-card-tests--entry "a" "other" 'ignore)
                      (romacs-popup-card-tests--entry "C-+" "text-scale-adjust" 'text-scale-adjust)
                      (romacs-popup-card-tests--entry "x" "function" f)
                      (romacs-popup-card-tests--entry "y" "function" g)
                      (romacs-popup-card-tests--entry "r" "copy" 'kill-ring-save)
                      (romacs-popup-card-tests--entry "w" "kill-ring-save" 'kill-ring-save)
                      (romacs-popup-card-tests--entry "m" "text-scale-adjust" 'text-scale-adjust nil 'mode)))))
    (should (equal (romacs-popup-card-tests--keys rows)
                   '(("C-0" "C-+") ("a") ("x") ("y") ("r") ("w") ("m"))))
    ;; Drawn after the label: the key column stays the first key's.
    (let ((line (car (romacs-popup--card-column (list (car rows)) 80))))
      (should (equal (substring-no-properties line) "C-0  text-scale-adjust  C-+")))
    ;; Wider in cells than the room though narrower as painted: still
    ;; the bare label, never a cut row of keys.
    (cl-letf (((symbol-function 'romacs-layer--toast-width) (lambda (s) (- (string-width s) 4))))
      (should (equal (substring-no-properties (car (romacs-popup--card-column (list (car rows)) 26)))
                     "C-0  text-scale-adjust")))
    ;; Cut to a narrow room, the other keys go first.
    (should (equal (substring-no-properties (car (romacs-popup--card-column (list (car rows)) 24)))
                   "C-0  text-scale-adjust"))))

(ert-deftest romacs-popup-card-a-row-of-keys-stays-narrow ()
  "A command's keys past `romacs-popup--merged-key-width' start its next
row, the rest joining that one: rows stay narrow, in order."
  (let ((rows (romacs-popup--card-merge
               (mapcar (lambda (k) (romacs-popup-card-tests--entry k "same" 'forward-char))
                       '("C-a" "C-b" "C-c" "C-d" "C-e" "C-f" "C-g")))))
    (should (equal (romacs-popup-card-tests--keys rows)
                   '(("C-a" "C-b" "C-c" "C-d") ("C-e" "C-f" "C-g"))))))

(provide 'romacs-popup-card-tests)
;;; romacs-popup-card-tests.el ends here
