;;; romacs-nnheader-tests.el --- Gnus finds its shipped data  -*- lexical-binding: t -*-
;; SPDX-License-Identifier: GPL-3.0-or-later

;; What lisp/romacs/romacs-nnheader.el adds to GNU's
;; `nnheader-find-etc-directory': the shipped etc/ is `data-directory'.

(require 'ert)
(require 'nnheader)

(ert-deftest romacs-nnheader-finds-shipped-data ()
  (should (equal (nnheader-find-etc-directory "gnus-tut.txt" t)
                 (expand-file-name "gnus-tut.txt" data-directory)))
  (should (equal (nnheader-find-etc-directory "images/gnus")
                 (file-name-as-directory (expand-file-name "images/gnus" data-directory))))
  (should-not (nnheader-find-etc-directory "no-such-package")))

(defmacro romacs-nnheader-tests--with-package (root &rest body)
  "Run BODY with ROOT bound to a package tree: ROOT/lisp and ROOT/etc."
  (declare (indent 1))
  `(let ((,root (make-temp-file "romacs-nnheader" t)))
     (unwind-protect
         (progn (make-directory (expand-file-name "lisp" ,root))
                (make-directory (expand-file-name "etc/images/gnus" ,root) t)
                (write-region "" nil (expand-file-name "etc/gnus-tut.txt" ,root) nil 'silent)
                ,@body)
       (delete-directory ,root t))))

(ert-deftest romacs-nnheader-keeps-load-path-order ()
  "With FIRST, a package before GNU's directory wins; one after it does not."
  (romacs-nnheader-tests--with-package root
    (let ((package (expand-file-name "lisp" root))
          (gnu load-path))
      (let ((load-path (cons package gnu)))
        (should (equal (nnheader-find-etc-directory "images/gnus" nil t)
                       (file-name-as-directory (expand-file-name "etc/images/gnus" root))))
        (should (equal (nnheader-find-etc-directory "gnus-tut.txt" t t)
                       (expand-file-name "etc/gnus-tut.txt" root))))
      (let ((load-path (append gnu (list package))))
        (should (equal (nnheader-find-etc-directory "images/gnus" nil t)
                       (file-name-as-directory (expand-file-name "images/gnus" data-directory))))
        (should (equal (nnheader-find-etc-directory "gnus-tut.txt" t t)
                       (expand-file-name "gnus-tut.txt" data-directory)))
        ;; Without FIRST the newest wins, as in GNU: the package made just now.
        (should (equal (nnheader-find-etc-directory "images/gnus")
                       (file-name-as-directory (expand-file-name "etc/images/gnus" root))))
        (should (equal (nnheader-find-etc-directory "gnus-tut.txt" t)
                       (expand-file-name "etc/gnus-tut.txt" root)))))))

(provide 'romacs-nnheader-tests)
;;; romacs-nnheader-tests.el ends here
