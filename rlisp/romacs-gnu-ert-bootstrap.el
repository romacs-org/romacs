;;; romacs-gnu-ert-bootstrap.el --- Strict GNU ERT source prerequisites
;; SPDX-License-Identifier: GPL-3.0-or-later

;;; Commentary:
;; ERT-specific source bootstrap: GNU's preloaded libraries, loaded as
;; source in loadup.el's order, except that env and the Mule files still
;; load after files.el here.  Once GNU's custom.el and cus-face.el
;; load, their defcustom/defface own declaration policy; the native forms
;; remain the bare-VM fallback only.

;;; Code:

(load "emacs-lisp/debug-early.el" nil t t)
(load "emacs-lisp/byte-run.el" nil t t)
(load "emacs-lisp/backquote.el" nil t t)
;; subr.el's defvars own these maps in GNU; the native bare-VM skeleton
;; binds them on first keymap use, so yield them before subr.el runs.  The
;; C-level keymaps (function-key-map and friends) stay native.
(current-global-map)
(dolist (map '(global-map esc-map ctl-x-map)) (makunbound map))
(load "subr.el" nil t t)
(load "keymap.el" nil t t)
(load "version.el" nil t t)
(load "widget.el" nil t t)
(load "custom.el" nil t t)
(load "cus-face.el" nil t t)
(load "faces.el" nil t t)
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
(require 'nadvice)
(load "emacs-lisp/easymenu.el" nil t t)
(load "tool-bar.el" nil t t)
(load "progmodes/prog-mode.el" nil t t)
(load "emacs-lisp/regexp-opt.el" nil t t)
(load "emacs-lisp/lisp-mode.el" nil t t)
(load "minibuffer.el" nil t t)
;; cus-start.el's Linux standard value for this C variable, applied before
;; files.el reads it.  GNU reaches it through custom-initialize-delay at
;; startup; loading all of cus-start needs C variables Romacs lacks.
(unless (bound-and-true-p temporary-file-directory)
  (setq temporary-file-directory
        (file-name-as-directory
         (or (getenv-internal "TMPDIR") (getenv-internal "TMP")
             (getenv-internal "TEMP") "/tmp"))))
(load "files.el" nil t t)
(load "frame.el" nil t t)
(load "startup.el" nil t t)
(load "progmodes/elisp-mode.el" nil t t)
(load "env.el" nil t t)
(load "international/mule.el" nil t t)
(load "international/mule-conf.el" nil t t)
(load "international/mule-cmds.el" nil t t)

(provide 'romacs-gnu-ert-bootstrap)
;;; romacs-gnu-ert-bootstrap.el ends here
