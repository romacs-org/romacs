;;; treesit-query-memo.el --- scaling probe  -*- lexical-binding: t -*-

;; A query compiled again once the process has compiled N others for the
;; same grammar: the compiled-query memo (crates/romacs-vm/src/treesit/
;; query_memo.rs) finds it by its grammar and text, so the N cost nothing.
;; Held by its slope, instructions per query already compiled, not by an
;; exponent: the body is the same 400 hits at every N.  Measured
;; 2026-11-18: 1.8M at N and 4N (about 4,500 instructions a hit), slope
;; -3; the same body without the memo is 4.37G, a compile each time.
(scaling-probe treesit-query-memo :n 100 :slope 50
  :setup (progn (require 'treesit)
                (dotimes (i n)
                  (treesit-query-compile 'c (format "((identifier) @i (#eq? @i \"memo-%d\"))" i) t))
                (treesit-query-compile 'c "((number_literal) @n (#eq? @n \"0\"))" t))
  :body (dotimes (_ 400)
          (treesit-query-compile 'c "((number_literal) @n (#eq? @n \"0\"))" t)))
