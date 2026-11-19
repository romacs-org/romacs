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

(defmacro file-truename-tests--with-fake-handler (&rest body)
  "Run BODY with a handler claiming names that begin with /fake:.
It answers `file-truename' with H before the name, and passes every
other operation on."
  (declare (indent 0))
  `(let ((file-name-handler-alist
          (cons (cons "\\`/fake:" #'file-truename-tests--fake) file-name-handler-alist)))
     ,@body))

(defun file-truename-tests--fake (op &rest args)
  (if (eq op 'file-truename)
      (concat "H" (car args))
    (let ((inhibit-file-name-handlers (cons #'file-truename-tests--fake inhibit-file-name-handlers))
          (inhibit-file-name-operation op))
      (apply op args))))

(defun file-truename-tests--both (name)
  "NAME's true name natively and by GNU's walk, an error as its data."
  (mapcar (lambda (walk) (condition-case e (funcall walk name) (error (list 'error (car e)))))
          (list #'file-truename (lambda (name) (file-truename name (list 100))))))

(ert-deftest file-truename-tests-what-gnu-might-answer-otherwise-is-gnus ()
  (let ((root (file-truename-tests--tree)))
    (unwind-protect
        (progn
          (make-symbolic-link "/fake:x/../plain" (concat root "via"))
          (make-symbolic-link "real/" (concat root "slash"))
          (make-symbolic-link (concat root "real/sub") (concat root "deep"))
          (with-temp-file (concat root "real/λ"))
          (cl-flet ((same (name)
                      (let ((both (file-truename-tests--both name)))
                        (should (equal (car both) (cadr both)))
                        (car both))))
            (file-truename-tests--with-fake-handler
              ;; A link whose target a handler claims on the way; pinned
              ;; GNU quotes the spliced name.
              (should (equal (same (concat root "via")) "/:H/fake:x/../plain"))
              ;; The same, with the handler inhibited for `file-truename'.
              (let ((inhibit-file-name-handlers (list #'file-truename-tests--fake))
                    (inhibit-file-name-operation 'file-truename))
                (should (equal (same (concat root "via")) "/:/plain"))))
            ;; Inside a handler's own call: a handler for a local directory,
            ;; inhibited for `file-truename' but live for `file-symlink-p',
            ;; where it answers a link the disk does not have.
            (let* ((local (concat root "real/sub/"))
                   (handler (lambda (op &rest args)
                              (if (and (eq op 'file-symlink-p) (equal (car args) (concat local "x")))
                                  "../f"
                                (let ((inhibit-file-name-handlers (cons 'file-truename-tests--local inhibit-file-name-handlers))
                                      (inhibit-file-name-operation op))
                                  (apply op args))))))
              (defalias 'file-truename-tests--local handler)
              (let ((file-name-handler-alist (cons (cons (regexp-quote local) 'file-truename-tests--local)
                                                   file-name-handler-alist))
                    (inhibit-file-name-handlers (list 'file-truename-tests--local))
                    (inhibit-file-name-operation 'file-truename))
                (should (equal (same (concat local "x")) (concat root "real/f")))))
            ;; A handler that answers `file-symlink-p' but not
            ;; `file-truename' for an exact leaf: a link only it knows.
            (defalias 'file-truename-tests--virtual
              (lambda (op &rest args)
                (if (eq op 'file-symlink-p)
                    "real"
                  (let ((inhibit-file-name-handlers (cons 'file-truename-tests--virtual inhibit-file-name-handlers))
                        (inhibit-file-name-operation op))
                    (apply op args)))))
            (put 'file-truename-tests--virtual 'operations '(file-symlink-p))
            (let ((file-name-handler-alist
                   (cons (cons (concat "\\`" (regexp-quote (concat root "virt")) "\\'") 'file-truename-tests--virtual)
                         file-name-handler-alist)))
              (should (equal (same (concat root "virt")) (concat root "real"))))
            ;; A HOME that is not normal: GNU expands it, `..' and all.
            (let ((process-environment (cons (concat "HOME=" root "deep/..") process-environment)))
              (let ((default-directory "~/"))
                (should (equal (same "f") (concat root "f")))))
            ;; `..' in an abbreviated `default-directory' climbs from the
            ;; link's target, not from the link.
            (let ((process-environment (cons (concat "HOME=" (directory-file-name root)) process-environment)))
              (let ((default-directory "~/deep/../"))
                (should (equal (same "f") (concat root "real/f")))))
            ;; Slashes GNU keeps: a target ending in one, a leading `//'.
            (same (concat root "slash/f"))
            (same (concat "/" root "real/f"))
            ;; A link to a name beginning with `//'.
            (make-symbolic-link "//tmp/missing" (concat root "dslash"))
            (same (concat root "dslash"))
            ;; A NUL in the name.
            (same (concat root "real/a\0b"))
            ;; A unibyte name with non-ASCII bytes, and a multibyte one,
            ;; each answered in its own representation.
            (let ((unibyte (encode-coding-string (concat root "real/λ") 'utf-8)))
              (should (equal (multibyte-string-p (same unibyte))
                             (multibyte-string-p (file-truename unibyte (list 100))))))
            ;; A multibyte name holding raw bytes.
            (let ((raw (concat root "real/" (string-to-multibyte (unibyte-string #xc3 #xa9)))))
              (should (equal (multibyte-string-p (same raw))
                             (multibyte-string-p (file-truename raw (list 100))))))
            (let ((multibyte (concat root "l/λ")))
              (should (equal (same multibyte) (concat root "real/λ")))
              (should (multibyte-string-p (file-truename multibyte))))))
      (delete-directory root t))))

;;; file-truename-tests.el ends here
