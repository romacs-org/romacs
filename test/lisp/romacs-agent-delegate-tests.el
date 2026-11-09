;;; romacs-agent-delegate-tests.el --- What a harness and a child are told -*- lexical-binding: t; -*-
;; SPDX-License-Identifier: GPL-3.0-or-later
;; Maintainer: Thanos Apollo <public@thanosapollo.org>
(require 'ert)
(require 'cl-lib)
(require 'romacs-mcp)

;; Loading romacs-agent installs romacs-agent-tools: the endpoint's
;; registrations are left as they were.
(let ((romacs-mcp-call-function romacs-mcp-call-function)
      (romacs-mcp-tools (copy-alist romacs-mcp-tools)))
  (require 'romacs-agent)
  (require 'romacs-agent-tools)
  (require 'romacs-agent-claude))

(ert-deftest romacs-agent-delegate-brief-names-the-role ()
  (should (equal (romacs-agent-tools--brief "reviewer" "Read x.el")
                 "Act as the reviewer sub-agent for this task.\n\nRead x.el"))
  (should (equal (romacs-agent-tools--brief nil "Read x.el") "Read x.el"))
  (should (equal (romacs-agent-tools--brief "" "Read x.el") "Read x.el"))
  ;; The child is still named after its task.
  (should (equal (romacs-agent--task-line (romacs-agent-tools--brief "reviewer" "Read x.el")) "Read x.el")))

(ert-deftest romacs-agent-delegate-wake-is-bounded ()
  (let* ((id "0f0e0d0c-0b0a-4908-8706-050403020100")
         (session (romacs-agent--make :id id :title "t"))
         (short (romacs-agent-tools--wake-text session (list 1 "done" nil)))
         (long (romacs-agent-tools--wake-text session (list 1 (make-string 40000 ?x) nil))))
    (should (equal (plist-get (romacs-agent-tools--wake-parse short) :result) "done"))
    (should (< (length long) 16500))
    (should (string-match-p (concat "40000 characters: romacs_task_status with taskId " id) long))
    ;; Still read back as a wake.
    (should (equal (plist-get (romacs-agent-tools--wake-parse long) :id) id))))

(ert-deftest romacs-agent-delegate-running-task-says-end-your-turn ()
  (let ((running (romacs-agent-tools--task (romacs-agent--make :id "a" :state 'running)))
        (done (romacs-agent-tools--task (romacs-agent--make :id "b" :state 'idle :turns 1))))
    (should (string-match-p "end your turn" (gethash "next" running)))
    (should (eq (gethash "next" done) :null))))

(ert-deftest romacs-agent-delegate-description-asks-for-a-complete-brief ()
  (let ((text (romacs-agent-tools--delegate-description)))
    (should (string-match-p "none of your conversation" text))
    (should (string-match-p "do not poll" text))))

(ert-deftest romacs-agent-delegate-harness-is-told-of-romacs ()
  (let ((top (romacs-agent--make :id "a" :harness 'claude))
        (child (romacs-agent--make :id "b" :harness 'claude :options '(:parent "a"))))
    (should (string-match-p "inside Romacs" (romacs-agent--instructions top)))
    (should-not (string-match-p "delegated task" (romacs-agent--instructions top)))
    (should (string-match-p "do the work yourself" (romacs-agent--instructions child)))
    (cl-letf (((symbol-function 'romacs-agent--write-json) #'ignore))
      (should (equal (cadr (member "--append-system-prompt" (romacs-agent--claude-argv child nil)))
                     (romacs-agent--instructions child))))))

(ert-deftest romacs-agent-delegate-image-captions-are-not-text ()
  (let ((content (vector (romacs-mcp--object "type" "text" "text" "see")
                         (romacs-mcp--object "type" "text" "text" (romacs-agent--image-caption "/a b/c.png"))
                         (romacs-mcp--object "type" "image"))))
    (should (equal (romacs-agent--text-of content) "see"))
    ;; Text that only mentions one stays.
    (should (equal (romacs-agent--text-of (vector (romacs-mcp--object "type" "text" "text" "x [Attached image saved at: /a]")))
                   "x [Attached image saved at: /a]"))))

(provide 'romacs-agent-delegate-tests)
;;; romacs-agent-delegate-tests.el ends here
