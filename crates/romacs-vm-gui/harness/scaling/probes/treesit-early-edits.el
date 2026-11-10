;;; treesit-early-edits.el --- scaling probe  -*- lexical-binding: t -*-

;; Edits made while it runs, which its tree catches up with: linear in the
;; edits, nothing in the text.
(scaling-probe treesit-early-edits :n 200
  :setup (with-current-buffer (get-buffer-create " scaling-treesit-early-edits")
           (dotimes (i 2000)
             (insert (format "fn f%d(x: u32) -> u32 { let y = x + %d; y * 2 }\n" i i)))
           (romacs--treesit-parse-early 'rust)
           (current-buffer))
  :body (with-current-buffer it
          (dotimes (i n)
            (goto-char (* 40 (1+ i)))
            (insert "x"))
          (treesit-parser-root-node (treesit-parser-create 'rust))))
