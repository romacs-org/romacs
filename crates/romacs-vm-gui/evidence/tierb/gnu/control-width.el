;; GNU 32.0.50 oracle: precision over control characters in a mode line.
;; Run: OUT=file emacs -Q -nw -l control-width.el   (writes after startup, exits)
(run-with-timer 1 nil
 (lambda ()
   (switch-to-buffer "probe")
   (let (out)
     (dolist (f '(("" (-3 "a\tb") "|") ("" (-9 "a\tb") "|") ("" (-10 "a\tb") "|")
                  ("" (-3 "a\001b") "|") ("" (-2 "a\001b") "|") ("a\tb|") ("a\001b|")
                  ;; widths select, characters count: n advances by SCHARS
                  ("BEGIN:" (-10 ("a\t" "bcd")) ":END") ("" (-4 ("a\t" "bcd")) "|")
                  ("" (5 "a\t") "|") ("" (-3 "日本語x") "|") ("" (4 "日") "|")
                  ("" (-4 ("日本" "xyz")) "|") ("" (-3 ("%b")) "|") ("" (8 "%b") "|")))
       (push (format "%S => %S" f (substring-no-properties (format-mode-line f))) out))
     (setq mode-line-format '("" (-3 "a\tb") "|"))
     (redisplay t)
     (push (format "displayed mode line for %S: %S" mode-line-format
                   (save-window-excursion
                     (let ((s (format-mode-line mode-line-format)))
                       (substring-no-properties s))))
           out)
     (with-temp-file (getenv "OUT")
       (insert "GNU " emacs-version " tab-width=" (number-to-string tab-width) "\n"
               (mapconcat #'identity (nreverse out) "\n") "\n")))
   (kill-emacs 0)))
