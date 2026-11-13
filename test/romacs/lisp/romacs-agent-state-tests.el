;;; romacs-agent-state-tests.el --- A session's one state, and settling it -*- lexical-binding: t; -*-
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
  (require 'romacs-agent-ui))

(defvar romacs-mode-line-agent)

(defun romacs-agent-state-tests--session (&rest fields)
  (apply #'romacs-agent--make (append fields (list :id "s" :harness 'claude :provider "claude"))))

(defconst romacs-agent-state-tests--table
  '(((:state waiting-approval :driver-state (:asks ("a"))) approval)
    ((:state waiting-approval :driver-state (:asks ("q") :questions ("q"))) question)
    ;; A tool call to approve outranks a question beside it.
    ((:state waiting-approval :driver-state (:asks ("a" "q") :questions ("q"))) approval)
    ((:state running) working)
    ((:state starting) working)
    ((:state idle :failed t :done-seq 3) failed)
    ;; An unseen failure outranks a new turn running.
    ((:state running :failed t :done-seq 3 :input-seq 4) failed)
    ((:state idle :failed t :done-seq 3 :seen-seq 3) idle)
    ((:state idle :failed t :done-seq 3 :cancelled t) ended)
    ((:state idle :done-seq 5) completed)
    ;; A delegated task's completion went to its parent.
    ((:state idle :done-seq 5 :options (:parent "p")) idle)
    ((:state idle :done-seq 5 :options (:parent "p") :failed t) failed)
    ((:state idle :done-seq 5 :seen-seq 5) idle)
    ((:state idle :done-seq 5 :settled-seq 5) settled)
    ;; New activity unsettles: a prompt, then the turn's end.
    ((:state running :done-seq 5 :settled-seq 5 :input-seq 6) working)
    ((:state idle :done-seq 7 :settled-seq 5 :input-seq 6) completed)
    ((:state detached) detached)
    ((:state detached :settled-seq 0) settled)
    ;; Its supervisor never came up: a failure until seen, then what it is.
    ((:state unstarted :failed t :done-seq 3) failed)
    ((:state unstarted :failed t :done-seq 3 :seen-seq 3) unstarted)
    ((:state ended :done-seq 5 :seen-seq 5) ended)
    ((:state ended :done-seq 5) completed)
    ((:state history) history)
    ((:state idle :options (:plan t) :input-seq 4 :planned-seq 4 :done-seq 5 :seen-seq 5) plan)
    ;; The plan was an earlier turn's.
    ((:state idle :options (:plan t) :input-seq 6 :planned-seq 4 :done-seq 7 :seen-seq 7) idle)
    ((:state idle :children-rows ((:key working))) waiting)
    ((:state idle :children-rows ((:key question))) waiting)
    ((:state idle :children-rows ((:key idle) (:key failed))) idle)
    ((:state idle :failed t :done-seq 3 :children-rows ((:key working))) failed))
  "Each session's FIELDS and the state key it is in.")

(ert-deftest romacs-agent-state-table ()
  (dolist (case romacs-agent-state-tests--table)
    (should (equal (list (car case) (romacs-agent-ui--state-key
                                     (apply #'romacs-agent-state-tests--session (car case))))
                   case))))

(ert-deftest romacs-agent-state-tuples-rank-in-order ()
  (let ((rank -1))
    (dolist (state romacs-agent-ui--states)
      (pcase-let ((`(,key ,glyph ,word ,face ,state-rank) state))
        (should (symbolp key))
        (should (or (null glyph) (stringp glyph)))
        (should (stringp word))
        (should (facep face))
        (should (= state-rank (cl-incf rank))))))
  ;; What wants the user outranks what runs, which outranks what finished.
  (should (equal (seq-filter #'romacs-agent-ui--news-p (mapcar #'car romacs-agent-ui--states))
                 '(approval question failed working waiting plan completed)))
  (should-not (nth 1 (romacs-agent-ui--state-of 'settled))))

(ert-deftest romacs-agent-state-task-words ()
  (should (equal (mapcar #'romacs-agent-ui--status-key '("running" "waiting" "failed" "completed" "cancelled" "detached"))
                 '(working approval failed completed ended detached)))
  ;; A harness task's completion is its turn's to tell; a failure is news
  ;; until the next turn.
  (should (eq (romacs-agent-ui--row-key '(:state "completed")) 'idle))
  (should (eq (romacs-agent-ui--row-key '(:state "failed" :unseen t)) 'failed))
  (should (eq (romacs-agent-ui--row-key '(:state "failed")) 'idle)))

(defmacro romacs-agent-state-tests--recorded (fields &rest body)
  "BODY with `session', of FIELDS, its manifest in the private `dir'."
  (declare (indent 1))
  `(let* ((dir (make-temp-file "romacs-state-" t))
          (romacs-agent-event-functions nil)
          (session (romacs-agent--make ,@fields :id "s" :harness 'claude :provider "claude" :dir dir
                                       :host-socket (expand-file-name "none.sock" dir))))
     (unwind-protect
         (progn (romacs-agent--manifest session nil) ,@body)
       (delete-directory dir t))))

(defun romacs-agent-state-tests--manifest (dir)
  (romacs-agent--read-json (expand-file-name "manifest.json" dir)))

(ert-deftest romacs-agent-state-settle-survives-a-restart ()
  (romacs-agent-state-tests--recorded (:state 'idle :input-seq 4 :done-seq 5)
    (should-not (gethash "settledSeq" (romacs-agent-state-tests--manifest dir)))
    (romacs-agent-settle session)
    (should (romacs-agent-settled-p session))
    (should-not (romacs-agent-unseen-p session))
    (let ((manifest (romacs-agent-state-tests--manifest dir)))
      (should (= (gethash "settledSeq" manifest) 5))
      (should (= (gethash "seenSeq" manifest) 5)))
    (let ((romacs-agent--sessions (make-hash-table :test #'equal)))
      (let ((loaded (romacs-agent--load dir)))
        (should (= (romacs-agent-settled-seq loaded) 5))
        (should (= (romacs-agent-seen-seq loaded) 5))))))

(ert-deftest romacs-agent-state-seen-survives-a-restart ()
  (romacs-agent-state-tests--recorded (:state 'idle :done-seq 5)
    (let ((writes 0))
      (cl-letf* ((put (symbol-function 'romacs-agent--manifest-put))
                 ((symbol-function 'romacs-agent--manifest-put)
                  (lambda (&rest args) (cl-incf writes) (apply put args))))
        (romacs-agent-mark-seen session)
        ;; Seen already: nothing to write (a chat's replay marks every turn).
        (romacs-agent-mark-seen session))
      (should (= writes 1)))
    (let ((romacs-agent--sessions (make-hash-table :test #'equal)))
      (should (= (romacs-agent-seen-seq (romacs-agent--load dir)) 5)))))

(defun romacs-agent-state-tests--journal (dir)
  "A journal in DIR of one turn, its end at seq 2."
  (let ((coding-system-for-write 'utf-8-unix))
    (write-region (concat (romacs-agent--json (romacs-mcp--object "seq" 1 "t" 1.0 "in" (romacs-mcp--object "prompt" "hi")))
                          "\n"
                          (romacs-agent--json (romacs-mcp--object "seq" 2 "t" 2.0
                                                                  "out" (romacs-mcp--object "type" "result" "result" "ok")))
                          "\n")
                  nil (expand-file-name "journal.jsonl" dir) nil 'silent)))

(ert-deftest romacs-agent-state-a-recorded-turn-end-is-news ()
  (romacs-agent-state-tests--recorded (:state 'idle)
    (romacs-agent-state-tests--journal dir)
    (let ((romacs-agent--sessions (make-hash-table :test #'equal)))
      (let ((loaded (romacs-agent--load dir)))
        (should (= (romacs-agent-done-seq loaded) 2))
        (should (romacs-agent-unseen-p loaded))))))

(ert-deftest romacs-agent-state-an-older-session-has-seen-its-past ()
  ;; Recorded before turn ends were kept as seen: what it had is no news,
  ;; and its manifest keeps that.
  (romacs-agent-state-tests--recorded (:state 'idle :seen-seq nil)
    (should-not (gethash "seenSeq" (romacs-agent-state-tests--manifest dir)))
    (romacs-agent-state-tests--journal dir)
    (let ((romacs-agent--sessions (make-hash-table :test #'equal)))
      (let ((loaded (romacs-agent--load dir)))
        (should (= (romacs-agent-seen-seq loaded) 2))
        (should-not (romacs-agent-unseen-p loaded))
        (should (= (gethash "seenSeq" (romacs-agent-state-tests--manifest dir)) 2))))))

(ert-deftest romacs-agent-state-new-activity-unsettles ()
  (romacs-agent-state-tests--recorded (:state 'idle :input-seq 4 :done-seq 5 :caught-up t)
    (romacs-agent-settle session)
    (should (eq (romacs-agent-ui--state-key session) 'settled))
    (romacs-agent--raise session '(:type input :seq 6 :text "more"))
    (should (eq (romacs-agent-ui--state-key session) 'working))
    (romacs-agent--raise session '(:type done :seq 7 :text "done"))
    (should (eq (romacs-agent-ui--state-key session) 'completed))
    (romacs-agent-settle session)
    (should (eq (romacs-agent-ui--state-key session) 'settled))))

(ert-deftest romacs-agent-state-questions-and-plans-from-events ()
  (romacs-agent-state-tests--recorded (:state 'running :input-seq 4)
    (let ((romacs-agent-approval-policy 'allow))
      (romacs-agent--raise session '(:type approval :request-id "q" :user t :questions (("Which?" "a" "b"))))
      (should (equal (romacs-agent-questions session) '("q")))
      (should (eq (romacs-agent-ui--state-key session) 'question))
      (romacs-agent--raise session '(:type approval :request-id "a" :user t :tool "Bash"))
      (should (eq (romacs-agent-ui--state-key session) 'approval))
      (romacs-agent--raise session '(:type answered :request-id "a"))
      (should (eq (romacs-agent-ui--state-key session) 'question))
      (romacs-agent--raise session '(:type answered :request-id "q"))
      (should-not (romacs-agent-questions session))
      (should (eq (romacs-agent-ui--state-key session) 'working))
      ;; A plan presented in plan mode, as Codex presents one (a plan
      ;; item): ready once its turn ends.
      (romacs-agent--set-options session '(:plan t))
      (dolist (event (romacs-agent--codex-item
                      session (romacs-mcp--object "type" "plan" "id" "p1" "text" "1. do it") t))
        (romacs-agent--raise session event))
      (should (= (romacs-agent-planned-seq session) 4))
      (romacs-agent--raise session '(:type done :seq 5))
      (should (eq (romacs-agent-ui--state-key session) 'plan))
      ;; The next prompt's turn has not presented it.
      (romacs-agent--raise session '(:type input :seq 6))
      (romacs-agent--raise session '(:type done :seq 7))
      (should (eq (romacs-agent-ui--state-key session) 'completed)))))

(ert-deftest romacs-agent-state-settling-tells-the-views ()
  (romacs-agent-state-tests--recorded (:state 'idle :done-seq 5)
    (let* ((heard nil)
           (romacs-agent-event-functions (list (lambda (_session event) (push event heard)))))
      (romacs-agent-settle session)
      (should (equal heard '((:type settled))))
      (should (romacs-agent-ui--state-event-p (car heard))))
    (should (romacs-agent-ui--state-event-p '(:type status :text "plan mode off")))))

(ert-deftest romacs-agent-state-a-grandchild-reaches-the-top ()
  ;; lead -> worker -> its own worker: the last ending makes the worker
  ;; idle, and the lead no longer waits on it.
  (let ((romacs-agent--sessions (make-hash-table :test #'equal))
        (romacs-agent--children (make-hash-table :test #'equal))
        (romacs-agent-ui--children-changed (make-hash-table :test #'eq))
        (romacs-agent-ui--children-timer nil))
    (cl-flet ((mk (id state parent)
                (let ((session (romacs-agent--make :id id :harness 'claude :state state
                                                   :options (and parent (list :parent parent)))))
                  (puthash id session romacs-agent--sessions)
                  (romacs-agent--index session)
                  session)))
      (let* ((lead (mk "lead" 'idle nil)) (worker (mk "worker" 'idle "lead")) (leaf (mk "leaf" 'running "worker")))
        (romacs-agent-ui--summarize-children worker)
        (romacs-agent-ui--summarize-children lead)
        (should (eq (romacs-agent-ui--state-key lead) 'waiting))
        (setf (romacs-agent-state leaf) 'idle)
        (romacs-agent-ui--track-children leaf '(:type done))
        (while romacs-agent-ui--children-timer
          (cancel-timer romacs-agent-ui--children-timer)
          (romacs-agent-ui--children-flush))
        (should (eq (romacs-agent-ui--state-key worker) 'idle))
        (should (eq (romacs-agent-ui--state-key lead) 'idle))))))

(ert-deftest romacs-agent-state-status-bar-rolls-up-the-news ()
  (let ((romacs-agent-glyphs nil)
        (romacs-mode-line-agent nil)
        (romacs-agent--sessions (make-hash-table :test #'equal)))
    (dolist (fields '((:id "a" :state waiting-approval) (:id "b" :state idle :done-seq 3)
                      (:id "c" :state idle :done-seq 3 :settled-seq 3) (:id "d" :state running)))
      (let ((session (apply #'romacs-agent--make :harness 'claude fields)))
        (puthash (romacs-agent-id session) session romacs-agent--sessions)))
    (romacs-agent-ui--status-bar)
    (should (equal (substring-no-properties romacs-mode-line-agent) "1 needs approval · 1 working · 1 completed"))
    (clrhash romacs-agent--sessions)
    (romacs-agent-ui--status-bar)
    (should-not romacs-mode-line-agent)))

(provide 'romacs-agent-state-tests)
;;; romacs-agent-state-tests.el ends here
