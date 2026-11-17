;;; elisp-mode-flush-tests.el --- An Elisp buffer refontifies after a load only for a new macro  -*- lexical-binding: t -*-

;; elisp-mode.el's `elisp--font-lock-flush-elisp-buffers' runs after every
;; load: an Elisp buffer's faces depend on which names are macros.  A load
;; that defines none, as the first call of `cl-some' loading cl-extra,
;; leaves every Elisp buffer's faces as they were.

(require 'ert)
(require 'elisp-mode)

(defun elisp-mode-flush-tests--flushes (source)
  "How many times loading a file of SOURCE flushes an Elisp buffer's faces."
  (let ((file (make-temp-file "elisp-mode-flush-" nil ".el"
                              (concat ";;; -*- lexical-binding: t -*-\n" source)))
        (flushes 0))
    (let ((count (lambda (&rest _) (setq flushes (1+ flushes)))))
      (with-temp-buffer
        (emacs-lisp-mode)
        (setq-local font-lock-support-mode 'jit-lock-mode)
        (advice-add 'font-lock-flush :before count)
        (unwind-protect (load file nil t)
          (advice-remove 'font-lock-flush count)
          (delete-file file))))
    flushes))

(ert-deftest elisp-mode-flush-only-after-a-load-that-defines-a-macro ()
  (should (= 0 (elisp-mode-flush-tests--flushes
                "(defun elisp-mode-flush-tests--fn () 1)\n(defvar elisp-mode-flush-tests--var 1)\n")))
  (should (< 0 (elisp-mode-flush-tests--flushes
                "(defmacro elisp-mode-flush-tests--mac (x) x)\n"))))

(ert-deftest elisp-mode-flush-assumes-a-macro-without-a-history ()
  (should (elisp--load-defined-macro-p nil))
  (should (elisp--load-defined-macro-p "/no/such/elisp-mode-flush-tests.el")))

;;; elisp-mode-flush-tests.el ends here
