;;; romacs-popup-hints-tests.el --- A transient map's keys as hints -*- lexical-binding: t; -*-
;; SPDX-License-Identifier: GPL-3.0-or-later
;; Maintainer: Thanos Apollo <public@thanosapollo.org>
(require 'ert)
(require 'romacs-popup)
(require 'cl-lib)

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

(ert-deftest romacs-popup-hints-walk-takes-what-map-keymap-takes ()
  ;; An autoloaded keymap embedded by name, a run of bindings after an
  ;; embedded map, and an old dense map's vector.
  (let* ((dir (make-temp-file "romacs-hints-" t))
         (file (expand-file-name "romacs-popup-hints-auto.el" dir)))
    (unwind-protect
        (progn
          (with-temp-file file
            (insert "(fset 'romacs-popup-hints-tests--auto (define-keymap \"z\" #'ignore))\n"))
          (fmakunbound 'romacs-popup-hints-tests--auto)
          (autoload 'romacs-popup-hints-tests--auto file nil nil 'keymap)
          ;; The walk itself loads the autoloaded map, before `map-keymap'.
          (let ((walked nil))
            (romacs-popup--walk-keymap (list 'keymap 'romacs-popup-hints-tests--auto)
                                       (lambda (event _binding _level _full) (push event walked)))
            (should (equal walked '(?z)))
            (should-not (autoloadp (symbol-function 'romacs-popup-hints-tests--auto))))
          (let ((dense (let ((v (make-vector 128 nil))) (aset v ?a #'ignore) (aset v ?c #'forward-char) (list 'keymap v))))
            (dolist (map (list (list 'keymap 'romacs-popup-hints-tests--auto)
                               (list 'keymap (define-keymap "a" #'ignore) '(?b . backward-char))
                               dense))
              (let (walked visited)
                (map-keymap (lambda (event binding) (push (cons event binding) visited)) map)
                (romacs-popup--walk-keymap map (lambda (event binding _level _full) (push (cons event binding) walked)))
                (should visited)
                (should (equal walked visited))))))
      (fmakunbound 'romacs-popup-hints-tests--auto)
      (delete-directory dir t))))

(ert-deftest romacs-popup-hints-a-full-map-s-characters-ascend ()
  ;; By code, whether in its char-table, its alist (a modified character)
  ;; or an old dense map's vector.
  (romacs-popup-hints-tests--with
    (let ((map (make-keymap)))
      (define-key map "z" #'romacs-popup-hints-tests--quit)
      (define-key map "a" #'romacs-popup-hints-tests--next)
      (should (equal (romacs-popup--hint-entries map) '(("a" . "next") ("z" . "quit")))))
    (let ((map (make-keymap)))
      (define-key map (kbd "H-a") #'romacs-popup-hints-tests--next)
      (define-key map (kbd "H-b") #'romacs-popup-hints-tests--quit)
      (should (equal (romacs-popup--hint-entries map) '(("H-a" . "next") ("H-b" . "quit")))))
    (let ((v (make-vector 128 nil)))
      (aset v ?a #'romacs-popup-hints-tests--next) (aset v ?c #'romacs-popup-hints-tests--quit)
      (should (equal (romacs-popup--hint-entries (list 'keymap v)) '(("a" . "next") ("c" . "quit")))))))

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

(ert-deftest romacs-popup-hints-meta-keys-agree-with-lookup ()
  ;; A meta key the first map unbinds shows when `lookup-key' finds it
  ;; in a later one (a composed sibling), and not when it does not (a
  ;; parent shadowed).
  (romacs-popup-hints-tests--with
    (let* ((first (define-keymap "M-x" nil "q" #'romacs-popup-hints-tests--quit))
           (second (define-keymap "M-x" #'romacs-popup-hints-tests--next))
           (composed (make-composed-keymap (list first second)))
           (parent (define-keymap "M-x" #'romacs-popup-hints-tests--next))
           (child (define-keymap :parent parent "M-x" nil "q" #'romacs-popup-hints-tests--quit)))
      (dolist (map (list composed child))
        (should (equal (and (assoc "M-x" (romacs-popup--hint-entries map)) t)
                       (and (lookup-key map (kbd "M-x")) t)))))))

(ert-deftest romacs-popup-hints-meta-keys-that-cannot-load-show-as-esc ()
  ;; An ESC map from a library that fails to load: ESC as a prefix, as
  ;; before meta keys were read.
  (romacs-popup-hints-tests--with
    (fmakunbound 'romacs-popup-hints-tests--missing)
    (autoload 'romacs-popup-hints-tests--missing "romacs-popup-hints-no-such-library" nil nil 'keymap)
    (unwind-protect
        (let ((map (define-keymap "q" #'romacs-popup-hints-tests--quit)))
          (define-key map [?\e] 'romacs-popup-hints-tests--missing)
          (should (equal (romacs-popup--hint-entries map) '(("q" . "quit") ("ESC" . "+...")))))
      (fmakunbound 'romacs-popup-hints-tests--missing))))

(ert-deftest romacs-popup-hints-a-range-s-part-goes-by-its-own-code ()
  ;; Single keys ahead of a range over them: the range's part left over
  ;; sorts by where it starts, after them.
  (romacs-popup-hints-tests--with
    (let ((table (make-char-table 'keymap)))
      (set-char-table-range table '(?a . ?c) #'romacs-popup-hints-tests--quit)
      (should (equal (romacs-popup--hint-entries
                      (list 'keymap '(?a . romacs-popup-hints-tests--next) '(?b . romacs-popup-hints-tests--previous) table))
                     '(("a" . "next") ("b" . "previous") ("c" . "quit")))))))

(ert-deftest romacs-popup-hints-pack-into-lines ()
  ;; Entries fill a line, then the next; past the last line allowed, it
  ;; ends in an ellipsis.
  (let ((truncate-string-ellipsis "…"))
    (should (equal (romacs-popup--hint-lines '("a 1" "b 2" "c 3") 9 2 #'string-width)
                   '("a 1   b 2" "c 3")))
    (should (equal (romacs-popup--hint-lines '("a 1" "b 2" "c 3") 9 1 #'string-width)
                   '("a 1   b …")))
    (should (equal (romacs-popup--hint-lines '("a long one") 5 2 #'string-width) '("a lo…")))
    (should (equal (romacs-popup--hint-lines nil 9 2 #'string-width) '("")))
    ;; Cut by the width as painted, the dots included: here a wide `w'.
    (let ((wide (lambda (s) (+ (string-width s) (* 2 (cl-count ?w s))))))
      (should (equal (romacs-popup--hint-lines '("ww 1" "b 2") 6 1 wide) '("w…"))))
    ;; Dots wider than the room: cut without them.
    (let ((truncate-string-ellipsis "..."))
      (should (equal (romacs-popup--hint-lines '("abc") 2 1 #'string-width) '("ab"))))))

(provide 'romacs-popup-hints-tests)
;;; romacs-popup-hints-tests.el ends here
