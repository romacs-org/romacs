#!/usr/bin/env bash
# GNU oracle transcript for VM2-AUTOLOAD1 behaviours (tests/autoload_load.rs).
# Usage: oracle.sh DIR > gnu-transcript.txt
set -u
D=${1:?dir}; mkdir -p "$D"
cat > "$D/al-fn.el" <<'EOF'
(defun al-fn (x) (list 'al-fn x))
(provide 'al-fn)
EOF
cat > "$D/al-mac.el" <<'EOF'
(defmacro al-mac (x) (list 'quote (list 'al-mac-expanded x)))
EOF
cat > "$D/al-bad.el" <<'EOF'
(defun al-helper-fresh () 'fresh)
(defun al-existing () 'redefined)
(defalias 'al-aliased (lambda () 'redefined))
(defun al-bad () 'bad)
(provide 'al-bad)
(error "al-bad deliberately fails")
EOF
cat > "$D/al-nodef.el" <<'EOF'
(defun al-other () 1)
EOF
cat > "$D/al-outer.el" <<'EOF'
(setq al-inner-result (condition-case e (al-bad) (error (list 'caught (cadr e)))))
(defun al-outer () (list 'outer al-inner-result))
EOF
cat > "$D/al-match.el" <<'EOF'
(string-match "b+" "xxbbbyy")
(defun al-match () 'm)
EOF
cat > "$D/al-req.el" <<'EOF'
(defun al-existing () 'req-redefined)
(provide 'al-req)
(error "al-req deliberately fails")
EOF
cat > "$D/cases.el" <<EOF
(setq load-path (cons "$D" load-path))
(defmacro case! (name form) \`(princ (format "%s => %S\n" ',name (condition-case e ,form (error (list 'signal e))))))
(autoload 'al-fn "al-fn")
(case! fn-call (list (al-fn 1) (featurep 'al-fn) (car-safe (symbol-function 'al-fn))))
(fset 'al-fn nil) (setq features (delq 'al-fn features))
(autoload 'al-fn "al-fn")
(case! fn-funcall (funcall 'al-fn 2))
(fset 'al-fn nil) (autoload 'al-fn "al-fn")
(case! fn-apply (apply 'al-fn '(3)))
(autoload 'al-mac "al-mac" nil nil t)
(case! mac-eval (eval '(al-mac foo) t))
(fset 'al-mac nil) (autoload 'al-mac "al-mac" nil nil t)
(case! mac-macroexpand (macroexpand '(al-mac bar)))
(fset 'al-mac nil) (autoload 'al-mac "al-mac" nil nil t)
(case! mac-only-on-function (let ((d (symbol-function 'al-fn))) (fset 'al-fn2 '(autoload "al-fn")) (list (autoload-do-load (symbol-function 'al-fn2) 'al-fn2 'macro) (symbol-function 'al-fn2))))
(case! do-load-nonautoload (autoload-do-load 42 'whatever))
(case! do-load-nil-name (progn (fset 'al-fn nil) (autoload 'al-fn "al-fn") (autoload-do-load (symbol-function 'al-fn))))
(case! do-load-name (progn (fset 'al-fn nil) (autoload 'al-fn "al-fn") (autoload-do-load (symbol-function 'al-fn) 'al-fn)))
(case! do-load-bad-name (autoload-do-load '(autoload "al-fn") 42))
(defun al-existing () 'original)
(defalias 'al-aliased (lambda () 'original))
(autoload 'al-bad "al-bad")
(case! failed-load (al-bad))
(case! after-failed (list (symbol-function 'al-bad) (fboundp 'al-helper-fresh) (al-existing) (al-aliased) (featurep 'al-bad)))
(autoload 'al-nodef "al-nodef")
(case! nodef (al-nodef))
(case! nodef-after (list (symbol-function 'al-nodef) (fboundp 'al-other)))
(autoload 'al-missing "al-no-such-file")
(case! missing (al-missing))
(case! missing-after (symbol-function 'al-missing))
(fset 'al-bad nil) (autoload 'al-bad "al-bad") (autoload 'al-outer "al-outer")
(case! nested (al-outer))
(case! nested-after (list (car-safe (symbol-function 'al-bad)) (car-safe (symbol-function 'al-outer))))
(autoload 'al-match "al-match")
(case! match-data (progn (string-match "c" "abcd") (al-match) (match-data)))
(defun al-existing () 'original)
(case! require-fail (require 'al-req))
(case! require-after (list (al-existing) (featurep 'al-req)))
(case! funcall-raw-autoload (funcall '(autoload "al-fn") 1))
EOF
/usr/bin/emacs -Q --batch -l "$D/cases.el" 2>&1 | grep -v "^Warning\|^You can\|^See \|^for more\|^$\|lexical-binding"
# T2 (primitive call request) case
echo "(defun al-test (a b) (equal a b))" > "$D/al-test.el"
/usr/bin/emacs -Q --batch --eval "(progn (setq load-path (cons \"$D\" load-path)) (autoload 'al-test \"al-test\") (princ (format \"t2-assoc-testfn => %S\n\" (list (assoc 2 '((1 . a) (2 . b)) 'al-test) (car-safe (symbol-function 'al-test))))))" 2>&1 | grep "=>"
# A succeeding nested autoload inside a failing one
printf "(defun al-x1 () 'outer-redefined)\n(al-in2)\n(error \"outer fails\")\n" > "$D/al-out2.el"
printf "(defun al-x2 () 'inner-redefined)\n(defun al-in2 () 'in)\n" > "$D/al-in2.el"
/usr/bin/emacs -Q --batch --eval "(progn (setq load-path (cons \"$D\" load-path)) (defun al-x1 () 'original) (defun al-x2 () 'original) (autoload 'al-out2 \"al-out2\") (autoload 'al-in2 \"al-in2\") (princ (format \"nested-ok-inner => %S\n\" (list (condition-case e (al-out2) (error (car e))) (al-x1) (al-x2) (car-safe (symbol-function 'al-in2)) (car-safe (symbol-function 'al-out2))))))" 2>&1 | grep "=>"
# Aliases of macros and macro autoloads; a failed macro autoload then a missing load
printf "(defmacro al-am (x) (list 'quote (list 'expanded x)))\n" > "$D/al-am.el"
printf "(defun al-other () 1)\n" > "$D/al-mnodef.el"
/usr/bin/emacs -Q --batch --eval "(progn (setq load-path (cons \"$D\" load-path)) (defmacro al-m1 (x) (list 'quote (list 'expanded x))) (defalias 'al-a1 'al-m1) (autoload 'al-am \"al-am\" nil nil 'macro) (defalias 'al-alias-am 'al-am) (defalias 'al-alias2 'al-alias-am) (princ (format \"alias => %S\n\" (list (al-a1 foo) (al-alias2 foo) (car-safe (symbol-function 'al-am))))))" 2>&1 | grep "=>"
/usr/bin/emacs -Q --batch --eval "(progn (setq load-path (cons \"$D\" load-path)) (autoload 'al-mnodef \"al-mnodef\" nil nil 'macro) (princ (format \"stale => %S\n\" (list (condition-case e (eval '(al-mnodef) t) (error (car e))) (condition-case e (load \"al-no-such-file\") (error (car e))) (condition-case e (require 'al-no-such-feature) (error (car e)))))))" 2>&1 | grep "=>"
# A macro autoload whose file installs an alias; calls through an alias whose target becomes a macro
printf "(defmacro al-real-m (x) (list 'quote (list 'redirected x)))\n(defalias 'al-redirect-m 'al-real-m)\n" > "$D/al-redirect.el"
printf "(defmacro al-later-m (x) (list 'quote (list 'later x)))\n" > "$D/al-later.el"
/usr/bin/emacs -Q --batch --eval "(progn (setq load-path (cons \"$D\" load-path)) (autoload 'al-redirect-m \"al-redirect\" nil nil 'macro) (princ (format \"redirect => %S\n\" (eval '(al-redirect-m foo) t))) (defun al-target-f (x) x) (defalias 'al-alias-f 'al-target-f) (princ (format \"alias-cache => %S\" (list (eval '(al-alias-f 7) t) (eval '(al-alias-f 7) t)))) (defmacro al-target-f (x) (list 'quote (list 'changed x))) (princ (format \" %S\" (list (eval '(al-alias-f 7) t) (eval '(al-alias-f 8) t)))) (defun al-later-m (x) x) (defalias 'al-alias-l 'al-later-m) (princ (format \" %S\" (eval '(al-alias-l 9) t))) (fset 'al-later-m nil) (autoload 'al-later-m \"al-later\" nil nil 'macro) (princ (format \" %S\n\" (eval '(al-alias-l 9) t))))" 2>&1 | grep "=>"
