#!/bin/sh
# GNU answers for tests/file_local_special.rs (fixtures fla.el, flb.el here).
cd "$(dirname "$0")"
echo ";; $(/usr/bin/emacs --version | head -1) sha256 $(sha256sum /usr/bin/emacs | cut -c1-64)"
/usr/bin/emacs -Q --batch --eval '(progn (setq load-path (cons default-directory load-path)) (prin1 (condition-case e (progn (load "flz" nil t) (load "fla" nil t) (load "flb" nil t) (list fla-r flb-r flz-1 flz-2)) (error (list (quote signal) e)))) (terpri))'
