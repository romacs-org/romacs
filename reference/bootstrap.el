;;; bootstrap.el --- Explicit VM2 source bootstrap -*- lexical-binding: t; -*-

;; Run with romacs-vm2-load and ., lisp/ plus lisp/emacs-lisp/ on load-path.
;; An error aborts the sequence; never provide this feature after partial load.
(load "crates/romacs-vm2/src/prims/prelude.el" nil nil t)
(load "debug-early")
(load "byte-run")
(load "backquote")
(load "subr")
(load "macroexp")
(provide 'romacs-bootstrap)
