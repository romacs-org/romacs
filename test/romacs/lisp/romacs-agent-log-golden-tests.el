;;; romacs-agent-log-golden-tests.el --- An agent chat drawn as it was -*- lexical-binding: t; -*-
;; SPDX-License-Identifier: GPL-3.0-or-later
;; Maintainer: Thanos Apollo <public@thanosapollo.org>
(require 'ert)
(require 'cl-lib)
(require 'romacs-mcp)

(let ((romacs-mcp-call-function romacs-mcp-call-function)
      (romacs-mcp-tools (copy-alist romacs-mcp-tools)))
  (require 'romacs-agent)
  (require 'romacs-agent-log))

(unless (fboundp 'romacs--run-window-change-functions)
  (load (expand-file-name "../vm/window-change.el" (file-name-directory (locate-library "romacs-agent.el"))) nil t))

;;; Golden transcripts: what the chat drew before its refactor

(defconst romacs-agent-log-golden-tests--scenarios
  '((text-in-chunks
     (delta "\n\nHel") (delta "lo, wor") (delta "ld.\n") (delta "\n") (delta "\n\nSecond para")
     (delta "graph.\n\n\n\nThird.") (delta "\n\n") (message-end))
    (thinking-then-text
     (thinking "\nI should look.") (thinking "\n\n") (thinking "Then decide.\nA line.")
     (thinking "\n") (thinking "Same para.\n\n\nNew para.") (delta "The answer.") (message-end))
    (work-then-text
     (input "count")
     (thinking "Plan.")
     (tool "t1" "Bash" "wc -l notes.txt")
     (done-tool "t1" "2 notes.txt")
     (delta "Two lines.") (delta "") (delta "\n\n") (delta "Done.")
     (tool "t2" "Read" "notes.txt") (done-tool "t2" "a\nb") (delta "After work.") (message-end))
    (subagent-lines
     (agent "a" "dig")
     (delta "hello " "a") (delta "world\nnext " "a") (thinking "pondering\nmore " "a")
     (delta "back\n" "a") (delta "tail" "a")
     (delta "Root while it digs.")
     (tool "r" "Read" "notes.txt" "a")
     (delta "after\nlast" "a")
     (done-tool "a" "found it")
     (delta "Root says.") (message-end))
    (blank-only-chunks
     (delta "\n\n\n") (delta "") (delta "Late start.") (delta "\n") (delta "\n")
     (thinking "Switch.") (thinking "\n\n\n\n") (thinking "On.") (delta "Back.") (message-end))
    (markdown-faces
     (delta "Some **bo") (redisplay) (delta "ld** words") (redisplay) (delta " and plain ones")
     (redisplay) (delta "\n\n```elisp\n(defun f ()") (delta "\n  1)\n```") (redisplay)
     (delta "\n\nAfter `code`.") (message-end) (redisplay))
    (cleared-mid-stream
     (delta "Going **on") (clear) (delta "Afresh.") (thinking "Hm.") (clear) (delta "Again.") (message-end))
    ;; Calls, their output and the work under them.
    (tool-lifecycle
     (input "go")
     (ev :type tool-output :id "e1" :text "early line\n")
     (ev :type tool :state started :id "e1" :name "Bash" :input (input "command" "make"))
     (ev :type tool-output :id "e1" :text "live 1\nlive 2\n")
     (ev :type tool :state completed :id "e1" :output "built\n3 targets")
     (ev :type tool :state started :id "f1" :name "Read" :input (input "file_path" "/x/missing.el"))
     (ev :type tool :state failed :id "f1" :output "File does not exist.")
     (ev :type tool :state started :id "w1" :name "Edit" :input (input "file_path" "/x/a.el"))
     (ev :type tool :state completed :id "w1" :input (input "file_path" "/x/a.el" "old_string" "a" "new_string" "b")
         :output "The file /x/a.el has been updated.")
     (ev :type tool :state completed :id "zz" :name "Grep" :output "a.el:3: hit")
     (ev :type tool :state failed :id "zy" :name "Bash" :output "boom")
     (ev :type tool :state started :id "m1" :name "mcp__romacs__romacs_eval" :input (input "form" "(+ 1 2)"))
     (ev :type tool :state completed :id "m1" :output "3")
     (ev :type done :duration 4 :cost 0.01 :time 0))
    (tasks-under-calls
     (ev :type tool :state started :id "b1" :name "Bash" :input (input "command" "sleep 9" "run_in_background" t))
     (ev :type task :id "b1" :state started :task "k1")
     (ev :type tool :state completed :id "b1" :output "started k1")
     (ev :type task :task "k1" :state progress :activity "sleeping")
     (ev :type task :task "k1" :state progress :activity "sleeping")
     (ev :type task :task "k1" :state completed :summary "slept")
     (agent "a" "dig")
     (delta "found\n" "a")
     (done-tool "a" "the report")
     (ev :type task :id "a" :state completed :summary "dug")
     (ev :type task :task "nobody" :state failed :summary "lost")
     (ev :type tool :state started :id "a2" :name "Agent" :input (input "description" "bg"))
     (ev :type task :id "a2" :state backgrounded)
     (done-tool "a2" "async launched")
     (delta "Meanwhile.") (message-end)
     (ev :type task :id "a2" :state completed :summary "bg done"))
    (asks
     (approve "b1")
     (ev :type approval :request-id "b1" :tool "Bash" :input (input "command" "rm -rf build"))
     (unapprove "b1")
     (ev :type attributed :request-id "b1" :actor (:kind "user" :client "editor") :decision "deny" :scope "once" :reason "not now")
     (ev :type answered :request-id "b1" :behavior "deny")
     (approve "q")
     (ev :type approval :request-id "q" :tool "AskUserQuestion" :user t :questions (("Colour?" :options ("Red" "Blue"))))
     (ev :type attributed :request-id "q" :actor (:kind "user") :decision "allow" :scope "once" :answers (input "Colour?" "Blue"))
     (unapprove "q")
     (ev :type answered :request-id "q" :behavior "allow")
     (ev :type attributed :request-id "x" :actor (:kind "policy") :decision "allow" :scope "once")
     (ev :type answered :request-id "x" :behavior "allow")
     (ev :type answered :request-id "y" :behavior "deny")
     (approve "z")
     (ev :type approval :request-id "z" :tool "Write" :input (input "file_path" "/x/b.el" "content" "x"))
     (ev :type done :time 0))
    (notes-and-arrivals
     (input "hi")
     (delta "Started.")
     (ev :type arrival :from "background task" :state "completed" :text "Build done" :full "<task-notification>…")
     (delta "It passed.")
     (ev :type compacted :text "one\ntwo\nthree\nfour\nfive\nsix\nseven\neight\nnine\nten\neleven\ntwelve")
     (ev :type status :text "requesting")
     (ev :type status :text "thinking hard")
     (ev :type selection :from (:model "a" :effort "high") :to (:model nil :effort nil))
     (ev :type unknown :method "weird/thing" :raw "x")
     (ev :type unknown :raw "first line\nsecond")
     (ev :type error :text "bad")
     (ev :type unstarted)
     (ev :type renewed :why "context")
     (ev :type forked :from "S0")
     (ev :type replied :message "m-1")
     (ev :type plan :text "1. a\n2. b")
     (ev :type exit :status 1)))
  "Chats drawn step by step, each step an event in short: what the
chat's code must keep drawing as it does.")

(defun romacs-agent-log-golden-tests--event (step)
  "The event STEP says: (KIND ARGS...)."
  (let ((input (lambda (key value) (let ((h (make-hash-table :test #'equal))) (puthash key value h) h))))
    (pcase step
      (`(delta ,text . ,parent) `(:type delta :text ,text ,@(and parent `(:parent ,(car parent)))))
      (`(thinking ,text . ,parent) `(:type thinking :text ,text ,@(and parent `(:parent ,(car parent)))))
      (`(input ,text) `(:type input :text ,text))
      (`(message-end) '(:type message-end))
      (`(tool ,id ,name ,arg . ,parent)
       `(:type tool :state started :id ,id :name ,name
               :input ,(funcall input (if (equal name "Bash") "command" "file_path") arg)
               ,@(and parent `(:parent ,(car parent)))))
      (`(agent ,id ,description)
       `(:type tool :state started :id ,id :name "Agent" :input ,(funcall input "description" description)))
      (`(done-tool ,id ,output) `(:type tool :state completed :id ,id :output ,output))
      (`(ev . ,plist)
       (cl-loop for (key value) on plist by #'cddr
                append (list key (pcase value
                                   (`(input . ,pairs)
                                    (let ((h (make-hash-table :test #'equal)))
                                      (cl-loop for (k v) on pairs by #'cddr do (puthash k v h))
                                      h))
                                   (_ value))))))))

(defvar romacs-agent-log-golden-tests--maps nil
  "The keymap variables a chat's text may name, while a scenario draws.")

(defun romacs-agent-log-golden-tests--plain (value)
  "VALUE, a text property's, as data that reads back: a keymap as the
variable that holds it, a hash table as its pairs, a marker as its
position, a function as `fn'."
  (cond ((keymapp value)
         (or (cl-loop for map in romacs-agent-log-golden-tests--maps
                      when (eq (symbol-value map) value) return map)
             'keymap))
        ((hash-table-p value) (cons 'hash (sort (let (pairs) (maphash (lambda (k v) (push (cons k (romacs-agent-log-golden-tests--plain v)) pairs)) value) pairs)
                                                 (lambda (a b) (string< (format "%S" (car a)) (format "%S" (car b)))))))
        ((markerp value) (cons 'marker (marker-position value)))
        ((stringp value) (substring-no-properties value))
        ((and (functionp value) (not (symbolp value))) 'fn)
        ((consp value) (cons (romacs-agent-log-golden-tests--plain (car value))
                             (romacs-agent-log-golden-tests--plain (cdr value))))
        ((vectorp value) (vconcat (mapcar #'romacs-agent-log-golden-tests--plain value)))
        (t value)))

(defun romacs-agent-log-golden-tests--runs ()
  "The transcript's property runs, (FROM TO PROPERTIES), each read back plain."
  (let ((pos (point-min)) (end (marker-position romacs-agent-log--end)) runs)
    (while (< pos end)
      (let ((next (min end (next-property-change pos nil end)))
            (props (cl-loop for (key value) on (text-properties-at pos) by #'cddr
                            append (list key (romacs-agent-log-golden-tests--plain value)))))
        (when props (push (list pos next props) runs))
        (setq pos next)))
    (nreverse runs)))

(defun romacs-agent-log-golden-tests--draw (steps)
  "The transcript after each of STEPS, as (TEXT RUNS)."
  (let ((romacs-agent-approval-policy 'ask)
        (romacs-agent-log-golden-tests--maps (apropos-internal "\\`romacs-agent-.*-map\\'" #'boundp))
        (session (romacs-agent--make :id "S" :harness 'claude :state 'running
                                     :tools (make-hash-table :test #'equal)
                                     :streamed (make-hash-table :test #'equal))))
    (with-temp-buffer
      (romacs-agent-log-mode)
      (setq romacs-agent-log--session session
            romacs-agent-log--end (point-min-marker)
            romacs-agent-log--input (point-min-marker))
      (set-marker-insertion-type romacs-agent-log--end t)
      (romacs-agent-log--separator t)
      (mapcar (lambda (step)
                (pcase step
                  ('(redisplay) (run-hook-with-args 'pre-redisplay-functions (selected-window)))
                  ('(clear) (romacs-agent-log--clear) (romacs-agent-log--separator t))
                  (`(approve ,id) (puthash id t (romacs-agent-approvals session)))
                  (`(unapprove ,id) (remhash id (romacs-agent-approvals session)))
                  (_ (romacs-agent-log-draw session (romacs-agent-log-golden-tests--event step))
                     (romacs-agent-log--separator)))
                (list (buffer-substring-no-properties (point-min) romacs-agent-log--end)
                      (romacs-agent-log-golden-tests--runs)))
              steps))))

(defconst romacs-agent-log-golden-tests--goldens-file
  (expand-file-name "romacs-agent-log-golden-resources/goldens.eld"
                    (file-name-directory (or load-file-name buffer-file-name)))
  "Every scenario's transcripts, printed from the code before its refactor.")

(defun romacs-agent-log-golden-tests--golden (steps)
  "What STEPS draw, as kept: (FINAL-TEXT DIGEST...), the transcript's text
at the end and, after each step, a digest of its text and properties."
  (let ((print-length nil) (print-level nil) (print-circle nil) (print-escape-newlines t)
        (snapshots (romacs-agent-log-golden-tests--draw steps)))
    (cons (car (car (last snapshots)))
          (mapcar (lambda (snapshot) (secure-hash 'sha1 (prin1-to-string snapshot))) snapshots))))

(defun romacs-agent-log-golden-tests-write-goldens ()
  "Print the goldens from the code as it is: only before a refactor."
  (let ((print-escape-newlines t) (print-length nil) (print-level nil))
    (with-temp-file romacs-agent-log-golden-tests--goldens-file
      (pcase-dolist (`(,name . ,steps) romacs-agent-log-golden-tests--scenarios)
        (prin1 (cons name (romacs-agent-log-golden-tests--golden steps)) (current-buffer))
        (insert "\n")))))

(ert-deftest romacs-agent-log-draws-as-it-did ()
  (let ((goldens (with-temp-buffer
                   (insert-file-contents romacs-agent-log-golden-tests--goldens-file)
                   (let (all) (condition-case nil (while t (push (read (current-buffer)) all)) (end-of-file))
                        (nreverse all)))))
    (should (equal (mapcar #'car goldens) (mapcar #'car romacs-agent-log-golden-tests--scenarios)))
    (pcase-dolist (`(,name . ,steps) romacs-agent-log-golden-tests--scenarios)
      (pcase-let ((`(,text . ,digests) (romacs-agent-log-golden-tests--golden steps))
                  (`(,want-text . ,want-digests) (alist-get name goldens)))
        (should (equal (list name text) (list name want-text)))
        (should (equal (list name (length want-digests)) (list name (length steps))))
        ;; The first step that draws otherwise, named.
        (cl-loop for step in steps for digest in digests for want in want-digests
                 do (should (equal (list name step digest) (list name step want))))))))

(provide 'romacs-agent-log-golden-tests)
;;; romacs-agent-log-golden-tests.el ends here
