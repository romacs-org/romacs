;;; romacs-loadup-bootstrap.el --- GNU loadup as source, for package suites
;; SPDX-License-Identifier: GPL-3.0-or-later

;;; Commentary:
;; The files GNU's loadup.el preloads on GNU/Linux, as source, in loadup's
;; order (of its window-system block, fringe.el so far), then the steps of
;; startup.el's `command-line' a batch session depends on.  Romacs's own display and
;; keymap layers load where GNU's C would already be in place.  Each Romacs
;; adaptation is commented where it happens.
;;
;; This is the regime for package suites (Gnosis, emacs-hermes), which
;; assume a dumped GNU Emacs.  The GNU ERT ratchet keeps the smaller
;; romacs-gnu-ert-bootstrap.el: this chain costs about a minute per fresh
;; VM, which its per-file budgets do not hold.

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
(load "emacs-lisp/map-ynp.el" nil t t)
(load "international/mule.el" nil t t)
(load "international/mule-conf.el" nil t t)
(load "env.el" nil t t)
(load "format.el" nil t t)
(load "bindings.el" nil t t)
(load "window.el" nil t t)
;; loadup.el: mini-windows can resize now, so the variable takes its
;; advertised default (xdisp.c starts it as nil).
(setq resize-mini-windows 'grow-only)
;; cus-start.el's Linux standard value for this C variable, applied before
;; files.el reads it.  GNU reaches it through custom-initialize-delay at
;; startup, after cus-start preloads.
(unless (bound-and-true-p temporary-file-directory)
  (setq temporary-file-directory
        (file-name-as-directory
         (or (getenv-internal "TMPDIR") (getenv-internal "TMP")
             (getenv-internal "TEMP") "/tmp"))))
(load "files.el" nil t t)
(load "emacs-lisp/macroexp.el" nil t t)
(load "cus-face.el" nil t t)
(load "faces.el" nil t t)
(load "loaddefs.el" nil t t)
(load "button.el" nil t t)
;; loadup.el: add-hook is interpreted here, so gv is required now.
(require 'gv)
(load "emacs-lisp/cl-preloaded.el" nil t t)
(load "emacs-lisp/oclosure.el" nil t t)
(load "obarray.el" nil t t)
(load "abbrev.el" nil t t)
(load "help.el" nil t t)
(load "jka-cmpr-hook.el" nil t t)
(load "epa-hook.el" nil t t)
(load "international/mule-cmds.el" nil t t)
(load "case-table.el" nil t t)
(load "international/charprop.el" t t t)
(load "international/characters.el" nil t t)
(load "composite.el" nil t t)
(load "language/chinese.el" nil t t)
(load "language/cyrillic.el" nil t t)
(load "language/indian.el" nil t t)
(load "language/sinhala.el" nil t t)
(load "language/english.el" nil t t)
(load "language/ethiopic.el" nil t t)
(load "language/european.el" nil t t)
(load "language/czech.el" nil t t)
(load "language/slovak.el" nil t t)
(load "language/romanian.el" nil t t)
(load "language/greek.el" nil t t)
(load "language/hebrew.el" nil t t)
(load "international/cp51932.el" nil t t)
(load "international/eucjp-ms.el" nil t t)
(load "language/japanese.el" nil t t)
(load "language/korean.el" nil t t)
(load "language/lao.el" nil t t)
(load "language/tai-viet.el" nil t t)
(load "language/thai.el" nil t t)
(load "language/tibetan.el" nil t t)
(load "language/vietnamese.el" nil t t)
(load "language/misc-lang.el" nil t t)
(load "language/utf-8-lang.el" nil t t)
(load "language/georgian.el" nil t t)
(load "language/khmer.el" nil t t)
(load "language/burmese.el" nil t t)
(load "language/cham.el" nil t t)
(load "language/philippine.el" nil t t)
(load "language/indonesian.el" nil t t)
(load "indent.el" nil t t)
;; GNU cl-generic, loaded through Romacs's type-query bridge.
(require 'romacs-cl-generic)
(load "simple.el" nil t t)
(require 'romacs-function-documentation)
(load "emacs-lisp/seq.el" nil t t)
(load "emacs-lisp/nadvice.el" nil t t)
(load "minibuffer.el" nil t t)
(load "frame.el" nil t t)
(load "startup.el" nil t t)
(load "term/tty-colors.el" nil t t)
;; Romacs owns the display layer of frame.el and faces.el (VM2-DISPLAY1).
(load "romacs-frame.el" nil t t)
;; GNU computes the active keymaps in C; Romacs in Lisp.
(load "romacs-keymap.el" nil t t)
(load "font-core.el" nil t t)
(load "emacs-lisp/syntax.el" nil t t)
(load "font-lock.el" nil t t)
(load "jit-lock.el" nil t t)
(load "mouse.el" nil t t)
(load "select.el" nil t t)
(load "emacs-lisp/timer.el" nil t t)
(load "emacs-lisp/easymenu.el" nil t t)
(load "isearch.el" nil t t)
(load "rfn-eshadow.el" nil t t)
(load "menu-bar.el" nil t t)
(load "tab-bar.el" nil t t)
(load "emacs-lisp/lisp.el" nil t t)
(load "textmodes/page.el" nil t t)
(load "register.el" nil t t)
(load "textmodes/paragraphs.el" nil t t)
(load "progmodes/prog-mode.el" nil t t)
(load "emacs-lisp/lisp-mode.el" nil t t)
(load "textmodes/text-mode.el" nil t t)
(load "textmodes/fill.el" nil t t)
(load "newcomment.el" nil t t)
(load "replace.el" nil t t)
(load "emacs-lisp/tabulated-list.el" nil t t)
(load "buff-menu.el" nil t t)
;; loadup.el's window-system block: Romacs is a graphical machine, and
;; pinned GNU, a graphical build, preloads fringe.el in batch too.
(load "fringe.el" nil t t)
(load "progmodes/elisp-mode.el" nil t t)
(load "emacs-lisp/float-sup.el" nil t t)
(load "vc/vc-hooks.el" nil t t)
(load "vc/ediff-hook.el" nil t t)
(load "uniquify.el" nil t t)
(load "electric.el" nil t t)
(load "paren.el" nil t t)
(load "emacs-lisp/shorthands.el" nil t t)
(load "emacs-lisp/eldoc.el" nil t t)
(load "emacs-lisp/cconv.el" nil t t)
;; loadup.el preloads cus-start with this flag bound.
(dlet ((cus-start--preload t)) (load "cus-start.el" nil t t))
(load "tooltip.el" nil t t)
(load "international/iso-transl.el" nil t t)
(load "emacs-lisp/rmc.el" nil t t)
(load "leim/leim-list.el" nil t t)

;; startup.el command-line (startup.el:1479): variables whose standard value
;; depends on the runtime (user-mail-address and friends) get it once the
;; preloaded files are in.  Romacs does not run command-line in this
;; bootstrap, so it takes this one step itself, in batch.  A session that
;; starts in another context evaluates the same list again in its own
;; (rlisp/romacs-gui-loadup.el).
(defvar romacs-loadup-delayed-init-variables
  (and (listp custom-delayed-init-variables) (reverse custom-delayed-init-variables))
  "The preloaded variables whose standard value depends on the runtime, in load order.
These are `custom-delayed-init-variables' as the preloaded files left it.")
(let ((user-emacs-directory-warning nil))
  (mapc #'custom-reevaluate-setting romacs-loadup-delayed-init-variables))
(setq custom-delayed-init-variables t)
;; startup.el command-line (startup.el:843): the initial frame's faces for
;; its background mode and display type.
(frame-set-background-mode (selected-frame))
;; startup.el command-line (startup.el:1717): initialization is over.
(setq after-init-time (current-time))

(provide 'romacs-loadup-bootstrap)
;;; romacs-loadup-bootstrap.el ends here
