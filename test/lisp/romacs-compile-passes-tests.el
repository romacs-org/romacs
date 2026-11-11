;;; romacs-compile-passes-tests.el --- The file compiler's source passes -*- lexical-binding: t; -*-
;; SPDX-License-Identifier: GPL-3.0-or-later

(require 'ert)
(require 'romacs-compile)

(defun romacs-compile-passes-tests--pre (form)
  "FORM as the file compiler preprocesses it in a `lexical-binding' file."
  (let ((lexical-binding t)) (romacs-compile--preprocess form)))

(defun romacs-compile-passes-tests--both (form)
  "FORM's value or signal, run as written and as preprocessed: (WRITTEN . PASSED)."
  (let ((run (lambda (f) (condition-case err (list :value (eval f t)) (error (list :error err))))))
    (cons (funcall run form) (funcall run (romacs-compile-passes-tests--pre form)))))

;;; Constant folding

(ert-deftest romacs-compile-folds-pure-calls-on-constants ()
  (should (equal (romacs-compile-passes-tests--pre '(concat "abc" "def")) ''"abcdef"))
  (should (equal (romacs-compile-passes-tests--pre '(symbol-name 'foo)) ''"foo"))
  ;; Nested: an inner fold makes the outer call's argument constant.
  (should (equal (romacs-compile-passes-tests--pre '(length (concat "ab" (symbol-name 'cd)))) ''4))
  (should (equal (romacs-compile-passes-tests--pre '(+ 1 2 (* 3 4))) ''15)))

(ert-deftest romacs-compile-leaves-what-it-may-not-fold ()
  ;; A call that signals runs, and signals, when the code runs.
  (should (equal (romacs-compile-passes-tests--pre '(car 5)) '(car 5)))
  ;; A variable argument, and a function GNU does not mark pure.
  (should (equal (romacs-compile-passes-tests--pre '(concat "a" x)) '(concat "a" x)))
  (should (equal (romacs-compile-passes-tests--pre '(format "%s" 1)) '(format "%s" 1)))
  (should (equal (romacs-compile-passes-tests--pre '(current-time-string)) '(current-time-string))))

;;; mapcar and mapc of a literal lambda

(defconst romacs-compile-passes-tests--sequences
  '(nil (1) (1 2 3) [1 2 3] "abc" (1 . 2) (1 2 . 3) 5)
  "Every kind of argument a mapping function meets, the refused ones included.")

(ert-deftest romacs-compile-maps-in-place-with-the-same-results ()
  (dolist (seq romacs-compile-passes-tests--sequences)
    (dolist (form `((mapcar (lambda (x) (list x x)) ',seq)
                    (mapc (lambda (x) (ignore x)) ',seq)
                    (let ((acc nil)) (mapc (lambda (x) (push x acc)) ',seq) acc)))
      (let ((both (romacs-compile-passes-tests--both form)))
        (should (equal (car both) (cdr both))))))
  ;; A circular list signals, as before.
  (let ((both (romacs-compile-passes-tests--both
               '(let ((l (list 1 2 3))) (setcdr (cddr l) l) (mapcar (lambda (x) x) l)))))
    (should (eq (car (nth 1 (car both))) 'circular-list))
    (should (equal (car both) (cdr both)))))

(ert-deftest romacs-compile-maps-in-place-as-the-function-walks ()
  ;; A body that shortens or lengthens the list it walks: the function
  ;; walks the length the list had, and stops at a tail that is not a cons.
  (dolist (form '((let ((l (list 1 2 3 4))) (mapcar (lambda (x) (setcdr (cdr l) nil) x) l))
                  (let ((l (list 1 2))) (mapcar (lambda (x) (nconc l (list 9)) x) l))))
    (let ((both (romacs-compile-passes-tests--both form)))
      (should (equal (car both) (cdr both)))))
  ;; Each element has a binding of its own, as each call had.
  (let ((both (romacs-compile-passes-tests--both
               '(mapcar #'funcall (mapcar (lambda (x) (lambda () x)) '(1 2 3))))))
    (should (equal (car both) '(:value (1 2 3))))
    (should (equal (car both) (cdr both)))))

(ert-deftest romacs-compile-maps-in-place-only-while-the-function-is-its-own ()
  ;; An advised or replaced mapcar is called with the list, as it would be.
  ;; Each stand-in passes every other call on, since evaluating a form
  ;; maps too; it marks only calls with the test's own list.
  (let* ((marked (list 1 2))
         (form `(mapcar (lambda (x) (* 10 x)) ',marked))
         (passed (romacs-compile-passes-tests--pre form))
         (original (symbol-function 'mapcar))
         (run (lambda (f) (condition-case err (eval f t) (error (list :error err))))))
    (cl-letf (((symbol-function 'mapcar)
               (lambda (fn seq) (if (eq seq marked) (list :replaced seq) (funcall original fn seq)))))
      (should (equal (funcall run form) (list :replaced marked)))
      (should (equal (funcall run passed) (list :replaced marked))))
    (let ((advice (lambda (orig fn seq) (if (eq seq marked) (cons :advised (funcall orig fn seq)) (funcall orig fn seq)))))
      (unwind-protect
          (progn (advice-add 'mapcar :around advice)
                 (should (equal (funcall run form) '(:advised 10 20)))
                 (should (equal (funcall run passed) '(:advised 10 20))))
        (advice-remove 'mapcar advice)))
    ;; And the original again: the loop in place.
    (should (equal (funcall run passed) '(10 20)))))

(ert-deftest romacs-compile-maps-in-place-only-what-it-can ()
  ;; Not a lambda of one required argument, or not a lexical file: as written.
  (dolist (form '((mapcar #'1+ l)
                  (mapcar (lambda (x y) (+ x y)) l)
                  (mapcar (lambda (&optional x) x) l)
                  (mapcar (lambda (x) "Doc." (interactive) x) l)))
    (should (eq (car (romacs-compile-passes-tests--pre form)) 'mapcar)))
  (let ((lexical-binding nil))
    (should (eq (car (romacs-compile--preprocess '(mapcar (lambda (x) x) l))) 'mapcar)))
  (should (eq (car (romacs-compile-passes-tests--pre '(mapcar (lambda (x) x) l))) 'let*)))

(provide 'romacs-compile-passes-tests)
;;; romacs-compile-passes-tests.el ends here
