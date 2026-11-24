;;; romacs-agent-thread-word-tests.el --- The working thread says busy or idle alone -*- lexical-binding: t; -*-
;; SPDX-License-Identifier: GPL-3.0-or-later
;; Maintainer: Thanos Apollo <public@thanosapollo.org>
(require 'ert)
(require 'cl-lib)
(require 'romacs-mcp)

(let ((romacs-mcp-call-function romacs-mcp-call-function)
      (romacs-mcp-tools (copy-alist romacs-mcp-tools)))
  (require 'romacs-agent)
  (require 'romacs-agent-ui))

(defun romacs-agent-thread-word-tests--separator ()
  "The chat's separator written again, as text."
  (romacs-agent-log--separator t)
  (string-trim (buffer-substring-no-properties romacs-agent-log--end (point-max))))

(ert-deftest romacs-agent-thread-word-busy-or-idle-has-no-word ()
  ;; The thread's motion says busy or idle; what runs, its row says.
  ;; What waits on the user, or failed, is said in words.
  (let ((session (romacs-agent--make :id "T" :harness 'claude :state 'running
                                     :tools (make-hash-table :test #'equal)
                                     :streamed (make-hash-table :test #'equal))))
    (with-temp-buffer
      (romacs-agent-log-mode)
      (setq romacs-agent-log--session session
            romacs-agent-log--end (point-min-marker))
      (set-marker-insertion-type romacs-agent-log--end t)
      (let ((input (make-hash-table :test #'equal)))
        (puthash "command" "ls" input)
        (romacs-agent-log-draw session (list :type 'tool :state 'started :id "a" :name "Bash" :input input)))
      (should (equal (romacs-agent-log--doing session) '(working)))
      (should-not (string-match-p "[[:alpha:]]" (romacs-agent-thread-word-tests--separator)))
      (setf (romacs-agent-state session) 'idle)
      (should (equal (romacs-agent-log--doing session) '(idle)))
      (should-not (string-match-p "[[:alpha:]]" (romacs-agent-thread-word-tests--separator)))
      (setf (romacs-agent-failed session) t)
      (should (string-match-p "failed" (romacs-agent-thread-word-tests--separator))))))

(provide 'romacs-agent-thread-word-tests)
;;; romacs-agent-thread-word-tests.el ends here
