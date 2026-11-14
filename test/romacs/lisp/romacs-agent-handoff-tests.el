;;; romacs-agent-handoff-tests.el --- A thread carried to another harness -*- lexical-binding: t; -*-
;; SPDX-License-Identifier: GPL-3.0-or-later
;; Maintainer: Thanos Apollo <public@thanosapollo.org>
(require 'ert)
(require 'cl-lib)
(require 'romacs-agent-handoff)

(defun romacs-agent-handoff-tests--items (n)
  "N exchanges: a request, a tool call, a reply each."
  (cl-loop for i from 1 to n
           append (list (list :role 'user :text (format "ask %d" i))
                        (list :role 'tool :text (format "Read file%d.el" i))
                        (list :role 'assistant :text (format "reply %d %s" i (make-string 50 ?x))))))

(ert-deftest romacs-agent-handoff-keeps-everything-within-budget ()
  (let ((brief (romacs-agent-handoff-brief (romacs-agent-handoff-tests--items 2) "claude · opus-5-5" "Go on.")))
    (should (string-prefix-p "Context handoff (claude · opus-5-5):" brief))
    (should (string-match-p "not a new request" brief))
    (should (string-match-p "User: ask 1\n\nTool: Read file1.el\n\nAssistant: reply 1" brief))
    (should (string-suffix-p "\n\n---\n\nGo on." brief))
    (should-not (string-match-p "left out" brief))))

(ert-deftest romacs-agent-handoff-a-small-budget-keeps-the-ends ()
  ;; The first request, the last request and the last reply always; the
  ;; rest newest first; what is left out is said.
  (let* ((items (romacs-agent-handoff-tests--items 10))
         (brief (romacs-agent-handoff-brief items "codex" "Next." 260)))
    (should (string-match-p "User: ask 1\n" brief))
    (should (string-match-p "User: ask 10\n" brief))
    (should (string-match-p "Assistant: reply 10 " brief))
    (should-not (string-match-p "ask 5\n" brief))
    (should (string-match-p "\\[[0-9]+ earlier items left out\\]" brief))
    ;; In the thread's order.
    (should (< (string-search "ask 1\n" brief) (string-search "ask 10\n" brief)))))

(ert-deftest romacs-agent-handoff-the-latest-stretch-is-unbroken ()
  ;; A big item stops the fill: no small older one is picked up past it.
  (let* ((items (list '(:role user :text "first")
                      '(:role tool :text "small old")
                      (list :role 'assistant :text (make-string 500 ?b))
                      '(:role tool :text "recent")
                      '(:role user :text "last")
                      '(:role assistant :text "end")))
         (brief (romacs-agent-handoff-brief items "x" "Go." 120)))
    (should (string-match-p "Tool: recent" brief))
    (should-not (string-match-p "small old" brief))))

(ert-deftest romacs-agent-handoff-an-empty-thread ()
  (should (string-suffix-p "---\n\nStart." (romacs-agent-handoff-brief nil "x" "Start."))))

(ert-deftest romacs-agent-handoff-a-thread-s-items ()
  (let ((input (make-hash-table :test #'equal)))
    (puthash "command" "make test\n  -j4" input)
    (should (equal (romacs-agent-handoff-items
                    (list '(:type input :text "fix it")
                          (list :type 'tool :state 'started :name "Bash" :input input)
                          '(:type tool :state started :name "Read" :parent "t1")
                          '(:type tool :state completed :name "Bash")
                          '(:type done :text "Fixed.")
                          '(:type done :text "")))
                   '((:role user :text "fix it") (:role tool :text "Bash make test -j4")
                     (:role assistant :text "Fixed."))))))

(ert-deftest romacs-agent-handoff-starts-a-session-on-a-brief ()
  (require 'romacs-agent)
  (let ((source (romacs-agent--make :id "s" :harness 'claude :provider "claude" :model "claude-opus-5-5"
                                    :cwd "/work/" :title "Plan"))
        (started nil))
    (cl-letf (((symbol-function 'romacs-agent-start)
               (lambda (&rest args) (setq started args) 'new)))
      (should (eq (romacs-agent-handoff source '((:type input :text "build it") (:type done :text "Built."))
                                        "codex" "gpt-6.1-sol" "Review it.")
                  'new)))
    (pcase-let ((`(,cwd ,prompt ,title ,provider ,options) started))
      (should (equal cwd "/work/"))
      (should (string-prefix-p "Context handoff (claude · claude-opus-5-5):" prompt))
      (should (string-match-p "User: build it\n\nAssistant: Built." prompt))
      (should (string-suffix-p "Review it." prompt))
      (should (equal title "Plan (→ codex)"))
      (should (equal provider "codex"))
      (should (equal options '(:model "gpt-6.1-sol"))))))

(ert-deftest romacs-agent-handoff-a-wake-is-context ()
  (require 'romacs-agent-tools)
  (let* ((child (romacs-agent--make :id "11111111-0000-4000-8000-000000000000" :title "t"))
         (wake (romacs-agent-tools--wake-text child '(1 "done" nil)))
         (items (romacs-agent-handoff-items (list '(:type input :text "do it") (list :type 'input :text wake)))))
    (should (equal (mapcar (lambda (i) (plist-get i :role)) items) '(user tool)))
    ;; The last request is the user's, not the wake.
    (should (string-match-p "User: do it" (romacs-agent-handoff-brief items "x" "Go." 40)))))

(ert-deftest romacs-agent-handoff-the-thread-starts-with-what-was-imported ()
  (require 'romacs-agent)
  (require 'romacs-agent-history)
  (let ((session (romacs-agent--make :id "i" :harness 'claude :imported '(:path "/x.jsonl" :size 9))))
    (cl-letf (((symbol-function 'romacs-agent-history-events)
               (lambda (_harness _imported) (cons '((:type input :text "before")) nil))))
      (should (equal (mapcar (lambda (e) (plist-get e :text))
                             (romacs-agent-handoff-thread session '((:type input :text "after"))))
                     '("before" "after"))))))

(ert-deftest romacs-agent-handoff-the-journal-replays-in-steps ()
  (require 'romacs-agent)
  (let* ((dir (make-temp-file "romacs-handoff-" t))
         (session (romacs-agent--make :id "j" :harness 'claude :dir dir :tools (make-hash-table :test #'equal)
                                      :streamed (make-hash-table :test #'equal)))
         (result :none) (ticks 0))
    (unwind-protect
        (progn
          (with-temp-file (expand-file-name "journal.jsonl" dir)
            (dotimes (i 5)
              (insert (json-serialize (romacs-mcp--object "seq" (1+ i) "in" (romacs-mcp--object "prompt" (format "p%d" i))))
                      "\n")))
          (cl-letf (((symbol-function 'run-at-time)
                     (lambda (_time _repeat f &rest args) (cl-incf ticks) (apply f args))))
            (romacs-agent-events-async session (lambda (events) (setq result events)) 2))
          ;; Two at a time: three steps, the last calling back.
          (should (= ticks 2))
          (should (equal (mapcar (lambda (e) (plist-get e :text)) (seq-filter (lambda (e) (eq (plist-get e :type) 'input)) result))
                         '("p0" "p1" "p2" "p3" "p4"))))
      (delete-directory dir t))))

(ert-deftest romacs-agent-handoff-streamed-replies ()
  ;; An imported transcript's reply comes as deltas, its end with no text.
  (should (equal (romacs-agent-handoff-items
                  '((:type input :text "q") (:type delta :text "Ans") (:type delta :text "wer.")
                    (:type delta :text "sub" :parent "t1") (:type done)
                    (:type input :text "q2") (:type delta :text "ignored") (:type done :text "Own text.")))
                 '((:role user :text "q") (:role assistant :text "Answer.")
                   (:role user :text "q2") (:role assistant :text "Own text.")))))

(ert-deftest romacs-agent-handoff-a-real-claude-transcript ()
  ;; Claude Code's own transcript, read as an import is, through the brief.
  (require 'romacs-agent-history)
  (let ((file (make-temp-file "romacs-handoff-" nil ".jsonl")))
    (unwind-protect
        (progn
          (with-temp-file file
            (dolist (line (list "{\"type\":\"user\",\"message\":{\"role\":\"user\",\"content\":\"name a colour\"},\"uuid\":\"u1\"}"
                                "{\"type\":\"assistant\",\"message\":{\"id\":\"m1\",\"role\":\"assistant\",\"content\":[{\"type\":\"text\",\"text\":\"Teal.\"}]},\"uuid\":\"a1\"}"))
              (insert line "\n")))
          (let* ((events (car (romacs-agent-history-events 'claude (list :path file))))
                 (brief (romacs-agent-handoff-brief (romacs-agent-handoff-items events) "claude" "Go.")))
            (should (string-match-p "User: name a colour" brief))
            (should (string-match-p "Assistant: Teal\\." brief))))
      (delete-file file))))

(ert-deftest romacs-agent-handoff-a-cut-import-says-so ()
  (require 'romacs-agent)
  (require 'romacs-agent-history)
  (let ((session (romacs-agent--make :id "i" :harness 'claude :imported '(:path "/x.jsonl" :size 9))))
    (cl-letf (((symbol-function 'romacs-agent-history-events)
               (lambda (_h _i) (cons '((:type input :text "recent ask") (:type done :text "recent reply")) t)))
              ((symbol-function 'romacs-agent-history-first-request)
               (lambda (_h _i) '(:type input :text "the first ask"))))
      (let ((brief (romacs-agent-handoff-brief
                    (romacs-agent-handoff-items (romacs-agent-handoff-thread session nil)) "claude" "Go." 200)))
        (should (string-match-p "User: the first ask\n\n\\[earlier turns of this thread left out\\]\n\nUser: recent ask"
                                brief))))))

(provide 'romacs-agent-handoff-tests)
;;; romacs-agent-handoff-tests.el ends here
