;;; romacs-precompile.el --- Compile likely libraries while the session is idle  -*- lexical-binding: t -*-

;; SPDX-License-Identifier: GPL-3.0-or-later
;; Maintainer: Thanos Apollo <public@thanosapollo.org>

;;; Commentary:

;; A compiled load runs a compiled-Lisp artifact where GNU would load an
;; `.elc' (crates/romacs-vm/src/compiled_lisp).  In an interactive session
;; a load whose file has none reads source, never waiting for a compile.
;; The session compiles while it is idle instead: first the files loads
;; read from source for want of an artifact, then the files compiled loads
;; used before, then every file of the installation's Lisp trees, as GNU's
;; `make' compiles its `.elc' files.
;;
;; A host thread does that work (compiled_lisp/background.rs): one compile
;; process at a time, at the idle CPU and I/O priority, started only once
;; the session has been idle `romacs-precompile-idle-delay' seconds.  It
;; never touches the Lisp heap, and the session does no work for it
;; between keys or while idle.  This file is its switch.  Not in GNU
;; (DIVERGENCES.md VM2-LOADPRECOMPILE).

;;; Code:

(defvar romacs-precompile t
  "Non-nil: compile likely libraries in the background while idle.")

(defvar romacs-precompile-idle-delay 2
  "Seconds of idleness before a background compile starts.")

(defun romacs-precompile-start ()
  "Compile likely libraries in the background while idle, in an
interactive session that compiles Lisp."
  (when (and romacs-precompile (not noninteractive) (fboundp 'romacs--compiled-lisp-background))
    (romacs--compiled-lisp-background t romacs-precompile-idle-delay)))

(defun romacs-precompile-stop ()
  "Start no more background compiles; a compile running finishes."
  (when (fboundp 'romacs--compiled-lisp-background)
    (romacs--compiled-lisp-background nil nil)))

(add-hook 'emacs-startup-hook #'romacs-precompile-start)

(provide 'romacs-precompile)
;;; romacs-precompile.el ends here
