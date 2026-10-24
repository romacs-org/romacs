;; GNU 32.0.50 oracle: mode-line widths that do not fit a C `int`.
;; display_mode_element keeps `%NNN' widths and integer-construct limits in
;; `int' (xdisp.c:28663 `field = field * 10 + c - '0'', xdisp.c:28879
;; `int lim = XFIXNUM (car)'), so huge values wrap modulo 2^32.
;; Run: OUT=file emacs -Q -nw -l field-wrap.el   (writes after startup, exits)
(run-with-timer 1 nil
 (lambda ()
   (switch-to-buffer "probe")
   (insert "line 1\nline 2\nline 3\n") (goto-char (point-min)) (forward-line 2)
   (redisplay t)
   (let (out)
     (dolist (f '(("%999999999999999999999999999999b|") ("%4294967297b|") ("%2147483648b|")
                  ("%9999999999999999999l|")
                  ((2305843009213693951 "x") "|") ((-2305843009213693951 "abcdefgh") "|")
                  ((4294967297 "abc") "|") ((-4294967295 "abc") "|") ((4294967295 "abc") "|")
                  ((-2305843009213693951 "x") (2305843009213693951 "y") "z")))
       (let ((s (format-mode-line f)))
         (push (format "%S len=%d head=%S tail=%S" f (length s)
                       (substring-no-properties s 0 (min 12 (length s)))
                       (substring-no-properties s (max 0 (- (length s) 4))))
               out)))
     (with-temp-file (getenv "OUT")
       (insert "GNU " emacs-version "\n" (mapconcat #'identity (nreverse out) "\n") "\n")))
   (kill-emacs 0)))
