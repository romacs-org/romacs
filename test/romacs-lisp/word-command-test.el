;;; word-command-test.el --- GNU word commands on the native substrate -*- lexical-binding: t -*-
;; SPDX-License-Identifier: GPL-3.0-or-later

(require 'romacs-test)

(romacs-deftest word-delete-bindings ()
  (romacs-should (eq (key-binding [C-backspace]) 'backward-kill-word))
  (romacs-should (eq (key-binding [134217855]) 'backward-kill-word))
  (romacs-should (eq (key-binding [27 127]) 'backward-kill-word)))

(romacs-deftest word-delete-and-yank ()
  (with-temp-buffer
    (let ((kill-ring nil) (kill-ring-yank-pointer nil)
          (interprogram-cut-function nil) (interprogram-paste-function nil)
          (last-command nil) (this-command 'backward-kill-word))
      (insert "one_two three-four")
      (call-interactively 'backward-kill-word)
      (romacs-should (equal (buffer-string) "one_two three-"))
      (romacs-should (equal (car kill-ring) "four"))
      (call-interactively 'yank)
      (romacs-should (equal (buffer-string) "one_two three-four")))))

(romacs-deftest consecutive-backward-word-kills-prepend ()
  (with-temp-buffer
    (let ((kill-ring nil) (kill-ring-yank-pointer nil)
          (interprogram-cut-function nil) (interprogram-paste-function nil)
          (last-command nil) (this-command 'backward-kill-word))
      (insert "one two three")
      (backward-kill-word 1)
      (setq last-command this-command this-command 'backward-kill-word)
      (backward-kill-word 1)
      (romacs-should (equal (buffer-string) "one "))
      (romacs-should (equal kill-ring '("two three"))))))

(provide 'word-command-test)
