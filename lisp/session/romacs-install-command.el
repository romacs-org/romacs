;;; romacs-install-command.el --- GUI command installation -*- lexical-binding: t -*-

;; SPDX-License-Identifier: GPL-3.0-or-later
;; Maintainer: Thanos Apollo <public@thanosapollo.org>

;;; Commentary:

;; GUI command installation for Romacs.

;;; Code:
(defun romacs-gui-install-command (key definition)
  "Install a zero-argument interactive DEFINITION on an unoccupied KEY.
DEFINITION is a quoted defun named romacs-gui- followed by a nonempty suffix.
KEY is a nonempty vector of character codes; prefix maps must already exist.
Existing bindings are never replaced, except an identical command symbol.
This is a temporary GUI adapter, not GNU command loading."
  (let ((name (car (cdr definition)))
        (args (car (cdr (cdr definition))))
        (body (cdr (cdr (cdr definition))))
        (map (current-global-map))
        (index 0))
    (unless (and (eq (car definition) 'defun)
                 (symbolp name)
                 (> (length (symbol-name name)) 11)
                 (equal (substring (symbol-name name) 0 11) "romacs-gui-")
                 (null args))
      (error "GUI command requires a romacs-gui- named zero-argument defun"))
    (when (stringp (car body)) (setq body (cdr body)))
    (unless (equal (car body) '(interactive))
      (error "GUI command requires (interactive) without arguments"))
    (unless (and (vectorp key) (> (length key) 0))
      (error "GUI command key requires a nonempty character vector"))
    (while (< index (length key))
      (unless (and (integerp (aref key index))
                   (>= (aref key index) 0)
                   (<= (aref key index) 4194303))
        (error "GUI command key requires character codes"))
      (setq index (+ index 1))
      (when (< index (length key))
        (unless (keymapp (lookup-key map (substring key 0 index)))
          (error "GUI command key prefix must already be a keymap"))))
    (let ((old (lookup-key map key)))
      (unless (or (null old) (eq old name))
        (error "GUI command key already has a binding")))
    (eval definition)
    (put name (intern "#:interactive") '(nil))
    (define-key map key name)
    name))
