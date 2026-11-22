;;; romacs-agent-log-rounds-turns-tests.el --- A round's turns recede apart -*- lexical-binding: t; -*-
;; SPDX-License-Identifier: GPL-3.0-or-later
;; Maintainer: Thanos Apollo <public@thanosapollo.org>
(require 'ert)
(require 'cl-lib)
(require 'romacs-mcp)

(let ((romacs-mcp-call-function romacs-mcp-call-function)
      (romacs-mcp-tools (copy-alist romacs-mcp-tools)))
  (require 'romacs-agent)
  (require 'romacs-agent-log))

(defun romacs-agent-log-rounds-turns-tests--receded-p (words)
  "Whether WORDS, the first place they are in the chat, recede."
  (goto-char (point-min))
  (search-forward words)
  (and (memq 'romacs-agent-log-other (ensure-list (get-text-property (match-beginning 0) 'face))) t))

(ert-deftest romacs-agent-log-rounds-a-queued-prompt-keeps-each-answer ()
  ;; The user sends while a turn runs: the prompt is drawn at once and opens
  ;; a round before the turn's answer arrives.  Each turn's answer stays its
  ;; own, never receded by the next turn's end; a turn's words before its
  ;; answer still recede.
  (let ((session (romacs-agent--make :id "Q" :harness 'claude :state 'running
                                     :tools (make-hash-table :test #'equal)
                                     :streamed (make-hash-table :test #'equal))))
    (with-temp-buffer
      (romacs-agent-log-mode)
      (setq romacs-agent-log--session session
            romacs-agent-log--end (point-min-marker)
            romacs-agent-log--input (point-min-marker))
      (set-marker-insertion-type romacs-agent-log--end t)
      (cl-flet ((draw (&rest event) (romacs-agent-log-draw session event)))
        (draw :type 'input :text "first question" :request 1)
        (draw :type 'delta :text "Looking into it.")
        (draw :type 'message-end)
        (draw :type 'input :text "second question, sent while it ran" :steer t :request 2)
        (draw :type 'tool :state 'started :id "t" :name "Bash" :input (let ((h (make-hash-table :test #'equal))) (puthash "command" "ls" h) h))
        (draw :type 'tool :state 'completed :id "t" :output "a")
        (draw :type 'delta :text "The first answer.")
        (draw :type 'message-end)
        (draw :type 'done :time 0)
        (draw :type 'delta :text "The second answer.")
        (draw :type 'message-end)
        (draw :type 'done :time 0)
        (should (romacs-agent-log-rounds-turns-tests--receded-p "Looking into it"))
        (should-not (romacs-agent-log-rounds-turns-tests--receded-p "The first answer"))
        (should-not (romacs-agent-log-rounds-turns-tests--receded-p "The second answer"))))))

(ert-deftest romacs-agent-log-rounds-an-arrival-under-a-call-splits-no-turn ()
  ;; Something arrives for a subagent's call, under it: the turn goes on,
  ;; and its words before still recede at its end, a prompt queued or not.
  (let ((session (romacs-agent--make :id "R" :harness 'claude :state 'running
                                     :tools (make-hash-table :test #'equal)
                                     :streamed (make-hash-table :test #'equal))))
    (with-temp-buffer
      (romacs-agent-log-mode)
      (setq romacs-agent-log--session session
            romacs-agent-log--end (point-min-marker)
            romacs-agent-log--input (point-min-marker))
      (set-marker-insertion-type romacs-agent-log--end t)
      (cl-flet ((draw (&rest event) (romacs-agent-log-draw session event)))
        (draw :type 'input :text "go" :request 1)
        (draw :type 'delta :text "Looking into it.")
        (draw :type 'message-end)
        (draw :type 'tool :state 'started :id "a" :name "Agent"
              :input (let ((h (make-hash-table :test #'equal))) (puthash "description" "dig" h) h))
        (draw :type 'arrival :parent "a" :from "a sibling" :state "message" :text "found it")
        (draw :type 'input :text "and also this" :steer t :request 2)
        (draw :type 'tool :state 'completed :id "a" :output "done")
        (draw :type 'delta :text "The answer.")
        (draw :type 'message-end)
        (draw :type 'done :time 0)
        (should (romacs-agent-log-rounds-turns-tests--receded-p "Looking into it"))
        (should-not (romacs-agent-log-rounds-turns-tests--receded-p "The answer"))))))

(provide 'romacs-agent-log-rounds-turns-tests)
;;; romacs-agent-log-rounds-turns-tests.el ends here
