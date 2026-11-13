;;; romacs-keymap.el --- Lisp called by keymap lookup -*- lexical-binding: t; -*-
;; Copyright (C) 1985-1989, 1993-1997, 1999-2026 Free Software Foundation,
;; Inc.
;; SPDX-License-Identifier: GPL-3.0-or-later
;; Maintainer: Thanos Apollo <public@thanosapollo.org>

;;; Commentary:
;; The Lisp that keymap lookup calls.  Which keymaps are active, and
;; `key-binding' over them, are primitives as in GNU: they read the
;; overriding maps, the emulation and minor-mode alists, the `keymap' and
;; `local-map' properties and the local and global maps, so the policy
;; stays those variables and properties.
;;
;; Adapted from GNU src/keyboard.c (`posn-point',
;; `menu_item_eval_property').

;;; Code:

(defun posn-point (position)
  "Return the buffer location in POSITION, or nil.
POSITION is a list of the form returned by `event-start' and `event-end'."
  (or (nth 5 position)
      (let ((posn (nth 1 position)))
        (cond ((consp posn) (car posn))
              ((integerp posn) posn)))))

(defun romacs--menu-filter (filter definition)
  "Apply a menu item's FILTER to its DEFINITION, as keymap lookup does.
Errors give nil (keyboard.c menu_item_eval_property)."
  (let ((inhibit-redisplay t))
    (condition-case nil (eval (list filter (list 'quote definition)) nil) (error nil))))

(provide 'romacs-keymap)
;;; romacs-keymap.el ends here
