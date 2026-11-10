;;; agent-t3-titles-kept.el --- scaling probe  -*- lexical-binding: t -*-

(scaling-probe agent-t3-titles-kept :n 2000 :budget 0.5
  :setup (let ((store (scaling--t3-store n))) (romacs-agent-t3-thread-titles store) store)
  :body (let ((titles (romacs-agent-t3-thread-titles it)))
          (dotimes (i 200) (gethash (romacs-agent-t3-title-key "claudeAgent" (format "n%d" i)) titles))))
