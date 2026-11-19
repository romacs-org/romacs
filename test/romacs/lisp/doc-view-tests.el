;;; doc-view-tests.el --- doc-view tells a password-protected PDF from the first page  -*- lexical-binding: t -*-

;; `doc-view-pdf-password-protected-ghostscript-p' runs ghostscript on the
;; first page alone: it says a file needs a password on opening it, so the
;; answer is the same as for the whole document, which takes seconds for a
;; book and blocked a restored doc-view buffer's waking.

(require 'ert)
(require 'doc-view)

(defun doc-view-tests--pdf (directory name &rest options)
  "A two-page PDF NAME in DIRECTORY, made by ghostscript with OPTIONS."
  (let ((source (expand-file-name "two.ps" directory))
        (pdf (expand-file-name name directory)))
    (with-temp-file source
      (insert "%!PS\n/Helvetica findfont 20 scalefont setfont\n"
              "72 720 moveto (one) show showpage\n72 720 moveto (two) show showpage\n"))
    (should (zerop (apply #'call-process doc-view-ghostscript-program nil nil nil
                          "-q" "-dSAFER" "-dBATCH" "-dNOPAUSE" "-sDEVICE=pdfwrite"
                          (concat "-sOutputFile=" pdf) (append options (list source)))))
    pdf))

(ert-deftest doc-view-password-check-tells-a-protected-pdf ()
  (skip-unless (executable-find doc-view-ghostscript-program))
  (let ((directory (make-temp-file "doc-view-tests-" t)))
    (unwind-protect
        (progn
          (should-not (doc-view-pdf-password-protected-ghostscript-p
                       (doc-view-tests--pdf directory "plain.pdf")))
          (should (doc-view-pdf-password-protected-ghostscript-p
                   (doc-view-tests--pdf directory "locked.pdf" "-sOwnerPassword=owner"
                                        "-sUserPassword=secret" "-dEncryptionR=3" "-dKeyLength=128"))))
      (delete-directory directory t))))

;;; doc-view-tests.el ends here
