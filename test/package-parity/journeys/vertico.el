;;; vertico.el --- Journey: a completing-read through Vertico's UI  -*- lexical-binding: t -*-
(pp-step "require" (require 'vertico) (vertico-mode 1) vertico-mode)
(defvar pp-vertico-seen nil)
(defvar pp-vertico-result nil)
(defun pp-vertico-pick ()
  (interactive)
  (minibuffer-with-setup-hook
      (lambda ()
        (add-hook 'post-command-hook
                  (lambda ()
                    (push (list (minibuffer-contents-no-properties) vertico--index
                                (seq-take vertico--candidates 5))
                          pp-vertico-seen))
                  90 t))
    (setq pp-vertico-result
          (completing-read "Pick: " '("alpha" "beta" "bar" "baz" "gamma" "abba")))))
(keymap-global-set "C-c p" #'pp-vertico-pick)
(pp-step "pick" (pp-keys "C-c p b a C-n RET") pp-vertico-result)
(pp-step "states" (reverse pp-vertico-seen))
(setq pp-vertico-seen nil)
(pp-step "pick first" (pp-keys "C-c p RET") pp-vertico-result)
