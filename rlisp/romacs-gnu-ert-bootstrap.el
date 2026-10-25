;;; romacs-gnu-ert-bootstrap.el --- Strict GNU ERT source prerequisites
;; SPDX-License-Identifier: GPL-3.0-or-later

;;; Commentary:
;; ERT-specific source bootstrap, separate from GNU's standard loadup order.
;; Custom declarations and display themes retain their native Romacs owners.

;;; Code:

(load "emacs-lisp/debug-early.el" nil t t)
(load "emacs-lisp/byte-run.el" nil t t)
(load "emacs-lisp/backquote.el" nil t t)
(load "subr.el" nil t t)
(load "keymap.el" nil t t)
(load "version.el" nil t t)
(load "emacs-lisp/pcase.el" nil t t)
(load "emacs-lisp/gv.el" nil t t)
(load "emacs-lisp/inline.el" nil t t)
(load "bindings.el" nil t t)
(load "emacs-lisp/easy-mmode.el" nil t t)
(load "emacs-lisp/cl-lib.el" nil t t)
(load "button.el" nil t t)
(load "emacs-lisp/derived.el" nil t t)
(load "obarray.el" nil t t)
(load "emacs-lisp/rx.el" nil t t)
(load "abbrev.el" nil t t)
(load "emacs-lisp/cl-preloaded.el" nil t t)
(require 'romacs-cl-generic)
(load "simple.el" nil t t)
(require 'romacs-function-documentation)
(load "emacs-lisp/seq.el" nil t t)
(load "emacs-lisp/easymenu.el" nil t t)
(load "faces.el" nil t t)
(load "tool-bar.el" nil t t)
(load "progmodes/prog-mode.el" nil t t)
(load "emacs-lisp/regexp-opt.el" nil t t)
(load "emacs-lisp/lisp-mode.el" nil t t)
(load "minibuffer.el" nil t t)
(load "files.el" nil t t)
(load "progmodes/elisp-mode.el" nil t t)
(load "env.el" nil t t)
(require 'nadvice)
(load "international/mule.el" nil t t)

(provide 'romacs-gnu-ert-bootstrap)
;;; romacs-gnu-ert-bootstrap.el ends here
