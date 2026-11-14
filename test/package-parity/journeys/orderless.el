;;; orderless.el --- Journey: filtering and highlighting  -*- lexical-binding: t -*-
(pp-step "require" (require 'orderless) (featurep 'orderless))
(defvar pp-orderless-table '("alpha" "beta" "bar" "baz" "gamma" "zabba" "find-file" "file-name-base"))
(pp-step "filter" (orderless-filter "ba z" pp-orderless-table))
(pp-step "initialism" (let ((orderless-matching-styles '(orderless-initialism))) (orderless-filter "ffb" pp-orderless-table)))
(pp-step "style"
  (let ((completion-styles '(orderless)))
    (completion-all-completions "fi e" pp-orderless-table nil 4)))
(pp-step "highlight" (orderless-highlight-matches "ba" '("alpha" "bar")))
