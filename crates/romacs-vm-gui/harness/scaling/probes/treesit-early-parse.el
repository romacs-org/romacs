;;; treesit-early-parse.el --- scaling probe  -*- lexical-binding: t -*-

;; The same buffer's first parse on another thread: what reaches the
;; evaluating thread is the tree, so its parser and a hundred more cost
;; nothing in N.
(scaling-probe treesit-early-parse :n 2000 :budget 0.1
  :setup (with-current-buffer (get-buffer-create " scaling-treesit-early")
           (dotimes (i n)
             (insert (format "fn f%d(x: u32) -> u32 { let y = x + %d; y * 2 }\n" i i)))
           (romacs--treesit-parse-early 'rust)
           (current-buffer))
  :body (with-current-buffer it
          (treesit-parser-root-node (treesit-parser-create 'rust))
          (dotimes (i 100)
            (treesit-parser-root-node
             (treesit-parser-create 'rust nil nil (intern (format "twin-%d" i)))))))
