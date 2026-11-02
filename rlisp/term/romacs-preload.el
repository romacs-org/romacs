;;; romacs-preload.el --- dump-safe Romacs backend defaults  -*- lexical-binding: t -*-

;; SPDX-License-Identifier: GPL-3.0-or-later
;; Maintainer: Thanos Apollo <public@thanosapollo.org>

;;; Commentary:

;; Keep backend defaults that must also exist in batch and TTY sessions here.
;; The full `term/romacs-win' layer remains a GUI-runtime concern and is therefore
;; intentionally not loaded into the portable dump.

;;; Code:

;; GNU's X-capable build preloads this binding from term/x-win.el.  Romacs
;; recognizes the same power-management keysym, so its dumped global map must
;; expose the same command even before the GUI terminal layer is loaded.
(global-set-key [XF86WakeUp] #'ignore)

;; `x-gtk-use-system-tooltips' has no C declaration in any GNU build: it is an
;; alias that term/x-win.el installs onto the `use-system-tooltips' DEFVAR_BOOL
;; in src/frame.c (term/x-win.el:1572, and term/pgtk-win.el:372 for pgtk), and
;; loadup.el preloads that file into the dump.  Aliasing rather than declaring
;; is what gives the name `use-system-tooltips' default of t, its docstring, and
;; the Boolean coercion the DEFVAR_BOOL slot performs -- a separate variable
;; here would have all three subtly wrong and would let the two names disagree.
(defvaralias 'x-gtk-use-system-tooltips 'use-system-tooltips)

;;; romacs-preload.el ends here
