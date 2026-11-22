;;; romacs-agent-images-tests.el --- The image picker -*- lexical-binding: t; -*-
;; SPDX-License-Identifier: GPL-3.0-or-later
;; Maintainer: Thanos Apollo <public@thanosapollo.org>
(require 'ert)
(require 'cl-lib)
(require 'romacs-agent-images)

(defun romacs-agent-images-tests--state (&rest names)
  (romacs-agent-images--state (vconcat (mapcar (lambda (name) (concat "/shots/" name)) names))))

(defun romacs-agent-images-tests--keys (state &rest keys)
  (dolist (key keys state) (setq state (romacs-agent-images--step state key))))

(ert-deftest romacs-agent-images-lists-images-newest-first ()
  (let ((dir (make-temp-file "romacs-images" t)))
    (unwind-protect
        (let ((old (expand-file-name "old.png" dir)) (new (expand-file-name "new.jpg" dir)))
          (write-region "x" nil old nil 'silent)
          (write-region "x" nil new nil 'silent)
          (write-region "x" nil (expand-file-name "notes.txt" dir) nil 'silent)
          (make-directory (expand-file-name "folder.png" dir))
          (set-file-times old (time-subtract nil 100))
          (should (equal (romacs-agent-images--files dir) (list new old))))
      (delete-directory dir t))))

(ert-deftest romacs-agent-images-keys-move-filter-and-end ()
  (let ((state (romacs-agent-images-tests--state "a-romacs.png" "b-term.png" "c-romacs.png")))
    ;; Moving stays within the matches.
    (should (= 2 (plist-get (romacs-agent-images-tests--keys state ?\C-n ?\C-n ?\C-n) :selected)))
    (should (= 0 (plist-get (romacs-agent-images-tests--keys state ?\C-p) :selected)))
    (should (= 2 (plist-get (romacs-agent-images-tests--keys state ?\C-v) :selected)))
    (should (= 0 (plist-get (romacs-agent-images-tests--keys state ?\C-v ?\M-v) :selected)))
    ;; Typing filters, every word anywhere in the name, case aside, and
    ;; the selection starts over.
    (let ((typed (romacs-agent-images-tests--keys state ?\C-n ?R ?o ?m ?\s ?c ?-)))
      (should (equal (plist-get typed :query) "Rom c-"))
      (should (= 0 (plist-get typed :selected)))
      (should (equal (plist-get typed :matches) ["/shots/c-romacs.png"]))
      (should (equal (plist-get (romacs-agent-images-tests--keys typed ?\C-m) :result) "/shots/c-romacs.png"))
      (should (equal (plist-get (romacs-agent-images-tests--keys typed ?\d ?\d ?\d) :query) "Rom")))
    ;; RET on no match stays; a stray key changes nothing.
    (let ((none (romacs-agent-images-tests--keys state ?z ?\C-m 'f5)))
      (should-not (plist-get none :result))
      (should (= 0 (plist-get (romacs-agent-images-tests--keys none ?\C-p ?\C-n) :selected))))
    (should (equal (romacs-agent-images-tests--keys state ?\d) state))
    (should (eq (plist-get (romacs-agent-images-tests--keys state ?\C-g) :result) 'cancel))
    (should (eq (plist-get (romacs-agent-images-tests--keys state ?\C-i) :result) 'browse))
    ;; A step leaves the old state as it was.
    (romacs-agent-images-tests--keys state ?a ?\C-n)
    (should (equal (plist-get state :query) ""))))

(ert-deftest romacs-agent-images-a-file-gone-at-ret-shows-the-menu-again ()
  (let ((dir (make-temp-file "romacs-images" t)))
    (unwind-protect
        (let* ((kept (expand-file-name "kept.png" dir)) (gone (expand-file-name "gone.png" dir))
               (state (romacs-agent-images--state (vector gone kept))))
          (write-region "x" nil kept nil 'silent)
          (let ((chosen (romacs-agent-images--step state ?\C-m)))
            (should (equal (plist-get chosen :result) gone))
            (let ((again (romacs-agent-images--checked chosen dir)))
              (should-not (plist-get again :result))
              (should (equal (plist-get again :matches) (vector kept)))))
          ;; What is there, and what is no file, pass as they are.
          (let ((good (plist-put (copy-sequence state) :result kept)))
            (should (eq (romacs-agent-images--checked good dir) good)))
          (let ((cancel (plist-put (copy-sequence state) :result 'cancel)))
            (should (eq (romacs-agent-images--checked cancel dir) cancel))))
      (delete-directory dir t))))

(ert-deftest romacs-agent-images-lays-the-cards-side-by-side ()
  (let* ((layout (romacs-agent-images--layout 0 30 2000 1300 10 20))
         (padding (* 2 (plist-get (alist-get 'card romacs-popup-look) :padding)))
         (list-end (+ (plist-get layout :list-x) (* 10 (plist-get layout :list-columns)) padding))
         (end (+ (plist-get layout :preview-x) (plist-get layout :preview-width) padding)))
    ;; The preview starts past the list, and the pair is centred.
    (should (> (plist-get layout :preview-x) list-end))
    (should (<= (abs (- (plist-get layout :list-x) (- 2000 end))) 1))
    (should (= (plist-get layout :height) 460))
    (should (= (plist-get layout :lines) 23))
    (should (= (plist-get layout :preview-width) (* 10 (plist-get layout :preview-columns))))
    (should (> (plist-get layout :y) 30)))
  ;; It fits as it is drawn, in whole cells: 16 lines of 30 are 480 pixels.
  (should-not (romacs-agent-images--layout 0 0 2000 510 15 30))
  (let* ((layout (romacs-agent-images--layout 0 0 2000 560 15 30))
         (padding (* 2 (plist-get (alist-get 'card romacs-popup-look) :padding))))
    (should (<= (+ (plist-get layout :y) (* 30 (plist-get layout :lines)) padding) 560)))
  ;; A narrow frame gives the preview what is left; too little, or too
  ;; short a frame, is no layout.
  (let ((layout (romacs-agent-images--layout 0 0 1000 1300 10 20)))
    (should (< (plist-get layout :preview-width) 400))
    (should (>= (plist-get layout :preview-width) romacs-agent-images--preview-min-width)))
  (should-not (romacs-agent-images--layout 0 0 700 1300 10 20))
  (should-not (romacs-agent-images--layout 0 0 2000 300 10 20)))

(ert-deftest romacs-agent-images-row-stays-one-line-with-its-age ()
  (let* ((name "screenshot-2026-11-06_09-41-20-agent-with-a-very-long-name.png")
         (row (romacs-agent-images--row-text name "12d" 30)))
    (should (<= (string-width row) 30))
    (should (string-match-p "…  12d\\'" row))
    ;; What fits stays whole.
    (should (equal (romacs-agent-images--row-text "a.png" "1h" 30) "  a.png  1h"))))

(defun romacs-agent-images-tests--pick-through (layouts)
  "Pick with the frame's layout going through LAYOUTS, one per key, the
size hook firing as `read-key' waits: the result and the layouts shown."
  (let* ((dir (make-temp-file "romacs-images" t)) (shown nil) (now (car layouts)) (next (cdr layouts)))
    (unwind-protect
        (progn
          (write-region "x" nil (expand-file-name "a.png" dir) nil 'silent)
          (cl-letf (((symbol-function 'romacs-agent-images--frame-layout) (lambda () now))
                    ((symbol-function 'romacs-agent-images--show) (lambda (_ _p layout) (push layout shown)))
                    ((symbol-function 'romacs-agent-images--draw-preview) #'ignore)
                    ((symbol-function 'romacs-layer-hide) #'ignore)
                    ((symbol-function 'romacs-layer--root-frame) (lambda () 'root))
                    ((symbol-function 'read-key)
                     (lambda (&rest _)
                       (setq now (if next (pop next) now))
                       (run-hook-with-args 'window-size-change-functions 'root)
                       (if unread-command-events (pop unread-command-events) ?\C-g))))
            (list (romacs-agent-images-pick dir) (reverse shown) window-size-change-functions)))
      (delete-directory dir t))))

(ert-deftest romacs-agent-images-follows-the-frame-and-browses-when-it-shrinks ()
  (let ((window-size-change-functions nil) (unread-command-events nil))
    ;; A new size moves the cards; the same one does nothing.
    (pcase-let ((`(,result ,shown ,hooks)
                 (romacs-agent-images-tests--pick-through '((:y 1) (:y 2)))))
      (should (eq result 'cancel))
      (should (equal shown '((:y 1) (:y 2))))
      (should-not hooks))
    ;; No room left: TAB, once, and the file prompt.
    (pcase-let ((`(,result ,shown ,_)
                 (romacs-agent-images-tests--pick-through '((:y 1) nil))))
      (should (eq result 'browse))
      (should (equal shown '((:y 1))))
      (should-not unread-command-events))))

(ert-deftest romacs-agent-images-short-frame-browses ()
  (let ((shown nil))
    (cl-letf (((symbol-function 'romacs-agent-images--frame-layout) #'ignore)
              ((symbol-function 'romacs-popup-show) (lambda (&rest _) (setq shown t)))
              ((symbol-function 'read-key) (lambda (&rest _) (error "No key is read"))))
      (should (eq (romacs-agent-images-pick temporary-file-directory) 'browse)))
    (should-not shown)))

(ert-deftest romacs-agent-images-keeps-the-selection-in-view ()
  (should (= 0 (romacs-agent-images--first-row 0 100)))
  (should (= 0 (romacs-agent-images--first-row 2 100)))
  (should (= 6 (romacs-agent-images--first-row 10 100)))
  (should (= 91 (romacs-agent-images--first-row 99 100)))
  (should (= 0 (romacs-agent-images--first-row 1 3))))

(ert-deftest romacs-agent-images-draws-only-the-rows-in-view ()
  (with-temp-buffer
    (let ((state (apply #'romacs-agent-images-tests--state
                        (mapcar (lambda (n) (format "%02d.png" n)) (number-sequence 0 49))))
          (shown nil))
      (cl-letf (((symbol-function 'romacs-agent-images--age) (lambda (_) "1d"))
                ((symbol-function 'frame-char-width) (lambda (&rest _) 10))
                ((symbol-function 'romacs-agent-images--insert-image)
                 (lambda (file &rest _) (push file shown))))
        (romacs-agent-images--draw (plist-put state :selected 10)))
      (should (= (length shown) romacs-agent-images--rows))
      (should (string-match-p "Attach image:   50/50" (buffer-string)))
      (should (string-match-p "10.png  1d" (buffer-string)))
      (should-not (string-match-p "20.png" (buffer-string))))))

(provide 'romacs-agent-images-tests)
;;; romacs-agent-images-tests.el ends here
