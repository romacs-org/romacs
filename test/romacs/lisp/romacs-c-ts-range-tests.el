;;; romacs-c-ts-range-tests.el --- C macro range discovery -*- lexical-binding: t; -*-
(require 'ert)
(require 'cl-lib)
(require 'c-ts-mode)

(defun romacs-c-ts-range-tests--calls (beg end)
  "Count native range queries by the C macro range update on BEG..END."
  (let ((query-range (symbol-function 'treesit-query-range))
        (calls 0))
    (cl-letf (((symbol-function 'treesit-query-range)
               (lambda (&rest args)
                 (cl-incf calls)
                 (apply query-range args))))
      (c-ts-mode--emacs-set-ranges beg end))
    calls))

(ert-deftest romacs-c-ts-range-absent-macro-prefix-skips-wide-query ()
  (skip-unless (treesit-language-available-p 'c))
  (with-temp-buffer
    (insert "/*" (make-string 20000 ?x) "*/\nfor_each_tail(int) lower;\n")
    (let ((case-fold-search t))
      (should (= 0 (romacs-c-ts-range-tests--calls (point-min) (point-max)))))
    (should-not c-ts-mode--for-each-tail-ranges)
    (let ((c-ts-mode--emacs-c-range-query
           (treesit-query-compile 'c '((primitive_type) @type))))
      (should (= 1 (romacs-c-ts-range-tests--calls (point-min) (point-max))))
      (should c-ts-mode--for-each-tail-ranges))))

(ert-deftest romacs-c-ts-range-wide-query-preserves-crossing-declarations ()
  (skip-unless (treesit-language-available-p 'c))
  (dolist (parts '(("FOR_EACH_TAIL(int) a[/*" . "*/1];\n")
                   ("static /*" . "*/ FOR_EACH_TAIL(int) a;\n")))
    (with-temp-buffer
      (insert (car parts))
      (let ((beg (1+ (point))))
        (insert (make-string 20000 ?x))
        (let ((end (1- (point))))
          (insert (cdr parts))
          (let* ((parser (treesit-parser-create 'c nil nil 'for-each))
                 (expected (treesit-query-range (treesit-parser-root-node parser)
                                                c-ts-mode--emacs-c-range-query
                                                beg end)))
            (should expected)
            (should (= 1 (romacs-c-ts-range-tests--calls beg end)))
            (should (equal expected c-ts-mode--for-each-tail-ranges))))))))

(ert-deftest romacs-c-ts-range-wide-query-respects-parser-views ()
  (skip-unless (treesit-language-available-p 'c))
  (with-temp-buffer
    (insert "FOR_EACH_TAIL(int) outside;\n/*" (make-string 20000 ?x) "*/\n")
    (save-restriction
      (goto-char (point-min))
      (forward-line)
      (narrow-to-region (point) (point-max))
      (should (= 1 (romacs-c-ts-range-tests--calls (point-min) (point-max))))
      (should-not c-ts-mode--for-each-tail-ranges))
    (let ((indirect (make-indirect-buffer (current-buffer) " c-ts-range-test" t)))
      (unwind-protect
          (with-current-buffer indirect
            (should (= 1 (romacs-c-ts-range-tests--calls (point-min) (point-max))))
            (should c-ts-mode--for-each-tail-ranges))
        (kill-buffer indirect)))))
