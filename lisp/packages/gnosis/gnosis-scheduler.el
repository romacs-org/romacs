;;; gnosis-scheduler.el --- Atomic FSRS review acceptance  -*- lexical-binding: t; -*-
;; Maintainer: Thanos Apollo <public@thanosapollo.org>

;; Copyright (C) 2026  Free Software Foundation, Inc.

;; Author: Thanos Apollo <public@thanosapollo.org>
;; Keywords: extensions
;; URL: https://git.thanosapollo.org/gnosis
;; Version: 0.0.1

;; This program is free software; you can redistribute it and/or modify
;; it under the terms of the GNU General Public License as published by
;; the Free Software Foundation, either version 3 of the License, or
;; (at your option) any later version.

;;; Commentary:

;; Accept binary reviews as immutable events and update the current FSRS
;; projection in the same SQLite transaction.

;;; Code:

(require 'cl-lib)
(require 'gnosis-db)
(require 'gnosis-exam)
(require 'gnosis-fsrs)
(require 'seq)

(defconst gnosis-scheduler--event-keys
  '(:event-id :thema-id :config-id :reviewed-at-us :review-day :rating
    :elapsed-days :prior-stability :prior-difficulty :stability :difficulty
    :raw-interval-days :calendar-interval-days :due-day :reps-before
    :reps-after :lapses-before :lapses-after :new-p)
  "Ordered plist keys matching the review-events schema.")

(defun gnosis-scheduler-event-id ()
  "Return a new opaque review-event identity."
  (secure-hash 'sha256
               (gnosis-sqlite--serialize
                (list (current-time) (emacs-pid) (random) (user-uid)))))

(defun gnosis-scheduler--event-id-p (value)
  "Return non-nil when VALUE is a canonical event identity."
  (and (stringp value)
       (let ((case-fold-search nil))
         (string-match-p "\\`[0-9a-f]\\{64\\}\\'" value))))

(defun gnosis-scheduler--day-time (day)
  "Return local noon for YYYYMMDD integer DAY, rejecting invalid dates."
  (unless (and (integerp day) (> day 0))
    (error "Invalid scheduler review day"))
  (let* ((date (gnosis--int-to-date day))
         (time (encode-time 0 0 12 (nth 2 date) (nth 1 date) (nth 0 date)))
         (decoded (decode-time time))
         (roundtrip (gnosis--date-to-int
                     (list (nth 5 decoded) (nth 4 decoded) (nth 3 decoded)))))
    (unless (= day roundtrip)
      (error "Invalid scheduler review day"))
    time))

(defun gnosis-scheduler--days-between (earlier later)
  "Return logical days from EARLIER through LATER YYYYMMDD dates."
  (let* ((difference (time-subtract (gnosis-scheduler--day-time later)
                                    (gnosis-scheduler--day-time earlier)))
         (days (round (/ (float-time difference) 86400.0))))
    (when (< days 0)
      (error "Review precedes current scheduler state"))
    days))

(defun gnosis-scheduler--add-days (day days)
  "Return YYYYMMDD integer DAYS after DAY."
  (unless (and (integerp days) (>= days 0))
    (error "Invalid scheduler interval"))
  (let* ((date (gnosis--int-to-date day))
         (time (encode-time 0 0 12 (+ (nth 2 date) days)
                            (nth 1 date) (nth 0 date)))
         (decoded (decode-time time)))
    (gnosis--date-to-int
     (list (nth 5 decoded) (nth 4 decoded) (nth 3 decoded)))))

(defun gnosis-scheduler--event-result (row)
  "Return a scheduler result plist decoded from event ROW."
  (cl-loop for key in gnosis-scheduler--event-keys
           for value in row append (list key value)))

(defun gnosis-scheduler--retained-result
    (row thema-id rating reviewed-at-us review-day)
  "Return ROW when THEMA-ID, RATING, REVIEWED-AT-US, and REVIEW-DAY match."
  (unless (and (= thema-id (nth 1 row)) (= rating (nth 5 row))
               (= reviewed-at-us (nth 3 row)) (= review-day (nth 4 row)))
    (error "Review event identity conflicts with retained evidence"))
  (gnosis-scheduler--event-result row))

(defun gnosis-scheduler--model-config (db config-id)
  "Return (RETENTION . CURRENT) for CONFIG-ID in DB, or signal.
CONFIG-ID must be the FSRS model with today's default parameters, under
any implementation of it: memory states depend on the parameters, not
on which implementation computed them.  CURRENT is non-nil when it names
today's implementation."
  (pcase (car (gnosis-sqlite-select
               db "SELECT desired_retention, implementation FROM scheduler_config
                    WHERE id = ? AND algorithm = ? AND model = ? AND parameters = ?"
               (list config-id gnosis-fsrs--algorithm gnosis-fsrs--model
                     gnosis-fsrs-default-parameters)))
    (`(,retention ,implementation)
     (cons retention (equal implementation gnosis-fsrs--implementation)))
    (_ (error "Unsupported scheduler config"))))

(defun gnosis-scheduler--compute-result
    (event-id thema-id outcome reviewed-at-us review-day state retention)
  "Return EVENT-ID evidence for THEMA-ID from OUTCOME and STATE.
REVIEWED-AT-US, REVIEW-DAY, and RETENTION complete the evidence."
  (let* ((config-id (nth 0 state))
         (prior-stability (nth 1 state))
         (prior-difficulty (nth 2 state))
         (last-reviewed-at-us (nth 3 state))
         (last-review-day (nth 4 state))
         (reps (nth 6 state))
         (lapses (nth 7 state))
         (memory-p (and prior-stability prior-difficulty)))
    (unless (or memory-p (and (null prior-stability) (null prior-difficulty)))
      (error "Incomplete scheduler memory state"))
    (when (and memory-p
               (or (null last-reviewed-at-us) (null last-review-day)
                   (<= reviewed-at-us last-reviewed-at-us)))
      (error "Invalid scheduler review ordering"))
    (let* ((elapsed (if memory-p
                        (gnosis-scheduler--days-between last-review-day review-day)
                      0))
           (prior (and memory-p
                       (list :stability prior-stability
                             :difficulty prior-difficulty)))
           (transition (gnosis-fsrs-transition prior elapsed outcome retention))
           (rating (if (eq outcome 'failure) 1 3))
           (interval (plist-get transition :calendar-interval-days))
           (lapses-after (+ lapses (if (= rating 1) 1 0))))
      (list :event-id event-id :thema-id thema-id :config-id config-id
            :reviewed-at-us reviewed-at-us :review-day review-day
            :rating rating :elapsed-days elapsed
            :prior-stability prior-stability :prior-difficulty prior-difficulty
            :stability (plist-get transition :stability)
            :difficulty (plist-get transition :difficulty)
            :raw-interval-days (plist-get transition :raw-interval-days)
            :calendar-interval-days interval
            :due-day (gnosis-scheduler--add-days review-day interval)
            :reps-before reps :reps-after (1+ reps)
            :lapses-before lapses :lapses-after lapses-after
            :new-p (if (zerop reps) 1 0)))))

(defun gnosis-scheduler--insert-event (db result)
  "Insert immutable scheduler RESULT into DB."
  (gnosis-sqlite-execute
   db
   "INSERT INTO review_events VALUES
      (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)"
   (gnosis-scheduler--evidence result)))

(defun gnosis-scheduler--update-state (db result)
  "Update DB current projection from accepted RESULT."
  (unless
      (= 1
         (gnosis-sqlite-execute
          db
          "UPDATE scheduler_state
              SET stability = ?, difficulty = ?, last_reviewed_at_us = ?,
                  last_review_day = ?, due_day = ?, reps = ?, lapses = ?, config_id = ?
            WHERE thema_id = ?"
          (list (plist-get result :stability) (plist-get result :difficulty)
                (plist-get result :reviewed-at-us) (plist-get result :review-day)
                (plist-get result :due-day) (plist-get result :reps-after)
                (plist-get result :lapses-after) (plist-get result :config-id)
                (plist-get result :thema-id))))
    (error "Scheduler state projection disappeared")))

(defun gnosis-scheduler--evidence (result)
  "Return RESULT's event evidence in review-events column order."
  (mapcar (lambda (key) (plist-get result key)) gnosis-scheduler--event-keys))

(defun gnosis-scheduler--state-result (thema-id state)
  "Return THEMA-ID scheduler plist decoded from replay STATE."
  (list :thema-id thema-id :config-id (nth 0 state)
        :stability (nth 1 state) :difficulty (nth 2 state)
        :last-reviewed-at-us (nth 3 state) :last-review-day (nth 4 state)
        :due-day (nth 5 state) :reps (nth 6 state) :lapses (nth 7 state)
        :suspended (nth 8 state) :new-p (if (zerop (nth 6 state)) 1 0)))

(defun gnosis-scheduler-initialize-themata (rows &optional db)
  "Initialize scheduler storage for ROWS in optional DB.
Each row is (THEMA-ID DUE-DAY SUSPENDED)."
  (unless (and rows
               (cl-every
                (lambda (row)
                  (and (= (length row) 3) (integerp (nth 0 row))
                       (memq (nth 2 row) '(0 1))
                       (progn (gnosis-scheduler--day-time (nth 1 row)) t)))
                rows))
    (error "Invalid scheduler initialization"))
  (let ((db (or db (gnosis--ensure-db))))
    (gnosis-sqlite-with-transaction db
      (let ((config (gnosis-scheduler-active-config db))
            (batch-size
             (max 1 (/ (gnosis-sqlite--max-variable-number db) 10))))
        (dolist (chunk (seq-partition rows batch-size))
          (gnosis-sqlite-execute
           db (concat "INSERT INTO scheduler_baseline VALUES "
                      (mapconcat (lambda (_) "(?,?,?,?)") chunk ","))
           (apply #'append
                  (mapcar (lambda (row)
                            (list (nth 0 row) (nth 1 row) 0 0))
                          chunk)))
          (gnosis-sqlite-execute
           db (concat "INSERT INTO scheduler_state VALUES "
                      (mapconcat (lambda (_) "(?,?,?,?,?,?,?,?,?,?)")
                                 chunk ","))
           (apply #'append
                  (mapcar (lambda (row)
                            (list (nth 0 row) config
                                  nil nil nil nil (nth 1 row)
                                  0 0 (nth 2 row)))
                          chunk))))))))

(defun gnosis-scheduler-initialize-thema
    (thema-id due-day suspended &optional db)
  "Initialize THEMA-ID at DUE-DAY with SUSPENDED state in optional DB."
  (gnosis-scheduler-initialize-themata
   (list (list thema-id due-day suspended)) db)
  (gnosis-scheduler--state-result
   thema-id (list (gnosis-scheduler-active-config db)
                  nil nil nil nil due-day 0 0 suspended)))

(defun gnosis-scheduler--config-table (configs)
  "Return a table of CONFIGS by identity, for replay.
CONFIGS is a list of (ID RETENTION PARAMETERS); PARAMETERS nil means the
model's defaults."
  (let ((table (make-hash-table :test #'eql)))
    (pcase-dolist (`(,id ,retention ,parameters) configs)
      (puthash id (cons retention parameters) table))
    table))

(defun gnosis-scheduler--configs (db)
  "Return DB's configurations of the FSRS model, as a replay table.
Any implementation or parameters of the model replay: an event's
recorded result is its state, and `gnosis-scheduler--drift' reports what
today's FSRS would schedule differently."
  (gnosis-scheduler--config-table
   (gnosis-sqlite-select
    db "SELECT id, desired_retention, parameters FROM scheduler_config
         WHERE algorithm = ? AND model = ?"
    (list gnosis-fsrs--algorithm gnosis-fsrs--model))))

(defun gnosis-scheduler--replay-step (state row configs)
  "Return the scheduler STATE after event ROW under CONFIGS.
CONFIGS is a replay table (`gnosis-scheduler--config-table').
STATE is (CONFIG STABILITY DIFFICULTY LAST-REVIEWED-AT-US LAST-REVIEW-DAY
DUE-DAY REPS LAPSES SUSPENDED).  ROW must continue STATE: its prior memory
is STATE's, its counts and timing follow on, and its due day is its
calendar interval after its review day.  Its stored posterior is taken as
recorded; it is not recomputed, so history stays valid across FSRS
implementations and parameters of the model."
  (pcase-let* ((`(,_config ,stability ,difficulty ,last-at ,last-day ,_due
                           ,reps ,lapses ,suspended)
                state)
               (`(,_id ,_thema ,config ,at ,day ,rating ,elapsed
                       ,prior-stability ,prior-difficulty ,posterior-stability
                       ,posterior-difficulty ,raw-interval ,interval ,due
                       ,reps-before ,reps-after ,lapses-before ,lapses-after
                       ,new-p)
                row)
               (memory-p (and stability t)))
    (unless (and (gethash config configs)
                 (memql rating '(1 3))
                 (eql prior-stability stability)
                 (eql prior-difficulty difficulty)
                 (or (null last-at) (> at last-at))
                 (= elapsed (if memory-p
                                (gnosis-scheduler--days-between last-day day)
                              0))
                 (gnosis-fsrs--finite-number-p posterior-stability)
                 (> posterior-stability 0)
                 (gnosis-fsrs--finite-number-p posterior-difficulty)
                 (<= 1 posterior-difficulty 10)
                 (= interval (max 1 (round raw-interval)))
                 (= due (gnosis-scheduler--add-days day interval))
                 (= reps-before reps) (= reps-after (1+ reps))
                 (= lapses-before lapses)
                 (= lapses-after (+ lapses (if (= rating 1) 1 0)))
                 (= new-p (if (zerop reps) 1 0)))
      (error "Review event does not replay"))
    (list config posterior-stability posterior-difficulty at day due
          reps-after lapses-after suspended)))

(defconst gnosis-scheduler--drift-tolerance 1.0e-4
  "Relative difference from today's FSRS that replay reports.")

(defun gnosis-scheduler--drift (row config)
  "Return how today's FSRS differs from event ROW's recorded result.
CONFIG is ROW's configuration, (RETENTION . PARAMETERS).  Return a list
of (KEY RECORDED COMPUTED) for each of stability, difficulty and raw
interval that differs by more than `gnosis-scheduler--drift-tolerance',
relative to the recorded value, or ((:error MESSAGE)) when today's FSRS
refuses the event; nil when they agree."
  (condition-case err
      (let ((computed (gnosis-fsrs-transition
                       (and (nth 7 row)
                            (list :stability (nth 7 row) :difficulty (nth 8 row)))
                       (nth 6 row) (if (= 1 (nth 5 row)) 'failure 'success)
                       (car config) (cdr config))))
        (cl-loop for (key recorded) in (list (list :stability (nth 9 row))
                                             (list :difficulty (nth 10 row))
                                             (list :raw-interval-days (nth 11 row)))
                 for value = (plist-get computed key)
                 when (> (abs (- value recorded))
                         (* gnosis-scheduler--drift-tolerance (abs recorded)))
                 collect (list key recorded value)))
    (error (list (list :error (error-message-string err))))))

(defun gnosis-scheduler-replay (baseline events configs suspended)
  "Replay BASELINE and EVENTS under CONFIGS with current SUSPENDED fact.
CONFIGS is a replay table (`gnosis-scheduler--config-table'); config 1
starts every baseline.  The state is built from each event's recorded
result.  Each event is also recomputed with today's FSRS under its own
configuration, and `:drift' lists (EVENT-ID . DIFFERENCES) for those it
schedules differently (see `gnosis-scheduler--drift'); drift is reported,
not rejected."
  (unless (and (= (length baseline) 4) (memq suspended '(0 1))
               (cl-every #'integerp baseline)
               (>= (nth 2 baseline) 0) (>= (nth 3 baseline) 0))
    (error "Invalid scheduler replay baseline"))
  (gnosis-scheduler--day-time (nth 1 baseline))
  (unless (gethash 1 configs)
    (error "Replay config missing"))
  (let ((thema-id (nth 0 baseline))
        (state (list 1 nil nil nil nil (nth 1 baseline)
                     (nth 2 baseline) (nth 3 baseline) suspended))
        drift)
    (dolist (row events)
      (unless (and (= (length row) 19)
                   (= thema-id (nth 1 row))
                   (gnosis-scheduler--event-id-p (nth 0 row)))
        (error "Incomplete scheduler replay event"))
      (setq state (gnosis-scheduler--replay-step state row configs))
      (when-let* ((differences (gnosis-scheduler--drift
                                row (gethash (nth 2 row) configs))))
        (push (cons (nth 0 row) differences) drift)))
    (append (gnosis-scheduler--state-result thema-id state)
            (list :drift (nreverse drift)))))

(defun gnosis-scheduler-replay-thema (thema-id suspended &optional db configs)
  "Replay THEMA-ID using current SUSPENDED fact and optional DB.
CONFIGS is DB's replay table, read when nil."
  (let* ((db (or db (gnosis--ensure-db)))
         (baseline (car (gnosis-sqlite-select
                         db "SELECT * FROM scheduler_baseline WHERE thema_id = ?"
                         (list thema-id))))
         (events (gnosis-sqlite-select
                  db "SELECT * FROM review_events WHERE thema_id = ?
                       AND event_id NOT IN (SELECT event_id FROM review_voids)
                       ORDER BY reviewed_at_us, event_id" (list thema-id))))
    (unless baseline (error "Scheduler baseline does not exist"))
    (gnosis-scheduler-replay baseline events
                             (or configs (gnosis-scheduler--configs db))
                             suspended)))

(defun gnosis-scheduler--drift-report (drift reviewed)
  "Report DRIFT, as `gnosis-scheduler-drift' returns it, over REVIEWED themata.
Count the themata that schedule differently and those whose history does
not replay, and list both in a read-only buffer when there are any."
  (let* ((value (lambda (n) (propertize (number-to-string n) 'face 'gnosis-scheduler-report-value)))
         (broken (seq-filter (lambda (thema) (eq :error (car (cadr thema)))) drift))
         (differ (seq-difference drift broken)))
    (if (null drift)
        (message "%s" (propertize (format "All %d reviewed themata replay as today's FSRS schedules them"
                                          reviewed)
                                  'face 'gnosis-scheduler-report-note))
      (with-current-buffer (get-buffer-create "*Gnosis Drift*")
        (let ((inhibit-read-only t))
          (erase-buffer)
          (pcase-dolist (`(,id . ,events) differ)
            (insert (format "Thema %s\n" id))
            (pcase-dolist (`(,event . ,differences) events)
              (insert (format "  review %s\n" (substring event 0 12)))
              (pcase-dolist (`(,key ,recorded ,computed) differences)
                (insert (if (eq key :error)
                            (format "    today's FSRS refuses it: %s\n" recorded)
                          (format "    %s %s, today %s\n" (substring (symbol-name key) 1)
                                  recorded computed))))))
          (pcase-dolist (`(,id (:error ,message)) broken)
            (insert (format "Thema %s does not replay: %s\n" id message)))
          (goto-char (point-min))
          (special-mode))
        (display-buffer (current-buffer)))
      (message "%s of %d reviewed themata schedule differently under today's FSRS; %s do not replay"
               (funcall value (length differ)) reviewed (funcall value (length broken))))))

;;;###autoload
(defun gnosis-scheduler-drift (&optional db)
  "Return the reviews in DB that today's FSRS schedules differently.
Replay every reviewed thema from its recorded results and list
\(THEMA-ID . DRIFT) for each with drift, DRIFT as
`gnosis-scheduler-replay' reports it, or (THEMA-ID (:error MESSAGE)) for
one whose history does not replay.  The collection is read in two
queries and each review recomputed once; nothing is written.
Interactively, report the themata that drift and those that do not
replay (`gnosis-scheduler--drift-report')."
  (interactive)
  (let* ((db (or db (gnosis--ensure-db)))
         (configs (gnosis-scheduler--configs db))
         (events (make-hash-table :test #'eql))
         (themata (gnosis-sqlite-select
                   db "SELECT b.thema_id, b.due_day, b.reps, b.lapses, s.suspended
                         FROM scheduler_baseline AS b
                         JOIN scheduler_state AS s ON s.thema_id = b.thema_id
                        WHERE b.thema_id IN
                              (SELECT thema_id FROM review_events
                                WHERE event_id NOT IN (SELECT event_id FROM review_voids))
                        ORDER BY b.thema_id"))
         drift)
    ;; Newest first per thema, reversed when replayed.
    (dolist (row (gnosis-sqlite-select
                  db "SELECT e.* FROM review_events AS e
                       LEFT JOIN review_voids AS v ON v.event_id = e.event_id
                      WHERE v.event_id IS NULL
                      ORDER BY e.reviewed_at_us, e.event_id"))
      (push row (gethash (nth 1 row) events)))
    (pcase-dolist (`(,id ,due ,reps ,lapses ,suspended) themata)
      (when-let* ((result
                   (condition-case err
                       (plist-get (gnosis-scheduler-replay
                                   (list id due reps lapses)
                                   (reverse (gethash id events)) configs suspended)
                                  :drift)
                     (error (list (list :error (error-message-string err)))))))
        (push (cons id result) drift)))
    (setq drift (nreverse drift))
    (when (called-interactively-p 'interactive)
      (gnosis-scheduler--drift-report drift (length themata)))
    drift))

(defun gnosis-scheduler-rebuild-state (thema-id suspended &optional db)
  "Rebuild THEMA-ID projection with current SUSPENDED fact in optional DB."
  (let ((db (or db (gnosis--ensure-db))))
    (gnosis-sqlite-with-transaction db
      (let ((result (gnosis-scheduler-replay-thema thema-id suspended db)))
        (gnosis-sqlite-execute
         db "INSERT OR REPLACE INTO scheduler_state VALUES
              (?, ?, ?, ?, ?, ?, ?, ?, ?, ?)"
         (list thema-id (plist-get result :config-id)
               (plist-get result :stability) (plist-get result :difficulty)
               (plist-get result :last-reviewed-at-us)
               (plist-get result :last-review-day) (plist-get result :due-day)
               (plist-get result :reps) (plist-get result :lapses) suspended))
        (when-let* ((drift (plist-get result :drift)))
          (display-warning
           'gnosis
           (format "Thema %s: %d review(s) schedule differently under today's FSRS; \
state kept from the recorded results (see `gnosis-scheduler-replay-thema')"
                   thema-id (length drift))))
        result))))

(defun gnosis-scheduler--validate-review
    (event-id thema-id outcome reviewed-at-us review-day)
  "Validate EVENT-ID, THEMA-ID, OUTCOME, REVIEWED-AT-US, and REVIEW-DAY."
  (unless (and (gnosis-scheduler--event-id-p event-id)
               (integerp thema-id) (memq outcome '(failure success))
               (integerp reviewed-at-us) (>= reviewed-at-us 0))
    (error "Invalid scheduler review input"))
  (gnosis-scheduler--day-time review-day))

(defun gnosis-scheduler--retention-config (db retention &optional create)
  "Return the pinned-model config ID in DB recording RETENTION.
Reuse the oldest matching immutable row.  Otherwise return the ID a new
row would receive, inserting it only when CREATE is non-nil."
  (let ((params (list gnosis-fsrs--algorithm gnosis-fsrs--model
                      gnosis-fsrs--implementation
                      gnosis-fsrs-default-parameters)))
    (or (caar (gnosis-sqlite-select
               db "SELECT MIN(id) FROM scheduler_config
                    WHERE algorithm = ? AND model = ? AND implementation = ?
                      AND parameters = ? AND desired_retention = ?"
               (append params (list retention))))
        (let ((id (1+ (caar (gnosis-sqlite-select
                             db "SELECT MAX(id) FROM scheduler_config")))))
          (when create
            (gnosis-sqlite-execute
             db "INSERT INTO scheduler_config VALUES (?, ?, ?, ?, ?, ?)"
             (list id gnosis-fsrs--algorithm gnosis-fsrs--model
                   gnosis-fsrs--implementation retention
                   gnosis-fsrs-default-parameters)))
          id))))

(defun gnosis-scheduler--fresh-result
    (db event-id thema-id outcome reviewed-at-us review-day &optional create)
  "Compute fresh EVENT-ID evidence for THEMA-ID from DB.
OUTCOME, REVIEWED-AT-US, and REVIEW-DAY complete the review facts.
A future exam in scope may raise the retention used (see
`gnosis-exam-review-retention'); the event then names an immutable
config recording that retention, created only when CREATE is non-nil."
  (let ((state
         (car (gnosis-sqlite-select
               db "SELECT config_id, stability, difficulty,
                          last_reviewed_at_us, last_review_day, due_day,
                          reps, lapses, suspended
                     FROM scheduler_state WHERE thema_id = ?"
               (list thema-id)))))
    (unless state (error "Scheduler state does not exist"))
    (gnosis-scheduler--model-config db (car state))
    (let ((latest (caar (gnosis-sqlite-select
                         db "SELECT MAX(reviewed_at_us) FROM review_events WHERE thema_id = ?"
                         (list thema-id))))
          (config (gnosis-scheduler-active-config db)))
      (when (and latest (<= reviewed-at-us latest))
        (error "Review precedes retained history, including voids"))
      (let* ((active (gnosis-scheduler--model-config db config))
             (base (car active))
             (first (gnosis-scheduler--compute-result
                     event-id thema-id outcome reviewed-at-us review-day
                     (cons config (cdr state)) base))
             ;; FSRS stability does not depend on desired retention, so the
             ;; first pass decides it, whatever configuration it named.
             (exam (gnosis-exam-review-retention
                    db thema-id review-day base (plist-get first :stability))))
        (if (and (null exam) (cdr active))
            first
          ;; One configuration for the retention used, under today's
          ;; implementation: found, or made only when CREATE.
          (let ((retention (or exam base)))
            (gnosis-scheduler--compute-result
             event-id thema-id outcome reviewed-at-us review-day
             (cons (gnosis-scheduler--retention-config db retention create) (cdr state))
             retention)))))))

(defun gnosis-scheduler-preview-review
    (event-id thema-id outcome reviewed-at-us review-day)
  "Preview EVENT-ID evidence for THEMA-ID without mutation.
OUTCOME, REVIEWED-AT-US, and REVIEW-DAY complete the review facts."
  (gnosis-scheduler--validate-review
   event-id thema-id outcome reviewed-at-us review-day)
  (gnosis-scheduler--fresh-result
   (gnosis--ensure-db) event-id thema-id outcome reviewed-at-us review-day))

(defun gnosis-scheduler-accept-review
    (event-id thema-id outcome reviewed-at-us review-day
              &optional config preview)
  "Accept EVENT-ID for THEMA-ID, OUTCOME, REVIEWED-AT-US, and REVIEW-DAY.
Return event evidence with `:inserted-p' reporting this call's effect.
When CONFIG is non-nil, reject a fresh acceptance if the configuration
it would record (active, or exam-raised) no longer matches the preview.
PREVIEW also pins the scheduling evidence shown before acceptance.
Retained retries are unchanged."
  (gnosis-scheduler--validate-review
   event-id thema-id outcome reviewed-at-us review-day)
  (let ((db (gnosis--ensure-db))
        (rating (if (eq outcome 'failure) 1 3)))
    (gnosis-sqlite-with-transaction db
      (let ((existing
             (car (gnosis-sqlite-select
                   db "SELECT * FROM review_events WHERE event_id = ?"
                   (list event-id)))))
        (when (gnosis-get 'event-id 'review-voids `(= event-id ,event-id))
          (error "Review event was voided"))
        (if existing
            (append
             (gnosis-scheduler--retained-result
              existing thema-id rating reviewed-at-us review-day)
             '(:inserted-p nil))
          (let ((result (gnosis-scheduler--fresh-result
                         db event-id thema-id outcome reviewed-at-us review-day t)))
            (when (and config (not (= config (plist-get result :config-id))))
              (user-error "Retention changed; cancel and reveal this question again"))
            (when (and preview
                       (not (equal (gnosis-scheduler--evidence preview)
                                   (gnosis-scheduler--evidence result))))
              (user-error "Schedule changed; cancel and reveal this question again"))
            (gnosis-scheduler--insert-event db result)
            (gnosis-scheduler--update-state db result)
            (append result '(:inserted-p t))))))))

(defun gnosis-scheduler-active-config (&optional db)
  "Return the active immutable configuration identity from DB."
  (or (caar (gnosis-sqlite-select (or db (gnosis--ensure-db))
                                 "SELECT config_id FROM scheduler_active WHERE id = 1"))
      (error "Active scheduler configuration missing")))

(defun gnosis-scheduler--validate-retention (retention day)
  "Validate RETENTION for the scheduler calendar in DAY's year.
DAY is a YYYYMMDD integer.  Check the interval at the model's maximum
stability, not just the first review.  Use December 31 so the calendar
conversion includes the largest month and day offsets.  Reject numeric
or calendar overflow rather than silently capping mathematical intervals."
  (unless (and (gnosis-fsrs--finite-number-p retention) (< 0 retention 1))
    (user-error "Desired retention must be a finite number between 0 and 1"))
  (condition-case err
      (let* ((transition
              (gnosis-fsrs-transition
               (list :stability gnosis-fsrs--maximum-stability :difficulty 1.0)
               0 'success retention))
             (interval (plist-get transition :calendar-interval-days))
             (year-end (gnosis--date-to-int (list (/ day 10000) 12 31)))
             (due-day (gnosis-scheduler--add-days year-end interval)))
        (unless (= interval (gnosis-scheduler--days-between year-end due-day))
          (error "Scheduler interval does not round-trip through the calendar")))
    (error (user-error "Unsupported desired retention: %s"
                       (error-message-string err)))))

;;;###autoload
(defun gnosis-scheduler-set-retention (retention)
  "Select user-wide desired RETENTION for future accepted reviews.
Append an immutable configuration snapshot.  Existing due dates and events
are unchanged.  This is a workload preference, not measured topic mastery.
RETENTION must be finite and strictly between 0 and 1.  Before writing,
require its longest model interval to fit the current year's scheduler
calendar.  Unsupported values signal `user-error'; intervals are not capped."
  (interactive "nDesired retention (strictly between 0 and 1): ")
  (gnosis-scheduler--validate-retention retention (gnosis--today-int))
  (let ((db (gnosis--ensure-db)))
    (gnosis-sqlite-with-transaction db
      (let* ((active (gnosis-scheduler-active-config db))
             (old (gnosis-scheduler--model-config db active)))
        ;; The same retention under an earlier implementation still takes
        ;; a new config, under today's.
        (if (and (= (car old) retention) (cdr old)) active
          (let ((id (1+ (caar (gnosis-sqlite-select db "SELECT MAX(id) FROM scheduler_config")))))
            (gnosis-sqlite-execute
             db "INSERT INTO scheduler_config VALUES (?, ?, ?, ?, ?, ?)"
             (list id gnosis-fsrs--algorithm gnosis-fsrs--model gnosis-fsrs--implementation
                   retention gnosis-fsrs-default-parameters))
            (gnosis-sqlite-execute db "UPDATE scheduler_active SET config_id = ? WHERE id = 1"
                                   (list id))
            (message "Desired retention applies to future reviews; current due dates unchanged")
            id))))))

(defun gnosis-scheduler-void-review (correction-id event-id)
  "Void latest effective EVENT-ID using idempotent CORRECTION-ID.
Keep original evidence and current suspension.  Reject superseded targets;
rebuild only from the baseline and effective, non-void review events."
  (unless (and (gnosis-scheduler--event-id-p correction-id)
               (gnosis-scheduler--event-id-p event-id))
    (error "Invalid correction identity"))
  (let ((db (gnosis--ensure-db)))
    (gnosis-sqlite-with-transaction db
      (let* ((retained (gnosis-get 'event-id 'review-voids `(= correction-id ,correction-id)))
             (event (car (gnosis-select '* 'review-events `(= event-id ,event-id))))
             (id (nth 1 event)))
        (cond
         (retained (unless (equal retained event-id) (error "Correction identity conflict")))
         ((null event) (user-error "Review no longer exists"))
         (t
          (unless (equal event-id
                         (caar (gnosis-sqlite-select
                                db "SELECT event_id FROM review_events WHERE thema_id = ?
                                     AND event_id NOT IN (SELECT event_id FROM review_voids)
                                     ORDER BY reviewed_at_us DESC, event_id DESC LIMIT 1" (list id))))
            (user-error "Only the latest effective review can be undone"))
          (gnosis-sqlite-execute db "INSERT INTO review_voids VALUES (?, ?)" (list correction-id event-id))
          (gnosis-scheduler-rebuild-state id (gnosis-get 'suspended 'scheduler-state `(= thema-id ,id)) db)))
        event-id))))

;;; Fitting weights to the review history

(defface gnosis-scheduler-report-value '((t :inherit bold))
  "Face for the numbers of the scheduler's fit and drift reports.")

(defface gnosis-scheduler-report-note '((t :inherit shadow))
  "Face for the defaults and notes of the scheduler's reports.")

(defun gnosis-scheduler--fit-report (fit)
  "Return the echo-area report of FIT, a `gnosis-scheduler-fit-parameters' result."
  (let ((value (lambda (x) (propertize (format "%s" x) 'face 'gnosis-scheduler-report-value)))
        (days (lambda (weights)
                (mapconcat (lambda (w) (format "%.2f" w)) (substring weights 0 4) " "))))
    (concat "FSRS fit from " (funcall value (plist-get fit :items)) " delayed reviews: "
            (pcase (plist-get fit :fitted)
              ('none (propertize "too little history; the defaults stand"
                                 'face 'gnosis-scheduler-report-note))
              ('initial-stability
               (concat "initial stabilities "
                       (funcall value (funcall days (plist-get fit :parameters)))
                       " days "
                       (propertize (format "(defaults %s)"
                                           (funcall days gnosis-fsrs-default-parameters))
                                   'face 'gnosis-scheduler-report-note)))
              ('all (funcall value "every weight fitted"))))))

;;;###autoload
(defun gnosis-scheduler-fit-parameters (&optional db)
  "Return FSRS weights fitted to the effective review history in DB.
The fit is fsrs-rs's optimizer, as Anki runs it, and nothing is written.
Only themata whose whole history is in Gnosis's events take part: one
migrated with earlier reviews (a baseline with repetitions) would enter
as a new card.  The plist has `:parameters', `:items' (the reviews it
could learn from, before the optimizer drops outliers) and `:fitted':
`none' when the weights are the defaults (too little history),
`initial-stability' when only the first four moved, else `all'.
Interactively, report the result in the echo area."
  (interactive)
  (let* ((db (or db (gnosis--ensure-db)))
         (fit (romacs-fsrs-compute-parameters
               (gnosis-sqlite-select
                db "SELECT e.thema_id, e.rating, e.elapsed_days
                      FROM review_events AS e
                      JOIN scheduler_baseline AS b ON b.thema_id = e.thema_id
                     WHERE b.reps = 0
                       AND e.event_id NOT IN (SELECT event_id FROM review_voids)
                     ORDER BY e.reviewed_at_us, e.event_id")))
         (weights (plist-get fit :parameters))
         (kept (lambda (from)
                 (cl-loop for i from from below (length weights)
                          always (< (abs (- (aref weights i)
                                            (aref gnosis-fsrs-default-parameters i)))
                                    1e-6))))
         (result (list :parameters weights
                       :items (plist-get fit :items)
                       :fitted (cond ((funcall kept 0) 'none)
                                     ((funcall kept 4) 'initial-stability)
                                     (t 'all)))))
    (when (called-interactively-p 'interactive)
      (message "%s" (gnosis-scheduler--fit-report result)))
    result))

(provide 'gnosis-scheduler)
;;; gnosis-scheduler.el ends here
