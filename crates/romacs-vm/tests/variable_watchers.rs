//! data.c notify_variable_watchers with Lisp watch functions against pinned
//! GNU: OPERATION set, let, unlet, makunbound and defvaralias, the WHERE
//! buffer, watchers running before the write, a signalling or throwing
//! watcher, watchers that run arbitrary Lisp, and aliases. Then specbind's
//! order (the binding is recorded before its watchers run), an unlet
//! watcher's signal aborting the restore, eval.c's dispatch of plain,
//! localized and forwarded bindings (`set' for a default binding, two
//! notifications for a global forwarded one), one handler-bind call per
//! watcher signal on every write path, a watcher removed by the one before
//! it, and defvar, defconst and the toplevel-value setters. Last, the
//! variables `load' binds, and an unlet watcher's signal after a load.
use romacs_vm::Vm;

/// One expression per line. LOG is a watcher recording its arguments, with
/// WHERE as a buffer name, and the value the variable had when it ran.
const CASES: &str = r###"
(let ((log nil) (w (lambda (&rest a) (setq log (cons (car a) log))))) (add-variable-watcher 'current-load-list w) (let ((current-load-list nil)) (setq log nil) (defvar wv-load 1) (defvaralias 'wv-load-alias 'wv-load) (defalias 'wv-load-f #'car) (provide 'wv-load-feature) (setq current-load-list nil)) (remove-variable-watcher 'current-load-list w) log)
(mapcar (lambda (s) (list s (condition-case e (add-variable-watcher s #'ignore) (error e)) (condition-case e (remove-variable-watcher s #'ignore) (error e)) (get-variable-watchers s))) '(nil t :kw most-positive-fixnum))
(progn (defvar wv-a 1) (setq log nil) (add-variable-watcher 'wv-a LOG) (setq wv-a 2) (let ((wv-a 3)) (setq wv-a 4)) (set 'wv-a 5) (makunbound 'wv-a) (list (nreverse log) (boundp 'wv-a) (length (get-variable-watchers 'wv-a))))
(progn (defvar wv-b 1) (make-variable-buffer-local 'wv-b) (defvar wv-c 1) (setq log nil) (add-variable-watcher 'wv-b LOG) (add-variable-watcher 'wv-c LOG) (with-temp-buffer (setq wv-b 2) (set-default 'wv-b 3) (kill-local-variable 'wv-b) (make-local-variable 'wv-c) (setq wv-c 4) (set-default 'wv-c 5) (kill-local-variable 'wv-c) (setq wv-c 6)) (list (nreverse log) wv-b wv-c))
(progn (defvar wv-d 1) (defvar wv-e 7) (setq log nil) (add-variable-watcher 'wv-d LOG) (defvaralias 'wv-d 'wv-e) (setq wv-e 8) (list (nreverse log) wv-d (indirect-variable 'wv-d)))
(progn (defvar wv-f 1) (add-variable-watcher 'wv-f (lambda (s n o w) (if (eq n 'bad) (error "No %s" n)))) (list (condition-case e (setq wv-f 'bad) (error e)) wv-f (condition-case e (let ((wv-f 'bad)) wv-f) (error e)) wv-f (progn (setq wv-f 2) wv-f)))
(progn (defvar wv-g 1) (add-variable-watcher 'wv-g (lambda (s n o w) (if (eq n 'jump) (throw 'out (list 'thrown wv-g))))) (list (catch 'out (setq wv-g 'jump) 'not-thrown) wv-g))
(progn (defvar wv-h 0) (setq calls 0) (add-variable-watcher 'wv-h (lambda (s n o w) (setq calls (1+ calls)) (setq wv-h (* 10 n)) (funcall (lambda (x) (list x (eval '(let ((y x)) (* y 2)) t))) n))) (list (progn (setq wv-h 3) wv-h) calls (let ((wv-h 4)) wv-h) wv-h calls))
(progn (defvar wv-mode t) (defvar wv-total nil) (setq disabled nil) (fset 'wv-disable (lambda () (setq wv-mode nil) (setq disabled t))) (let ((w (lambda (_s _v op _w) (when (and (eq op 'set) wv-mode (not disabled)) (wv-disable))))) (add-variable-watcher 'wv-total w) (setq wv-total 5) (remove-variable-watcher 'wv-total w) (setq wv-total 6) (list disabled wv-mode wv-total (get-variable-watchers 'wv-total))))
(progn (defvar wv-i 1) (defvaralias 'wv-j 'wv-i) (setq log nil) (add-variable-watcher 'wv-j LOG) (setq wv-i 2) (setq wv-j 3) (list (nreverse log) (length (get-variable-watchers 'wv-i)) (equal (get-variable-watchers 'wv-i) (get-variable-watchers 'wv-j))))
(progn (defvar wv-k 1) (add-variable-watcher 'wv-k (lambda (s n o w) (if (eq o 'let) (set s 99)))) (list (let ((wv-k 2)) wv-k) wv-k))
(progn (defvar wv-l 1) (add-variable-watcher 'wv-l (lambda (s n o w) (if (eq o 'unlet) (error "No unlet")))) (list (condition-case e (let ((wv-l 2)) 'body) (error (list e wv-l))) wv-l))
(progn (setq log nil) (fset 'wv-log LOG) (add-variable-watcher 'fill-column 'wv-log) (with-temp-buffer (rename-buffer "wv-1") (let ((fill-column 33)) fill-column) (setq fill-column 44) (let ((fill-column 55)) (kill-local-variable 'fill-column)) (setq fill-column 66) (let ((fill-column 77)) fill-column) (kill-local-variable 'fill-column) (set-default 'fill-column 70)) (list (nreverse log) (default-value 'fill-column)))
(progn (setq log nil) (fset 'wv-log LOG) (add-variable-watcher 'inhibit-field-text-motion 'wv-log) (let ((inhibit-field-text-motion 1)) inhibit-field-text-motion) (set-default 'inhibit-field-text-motion 2) (setq inhibit-field-text-motion 3) (list (nreverse log) inhibit-field-text-motion))
(progn (defvar wv-o 1) (make-variable-buffer-local 'wv-o) (defvar wv-p 1) (setq log nil) (fset 'wv-log LOG) (add-variable-watcher 'wv-o 'wv-log) (add-variable-watcher 'wv-p 'wv-log) (with-temp-buffer (rename-buffer "wv-2") (let ((wv-o 2)) wv-o) (setq wv-o 3) (let ((wv-o 4)) (kill-local-variable 'wv-o)) (let ((wv-p 5)) (make-local-variable 'wv-p) (setq wv-p 6)) (kill-local-variable 'wv-p) (make-local-variable 'wv-p) (let ((wv-p 7)) wv-p) (set-default 'wv-p 8)) (list (nreverse log) wv-o wv-p))
(progn (setq hb nil) (defvar wv-m 1) (add-variable-watcher 'wv-m (lambda (_s n o _w) (if (memq o '(set let unlet defvaralias)) (error "No %s" o)))) (add-variable-watcher 'wv-x (lambda (&rest _) (error "No defvar"))) (fset 'wv-hb (lambda (f) (condition-case e (handler-bind-1 f '(error) (lambda (e) (setq hb (cons (car (cdr e)) hb)))) (error (car (cdr e)))))) (list (wv-hb (lambda () (set-default 'wv-m 2))) (wv-hb (lambda () (setq wv-m 3))) (wv-hb (lambda () (let ((wv-m 4)) wv-m))) (wv-hb (lambda () (defvaralias 'wv-m 'wv-q))) (wv-hb (lambda () (defvar wv-x 5))) (nreverse hb) wv-m))
(progn (defvar wv-n 1) (setq log nil) (fset 'wv-b (lambda (&rest _) (setq log (cons 'b log)))) (fset 'wv-a (lambda (&rest _) (setq log (cons 'a log)) (remove-variable-watcher 'wv-n 'wv-b))) (add-variable-watcher 'wv-n 'wv-b) (add-variable-watcher 'wv-n 'wv-a) (setq wv-n 2) (setq wv-n 3) (list (nreverse log) (get-variable-watchers 'wv-n)))
(progn (setq log nil) (fset 'wv-log LOG) (add-variable-watcher 'wv-r 'wv-log) (add-variable-watcher 'wv-s 'wv-log) (defvar wv-r 5) (defvar wv-r 6) (defconst wv-s 7) (set-default 'wv-r 8) (list (nreverse log) wv-r wv-s))
(progn (setq log nil) (defvar wv-t 1) (fset 'wv-log LOG) (add-variable-watcher 'wv-t 'wv-log) (let ((wv-t 2)) (set-default-toplevel-value 'wv-t 3) (list wv-t (default-toplevel-value 'wv-t))) (set-default-toplevel-value 'wv-t 4) (list (nreverse log) wv-t))
(progn (setq log nil) (defvar wv-u 1) (make-variable-buffer-local 'wv-u) (fset 'wv-log LOG) (add-variable-watcher 'wv-u 'wv-log) (with-temp-buffer (rename-buffer "wv-3") (setq wv-u 2) (let ((wv-u 3)) (set-buffer-local-toplevel-value 'wv-u 4)) (set-buffer-local-toplevel-value 'wv-u 5)) (list (nreverse log) wv-u))
(progn (setq log nil) (fset 'wv-log LOG) (add-variable-watcher 'fill-column 'wv-log) (with-temp-buffer (rename-buffer "wv-4") (let ((fill-column 40)) (setq fill-column 41) fill-column)) (list (nreverse log) (condition-case e (makunbound 'fill-column) (error e)) (default-value 'fill-column)))
(progn (setq log nil) (fset 'wv-log LOG) (add-variable-watcher 'wv-v 'wv-log) (let ((wv-v 1)) wv-v) (list (nreverse log) (boundp 'wv-v)))
(progn (setq log nil) (defvar wv-w 1) (fset 'wv-log LOG) (add-variable-watcher 'wv-w 'wv-log) (with-temp-buffer (rename-buffer "wv-5") (make-local-variable 'wv-w) (setq wv-w 2) (let ((wv-w 3)) (kill-local-variable 'wv-w) (make-local-variable 'wv-w) (setq wv-w 9))) (list (nreverse log) wv-w))
(progn (defvar wv-y 1) (defvar wv-yz 0) (with-temp-buffer (make-local-variable 'wv-y) (setq wv-y 2) (add-variable-watcher 'wv-y (lambda (s n o w) (when (eq o 'unlet) (kill-local-variable 'wv-y) (garbage-collect) (make-local-variable 'wv-yz) (setq wv-yz 'sentinel)))) (list (let ((wv-y 3)) wv-y) wv-y wv-yz (local-variable-p 'wv-y) (default-value 'wv-y))))
(progn (defvar wv-x2 1) (with-temp-buffer (make-local-variable 'wv-x2) (setq wv-x2 2) (add-variable-watcher 'wv-x2 (lambda (s n o w) (when (eq o 'unlet) (kill-local-variable 'wv-x2) (garbage-collect) (make-local-variable 'wv-x2) (setq wv-x2 9)))) (list (let ((wv-x2 3)) wv-x2) wv-x2 (default-value 'wv-x2))))
(progn (setq log nil) (fset 'wv-log LOG) (add-variable-watcher 'last-command-event 'wv-log) (with-temp-buffer (let ((last-command-event ?a)) (self-insert-command 1 ?b) (setq log (nreverse log)) (list log last-command-event (buffer-string)))))
(let ((current-load-list nil)) (setq log nil) (fset 'wv-log LOG) (add-variable-watcher 'current-load-list 'wv-log) (defalias 'wv-df #'ignore) (list log (length current-load-list)))
(progn (setq log nil) (fset 'wv-log LOG) (defvar wv-hk nil) (put 'wv-hk 'permanent-local 'permanent-local-hook) (put 'wv-keep 'permanent-local-hook t) (with-temp-buffer (rename-buffer "wv-hb") (set (make-local-variable 'wv-hk) (list 'wv-keep 'wv-drop t)) (add-variable-watcher 'wv-hk 'wv-log) (kill-all-local-variables) (list wv-hk (nreverse log))))
(progn (setq log nil) (fset 'wv-log LOG) (defvar wv-hl '(global)) (put 'wv-hl 'permanent-local 'permanent-local-hook) (with-temp-buffer (rename-buffer "wv-hb") (set (make-local-variable 'wv-hl) nil) (add-variable-watcher 'wv-hl 'wv-log) (kill-all-local-variables) (list wv-hl (nreverse log))))
(progn (setq log nil) (fset 'wv-log LOG) (with-temp-buffer (rename-buffer "wv-hb") (mapc (lambda (s) (add-variable-watcher s 'wv-log) (make-local-variable s)) '(abbrev-mode auto-fill-function bidi-display-reordering bidi-paragraph-direction bidi-paragraph-separate-re bidi-paragraph-start-re buffer-auto-save-file-format buffer-auto-save-file-name buffer-backed-up buffer-display-count buffer-display-table buffer-display-time buffer-file-coding-system buffer-file-format buffer-file-name buffer-file-truename buffer-invisibility-spec buffer-read-only buffer-saved-size buffer-undo-list cache-long-scans ctl-arrow cursor-in-non-selected-windows cursor-type default-directory fill-column fringe-cursor-alist fringe-indicator-alist fringes-outside-margins header-line-format horizontal-scroll-bar indicate-buffer-boundaries indicate-empty-lines left-fringe-width left-margin left-margin-width line-spacing local-abbrev-table local-minor-modes major-mode mark-active mode-line-format mode-name overwrite-mode point-before-scroll right-fringe-width right-margin-width scroll-bar-height scroll-bar-width scroll-down-aggressively scroll-up-aggressively selective-display selective-display-ellipses tab-line-format tab-width text-conversion-style truncate-lines vertical-scroll-bar word-wrap)) (kill-all-local-variables) log))
(progn (setq log nil) (fset 'wv-log LOG) (with-temp-buffer (rename-buffer "wv-hb") (add-variable-watcher 'case-symbols-as-words 'wv-log) (set (make-local-variable 'case-symbols-as-words) t) (kill-all-local-variables) (list case-symbols-as-words (nreverse log))))
(progn (setq log nil) (fset 'wv-log LOG) (defvar wv-sd 1) (add-variable-watcher 'wv-sd 'wv-log) (list (set-default 'wv-sd (list 1 2 3)) (default-value 'wv-sd) (nreverse log)))
(progn (setq log nil) (defvar wv-sc 1) (add-variable-watcher 'wv-sc (lambda (s n o w) (setq log (cons (list (consp (garbage-collect)) (default-value s)) log)))) (list (set-default 'wv-sc (list 'a (make-string 3 ?x))) (default-value 'wv-sc) log))
(progn (defvar wv-sd2 1) (add-variable-watcher 'wv-sd2 (lambda (s n o w) (if (eq n 'bad) (error "No %s" n)))) (list (condition-case e (set-default 'wv-sd2 'bad) (error e)) (default-value 'wv-sd2)))
(progn (defvar wv-sd3 1) (add-variable-watcher 'wv-sd3 (lambda (s n o w) (throw 'out (list 'thrown n (default-value s))))) (list (catch 'out (set-default 'wv-sd3 'jump)) (default-value 'wv-sd3)))
(progn (defvar wv-new 1) (defvar wv-base 2) (add-variable-watcher 'wv-new (lambda (s n o w) (if (eq o 'defvaralias) (error "No alias %s" n)))) (list (condition-case e (defvaralias 'wv-new 'wv-base) (error e)) (indirect-variable 'wv-new) wv-new))
(progn (defvar wv-new2 1) (defvar wv-base2 2) (add-variable-watcher 'wv-new2 (lambda (s n o w) (throw 'out (list 'thrown n o)))) (list (catch 'out (defvaralias 'wv-new2 'wv-base2)) (indirect-variable 'wv-new2)))
(progn (setq log nil) (fset 'wv-log LOG) (defvar wv-new3 1) (defvar wv-base3 2) (add-variable-watcher 'wv-new3 'wv-log) (list (defvaralias 'wv-new3 'wv-base3) (indirect-variable 'wv-new3) wv-new3 (nreverse log)))
(progn (setq log nil) (fset 'wv-log LOG) (defvar wv-bn 5) (add-variable-watcher 'wv-bb 'wv-log) (defvaralias 'wv-bn 'wv-bb) (list wv-bb wv-bn (nreverse log)))
(progn (setq log nil) (defvar wv-fb 5) (fset 'wv-log LOG) (add-variable-watcher 'wv-fbase 'wv-log) (let ((wv-fb 6)) (list (condition-case e (defvaralias 'wv-fb 'wv-fbase) (error (car e))) (boundp 'wv-fbase) (nreverse log))))
(progn (setq log nil) (defvar wv-kl 1) (with-temp-buffer (rename-buffer "wv-kb") (set (make-local-variable 'wv-kl) 2) (add-variable-watcher 'wv-kl (lambda (s n o w) (setq log (cons (list n o (buffer-name w) (symbol-value s) (local-variable-p s)) log)))) (list (kill-local-variable 'wv-kl) (local-variable-p 'wv-kl) wv-kl log)))
(progn (defvar wv-kl2 1) (with-temp-buffer (set (make-local-variable 'wv-kl2) 2) (add-variable-watcher 'wv-kl2 (lambda (s n o w) (error "No kill"))) (list (condition-case e (kill-local-variable 'wv-kl2) (error e)) (local-variable-p 'wv-kl2) wv-kl2)))
(progn (defvar wv-kl3 1) (with-temp-buffer (set (make-local-variable 'wv-kl3) 2) (add-variable-watcher 'wv-kl3 (lambda (s n o w) (throw 'out (list 'thrown o)))) (list (catch 'out (kill-local-variable 'wv-kl3)) (local-variable-p 'wv-kl3) wv-kl3)))
(progn (setq log nil) (with-temp-buffer (let ((u (make-symbol "wv-u"))) (set (make-local-variable u) 2) (add-variable-watcher u (lambda (s n o w) (setq log (cons (consp (garbage-collect)) log)))) (list (symbol-name (kill-local-variable (prog1 u (setq u nil)))) log))))
(progn (setq log nil) (defvar wv-na 0) (defvar wv-nb 0) (add-variable-watcher 'wv-nb (lambda (s n o w) (setq log (cons (list 'b n (default-value 'wv-na) (default-value 'wv-nb)) log)))) (add-variable-watcher 'wv-na (lambda (s n o w) (setq log (cons (list 'a-in n) log)) (set-default 'wv-nb n) (setq log (cons (list 'a-out n (default-value 'wv-nb)) log)))) (set-default 'wv-na 1) (list (default-value 'wv-na) (nreverse log)))
(progn (setq log nil) (defvar wv-sb 0) (with-temp-buffer (rename-buffer "wv-s") (let ((other (get-buffer-create "wv-o"))) (add-variable-watcher 'wv-sb (lambda (s n o w) (set-buffer other))) (set-default 'wv-sb 1) (buffer-name))))
(progn (setq log nil) (fset 'wv-log LOG) (with-temp-buffer (rename-buffer "wv-db") (add-variable-watcher 'deactivate-mark 'wv-log) (setq deactivate-mark nil) (setq log nil) (insert "x") (let ((seen deactivate-mark)) (list seen (nreverse log)))))
(progn (setq log nil) (with-temp-buffer (rename-buffer "wv-src") (let ((dst (get-buffer-create "wv-dst"))) (add-variable-watcher 'deactivate-mark (lambda (s n o w) (setq log (cons (list n o (if (bufferp w) (buffer-name w) w) (buffer-name)) log)))) (prin1 'x dst) (list (buffer-name) deactivate-mark (buffer-local-value 'deactivate-mark dst) (with-current-buffer dst (buffer-string)) (nreverse log)))))
(progn (setq log nil) (fset 'wv-log LOG) (defvar wv-fn 1) (add-variable-watcher 'wv-fn 'wv-log) (list (funcall 'set-default 'wv-fn 2) (apply 'set-default '(wv-fn 3)) (let ((seen (default-value 'wv-fn))) seen) (nreverse log)))
(progn (setq log nil) (fset 'wv-log LOG) (with-temp-buffer (rename-buffer "wv-tp") (insert "abc") (add-variable-watcher 'deactivate-mark 'wv-log) (setq deactivate-mark nil) (setq log nil) (put-text-property 1 2 'face 'bold) (let ((seen deactivate-mark)) (list seen (nreverse log)))))
(progn (setq log nil) (defvar wv-sb2 0) (with-temp-buffer (rename-buffer "wv-p") (add-variable-watcher 'wv-sb2 (lambda (s n o w) (setq log (cons (list n (buffer-name)) log)))) (let ((v (with-temp-buffer (rename-buffer "wv-q") (set-default 'wv-sb2 1) (buffer-name)))) (list v (buffer-name) log))))
(progn (defvar wv-ka 0) (defvar wv-kb2 0) (let ((orig (get-buffer-create "wv-orig")) (other (get-buffer-create "wv-other"))) (with-current-buffer other (set (make-local-variable 'wv-kb2) 22)) (with-current-buffer orig (set (make-local-variable 'wv-ka) 1) (set (make-local-variable 'wv-kb2) 2) (fset 'wv-killer (lambda (s n o w) (if (buffer-live-p orig) (progn (set-buffer other) (kill-buffer orig))))) (add-variable-watcher 'wv-ka 'wv-killer) (add-variable-watcher 'wv-kb2 'wv-killer) (kill-all-local-variables)) (list (buffer-live-p orig) (local-variable-p 'wv-kb2 other) (with-current-buffer other wv-kb2) (default-value 'wv-kb2))))
(progn (defvar wv-la 0) (defvar wv-lb 0) (let ((orig (get-buffer-create "wv-orig")) (other (get-buffer-create "wv-other"))) (with-current-buffer other (set (make-local-variable 'wv-lb) 22)) (with-current-buffer orig (set (make-local-variable 'wv-la) 1) (set (make-local-variable 'wv-lb) 2) (fset 'wv-mover (lambda (s n o w) (set-buffer other))) (add-variable-watcher 'wv-la 'wv-mover) (add-variable-watcher 'wv-lb 'wv-mover) (kill-all-local-variables)) (list (local-variable-p 'wv-lb orig) (local-variable-p 'wv-la orig) (local-variable-p 'wv-lb other) (with-current-buffer other wv-lb))))
"###;

/// Writes GNU's C makes by assignment (`Vname = value`), which no watcher
/// hears, reached through the real coding systems and commands.
const LOADED_CASES: &str = r###"
(progn (setq log nil) (fset 'wv-log LOG) (add-variable-watcher 'last-coding-system-used 'wv-log) (decode-coding-string "abc" 'utf-8) (encode-coding-string "abc" 'utf-8) (list log last-coding-system-used))
(progn (setq log nil) (fset 'wv-log LOG) (add-variable-watcher 'coding-category-list 'wv-log) (set-coding-system-priority 'utf-8) (list log (car (coding-system-priority-list))))
(progn (setq log nil) (fset 'wv-log LOG) (add-variable-watcher 'this-command 'wv-log) (add-variable-watcher 'real-this-command 'wv-log) (let ((this-command 'wv-outer)) (setq log nil) (call-interactively 'ignore) (list log this-command)))
(progn (setq log nil) (fset 'wv-log LOG) (add-variable-watcher 'coding-category-utf-8 'wv-log) (set-coding-system-priority 'utf-8) (list (mapcar (lambda (e) (list (nth 0 e) (nth 2 e))) log) (car (coding-system-priority-list))))
(progn (setq log nil) (fset 'wv-log LOG) (add-variable-watcher 'buffer-file-coding-system 'wv-log) (with-temp-buffer (rename-buffer "wv-fc") (let ((coding-system-for-read 'utf-8)) (insert-file-contents "/etc/passwd")) (list buffer-file-coding-system (mapcar (lambda (e) (list (nth 0 e) (nth 1 e) (nth 2 e) (nth 3 e))) (nreverse log)))))
(progn (setq log nil) (fset 'wv-log LOG) (add-variable-watcher 'wv-state 'wv-log) (setq wv-state nil) (setq log nil) (frame-or-buffer-changed-p 'wv-state) (list (mapcar (lambda (e) (list (nth 0 e) (vectorp (nth 1 e)) (nth 2 e))) log) (vectorp wv-state)))
(progn (setq log nil) (fset 'wv-cmd (lambda () (interactive) (setq log (cons (list 'command (length command-history)) log)))) (add-variable-watcher 'command-history (lambda (s n o w) (setq log (cons (list 'history o (length n) (length command-history)) log)))) (let ((command-history nil)) (setq log nil) (call-interactively 'wv-cmd t) (list (nreverse log) (length command-history))))
"###;

const LOG: &str = "(lambda (s n o w) (setq log (cons (list s n o (if (bufferp w) (buffer-name w) w) (if (boundp s) (symbol-value s) 'void)) log)))";

/// Cases that load FILE, whose source is the first element. GNU's load
/// binds (lread.c Fload, mule.el load-with-code-conversion, readevalloop)
/// and restores each variable as a `let' does, keeps load-in-progress bound
/// over the after-load forms, and propagates an unlet watcher's signal from
/// a load that otherwise succeeded, leaving that binding installed; through
/// `require', the failed load's feature is undone. A `(defvar NAME)' records NAME
/// in `macroexp--dynvars' with a `set' its watchers hear. That variable is forwarded
/// in GNU, so restoring its binding reports `set' as well as `unlet', which Romacs's
/// does not; the case reports only the declaration's write.
const LOAD_CASES: &[(&str, &str)] = &[
    (";;; -*- lexical-binding: t -*-\n(setq wv-in (list load-file-name load-true-file-name load-in-progress current-load-list))\n",
     "(progn (setq log nil) (setq wv-outer current-load-list) (fset 'wv-log (lambda (s n o w) (setq log (cons (list s o (cond ((equal n FILE) 'file) ((equal n (list FILE)) '(file)) ((and (eq s 'current-load-list) (eq n wv-outer)) 'outer) (t n)) w) log)))) (dolist (v '(load-file-name load-true-file-name load-in-progress current-load-list)) (add-variable-watcher v 'wv-log)) (fset 'do-after-load-evaluation (lambda (f) (setq wv-after (list (equal f FILE) load-in-progress load-file-name)))) (list (load FILE nil t) (nreverse log) (equal wv-in (list FILE FILE t (list FILE))) wv-after load-file-name load-true-file-name load-in-progress (eq current-load-list wv-outer)))"),
    (";;; -*- lexical-binding: t -*-\n(setq wv-z 1)\n",
     "(progn (fset 'wv-cll (lambda (s n o w) (if (eq o 'unlet) (error \"No unlet\")))) (add-variable-watcher 'current-load-list 'wv-cll) (list (condition-case e (load FILE nil t) (error e)) (progn (remove-variable-watcher 'current-load-list 'wv-cll) current-load-list) load-file-name load-true-file-name load-in-progress wv-z))"),
    (";;; -*- lexical-binding: t -*-\n(defvar wv-declared)\n(setq wv-after t)\n",
     "(progn (setq log nil) (fset 'wv-dyn (lambda (s n o w) (if (and (eq o 'set) n) (setq log (cons (list s (car n) o) log))))) (add-variable-watcher 'macroexp--dynvars 'wv-dyn) (load FILE nil t) log)"),
    (";;; -*- lexical-binding: t -*-\n(provide 'wv-feature)\n",
     "(progn (fset 'wv-cll (lambda (s n o w) (if (eq o 'unlet) (error \"No unlet\")))) (add-variable-watcher 'current-load-list 'wv-cll) (list (condition-case e (require 'wv-feature FILE) (error e)) (progn (remove-variable-watcher 'current-load-list 'wv-cll) (featurep 'wv-feature)) load-file-name load-in-progress))"),
];

fn prelude() -> Vm {
    let mut vm = Vm::new();
    vm.install_lisp_prelude().unwrap();
    vm
}

/// The loadup chain (process.rs's), for coding systems and commands.
fn bootstrapped() -> Vm {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let mut vm = Vm::new();
    vm.eval_source("(setq noninteractive t)").unwrap();
    let lisp = root.join("gnu-lisp");
    let mut names: Vec<String> = std::fs::read_dir(&lisp).unwrap()
        .filter_map(|entry| entry.ok().filter(|e| e.path().is_dir()).and_then(|e| e.file_name().into_string().ok()))
        .filter(|name| !["term", "RCS", "CVS", "Old"].contains(&name.as_str()) && !name.starts_with('='))
        .collect();
    names.sort();
    let mut paths = vec![lisp.clone()];
    paths.extend(names.iter().rev().filter(|n| *n != "obsolete").map(|n| lisp.join(n)));
    paths.push(root.join("rlisp"));
    vm.set_load_path(&paths).unwrap();
    vm.install_lisp_prelude().unwrap();
    vm.load_file(&root.join("rlisp/romacs-loadup-bootstrap.el")).unwrap();
    vm
}

/// EXPRESSION's printed value from pinned GNU and from Romacs, as a failure
/// report when they differ.
fn compare(binary: &str, expression: &str, mut vm: Vm) -> Option<String> {
    // As character codes, as tests/json.rs compares.
    let source = format!("(append (prin1-to-string (condition-case e {expression} (error (list 'signal e)))) nil)");
    let output = std::process::Command::new(binary).args(["-Q", "--batch", "--eval", &format!("(prin1 {source})")]).output().unwrap();
    assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
    let expected = String::from_utf8(output.stdout).unwrap();
    // Collect at every safepoint, so a watcher that ran while collection
    // was inhibited is followed by collections that would expose an
    // unforwarded register.
    vm.set_collection_floor(1);
    let actual = vm.eval_source(&source).and_then(|v| v.display());
    (actual.as_deref() != Ok(expected.as_str())).then(|| {
        let text = |codes: &str| codes.trim_matches(|c| c == '(' || c == ')').split_whitespace()
            .filter_map(|n| n.parse::<u32>().ok().and_then(char::from_u32)).collect::<String>();
        format!("{expression}\n  GNU    {}\n  Romacs {:?}", text(&expected), actual.as_deref().map(text))
    })
}

#[test]
fn lisp_watchers_match_pinned_gnu() {
    let binary = std::env::var("ROMACS_GNU_ORACLE").expect("pinned GNU oracle");
    let failures: Vec<_> = CASES.lines().filter(|line| !line.is_empty())
        .filter_map(|expression| compare(&binary, &expression.replace("LOG", LOG), prelude())).collect();
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

/// A primitive's queued write (`watchers.rs`) differs from GNU's in place in
/// one way Lisp can see: the rest of the primitive has run when its watcher
/// does, so what it did stands if the watcher signals. DIVERGENCES VM2-WATCH1.
#[test]
fn a_queued_write_runs_after_the_rest_of_its_primitive() {
    let mut vm = prelude();
    vm.set_collection_floor(1);
    let cases = [
        // The text GNU has not inserted yet when `deactivate-mark' is set.
        (r#"(with-temp-buffer (add-variable-watcher 'deactivate-mark (lambda (_s _n _o _w) (error "No"))) (list (condition-case e (insert "x") (error (car (cdr e)))) (buffer-string)))"#,
         r#"("No" "x")"#),
        // The docstring, set after the alias GNU's watcher would have stopped.
        (r#"(progn (defvar wv-d1 1) (defvar wv-d2 2) (add-variable-watcher 'wv-d1 (lambda (_s _n _o _w) (error "No"))) (list (condition-case e (defvaralias 'wv-d1 'wv-d2 "Doc") (error (car (cdr e)))) (indirect-variable 'wv-d1) (get 'wv-d1 'variable-documentation)))"#,
         r#"("No" wv-d1 "Doc")"#),
    ];
    for (source, expected) in cases {
        assert_eq!(vm.eval_source(source).and_then(|v| v.display()).as_deref(), Ok(expected), "{source}");
    }
}

#[test]
fn native_assignments_match_pinned_gnu() {
    let binary = std::env::var("ROMACS_GNU_ORACLE").expect("pinned GNU oracle");
    let failures: Vec<_> = LOADED_CASES.lines().filter(|line| !line.is_empty())
        .filter_map(|expression| compare(&binary, &expression.replace("LOG", LOG), bootstrapped())).collect();
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

#[test]
fn load_bindings_match_pinned_gnu() {
    let binary = std::env::var("ROMACS_GNU_ORACLE").expect("pinned GNU oracle");
    let mut failures = Vec::new();
    for (i, (contents, expression)) in LOAD_CASES.iter().enumerate() {
        let file = std::env::temp_dir().join(format!("romacs-watchers-{}-{i}.el", std::process::id()));
        std::fs::write(&file, contents).unwrap();
        let name = format!("{:?}", file.to_str().unwrap());
        failures.extend(compare(&binary, &expression.replace("FILE", &name), prelude()));
        std::fs::remove_file(&file).unwrap();
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}
