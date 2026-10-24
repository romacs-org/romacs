;;; gnosis-test-leech.el --- Leech detection and tutor rework -*- lexical-binding: t; -*-

;; Copyright (C) 2026 Free Software Foundation, Inc.

;;; Commentary:
;; The pure leech rule, the agent query with recent wrong answers, the
;; feedback note and `w' request through the native review loop, request
;; stubs in the inbox, tutor proposals (split, replace, annotate) and the
;; batch report/propose entry points.

;;; Code:

(require 'ert)
(require 'json)
(require 'gnosis-review-test-support)
(require 'gnosis-inbox)
(require 'gnosis-leech)

(defun gnosis-test-leech--day (days-ago)
  "Return the YYYYMMDD day DAYS-AGO days before today."
  (let ((time (time-subtract nil (days-to-time days-ago))))
    (string-to-number (format-time-string "%Y%m%d" time))))

(defun gnosis-test-leech--at-us (days-ago &optional offset)
  "Return microseconds at local noon DAYS-AGO days before today, plus OFFSET."
  (+ (car (time-convert (gnosis-scheduler--day-time (gnosis-test-leech--day days-ago))
                        1000000))
     (or offset 0)))

(defun gnosis-test-leech--fail (id days-ago &optional success)
  "Accept a scheduled failure (or SUCCESS) of ID DAYS-AGO days ago.
Return the event ID."
  (let ((event (gnosis-scheduler-event-id)))
    (gnosis-scheduler-accept-review event id (if success 'success 'failure)
                                    (gnosis-test-leech--at-us days-ago)
                                    (gnosis-test-leech--day days-ago))
    event))

(defun gnosis-test-leech--card (id keimenon answer failures &optional parathema)
  "Add basic card ID with KEIMENON and ANSWER and FAILURES past lapses.
PARATHEMA is its explanation."
  (gnosis-test--add-basic-thema keimenon answer '("anatomy") parathema id)
  (cl-loop for i from 0 below failures
           collect (gnosis-test-leech--fail id (- 40 (* i 6)))))

(defun gnosis-test-leech--practice (id text days-ago &optional success)
  "Record a practice answer TEXT for ID DAYS-AGO days ago; return event ID."
  (let ((event (gnosis-scheduler-event-id))
        (answer (gnosis-get 'answer 'themata `(= id ,id))))
    (gnosis-study-accept-practice
     (list :mode 'practice :event-id event :thema-id id
           :session-id (gnosis-scheduler-event-id) :attempt 1
           :reviewed-at-us (gnosis-test-leech--at-us days-ago 7)
           :outcome (if success 'success 'failure)
           :encounter (list :version 1 :kind "basic" :prompt "Q"
                            :hypothesis [] :expected-answers (vconcat answer)
                            :accepted-aliases [] :hints-available [] :hints-shown []
                            :original-outcome (if success "success" "failure")
                            :response (list :kind "text" :text text)
                            :coaching nil :parathema nil :review-image nil
                            :match-rule (list :kind "exact"))))
    event))

(defmacro gnosis-test-leech (&rest body)
  "Run BODY with isolated data and owned buffers."
  (declare (indent 0) (debug t))
  `(gnosis-test-with-db
     (let ((buffers (buffer-list))
           (gnosis-review-buffer-name "*Gnosis Leech Test*")
           (gnosis-review--running nil)
           (gnosis-new-themata-limit nil)
           (gnosis-center-content nil)
           (gnosis-monkeytype-enable nil)
           (register-alist nil))
       (save-window-excursion
         (unwind-protect (progn ,@body)
           (dolist (buffer (seq-difference (buffer-list) buffers))
             (when (buffer-live-p buffer) (kill-buffer buffer))))))))

(defun gnosis-test-leech--entries ()
  "Return parsed inbox entries."
  (gnosis-inbox-parse (gnosis-inbox--read (gnosis-inbox-file))))

;;; The rule

(ert-deftest gnosis-test-leech-rule ()
  "Lapses, hard lapses and stuck stability each make a leech."
  (should-not (gnosis-leech-p nil))
  (should-not (gnosis-leech-p '(:lapses 3 :difficulty 7.9 :reps 5 :stability 4.0)))
  (should (equal '(lapses) (gnosis-leech-reasons '(:lapses 4 :difficulty 5.0 :reps 6 :stability 9.0))))
  (should (equal '(difficulty) (gnosis-leech-reasons '(:lapses 3 :difficulty 8.0 :reps 5 :stability 9.0))))
  (should (equal '(stability) (gnosis-leech-reasons '(:lapses 2 :difficulty 6.0 :reps 8 :stability 1.5))))
  (should-not (gnosis-leech-p '(:lapses 2 :difficulty 6.0 :reps 8 :stability 30.0)))
  (should (equal '(lapses difficulty stability)
                 (gnosis-leech-reasons '(:lapses 6 :difficulty 9.1 :reps 9 :stability 0.5))))
  ;; New cards have no memory state.
  (should-not (gnosis-leech-p '(:lapses 0 :reps 0)))
  (let ((gnosis-leech-lapses 2))
    (should (gnosis-leech-p '(:lapses 2 :difficulty 3.0 :reps 3 :stability 5.0)))))

;;; Agent query

(ert-deftest gnosis-test-leech-query ()
  "Leeches come with content, evidence, revision and recent wrong answers."
  (gnosis-test-leech
    (gnosis-test-leech--card 101 "Which nerve supplies the deltoid?" "axillary" 4
                             "C5-C6 via the posterior cord.")
    (gnosis-test-leech--card 102 "Which artery runs with the radial nerve in the spiral groove?"
                             "profunda brachii" 1)
    (gnosis-test-leech--card 103 "Suspended leech" "x" 5)
    (gnosis-toggle-suspend-themata '(103) 1 t)
    (gnosis-test-leech--practice 101 "musculocutaneous" 3)
    (gnosis-test-leech--practice 101 "radial" 2)
    (gnosis-test-leech--practice 101 "axillary" 2 t)
    (let ((voided (gnosis-test-leech--practice 101 "ulnar" 1)))
      (gnosis-study-void-practice (gnosis-scheduler-event-id) voided))
    (let ((leeches (gnosis-leeches)))
      (should (equal '(101) (mapcar (lambda (l) (plist-get l :id)) leeches)))
      (let ((leech (car leeches)))
        (should (equal "basic" (plist-get leech :type)))
        (should (equal "Which nerve supplies the deltoid?" (plist-get leech :keimenon)))
        (should (equal '("axillary") (plist-get leech :answer)))
        (should (equal "C5-C6 via the posterior cord." (plist-get leech :parathema)))
        (should (equal '("anatomy") (plist-get leech :tags)))
        (should (= 4 (plist-get leech :lapses)))
        (should (memq 'lapses (plist-get leech :reasons)))
        (should (equal (gnosis-inbox--revision 101) (plist-get leech :revision)))
        ;; Newest first; correct and voided answers excluded.
        (should (equal '("radial" "musculocutaneous")
                       (mapcar (lambda (w) (plist-get w :response))
                               (plist-get leech :recent-wrong))))
        (should (equal '(practice practice)
                       (mapcar (lambda (w) (plist-get w :source)) (plist-get leech :recent-wrong))))
        (should-not (plist-get leech :request))
        (should (stringp (plist-get leech :draft)))))
    (should (equal '(103 101) (mapcar (lambda (l) (plist-get l :id))
                                      (gnosis-leeches :include-suspended t))))
    (should (equal '(102) (mapcar (lambda (l) (plist-get l :id))
                                  (gnosis-leeches :ids '(102) :all t))))))

(ert-deftest gnosis-test-leech-scheduled-encounters-optional ()
  "Scheduled wrong answers are read only when review_encounters exists."
  (gnosis-test-leech
    (let ((events (gnosis-test-leech--card 101 "Deltoid nerve?" "axillary" 4)))
      (should (gnosis-leeches))
      (should-not (plist-get (car (gnosis-leeches)) :recent-wrong))
      (gnosis-sqlite-execute
       gnosis-db "CREATE TABLE review_encounters (event_id TEXT PRIMARY KEY NOT NULL,
                                                  data TEXT NOT NULL)")
      (gnosis-sqlite-execute
       gnosis-db "INSERT INTO review_encounters VALUES (?, ?)"
       (list (car (last events))
             (list :version 1 :kind "basic" :hypothesis [] :expected-answers ["axillary"]
                   :response (list :kind "text" :text "long thoracic"))))
      (gnosis-test-leech--practice 101 "radial" 1)
      (should (equal '((practice "radial") (scheduled "long thoracic"))
                     (mapcar (lambda (w) (list (plist-get w :source) (plist-get w :response)))
                             (plist-get (car (gnosis-leeches)) :recent-wrong)))))))

(ert-deftest gnosis-test-leech-recent-limit ()
  "At most `gnosis-leech-recent-limit' wrong answers are returned."
  (gnosis-test-leech
    (gnosis-test-leech--card 101 "Deltoid nerve?" "axillary" 4)
    (dotimes (i 5) (gnosis-test-leech--practice 101 (format "wrong %d" i) (- 10 i)))
    (let ((gnosis-leech-recent-limit 3))
      (should (equal '("wrong 4" "wrong 3" "wrong 2")
                     (mapcar (lambda (w) (plist-get w :response))
                             (plist-get (car (gnosis-leeches)) :recent-wrong)))))))

;;; Requests

(ert-deftest gnosis-test-leech-draft-error-is-per-card ()
  "A card that cannot be written as Org keeps the query and its evidence."
  (gnosis-test-leech
    (gnosis-test-leech--card 101 "Deltoid nerve?" "axillary" 4)
    (gnosis-sqlite-execute gnosis-db "INSERT INTO thema_tag VALUES (101, 'upper-limb')")
    (let ((leech (car (gnosis-leeches))))
      (should (= 101 (plist-get leech :id)))
      (should-not (plist-get leech :draft))
      (should (string-match-p "upper-limb" (plist-get leech :draft-error))))))

(ert-deftest gnosis-test-leech-request-stub ()
  "A request is a content-free inbox stub, never a pending draft."
  (gnosis-test-leech
    (gnosis-test-leech--card 101 "Deltoid nerve?" "axillary" 4)
    (let ((rows (gnosis-test-content--evidence)))
      (should (eq 'added (plist-get (gnosis-leech-request 101 "4 lapses") :status)))
      (should (eq 'exists (plist-get (gnosis-leech-request 101 "4 lapses") :status)))
      (should (equal rows (gnosis-test-content--evidence))))
    (let ((entries (gnosis-test-leech--entries)))
      (should (= 1 (length entries)))
      (should (eq 'requested (plist-get (car entries) :state)))
      (should (= 101 (plist-get (car entries) :target)))
      (should-not (string-match-p "Deltoid\\|axillary" (gnosis-inbox--read (gnosis-inbox-file)))))
    (should (= 0 (gnosis-inbox-count)))
    (should (equal '(101) (mapcar (lambda (r) (plist-get r :target)) (gnosis-leech-requests))))
    (should (eq 'requested (plist-get (plist-get (car (gnosis-leeches)) :request) :state)))
    ;; The inbox view shows no draft for a request.
    (save-window-excursion
      (gnosis-inbox)
      (unwind-protect (should (string-match-p "Inbox empty" (buffer-string)))
        (kill-buffer "*Gnosis Inbox*")))
    (should-error (gnosis-leech-request 999))))

(defun gnosis-test-leech--split-drafts (&optional revision)
  "Return two split drafts for the deltoid card #101.
They record REVISION, defaulting to the card's current revision."
  (let ((revision (or revision (gnosis-inbox--revision 101))))
    (list (list :type "basic" :keimenon "Which cord gives rise to the axillary nerve?"
                :answer '("posterior cord") :tags '("anatomy") :target-revision revision)
          (list :type "basic" :keimenon "Which shoulder muscle does the axillary nerve supply besides teres minor?"
                :answer '("deltoid") :tags '("anatomy") :target-revision revision))))

(ert-deftest gnosis-test-leech-propose-refuses-stale-revision ()
  "A replace or split written against an older revision is refused.
The inbox, its requests and the target are unchanged; the recorded
revision is never rewritten to the current one."
  (gnosis-test-leech
    (gnosis-test-leech--card 101 "Deltoid nerve?" "axillary" 4)
    (gnosis-leech-request 101)
    (let* ((seen (gnosis-inbox--revision 101))
           (split (gnosis-test-leech--split-drafts seen))
           (replace (list (list :type "basic" :keimenon "Which nerve supplies deltoid?"
                                :answer '("axillary") :tags '("anatomy")
                                :target-revision seen))))
      (gnosis-update 'themata '(= keimenon "Deltoid and teres minor nerve?") '(= id 101))
      (let ((inbox (gnosis-inbox--read (gnosis-inbox-file)))
            (rows (gnosis-test-content--evidence)))
        (should-error (gnosis-leech-propose 101 replace :kind "rework-replace"
                                            :agent "hermes/tutor" :reason "stale")
                      :type 'user-error)
        (should-error (gnosis-leech-propose 101 split :kind "rework-split"
                                            :agent "hermes/tutor" :reason "stale")
                      :type 'user-error)
        (should-error (gnosis-inbox-propose-rework 101 replace :agent "a" :reason "r")
                      :type 'user-error)
        ;; A draft without its revision is refused at the leech boundary.
        (should-error (gnosis-leech-propose
                       101 (list (list :type "basic" :keimenon "Q?" :answer '("a")))
                       :kind "rework-replace")
                      :type 'user-error)
        (should (equal inbox (gnosis-inbox--read (gnosis-inbox-file))))
        (should (equal rows (gnosis-test-content--evidence)))
        (should (equal '(101) (mapcar (lambda (r) (plist-get r :target))
                                      (gnosis-leech-requests))))
        (should-not (gnosis-suspended-p 101))))
    ;; Same-revision control queues and approves normally.
    (let ((current (gnosis-inbox--revision 101)))
      (should (equal '(added)
                     (mapcar (lambda (r) (plist-get r :status))
                             (gnosis-leech-propose
                              101 (list (list :type "basic" :keimenon "Which nerve supplies deltoid?"
                                              :answer '("axillary") :tags '("anatomy")
                                              :target-revision current))
                              :kind "rework-replace" :agent "hermes/tutor" :reason "fresh"))))
      (let ((entry (seq-find (lambda (e) (eq (plist-get e :state) 'pending))
                             (gnosis-test-leech--entries))))
        (should (equal current (plist-get entry :target-revision)))
        (gnosis-inbox-apply entry))
      (should (gnosis-suspended-p 101))
      (should-not (gnosis-leech-requests)))))

(defun gnosis-test-leech--annotate-collision ()
  "Add target #101 and an unrelated #102 already holding its annotation.
Queue the annotation adding \"More.\" to #101."
  (gnosis-test--add-basic-thema "Question?" "answer" '("test") "Original." 101)
  (gnosis-test--add-basic-thema "Question?" "answer" '("test") "Original.\n\nMore." 102)
  (gnosis-leech-propose 101 (list (gnosis-leech-annotation-draft 101 "More."))
                        :kind "rework-annotate" :agent "hermes/tutor" :reason "more"))

(defun gnosis-test-leech--schedules ()
  "Return scheduler rows and review events, for unchanged-evidence checks."
  (list (gnosis-select '* 'scheduler-state nil)
        (sqlite-select gnosis-db "SELECT * FROM review_events")))

(ert-deftest gnosis-test-leech-annotate-accept-ignores-other-card ()
  "Native accept annotates the target even when another card matches."
  (gnosis-test-leech
    (gnosis-test-leech--annotate-collision)
    (let ((evidence (gnosis-test-leech--schedules)))
      (gnosis-inbox)
      (gnosis-inbox-accept)
      (should (equal "Original.\n\nMore." (gnosis-get 'parathema 'extras '(= id 101))))
      (should (equal "Original.\n\nMore." (gnosis-get 'parathema 'extras '(= id 102))))
      (should (equal evidence (gnosis-test-leech--schedules)))
      (should (equal '((applied 101 (101)))
                     (mapcar (lambda (e) (list (plist-get e :state) (plist-get e :target)
                                               (plist-get e :themata)))
                             (gnosis-test-leech--entries)))))))

(ert-deftest gnosis-test-leech-annotate-accept-recovers-own-commit ()
  "If only the target's edit committed, accepting again just closes it."
  (gnosis-test-leech
    (gnosis-test-leech--annotate-collision)
    (let ((entry (car (gnosis-test-leech--entries))))
      ;; The database commit succeeded but inbox settlement did not.
      (gnosis-inbox--commit entry (list (gnosis-inbox--effective-thema entry))))
    (let ((rows (gnosis-test-content--evidence)))
      (gnosis-inbox)
      (gnosis-inbox-accept)
      (should (equal rows (gnosis-test-content--evidence)))
      (should (equal '((duplicate 101 (101)))
                     (mapcar (lambda (e) (list (plist-get e :state) (plist-get e :target)
                                               (plist-get e :themata)))
                             (gnosis-test-leech--entries)))))))

(ert-deftest gnosis-test-leech-annotate-accept-refuses-changed-target ()
  "A target edited since the annotation is refused, not closed elsewhere."
  (gnosis-test-leech
    (gnosis-test-leech--annotate-collision)
    (gnosis-update 'extras '(= parathema "Edited.") '(= id 101))
    (let ((rows (gnosis-test-content--evidence)))
      (gnosis-inbox)
      (should-error (gnosis-inbox-accept) :type 'user-error)
      (should (equal rows (gnosis-test-content--evidence)))
      (should (equal '(pending) (mapcar (lambda (e) (plist-get e :state))
                                        (gnosis-test-leech--entries)))))))

(ert-deftest gnosis-test-leech-propose-distinct-repairs ()
  "Distinct additions are distinct proposals; history does not close requests."
  (gnosis-test-leech
    (gnosis-test--add-basic-thema "Question?" "answer" '("test") "Original." 101)
    (let* ((a (gnosis-leech-annotation-draft 101 "Mnemonic A."))
           (b (gnosis-leech-annotation-draft 101 "Prerequisite B."))
           (propose (lambda (draft)
                      (car (gnosis-leech-propose 101 (list draft) :kind "rework-annotate"
                                                 :agent "hermes/tutor" :reason "x"))))
           (first (funcall propose a))
           (second (funcall propose b)))
      (should (eq 'added (plist-get first :status)))
      (should (eq 'added (plist-get second :status)))
      (should-not (equal (plist-get first :key) (plist-get second :key)))
      ;; An exact rerun is a duplicate.
      (should (eq 'duplicate (plist-get (funcall propose a) :status)))
      (should (= 2 (gnosis-inbox-count)))
      ;; Reject both, ask again: repeating a rejected draft keeps the request.
      (gnosis-inbox)
      (gnosis-inbox-reject "not useful")
      (gnosis-inbox-reject "not useful")
      (should (= 0 (gnosis-inbox-count)))
      (gnosis-leech-request 101 "try another")
      (should (eq 'duplicate (plist-get (funcall propose b) :status)))
      (should (equal '(101) (mapcar (lambda (r) (plist-get r :target)) (gnosis-leech-requests))))
      ;; A new repair closes it.
      (should (eq 'added (plist-get (funcall propose
                                             (gnosis-leech-annotation-draft 101 "Contrast C."))
                                    :status)))
      (should-not (gnosis-leech-requests)))))

(ert-deftest gnosis-test-leech-propose-closes-request ()
  "Proposing a rework closes the target's open request."
  (gnosis-test-leech
    (gnosis-test-leech--card 101 "Deltoid nerve?" "axillary" 4)
    (gnosis-leech-request 101)
    (let ((results (gnosis-leech-propose 101 (gnosis-test-leech--split-drafts)
                                         :kind "rework-split" :agent "hermes/tutor"
                                         :reason "two facts in one card")))
      (should (equal '(added added) (mapcar (lambda (r) (plist-get r :status)) results))))
    (should-not (gnosis-leech-requests))
    (should (= 2 (gnosis-inbox-count)))
    (let ((leech (car (gnosis-leeches))))
      (should-not (plist-get leech :request))
      (should (= 2 (plist-get leech :proposals))))
    (should (cl-find 'proposed (gnosis-test-leech--entries)
                     :key (lambda (e) (plist-get e :state))))
    ;; A split accepted in the inbox suspends the original.
    (let ((entry (car (seq-filter (lambda (e) (eq (plist-get e :state) 'pending))
                                  (gnosis-test-leech--entries)))))
      (gnosis-inbox-apply entry))
    (should (gnosis-suspended-p 101))))

(ert-deftest gnosis-test-leech-rejection-memory ()
  "Rejected proposals are reported with their reason."
  (gnosis-test-leech
    (gnosis-test-leech--card 101 "Deltoid nerve?" "axillary" 4)
    (gnosis-leech-propose 101 (list (car (gnosis-test-leech--split-drafts)))
                          :kind "rework-split" :agent "hermes/tutor" :reason "split")
    (save-window-excursion
      (gnosis-inbox)
      (unwind-protect (gnosis-inbox-reject "trivial")
        (kill-buffer "*Gnosis Inbox*")))
    (should (equal '((:kind "rework-split" :reason "trivial"))
                   (mapcar (lambda (r) (list :kind (plist-get r :kind) :reason (plist-get r :reason)))
                           (plist-get (car (gnosis-leeches)) :rejected))))))

;;; Annotate proposals (mnemonic, prerequisite link)

(ert-deftest gnosis-test-leech-annotate-keeps-schedule ()
  "An annotate proposal appends to the explanation and keeps the card."
  (gnosis-test-leech
    (gnosis-test-leech--card 101 "Deltoid nerve?" "axillary" 4 "C5-C6.")
    (gnosis-test--add-basic-thema "Unrelated" "other" nil nil 102)
    (let ((state (gnosis-select '* 'scheduler-state '(= thema-id 101)))
          (events (sqlite-select gnosis-db "SELECT * FROM review_events")))
      (should (equal '(added)
                     (mapcar (lambda (r) (plist-get r :status))
                             (gnosis-leech-propose
                              101 (list (gnosis-leech-annotation-draft
                                         101 "Mnemonic: the axillary nerve wraps the surgical neck."))
                              :kind "rework-annotate" :agent "hermes/tutor" :reason "mnemonic"))))
      (let ((entry (car (gnosis-test-leech--entries))))
        (should (equal '(101) (gnosis-inbox-apply entry))))
      (should (equal "C5-C6.\n\nMnemonic: the axillary nerve wraps the surgical neck."
                     (gnosis-get 'parathema 'extras '(= id 101))))
      (should (equal "Deltoid nerve?" (gnosis-get 'keimenon 'themata '(= id 101))))
      (should (equal state (gnosis-select '* 'scheduler-state '(= thema-id 101))))
      (should (equal events (sqlite-select gnosis-db "SELECT * FROM review_events")))
      (should (= 2 (length (gnosis-select 'id 'themata nil t)))))))

(ert-deftest gnosis-test-leech-annotate-refuses-body-changes ()
  "An annotate draft may only append to the explanation."
  (gnosis-test-leech
    (gnosis-test-leech--card 101 "Deltoid nerve?" "axillary" 4 "C5-C6.")
    (let ((rows (gnosis-test-content--evidence))
          (bad (plist-put (gnosis-leech-annotation-draft 101 "More.") :keimenon "Changed?"))
          (shrunk (plist-put (gnosis-leech-annotation-draft 101 "More.") :parathema "Gone.")))
      (gnosis-leech-propose 101 (list bad shrunk) :kind "rework-annotate"
                            :agent "hermes/tutor" :reason "x")
      (dolist (entry (gnosis-test-leech--entries))
        (should-error (gnosis-inbox-apply entry) :type 'user-error))
      (should (equal rows (gnosis-test-content--evidence)))
      (should (equal "C5-C6." (gnosis-get 'parathema 'extras '(= id 101)))))))

(ert-deftest gnosis-test-leech-annotate-renders-addition ()
  "The inbox shows the target, its lapses and the added explanation."
  (gnosis-test-leech
    (gnosis-test-leech--card 101 "Deltoid nerve?" "axillary" 4 "C5-C6.")
    (gnosis-test-leech--practice 101 "radial" 1)
    (gnosis-leech-propose 101 (list (gnosis-leech-annotation-draft 101 "Mnemonic: wraps the neck."))
                          :kind "rework-annotate" :agent "hermes/tutor" :reason "mnemonic")
    (save-window-excursion
      (gnosis-inbox)
      (unwind-protect
          (let ((text (buffer-string)))
            (should (string-match-p "adds to #101" text))
            (should (string-match-p "schedule kept" text))
            (should (string-match-p "⚑ leech · 4 lapses in 4 reviews" text))
            (let ((flag (string-match "⚑ leech" text)))
              (should (memq 'gnosis-inbox-warning
                            (ensure-list (get-text-property flag 'face text))))
              (should-not (memq 'gnosis-inbox-warning
                                (ensure-list (get-text-property (+ flag 10) 'face text)))))
            (should (string-match-p "radial" text))
            (let ((pos (string-match "Mnemonic: wraps the neck" text)))
              (should pos)
              (should (memq 'gnosis-inbox-added
                            (ensure-list (get-text-property pos 'face text)))))
            (should-not (string-match-p "suspended on accept" text)))
        (kill-buffer "*Gnosis Inbox*")))))

;;; Feedback note and the `w' key

(ert-deftest gnosis-test-leech-note ()
  "The note appears for a wrong answer whose projected state is a leech."
  (gnosis-test-leech
    (gnosis-test-leech--card 101 "Deltoid nerve?" "axillary" 3)
    (let ((leech (list :preview '(:lapses-after 4 :reps-after 5 :difficulty 6.0 :stability 1.0)))
          (plain (list :preview '(:lapses-after 2 :reps-after 5 :difficulty 6.0 :stability 3.0))))
      (should (string-match-p "leech · 4 lapses · w ask tutor to rework"
                              (gnosis-leech-note 101 leech)))
      (should-not (gnosis-leech-note 101 plain))
      (should-not (gnosis-leech-note 101 (append '(:local-owner t) leech)))
      (gnosis-leech-request 101)
      (should (string-match-p "rework requested" (gnosis-leech-note 101 leech)))
      ;; Practice results read the stored state: three real failures make
      ;; FSRS difficulty high enough for the hard-lapses rule.
      (should (string-match-p "3 lapses · difficulty"
                              (gnosis-leech-note 101 '(:mode practice))))
      (gnosis-test-leech--card 102 "Deltoid nerve?" "axillary" 1)
      (should-not (gnosis-leech-note 102 '(:mode practice))))))

(ert-deftest gnosis-test-leech-feedback-request ()
  "A wrong due answer crossing the threshold shows the note; w requests."
  (gnosis-test-leech
    (gnosis-test-leech--card 101 "Which nerve supplies the deltoid?" "axillary" 3)
    (let* ((before nil) (after nil)
           (rows nil))
      ;; The failed card returns later in the batch and is then answered.
      (gnosis-test-drive
       '("radial" "axillary")
       (list (lambda () (setq before (buffer-string)
                              rows (gnosis-test-content--evidence)))
             "w"
             (lambda () (setq after (buffer-string))
               (should (equal rows (gnosis-test-content--evidence))))
             "n" "n")
       (lambda ()
         (cl-letf (((symbol-function 'gnosis-review--feedback-show) #'ignore))
           (gnosis-review-loop '(101) 'due))))
      (should (string-match-p "⚑ leech · 4 lapses · .*w ask tutor to rework" before))
      (should (string-match-p "⚑ leech · 4 lapses · .*rework requested" after))
      (should-not (string-match-p "w ask tutor" after))
      (should (equal '(101) (mapcar (lambda (r) (plist-get r :target)) (gnosis-leech-requests))))
      ;; The wrong answer was accepted as usual, then the retry.
      (should (equal '((1) (1) (1) (1) (3))
                     (sqlite-select gnosis-db "SELECT rating FROM review_events
                                                ORDER BY reviewed_at_us"))))))

(ert-deftest gnosis-test-leech-feedback-quiet-otherwise ()
  "Correct answers and non-leeches show no note, and w refuses."
  (gnosis-test-leech
    (gnosis-test-leech--card 101 "Deltoid nerve?" "axillary" 1)
    (let (text)
      (gnosis-test-drive
       '("radial" "axillary")
       (list (lambda () (setq text (buffer-string))
               (should-error (call-interactively #'gnosis-leech-request-rework)
                             :type 'user-error))
             "n"
             (lambda () (should-not (string-match-p "leech" (buffer-string))))
             "n")
       (lambda ()
         (cl-letf (((symbol-function 'gnosis-review--feedback-show) #'ignore))
           (gnosis-review-loop '(101) 'due))))
      (should-not (string-match-p "leech" text))
      (should-not (gnosis-leech-requests)))))

;;; Batch entry points

(defun gnosis-test-leech--batch (function dir &rest args)
  "Run batch FUNCTION with GNOSIS_DIR DIR and ARGS; return (EXIT . JSON)."
  (let* ((root (file-name-directory (directory-file-name
                                     (file-name-directory (locate-library "gnosis")))))
         (popup (file-name-directory (locate-library "keymap-popup")))
         (process-environment (cons (concat "GNOSIS_DIR=" (or dir "")) process-environment)))
    (with-temp-buffer
      (let ((exit (apply #'call-process (expand-file-name invocation-name invocation-directory)
                         nil (list t nil) nil
                         "-Q" "--batch" "-L" (expand-file-name "lisp" root) "-L" popup
                         "-l" "gnosis-leech" "-f" function args)))
        (cons exit (progn (goto-char (point-min)) (json-parse-buffer :object-type 'plist
                                                                    :array-type 'list)))))))

(ert-deftest gnosis-test-leech-batch-report-and-propose ()
  "The tutor reads requests read-only and proposes through the inbox."
  (gnosis-test-leech
    (gnosis-test-leech--card 101 "Deltoid nerve?" "axillary" 4 "C5-C6.")
    (gnosis-test-leech--practice 101 "radial" 1)
    (gnosis-leech-request 101 "4 lapses")
    (let* ((db-file (expand-file-name "gnosis.db" gnosis-dir))
           (version (caar (sqlite-select gnosis-db "PRAGMA user_version")))
           (rows (gnosis-test-content--evidence))
           (report (gnosis-test-leech--batch "gnosis-leech-batch-report" gnosis-dir)))
      (should (= 0 (car report)))
      (let* ((json (cdr report))
             (request (car (plist-get json :requests))))
        (should (stringp (plist-get json :instructions)))
        (should (= 101 (plist-get request :id)))
        (should (equal "Deltoid nerve?" (plist-get request :keimenon)))
        (should (equal '("radial") (mapcar (lambda (w) (plist-get w :response))
                                           (plist-get request :recent-wrong))))
        (should (equal (gnosis-inbox--revision 101) (plist-get request :revision)))
        ;; The agent edits the prefilled draft: append a mnemonic.
        (let ((drafts (expand-file-name "drafts.org" gnosis-dir)))
          (with-temp-file drafts
            (insert (replace-regexp-in-string
                     "C5-C6\\." "C5-C6.\n\nMnemonic: wraps the surgical neck."
                     (plist-get request :draft) t t))
            (goto-char (point-min))
            (re-search-forward "^:END:")
            (beginning-of-line)
            (insert ":GNOSIS_AGENT: hermes/tutor\n:GNOSIS_REASON: mnemonic\n"))
          (let ((propose (gnosis-test-leech--batch "gnosis-leech-batch-propose" gnosis-dir drafts)))
            (should (= 0 (car propose)))
            (should (equal '("added") (mapcar (lambda (r) (plist-get r :status))
                                              (plist-get (cdr propose) :results)))))
          ;; Rerunning proposes nothing new.
          (should (equal '("duplicate")
                         (mapcar (lambda (r) (plist-get r :status))
                                 (plist-get (cdr (gnosis-test-leech--batch
                                                  "gnosis-leech-batch-propose" gnosis-dir drafts))
                                            :results))))))
      (should (equal version (caar (sqlite-select gnosis-db "PRAGMA user_version"))))
      (should (equal rows (gnosis-test-content--evidence)))
      (should (file-regular-p db-file))
      (should-not (gnosis-leech-requests))
      (should (= 1 (gnosis-inbox-count))))))

(ert-deftest gnosis-test-leech-batch-refusals ()
  "Missing GNOSIS_DIR, a missing database or a stale revision refuse."
  (gnosis-test-leech
    (should (= 2 (car (gnosis-test-leech--batch "gnosis-leech-batch-report" nil))))
    (let ((empty (make-temp-file "gnosis-leech-empty-" t)))
      (unwind-protect
          (progn
            (should (= 2 (car (gnosis-test-leech--batch "gnosis-leech-batch-report" empty))))
            (should-not (file-exists-p (expand-file-name "gnosis.db" empty))))
        (delete-directory empty t)))
    (gnosis-test-leech--card 101 "Deltoid nerve?" "axillary" 4 "C5-C6.")
    (let ((drafts (expand-file-name "drafts.org" gnosis-dir))
          (draft (plist-get (car (gnosis-leeches)) :draft)))
      (with-temp-file drafts
        (insert (replace-regexp-in-string "C5-C6\\." "C5-C6. More." draft t t))
        (goto-char (point-min))
        (re-search-forward "^:END:")
        (beginning-of-line)
        (insert ":GNOSIS_AGENT: hermes/tutor\n:GNOSIS_REASON: x\n"))
      ;; The card changes after the report.
      (gnosis-update 'extras '(= parathema "Edited.") '(= id 101))
      (let ((propose (gnosis-test-leech--batch "gnosis-leech-batch-propose" gnosis-dir drafts)))
        (should (= 1 (car propose)))
        (should (equal '("invalid") (mapcar (lambda (r) (plist-get r :status))
                                            (plist-get (cdr propose) :results))))
        (should (= 0 (gnosis-inbox-count)))))))

(provide 'gnosis-test-leech)
;;; gnosis-test-leech.el ends here
