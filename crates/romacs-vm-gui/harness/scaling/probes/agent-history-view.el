;;; agent-history-view.el --- scaling probe  -*- lexical-binding: t -*-

;; A transcript of N turns (a prompt, a tool call and its result, a reply), viewed.
(scaling-probe agent-history-view :n 50
  :setup (let ((file (make-temp-file "scaling-transcript-" nil ".jsonl")))
           (scaling--agent)
           (require 'romacs-agent-history)
           (romacs-agent-ui--helpers)
           (with-temp-file file
             (dotimes (i n)
               (insert (format "{\"type\":\"user\",\"cwd\":\"/tmp\",\"timestamp\":\"2026-11-03T10:00:00Z\",\"message\":{\"role\":\"user\",\"content\":\"prompt %d\"}}\n" i)
                       (format "{\"message\":{\"id\":\"a%d\",\"type\":\"message\",\"role\":\"assistant\",\"content\":[{\"type\":\"tool_use\",\"id\":\"t%d\",\"name\":\"Bash\",\"input\":{\"command\":\"ls\"}}]},\"type\":\"assistant\"}\n" i i)
                       (format "{\"type\":\"user\",\"message\":{\"role\":\"user\",\"content\":[{\"type\":\"tool_result\",\"tool_use_id\":\"t%d\",\"content\":\"a.txt\"}]}}\n" i)
                       (format "{\"message\":{\"id\":\"b%d\",\"type\":\"message\",\"role\":\"assistant\",\"content\":[{\"type\":\"text\",\"text\":\"One **file**.\"}]},\"type\":\"assistant\",\"timestamp\":\"2026-11-03T10:00:05Z\"}\n" i))))
           (list :path file :id "scaling-view" :harness 'claude :provider "claude" :cwd "/tmp" :title "view"))
  :body (kill-buffer (romacs-agent-history-open it)))
