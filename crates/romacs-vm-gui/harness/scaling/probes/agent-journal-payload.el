;;; agent-journal-payload.el --- scaling probe  -*- lexical-binding: t -*-

;; A message with a file of N kilobytes attached: what the journal keeps of
;; it, which a replay parses, does not grow with the image.
(scaling-probe agent-journal-payload :n 2000 :budget 0.1
  :setup (progn (scaling--agent)
                (let ((file (make-temp-file "scaling-payload" nil ".png")))
                  (with-temp-file file (insert (make-string (* n 1024) ?x)))
                  file))
  :body (let ((session (romacs-agent--make :harness 'claude :id "scaling-payload")) (kept nil))
          (cl-letf (((symbol-function 'romacs-agent--line) (lambda (_ _wire journal) (setq kept journal)))
                    ((symbol-function 'romacs-agent--claude-image) (lambda (_) (romacs-mcp--object "type" "image"))))
            (dotimes (_ 200)
              (romacs-agent--input session "see" (list it))
              (json-parse-string (romacs-agent--json kept))))))
