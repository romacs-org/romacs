;;; projectile.el --- Journey: a project's root and files  -*- lexical-binding: t -*-
(pp-step "require" (require 'projectile) (projectile-mode 1) projectile-mode)
(let ((repo (pp-fixture-repo "proj" '(("a.el" . "(a)\n") ("README" . "x\n") ("src/b.el" . "(b)\n")))))
  (pp-step "root" (let ((default-directory (expand-file-name "src/" repo))) (projectile-project-root)))
  (pp-step "files" (sort (projectile-project-files repo) #'string<))
  (pp-step "type" (let ((default-directory repo)) (projectile-project-type)))
  (pp-step "name" (let ((default-directory repo)) (projectile-project-name))))
