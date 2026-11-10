;;; treesit-parse-edit.el --- scaling probe  -*- lexical-binding: t -*-

;; A Rust buffer of N lines parsed by the bundled grammar, then twenty
;; edits, each forcing a reparse and a capture over its line: the parse is
;; linear in the text, an edit's reparse and capture are not in N.
(scaling-probe treesit-parse-edit :n 2000
  :setup (with-current-buffer (get-buffer-create " scaling-treesit")
           (dotimes (i n)
             (insert (format "fn f%d(x: u32) -> u32 { let y = x + %d; y * 2 }\n" i i)))
           (current-buffer))
  :body (with-current-buffer it
          (let ((parser (treesit-parser-create 'rust)))
            (treesit-parser-root-node parser)
            (goto-char (/ (point-max) 2))
            (dotimes (_ 20)
              (insert "x")
              (treesit-query-capture (treesit-parser-root-node parser) "(identifier) @i"
                                     (line-beginning-position) (line-end-position))))))
