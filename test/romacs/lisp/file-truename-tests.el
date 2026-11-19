;;; file-truename-tests.el --- The native true name walk answers as GNU's Lisp -*- lexical-binding: t; -*-
;; SPDX-License-Identifier: GPL-3.0-or-later
;; Maintainer: Thanos Apollo <public@thanosapollo.org>

;; `file-truename' walks a local name natively (LISP-NATIVE-TRUENAME).
;; Each case is answered three ways: by the native walk, by GNU's own
;; algorithm (`file-truename--walk', which COUNTER also selects), and by
;; pinned GNU, whose answers are recorded below with the tree's root as R.

(require 'ert)
(require 'cl-lib)

(defun file-truename-tests--tree ()
  "A temporary tree of links; its true root, with a slash."
  (let ((root (file-name-as-directory (file-truename--walk (make-temp-file "truename" t) nil nil))))
    (dolist (dir '("real" "other" "real/sub"))
      (make-directory (expand-file-name dir root)))
    (with-temp-file (expand-file-name "real/f" root))
    (make-symbolic-link (expand-file-name "real" root) (expand-file-name "l" root))
    (make-symbolic-link "real" (expand-file-name "rel" root))
    (make-symbolic-link "../other" (expand-file-name "real/up" root))
    (make-symbolic-link "loop2" (expand-file-name "loop1" root))
    (make-symbolic-link "loop1" (expand-file-name "loop2" root))
    (dotimes (i 5)
      (make-symbolic-link (if (= i 4) "real" (format "chain%d" (1+ i))) (expand-file-name (format "chain%d" i) root)))
    root))

(defconst file-truename-tests--cases
  '("l/f" "rel/f" "l/../x" "real/up/g" "l/" "l" "nothing/tail" "l/missing/deeper"
    "chain0/f" "./real//f" "real/sub/../f" "real/.")
  "Names under the tree's root.")

(defun file-truename-tests--answers (root walk)
  "Each case's answer from WALK under ROOT, ROOT shown as R."
  (let ((shown (lambda (name) (string-replace root "R/" name))))
    (append
     (mapcar (lambda (case) (funcall shown (funcall walk (concat root case)))) file-truename-tests--cases)
     (list
      ;; A relative name, read in `default-directory', itself through a link.
      (funcall shown (let ((default-directory (concat root "l/"))) (funcall walk "f")))
      (funcall shown (let ((default-directory (concat root "rel/sub/"))) (funcall walk "../f")))
      ;; A loop of links.
      (condition-case e (funcall walk (concat root "loop1/f"))
        (error (funcall shown (cadr e))))))))

(ert-deftest file-truename-tests-the-native-walk-answers-as-gnu ()
  (let ((root (file-truename-tests--tree)))
    (unwind-protect
        (let ((native (file-truename-tests--answers root #'file-truename))
              (lisp (file-truename-tests--answers root (lambda (name) (file-truename name (list 100))))))
          (should (equal native lisp))
          ;; Which link a loop's message names depends on how deep the
          ;; root is (GNU's budget of 100 steps runs out somewhere along
          ;; the loop), so only its start is compared with GNU's.
          (should (equal (append (butlast native)
                                 (list (string-prefix-p "Apparent cycle of symbolic links for R/loop"
                                                        (car (last native)))))
                         ;; Pinned GNU's answers.
                         '("R/real/f" "R/real/f" "R/x" "R/other/g" "R/real/" "R/real" "R/nothing/tail"
                           "R/real/missing/deeper" "R/real/f" "R/real/f" "R/real/f" "R/real"
                           "R/real/f" "R/real/f" t))))
      (delete-directory root t))))

(ert-deftest file-truename-tests-home-handlers-and-the-optional-arguments ()
  (let ((root (file-truename-tests--tree)))
    (unwind-protect
        (progn
          ;; `~' and `~/NAME' are the home directory's true name.
          (should (equal (file-truename "~") (file-truename--walk (expand-file-name "~") nil nil)))
          (should (equal (file-truename "~/x") (file-truename (expand-file-name "~/x") (list 100))))
          ;; A handler that claims the name answers for it, as in GNU.
          (let ((file-name-handler-alist
                 (cons (cons "\\`/fake:" (lambda (op &rest args) (if (eq op 'file-truename) (concat "H" (car args))
                                                                  (let ((inhibit-file-name-handlers (list (cdr (car file-name-handler-alist))))
                                                                        (inhibit-file-name-operation op))
                                                                    (apply op args)))))
                       file-name-handler-alist)))
            (should (equal (file-truename "/fake:x") "H/fake:x"))
            ;; A local link into a name the handler claims is the
            ;; handler's too, as GNU's walk finds it.
            (make-symbolic-link "/fake:y" (concat root "tofake"))
            (should (equal (file-truename (concat root "tofake"))
                           (file-truename (concat root "tofake") (list 100))))
            (should-not (equal (file-truename (concat root "tofake")) "/fake:y")))
          ;; COUNTER and PREV-DIRS still select GNU's algorithm, with the
          ;; same answer.
          (should (equal (file-truename (concat root "l/f") (list 100) (list nil)) (concat root "real/f"))))
      (delete-directory root t))))

;;; file-truename-tests.el ends here
