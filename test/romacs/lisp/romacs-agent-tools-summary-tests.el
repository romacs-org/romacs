;;; romacs-agent-tools-summary-tests.el --- Session summaries at the MCP boundary -*- lexical-binding: t; -*-
;; SPDX-License-Identifier: GPL-3.0-or-later
;; Maintainer: Thanos Apollo <public@thanosapollo.org>

(require 'ert)
(require 'cl-lib)
(require 'json)
(require 'nadvice)
(let ((load-path (cons (file-name-directory load-file-name) load-path)))
  (require 'romacs-agent-test-stored))
(require 'romacs-agent-tools)
(require 'romacs-agent-remote)
(require 'romacs-agent-summary)
(require 'romacs-agent-claude)
(require 'romacs-agent-codex)

(defun romacs-agent-tools-summary-tests--id (number)
  "The fixture task id for NUMBER."
  (format "%08x-0000-4000-8000-000000000000" number))

(defun romacs-agent-tools-summary-tests--session (number &rest fields)
  "Make fixture NUMBER with FIELDS overriding its defaults."
  (let* ((id (romacs-agent-tools-summary-tests--id number))
         (defaults (list :id id :harness 'claude :provider "claude"
                         :title (format "Task %d" number) :state 'idle
                         :cwd "/var/tmp" :last-time (+ 500.0 number)
                         :dir (expand-file-name id (romacs-agent--state-root))
                         :options (list :parent (romacs-agent-tools-summary-tests--id 1)))))
    (cl-loop for (key value) on fields by #'cddr do
             (setq defaults (plist-put defaults key value)))
    (apply #'romacs-agent--make defaults)))

(defun romacs-agent-tools-summary-tests--register (session)
  "Register and index SESSION in the private fixture."
  (puthash (romacs-agent-id session) session romacs-agent--sessions)
  (romacs-agent--index session)
  session)

(defun romacs-agent-tools-summary-tests--failed ()
  "Make a failed turn with a provider limit and a reply."
  (romacs-agent-tools-summary-tests--session
   6 :state 'idle :failed t :turns 1 :reply "Provider spent." :done-seq 19
   :ends '((19 "Provider spent." t))
   :limited '(:provider "claude" :text "Provider spent." :window "weekly")))

(defun romacs-agent-tools-summary-tests--completed ()
  "Make a completed task with non-default task metadata and a long reply."
  (let ((reply (concat "Completed.\n" (make-string 300 ?x))))
    (romacs-agent-tools-summary-tests--session
     5 :turns 3 :reply reply :cost 0.625 :harness-session "native-thread"
     :options (list :parent (romacs-agent-tools-summary-tests--id 1)
                    :model "requested-model" :role "reviewer"
                    :worktree "/var/tmp/review" :branch "review")
     :done-seq 17 :ends (list (list 17 reply nil "m-request")))))

(defun romacs-agent-tools-summary-tests--mirror ()
  "Make the actual client mirror of a service row for task 10."
  (romacs-agent-remote--mirror
   (romacs-agent-remote--make)
   (romacs-mcp--object
    "id" (romacs-agent-tools-summary-tests--id 10)
    "title" "Remote task" "harness" "codex" "provider" "remote-codex"
    "cwd" "/var/tmp/remote" "model" "remote-model" "state" "running"
    "parent" (romacs-agent-tools-summary-tests--id 1) "last_time" 510.0 "cost" 1.25)))

(defun romacs-agent-tools-summary-tests--scene ()
  "Seed an MCP caller, a tree of tasks, and a row with no session directory."
  (let ((parent (romacs-agent-tools-summary-tests--id 1)))
    (mapc #'romacs-agent-tools-summary-tests--register
          (list (romacs-agent-tools-summary-tests--session 1 :options nil :harness 'codex
                                                        :provider "lead-codex" :cwd "/var/tmp/parent")
                (romacs-agent-tools-summary-tests--session 2)
                (romacs-agent-tools-summary-tests--session
                 3 :state 'running :options (list :parent parent :wake "none"))
                (romacs-agent-tools-summary-tests--session 4 :state 'waiting-approval)
                (romacs-agent-tools-summary-tests--completed)
                (romacs-agent-tools-summary-tests--failed)
                (romacs-agent-tools-summary-tests--session 7 :cancelled t :turns 1)
                (romacs-agent-tools-summary-tests--session
                 8 :state 'idle :options (list :parent (romacs-agent-tools-summary-tests--id 2) :wake "failure"))
                (romacs-agent-tools-summary-tests--mirror)))
    (let ((waiting (gethash (romacs-agent-tools-summary-tests--id 4) romacs-agent--sessions))
          (queued (gethash (romacs-agent-tools-summary-tests--id 8) romacs-agent--sessions)))
      (puthash "request" t (romacs-agent-approvals waiting))
      (let ((romacs-agent--queue-quiet t))
        (romacs-agent--enqueue queued (romacs-agent-item "Queued" :delivered #'ignore))))
    (should (romacs-agent-store--writer-open))
    (dolist (task (list (romacs-agent-tools-summary-tests--session 9 :title "Row only" :model "stored-model")
                        (gethash (romacs-agent-tools-summary-tests--id 5) romacs-agent--sessions)))
      (should (romacs-agent-store--put task nil)))))

(defmacro romacs-agent-tools-summary-tests--with (&rest body)
  "Run BODY with private fixture sessions, store, and provider accounts."
  (declare (indent 0))
  `(romacs-agent-test-with-store
     (let ((romacs-agent--sessions (make-hash-table :test #'equal))
           (romacs-agent--children (make-hash-table :test #'equal))
           (romacs-agent-tools--pending (make-hash-table :test #'equal))
           (romacs-agent-tools--answered (make-hash-table :test #'equal))
           (romacs-agent--room (make-hash-table :test #'equal))
           (romacs-agent--room-loaded t)
           (romacs-agent-pools nil)
           (romacs-agent-providers '(("claude" :harness claude)
                                     ("claude2" :harness claude)
                                     ("codex" :harness codex)))
           (romacs-mcp--active nil)
           (romacs-mcp-failure-functions nil)
           (romacs-mcp-full-access t))
       (romacs-agent-tools-summary-tests--scene)
       (cl-letf (((symbol-function 'romacs-agent-tools--caller)
                  (lambda () (romacs-agent-tools-summary-tests--id 1))))
         ,@body))))

(defun romacs-agent-tools-summary-tests--data (value)
  "Canonicalize JSON VALUE for structural equality, preserving arrays."
  (cond
   ((hash-table-p value)
    (sort (mapcar (lambda (key)
                   (cons key (romacs-agent-tools-summary-tests--data (gethash key value))))
                 (hash-table-keys value))
          (lambda (a b) (string< (car a) (car b)))))
   ((vectorp value) (vconcat (mapcar #'romacs-agent-tools-summary-tests--data value)))
   (t value)))

(defun romacs-agent-tools-summary-tests--envelope (name args)
  "Call registered MCP tool NAME with ARGS."
  (romacs-mcp--call
   (romacs-mcp--object "name" name "arguments" (apply #'romacs-mcp--object args)) nil))

(defun romacs-agent-tools-summary-tests--text (envelope)
  "Read the text in MCP ENVELOPE."
  (gethash "text" (aref (gethash "content" envelope) 0)))

(defun romacs-agent-tools-summary-tests--call (name &rest args)
  "Call registered MCP tool NAME with ARGS and return its decoded payload."
  (let* ((envelope (romacs-agent-tools-summary-tests--envelope name args))
         (text (romacs-agent-tools-summary-tests--text envelope)))
    (ert-info (text)
      (should (eq (gethash "isError" envelope) :false)))
    (romacs-agent-tools-summary-tests--data
     (json-parse-string text :null-object :null :false-object :false))))

(defun romacs-agent-tools-summary-tests--status (number)
  "Return the MCP task status for fixture NUMBER."
  (romacs-agent-tools-summary-tests--call "romacs_task_status"
                                         "taskId" (romacs-agent-tools-summary-tests--id number)))

(defun romacs-agent-tools-summary-tests--wake (&optional guard)
  "Return the completed and failed children's delivery, using GUARD for the flush."
  (let* ((id (romacs-agent-tools-summary-tests--id 1))
         (parent (gethash id romacs-agent--sessions))
         (children (mapcar (lambda (number)
                             (gethash (romacs-agent-tools-summary-tests--id number) romacs-agent--sessions))
                           '(6 5)))
         (serial 0) delivery)
    (puthash id (mapcar (lambda (child) (cons child (car (romacs-agent-ends child)))) children)
             romacs-agent-tools--pending)
    (cl-letf (((symbol-function 'romacs-agent--uuid) (lambda () (format "wake-%d" (cl-incf serial))))
              ((symbol-function 'romacs-agent-tools--deliver)
               (lambda (_parent text _delivered _valid messages kind)
                 (should-not delivery)
                 (setq delivery (list text messages kind)))))
      (funcall (or guard #'funcall) (lambda () (romacs-agent-tools--flush parent))))
    delivery))

(defconst romacs-agent-tools-summary-tests--limit
  '(("alternatives" . [(("lossless" . t) ("provider" . "claude2"))
                       (("lossless" . :false) ("provider" . "codex"))])
    ("model" . :null) ("provider" . "claude") ("resets" . :null)
    ("text" . "Provider spent.") ("window" . "weekly"))
  "The provider-limit object captured from task 6.")

(defconst romacs-agent-tools-summary-tests--reply (concat "Completed.\n" (make-string 300 ?x))
  "The full reply captured from task 5.")

(defconst romacs-agent-tools-summary-tests--differences
  `((1 "parent" :null "harness" "codex" "provider" "lead-codex" "cwd" "/var/tmp/parent")
    (3 "state" "running" "status" "running"
       "next" "It will not wake you: read its result with romacs_task_status when you need it.")
    (4 "state" "waiting-approval" "status" "waiting" "pendingApproval" t)
    (5 "branch" "review" "conversation" "native-thread" "cost" 0.625 "model" "requested-model"
       "result" ,romacs-agent-tools-summary-tests--reply "role" "reviewer" "status" "completed"
       "turns" 3 "worktree" "/var/tmp/review")
    (6 "limit" ,romacs-agent-tools-summary-tests--limit "result" "Provider spent."
       "status" "failed" "turns" 1)
    (7 "status" "cancelled" "turns" 1)
    (8 "parent" ,(romacs-agent-tools-summary-tests--id 2) "status" "running"
       "next" "Only its failure wakes you, so end your turn now: do not wait on it or poll it.")
    (9 "title" "Row only" "model" "stored-model")
    (10 "cost" 1.25 "cwd" "/var/tmp/remote" "harness" "codex" "model" "remote-model"
        "next" "Its end wakes you with its result, so end your turn now: do not wait on it or poll it."
        "provider" "remote-codex" "state" "running" "status" "running" "title" "Remote task"))
  "Captured changes from the idle task's fields, by fixture number.")

(defun romacs-agent-tools-summary-tests--gold (number &optional row)
  "Captured answer for NUMBER, in task-list form when ROW is non-nil."
  (let* ((id (romacs-agent-tools-summary-tests--id number))
         (object (romacs-mcp--object
                  "branch" :null "conversation" id "cost" :null "cwd" "/var/tmp" "harness" "claude"
                  "limit" :null "model" :null "next" :null
                  "parent" (romacs-agent-tools-summary-tests--id 1) "pendingApproval" :false
                  "provider" "claude" "result" :null "role" :null "state" "idle" "status" "idle"
                  "taskId" id "title" (format "Task %d" number) "turns" 0 "worktree" :null)))
    (cl-loop for (key value) on (alist-get number romacs-agent-tools-summary-tests--differences)
             by #'cddr do (puthash key value object))
    (when row
      (remhash "next" object)
      (pcase number
        (5 (puthash "result" (concat "Completed.\n" (make-string 289 ?x) "…") object)
           (puthash "resultChars" 311 object))
        (6 (puthash "resultChars" 15 object))))
    (romacs-agent-tools-summary-tests--data object)))

(defun romacs-agent-tools-summary-tests--check-list ()
  "Check caller-only and all task lists, including the never-loaded row."
  (dolist (case '((nil 10 9 7 6 5 4 3 2) (t 10 9 8 7 6 5 4 3 2)))
    (ert-info ((format "all=%S" (car case)))
      (should
       (equal (romacs-agent-tools-summary-tests--call "romacs_task_list" "all" (or (car case) :false))
              (list (cons "nextOffset" :null)
                    (cons "tasks" (vconcat (mapcar (lambda (number)
                                                    (romacs-agent-tools-summary-tests--gold number t))
                                                  (cdr case))))
                    (cons "total" (length (cdr case))))))))
  (should-not (gethash (romacs-agent-tools-summary-tests--id 9) romacs-agent--sessions)))

(defun romacs-agent-tools-summary-tests--wake-gold ()
  "The parent's captured wake text and message records."
  (let ((id5 (romacs-agent-tools-summary-tests--id 5))
        (id6 (romacs-agent-tools-summary-tests--id 6))
        (success romacs-agent-tools-summary-tests--reply)
        (failure (concat "Provider spent.\n[Usage limit on claude. With room: "
                         "claude2 (the same conversation moves), codex (a handoff on a brief). "
                         "romacs_task_switch with that provider goes on there.]")))
    (list (format "Delegated task %s (Task 5) [completed, 311 chars, re m-request]:\n%s\n\nDelegated task %s (Task 6) [failed, 173 chars]:\n%s"
                  id5 success id6 failure)
          `((:id "m-wake-1" :kind "child"
                 :from (:session ,id5 :name "Task 5" :address nil :claude-session nil :by-user nil)
                 :in-reply-to "m-request" :text ,success :wake (:state "completed" :chars 311))
            (:id "m-wake-2" :kind "child"
                 :from (:session ,id6 :name "Task 6" :address nil :claude-session nil :by-user nil)
                 :in-reply-to nil :text ,failure :wake (:state "failed" :chars 173)))
          'next)))

(defun romacs-agent-tools-summary-tests--delegate ()
  "Run the delegation tool up to the launch boundary and capture its inputs."
  (let ((boundary (make-symbol "launch")))
    (cl-letf (((symbol-function 'romacs-agent-start)
               (lambda (cwd _brief title provider options)
                 (throw boundary (list cwd title provider (plist-get options :parent))))))
      (catch boundary
        (romacs-agent-tools-summary-tests--call "romacs_delegate_task" "task" "Review" "title" "Child")))))

(defvar romacs-agent-tools-summary-tests--record-access nil
  "Non-nil while a summary or a wake delivery bookkeeping function runs.")

(defun romacs-agent-tools-summary-tests--allow (function &rest args)
  "Allow FUNCTION with ARGS to read records during the trap."
  (let ((romacs-agent-tools-summary-tests--record-access t))
    (apply function args)))

(defun romacs-agent-tools-summary-tests--record-readers ()
  "Return the record lookups and generated session field readers."
  (append '(romacs-agent-get romacs-agent-loaded romacs-agent-load-session)
          (cl-loop for (slot) in (cl-struct-slot-info 'romacs-agent)
                   for reader = (intern (format "romacs-agent-%s" slot))
                   when (fboundp reader) collect reader)))

(defun romacs-agent-tools-summary-tests--guard (body &optional wake)
  "Run BODY with record reads trapped outside the summary.
WAKE permits ID routing, child enumeration and delivery deduplication."
  (let* ((tag (make-symbol "record-read"))
         (allowed (if wake '(romacs-agent-id romacs-agent-children
                             romacs-agent-tools--wanted-p romacs-agent-tools--unhanded)))
         (gateways (append '(romacs-agent-summary romacs-agent-tools--ask-owner)
                           (delq 'romacs-agent-id (copy-sequence allowed))))
         (readers (delq nil (mapcar (lambda (reader)
                                    (unless (memq reader allowed)
                                      (cons reader (lambda (&rest _)
                                                     (unless romacs-agent-tools-summary-tests--record-access
                                                       (throw tag (list :direct-record-read reader)))))))
                                  (romacs-agent-tools-summary-tests--record-readers))))
         (result
          (unwind-protect
              (progn
                (dolist (gateway gateways)
                  (when (fboundp gateway)
                    (advice-add gateway :around #'romacs-agent-tools-summary-tests--allow)))
                (dolist (entry readers) (advice-add (car entry) :before (cdr entry)))
                (catch tag (cons :answer (funcall body))))
            (dolist (entry readers) (advice-remove (car entry) (cdr entry)))
            (dolist (gateway gateways)
              (advice-remove gateway #'romacs-agent-tools-summary-tests--allow)))))
    (unless (eq (car result) :answer) (ert-fail result))
    (cdr result)))

(defmacro romacs-agent-tools-summary-tests--status-case (case number)
  "Define the golden and guarded status tests for CASE and fixture NUMBER."
  `(progn
     (ert-deftest ,(intern (format "romacs-agent-tools-summary-golden-%s" case)) ()
       (romacs-agent-tools-summary-tests--with
         (should (equal (romacs-agent-tools-summary-tests--status ,number)
                        (romacs-agent-tools-summary-tests--gold ,number)))))
     (ert-deftest ,(intern (format "romacs-agent-tools-summary-guarded-%s" case)) ()
       (romacs-agent-tools-summary-tests--with
         (should (equal (romacs-agent-tools-summary-tests--guard
                         (lambda () (romacs-agent-tools-summary-tests--status ,number)))
                        (romacs-agent-tools-summary-tests--gold ,number)))))))

(romacs-agent-tools-summary-tests--status-case idle 2)
(romacs-agent-tools-summary-tests--status-case running 3)
(romacs-agent-tools-summary-tests--status-case waiting 4)
(romacs-agent-tools-summary-tests--status-case completed 5)
(romacs-agent-tools-summary-tests--status-case failed 6)
(romacs-agent-tools-summary-tests--status-case cancelled 7)
(romacs-agent-tools-summary-tests--status-case parent 1)
(romacs-agent-tools-summary-tests--status-case queued 8)
(romacs-agent-tools-summary-tests--status-case remote-mirror 10)

(ert-deftest romacs-agent-tools-summary-golden-list ()
  (romacs-agent-tools-summary-tests--with
    (romacs-agent-tools-summary-tests--check-list)))

(ert-deftest romacs-agent-tools-summary-guarded-list ()
  (romacs-agent-tools-summary-tests--with
    (romacs-agent-tools-summary-tests--guard #'romacs-agent-tools-summary-tests--check-list)))

(ert-deftest romacs-agent-tools-summary-golden-wake ()
  (romacs-agent-tools-summary-tests--with
    (should (equal (romacs-agent-tools-summary-tests--wake)
                   (romacs-agent-tools-summary-tests--wake-gold)))))

(ert-deftest romacs-agent-tools-summary-guarded-wake ()
  (romacs-agent-tools-summary-tests--with
    (should (equal (romacs-agent-tools-summary-tests--wake
                    (lambda (flush) (romacs-agent-tools-summary-tests--guard flush t)))
                   (romacs-agent-tools-summary-tests--wake-gold)))))

(ert-deftest romacs-agent-tools-summary-golden-prefix ()
  (romacs-agent-tools-summary-tests--with
    (should (equal (romacs-agent-tools-summary-tests--call "romacs_task_status" "taskId" "00000005")
                   (romacs-agent-tools-summary-tests--gold 5)))))

(ert-deftest romacs-agent-tools-summary-guarded-prefix ()
  (romacs-agent-tools-summary-tests--with
    (should (equal (romacs-agent-tools-summary-tests--guard
                    (lambda ()
                      (romacs-agent-tools-summary-tests--call "romacs_task_status" "taskId" "00000005")))
                   (romacs-agent-tools-summary-tests--gold 5)))))

(ert-deftest romacs-agent-tools-summary-golden-delegate-parent ()
  (romacs-agent-tools-summary-tests--with
    (should (equal (romacs-agent-tools-summary-tests--delegate)
                   (list "/var/tmp/parent" "Child" 'codex (romacs-agent-tools-summary-tests--id 1))))))

(ert-deftest romacs-agent-tools-summary-guarded-delegate-parent ()
  (romacs-agent-tools-summary-tests--with
    (should (equal (romacs-agent-tools-summary-tests--guard #'romacs-agent-tools-summary-tests--delegate)
                   (list "/var/tmp/parent" "Child" 'codex (romacs-agent-tools-summary-tests--id 1))))))

(defun romacs-agent-tools-summary-tests--owner-refusal ()
  "Ask to cancel another caller's child and check the route in the refusal."
  (let* ((owner (romacs-agent-tools-summary-tests--id 2))
         (target (romacs-agent-tools-summary-tests--id 8)))
    (setf (romacs-agent-state (gethash owner romacs-agent--sessions)) 'starting)
    (cl-letf (((symbol-function 'romacs-agent-tools--caller)
               (lambda () (romacs-agent-tools-summary-tests--id 3))))
      (let ((answer (romacs-agent-tools-summary-tests--envelope
                     "romacs_task_cancel" (list "taskId" target))))
        (should (eq (gethash "isError" answer) t))
        (should (equal (romacs-agent-tools-summary-tests--text answer)
                       (format "Task %s belongs to session %s, and among agents only the session that delegated it may cancel it. Please ask that session with romacs_task_send."
                               target owner)))))))

(ert-deftest romacs-agent-tools-summary-golden-owner-route ()
  (romacs-agent-tools-summary-tests--with
    (romacs-agent-tools-summary-tests--owner-refusal)))

(ert-deftest romacs-agent-tools-summary-guarded-owner-route ()
  (romacs-agent-tools-summary-tests--with
    (let* (ran
           (guard (lambda (route &rest args)
                    (setq ran t)
                    (romacs-agent-tools-summary-tests--guard (lambda () (apply route args))))))
      (unwind-protect
          (progn
            (advice-add 'romacs-agent-tools--owner-route :around guard)
            (romacs-agent-tools-summary-tests--owner-refusal)
            (should ran))
        (advice-remove 'romacs-agent-tools--owner-route guard)))))

(defun romacs-agent-tools-summary-tests--slot (slot number)
  "Read summary SLOT for fixture NUMBER through the accessor."
  (let ((accessor (intern (format "romacs-agent-summary-%s" slot)))
        (summary (romacs-agent-summary (romacs-agent-tools-summary-tests--id number))))
    (should summary)
    (should (fboundp accessor))
    (funcall accessor summary)))

(defmacro romacs-agent-tools-summary-tests--slot-case (slot number &rest body)
  "Compare SLOT for NUMBER with the corresponding MCP evidence in BODY.
The comparison can refer to `value' and `task'."
  (declare (indent 2))
  `(ert-deftest ,(intern (format "romacs-agent-tools-summary-slot-%s" slot)) ()
     (romacs-agent-tools-summary-tests--with
       (let ((task (romacs-agent-tools-summary-tests--status ,number))
             (value (romacs-agent-tools-summary-tests--slot ',slot ,number)))
         ,@body))))

(romacs-agent-tools-summary-tests--slot-case turns 5
  (should (= value 3))
  (should (= value (alist-get "turns" task nil nil #'equal))))

(romacs-agent-tools-summary-tests--slot-case reply 5
  (should (equal value (alist-get "result" task nil nil #'equal))))

(romacs-agent-tools-summary-tests--slot-case role 5
  (should (equal value (alist-get "role" task nil nil #'equal))))

(romacs-agent-tools-summary-tests--slot-case worktree 5
  (should (equal value (alist-get "worktree" task nil nil #'equal))))

(romacs-agent-tools-summary-tests--slot-case branch 5
  (should (equal value (alist-get "branch" task nil nil #'equal))))

(romacs-agent-tools-summary-tests--slot-case conversation 5
  (should (equal value (alist-get "conversation" task nil nil #'equal))))

(romacs-agent-tools-summary-tests--slot-case requested-model 5
  (should (equal value (alist-get "model" task nil nil #'equal))))

(romacs-agent-tools-summary-tests--slot-case cancelled 7
  (should (eq (and value t) (equal (alist-get "status" task nil nil #'equal) "cancelled"))))

(romacs-agent-tools-summary-tests--slot-case wake-rule 3
  (should (equal (cons value (alist-get "next" task nil nil #'equal))
                 '("none" . "It will not wake you: read its result with romacs_task_status when you need it."))))

(romacs-agent-tools-summary-tests--slot-case limited 6
  (let ((limit (alist-get "limit" task nil nil #'equal)))
    (dolist (key '("provider" "model" "text" "window" "resets"))
      (ert-info (key)
        (should (equal (or (plist-get value (intern (concat ":" key))) :null)
                       (alist-get key limit nil nil #'equal)))))))

(romacs-agent-tools-summary-tests--slot-case alternatives 6
  (let ((limit (alist-get "limit" task nil nil #'equal)))
    (should (equal (vconcat
                    (mapcar (lambda (entry)
                              `(("lossless" . ,(if (plist-get entry :lossless) t :false))
                                ("provider" . ,(plist-get entry :provider))))
                            value))
                   (alist-get "alternatives" limit nil nil #'equal)))))

(romacs-agent-tools-summary-tests--slot-case waiting-count 4
  (should (eq (> value 0) (alist-get "pendingApproval" task nil nil #'equal))))

(provide 'romacs-agent-tools-summary-tests)
;;; romacs-agent-tools-summary-tests.el ends here
