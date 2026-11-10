;;; agent-history-t3-relist.el --- scaling probe  -*- lexical-binding: t -*-

;; The history listed anew with N instances from T3, each a transcript of
;; its own: each row's instance, and its title, by an index.
(scaling-probe agent-history-t3-relist :n 200
  :setup (let* ((root (make-temp-file "scaling-t3h-" t))
                (settings (expand-file-name "t3/settings.json" root))
                (instances (make-hash-table :test #'equal)))
           (scaling--agent)
           (require 'romacs-agent-history)
           (make-directory (file-name-directory settings) t)
           (dotimes (i n)
             (let* ((home (expand-file-name (format "home-%d" i) root))
                    (dir (expand-file-name "projects/-work" home))
                    (id (format "%08x-0000-4000-8000-%012x" i i)))
               (make-directory dir t)
               (with-temp-file (expand-file-name (concat id ".jsonl") dir)
                 (insert (json-serialize (romacs-mcp--object "type" "user" "cwd" "/work"
                                                             "message" (romacs-mcp--object "role" "user" "content" (format "task %d" i))))
                         "\n"))
               (puthash (format "claudeAgent_%d" i)
                        (romacs-mcp--object "driver" "claudeAgent" "displayName" (format "C %d" i)
                                            "config" (romacs-mcp--object "homePath" home))
                        instances)))
           (with-temp-file settings (insert (json-serialize (romacs-mcp--object "providerInstances" instances))))
           (let ((romacs-agent-providers (list (list 't3 :settings settings)))) (romacs-agent-history-list))
           (list (list 't3 :settings settings)))
  :body (let ((romacs-agent-providers it)) (romacs-agent-history-list)))
