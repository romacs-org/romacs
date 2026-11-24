;;; paren-treesit-tests.el --- Show Paren in tree-sitter modes -*- lexical-binding: t; -*-

(require 'ert)
(require 'c-ts-mode)
(require 'paren)

(ert-deftest paren-treesit-c-mode-matches-lexically ()
  (skip-unless (treesit-language-available-p 'c))
  (with-temp-buffer
    (c-ts-mode)
    (should (eq major-mode 'c-ts-mode))
    (should (eq show-paren-data-function #'show-paren--default))
    (insert "f (a)")
    (should (equal (funcall show-paren-data-function) '(5 6 3 4 nil)))
    (goto-char 3)
    (should (equal (funcall show-paren-data-function) '(3 4 5 6 nil)))))

;;; paren-treesit-tests.el ends here
