;;; treesit-twin-parser.el --- scaling probe  -*- lexical-binding: t -*-

;; The same buffer parsed, then a hundred more parsers of it, each like
;; c-ts-mode's `for-each' one: a first parse copies the tree, so none costs
;; anything in N.
(scaling-probe treesit-twin-parser :n 2000 :budget 0.1
  :setup (with-current-buffer (get-buffer-create " scaling-treesit-twin")
           (dotimes (i n)
             (insert (format "fn f%d(x: u32) -> u32 { let y = x + %d; y * 2 }\n" i i)))
           (treesit-parser-root-node (treesit-parser-create 'rust))
           (current-buffer))
  :body (with-current-buffer it
          (dotimes (i 100)
            (treesit-parser-root-node
             (treesit-parser-create 'rust nil nil (intern (format "twin-%d" i)))))))
