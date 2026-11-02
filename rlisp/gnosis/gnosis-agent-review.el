;;; gnosis-agent-review.el --- Session-local adaptive practice -*- lexical-binding: t; -*-
;; Maintainer: Thanos Apollo <public@thanosapollo.org>

;; Copyright (C) 2026 Free Software Foundation, Inc.
;; Author: Thanos Apollo <public@thanosapollo.org>
;; Keywords: extensions
;; URL: https://thanosapollo.org/projects/gnosis

;; This program is free software; you can redistribute it and/or modify
;; it under the terms of the GNU General Public License as published by
;; the Free Software Foundation, either version 3 of the License, or
;; (at your option) any later version.
;; This program is distributed in the hope that it will be useful,
;; but WITHOUT ANY WARRANTY; without even the implied warranty of
;; MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE.  See the
;; GNU General Public License for more details.
;; You should have received a copy of the GNU General Public License
;; along with this program.  If not, see <https://www.gnu.org/licenses/>.

;;; Commentary:

;; Native review with local question values, never temporary collection rows.
;; Native matching is immediate; semantic evaluation includes a provisional
;; unseen-tail adjustment in one turn.  Next alone accepts.  Optional native
;; adaptation runs behind prepared input; callbacks only stage proposals.
;; Version 3 blobs retain evidence without collection rows or FSRS writes.
;; Legacy version 2 sessions keep their original two-phase protocol.

;;; Code:

(require 'gnosis-agent)
(require 'gnosis-agent-eval)
(require 'gnosis-agent-review-data)

(autoload 'gnosis-agent-review-hermes "gnosis-agent-review-hermes")

(defcustom gnosis-agent-review-function #'gnosis-agent-review-hermes
  "Function providing contextual evaluation and adaptation.
Call with REQUEST, RESOLVE, REJECT; return an idempotent cancel function.
REQUEST includes :api-version, :phase, session/occurrence/request IDs,
revision, goal, source, context, accepted transcript and remaining questions.
API 2 evaluate supplies captured :question and :response; resolve exactly
  (:api-version 2 :evaluation (:verdict STRING :explanation STRING)
   :adjustment PLAN-OR-NIL).  Verdict is pass, fail or ungradable.  The plan is
provisional until native acceptance; override or editing discards it.
API 2 adapt follows native local acceptance and resolves PLAN directly.
PLAN has exactly :agent-note, :context (object or nil), :remaining (vector)
and :done (t iff remaining is empty, otherwise :false).  Only the supplied
unseen tail may change; captured/prepared IDs cannot be requeued.
Legacy saved sessions use API 1: evaluate resolves only the evaluation,
then adapt supplies a plan after acceptance.  Ordinary agent-eval is separate.
Never mutate requests or write grades.  Errors preserve answers for retry."
  :type 'function :group 'gnosis)

(defvar gnosis-agent-review--providers nil
  "Process-local provider records qualified by open connection and session.
An explicitly retired record retains a nil function to refuse fallback.")

(defun gnosis-agent-review--provider-entry (state)
  "Return the provider record for STATE, creating an unbound one if absent."
  (or (seq-find (lambda (entry)
                  (and (eq (plist-get entry :connection)
                           (gnosis-review-state-database state))
                       (equal (plist-get entry :session-id)
                              (gnosis-review-state-session-id state))))
                gnosis-agent-review--providers)
      (let ((entry (list :connection (gnosis-review-state-database state)
                         :session-id (copy-sequence (gnosis-review-state-session-id state))
                         :explicit nil :function nil :pending nil)))
        (push entry gnosis-agent-review--providers)
        entry)))

;;;###autoload
(defun gnosis-agent-review-bind-provider (session-id connection function)
  "Bind local SESSION-ID on open CONNECTION to FUNCTION.
FUNCTION receives REQUEST, RESOLVE and REJECT and returns a cancellation
function, as in `gnosis-agent-review-function'.  Both phases use this
binding, independent of the global default.  Refuse a historical, cancelled
or completed session, a closed connection, or replacement during a pending
request.  Binding is process-local: reopening a connection requires explicit
reattachment before resume.

Return an idempotent unbind function.  It retires this binding and pending
callback authority before reporting provider loss and cancelling transport.
It cannot retire a successor binding.  Explicitly unbound sessions refuse
requests until rebound; they never silently use the global default."
  (unless (and (sqlitep connection) (functionp function))
    (user-error "Provide an open SQLite connection and provider function"))
  ;; Read the supplied connection, never initialize or reopen an ambient DB.
  (sqlite-select connection "SELECT 1")
  (let* ((gnosis-db connection)
         (state (gnosis-agent--session session-id))
         (current (gnosis-review--read-session)))
    (gnosis-agent-review--data state)
    (unless (and current
                 (equal session-id (gnosis-review-state-session-id current))
                 (not (gnosis-review-state-cancelled-p state))
                 (gnosis-review--work-p state))
      (user-error "Bind only the current unfinished local session"))
    (let* ((entry (gnosis-agent-review--provider-entry state))
           (binding (list function)))
      (when (plist-get entry :pending)
        (user-error "A provider request is still pending"))
      (setf (plist-get entry :explicit) t
            (plist-get entry :function) binding)
      (lambda ()
        (when (eq binding (plist-get entry :function))
          (setf (plist-get entry :function) nil)
          (when-let* ((pending (plist-get entry :pending)))
            (funcall pending t)))))))

(defun gnosis-agent-review--call-provider (state request resolve reject)
  "Call STATE's provider with REQUEST, RESOLVE and REJECT.
Retain pending ownership through synchronous delivery and transport startup.
Return an idempotent cancellation function; retire before invoking cleanup."
  (unless (eq gnosis-db (gnosis-review-state-database state))
    (user-error "Local study database changed"))
  (let* ((entry (gnosis-agent-review--provider-entry state))
         (function (if (plist-get entry :explicit)
                       (car (plist-get entry :function))
                     gnosis-agent-review-function))
         (active t) stopped cancel retire)
    (unless function (user-error "Session provider was unbound; reattach before retrying"))
    (when (plist-get entry :pending) (user-error "A provider request is still pending"))
    (cl-labels ((release ()
                  (setq active nil)
                  (when (eq retire (plist-get entry :pending))
                    (setf (plist-get entry :pending) nil)))
                (deliver (callback value)
                  (when active
                    (release)
                    (funcall callback value))))
      (setq retire
            (lambda (&optional lost)
              (unless stopped
                (let ((notify (and active lost)))
                  (setq stopped t)
                  (release)
                  (unwind-protect
                      (when notify (funcall reject "Session provider was unbound"))
                    (gnosis-agent-eval--cancel cancel))))))
      (setf (plist-get entry :pending) retire)
      (condition-case err
          (let ((handle (funcall function request
                                 (lambda (value) (deliver resolve value))
                                 (lambda (value) (deliver reject value)))))
            (unless (functionp handle)
              (error "Provider did not return a cancellation function"))
            ;; Unbinding can occur inside FUNCTION before its handle exists.
            (if stopped (gnosis-agent-eval--cancel handle)
              (setq cancel handle))
            retire)
        ((error quit)
         (funcall retire)
         (signal (car err) (cdr err)))))))

(defun gnosis-agent-review--data (state)
  "Return local payload of STATE, refusing collection sessions."
  (or (gnosis-review-state-agent-review state)
      (user-error "Not a session-local agent review")))

(defun gnosis-agent-review--commit (state data remaining)
  "Commit fresh DATA and REMAINING only while STATE owns its checkpoint.
Publish into the UI object only after the whole transaction commits."
  (unless (eq (gnosis-review-state-database state) gnosis-db)
    (user-error "Local study database changed; no checkpoint was written"))
  (let ((next (copy-gnosis-review-state state)))
    (setf (plist-get data :revision) (1+ (plist-get (gnosis-agent-review--data state) :revision))
          (gnosis-review-state-agent-review next) data
          (gnosis-review-state-remaining next) remaining
          (gnosis-review-state-reviewed next) (length (plist-get data :encounters))
          (gnosis-review-state-total next) (+ (length (plist-get data :encounters)) (length remaining)))
    (gnosis-agent-review--validate data remaining)
    (gnosis-sqlite-with-transaction (gnosis-review-state-database state)
				    (gnosis-review--check-database state)
				    (gnosis-review--check-action-target
				     (cons (gnosis-review-state-database state) (gnosis-review--state-data state)))
				    (gnosis-review--save-session next))
    (gnosis-review--copy-state state next)))

;;;###autoload
(cl-defun gnosis-agent-start-review-session (&key questions goal source provider)
  "Reserve local QUESTIONS with teaching GOAL and fixed SOURCE context.
QUESTIONS is a nonempty vector (at most 128) of exact keyword records:
:id, :type, :question, :answer (string vector), :choices (string vector),
:rubric, :hints (vector), :parathema (string or nil), :tags (vector), :source.
Types are basic, cloze, mcq, mc-cloze or agent-eval.  Only cloze has multiple
answers.  MC types require choices including their single answer; otherwise
choices is empty.  Only agent-eval has a nonempty rubric; others use nil.
Legacy records without :type retain :reference-answer and no answer/choices.
New sessions use combined provider API 2 and saved format 3.
Return JSON-safe status immediately and defer native answering.  No permanent
themata or grades are created.  An unfinished previous batch ends early only
after validation succeeds.  Use `gnosis-agent-resume' and `gnosis-agent-cancel'
for recovery.  Content remains local unless explicitly saved while paused.
Optional PROVIDER follows `gnosis-agent-review-function' and is bound to the
exact session before scheduling, including reentrant startup.  A failed
startup retires that binding; resume requires explicit reattachment."
  (when (and provider (not (functionp provider)))
    (user-error "Provider must be a function"))
  (let* ((target (gnosis-review--session-target))
         (questions (gnosis-agent-review--copy (gnosis-agent-review--questions questions)))
         (ids (mapcar (lambda (q) (plist-get q :id)) (append questions nil)))
         (data (list :protocol 2 :proposal nil :phase "question" :revision 0
                     :goal (gnosis-agent-review--copy (gnosis-agent-review--text goal))
                     :source (gnosis-agent-review--copy (gnosis-agent-review--text source)) :context nil
                     :questions questions :current nil :attempts [] :encounters []
                     :adaptations [] :saves [] :agent-note ""))
         (state (gnosis-review-state-create
                 :mode 'practice :database (car target) :persistent-p t
                 :session-id (gnosis-scheduler-event-id) :event-id (gnosis-scheduler-event-id)
                 :initial (length ids) :total (length ids) :selected ids :remaining ids
                 :agent-review data)))
    (unless ids (user-error "Provide at least one local question"))
    (gnosis-agent-review--validate data ids)
    (gnosis-review--replace-session state target)
    (let ((unbind (when provider
                    (gnosis-agent-review-bind-provider
                     (gnosis-review-state-session-id state)
                     (gnosis-review-state-database state) provider)))
          started)
      (unwind-protect
          (prog1 (progn
                   (gnosis-agent--schedule state)
                   (gnosis-agent-review-status (gnosis-review-state-session-id state)))
            (setq started t))
        (unless started (when unbind (funcall unbind)))))))

;;;###autoload
(defun gnosis-agent-review-status (session-id)
  "Return JSON-safe status of exact local SESSION-ID, including pending adaptation.
A process restart leaves unfinished work, not an active provider request.
No content is written by this read.  Use explicit results to inspect answers."
  (let* ((state (gnosis-agent--session session-id))
         (data (gnosis-agent-review--data state))
         (pending (seq-some
                   (lambda (r) (and (eq gnosis-db (plist-get r :db))
                                    (equal session-id (plist-get r :session-id))
                                    (equal (gnosis-review-state-launch-token state)
                                           (plist-get r :token)))) gnosis-agent--launches)))
    (list :api-version 1 :session-id session-id :mode "agent-review"
          :database (nth 2 (assoc 0 (sqlite-select gnosis-db "PRAGMA database_list")))
          :status (cond ((gnosis-review-state-cancelled-p state) "cancelled")
                        ((equal gnosis-review--running session-id) "running")
                        ((not (gnosis-review--work-p state)) "completed")
                        (pending "pending") (t "unfinished"))
          :phase (plist-get data :phase) :revision (plist-get data :revision)
          :schedule-updated :false :accepted (length (plist-get data :encounters))
          :remaining-ids (vconcat (gnosis-review-state-remaining state))
          :question-count (length (plist-get data :questions))
          :presented (+ (length (plist-get data :encounters)) (if (plist-get data :current) 1 0))
          :current-question-id (plist-get (plist-get data :current) :id))))

;;;###autoload
(defun gnosis-agent-review-results (session-id)
  "Return status and exact retained local evidence for SESSION-ID.
The :study object contains captured questions, actual response drafts,
submitted attempts, accepted outcomes/overrides, shown help, source context,
agent context, adaptation plans and explicit-save receipts.  Evidence is
session-local, excluded from collection practice events and all FSRS replay.
Use `json-serialize' with :false-object :false and :null-object nil."
  (append (gnosis-agent-review-status session-id)
          (list :study (gnosis-agent-review--copy (gnosis-agent-review--data
						   (gnosis-agent--session session-id))))))

(defun gnosis-agent-review--validator (state occurrence)
  "Capture STATE's native lifetime and current OCCURRENCE."
  (let ((check (gnosis-review--session-validator state)))
    (lambda ()
      (unless (eq (gnosis-review-state-database state) gnosis-db)
        (user-error "Local study database changed"))
      (funcall check)
      (unless (equal occurrence (plist-get (plist-get (gnosis-agent-review--data state)
                                                      :current) :occurrence))
        (user-error "Local question occurrence changed")))))

(defun gnosis-agent-review--present (state)
  "Checkpoint the next immutable presentation in STATE, unless restoring one."
  (unless (plist-get (gnosis-agent-review--data state) :current)
    (let* ((data (gnosis-agent-review--copy (gnosis-agent-review--data state)))
           (id (car (gnosis-review-state-remaining state)))
           (question (gnosis-agent-review--find data id)))
      (setf (plist-get data :current)
            (list :id id :occurrence (gnosis-scheduler-event-id) :response ""
                  :evaluation nil :outcome nil :help (plist-get question :hints) :source-viewed :false))
      (gnosis-agent-review--commit state data (gnosis-review-state-remaining state)))))

(defun gnosis-agent-review--request (state phase request-id &optional response)
  "Build contextual request from STATE for PHASE and REQUEST-ID with RESPONSE."
  (let* ((data (gnosis-agent-review--data state))
         (current (plist-get data :current))
         (encounters (plist-get data :encounters)))
    (gnosis-agent-review--copy
     (list :api-version (or (plist-get data :protocol) 1) :phase phase :session-id (gnosis-review-state-session-id state)
           :request-id request-id :revision (plist-get data :revision)
           :occurrence-id (if current (plist-get current :occurrence)
                            (plist-get (aref encounters (1- (length encounters))) :occurrence))
           :goal (plist-get data :goal) :source (plist-get data :source)
           :context (plist-get data :context) :transcript encounters
           :question (when current (gnosis-agent-review--find data (plist-get current :id)))
           :response response
           :accepted (when (> (length encounters) 0) (aref encounters (1- (length encounters))))
           :remaining (vconcat (mapcar (lambda (id) (gnosis-agent-review--find data id))
                                       (if (plist-get data :protocol)
                                           (cdr (gnosis-review-state-remaining state))
                                         (gnosis-review-state-remaining state))))))))

(defun gnosis-agent-review--checkpoint (state occurrence event response result &optional failure request-id plan)
  "Retain EVENT, RESPONSE and provisional RESULT for STATE's OCCURRENCE.
Native evaluator symbols are converted to string enums only at this boundary.
FAILURE is retained as an ungraded attempt diagnostic.  REQUEST-ID identifies
the submitted evaluation; only its still-pending attempt may settle or cancel.
PLAN is the combined response's provisional unseen-tail adjustment."
  (let* ((data (gnosis-agent-review--copy (gnosis-agent-review--data state)))
         (current (plist-get data :current))
         (evaluation (when result
                       (list :verdict (symbol-name (plist-get result :verdict))
                             :explanation (plist-get result :explanation)))))
    (unless (equal occurrence (plist-get current :occurrence))
      (user-error "Response belongs to an outdated occurrence"))
    (gnosis-agent-review--text response t)
    (setf (plist-get current :response) response
          (plist-get current :evaluation) evaluation)
    (when (and (plist-get data :protocol) (member event '("submit" "cancel")))
      (setf (plist-get data :proposal) nil))
    (when (equal event "cancel") (setf (plist-get current :outcome) nil))
    (when (equal event "submit")
      (setf (plist-get current :outcome) nil
            (plist-get data :attempts)
            (vconcat (plist-get data :attempts)
                     (vector (list :phase "evaluate" :occurrence occurrence
                                   :request-id request-id
                                   :response response :evaluation nil :error nil :plan nil
                                   :context (plist-get data :context))))))
    (when (and (member event '("settle" "pause")) evaluation)
      (setf (plist-get current :outcome)
            (or (plist-get current :outcome)
                (pcase (plist-get evaluation :verdict)
                  ("pass" "success") ("fail" "failure")))))
    (when (member event '("settle" "cancel"))
      (let* ((attempts (plist-get data :attempts))
             (last (and (> (length attempts) 0) (aref attempts (1- (length attempts))))))
        (when (and request-id
                   (equal request-id (plist-get last :request-id))
                   (equal "evaluate" (plist-get last :phase))
                   (equal occurrence (plist-get last :occurrence))
                   (not (plist-get last :evaluation))
                   (not (plist-get last :error)))
          (setf (plist-get last :evaluation) evaluation
                (plist-get last :error) failure)
          (when (and plan (plist-get current :outcome) (equal event "settle"))
            (gnosis-agent-review--plan data plan)
            (setf (plist-get last :plan) (gnosis-agent-review--copy plan)
                  (plist-get data :proposal)
                  (list :occurrence occurrence :response response
                        :outcome (plist-get current :outcome)
                        :remaining (vconcat (cdr (gnosis-review-state-remaining state)))
                        :plan (gnosis-agent-review--copy plan)))))))
    (gnosis-agent-review--commit state data (gnosis-review-state-remaining state))))

(defun gnosis-agent-review--note (text validate &optional label authored)
  "Display TEXT using VALIDATE, with LABEL or a teaching-note label.
Keep TEXT literal unless AUTHORED is non-nil, for Org explanation content."
  (when (and text (not (string-empty-p text)))
    (gnosis-review--append-section
     (gnosis-review--format-string
      (concat (propertize (concat (or label "Agent teaching note") ": ")
                          'face 'gnosis-face-directions)
              (if authored (gnosis-org-format-string text) text))
      (not authored))
     validate "\n\n" "\n")))

(defun gnosis-agent-review--response-text (question response)
  "Return readable literal input from QUESTION's captured RESPONSE.
Cloze evidence retains blank bookkeeping as JSON; display only its typed
text, in submission order, without changing that evidence or rematching it."
  (if (or (not (equal (gnosis-agent-review--type question) "cloze"))
          (string-empty-p response))
      response
    (mapconcat (lambda (input) (plist-get input :text))
               (json-parse-string response :object-type 'plist
                                  :array-type 'array :null-object nil)
               "\n")))

(defun gnosis-agent-review--native-input (state question validate)
  "Read STATE's QUESTION using native input and matching, checking VALIDATE.
Retain partial cloze inputs between pauses.  No provider or collection writes."
  (let* ((current (plist-get (gnosis-agent-review--data state) :current))
         (occurrence (plist-get current :occurrence))
         (type (gnosis-agent-review--type question))
         (answers (append (gnosis-agent-review--answers question) nil))
         (hints (append (plist-get question :hints) nil))
         (prompt (plist-get question :question))
         (tolerance gnosis-string-difference)
         (response (plist-get current :response))
         (evaluation (plist-get current :evaluation))
         success)
    (unless (member type '("cloze" "mc-cloze"))
      (gnosis-display-keimenon prompt)
      (gnosis-display-hint (mapconcat #'identity hints "\n")))
    (unless evaluation
      (pcase type
        ("basic"
         (setq response (gnosis--read-string-with-input-method "Answer: " (car answers)))
         (funcall validate)
         (setq success (gnosis-answer-match-p (car answers) response nil tolerance)))
        ((or "mcq" "mc-cloze")
         (when (equal type "mc-cloze")
           (gnosis-display-cloze-string prompt answers nil nil nil))
         (setq response (gnosis-completing-read "Select answer: "
                                               (append (plist-get question :choices) nil)))
         (funcall validate)
         (setq success (equal response (car answers))))
        ("cloze"
         (let ((indices (number-sequence 0 (1- (length answers))))
               (inputs (unless (string-empty-p response)
                         (append (json-parse-string response :object-type 'plist
                                                   :array-type 'array :null-object nil) nil))))
           ;; Reuse captured blank identity, never rematch previous input.
           (dolist (input inputs)
             (setq indices (remq (plist-get input :matched-blank-index) indices)))
           (setq success t)
           (gnosis-display-cloze-string prompt answers hints nil nil (vconcat indices))
           (while (and indices success)
             (let* ((remaining (mapcar (lambda (i) (nth i answers)) indices))
                    (input (let ((gnosis-string-difference tolerance))
                             (gnosis-review-cloze--input remaining)))
                    (index (and (car input) (nth (car input) indices))))
               (funcall validate)
               (setq inputs (append inputs (list (list :text (cdr input)
                                                       :matched-blank-index index
                                                       :remaining-blank-indices (vconcat indices)))))
               (setq response (decode-coding-string
                               (json-serialize (vconcat inputs) :null-object nil) 'utf-8))
               (if index (setq indices (remq index indices)) (setq success nil))
               ;; A wrong final answer is checkpointed with its verdict below.
               (when success
                 (gnosis-agent-review--checkpoint state occurrence "pause" response nil))
               (gnosis-display-cloze-string prompt answers hints nil (not success) (vconcat indices)))))))
      (funcall validate)
      (gnosis-agent-review--checkpoint
       state occurrence "settle" response
       (list :verdict (if success 'pass 'fail)
             :explanation (if (member type '("basic" "cloze"))
                              (format "Native text matching (tolerance %s)." tolerance)
                            "Native exact choice matching."))))
    (when (member type '("cloze" "mc-cloze"))
      (gnosis-display-keimenon prompt))))

(defvar gnosis-agent-review--background nil
  "One run-owned opportunistic adjustment; callbacks only stage plain data.")

(defun gnosis-agent-review--stop-background ()
  "Retire the run's adjustment before callback-capable transport cleanup."
  (when-let* ((record gnosis-agent-review--background))
    (setq gnosis-agent-review--background nil)
    (setf (plist-get record :active) nil)
    (gnosis-agent-eval--cancel (plist-get record :cancel))))

(defun gnosis-agent-review--start-background (state)
  "Start optional adjustment after STATE's native answer, reserving next item.
Prepared input proceeds even when transport is unavailable or never responds.
For protocol 2, skip dispatch if either of the next two items is semantic:
its combined evaluation can teach from the accepted native transcript and
adjust the later tail without optional work occupying the shared provider.
More distant semantic items do not suppress useful native-only adaptation."
  (unless (or gnosis-agent-review--background
              (and (equal (plist-get (gnosis-agent-review--data state) :protocol) 2)
                   (seq-some
                    (lambda (id)
                      (equal (gnosis-agent-review--type
                              (gnosis-agent-review--find (gnosis-agent-review--data state) id))
                             "agent-eval"))
                    (seq-take (gnosis-review-state-remaining state) 2))))
    (let* ((data (gnosis-agent-review--data state))
           (request-id (gnosis-scheduler-event-id))
           (request (gnosis-agent-review--request state "adapt" request-id))
           (record (list :active t :settled nil :cancel nil :plan nil :error nil
                         :state state :validate (gnosis-review--session-validator state)
                         :request-id request-id
                         :remaining (copy-sequence (gnosis-review-state-remaining state))
                         :encounters (gnosis-agent-review--copy (plist-get data :encounters))
                         :context (gnosis-agent-review--copy (plist-get data :context))
                         :deadline (+ (float-time) gnosis-agent-eval-timeout))))
      (setq gnosis-agent-review--background record)
      (condition-case err
          (let ((cancel
                 (gnosis-agent-review--call-provider
                  state request
                  (lambda (value)
                    (when (plist-get record :active)
                      (setf (plist-get record :settled) t
                            (plist-get record :active) nil
                            (plist-get record :plan) (gnosis-agent-review--copy value))))
                  (lambda (failure)
                    (when (plist-get record :active)
                      (setf (plist-get record :settled) t
                            (plist-get record :active) nil
                            (plist-get record :error)
                            (if (stringp failure) failure "Adjustment unavailable")))))))
            (if (eq record gnosis-agent-review--background)
                (setf (plist-get record :cancel) cancel)
              (gnosis-agent-eval--cancel cancel)))
        (error (setf (plist-get record :settled) t
                     (plist-get record :active) nil
                     (plist-get record :error) (error-message-string err)))))))

(defun gnosis-agent-review--drain-background (state)
  "Apply a staged proposal only at STATE's native boundary with exact fences.
Keep the reserved first item/current presentation; reject a consumed tail."
  (when-let* ((record gnosis-agent-review--background))
    (when (or (plist-get record :settled)
              (>= (float-time) (plist-get record :deadline)))
      (unless (and (eq state (plist-get record :state))
                   (eq gnosis-db (gnosis-review-state-database state)))
        (user-error "Adjustment belongs to another session or database"))
      (let* ((validate (plist-get record :validate))
             (checkpoint (copy-tree (gnosis-review--state-data state)))
             (data (gnosis-agent-review--copy (gnosis-agent-review--data state)))
             (remaining (gnosis-review-state-remaining state))
             (plan (plist-get record :plan))
             (failure (or (plist-get record :error)
                          (unless (plist-get record :settled) "Adjustment timed out"))))
        (funcall validate checkpoint)
        (gnosis-agent-review--stop-background)
        (funcall validate checkpoint)
        (unless (and (equal remaining (plist-get record :remaining))
                     (equal (plist-get data :encounters) (plist-get record :encounters))
                     (equal (plist-get data :context) (plist-get record :context)))
          (setq failure "Adjustment discarded: prepared queue or accepted evidence changed"))
        (unless failure
          (condition-case err
              (progn
                (when (seq-some (lambda (q) (equal (plist-get q :id) (car remaining)))
                                (plist-get plan :remaining))
                  (user-error "Adjustment cannot replace the prepared question"))
                (let* ((next (gnosis-agent-review--plan data plan))
                       (current (plist-get data :current)))
                  (setq remaining (cons (car remaining)
                                        (mapcar (lambda (q) (plist-get q :id))
                                                (append (plist-get plan :remaining) nil))))
                  (setf (plist-get next :current) current
                        (plist-get next :phase) "question")
                  (setq data next)))
            (error (setq failure (error-message-string err)))))
        (setf (plist-get data :attempts)
              (vconcat (plist-get data :attempts)
                       (vector (list :phase "adapt" :occurrence nil
                                     :request-id (plist-get record :request-id)
                                     :response nil :evaluation nil :error failure
                                     :plan (unless failure plan) :context (plist-get record :context)))))
        (gnosis-agent-review--commit state data remaining)))))

(defun gnosis-agent-review--question-input (state)
  "Present STATE's current question through the shared native response reader."
  (gnosis-agent-review--present state)
  (let* ((data (gnosis-agent-review--data state))
         (current (plist-get data :current))
         (id (plist-get current :id))
         (occurrence (plist-get current :occurrence))
         (question (gnosis-agent-review--find data id))
         (validate (gnosis-agent-review--validator state occurrence))
         (gnosis-review--display-buffer (current-buffer))
         (gnosis-review--display-validate validate)
         (evaluation (plist-get current :evaluation))
         pending-request pending-plan
         (restored (list :response (plist-get current :response)
                         :result (when (member (plist-get evaluation :verdict) '("pass" "fail"))
                                   (list :verdict (intern (plist-get evaluation :verdict))
                                         :explanation (plist-get evaluation :explanation))))))
    (if (not (equal (gnosis-agent-review--type question) "agent-eval"))
        (gnosis-agent-review--native-input state question validate)
      (let ((checkpoint (copy-tree (gnosis-review--state-data state)))
            (check (gnosis-review--session-validator state)))
        (gnosis-agent-review--stop-background)
        (funcall check checkpoint))
      (gnosis-agent-eval--read
     (list :question (plist-get question :question)
           :reference-answer (aref (gnosis-agent-review--answers question) 0)
           :rubric (plist-get question :rubric)) validate
     (lambda ()
       (gnosis-display-keimenon (plist-get question :question))
       (gnosis-agent-review--note (plist-get data :agent-note) validate)
       (gnosis-display-hint (mapconcat #'identity (plist-get question :hints) "\n")))
     (lambda (request resolve reject)
       (gnosis-agent-review--call-provider
                state (gnosis-agent-review--request
                 state "evaluate" pending-request
                 (plist-get request :response))
                (lambda (value)
                  (condition-case err
                      (progn
                        (when (plist-get data :protocol)
                          (gnosis-agent-review--object value '(:api-version :evaluation :adjustment))
                          (unless (eq (plist-get value :api-version) 2)
                            (user-error "Unsupported combined tutor response"))
                          (when (plist-get value :adjustment)
                            (gnosis-agent-review--plan
                             (gnosis-agent-review--data state) (plist-get value :adjustment)))
                          (setq pending-plan (gnosis-agent-review--copy (plist-get value :adjustment))
                                value (plist-get value :evaluation)))
                        (gnosis-agent-review--evaluation value)
                        (funcall resolve (list :verdict (intern (plist-get value :verdict))
                                               :explanation (gnosis-agent-review--copy
                                                             (plist-get value :explanation)))))
                    (error (funcall reject (error-message-string err)))))
                reject))
     restored
     (lambda (event response result &optional failure)
       (funcall validate)
       (let ((request-id (if (equal event "submit")
                             (gnosis-scheduler-event-id) pending-request)))
         (when (member event '("submit" "cancel")) (setq pending-plan nil))
         (gnosis-agent-review--checkpoint state occurrence event response result failure request-id pending-plan)
         ;; Retire before shared input invokes callback-capable cancellation.
         ;; A reopened reader must never claim a previous reader's attempt.
         (when (member event '("submit" "settle" "cancel"))
           (setq pending-request (and (equal event "submit") request-id)))))
     id t))
    (funcall validate)
    (gnosis-agent-review--drain-background state)
    (funcall validate)
    (let* ((current (plist-get (gnosis-agent-review--data state) :current))
           (success (equal (plist-get current :outcome) "success"))
           (result (list :mode 'practice :thema-id id :event-id occurrence
                         :outcome (if success 'success 'failure)
                         :local-owner (list (current-buffer) state occurrence validate))))
      (let ((feedback-start (point-max))
            (evaluated-success (equal (plist-get (plist-get current :evaluation) :verdict) "pass")))
        (gnosis-review--append-section
         (concat (propertize (if evaluated-success "Recall: correct" "Recall: not yet correct")
                             'face (if evaluated-success 'success 'warning))
                 (if (equal (gnosis-agent-review--type question) "agent-eval")
                     " (evaluator) — not yet accepted" " (native) — not yet accepted")) validate "\n\n" "\n")
        (when (equal (gnosis-agent-review--type question) "agent-eval")
          (gnosis-agent-review--note
           (plist-get (plist-get current :evaluation) :explanation) validate
           "Evaluator feedback"))
        (gnosis-agent-review--note (plist-get question :parathema) validate "Explanation" t)
        (gnosis-display-basic-answer
         (mapconcat #'identity (gnosis-agent-review--answers question) "; ") t nil "Reference answer")
        (gnosis-agent-review--note
         (gnosis-agent-review--response-text question (plist-get current :response))
         validate "Your response")
        (gnosis-display-next-review nil success)
        (funcall validate)
        (goto-char feedback-start)
        (skip-chars-forward "\n")
        (when-let* ((window (get-buffer-window (current-buffer))))
          (set-window-start window (point))
          (set-window-point window (point))))
      (gnosis-review-actions success id result))))

(defun gnosis-agent-review--check-result (id result)
  "Reject RESULT unless it owns exact local question ID and native lifetime."
  (let* ((owner (plist-get result :local-owner))
         (state (nth 1 owner)))
    (unless (and owner (eq (car owner) (current-buffer))
                 (eq state gnosis-review--state)
                 (gnosis-review-state-agent-review state)
                 (equal id (plist-get result :thema-id))
                 (equal (nth 2 owner) (plist-get result :event-id)))
      (user-error "Missing or stale local result owner"))
    (funcall (nth 3 owner))
    (unless (equal id (plist-get (plist-get (gnosis-agent-review--data state) :current) :id))
      (user-error "Local result belongs to a different question"))))

(defun gnosis-agent-review--override (result success)
  "Checkpoint explicit pending RESULT override to SUCCESS, without acceptance."
  (let* ((state (nth 1 (plist-get result :local-owner)))
         (data (gnosis-agent-review--copy (gnosis-agent-review--data state))))
    (gnosis-agent-review--check-result (plist-get result :thema-id) result)
    (when (plist-get data :protocol) (setf (plist-get data :proposal) nil))
    (setf (plist-get (plist-get data :current) :outcome) (if success "success" "failure"))
    (gnosis-agent-review--commit state data (gnosis-review-state-remaining state))))

(defun gnosis-agent-review--accept (id success result)
  "Accept exact local RESULT for ID and SUCCESS, atomically entering adaptation."
  (gnosis-agent-review--check-result id result)
  (let* ((state gnosis-review--state)
         (data (gnosis-agent-review--copy (gnosis-agent-review--data state)))
         (current (plist-get data :current))
         (outcome (if success "success" "failure"))
         (evaluation (plist-get current :evaluation))
         (remaining (cdr (gnosis-review-state-remaining state)))
         (proposal (plist-get data :proposal)))
    (unless (and (eq (plist-get result :outcome) (if success 'success 'failure))
                 (equal outcome (plist-get current :outcome))
                 (member (plist-get evaluation :verdict) '("pass" "fail")))
      (user-error "Local result does not match pending evaluation"))
    (setf (plist-get data :encounters)
          (vconcat (plist-get data :encounters)
                   (vector (list :occurrence (plist-get current :occurrence)
                                 :question (gnosis-agent-review--find data id)
                                 :response (plist-get current :response)
                                 :help (plist-get current :help) :source-viewed (plist-get current :source-viewed)
                                 :evaluation evaluation
                                 :context (plist-get data :context)
                                 :outcome outcome
                                 :override (if (eq success (equal (plist-get evaluation :verdict) "pass"))
                                               :false t))))
          (plist-get data :phase) "adapt"
          (plist-get data :current) nil)
    (when (plist-get data :protocol)
      (setf (plist-get data :proposal) nil
            (plist-get data :phase) (if remaining "question" "done"))
      (when (and proposal
                 (equal (plist-get proposal :occurrence) (plist-get current :occurrence))
                 (equal (plist-get proposal :response) (plist-get current :response))
                 (equal (plist-get proposal :outcome) outcome)
                 (equal (plist-get proposal :remaining) (vconcat remaining)))
        (let ((plan (plist-get proposal :plan)))
          (setq data (gnosis-agent-review--plan data plan)
                remaining (mapcar (lambda (q) (plist-get q :id))
                                  (append (plist-get plan :remaining) nil))))))
    (gnosis-agent-review--commit state data remaining)
    (list :rating (if success 3 1))))

(defun gnosis-agent-review--adapt-request (state validate)
  "Wait interruptibly for STATE's adaptation, checking VALIDATE throughout.
Callbacks stage a proposal only.  Retire before cancelling transport; never
write or continue from a callback or after cancellation reentry."
  (let* ((request-id (gnosis-scheduler-event-id))
         (data (gnosis-agent-review--copy (gnosis-agent-review--data state))))
    (setf (plist-get data :attempts)
          (vconcat (plist-get data :attempts)
                   (vector (list :phase "adapt" :occurrence nil :request-id request-id
                                 :response nil :evaluation nil :error nil :plan nil
                                 :context (plist-get data :context)))))
    (gnosis-agent-review--commit state data (gnosis-review-state-remaining state))
    (let* ((request (gnosis-agent-review--request state "adapt" request-id))
           (checkpoint (copy-tree (gnosis-review--state-data state)))
           (active t) result failure cancel
           (buffer (current-buffer))
           (header header-line-format)
           (waiting (replace-regexp-in-string
                     "%" "%%"
                     (concat (propertize " Adjusting next question… " 'face 'warning)
                             (substitute-command-keys "\\[keyboard-quit] Pause")) t t))
           (deadline (+ (float-time) gnosis-agent-eval-timeout))
           (valid (lambda ()
                    (and active
			 (eq (gnosis-review-state-database state) gnosis-db)
			 (condition-case nil
                             (progn (funcall validate checkpoint) t)
                           (error nil))))))
      (unwind-protect
          (progn
            (setq header-line-format waiting)
            (force-mode-line-update)
            (condition-case err
		(setq cancel
                      (gnosis-agent-review--call-provider state request
                               (lambda (value)
				 (when (funcall valid)
                                   (setq active nil)
                                   (condition-case err
                                       (progn
					 (gnosis-agent-review--plan (gnosis-agent-review--data state) value)
					 (setq result (gnosis-agent-review--copy value)))
                                     (error (setq failure (error-message-string err))))))
                               (lambda (error-text)
				 (when (funcall valid)
                                   (setq active nil failure (if (stringp error-text) error-text
                                                              "Adaptation failed"))))))
              (error (setq active nil failure (error-message-string err))))
            (unless (functionp cancel)
              (setq active nil result nil failure "Provider did not return a cancellation function"))
            (while (and active (< (float-time) deadline))
              (funcall validate checkpoint)
              (accept-process-output nil 0.05))
            (funcall validate checkpoint)
            (when active (setq failure "Adaptation timed out"))
            (list result failure checkpoint))
	(setq active nil)
        ;; Restore before callback-capable cancellation, only in our occurrence.
        (when (buffer-live-p buffer)
          (with-current-buffer buffer
            (when (and (eq header-line-format waiting)
                       (condition-case nil (progn (funcall validate checkpoint) t) (error nil)))
              (setq header-line-format header)
              (force-mode-line-update))))
	(gnosis-agent-eval--cancel cancel)))))

(defun gnosis-agent-review--adapt (state &optional authorized)
  "Adjust STATE after acceptance, or recover on explicit AUTHORIZED retry.
Successful Next already authorizes the wait/apply/advance sequence.  Recovery
and failure offer Retry, Pause, or explicit Continue without adjustment."
  (let ((validate (gnosis-review--session-validator state))
        (failure (unless authorized "Pending adaptation; no answer will be graded again"))
        finished)
    (while (not finished)
      (funcall validate)
      (let ((choice (if failure
                        (car (read-multiple-choice
                              failure '((?r "retry" "Request adjustment")
                                        (?p "pause" "Keep the accepted answer and old tail")
                                        (?c "continue" "Continue without adjustment"))))
                      ?r)))
        (funcall validate)
        (pcase choice
          (?p (throw 'review-loop t))
          (?c
           (let* ((data (gnosis-agent-review--data state))
                  (remaining (gnosis-review-state-remaining state))
                  (plan (list :agent-note "Continued without agent adjustment."
                              :context (plist-get data :context)
                              :remaining (vconcat (mapcar (lambda (id) (gnosis-agent-review--find data id))
                                                          remaining))
                              :done (if remaining :false t))))
             (gnosis-agent-review--commit state (gnosis-agent-review--plan data plan) remaining)
             (setq finished t)))
          (?r
           (message "Adjusting practice… C-g pauses without losing the accepted answer")
           (pcase-let ((`(,plan ,error-text ,checkpoint) (gnosis-agent-review--adapt-request state validate)))
             ;; Cancellation is callback-capable; retain the exact settled revision.
             (funcall validate checkpoint)
             (let* ((data (gnosis-agent-review--copy (gnosis-agent-review--data state)))
                    (attempts (plist-get data :attempts))
                    (attempt (aref attempts (1- (length attempts)))))
               (setf (plist-get attempt :error) error-text
                     (plist-get attempt :plan) plan)
               (gnosis-agent-review--commit state data (gnosis-review-state-remaining state)))
             (if error-text (setq failure error-text)
               (let* ((data (gnosis-agent-review--plan (gnosis-agent-review--data state) plan))
                      (ids (mapcar (lambda (q) (plist-get q :id))
                                   (append (plist-get plan :remaining) nil))))
                 (gnosis-agent-review--commit state data ids)
                 (setq finished t))))))))))

(defun gnosis-agent-review--run (state)
  "Run STATE through native input, explicit acceptance and optional adaptation."
  (let ((buffer (current-buffer))
        (validate (gnosis-review--session-validator state))
        (gnosis-agent-review--background nil))
    (unwind-protect
        (while (gnosis-review--work-p state)
          (funcall validate)
          (pop-to-buffer-same-window buffer)
          (funcall validate)
          (gnosis-agent-review--drain-background state)
          (pcase (plist-get (gnosis-agent-review--data state) :phase)
            ("adapt" (gnosis-agent-review--adapt state))
            ("question"
             (gnosis-agent-review--question-input state)
             (funcall validate)
             (if (not (plist-get (gnosis-agent-review--data state) :protocol))
                 (gnosis-agent-review--adapt state t)
               (let* ((encounters (plist-get (gnosis-agent-review--data state) :encounters))
                      (last (aref encounters (1- (length encounters)))))
                 (when (and (gnosis-review--work-p state)
                            (not (equal (gnosis-agent-review--type (plist-get last :question)) "agent-eval")))
                   (gnosis-agent-review--start-background state)))))
            (_ (error "Invalid agent review controller phase"))))
      (gnosis-agent-review--stop-background)))
  state)

(defun gnosis-agent-review--source (success id result)
  "Show captured source for ID and RESULT, preserving pending SUCCESS.
Use native source-return lifetime machinery without querying thema links."
  (gnosis-agent-review--check-result id result)
  (let* ((origin (current-buffer))
         (state gnosis-review--state)
         (question (gnosis-agent-review--find (gnosis-agent-review--data state) id))
         (validate (lambda ()
                     (unless (buffer-live-p origin) (user-error "Review buffer was killed"))
                     (with-current-buffer origin (gnosis-agent-review--check-result id result))))
         (buffer (generate-new-buffer "*Gnosis captured source*"))
         (entry (list buffer)))
    (unwind-protect
        (progn
          (with-current-buffer buffer
            (insert (plist-get question :source))
            (special-mode))
          (window-configuration-to-register :gnosis-link-view)
          (funcall validate)
          (pop-to-buffer buffer)
          (funcall validate)
          (let ((data (gnosis-agent-review--copy (gnosis-agent-review--data state))))
            (setf (plist-get (plist-get data :current) :source-viewed) t)
            (gnosis-agent-review--commit state data (gnosis-review-state-remaining state)))
          (let ((gnosis-link-view--entry entry)) (gnosis-link-view-mode 1))
          (with-current-buffer origin
            (gnosis-review--wait-for-edit origin buffer t validate)))
      (gnosis-link-view--cleanup entry))
    (funcall validate)
    (cons success result)))

(defun gnosis-agent-review--summary (state)
  "Display local STATE's phase-aware summary, not collection practice statistics."
  (let* ((data (gnosis-agent-review--data state))
         (buffer (generate-new-buffer "*Gnosis Agent Review Summary*"))
         (target (cons (gnosis-review-state-database state)
                       (copy-tree (gnosis-review--state-data state)))))
    (with-current-buffer buffer
      (insert (propertize "Agent review — session-local; schedule unchanged\n" 'face 'bold)
              (propertize
               (cond ((gnosis-review-state-cancelled-p state) "Ended early")
                     ((not (gnosis-review--work-p state)) "Completed")
                     ((equal (plist-get data :phase) "adapt") "Paused — adjustment pending")
                     (t "Paused — answer pending"))
               'face (cond ((gnosis-review-state-cancelled-p state) 'shadow)
                           ((gnosis-review--work-p state) 'warning) (t 'success)))
              "\nAccepted: "
              (propertize (number-to-string (length (plist-get data :encounters)))
                          'face 'font-lock-type-face)
              "   Remaining queued: "
              (propertize (number-to-string (length (gnosis-review-state-remaining state)))
                          'face 'font-lock-type-face)
              "\nQuestions are not in the collection unless explicitly saved.\n")
      (gnosis-agent-review--note (plist-get data :agent-note) #'ignore)
      (delay-mode-hooks
        (gnosis-agent-review-summary-mode)
        (setq gnosis-review--summary-target target))
      (run-mode-hooks))
    (pop-to-buffer buffer)))

(defun gnosis-agent-review--summary-unfinished-p ()
  "Return non-nil for an owned, unfinished local summary."
  (let* ((data (cdr gnosis-review--summary-target))
         (local (plist-get data :agent-review)))
    (and (derived-mode-p 'gnosis-agent-review-summary-mode)
         (not buffer-file-name) local
         (not (plist-get data :cancelled-p))
         (not (equal (plist-get local :phase) "done")))))

(keymap-popup-define gnosis-agent-review-summary-mode-map
  "Local practice summary"
  :popup-key "?"
  :parent special-mode-map
  :group "Session"
  "r" ("Resume" gnosis-review-resume :if #'gnosis-agent-review--summary-unfinished-p)
  "d" ("Discard" gnosis-review-discard :if #'gnosis-agent-review--summary-unfinished-p)
  "q" ("Quit" quit-window))

(define-derived-mode gnosis-agent-review-summary-mode gnosis-review-summary-mode "Gnosis local summary"
  "Inspect local practice; resume with \\[gnosis-review-resume].
Accepted local answers cannot be undone; override before acceptance instead."
  (setq-local truncate-lines nil truncate-partial-width-windows nil word-wrap t)
  (set-keymap-parent gnosis-agent-review-summary-mode-map special-mode-map)
  (dolist (hook '(kill-buffer-hook change-major-mode-hook after-set-visited-file-name-hook))
    (add-hook hook #'gnosis-review--retire-summary nil t)))

(defun gnosis-agent-review--saved-content (question)
  "Return native content row expected when saving exact QUESTION."
  (list (gnosis-agent-review--type question) (plist-get question :question)
        (append (if (member (gnosis-agent-review--type question) '("mcq" "mc-cloze"))
                    (plist-get question :choices) (plist-get question :hints)) nil)
        (append (gnosis-agent-review--answers question) nil)
        nil (concat (or (plist-get question :parathema) "") "\n\nSource:\n"
                    (plist-get question :source))
        "" (plist-get question :rubric)))

(defun gnosis-agent-review--verify-save (question receipt)
  "Read back QUESTION against exact native RECEIPT, refusing later drift."
  (let ((id (string-to-number (plist-get receipt :thema-id))))
    (unless (and (equal (gnosis-review--content-thema id)
                        (list (gnosis-agent-review--saved-content question)))
                 (equal (sort (gnosis-select 'tag 'thema-tag `(= thema-id ,id) t) #'string<)
                        (sort (append (plist-get question :tags) nil) #'string<))
                 (equal (sort (gnosis-select 'dest 'thema-links `(= source ,id) t) #'string<)
                        (sort (gnosis--thema-expected-links
                               (plist-get question :question)
                               (nth 5 (gnosis-agent-review--saved-content question))) #'string<)))
      (user-error "Saved question changed or was deleted; reconcile it before retrying"))
    receipt))

;;;###autoload
(defun gnosis-agent-review-save-question (session-id question-id)
  "Explicitly save exact QUESTION-ID from paused current SESSION-ID.
This instruction is the authorization; providers never call it implicitly.
Create an ordinary source-bearing thema through its native content writer,
with its save receipt in the same transaction.  No review evidence is created.
Return a JSON-safe receipt with decimal-string :thema-id after exact readback.
Retry verifies the receipt instead of recreating a changed/deleted card.
Historical sessions and running native input refuse; saving never replaces
another current batch.  The ongoing question remains session-local."
  (let* ((state (gnosis-agent--current session-id))
         (data (gnosis-agent-review--copy (gnosis-agent-review--data state)))
         (question (gnosis-agent-review--find data question-id))
         (old (seq-find (lambda (r) (equal question-id (plist-get r :question-id)))
                        (plist-get data :saves)))
         (receipt
          (or old
              (gnosis-sqlite-with-transaction (gnosis-review-state-database state)
					      (gnosis-review--check-action-target
					       (cons (gnosis-review-state-database state) (gnosis-review--state-data state)))
					      (let* ((row (gnosis-agent-review--saved-content question))
						     (id (gnosis-add-thema-fields
							  (car row) (nth 1 row) (nth 2 row) (nth 3 row) (nth 5 row)
							  (append (plist-get question :tags) nil) 0
							  (gnosis--thema-expected-links (nth 1 row) (nth 5 row))
							  "" nil nil (nth 7 row)))
						     (receipt (list :session-id session-id :question-id question-id
								    :thema-id (number-to-string id))))
						(gnosis-agent-review--verify-save question receipt)
						(setf (plist-get data :saves) (vconcat (plist-get data :saves) (vector receipt)))
						(gnosis-agent-review--commit state data (gnosis-review-state-remaining state))
						receipt)))))
    (gnosis-agent-review--copy (gnosis-agent-review--verify-save question receipt))))

(provide 'gnosis-agent-review)
;;; gnosis-agent-review.el ends here
