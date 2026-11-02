;;; romacs-gnu-ert-bootstrap.el --- Strict GNU ERT source prerequisites
;; SPDX-License-Identifier: GPL-3.0-or-later
;; Maintainer: Thanos Apollo <public@thanosapollo.org>

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
;; loadup.el:138: window.el before files.el (`walk-windows',
;; `pop-to-buffer-same-window', which auto-revert and uniquify reach).
(load "window.el" nil t t)
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
;; simple.el's undo-auto--undoable-change runs on every undoable change and
;; sets its boundary timer with run-at-time, as GNU's loadup provides it.
(load "emacs-lisp/timer.el" nil t t)
;; loadup.el:275: menu-bar.el's maps, which files.el's and simple.el's
;; menu items and tests (filenotify's backup case) reach.
(load "menu-bar.el" nil t t)
;; loaddefs.el's autoload of filenotify's handler, which the `file-notify'
;; special event runs; of cond-star's `cond*', which vc-hooks.el's
;; `vc-refresh-state' (auto-revert's) uses, so before vc-hooks.el loads;
;; and cus-edit's autoloaded alist, which trampver.el adds to at top level.
(autoload 'file-notify-handle-event "filenotify" nil t)
(autoload 'cond* "cond-star" nil nil 'macro)
(defvar customize-package-emacs-version-alist nil)
;; More of GNU's dump (loadup.el:249, 258-260, 280, 293-296, 377, 379), each
;; one a GNU test file reaches.  fringe.el supplies `define-fringe-bitmap'
;; where no window system defines it, as GNU's loaddefs does.
(load "indent.el" nil t t)
(load "font-core.el" nil t t)
(load "emacs-lisp/syntax.el" nil t t)
(load "font-lock.el" nil t t)
(load "textmodes/paragraphs.el" nil t t)
(load "fringe.el" nil t t)
(load "image.el" nil t t)
(load "vc/vc-hooks.el" nil t t)
(load "uniquify.el" nil t t)
;; loaddefs.el's autoload of shortdoc's group macro, which treesit.el
;; calls at top level.
(defvar shortdoc--groups nil)
(autoload 'define-short-documentation-group "shortdoc" nil nil 'macro)

;; GNU batch runs startup before the test file loads; the adapter does not
;; call `command-line', so take its runtime steps here.  normal-top-level
;; (startup.el:585-592): the per-user directory the delayed standard values
;; build on.  command-line (startup.el:1477-1484): preloaded variables whose
;; standard value depends on the runtime (`image-load-path' and friends).
(setq startup--xdg-config-home-emacs
      (let ((xdg-config-home (getenv-internal "XDG_CONFIG_HOME")))
        (if xdg-config-home
            (concat xdg-config-home "/emacs/")
          startup--xdg-config-default)))
(setq user-emacs-directory
      (startup--xdg-or-homedot startup--xdg-config-home-emacs nil))
(let ((user-emacs-directory-warning nil))
  (when (listp custom-delayed-init-variables)
    (mapc #'custom-reevaluate-setting (reverse custom-delayed-init-variables))))
(setq custom-delayed-init-variables t)

(provide 'romacs-gnu-ert-bootstrap)
;;; romacs-gnu-ert-bootstrap.el ends here
