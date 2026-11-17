;;; magit.el --- Journey: status, stage, commit  -*- lexical-binding: t -*-
(let ((repo (pp-fixture-repo "repo" '(("a.txt" . "one\n") ("b.txt" . "two\n")))))
  (pp-step "require" (require 'magit) (featurep 'magit))
  (with-temp-file (expand-file-name "a.txt" repo) (insert "one\nchanged\n"))
  (with-temp-file (expand-file-name "c.txt" repo) (insert "new\n"))
  (pp-step "status"
    (let ((default-directory repo))
      (magit-status-setup-buffer repo)
      (list (buffer-name) major-mode)))
  (pp-step "status text" (pp-buffer-text nil 20))
  (pp-step "stage"
    (let ((default-directory repo))
      (magit-stage-files '("a.txt"))
      (magit-refresh)
      (pp-buffer-text nil 20)))
  (pp-step "staged" (let ((default-directory repo)) (magit-git-lines "diff" "--cached" "--name-only")))
  (pp-step "commit"
    (let ((default-directory repo))
      (magit-call-git "commit" "-q" "-m" "Second")
      (magit-refresh)
      (list (magit-rev-parse "HEAD") (pp-buffer-text nil 12)))))
