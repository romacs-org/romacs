;;; gnosis-test-exam.el --- Exam-date scheduling and coverage tests  -*- lexical-binding: t; -*-

;; Copyright (C) 2026  Free Software Foundation, Inc.

;;; Commentary:

;; Verify the exam retention ramp, the interval cap at the exam date,
;; scope isolation, replay without exam plans, schema 12 migration and
;; one-key coverage batches.

;;; Code:

(require 'ert)
(require 'cl-lib)
(require 'gnosis)
(require 'gnosis-exam)
(require 'gnosis-review)
(require 'gnosis-scheduler)
(require 'gnosis-test-helpers)
(require 'gnosis-fixture-schema-v9)
(require 'gnosis-test-schema-v8)

(defun gnosis-test-exam--snapshot (file)
  "Return version, schema objects and all rows of FILE without initialization."
  (let ((db (sqlite-open file)))
    (unwind-protect
        (list (sqlite-select db "PRAGMA user_version")
              (sqlite-select db "SELECT type, name, tbl_name, sql FROM sqlite_master ORDER BY type, name")
              (mapcar (lambda (row)
                        (cons (car row) (sqlite-select db (format "SELECT * FROM %s ORDER BY 1" (car row)))))
                      (sqlite-select db "SELECT name FROM sqlite_master WHERE type = 'table' ORDER BY name")))
      (sqlite-close db))))

;;; Pure scheduling math

(ert-deftest gnosis-test-exam-ramp-is-monotonic-and-bounded ()
  "Retention never falls as the exam nears, starts at base, ends at target."
  (let ((previous 0.0))
    (cl-loop for days-left from 90 downto 1
             for ramp = (gnosis-exam-ramp 0.9 0.95 days-left 30)
             do (should (>= ramp previous))
             (should (<= 0.9 ramp 0.95))
             (setq previous ramp)))
  (should (= 0.9 (gnosis-exam-ramp 0.9 0.95 31 30)))
  (should (= 0.9 (gnosis-exam-ramp 0.9 0.95 90 30)))
  (should (< (abs (- 0.95 (gnosis-exam-ramp 0.9 0.95 1 30))) 1e-12))
  ;; A target below the configured baseline never lowers retention.
  (should (= 0.92 (gnosis-exam-ramp 0.92 0.85 1 30))))

(ert-deftest gnosis-test-exam-retention-caps-interval-before-exam ()
  "The chosen retention keeps the next due day strictly before the exam."
  (dolist (stability '(0.5 2.3 8.0 21.0 60.0 180.0 400.0))
    (dolist (days-left '(2 3 5 10 20 45 90))
      (let* ((retention (gnosis-exam-retention 0.9 stability days-left 0.95 30))
             (used (or retention 0.9))
             (interval (gnosis-exam--calendar-interval stability used)))
        (when retention
          (should (< 0.9 retention 1.0))
          ;; Grid values bound the number of immutable config rows.
          (should (= retention (/ (round (* retention 1000)) 1000.0))))
        (unless (and retention (>= retention 0.999))
          (should (<= interval (max 1 (1- days-left)))))
        ;; Minimal: one grid step lower would either breach the cap or the ramp.
        (when (and retention (> retention 0.901))
          (let ((lower (- retention 0.001)))
            (should (or (< lower (gnosis-exam-ramp 0.9 0.95 days-left 30))
                        (> (gnosis-exam--calendar-interval stability lower)
                           (max 1 (1- days-left)))))))))))

(ert-deftest gnosis-test-exam-retention-monotonic-toward-exam ()
  "For fixed memory, required retention never falls as the exam nears."
  (dolist (stability '(3.0 15.0 50.0))
    (let ((previous 0.0))
      (cl-loop for days-left from 60 downto 2
               for retention = (or (gnosis-exam-retention 0.9 stability days-left 0.95 30) 0.9)
               do (should (>= retention previous))
               (setq previous retention)))))

(ert-deftest gnosis-test-exam-retention-nil-without-pressure ()
  "Far or past exams leave the baseline unchanged."
  (should-not (gnosis-exam-retention 0.9 2.0 120 0.95 30))
  (should-not (gnosis-exam-retention 0.9 2.0 0 0.95 30))
  (should-not (gnosis-exam-retention 0.9 2.0 -5 0.95 30))
  ;; A stricter baseline already satisfies a lower target and the cap.
  (should-not (gnosis-exam-retention 0.97 1.0 5 0.95 30)))

(ert-deftest gnosis-test-exam-scope-predicate ()
  (should (gnosis-exam-in-scope-p '("anat" "limb") '(("anat"))))
  (should (gnosis-exam-in-scope-p '("x") '(nil)))
  (should-not (gnosis-exam-in-scope-p '("phys") '(("anat"))))
  (should-not (gnosis-exam-in-scope-p '("anat" "skip") '(("anat") "skip")))
  (should-not (gnosis-exam-in-scope-p nil '(("anat")))))

(ert-deftest gnosis-test-exam-coverage-buckets ()
  "Partition seen themata into due, scheduled before exam, at risk and on track."
  (let* ((rows '((1 0 20261001 nil nil)            ; unseen
                 (2 3 20261010 1.0 20261008)       ; due, weak
                 (3 3 20261130 400.0 20261009)     ; strong, due after exam
                 (4 2 20261020 5.0 20261009)       ; review scheduled before exam
                 (6 2 20261105 5.0 20261009)       ; due after exam, weak
                 (7 2 20261101 1.0 20261012)       ; reviewed today, due on exam day
                 (5 1 20261005 nil nil)))          ; legacy memory unknown, due
         (coverage (gnosis-exam-coverage rows 20261012 20261101 0.95)))
    (should (= 7 (plist-get coverage :total)))
    (should (equal '(1) (plist-get coverage :unseen)))
    (should (equal '(2 5) (plist-get coverage :due)))
    (should (equal '(4) (plist-get coverage :scheduled)))
    (should (equal '(7 6) (plist-get coverage :at-risk)))
    (should (equal '(3) (plist-get coverage :on-track)))
    ;; Buckets partition the scope.
    (should (= 7 (cl-loop for key in '(:unseen :due :scheduled :at-risk :on-track)
                          sum (length (plist-get coverage key)))))
    ;; Due work first; never a card already reviewed today.
    (should (equal '(2 5 6) (gnosis-exam-cram-ids coverage)))
    (should (equal '(2 5) (gnosis-exam-cram-ids coverage 2)))))

(ert-deftest gnosis-test-exam-coverage-unknown-memory-not-due ()
  "Unknown memory without a review before the exam is at risk, not on track."
  (let ((coverage (gnosis-exam-coverage '((8 1 20261201 nil nil))
                                        20261012 20261101 0.95)))
    (should (equal '(8) (plist-get coverage :at-risk)))
    (should (equal '(8) (gnosis-exam-cram-ids coverage)))))

;;; Storage and scheduler integration

(defmacro gnosis-test-exam--with-db (&rest body)
  "Run BODY in a fresh isolated database."
  (declare (indent 0) (debug t))
  `(gnosis-test-with-db ,@body))

(defun gnosis-test-exam--thema (id tags)
  "Insert basic thema ID with TAGS, due on 2026-09-01."
  (gnosis-sqlite-with-transaction gnosis-db
    (gnosis--insert-into 'themata `([,id "basic" "Q" ("") ("A") nil]))
    (gnosis-scheduler-initialize-thema id 20260901 0)
    (dolist (tag tags) (gnosis--insert-into 'thema-tag `([,id ,tag]))))
  id)

(defun gnosis-test-exam--review (id n day &optional outcome)
  "Accept review N of thema ID on YYYYMMDD DAY with OUTCOME."
  (gnosis-scheduler-accept-review
   (format "%064x" (+ (* id 1000) n)) id (or outcome 'success)
   (+ (* id 1000000) (* n 1000)) day))

(defun gnosis-test-exam--history (ids)
  "Return effective review evidence for IDS from fixed review days."
  (cl-loop for id in ids
           collect (cl-loop for (n . day) in '((1 . 20261001) (2 . 20261004)
                                               (3 . 20261012) (4 . 20261020))
                            collect (gnosis-test-exam--review id n day))))

(ert-deftest gnosis-test-exam-no-exam-and-out-of-scope-are-unchanged ()
  "Without an in-scope exam, accepted evidence equals the baseline scheduler."
  (let (baseline with-exam)
    (gnosis-test-exam--with-db
      (gnosis-test-exam--thema 11 '("phys"))
      (setq baseline (gnosis-test-exam--history '(11))))
    (gnosis-test-exam--with-db
      (gnosis-test-exam--thema 11 '("phys"))
      (gnosis-exam-save "Anatomy" 20261030 '(("anat")))
      (gnosis-exam-save "Past" 20260901 '(nil))
      (setq with-exam (gnosis-test-exam--history '(11)))
      (should (equal '((1)) (sqlite-select gnosis-db "SELECT id FROM scheduler_config"))))
    (should (equal baseline with-exam))
    (should (cl-every (lambda (r) (= 1 (plist-get r :config-id))) (car baseline)))))

(ert-deftest gnosis-test-exam-review-after-an-implementation-upgrade ()
  "Preview and accept agree on the one configuration an exam review makes.
Under an active configuration naming an earlier implementation, a review
the exam raises records a single new configuration, the one its preview
named, under today's implementation and the raised retention."
  (gnosis-test-exam--with-db
    (gnosis-test-exam--thema 21 '("anat"))
    (gnosis-exam-save "Anatomy" 20261030 '(("anat")) 0.95)
    (let* ((gnosis-fsrs--implementation "fsrs-rs-7.0.0")
           (preview (gnosis-scheduler-preview-review
                     (format "%064x" 21001) 21 'success 21001000 20261001))
           (accepted (gnosis-scheduler-accept-review
                      (format "%064x" 21001) 21 'success 21001000 20261001
                      (plist-get preview :config-id) preview)))
      (should (plist-get accepted :inserted-p))
      (should (= (plist-get preview :config-id) (plist-get accepted :config-id)))
      (pcase-let ((`((,id ,implementation ,retention))
                   (gnosis-sqlite-select
                    gnosis-db "SELECT id, implementation, desired_retention
                                 FROM scheduler_config WHERE id > 1")))
        (should (= id (plist-get accepted :config-id)))
        (should (equal "fsrs-rs-7.0.0" implementation))
        (should (> retention 0.9))))))

(ert-deftest gnosis-test-exam-in-scope-caps-due-and-replays ()
  "In-scope reviews stay due before the exam and replay without exam data."
  (gnosis-test-exam--with-db
    (gnosis-test-exam--thema 21 '("anat"))
    (gnosis-test-exam--thema 22 '("phys"))
    (gnosis-exam-save "Anatomy" 20261030 '(("anat")) 0.95)
    (let ((configs-before (sqlite-select gnosis-db "SELECT COUNT(*) FROM scheduler_config"))
          (preview (gnosis-scheduler-preview-review
                    (format "%064x" 21001) 21 'success 21001000 20261001)))
      ;; Preview predicts the config row without writing it.
      (should (equal configs-before
                     (sqlite-select gnosis-db "SELECT COUNT(*) FROM scheduler_config")))
      (let ((accepted (gnosis-scheduler-accept-review
                       (format "%064x" 21001) 21 'success 21001000 20261001
                       (plist-get preview :config-id) preview)))
        (should (plist-get accepted :inserted-p))
        (should (equal (plist-get preview :config-id) (plist-get accepted :config-id)))))
    (cl-loop for (n . day) in '((2 . 20261004) (3 . 20261012) (4 . 20261020))
             do (gnosis-test-exam--review 21 n day)
             (gnosis-test-exam--review 22 n day))
    (gnosis-test-exam--review 22 5 20261021)
    (dolist (row (sqlite-select gnosis-db
                                "SELECT e.due_day, e.review_day, c.desired_retention
                                   FROM review_events e JOIN scheduler_config c ON c.id = e.config_id
                                  WHERE e.thema_id = 21"))
      (should (< (nth 0 row) 20261030))
      (should (> (nth 2 row) 0.9)))
    ;; Out-of-scope thema keeps the active configuration.
    (should (equal '((1)) (sqlite-select gnosis-db
                                         "SELECT DISTINCT config_id FROM review_events WHERE thema_id = 22")))
    ;; Retention rises as the exam nears.
    (let ((retentions (mapcar #'car (sqlite-select
                                     gnosis-db "SELECT c.desired_retention FROM review_events e
                                                  JOIN scheduler_config c ON c.id = e.config_id
                                                 WHERE e.thema_id = 21 ORDER BY e.reviewed_at_us"))))
      (should (equal retentions (sort (copy-sequence retentions) #'<=))))
    ;; Replay depends only on immutable events and configs, not exam plans.
    (let ((state (sqlite-select gnosis-db "SELECT * FROM scheduler_state WHERE thema_id = 21")))
      (gnosis-exam-delete (plist-get (car (gnosis-exam-list)) :id))
      (gnosis-scheduler-rebuild-state 21 0)
      (should (equal state (sqlite-select gnosis-db "SELECT * FROM scheduler_state WHERE thema_id = 21"))))))

(ert-deftest gnosis-test-exam-config-rows-are-reused ()
  "Equal exam retentions reuse one immutable configuration row."
  (gnosis-test-exam--with-db
    (dolist (id '(31 32 33)) (gnosis-test-exam--thema id '("anat")))
    (gnosis-exam-save "Anatomy" 20261010 '(("anat")) 0.95)
    (dolist (id '(31 32 33)) (gnosis-test-exam--review id 1 20261001))
    (should (= 1 (caar (sqlite-select gnosis-db
                                      "SELECT COUNT(DISTINCT config_id) FROM review_events"))))
    (should (= 2 (caar (sqlite-select gnosis-db "SELECT COUNT(*) FROM scheduler_config"))))
    (should (= 1 (gnosis-scheduler-active-config)))))

(ert-deftest gnosis-test-exam-stale-preview-is-refused ()
  "Changing the exam after preview refuses acceptance without writes."
  (gnosis-test-exam--with-db
    (gnosis-test-exam--thema 41 '("anat"))
    (gnosis-test-exam--review 41 1 20261001)
    (let* ((event (format "%064x" 41002))
           (preview (gnosis-scheduler-preview-review event 41 'success 41002000 20261004)))
      (gnosis-exam-save "Anatomy" 20261008 '(("anat")) 0.95)
      (should-error (gnosis-scheduler-accept-review event 41 'success 41002000 20261004
                                                    (plist-get preview :config-id) preview)
                    :type 'user-error)
      (should (= 1 (caar (sqlite-select gnosis-db "SELECT COUNT(*) FROM review_events"))))
      (should (= 1 (caar (sqlite-select gnosis-db "SELECT COUNT(*) FROM scheduler_config")))))))

(ert-deftest gnosis-test-exam-scope-rows-and-save ()
  (gnosis-test-exam--with-db
    (gnosis-test-exam--thema 51 '("anat"))
    (gnosis-test-exam--thema 52 '("anat" "skip"))
    (gnosis-test-exam--thema 53 '("phys"))
    (gnosis-sqlite-execute gnosis-db "UPDATE scheduler_state SET suspended = 1 WHERE thema_id = 51")
    (gnosis-test-exam--thema 54 '("anat"))
    (should (equal '(52 54) (mapcar #'car (gnosis-exam-scope-rows '(("anat"))))))
    (should (equal '(54) (mapcar #'car (gnosis-exam-scope-rows '(("anat") "skip")))))
    (should (equal '(52 53 54) (mapcar #'car (gnosis-exam-scope-rows '(nil))))))
  (gnosis-test-exam--with-db
    (let ((id (gnosis-exam-save "Final" 20261120 '(("anat")))))
      (should (= id (gnosis-exam-save "Final" 20261121 '(("anat" "neuro")))))
      (should (equal (list :id id :name "Final" :exam-day 20261121 :target 0.95
                           :tags '(("anat" "neuro")))
                     (car (gnosis-exam-list)))))
    (should-error (gnosis-exam-save "" 20261120 '(nil)) :type 'user-error)
    (should-error (gnosis-exam-save "Bad" 20261131 '(nil)) :type 'user-error)
    (should-error (gnosis-exam-save "Bad" 20261120 '(nil) 1.0) :type 'user-error)
    ;; The retention grid ends at 0.999: higher targets are unsupported.
    (should-error (gnosis-exam-save "Bad" 20261120 '(nil) 0.9995) :type 'user-error)
    (should (gnosis-exam-save "High" 20261120 '(nil) 0.999))))

(ert-deftest gnosis-test-exam-high-target-and-baseline-reach-the-grid-top ()
  "Supported top targets and high baselines meet the ramp on the grid."
  (dolist (base '(0.9 0.98 0.995))
    ;; The day before the exam the ramp reaches the supported top value.
    (should (= 0.999 (gnosis-exam-retention base 2.0 1 0.999 30)))
    (dolist (days-left '(2 5 20))
      (let ((retention (gnosis-exam-retention base 2.0 days-left 0.999 30)))
        (should (<= retention 0.999))
        (should (>= (+ retention 1e-9) (gnosis-exam-ramp base 0.999 days-left 30)))))))

(ert-deftest gnosis-test-exam-status-is-read-only-counts ()
  (gnosis-test-exam--with-db
    (dolist (id '(71 72)) (gnosis-test-exam--thema id '("anat")))
    (gnosis-test-exam--review 71 1 20261001)
    (gnosis-exam-save "Anatomy" 20261030 '(("anat")))
    (let ((before (sqlite-select gnosis-db "SELECT * FROM scheduler_state")))
      (should (equal '((:name "Anatomy" :exam-day 20261030 :days-left 20 :target 0.95
                        :total 2 :unseen 1 :due 1 :scheduled 0 :at-risk 0
                        :on-track 0))
                     (gnosis-exam-status nil 20261010)))
      (should (equal before (sqlite-select gnosis-db "SELECT * FROM scheduler_state"))))))

;;; Migration

(defun gnosis-test-exam--schema-11 ()
  "Create a populated schema 11 database on this connection."
  (gnosis-fixture-create-v9)
  (gnosis-db--migrate-v10)
  (gnosis-db--migrate-v11)
  (gnosis-fixture-add-basic 101 "Question")
  (gnosis-scheduler-accept-review (make-string 64 ?a) 101 'success 1000000 20260913)
  (gnosis--insert-into 'practice-events '(["retained" 101 "batch" 1 2000000 1]))
  (gnosis-db--check-schema gnosis-db 11))

(ert-deftest gnosis-test-exam-migration-11-to-12-preserves-rows ()
  (gnosis-test-with-old-db
    (gnosis-test-exam--schema-11)
    (let ((before (nth 2 (gnosis-test-exam--snapshot gnosis-test--db-file))))
      (dotimes (_ 2)
        (gnosis-db-init)
        (should (= 12 (gnosis--db-version)))
        (gnosis-db--check-schema gnosis-db 12)
        (let ((after (nth 2 (gnosis-test-exam--snapshot gnosis-test--db-file))))
          (should (equal '("exams") (assoc "exams" after)))
          (should (equal before (assoc-delete-all "exams" (copy-sequence after))))))
      (gnosis-exam-save "Anatomy" 20261030 '(("test")))
      (should (= 1 (length (gnosis-exam-list)))))))

(ert-deftest gnosis-test-exam-migration-fault-rolls-back ()
  (dolist (fault '(error quit))
    (gnosis-test-with-old-db
      (gnosis-test-exam--schema-11)
      (let ((before (gnosis-test-exam--snapshot gnosis-test--db-file))
            (set-version (symbol-function 'gnosis--db-set-version)))
        (cl-letf (((symbol-function 'gnosis--db-set-version)
                   (lambda (target)
                     (funcall set-version target)
                     (when (= target 12) (signal fault '("Exam migration fault"))))))
          (should (eq fault (condition-case err (progn (gnosis-db-init) nil)
                              (error (car err)) (quit 'quit)))))
        (should (equal before (gnosis-test-exam--snapshot gnosis-test--db-file)))
        (gnosis-db-init)
        (should (= 12 (gnosis--db-version)))))))

(ert-deftest gnosis-test-exam-migration-refuses-v11-with-exams-table ()
  "A schema 11 database that already has an exams table is not migrated."
  (gnosis-test-with-old-db
    (gnosis-test-exam--schema-11)
    (sqlite-execute gnosis-db "CREATE TABLE exams (id INTEGER)")
    (let ((before (gnosis-test-exam--snapshot gnosis-test--db-file)))
      (should-error (gnosis-db-init))
      (should (equal before (gnosis-test-exam--snapshot gnosis-test--db-file))))))

(ert-deftest gnosis-test-exam-fresh-schema-matches-migration ()
  "Fresh creation and migration produce the same exams declaration."
  (let (fresh migrated)
    (gnosis-test-with-db
      (setq fresh (sqlite-select gnosis-db "SELECT sql FROM sqlite_master WHERE name = 'exams'")))
    (gnosis-test-with-old-db
      (gnosis-test-exam--schema-11)
      (gnosis-db-init)
      (setq migrated (sqlite-select gnosis-db "SELECT sql FROM sqlite_master WHERE name = 'exams'")))
    (should fresh)
    (should (equal fresh migrated))))

;;; Coverage view

(defmacro gnosis-test-exam--in-view (&rest body)
  "Run BODY in a fresh rendered exam view, killed afterwards."
  (declare (indent 0) (debug t))
  `(save-window-excursion
     (gnosis-exam)
     (let ((view (get-buffer gnosis-exam-buffer-name)))
       (unwind-protect
           (with-current-buffer view (goto-char (point-min)) ,@body)
         (when (buffer-live-p view)
           (with-current-buffer view (set-buffer-modified-p nil))
           (kill-buffer view))))))

(defun gnosis-test-exam--goto (name)
  "Move point to the header of exam NAME."
  (goto-char (point-min))
  (search-forward name)
  (beginning-of-line))

(ert-deftest gnosis-test-exam-view-renders-and-starts-batch-with-one-key ()
  "RET starts the due-first batch of the exam at point without prompts."
  (gnosis-test-exam--with-db
    (dolist (id '(61 62 63)) (gnosis-test-exam--thema id '("anat")))
    (gnosis-test-exam--review 61 1 (gnosis--today-int))
    (gnosis-exam-save "Anatomy" (gnosis-scheduler--add-days (gnosis--today-int) 20)
                      '(("anat") "skip"))
    (let (started)
      (gnosis-test-exam--in-view
        (should (string-match-p "Anatomy" (buffer-string)))
        (should (string-match-p "scope: anat, not skip" (buffer-string)))
        (should (string-match-p "3 in scope" (buffer-string)))
        (should (string-match-p "2 unseen" (buffer-string)))
        (should (string-match-p "1 scheduled before exam" (buffer-string)))
        (should (string-match-p "0 at risk" (buffer-string)))
        (cl-letf (((symbol-function 'gnosis-review-loop)
                   (lambda (ids &optional mode _target validate)
                     (funcall validate)
                     (setq started (list ids mode))))
                  ((symbol-function 'read-from-minibuffer)
                   (lambda (&rest _) (error "Unexpected prompt"))))
          ;; Just reviewed and scheduled before the exam: nothing to cram.
          (call-interactively (keymap-lookup gnosis-exam-mode-map "RET"))
          (should-not started)
          (call-interactively (keymap-lookup gnosis-exam-mode-map "n"))
          (should (equal '((62 63) nil) started)))))))

(ert-deftest gnosis-test-exam-cram-after-acceptance-skips-reviewed-and-serves-due ()
  "An accepted review leaves the next RET to pending due work."
  (gnosis-test-exam--with-db
    (let* ((today (gnosis--today-int))
           (gnosis-exam-batch-size 2)
           started)
      (dolist (id '(91 92 93 94)) (gnosis-test-exam--thema id '("anat")))
      ;; Old history: every card seen and due before the exam is planned.
      (dolist (id '(91 92 93 94))
        (gnosis-test-exam--review
         id 1 (string-to-number
               (format-time-string "%Y%m%d" (time-subtract nil (* 30 86400))))))
      (gnosis-exam-save "Anatomy" (gnosis-scheduler--add-days today 18) '(("anat")))
      (gnosis-test-exam--in-view
        (cl-letf (((symbol-function 'gnosis-review-loop)
                   (lambda (ids &optional _mode _target validate)
                     (funcall validate)
                     (setq started ids)
                     ;; Real acceptance of the first card in this batch.
                     (gnosis-test-exam--review (car ids) 2 today))))
          (call-interactively #'gnosis-exam-cram)
          (should (equal '(91 92) started))
          (let ((due (gnosis-sqlite-select
                      gnosis-db "SELECT due_day FROM scheduler_state WHERE thema_id = 91")))
            (should (< today (caar due) (gnosis-scheduler--add-days today 18))))
          (call-interactively #'gnosis-exam-cram)
          (should (equal '(92 93) started))
          ;; The view was redrawn after each batch: both accepted cards wait.
          (should (string-match-p "2 due   0 at risk   2 scheduled before exam"
                                  (buffer-string))))))))

(ert-deftest gnosis-test-exam-add-refuses-database-replaced-during-prompt ()
  "Add captures its database before prompting and refuses a successor."
  (gnosis-test-with-db
    (let ((db-a gnosis-db))
      (gnosis-test-with-db
        (let ((db-b gnosis-db))
          (unwind-protect
              (progn
                (setq gnosis-db db-a)
                (cl-letf (((symbol-function 'read-string)
                           (lambda (&rest _) (setq gnosis-db db-b) "Redirected"))
                          ((symbol-function 'org-read-date) (lambda (&rest _) "2026-11-01"))
                          ((symbol-function 'gnosis-tags-filter-prompt) (lambda (&rest _) '(nil))))
                  (should-error (call-interactively #'gnosis-exam-add) :type 'user-error))
                (should-not (gnosis-exam-list db-a))
                (should-not (gnosis-exam-list db-b)))
            (setq gnosis-db db-b)))))))

(ert-deftest gnosis-test-exam-add-same-owner-and-cancel ()
  "Same-owner add writes and refreshes; a quit prompt writes nothing."
  (gnosis-test-exam--with-db
    (gnosis-test-exam--in-view
      (cl-letf (((symbol-function 'read-string) (lambda (&rest _) (signal 'quit nil))))
        (should (eq 'quit (condition-case nil
                              (progn (call-interactively #'gnosis-exam-add) nil)
                            (quit 'quit)))))
      (should-not (gnosis-exam-list))
      (cl-letf (((symbol-function 'read-string) (lambda (&rest _) "Final"))
                ((symbol-function 'org-read-date) (lambda (&rest _) "2026-11-01"))
                ((symbol-function 'gnosis-tags-filter-prompt) (lambda (&rest _) '(("anat")))))
        (call-interactively #'gnosis-exam-add))
      (should (equal '("Final") (mapcar (lambda (e) (plist-get e :name)) (gnosis-exam-list))))
      (should (string-match-p "Final" (buffer-string))))))

(ert-deftest gnosis-test-exam-add-refuses-plan-changed-during-prompt ()
  "A concurrent change to exam plans during prompts refuses the write."
  (gnosis-test-exam--with-db
    (gnosis-exam-save "Final" 20261101 '(nil) 0.97)
    (gnosis-test-exam--in-view
      (cl-letf (((symbol-function 'read-string)
                 (lambda (&rest _) (gnosis-exam-save "Final" 20261102 '(nil) 0.9) "Final"))
                ((symbol-function 'org-read-date) (lambda (&rest _) "2026-11-05"))
                ((symbol-function 'gnosis-tags-filter-prompt) (lambda (&rest _) '(nil))))
        (should-error (call-interactively #'gnosis-exam-add) :type 'user-error))
      (should (equal '(20261102 0.9)
                     (let ((e (car (gnosis-exam-list))))
                       (list (plist-get e :exam-day) (plist-get e :target))))))))

(ert-deftest gnosis-test-exam-target-keeps-prompted-exam-and-owner ()
  "Target writes the exam captured before its prompt, in its database only."
  (gnosis-test-exam--with-db
    (gnosis-exam-save "Alpha" 20261101 '(nil) 0.95)
    (gnosis-exam-save "Beta" 20261201 '(nil) 0.95)
    (gnosis-test-exam--in-view
      (gnosis-test-exam--goto "Alpha")
      ;; Point moves to another exam during the prompt.
      (cl-letf (((symbol-function 'read-number)
                 (lambda (&rest _) (gnosis-test-exam--goto "Beta") 0.97)))
        (call-interactively #'gnosis-exam-set-target))
      (should (equal '(("Alpha" 0.97) ("Beta" 0.95))
                     (mapcar (lambda (e) (list (plist-get e :name) (plist-get e :target)))
                             (gnosis-exam-list))))
      ;; Database replaced during the prompt.
      (gnosis-test-exam--goto "Alpha")
      (let ((db-a gnosis-db))
        (gnosis-test-with-db
          (let ((db-b gnosis-db))
            (gnosis-exam-save "Alpha" 20261101 '(nil) 0.95)
            (unwind-protect
                (progn
                  (setq gnosis-db db-a)
                  (cl-letf (((symbol-function 'read-number)
                             (lambda (&rest _) (setq gnosis-db db-b) 0.99)))
                    (should-error (call-interactively #'gnosis-exam-set-target)
                                  :type 'user-error))
                  (should (equal 0.95 (plist-get (car (gnosis-exam-list db-b)) :target)))
                  (should (equal 0.97 (plist-get (car (gnosis-exam-list db-a)) :target))))
              (setq gnosis-db db-b))))
        (setq gnosis-db db-a)))))

(defun gnosis-test-exam--associate (file)
  "Associate the current buffer with FILE inside `gnosis-dir'."
  (set-visited-file-name (expand-file-name file gnosis-dir) t))

(ert-deftest gnosis-test-exam-file-association-retires-view ()
  "A file-associated view keeps no write authority, even after detaching."
  (gnosis-test-exam--with-db
    (gnosis-exam-save "A" 20261101 '(nil))
    (with-temp-buffer
      (gnosis-exam-mode)
      (gnosis-exam-refresh)
      (gnosis-test-exam--associate "successor.txt")
      (let ((inhibit-read-only t)) (insert "successor text"))
      (let ((text (buffer-string)))
        (goto-char (point-min))
        (cl-letf (((symbol-function 'yes-or-no-p) (lambda (&rest _) t))
                  ((symbol-function 'read-number) (lambda (&rest _) 0.97))
                  ((symbol-function 'gnosis-review-loop)
                   (lambda (&rest _) (error "Batch started"))))
          (dolist (command '(gnosis-exam-delete-at-point gnosis-exam-set-target
                             gnosis-exam-cram gnosis-exam-refresh))
            (should-error (call-interactively command) :type 'user-error))
          ;; Detaching does not revive the former view.
          (set-visited-file-name nil)
          (dolist (command '(gnosis-exam-delete-at-point gnosis-exam-cram
                             gnosis-exam-refresh))
            (should-error (call-interactively command) :type 'user-error)))
        (should (equal text (buffer-string)))
        (should (equal '("A") (mapcar (lambda (e) (plist-get e :name)) (gnosis-exam-list))))
        (set-buffer-modified-p nil)))))

(ert-deftest gnosis-test-exam-delete-refuses-repurpose-during-confirmation ()
  "Repurposing the view during confirmation refuses deletion."
  (gnosis-test-exam--with-db
    (gnosis-exam-save "A" 20261101 '(nil))
    (with-temp-buffer
      (gnosis-exam-mode)
      (gnosis-exam-refresh)
      (goto-char (point-min))
      (cl-letf (((symbol-function 'yes-or-no-p)
                 (lambda (&rest _) (gnosis-test-exam--associate "successor.txt") t)))
        (should-error (call-interactively #'gnosis-exam-delete-at-point) :type 'user-error))
      (should (gnosis-exam-list))
      (set-buffer-modified-p nil)))
  ;; Positive control: same owner confirms and deletes.
  (gnosis-test-exam--with-db
    (gnosis-exam-save "A" 20261101 '(nil))
    (gnosis-test-exam--in-view
      (cl-letf (((symbol-function 'yes-or-no-p) (lambda (&rest _) t)))
        (call-interactively #'gnosis-exam-delete-at-point))
      (should-not (gnosis-exam-list))
      (should (string-match-p "No exams" (buffer-string))))))

(ert-deftest gnosis-test-exam-batch-return-preserves-repurposed-buffer ()
  "A view repurposed during a batch refuses later checks and is not redrawn."
  (gnosis-test-exam--with-db
    (gnosis-test-exam--thema 81 '("anat"))
    (gnosis-exam-save "A" (gnosis-scheduler--add-days (gnosis--today-int) 20) '(nil))
    (with-temp-buffer
      (gnosis-exam-mode)
      (gnosis-exam-refresh)
      (goto-char (point-min))
      (let ((view (current-buffer))
            validator)
        (cl-letf (((symbol-function 'gnosis-review-loop)
                   (lambda (_ids &optional _mode _target validate)
                     (funcall validate)
                     (setq validator validate)
                     (with-current-buffer view
                       (gnosis-test-exam--associate "successor.txt")
                       (let ((inhibit-read-only t))
                         (erase-buffer)
                         (insert "successor text"))))))
          (call-interactively #'gnosis-exam-review-unseen))
        (should (equal "successor text" (buffer-string)))
        (should-error (funcall validator) :type 'user-error)
        (set-buffer-modified-p nil)))))

(defun gnosis-test-exam--plans ()
  "Return every exam plan as (ID NAME DAY TARGET TAGS)."
  (mapcar (lambda (e) (list (plist-get e :id) (plist-get e :name) (plist-get e :exam-day)
                            (plist-get e :target) (plist-get e :tags)))
          (gnosis-exam-list)))

(ert-deftest gnosis-test-exam-delete-refuses-reused-id-of-another-exam ()
  "A retained Alpha row never deletes Beta created later under the same ID."
  (gnosis-test-exam--with-db
    (gnosis-exam-save "Alpha" 20261101 '(("anat")))
    (gnosis-test-exam--in-view
      (gnosis-test-exam--goto "Alpha")
      ;; Public plan API: delete Alpha, create Beta; the ID is reused.
      (gnosis-exam-delete (plist-get (car (gnosis-exam-list)) :id))
      (gnosis-exam-save "Beta" 20261201 '(("phys")))
      (should (equal 1 (plist-get (car (gnosis-exam-list)) :id)))
      (let ((plans (gnosis-test-exam--plans))
            asked)
        (cl-letf (((symbol-function 'yes-or-no-p)
                   (lambda (prompt) (setq asked prompt) t)))
          (should-error (call-interactively #'gnosis-exam-delete-at-point)
                        :type 'user-error))
        (should-not asked)
        (should (equal plans (gnosis-test-exam--plans)))
        (should (string-match-p "Alpha" (buffer-string)))
        ;; Same-row control after an explicit refresh.
        (gnosis-exam-refresh)
        (gnosis-test-exam--goto "Beta")
        (cl-letf (((symbol-function 'yes-or-no-p)
                   (lambda (prompt) (setq asked prompt) t)))
          (call-interactively #'gnosis-exam-delete-at-point))
        (should (equal "Delete exam Beta? " asked))
        (should-not (gnosis-exam-list))))))

(ert-deftest gnosis-test-exam-stale-row-refuses-target-add-and-batch ()
  "Commands on a retained row whose plan changed refuse instead of restoring it."
  (gnosis-test-exam--with-db
    (gnosis-test-exam--thema 71 '("anat"))
    (gnosis-test-exam--thema 72 '("phys"))
    (let ((day (gnosis-scheduler--add-days (gnosis--today-int) 20))
          (later (gnosis-scheduler--add-days (gnosis--today-int) 50)))
      (gnosis-exam-save "Alpha" day '(("anat")) 0.95)
      (gnosis-test-exam--in-view
        (dolist (change (list (lambda () (gnosis-exam-save "Alpha" later '(("phys"))))
                              (lambda () (gnosis-exam-save "Alpha" later '(("anat"))))
                              (lambda () (gnosis-exam-save "Alpha" day '(("phys"))))
                              (lambda () (gnosis-exam-save "Alpha" day '(("anat")) 0.9))))
          (gnosis-exam-save "Alpha" day '(("anat")) 0.95)
          (gnosis-exam-refresh)
          (gnosis-test-exam--goto "Alpha")
          (funcall change)
          (let ((plans (gnosis-test-exam--plans))
                started prompted)
            (cl-letf (((symbol-function 'read-number)
                       (lambda (&rest _) (setq prompted t) 0.97))
                      ((symbol-function 'read-string)
                       (lambda (&rest _) (setq prompted t) "Alpha"))
                      ((symbol-function 'org-read-date)
                       (lambda (&rest _) (setq prompted t) "2026-11-01"))
                      ((symbol-function 'gnosis-tags-filter-prompt)
                       (lambda (&rest _) (setq prompted t) '(("anat"))))
                      ((symbol-function 'gnosis-review-loop)
                       (lambda (ids &rest _) (setq started ids))))
              (dolist (command '(gnosis-exam-set-target gnosis-exam-add
                                 gnosis-exam-cram gnosis-exam-review-unseen))
                (should-error (call-interactively command) :type 'user-error)))
            (should-not prompted)
            (should-not started)
            (should (equal plans (gnosis-test-exam--plans)))))
        ;; Same-row control: after refresh, target writes only the target.
        (gnosis-exam-save "Alpha" later '(("phys")) 0.95)
        (gnosis-exam-refresh)
        (gnosis-test-exam--goto "Alpha")
        (cl-letf (((symbol-function 'read-number) (lambda (&rest _) 0.97)))
          (call-interactively #'gnosis-exam-set-target))
        (should (equal `((1 "Alpha" ,later 0.97 (("phys")))) (gnosis-test-exam--plans)))
        (let (started)
          (cl-letf (((symbol-function 'gnosis-review-loop)
                     (lambda (ids &rest _) (setq started ids))))
            (gnosis-test-exam--goto "Alpha")
            (call-interactively #'gnosis-exam-review-unseen))
          (should (equal '(72) started)))))))

(ert-deftest gnosis-test-exam-plan-writers-compare-and-set-full-row ()
  "Target and delete writers change exactly the captured row, or nothing."
  (gnosis-test-exam--with-db
    (gnosis-exam-save "Alpha" 20261101 '(("anat")) 0.95)
    (let ((captured (car (gnosis-exam-list))))
      (gnosis-exam-save "Alpha" 20261201 '(("phys")))
      (should-error (gnosis-exam--write-target captured 0.97 gnosis-db) :type 'user-error)
      (should-error (gnosis-exam--write-delete captured gnosis-db) :type 'user-error)
      (should (equal '((1 "Alpha" 20261201 0.95 (("phys")))) (gnosis-test-exam--plans)))
      (let ((current (car (gnosis-exam-list))))
        (gnosis-exam--write-target current 0.97 gnosis-db)
        (should (equal '((1 "Alpha" 20261201 0.97 (("phys")))) (gnosis-test-exam--plans)))
        (should-error (gnosis-exam--write-delete current gnosis-db) :type 'user-error)
        (gnosis-exam--write-delete (car (gnosis-exam-list)) gnosis-db)
        (should-not (gnosis-exam-list))))))

(defun gnosis-test-exam--batch-successor (successor)
  "Start an unseen batch whose loop runs SUCCESSOR in the view.
Return (VALIDATOR TEXT POINT) as left by SUCCESSOR."
  (let ((view (current-buffer))
        validator text pos)
    (cl-letf (((symbol-function 'gnosis-review-loop)
               (lambda (_ids &optional _mode _target validate)
                 (funcall validate)
                 (setq validator validate)
                 (with-current-buffer view
                   (funcall successor)
                   (let ((inhibit-read-only t))
                     (goto-char (point-max))
                     (insert "successor text"))
                   (goto-char (point-min))
                   (forward-line 1)
                   (setq text (buffer-string) pos (point))))))
      (call-interactively #'gnosis-exam-review-unseen))
    (list validator text pos)))

(ert-deftest gnosis-test-exam-batch-return-spares-successor-occurrence ()
  "Returning from a batch never redraws a successor view occurrence."
  (gnosis-test-exam--with-db
    (gnosis-test-exam--thema 81 '("anat"))
    (gnosis-exam-save "A" (gnosis-scheduler--add-days (gnosis--today-int) 20) '(nil))
    ;; Mode round trip and explicit refresh during the batch: new token.
    (with-temp-buffer
      (gnosis-exam-mode)
      (gnosis-exam-refresh)
      (goto-char (point-min))
      (pcase-let ((`(,validator ,text ,pos)
                   (gnosis-test-exam--batch-successor
                    (lambda () (text-mode) (gnosis-exam-mode) (gnosis-exam-refresh)))))
        (should (equal text (buffer-string)))
        (should (= pos (point)))
        (should-error (funcall validator) :type 'user-error))
      (set-buffer-modified-p nil))
    ;; Database replaced and the view explicitly refreshed during the batch.
    (let ((db-a gnosis-db))
      (gnosis-test-with-db
        (let ((db-b gnosis-db))
          (unwind-protect
              (progn
                (setq gnosis-db db-a)
                (with-temp-buffer
                  (gnosis-exam-mode)
                  (gnosis-exam-refresh)
                  (goto-char (point-min))
                  (pcase-let ((`(,validator ,text ,pos)
                               (gnosis-test-exam--batch-successor
                                (lambda () (setq gnosis-db db-b) (gnosis-exam-refresh)))))
                    (should (equal text (buffer-string)))
                    (should (= pos (point)))
                    (should-error (funcall validator) :type 'user-error))
                  (set-buffer-modified-p nil)))
            (setq gnosis-db db-b)))))
    ;; Control: the same occurrence is redrawn after its batch.
    (with-temp-buffer
      (gnosis-exam-mode)
      (gnosis-exam-refresh)
      (goto-char (point-min))
      (gnosis-test-exam--batch-successor #'ignore)
      (should-not (string-match-p "successor text" (buffer-string)))
      (set-buffer-modified-p nil))))

(ert-deftest gnosis-test-exam-footer-discloses-unknown-prediction ()
  "The at-risk explanation names unknown predictions."
  (gnosis-test-exam--with-db
    (gnosis-test-exam--in-view
      (should (string-match-p "below target or unknown" (buffer-string))))))

(provide 'gnosis-test-exam)
;;; gnosis-test-exam.el ends here
