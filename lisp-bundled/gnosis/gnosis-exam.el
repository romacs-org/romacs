;;; gnosis-exam.el --- Exam-date retention ramp and coverage  -*- lexical-binding: t; -*-

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

;; An exam is a name, a date and a tag selection.  For themata in its
;; scope, scheduled reviews accepted before the exam use a desired
;; retention that ramps linearly from the active baseline toward the
;; exam's target over `gnosis-exam-ramp-days', and that is raised
;; further when needed so the next due day falls before the exam.
;;
;; The retention actually used is recorded through an immutable
;; `scheduler_config' row (on a 0.001 grid), so review events stay
;; replayable without knowing exam plans.  Exam plans never rewrite
;; existing due dates or history.
;;
;; Two exceptions are inherent: a review on the day before the exam is
;; due on exam day at the earliest, and an item so stable that even
;; 0.999 schedules it after the exam stays due later (it is then
;; predicted above 99.9% on exam day).  Targets are therefore bounded by
;; the 0.999 grid top.
;;
;; The coverage view partitions each exam's scope into unseen, due,
;; review scheduled before the exam, at risk (no review planned before
;; the exam and predicted below target there without further review)
;; and on track.  RET reviews due work first, then at-risk items not
;; already reviewed today.

;;; Code:

(require 'cl-lib)
(require 'seq)
(require 'subr-x)
(require 'gnosis-db)
(require 'gnosis-fsrs)
(require 'gnosis-sqlite)

(declare-function gnosis-review-loop "gnosis-review"
                  (collector &optional mode target validate))
(declare-function gnosis-review--session-target "gnosis-review" ())
(declare-function gnosis-get-tags--unique "gnosis-tags" ())
(declare-function gnosis-tags-filter-prompt "gnosis-tags" (&optional tags))
(declare-function org-read-date "org"
                  (&optional with-time to-time from-string prompt
                             default-time default-input inactive))

(defgroup gnosis-exam nil
  "Exam-date scheduling and coverage."
  :group 'gnosis)

(defcustom gnosis-exam-default-target 0.95
  "Desired retention reached on the day before a new exam."
  :type 'number)

(defcustom gnosis-exam-ramp-days 30
  "Days before an exam over which retention ramps toward its target."
  :type 'natnum)

(defcustom gnosis-exam-batch-size 40
  "Maximum themata in one coverage batch, or nil for no limit.
Short batches keep reviews quick; start the next batch with one key."
  :type '(choice (const :tag "No limit" nil) natnum))

(defconst gnosis-exam--grid 1000
  "Exam retention resolution: 1/1000.  Bounds new config rows.")

;;; Pure scheduling core

(defun gnosis-exam--day-number (day)
  "Return an absolute day number for YYYYMMDD integer DAY."
  (let ((date (gnosis--int-to-date day)))
    (time-to-days (encode-time 0 0 12 (nth 2 date) (nth 1 date) (nth 0 date)))))

(defun gnosis-exam-days-until (day exam-day)
  "Return calendar days from YYYYMMDD DAY until YYYYMMDD EXAM-DAY."
  (- (gnosis-exam--day-number exam-day) (gnosis-exam--day-number day)))

(defun gnosis-exam-ramp (base target days-left &optional ramp-days)
  "Return ramped retention for DAYS-LEFT before an exam.
Rise linearly from BASE, RAMP-DAYS (default `gnosis-exam-ramp-days')
before the day preceding the exam, to TARGET on that day.  Never return
less than BASE."
  (let* ((ramp-days (max 1 (or ramp-days gnosis-exam-ramp-days)))
         (progress (min 1.0 (max 0.0 (- 1.0 (/ (float (1- days-left)) ramp-days))))))
    (max base (+ base (* (- target base) progress)))))

(defun gnosis-exam--calendar-interval (stability retention)
  "Return the scheduler's calendar interval for STABILITY at RETENTION."
  (max 1 (round (gnosis-fsrs--interval stability retention))))

(defun gnosis-exam-retention (base stability days-left target &optional ramp-days)
  "Return exam retention for a review DAYS-LEFT before an exam, or nil.
BASE is the active desired retention, STABILITY the post-review FSRS
stability (independent of retention) and TARGET the exam target.  The
result is the smallest 0.001 grid value at least the ramp (see
`gnosis-exam-ramp' with RAMP-DAYS) whose interval keeps the next due day
before the exam.  It stops at 0.999: an item still due later is then
predicted above 99.9% on exam day.  Return nil when BASE already
satisfies both, or when the exam is not in the future."
  (when (> days-left 0)
    (let* ((ramp (gnosis-exam-ramp base target days-left ramp-days))
           (cap (max 1 (1- days-left)))
           (fits (lambda (step)
                   (<= (gnosis-exam--calendar-interval
                        stability (/ step (float gnosis-exam--grid)))
                       cap))))
      (unless (and (<= ramp base)
                   (<= (gnosis-exam--calendar-interval stability base) cap))
        (let* ((top (1- gnosis-exam--grid))
               (low (min top (max 1 (ceiling (- (* (max base ramp) gnosis-exam--grid)
                                                 1e-6)))))
               (high top))
          ;; Intervals shrink as retention rises: find the first fitting step.
          (if (funcall fits low)
              (setq high low)
            (while (< (1+ low) high)
              (let ((mid (/ (+ low high) 2)))
                (if (funcall fits mid) (setq high mid) (setq low mid)))))
          (let ((retention (/ high (float gnosis-exam--grid))))
            (and (> retention base) retention)))))))

(defun gnosis-exam-in-scope-p (tags exam-tags)
  "Return non-nil when thema TAGS fall in EXAM-TAGS (INCLUDE . EXCLUDE).
Empty INCLUDE selects every thema; any EXCLUDE tag removes it."
  (and (or (null (car exam-tags))
           (seq-intersection tags (car exam-tags)))
       (not (seq-intersection tags (cdr exam-tags)))))

(defun gnosis-exam-predicted-retention (stability last-review-day exam-day)
  "Return predicted recall on EXAM-DAY from STABILITY and LAST-REVIEW-DAY.
Return nil when memory state is unknown."
  (when (and stability last-review-day)
    (gnosis-fsrs--retrievability
     (max 0 (gnosis-exam-days-until last-review-day exam-day)) stability)))

(defun gnosis-exam-coverage (rows today exam-day target)
  "Return coverage plist for scheduler ROWS on TODAY before EXAM-DAY.
Each row is (ID REPS DUE-DAY STABILITY LAST-REVIEW-DAY).  The buckets
partition the rows:
  :unseen     never reviewed;
  :due        seen and due by TODAY;
  :scheduled  next review already scheduled before EXAM-DAY;
  :at-risk    no review scheduled before EXAM-DAY, and recall on
              EXAM-DAY without further review is predicted below
              TARGET or unknown; lowest prediction first;
  :on-track   the remaining seen themata.
:reviewed-today lists at-risk IDs already reviewed on TODAY.  Nothing
here assumes the outcome of a future review."
  (let (unseen due scheduled risk on-track fresh)
    (pcase-dolist (`(,id ,reps ,due-day ,stability ,last) rows)
      (cond
       ((zerop reps) (push id unseen))
       ((<= due-day today) (push id due))
       ((< due-day exam-day) (push id scheduled))
       (t
        (let ((r (gnosis-exam-predicted-retention stability last exam-day)))
          (if (and r (>= r target))
              (push id on-track)
            (push (cons id (or r -1.0)) risk)
            (when (and last (>= last today)) (push id fresh)))))))
    (list :total (length rows)
          :unseen (nreverse unseen)
          :due (nreverse due)
          :scheduled (nreverse scheduled)
          :at-risk (mapcar #'car (sort (nreverse risk)
                                       (lambda (a b) (< (cdr a) (cdr b)))))
          :on-track (nreverse on-track)
          :reviewed-today (nreverse fresh))))

(defun gnosis-exam-cram-ids (coverage &optional limit)
  "Return the default batch from COVERAGE: due, then at risk.
At-risk themata already reviewed today are left out, so a batch never
repeats today's work.  Keep at most LIMIT IDs when LIMIT is non-nil."
  (let* ((fresh (plist-get coverage :reviewed-today))
         (ids (append (plist-get coverage :due)
                      (seq-remove (lambda (id) (memq id fresh))
                                  (plist-get coverage :at-risk)))))
    (if (and limit (> (length ids) limit)) (seq-take ids limit) ids)))

;;; Storage

(defun gnosis-exam-list (&optional db)
  "Return exams in DB as plists ordered by date and name."
  (mapcar (lambda (row)
            (list :id (nth 0 row) :name (nth 1 row) :exam-day (nth 2 row)
                  :target (nth 3 row) :tags (nth 4 row)))
          (gnosis-sqlite-select
           (or db (gnosis--ensure-db))
           "SELECT id, name, exam_day, target_retention, tags FROM exams
             ORDER BY exam_day, name")))

(defun gnosis-exam--validate (name exam-day target tags)
  "Validate NAME, EXAM-DAY, TARGET and TAGS before a write."
  (unless (and (stringp name) (not (string-blank-p name)))
    (user-error "Exam name must be nonempty"))
  (unless (and (integerp exam-day)
               (equal (gnosis--int-to-date exam-day)
                      (let ((d (decode-time
                                (encode-time 0 0 12 (% exam-day 100)
                                             (% (/ exam-day 100) 100)
                                             (/ exam-day 10000)))))
                        (list (nth 5 d) (nth 4 d) (nth 3 d)))))
    (user-error "Exam date must be a valid YYYYMMDD day"))
  (unless (and (gnosis-fsrs--finite-number-p target)
               (< 0 target)
               (<= target (/ (1- gnosis-exam--grid) (float gnosis-exam--grid))))
    (user-error "Exam target retention must be above 0 and at most %s"
                (/ (1- gnosis-exam--grid) (float gnosis-exam--grid))))
  (unless (and (consp tags) (cl-every #'stringp (car tags))
               (listp (cdr tags)) (cl-every #'stringp (cdr tags)))
    (user-error "Exam tags must be (INCLUDE . EXCLUDE) string lists")))

(defun gnosis-exam-save (name exam-day tags &optional target db)
  "Create or replace exam NAME on EXAM-DAY over TAGS in DB.
TAGS is (INCLUDE . EXCLUDE); TARGET defaults to the existing exam's
target or `gnosis-exam-default-target'.  Return the exam ID."
  (let* ((db (or db (gnosis--ensure-db)))
         (existing (car (gnosis-sqlite-select
                         db "SELECT id, target_retention FROM exams WHERE name = ?"
                         (list name))))
         (target (or target (cadr existing) gnosis-exam-default-target)))
    (gnosis-exam--validate name exam-day target tags)
    (gnosis-sqlite-with-transaction db
      (if existing
          (progn
            (gnosis-sqlite-execute
             db "UPDATE exams SET exam_day = ?, target_retention = ?, tags = ?
                  WHERE id = ?"
             (list exam-day target tags (car existing)))
            (car existing))
        (let ((id (1+ (or (caar (gnosis-sqlite-select db "SELECT MAX(id) FROM exams")) 0))))
          (gnosis-sqlite-execute db "INSERT INTO exams VALUES (?, ?, ?, ?, ?)"
                                 (list id name exam-day target tags))
          id)))))

(defun gnosis-exam-delete (id &optional db)
  "Delete exam ID from DB.  Schedules and review history are unchanged."
  (gnosis-sqlite-execute (or db (gnosis--ensure-db))
                         "DELETE FROM exams WHERE id = ?" (list id)))

(defun gnosis-exam--write-exact (db sql exam &rest params)
  "Run SQL in DB on exactly the stored row equal to EXAM plist.
SQL ends in a WHERE clause matching every exam column; PARAMS precede
EXAM's columns.  Refuse without writing unless one row changed."
  (unless (eql 1 (gnosis-sqlite-execute
                  db sql (append params
                                 (list (plist-get exam :id) (plist-get exam :name)
                                       (plist-get exam :exam-day) (plist-get exam :target)
                                       (plist-get exam :tags)))))
    (user-error "Exam %s changed; press g to refresh.  Nothing was written"
                (plist-get exam :name))))

(defun gnosis-exam--write-target (exam target db)
  "Set TARGET on the row equal to EXAM in DB, changing nothing else."
  (gnosis-exam--validate (plist-get exam :name) (plist-get exam :exam-day)
                         target (plist-get exam :tags))
  (gnosis-exam--write-exact
   db "UPDATE exams SET target_retention = ?
        WHERE id = ? AND name = ? AND exam_day = ? AND target_retention = ? AND tags = ?"
   exam target))

(defun gnosis-exam--write-delete (exam db)
  "Delete the row equal to EXAM from DB."
  (gnosis-exam--write-exact
   db "DELETE FROM exams
        WHERE id = ? AND name = ? AND exam_day = ? AND target_retention = ? AND tags = ?"
   exam))

(defun gnosis-exam-review-retention (db thema-id review-day base stability)
  "Return exam retention for THEMA-ID reviewed on REVIEW-DAY in DB, or nil.
BASE is the active retention and STABILITY the post-review stability.
Take the most demanding of all future exams in scope."
  (let ((exams (and (sqlite-select
                     db "SELECT 1 FROM sqlite_master WHERE type = 'table' AND name = 'exams'")
                    ;; Validated pre-12 fixtures have no exam plans.
                    (gnosis-sqlite-select
                     db "SELECT exam_day, target_retention, tags FROM exams
                          WHERE exam_day > ?" (list review-day)))))
    (when exams
      (let ((tags (mapcar #'car (gnosis-sqlite-select
                                 db "SELECT tag FROM thema_tag WHERE thema_id = ?"
                                 (list thema-id)))))
        (cl-loop for (exam-day target exam-tags) in exams
                 for retention = (and (gnosis-exam-in-scope-p tags exam-tags)
                                      (gnosis-exam-retention
                                       base stability
                                       (gnosis-exam-days-until review-day exam-day)
                                       target))
                 when retention maximize retention)))))

(defun gnosis-exam-scope-rows (exam-tags &optional db)
  "Return unsuspended scheduler rows within EXAM-TAGS in DB.
Rows are (ID REPS DUE-DAY STABILITY LAST-REVIEW-DAY)."
  (let* ((include (car exam-tags))
         (exclude (cdr exam-tags))
         (marks (lambda (tags) (mapconcat (lambda (_) "?") tags ","))))
    (gnosis-sqlite-select
     (or db (gnosis--ensure-db))
     (concat "SELECT s.thema_id, s.reps, s.due_day, s.stability, s.last_review_day
                FROM scheduler_state AS s
               WHERE s.suspended = 0"
             (when include
               (format " AND EXISTS (SELECT 1 FROM thema_tag AS t
                         WHERE t.thema_id = s.thema_id AND t.tag IN (%s))"
                       (funcall marks include)))
             (when exclude
               (format " AND NOT EXISTS (SELECT 1 FROM thema_tag AS t
                         WHERE t.thema_id = s.thema_id AND t.tag IN (%s))"
                       (funcall marks exclude)))
             " ORDER BY s.thema_id")
     (append include exclude))))

(defun gnosis-exam-exam-coverage (exam &optional db today)
  "Return coverage for EXAM plist in DB on TODAY."
  (gnosis-exam-coverage (gnosis-exam-scope-rows (plist-get exam :tags) db)
                        (or today (gnosis--today-int))
                        (plist-get exam :exam-day) (plist-get exam :target)))

;;;###autoload
(defun gnosis-exam-status (&optional db today)
  "Return one count plist per exam in DB on TODAY, for agents and tools.
Keys: :name :exam-day :days-left :target :total :unseen :due :scheduled
:at-risk :on-track; the buckets partition :total (see
`gnosis-exam-coverage').  Counts only; read-only."
  (let ((db (or db (gnosis--ensure-db)))
        (today (or today (gnosis--today-int))))
    (mapcar (lambda (exam)
              (let ((coverage (gnosis-exam-exam-coverage exam db today)))
                (append (list :name (plist-get exam :name)
                              :exam-day (plist-get exam :exam-day)
                              :days-left (gnosis-exam-days-until today (plist-get exam :exam-day))
                              :target (plist-get exam :target)
                              :total (plist-get coverage :total))
                        (cl-loop for key in '(:unseen :due :scheduled :at-risk :on-track)
                                 append (list key (length (plist-get coverage key)))))))
            (gnosis-exam-list db))))

;;; Coverage view

(defface gnosis-exam-title '((t :inherit bold :height 1.1))
  "Exam names in the coverage view.")
(defface gnosis-exam-unseen '((t :inherit shadow))
  "Unseen bucket in the coverage view.")
(defface gnosis-exam-at-risk '((t :inherit warning))
  "At-risk bucket in the coverage view.")
(defface gnosis-exam-due '((t :inherit font-lock-keyword-face))
  "Due bucket in the coverage view.")
(defface gnosis-exam-scheduled '((t :inherit font-lock-constant-face))
  "Review-scheduled-before-exam bucket in the coverage view.")
(defface gnosis-exam-on-track '((t :inherit success))
  "On-track bucket in the coverage view.")

(defconst gnosis-exam-buffer-name "*Gnosis Exams*"
  "Name of the exam coverage buffer.")

(defvar-local gnosis-exam--database nil
  "Database rendered in this coverage buffer.")

(defvar-local gnosis-exam--view nil
  "Token for this coverage view's lifetime, or nil once retired.
A new token is issued only when the view adopts a database; file or
mode reassociation retires the view permanently.")

(defvar-local gnosis-exam--retired nil
  "Non-nil once this buffer stopped being an exam view.")

(defconst gnosis-exam--segments
  '((:on-track gnosis-exam-on-track ?█) (:scheduled gnosis-exam-scheduled ?█)
    (:due gnosis-exam-due ?█) (:at-risk gnosis-exam-at-risk ?█)
    (:unseen gnosis-exam-unseen ?░))
  "Bar segments in display order: coverage key, face and character.")

(defun gnosis-exam--bar (coverage width)
  "Return a bar of WIDTH cells for the partitioned COVERAGE."
  (let ((total (max 1 (plist-get coverage :total)))
        (used 0)
        (last (caar (last gnosis-exam--segments))))
    (mapconcat
     (pcase-lambda (`(,key ,face ,char))
       (let ((cells (if (eq key last)
                        (- width used)
                      (round (* width (/ (float (length (plist-get coverage key)))
                                         total))))))
         (setq cells (max 0 (min cells (- width used))))
         (setq used (+ used cells))
         (propertize (make-string cells char) 'face face)))
     gnosis-exam--segments "")))

(defun gnosis-exam--date-string (day)
  "Return YYYY-MM-DD for YYYYMMDD DAY."
  (apply #'format "%04d-%02d-%02d" (gnosis--int-to-date day)))

(defun gnosis-exam--scope-string (tags)
  "Return a short description of exam TAGS (INCLUDE . EXCLUDE)."
  (concat "scope: "
          (if (car tags) (string-join (car tags) ", ") "all themata")
          (when (cdr tags) (concat ", not " (string-join (cdr tags) ", ")))))

(defun gnosis-exam--insert-exam (exam today)
  "Insert the coverage section for EXAM on TODAY."
  (let* ((coverage (gnosis-exam-exam-coverage exam gnosis-exam--database today))
         (days (gnosis-exam-days-until today (plist-get exam :exam-day)))
         (count (lambda (key face label)
                  (propertize (format "%d %s" (length (plist-get coverage key)) label)
                              'face face)))
         (start (point)))
    (insert (propertize (plist-get exam :name) 'face 'gnosis-exam-title)
            (propertize
             (format "  %s · %s · target %s%% · %s"
                     (gnosis-exam--date-string (plist-get exam :exam-day))
                     (cond ((> days 1) (format "%d days" days))
                           ((= days 1) "tomorrow")
                           ((= days 0) "today")
                           (t "past"))
                     (string-remove-suffix
                      ".0" (format "%.1f" (* 100 (plist-get exam :target))))
                     (gnosis-exam--scope-string (plist-get exam :tags)))
             'face 'shadow)
            "\n  " (gnosis-exam--bar coverage 40) "\n  "
            (format "%d in scope: " (plist-get coverage :total))
            (string-join
             (list (funcall count :due 'gnosis-exam-due "due")
                   (funcall count :at-risk 'gnosis-exam-at-risk "at risk")
                   (funcall count :scheduled 'gnosis-exam-scheduled
                            "scheduled before exam")
                   (funcall count :on-track 'gnosis-exam-on-track "on track")
                   (funcall count :unseen 'gnosis-exam-unseen "unseen"))
             "   ")
            "\n")
    (put-text-property start (point) 'gnosis-exam exam)))

(defun gnosis-exam--render ()
  "Render every exam into the current coverage buffer."
  (let ((inhibit-read-only t)
        (line (line-number-at-pos))
        (today (gnosis--today-int))
        (exams (gnosis-exam-list gnosis-exam--database)))
    (erase-buffer)
    (if (null exams)
        (insert (propertize "No exams.  Press a to add one.\n" 'face 'shadow))
      (dolist (exam exams)
        (gnosis-exam--insert-exam exam today)
        (insert "\n")))
    (insert (propertize
             (concat
              "At risk: no review before the exam, and recall there without one\n"
              "is predicted below target or unknown.\n"
              "RET due, then at risk (not today's)  d due  r at risk early  n unseen"
              "  a add/edit  t target  k delete  g refresh")
             'face 'shadow))
    (goto-char (point-min))
    (forward-line (1- line))))

(defun gnosis-exam--retire ()
  "Retire this exam view; detaching or refreshing never revives it."
  (setq gnosis-exam--view nil
        gnosis-exam--database nil
        gnosis-exam--retired t))

(defun gnosis-exam--context ()
  "Capture the exam view owner ahead of any prompt or confirmation.
Return a plist with :db, :buffer, :view and :exams, the database's exam
plans at capture time.  Outside an exam view only the database counts."
  (let ((in-view (derived-mode-p 'gnosis-exam-mode)))
    (when (and in-view (not (gnosis-exam--owned-p)))
      (user-error "Exam view is stale; press g to refresh"))
    (let ((db (if in-view gnosis-exam--database (gnosis--ensure-db))))
      (list :db db
            :buffer (and in-view (current-buffer))
            :view (and in-view gnosis-exam--view)
            :exams (gnosis-exam-list db)))))

(defun gnosis-exam--owned-p (&optional context)
  "Return non-nil while the current view (or CONTEXT) still owns its writes.
Without CONTEXT, check the current buffer is a live exam view of the
current database.  With CONTEXT from `gnosis-exam--context', also
require the same view occurrence and, when CONTEXT has :exams,
unchanged exam plans."
  (let ((buffer (if context (plist-get context :buffer) (current-buffer)))
        (db (if context (plist-get context :db) gnosis-exam--database)))
    (and (eq db gnosis-db)
         (gnosis-sqlite-live-p db)
         (or (null buffer)
             (and (buffer-live-p buffer)
                  (with-current-buffer buffer
                    (and (derived-mode-p 'gnosis-exam-mode)
                         (not gnosis-exam--retired)
                         (not buffer-file-name)
                         gnosis-exam--view
                         (eq db gnosis-exam--database)
                         (or (null context)
                             (eq gnosis-exam--view (plist-get context :view)))))))
         (or (not (plist-member context :exams))
             (equal (plist-get context :exams) (gnosis-exam-list db))))))

(defun gnosis-exam--check (context)
  "Signal unless CONTEXT still owns the action; return its database."
  (unless (gnosis-exam--owned-p context)
    (user-error "Exam view or plans changed; nothing was written"))
  (plist-get context :db))

(defun gnosis-exam--rerender (context)
  "Redraw CONTEXT's view if that exact occurrence still owns its buffer.
The captured plans are ignored: a successful write changes them."
  (let ((buffer (plist-get context :buffer)))
    (when (and buffer
               (gnosis-exam--owned-p
                (list :db (plist-get context :db) :buffer buffer
                      :view (plist-get context :view))))
      (with-current-buffer buffer (gnosis-exam--render)))))

(defun gnosis-exam--selected (exam context)
  "Return EXAM if it equals a current plan in CONTEXT, else refuse.
A displayed row whose plan changed, or whose ID now names another exam,
never selects or writes anything."
  (or (car (member exam (plist-get context :exams)))
      (user-error "Exam %s changed since shown; press g to refresh"
                  (plist-get exam :name))))

(defun gnosis-exam--at-point ()
  "Return the exam at point, or the only exam."
  (unless (gnosis-exam--owned-p)
    (user-error "Exam view is stale; press g to refresh"))
  (or (get-text-property (point) 'gnosis-exam)
      (get-text-property (max (point-min) (1- (line-beginning-position))) 'gnosis-exam)
      (let ((exams (gnosis-exam-list gnosis-exam--database)))
        (and (= (length exams) 1) (car exams)))
      (user-error "No exam at point")))

(defun gnosis-exam--start (bucket)
  "Start a scheduled review of BUCKET for the exam at point.
BUCKET is `cram', `due', `at-risk' or `unseen'."
  (let* ((context (gnosis-exam--context))
         (exam (gnosis-exam--selected (gnosis-exam--at-point) context))
         (db (plist-get context :db))
         (_ (require 'gnosis-review))
         (target (gnosis-review--session-target))
         (coverage (gnosis-exam-exam-coverage exam db))
         (ids (if (eq bucket 'cram)
                  (gnosis-exam-cram-ids coverage gnosis-exam-batch-size)
                (let ((ids (plist-get coverage (intern (format ":%s" bucket)))))
                  (if (and gnosis-exam-batch-size (> (length ids) gnosis-exam-batch-size))
                      (seq-take ids gnosis-exam-batch-size)
                    ids)))))
    (if (null ids)
        (message "Nothing %s for %s"
                 (pcase bucket ('cram "due or at risk") ('at-risk "at risk")
                        (_ (symbol-name bucket)))
                 (plist-get exam :name))
      ;; Plans may change during the batch; only the view must persist.
      (let ((owner (list :db db :buffer (plist-get context :buffer)
                         :view (plist-get context :view))))
        (gnosis-review-loop
         ids nil target
         (lambda ()
           (unless (gnosis-exam--owned-p owner)
             (user-error "Exam view changed; reopen it before retrying")))))
      (gnosis-exam--rerender context))))

(defun gnosis-exam-cram ()
  "Review due themata of the exam at point, then those at risk.
At-risk themata already reviewed today are skipped."
  (interactive)
  (gnosis-exam--start 'cram))

(defun gnosis-exam-review-due ()
  "Review due themata of the exam at point."
  (interactive)
  (gnosis-exam--start 'due))

(defun gnosis-exam-review-at-risk ()
  "Review at-risk themata early, before they are due.
At risk means no review is scheduled before the exam and predicted
recall on exam day without further review is below target."
  (interactive)
  (gnosis-exam--start 'at-risk))

(defun gnosis-exam-review-unseen ()
  "Review never-seen themata of the exam at point."
  (interactive)
  (gnosis-exam--start 'unseen))

(defun gnosis-exam-refresh ()
  "Refresh the coverage view against the current database.
A view retired by file or mode reassociation cannot be refreshed."
  (interactive)
  (unless (derived-mode-p 'gnosis-exam-mode)
    (user-error "Not an exam view"))
  (when (or gnosis-exam--retired buffer-file-name)
    (user-error "This buffer is no longer an exam view; open M-x gnosis-exam"))
  (let ((db (gnosis--ensure-db)))
    (unless (and gnosis-exam--view (eq db gnosis-exam--database))
      (setq gnosis-exam--view (list 'gnosis-exam-view)
            gnosis-exam--database db)))
  (gnosis-exam--render))

(defun gnosis-exam-add (name exam-day tags &optional context)
  "Add or update exam NAME on EXAM-DAY over TAGS (INCLUDE . EXCLUDE).
CONTEXT is the owner captured before the prompts (see
`gnosis-exam--context'); refuse when its database, view or plans
changed meanwhile."
  (interactive
   (let ((context (gnosis-exam--context)))
     (require 'gnosis-tags)
     (require 'org)
     (let* ((current (and (plist-get context :buffer)
                          (get-text-property (point) 'gnosis-exam)))
            (current (and current (gnosis-exam--selected current context)))
            (name (read-string "Exam: " (plist-get current :name)))
            (date (org-read-date nil nil nil "Exam date: "
                                 nil (and current (gnosis-exam--date-string
                                                   (plist-get current :exam-day))))))
       (list name
             (string-to-number (string-replace "-" "" date))
             (gnosis-tags-filter-prompt (gnosis-get-tags--unique))
             context))))
  (let ((context (or context (gnosis-exam--context))))
    (gnosis-exam-save name exam-day tags nil (gnosis-exam--check context))
    (gnosis-exam--rerender context)))

(defun gnosis-exam-set-target (target &optional exam context)
  "Set the exam-day TARGET retention of EXAM, by default the one at point.
CONTEXT is the owner captured before the prompt; refuse when it changed."
  (interactive
   (let* ((context (gnosis-exam--context))
          (exam (gnosis-exam--selected (gnosis-exam--at-point) context)))
     (list (read-number (format "Exam target retention (at most %s): "
                                (/ (1- gnosis-exam--grid) (float gnosis-exam--grid)))
                        (plist-get exam :target))
           exam context)))
  (let* ((context (or context (gnosis-exam--context)))
         (exam (gnosis-exam--selected (or exam (gnosis-exam--at-point)) context)))
    (gnosis-exam--write-target exam target (gnosis-exam--check context))
    (gnosis-exam--rerender context)))

(defun gnosis-exam-delete-at-point ()
  "Delete the exam at point; schedules and history are unchanged."
  (interactive)
  (let* ((context (gnosis-exam--context))
         (exam (gnosis-exam--selected (gnosis-exam--at-point) context)))
    (when (yes-or-no-p (format "Delete exam %s? " (plist-get exam :name)))
      (gnosis-exam--write-delete exam (gnosis-exam--check context))
      (gnosis-exam--rerender context))))

(defvar-keymap gnosis-exam-mode-map
  :parent special-mode-map
  "RET" #'gnosis-exam-cram
  "c" #'gnosis-exam-cram
  "d" #'gnosis-exam-review-due
  "r" #'gnosis-exam-review-at-risk
  "n" #'gnosis-exam-review-unseen
  "a" #'gnosis-exam-add
  "t" #'gnosis-exam-set-target
  "k" #'gnosis-exam-delete-at-point
  "g" #'gnosis-exam-refresh)

(define-derived-mode gnosis-exam-mode special-mode "Gnosis Exams"
  "Exam coverage: unseen, due and at-risk themata per exam."
  (setq-local truncate-lines t)
  ;; File association keeps the major mode but ends the view.
  (add-hook 'after-set-visited-file-name-hook #'gnosis-exam--retire nil t)
  (add-hook 'change-major-mode-hook #'gnosis-exam--retire nil t))

;;;###autoload
(defun gnosis-exam ()
  "Show exam coverage and start focused review batches."
  (interactive)
  (let ((buffer (get-buffer gnosis-exam-buffer-name)))
    (unless (and buffer
                 (with-current-buffer buffer
                   (and (derived-mode-p 'gnosis-exam-mode)
                        (not gnosis-exam--retired) (not buffer-file-name))))
      ;; Never repurpose a retired or unrelated buffer of that name.
      (setq buffer (generate-new-buffer gnosis-exam-buffer-name))
      (with-current-buffer buffer (gnosis-exam-mode)))
    (with-current-buffer buffer
      (gnosis-exam-refresh))
    (pop-to-buffer-same-window buffer)))

(provide 'gnosis-exam)
;;; gnosis-exam.el ends here
