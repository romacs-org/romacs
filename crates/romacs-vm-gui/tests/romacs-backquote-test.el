;;; romacs-backquote-test.el --- Native backquote against GNU's Lisp -*- lexical-binding: t -*-
;; SPDX-License-Identifier: GPL-3.0-or-later
;; Maintainer: Thanos Apollo <public@thanosapollo.org>

;; `backquote-process' is native and the compiler takes its answer without
;; calling the macro (DIVERGENCES.md VM2-BACKQUOTE).  GNU's backquote.el,
;; its functions under other names, is the reference here.

(defun backquote-test--read-all (file)
  "Every top-level form of the library FILE."
  (with-temp-buffer
    (insert-file-contents (locate-library file))
    (let (forms)
      (condition-case nil
          (while t (push (read (current-buffer)) forms))
        (end-of-file (nreverse forms))))))

(with-temp-buffer
  (insert-file-contents (locate-library "backquote.el"))
  (while (re-search-forward "backquote-\\(process\\|listify\\|delay-process\\)" nil t)
    (replace-match "backquote-test--\\1"))
  (goto-char (point-min))
  (condition-case nil
      (while t
        (let ((form (read (current-buffer))))
          (when (and (eq (car-safe form) 'defun)
                     (string-prefix-p "backquote-test--" (symbol-name (cadr form))))
            (eval form t))))
    (end-of-file nil)))

(defconst backquote-test--templates
  '(a nil t :k 1 1.5 "s" () (nil) (t) (:k) ("s" 1) (nil . a) (a) (a b) (a b . c)
    ((a . b) c) (quote a) (function a) (\, a) (\,@ a) (a (\, b)) (a (\,@ b))
    ((\,@ a) (\,@ b)) ((\,@ a) b (\,@ c) d) (a (\,@ b) c) (a b (\,@ c)) (a b (\,@ c) d e)
    ((\, a) . b) (a . (\, b)) (a (\, b) . c) (a \,@ b) (a \` b) (a (\,@ b) . c)
    ((\,@ a) . (\, b)) ((a (\, b)) ((\,@ c))) (((\, a)) ((\,@ b)))
    [a b] [] [(\, a)] [a (\,@ b)] [(\,@ a)] [(\,@ a) (\,@ b)] [a [(\, b)]] [\, a] (a . [b (\, c)])
    ([(\, a)] . b) (\, (quote a)) (a (\, (quote b)) c) ((\, (quote a)) . (\, (quote b)))
    (quote (\, a)) (function (\, a)) (\` a) (\` (a (\, b))) (\` (a (\, (\, b))))
    (\` (a (\,@ (\,@ b)))) (a (\` (b (\, (c (\, d)))))) (\` (\` (a (\, (\, (\, b))))))
    (\` [(\, a)]) (a (\,) b))
  "Templates that take each path of `backquote-process'.")

(defconst backquote-test--variables
  '((a . (1 2)) (b . (3 4)) (c . (5)) (d . 6))
  "What the templates' variables hold.")

(defun backquote-test--walk (form visit)
  "Call VISIT on each template under FORM."
  (cond ((consp form)
         (when (and (eq (car form) '\`) (consp (cdr form))) (funcall visit (cadr form)))
         (while (consp form)
           (backquote-test--walk (car form) visit)
           (setq form (cdr form)))
         (when form (backquote-test--walk form visit)))
        ((vectorp form) (mapc (lambda (item) (backquote-test--walk item visit)) form))))

(romacs-deftest backquote-the-walk-is-native ()
  (romacs-should (subrp (symbol-function 'backquote-process)))
  (romacs-should (eq (cdr (symbol-function '\`)) (get 'backquote 'romacs--standard-macro)))
  (romacs-should (documentation 'backquote)))

(romacs-deftest backquote-processes-as-gnu-at-each-level ()
  (dolist (template backquote-test--templates)
    (dolist (level '(nil 0 1 2))
      (romacs-should (equal (backquote-process template level)
                            (backquote-test--process template level))))))

(romacs-deftest backquote-processes-gnu-s-own-templates-as-gnu ()
  (dolist (library '("subr.el" "pcase.el" "cl-macs.el" "gv.el" "custom.el"))
    (dolist (form (backquote-test--read-all library))
      (backquote-test--walk
       form
       (lambda (template)
         (romacs-should (equal (backquote-process template)
                               (backquote-test--process template))))))))

(romacs-deftest backquote-the-macro-answers-the-walk ()
  (dolist (template backquote-test--templates)
    (romacs-should (equal (macroexpand-1 (list '\` template))
                          (cdr (backquote-test--process template))))))

(romacs-deftest backquote-compiled-templates-evaluate-as-gnu-s ()
  (dolist (template backquote-test--templates)
    (let ((ours (condition-case failure
                    (eval (list '\` template) backquote-test--variables)
                  (error failure)))
          (theirs (condition-case failure
                      (eval (cdr (backquote-test--process template)) backquote-test--variables)
                    (error failure))))
      (romacs-should (equal ours theirs)))))

(romacs-deftest backquote-a-redefined-macro-is-called ()
  (let ((standard (symbol-function '\`)))
    (unwind-protect
        (progn
          (defalias '\` (cons 'macro (lambda (_) ''replaced)))
          (romacs-should (eq (eval '(\` redefined-template) t) 'replaced)))
      (defalias '\` standard))))

(romacs-deftest backquote-a-redefined-walk-is-called ()
  (let ((native (symbol-function 'backquote-process)))
    (unwind-protect
        (progn
          (defalias 'backquote-process (lambda (&rest _) '(0 . 'replaced)))
          (romacs-should (eq (eval '(\` walked-template) t) 'replaced)))
      (defalias 'backquote-process native))))

(romacs-deftest backquote-by-either-name-reads-the-same-template ()
  ;; An `eval' datum carries a quotation whole, outside a template; the
  ;; named macro's must still reach its comma.
  (dolist (head '(backquote \`))
    (romacs-should (equal (eval (list head '(quote (\, x))) '((x . 42))) ''42))
    (romacs-should (equal (eval (list head '(a (quote (\, x)) (\,@ y))) '((x . 1) (y . (2))))
                          '(a '1 2)))))

(romacs-deftest backquote-takes-the-advertised-symbols ()
  (let ((variables '((x . 1) (y . (2 3)))))
    (dolist (case '((backquote-unquote-symbol unquote (a (unquote x) b))
                    (backquote-splice-symbol splice (a (splice y) b))
                    (backquote-backquote-symbol quasi (a (quasi (b (\, (\, x))))))))
      (let ((template (nth 2 case)))
        (cl-progv (list (car case)) (list (nth 1 case))
          (romacs-should (equal (backquote-process template) (backquote-test--process template)))
          (romacs-should (equal (macroexpand-1 (list '\` template))
                                (cdr (backquote-test--process template))))
          (romacs-should (equal (eval (list '\` template) variables)
                                (eval (cdr (backquote-test--process template)) variables))))))))

(romacs-deftest backquote-list*-is-a-name-anyone-may-define ()
  (let ((standard (symbol-function 'backquote-list*)))
    (unwind-protect
        (progn
          (fset 'backquote-list* (lambda (&rest _) 'replaced))
          (romacs-should (eq (eval '(backquote-list* 1 2) t) 'replaced)))
      (fset 'backquote-list* standard))))

(romacs-deftest backquote-evaluates-a-constant-as-gnu-does ()
  ;; `backquote-listify' evaluates what it folds, so a malformed quotation
  ;; signals, with GNU's condition, where it is folded and only there.
  (dolist (template '((a (\, (quote b c))) (a (\, (quote))) ((\, (quote b c)) . d)
                      (a \, (quote b c)) (a (\, (quote x . y))) (\, (quote b c)) (\, (quote))))
    (let ((ours (condition-case failure (backquote-process template) (error failure)))
          (theirs (condition-case failure (backquote-test--process template) (error failure))))
      (romacs-should (equal ours theirs))
      ;; Where the walk stands, the code it gives signals when evaluated.
      (romacs-should (equal (condition-case failure (eval (list '\` template) t) (error failure))
                            (if (integerp (car theirs))
                                (condition-case failure (eval (cdr theirs) t) (error failure))
                              theirs))))))

(romacs-deftest backquote-refuses-what-gnu-refuses ()
  (dolist (template '((\, a b) (\,@ a b) (x (\, a b c))))
    (romacs-should (equal (condition-case failure (eval (list '\` template) t) (error failure))
                          (condition-case failure (backquote-test--process template) (error failure)))))
  (romacs-should (equal (condition-case failure (eval '(\` (\, a . b)) t) (error failure))
                        '(wrong-type-argument listp b))))

;;; romacs-backquote-test.el ends here
