;;; gnosis-agent.el --- Asynchronous native practice handoff -*- lexical-binding: t; -*-
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

;; Optional, agent-independent handoff to native human answering.  No grades
;; or learner transcript are synthesized here.  Practice and corrections use
;; the same durable core as interactive review.  API v1 returns keyword plists,
;; strings, numbers, vectors, t and :false; nil denotes JSON null.  Serialize
;; with `json-serialize' using :false-object :false and :null-object nil.

;;; Code:

(require 'gnosis-review)

(autoload 'gnosis-agent-review-status "gnosis-agent-review")
(autoload 'gnosis-agent-review-results "gnosis-agent-review")

(defvar gnosis-agent--launches nil
  "Pending launch records, owned by exact database, session and token.")

(defun gnosis-agent--session (session-id)
  "Read the exact study SESSION-ID, never the latest batch."
  (unless (and (stringp session-id) (not (string-empty-p session-id)))
    (user-error "Session ID must be a nonempty string"))
  (let ((data (gnosis-get 'data 'study-history `(= session-id ,session-id))))
    (unless (and (memq (plist-get data :version) '(1 2 3))
                 (memq (plist-get data :mode) '(practice due)))
      (user-error "Unknown study session: %s" session-id))
    (gnosis-review--decode-session data)))

(defun gnosis-agent--current (session-id)
  "Return current agent SESSION-ID, refusing historical or active input."
  (when gnosis-review--running (user-error "Quit native review before changing the batch"))
  (let ((state (gnosis-agent--session session-id))
        (current (gnosis-review--read-session)))
    (unless (and current
                 (equal session-id (gnosis-review-state-session-id current))
                 (not (gnosis-review-state-cancelled-p state)))
      (user-error "Session is no longer the current batch"))
    state))

(defun gnosis-agent--release (record)
  "Retire exact launch RECORD, cancelling only its timer.
Already queued ordinary events become harmless no-ops."
  (setq gnosis-agent--launches (delq record gnosis-agent--launches))
  (when (timerp (plist-get record :timer))
    (cancel-timer (plist-get record :timer))))

(defun gnosis-agent--enqueue (record)
  "Append one ordinary input event for the still-pending launch RECORD.
Return without entering native input, including when called by a timer."
  (when (and (memq record gnosis-agent--launches)
             (not (plist-get record :queued)))
    (setf (plist-get record :queued) t)
    (setq unread-command-events
          (append unread-command-events (list (list 'gnosis-agent-launch record))))))

(defun gnosis-agent--dispatch (event)
  "Dispatch ordinary launch EVENT once, never from a special-event reader."
  (interactive "e")
  (when (and (consp event) (eq (car event) 'gnosis-agent-launch))
    (gnosis-agent--launch (cadr event))))

;; This private event must survive readers which preserve ordinary input.
;; Special events can instead run inside a foreign synchronous reader.
(define-key global-map [gnosis-agent-launch] #'gnosis-agent--dispatch)

(defun gnosis-agent--check-launch (record)
  "Require RECORD's original environment before presenting native input."
  (gnosis-review--check-input-entry)
  (unless (and (eq gnosis-db (plist-get record :db)) (sqlitep gnosis-db)
               (equal (expand-file-name gnosis-dir) (plist-get record :dir))
               (eq (selected-frame) (plist-get record :frame))
               (frame-live-p (plist-get record :frame))
               (= (recursion-depth) (plist-get record :depth))
               (not (active-minibuffer-window))
               (not gnosis-review--running))
    (user-error "Study launch context changed; resume the same session")))

(defun gnosis-agent--launch (record)
  "Present native input only while RECORD owns the exact pending batch.
Consume the record before callback-capable UI entry.  Stale delivery leaves
its durable checkpoint untouched and available for explicit resume."
  (when (memq record gnosis-agent--launches)
    (gnosis-agent--release record)
    (condition-case err
        (progn
          (gnosis-agent--check-launch record)
          (let ((buffer (plist-get record :buffer))
                (state (gnosis-review--read-session)))
            (if buffer
                (progn
                  (unless (buffer-live-p buffer) (user-error "Study buffer was killed"))
                  (with-current-buffer buffer
                    (unless (and (plist-get record :setup)
                                 (eq gnosis-review--state (plist-get record :state))
                                 (eq major-mode 'gnosis-mode))
                      (user-error "Study buffer has no owned setup"))
                    (gnosis-review--watch-buffer (plist-get record :setup))))
              (when (get-buffer (plist-get record :buffer-name))
                (user-error "Study buffer name was claimed before launch")))
            (when (and state
                       (equal (plist-get record :session-id)
                              (gnosis-review-state-session-id state))
                       (equal (plist-get record :token)
                              (gnosis-review-state-launch-token state))
                       (not (gnosis-review-state-cancelled-p state))
                       (gnosis-review--work-p state))
              ;; A renamed owner remains the destination, not its former name.
              (let ((gnosis-review-buffer-name (or buffer (plist-get record :buffer-name))))
                (gnosis-review--resume
                 (cons (plist-get record :db) (gnosis-review--state-data state))
                 (lambda () (gnosis-agent--check-launch record)))))))
      (quit (message "Gnosis study interrupted; resume the same session"))
      (error (message "Gnosis study stopped: %s" (error-message-string err))))))

(defun gnosis-agent--schedule (state)
  "Schedule exact STATE for native input after returning to the event loop."
  (when (gnosis-review--work-p state)
    (let* ((token (gnosis-scheduler-event-id))
           (buffer (get-buffer gnosis-review-buffer-name))
           (record (list :db (gnosis--ensure-db)
                         :dir (expand-file-name gnosis-dir)
                         :frame (selected-frame) :depth (recursion-depth)
                         :buffer-name gnosis-review-buffer-name :buffer buffer
                         :setup (and buffer (buffer-local-value 'gnosis-review--setup-owner buffer))
                         :state (and buffer (buffer-local-value 'gnosis-review--state buffer))
                         :session-id (gnosis-review-state-session-id state)
                         :token token :timer nil :queued nil)))
      (gnosis-sqlite-with-transaction (gnosis--ensure-db)
        (unless (equal (gnosis-review--state-data state)
                       (gnosis-review--state-data (gnosis-agent--current
                                                   (gnosis-review-state-session-id state))))
          (user-error "Session changed before launch"))
        (setf (gnosis-review-state-launch-token state) token)
        (gnosis-review--save-session state))
      (dolist (old (copy-sequence gnosis-agent--launches))
        (when (eq (plist-get old :db) gnosis-db) (gnosis-agent--release old)))
      (push record gnosis-agent--launches)
      (condition-case err
          (setf (plist-get record :timer) (run-with-timer 0 nil #'gnosis-agent--enqueue record))
        (error
         (gnosis-agent--release record)
         (message "Study reserved but not launched: %s" (error-message-string err)))))))

(cl-defun gnosis-agent-start-practice (&key thema-ids topic-ids limit policy)
          "Reserve native practice and return API v1 status immediately.
Supply exactly one nonempty list: integer THEMA-IDS or string TOPIC-IDS.
LIMIT is a required positive unique-item limit.  Select direct
topic links only, sorted by thema ID; exclude missing/suspended themata,
deduplicate before limiting and report shortfall.  Freeze validated POLICY
as described by `gnosis-review-practice-policy'.  A nonempty selection ends
unfinished progress early, retaining accepted evidence and schedules.  An empty
selection returns a completed shortfall report without replacing the batch.
Refuse active native input.  The human answers later; this records no grade."
          (unless (and (integerp limit) (> limit 0)
                       (if thema-ids (null topic-ids) topic-ids)
                       (proper-list-p (or thema-ids topic-ids))
                       (seq-every-p (if thema-ids #'integerp
                                      (lambda (id) (and (stringp id) (not (string-empty-p id)))))
                                    (or thema-ids topic-ids)))
            (user-error "Provide thema IDs or topic IDs and a positive unique limit"))
          (let* ((target (gnosis-review--session-target))
                 (policy (gnosis-review-practice-policy policy))
                 (topics (delete-dups (copy-sequence topic-ids)))
                 (unknown (seq-remove (lambda (id) (gnosis-get 'id 'nodes `(= id ,id))) topics)))
            (when unknown (user-error "Unknown topics: %S" unknown))
            (let* ((candidates (sort (if thema-ids
                                         (delete-dups (copy-sequence thema-ids))
                                       (gnosis-study--topic-linked-ids topics)) #'<))
                   (rows (when candidates
                           (gnosis-sqlite-select-batch
                            (gnosis--ensure-db)
                            "SELECT t.id, s.suspended, s.due_day FROM themata t
                              JOIN scheduler_state s ON s.thema_id = t.id
                             WHERE t.id IN (%s)" candidates)))
                   (eligible (gnosis-study--eligible-ids candidates rows nil))
                   (ids (seq-take eligible limit))
                   (selection (list :limit limit :candidates (length candidates)
                                    :eligible (length eligible) :selected (length ids)
                                    :shortfall (max 0 (- limit (length ids)))
                                    :omitted-by-limit (max 0 (- (length eligible) limit))
                                    :excluded-ids (vconcat (seq-difference candidates eligible))
                                    :topic-ids (vconcat topics)))
                   (state (gnosis-review--reserve-practice ids policy selection target)))
              (gnosis-agent--schedule state)
              (gnosis-agent-status (gnosis-review-state-session-id state)))))

(defun gnosis-agent--group-selection (tags thema-groups limit due)
  "Resolve ordered TAGS or THEMA-GROUPS with unique LIMIT and DUE eligibility.
Return selection metadata with explicit per-group dispositions.  First
membership wins, even when excluded.  Tag members use ascending integer IDs;
explicit groups retain their input order.  Never shuffle caller input."
  (unless (and (integerp limit) (> limit 0)
               (if tags (null thema-groups) thema-groups)
               (proper-list-p (or tags thema-groups))
               (if tags
                   (seq-every-p (lambda (tag) (and (stringp tag) (not (string-empty-p tag)))) tags)
                 (seq-every-p (lambda (ids) (and (proper-list-p ids) (seq-every-p #'integerp ids)))
                              thema-groups)))
    (user-error "Supply ordered tags or thema groups and a positive unique limit"))
  (let* ((groups (if tags
                     (mapcar (lambda (tag) (sort (gnosis-get-tag-themata tag) #'<)) tags)
                   (copy-tree thema-groups)))
         (all (delete-dups (apply #'append (copy-tree groups))))
         (rows (when all
                 (gnosis-sqlite-select-batch
                  (gnosis--ensure-db)
                  "SELECT t.id, s.suspended, s.due_day FROM themata t
                    JOIN scheduler_state s ON s.thema_id = t.id WHERE t.id IN (%s)" all)))
         (eligible (gnosis-study--eligible-ids all rows (and due (gnosis--today-int))))
         (selected (seq-take eligible limit))
         (seen nil)
         (reports
          (cl-loop for group in groups for index from 0 collect
                   (let* ((members (delete-dups (copy-sequence group)))
                          (overlap (seq-intersection members seen))
                          (owned (seq-difference members seen)))
                     (setq seen (append seen owned))
                     (list :tag (nth index tags) :requested-ids (vconcat group)
                           :overlap-ids (vconcat overlap)
                           :excluded-ids (vconcat (seq-difference owned eligible))
                           :omitted-ids (vconcat (seq-difference
                                                 (seq-intersection owned eligible) selected))
                           :selected-ids (vconcat (seq-intersection owned selected)))))))
    (list :limit limit :candidates (length all) :eligible (length eligible)
          :selected (length selected) :shortfall (max 0 (- limit (length selected)))
          :omitted-by-limit (max 0 (- (length eligible) limit))
          :excluded-ids (vconcat (seq-difference all eligible))
          :groups (vconcat reports))))

(defun gnosis-agent--start-groups (mode tags thema-groups limit policy)
  "Reserve MODE using TAGS or THEMA-GROUPS, LIMIT and practice POLICY."
  (let* ((target (gnosis-review--session-target))
         (selection (gnosis-agent--group-selection tags thema-groups limit (eq mode 'due)))
         (ids (mapcan (lambda (group) (append (plist-get group :selected-ids) nil))
                      (append (plist-get selection :groups) nil)))
         (state (gnosis-review--reserve-batch ids mode policy selection target)))
    (gnosis-agent--schedule state)
    (gnosis-agent-status (gnosis-review-state-session-id state))))

(cl-defun gnosis-agent-start-practice-groups (&key tags thema-groups limit policy)
  "Reserve ordered practice from TAGS or THEMA-GROUPS and return API v1 status.
Supply exactly one nonempty list: TAGS (strings, each defining one group),
or THEMA-GROUPS (lists of integer IDs, including empty groups).  LIMIT is a
required positive unique-item limit across groups.  POLICY uses the same
frozen rules as `gnosis-agent-start-practice'.

Group order is caller order.  Tag members use ascending IDs; explicit groups
retain their order.  First membership wins overlaps.  Exclude missing and
suspended items before limiting.  Selection :groups reports requested,
overlap, excluded, omitted-by-limit and selected IDs, including empty groups.
Persist those groups and the actual queue for resume, without reselecting tags.
Initial presentations stay grouped; ordinary retries and successful policy
continuations may cross group boundaries.  Their spacing and policy do not
change.  Practice never changes FSRS.  Empty selection preserves the current
batch; invalid arguments refuse before replacement.  Native human input is
deferred as in `gnosis-agent-start-practice'."
  (gnosis-agent--start-groups 'practice tags thema-groups limit policy))

(cl-defun gnosis-agent-start-review-groups (&key tags thema-groups limit)
  "Reserve ordered scheduled review from TAGS or THEMA-GROUPS with LIMIT.
Use the group format and dispositions of `gnosis-agent-start-practice-groups'.
Also exclude items not due on the current logical day.  Like explicit topic
review, do not apply the global daily new-item cap.  Ordering never changes
due dates; native acceptance uses ordinary FSRS and scheduled retry policy.
Return API v1 status; use `gnosis-agent-status', `gnosis-agent-resume' and
`gnosis-agent-cancel' with the returned session ID.  Detailed
`gnosis-agent-results' remains practice-only."
  (gnosis-agent--start-groups 'due tags thema-groups limit nil))

(defun gnosis-agent-current-practice ()
  "Return status for the current practice reservation, or nil.
Read only the connected database's exact active checkpoint, including a
zero-attempt reservation.  Do not start, resume, cancel or select a batch.
A completed checkpoint remains discoverable until replaced or discarded;
scheduled sessions and cancelled reservations return nil."
  (gnosis-sqlite-with-transaction (gnosis--ensure-db)
    (when-let* ((state (gnosis-review--read-session))
                ((eq (gnosis-review-state-mode state) 'practice))
                ((not (gnosis-review-state-cancelled-p state))))
      (gnosis-agent-status (gnosis-review-state-session-id state)))))

(defvar gnosis-agent--history-owners (make-hash-table :test 'eq :weakness 'key)
  "Opaque process-local identities for history cursor connections.")

(defun gnosis-agent--history-epoch (connection)
  "Return CONNECTION's local, external and schema change counters."
  (list (caar (sqlite-select connection "SELECT total_changes()"))
        (caar (sqlite-select connection "PRAGMA data_version"))
        (caar (sqlite-select connection "PRAGMA schema_version"))))

(defun gnosis-agent--history-read (text)
  "Read exactly one serialized value from TEXT, allowing trailing whitespace."
  (let* ((read-circle t)
         (parsed (read-from-string text)))
    (unless (string-match-p "\\`[[:space:]]*\\'" (substring text (cdr parsed)))
      (error "Trailing session data"))
    (car parsed)))

(defun gnosis-agent--history-check-tree (data)
  "Reject cycles in DATA before decoding, but allow shared lists and vectors."
  (let ((seen (make-hash-table :test 'eq))
        (pending (list (cons data nil))))
    ;; Explicit enter/leave frames avoid recursion and revisit shared subtrees
    ;; only once.  The serialized byte bound limits the number of objects.
    (while pending
      (pcase-let ((`(,value . ,leave) (pop pending)))
        (when (or (consp value) (vectorp value))
          (cond (leave (puthash value 'done seen))
                ((eq (gethash value seen) 'active) (error "Circular session data"))
                ((not (gethash value seen))
                 (puthash value 'active seen)
                 (push (cons value t) pending)
                 (if (consp value)
                     (progn (push (cons (cdr value) nil) pending)
                            (push (cons (car value) nil) pending))
                   (mapc (lambda (child) (push (cons child nil) pending)) value)))))))))

(defun gnosis-agent--history-item (row)
  "Decode bounded history ROW into metadata or an explicit problem record."
  (pcase-let* ((`(,position ,encoded-id ,text) row)
               (id (condition-case nil (gnosis-agent--history-read encoded-id)
                     (error nil)))
               (base (list :session-id (and (stringp id) id)
                           :position (number-to-string position))))
    (append
     base
     (if (null text) (list :problem "oversized")
       (condition-case nil
           (let ((data (gnosis-agent--history-read text)))
             (gnosis-agent--history-check-tree data)
             (unless (and (stringp id) (not (string-empty-p id))
                          ;; Exact-result lookup encodes the ID again.  Even
                          ;; valid Lisp with a noncanonical key cannot address
                          ;; this row, so do not advertise it as evidence.
                          (equal encoded-id (gnosis-sqlite--encode-param id))
                          (proper-list-p data) (eq (car data) :version)
                          (zerop (% (length data) 2)))
               (error "Malformed session"))
             (let (seen)
               (cl-loop for (key _value) on data by #'cddr do
                        (unless (and (keywordp key) (not (memq key seen)))
                          (error "Invalid session field"))
                        (push key seen)))
             (if (not (memq (plist-get data :version) '(1 2 3)))
                 (list :problem "unsupported")
               (unless (and (stringp id) (not (string-empty-p id))
                            (equal id (plist-get data :session-id))
                            (memq (plist-get data :mode) '(due practice))
                            (plist-member data :cancelled-p)
                            (memq (plist-get data :cancelled-p) '(nil t))
                            (plist-member data :remaining)
                            (proper-list-p (plist-get data :remaining)))
                 (error "Invalid session identity or state"))
               (when (= (plist-get data :version) 1)
                 (dolist (key '(:remaining :selected :requeued :skipped))
                   (let ((ids (plist-get data key)))
                     (unless (and (proper-list-p ids)
                                  (seq-every-p #'integerp ids))
                       (error "Invalid collection session IDs")))))
               (let* ((state (gnosis-review--decode-session data))
                      (local (gnosis-review-state-agent-review state)))
                 (list :version (plist-get data :version)
                       :kind (if local "agent-review" (symbol-name (gnosis-review-state-mode state)))
                       :status (cond ((gnosis-review-state-cancelled-p state) "cancelled")
                                     ((gnosis-review--work-p state) "unfinished")
                                     (t "completed"))
                       :goal (plist-get local :goal) :source (plist-get local :source)))))
         (error (list :problem "malformed")))))))

(cl-defun gnosis-agent-sessions (&key connection (limit 10) cursor kind)
  "Discover retained sessions on explicit open CONNECTION, without writes.
LIMIT bounds examined payloads, not just matches: an integer from 1 to 50,
default 10.  CURSOR is nil initially or the previous :next-cursor unchanged.
KIND is nil for all sessions, or \"agent-review\", \"practice\" or \"due\".

Return JSON-safe API v1 :database filename, :owner connection identity,
:order \"history-insertion-desc\", :items, :issues, :examined, :complete
and :next-cursor.  Each item has :session-id, decimal-string :position,
:version, :kind, durable :status, and nullable tutor :goal and :source.
Statuses are completed, cancelled (including replacement), or unfinished;
they do not report process-local pending/running state or resumability.

Order is newest retained insertion first, not last activity.  The schema
has no session timestamps; positions are SQLite rowids, not dates or portable
identities.  Native updates preserve insertion position.  No wall-clock
chronology, completion time, or mastery is inferred.  Use exact IDs with
`gnosis-agent-results' on the same bound database for detailed practice or
tutor evidence; scheduled batches instead support `gnosis-agent-status'.

Read at most LIMIT payloads of at most 2 MiB each; larger serialized payloads
are reported as oversized without loading them.  Malformed and unsupported
rows appear in :issues even with KIND filtering; they never stop traversal.
An empty :items vector is not exhaustion: follow :next-cursor until :complete
is t, yielding between calls.  Completion describes traversal, not validity
of rows reported in :issues.  No answers or current checkpoint are returned.

Cursors belong to this query and exact open connection.  Database changes
or reopen invalidate them: restart discovery rather than mixing snapshots.
The filename alone cannot identify a moved or replaced database.  CONNECTION
must be outside any transaction or savepoint: this call owns a short read
transaction and refuses caller-owned transactions without settling them.
It never opens, initializes, resumes, replaces or updates a session."
  (unless (and (sqlitep connection) (integerp limit) (<= 1 limit 50)
               (member kind '(nil "agent-review" "practice" "due")))
    (user-error "Supply an open connection, limit 1..50, and a supported kind"))
  (let ((epoch (gnosis-agent--history-epoch connection)))
    ;; Native Emacs exposes no autocommit predicate.  BEGIN succeeds only
    ;; outside a transaction (including an outer SAVEPOINT).  Its failure
    ;; leaves the caller's transaction alone; only success grants cleanup.
    (unless (sqlite-transaction connection)
      (user-error "Session discovery requires a connection outside a transaction"))
    (let ((page (unwind-protect
                    (gnosis-agent--history-page connection limit cursor kind)
                  (sqlite-execute connection "ROLLBACK"))))
      ;; A WAL commit may have been hidden by our read snapshot.  Recheck
      ;; after ending it, before publishing metadata or a continuation.
      (unless (equal epoch (gnosis-agent--history-epoch connection))
        (user-error "Database changed during discovery; restart the query"))
      page)))

(defun gnosis-agent--history-page (connection limit cursor kind)
  "Read one history page from CONNECTION with LIMIT, CURSOR and KIND."
  (let* ((gnosis-db connection)
         (database (nth 2 (assoc 0 (sqlite-select connection "PRAGMA database_list"))))
         (owner (or (gethash connection gnosis-agent--history-owners)
                    (puthash connection (gnosis-scheduler-event-id) gnosis-agent--history-owners)))
         (epoch (gnosis-agent--history-epoch connection))
         (scope (list :owner owner :epoch (vconcat epoch) :kind kind))
         (before (plist-get cursor :before)))
    (when cursor
      (unless (and (equal (plist-get cursor :scope) scope)
                   (stringp before)
                   (string-match-p (rx string-start (? "-")
                                       (or "0" (seq (in "1-9") (* digit))) string-end)
                                   before)
                   (<= -9223372036854775808 (string-to-number before) 9223372036854775807))
        (user-error "Cursor is stale or belongs to another query or connection")))
    (let* ((rows (sqlite-select
                  connection
                  (concat "SELECT rowid, session_id, CASE WHEN length(CAST(data AS BLOB)) <= 2097152"
                          " THEN data ELSE NULL END FROM study_history"
                          (if cursor " WHERE rowid < ?" "") " ORDER BY rowid DESC LIMIT ?")
                  (if cursor (list (string-to-number before) limit) (list limit))))
           (records (mapcar #'gnosis-agent--history-item rows))
           (last (caar (last rows)))
           (more (and last (sqlite-select connection
                                         "SELECT 1 FROM study_history WHERE rowid < ? LIMIT 1"
                                         (list last)))))
      (unless (equal epoch (gnosis-agent--history-epoch connection))
        (user-error "Database changed during discovery; restart the query"))
      (list :api-version 1 :database database :owner owner
            :order "history-insertion-desc" :examined (length rows)
            :items (vconcat (seq-filter
                             (lambda (item) (and (not (plist-get item :problem))
                                                 (or (null kind) (equal kind (plist-get item :kind)))))
                             records))
            :issues (vconcat (seq-filter (lambda (item) (plist-get item :problem)) records))
            :complete (if more :false t)
            :next-cursor (when more (list :scope scope :before (number-to-string last)))))))

(defun gnosis-agent-status (session-id)
  "Return API v1 status and progress for exact SESSION-ID.
Statuses are pending, running, unfinished, completed and cancelled strings.
Pending is process-local: after restart a reserved batch is unfinished.
Native topic practice is supported too: :policy and :targets are nil for
its legacy one-retry policy.  Scheduled batches return their native summary;
:schedule-updated is t when they retain accepted outcomes, otherwise :false.
Practice always returns :false.  Opening a source does not accept an outcome.
Cancelled includes batches ended early by a replacement, never completion.
Their remaining IDs record abandoned membership, not resumable active work.
Return :api-version 1, :session-id, :database (connected main filename),
:mode (\"practice\" or \"due\"), :status, :schedule-updated,
:selected-ids and :remaining-ids vectors, frozen :policy, :selection counts,
:summary effective first/retry grade counts, and :targets reason counts.
Ordered batches additionally retain :selection :groups as documented by
`gnosis-agent-start-practice-groups'.
Selection records limit, candidates, eligible, selected, shortfall,
omitted-by-limit, excluded-ids and topic-ids.  Targets count target-reached,
attempt-limit, unfinished and excluded items.  Summary needs-work counts
last effective failures, not unmet repetition targets; consult :targets.
Use `json-serialize' with :false-object :false and :null-object nil."
  (gnosis-sqlite-with-transaction (gnosis--ensure-db)
    (let ((state (gnosis-agent--session session-id)))
      (if (gnosis-review-state-agent-review state)
          (gnosis-agent-review-status session-id)
        (gnosis-agent--status state (gnosis-study-practice-events session-id))))))

(defun gnosis-agent--status (state events)
  "Render API status from exact STATE and ordered retained EVENTS."
  (let* ((session-id (gnosis-review-state-session-id state))
           (token (gnosis-review-state-launch-token state))
           (pending (seq-some (lambda (record)
                                (and (eq gnosis-db (plist-get record :db))
                                     (equal token (plist-get record :token))))
                              gnosis-agent--launches))
           (policy (copy-sequence (gnosis-review-state-policy state)))
           (practice (eq (gnosis-review-state-mode state) 'practice))
           (projection (if practice (gnosis-review-practice-projection state events) state)))
      (when (and policy (not (plist-get policy :consecutive)))
        (setq policy (plist-put policy :consecutive :false)))
      (list :api-version 1 :session-id session-id :mode (if practice "practice" "due")
            :database (nth 2 (assoc 0 (sqlite-select (gnosis--ensure-db) "PRAGMA database_list")))
            :status (cond ((gnosis-review-state-cancelled-p state) "cancelled")
                          ((equal gnosis-review--running session-id) "running")
                          ((null (gnosis-review--work-p state)) "completed")
                          (pending "pending")
                          (t "unfinished"))
            :schedule-updated (if (and (not practice) (gnosis-review-state-outcomes state)) t :false)
            :selected-ids (vconcat (gnosis-review-state-selected state))
            :policy policy :selection (gnosis-review-state-selection state)
            :summary (gnosis-review-summary projection)
            :targets (when policy (gnosis-review-policy-summary projection))
            :remaining-ids (vconcat (gnosis-review-state-remaining state)))))

(defun gnosis-agent--item (state id events)
  "Return result for thema ID in STATE using retained EVENTS."
  (let* ((rows (seq-filter (lambda (row) (equal id (nth 1 row))) events))
         (effective (seq-remove (lambda (row) (nth 6 row)) rows))
         (ordered (mapcar (lambda (row) (= 3 (nth 5 row))) effective))
         (first-correct (seq-position ordered t))
         (outcomes (reverse ordered))
         (progress (when (gnosis-review-state-policy state)
                     (gnosis-review-policy-progress (gnosis-review-state-policy state) outcomes)))
         (deleted (not (gnosis-get 'id 'themata `(= id ,id))))
         (excluded (or deleted (member id (gnosis-review-state-skipped state)))))
    (list :thema-id id
          :reason (cond (excluded "excluded")
                        (progress (plist-get progress :reason))
                        ((member id (gnosis-review-state-remaining state)) "unfinished")
                        (t "completed"))
          :deleted (if deleted t :false)
          :attempts (length effective) :total-attempts (length rows)
          :first-outcome (when effective (if (car ordered) "success" "failure"))
          :attempts-to-first-correct (and first-correct (1+ first-correct))
          :retries-to-first-correct first-correct
          :successes (if progress (plist-get progress :successes)
                       (seq-count #'identity outcomes))
          :target (plist-get progress :target)
          :events
          (vconcat (mapcar
                    (lambda (row)
                      (list :event-id (car row) :attempt (nth 3 row)
                            :reviewed-at-us (nth 4 row)
                            :outcome (if (= 3 (nth 5 row)) "success" "failure")
                            :effective (if (nth 6 row) :false t)
                            :correction-id (nth 6 row)
                            :encounter (nth 7 row))) rows)))))

(defun gnosis-agent-results (session-id)
  "Return API v1 status, per-item outcomes and corrections for SESSION-ID.
The additional :items vector contains thema-id, reason, deleted, attempts,
successes, target and events.  Reasons are target-reached, attempt-limit,
unfinished or excluded strings.  Each event has event-id, attempt (the
session-wide ordinal), reviewed-at-us, accepted outcome (success/failure),
effective (t or :false) and correction-id (string or nil).  Event vectors
retain accepted grades and void identifiers.  Optional encounter data retains
versioned presented content, type-specific response, captured matching rules,
shown hints and the original pre-override outcome.  Nil means unavailable;
legacy events are never supplemented with current question content.
Items also include total-attempts (including voids), first-outcome and nullable
attempts-to-first-correct/retries-to-first-correct from effective evidence.
Native practice without a frozen policy has nil :target, total accepted
successes at :successes, and reasons completed, unfinished or excluded.
Effective grades alone determine first/retry counts and policy progress.
Hard thema deletion removes owned events; deleted membership remains visible.
Completion and immediate repetition are not mastery or calibrated retention."
  (if (gnosis-review-state-agent-review (gnosis-agent--session session-id))
      (gnosis-agent-review-results session-id)
    (gnosis-sqlite-with-transaction (gnosis--ensure-db)
				    (let* ((state (gnosis-agent--session session-id))
					   (events (if (eq (gnosis-review-state-mode state) 'practice)
						       (gnosis-study-practice-events session-id t)
						     (user-error "Detailed results are practice-only; use gnosis-agent-status"))))
				      (append (gnosis-agent--status state events)
					      (list :items (vconcat (mapcar (lambda (id) (gnosis-agent--item state id events))
									    (gnosis-review-state-selected state)))))))))

(defun gnosis-agent-resume (session-id)
  "Schedule exact current unfinished SESSION-ID and return API v1 status."
  (let ((state (gnosis-agent--current session-id)))
    (unless (gnosis-review--work-p state) (user-error "Batch is complete"))
    (gnosis-agent--schedule state)
    (gnosis-agent-status session-id)))

(defun gnosis-agent-cancel (session-id)
  "Cancel pending or interrupted SESSION-ID, retaining all accepted evidence.
Refuse active native input: the human must quit that input first.  Repeated
cancellation is harmless; completed and historical sessions are not changed."
  (let ((state (gnosis-agent--session session-id)))
    (unless (or (gnosis-review-state-cancelled-p state)
                (null (gnosis-review--work-p state)))
      (gnosis-sqlite-with-transaction (gnosis--ensure-db)
        (setq state (gnosis-agent--current session-id))
        (setf (gnosis-review-state-cancelled-p state) t
              (gnosis-review-state-launch-token state) nil)
        (gnosis-review--save-history state)
        (gnosis--delete 'study-session))
      (dolist (record (copy-sequence gnosis-agent--launches))
        (when (and (eq gnosis-db (plist-get record :db))
                   (equal session-id (plist-get record :session-id)))
          (gnosis-agent--release record))))
    (gnosis-agent-status session-id)))

(provide 'gnosis-agent)
;;; gnosis-agent.el ends here
