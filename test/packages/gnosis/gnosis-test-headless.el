;;; gnosis-test-headless.el --- Headless chat practice regressions -*- lexical-binding: t; -*-

;; Copyright (C) 2026 Free Software Foundation, Inc.

;;; Commentary:

;; Drive the headless practice API the way a chat agent does: one call per
;; turn, JSON on the wire, no review buffer.  Every test checks that the
;; FSRS scheduler tables are untouched.

;;; Code:

(require 'ert)
(require 'json)
(require 'gnosis-headless)
(require 'gnosis-test-helpers)

(defmacro gnosis-test-headless (&rest body)
  "Run BODY with isolated data and no live review or retained sessions."
  (declare (indent 0) (debug t))
  `(gnosis-test-with-db
     (let ((buffers (buffer-list))
           (gnosis-headless--sessions (make-hash-table :test 'equal))
           (gnosis-agent--launches nil)
           (gnosis-review--running nil)
           (gnosis-review-event-functions nil)
           (gnosis-practice-completed-hook nil)
           (gnosis-string-difference 1)
           (gnosis-practice-retry-distance 10))
       (unwind-protect (progn ,@body)
         (should (equal buffers (buffer-list)))))))

(defun gnosis-test-headless--add (type keimenon hypothesis answer &optional id tags parathema)
  "Insert a TYPE thema with KEIMENON, HYPOTHESIS and ANSWER; return its ID.
Optional ID, TAGS (default (\"upper_limb\")) and PARATHEMA."
  (let ((id (or id (gnosis-generate-id))))
    (gnosis-sqlite-with-transaction gnosis-db
      (gnosis--insert-into 'themata `([,id ,type ,keimenon ,hypothesis ,answer nil]))
      (gnosis-scheduler-initialize-thema id (gnosis--today-int) 0)
      (gnosis--insert-into 'extras `([,id ,(or parathema "") ""]))
      (dolist (tag (or tags '("upper_limb")))
        (gnosis--insert-into 'thema-tag `([,id ,tag]))))
    id))

(defun gnosis-test-headless--deck ()
  "Insert one card of each text kind plus an unsupported model card."
  (gnosis-test-headless--add "basic" "Which nerve supplies deltoid?" '("")
                             '("axillary") 101 nil
                             "The [[id:abc][axillary nerve]] (C5-C6) supplies deltoid and teres minor.")
  (gnosis-test-headless--add "cloze" "Erb palsy injures the C5 and C6 roots." '("" "")
                             '("C5" "C6") 102)
  (gnosis-test-headless--add "mcq" "Which muscle initiates abduction?"
                             '("Deltoid" "Supraspinatus" "Teres major") '("Supraspinatus") 103)
  (gnosis-test-headless--add "mc-cloze" "Wrist drop follows radial nerve injury."
                             '("radial" "ulnar" "median") '("radial") 104)
  (gnosis-test-headless--add "model" "Click the deltoid." '("") '("deltoid") 105))

(defun gnosis-test-headless--fsrs ()
  "Return every scheduling authority row."
  (mapcar (lambda (table) (gnosis-select '* table))
          '(scheduler-config scheduler-active scheduler-baseline scheduler-state
                             review-events review-voids review-activity-baseline)))

(defun gnosis-test-headless--wire (value)
  "Round-trip VALUE through JSON exactly as an agent bridge would."
  (json-parse-string (json-serialize value :false-object :false :null-object nil)
                     :object-type 'plist :array-type 'array
                     :false-object :false :null-object nil))

(defun gnosis-test-headless--practice-count ()
  "Return the number of recorded practice encounters."
  (caar (sqlite-select gnosis-db "SELECT COUNT(*) FROM practice_events")))

(defun gnosis-test-headless--strip-duplicate (plist)
  "Return PLIST without its :duplicate marker."
  (cl-loop for (key value) on plist by #'cddr
           unless (eq key :duplicate) append (list key value)))

(ert-deftest gnosis-headless-start-selects-text-kinds-and-hides-answers ()
  (gnosis-test-headless
    (gnosis-test-headless--deck)
    (let* ((fsrs (gnosis-test-headless--fsrs))
           (start (gnosis-test-headless--wire
                   (gnosis-headless-start :tags '("upper_limb") :limit 10)))
           (question (plist-get start :question)))
      (should (equal "started" (plist-get start :status)))
      (should (= 4 (plist-get start :selected)))
      (should (equal [105] (vconcat (mapcar (lambda (s) (plist-get s :thema-id))
                                            (plist-get start :skipped)))))
      (should (equal "model" (plist-get (aref (plist-get start :skipped) 0) :kind)))
      (should (equal :false (plist-get start :schedule-updated)))
      (should (equal 101 (plist-get question :thema-id)))
      (should (equal "basic" (plist-get question :kind)))
      (should (equal "text" (plist-get question :answer-format)))
      (should (string-prefix-p (plist-get start :session-id) (plist-get question :token)))
      ;; No answer leaks into the presented question.
      (should-not (string-search "axillary" (json-serialize question :false-object :false
                                                            :null-object nil)))
      (should (string-search "Which nerve supplies deltoid?" (plist-get start :text)))
      ;; It is an ordinary resumable practice batch, marked headless.
      (let ((state (gnosis-review--read-session)))
        (should (eq 'practice (gnosis-review-state-mode state)))
        (should (equal '(101 102 103 104) (gnosis-review-state-remaining state)))
        (should (eq t (plist-get (gnosis-review-state-selection state) :headless))))
      (should (zerop (gnosis-test-headless--practice-count)))
      (should (equal fsrs (gnosis-test-headless--fsrs))))))

(ert-deftest gnosis-headless-answer-pends-then-accept-records-practice-only ()
  (gnosis-test-headless
    (gnosis-test-headless--deck)
    (let* ((fsrs (gnosis-test-headless--fsrs))
           (events nil)
           (gnosis-review-event-functions (list (lambda (e) (push (plist-get e :type) events))))
           (start (gnosis-headless-start :tags '("upper_limb") :limit 4))
           (token (plist-get start :token))
           (verdict (gnosis-test-headless--wire (gnosis-headless-answer token "Axilary"))))
      (should (equal "pending" (plist-get verdict :status)))
      (should (eq t (plist-get verdict :correct)))
      (should (equal ["axillary"] (plist-get verdict :expected)))
      (should (string-search "axillary nerve (C5-C6)" (plist-get verdict :explanation)))
      (should-not (string-search "id:abc" (plist-get verdict :explanation)))
      (should (zerop (gnosis-test-headless--practice-count)))
      (let ((accepted (gnosis-test-headless--wire (gnosis-headless-accept token))))
        (should (equal "accepted" (plist-get accepted :status)))
        (should (equal "success" (plist-get (plist-get accepted :accepted) :outcome)))
        (should (equal :false (plist-get accepted :done)))
        (should (equal 102 (plist-get (plist-get accepted :question) :thema-id)))
        (should-not (equal token (plist-get accepted :token))))
      (should (= 1 (gnosis-test-headless--practice-count)))
      (let ((encounter (gnosis-get 'data 'practice-encounters)))
        (should (equal "text" (plist-get (plist-get encounter :response) :kind)))
        (should (equal "Axilary" (plist-get (plist-get encounter :response) :text)))
        (should (equal "success" (plist-get encounter :original-outcome))))
      (should (equal '(presented answered accepted presented) (reverse events)))
      (should (equal fsrs (gnosis-test-headless--fsrs))))))

(ert-deftest gnosis-headless-tokens-are-idempotent-and-session-scoped ()
  (gnosis-test-headless
    (gnosis-test-headless--deck)
    (let* ((start (gnosis-headless-start :tags '("upper_limb") :limit 4))
           (token (plist-get start :token))
           (first (gnosis-headless-answer token "radial")))
      (should (eq :false (plist-get first :correct)))
      ;; Same reply again (a resent chat message) returns the same verdict.
      (let ((again (gnosis-headless-answer token "radial")))
        (should (eq t (plist-get again :duplicate)))
        (should (equal (plist-get first :expected) (plist-get again :expected))))
      ;; A different reply cannot regrade the presented question.
      (should (equal "already-answered"
                     (plist-get (gnosis-headless-answer token "axillary") :reason)))
      (let ((accepted (gnosis-headless-accept token)))
        (should (equal accepted (gnosis-test-headless--strip-duplicate
                                 (gnosis-headless-accept token))))
        (should (eq t (plist-get (gnosis-headless-accept token) :duplicate))))
      (should (= 1 (gnosis-test-headless--practice-count)))
      ;; Malformed and foreign tokens are refused, not guessed.
      (should-error (gnosis-headless-answer "nonsense" "x") :type 'user-error)
      (should (equal "not-current"
                     (plist-get (gnosis-headless-answer "other-session/abc" "x") :reason))))))

(ert-deftest gnosis-headless-override-and-retry-follow-practice-policy ()
  (gnosis-test-headless
    (gnosis-test-headless--deck)
    (let* ((start (gnosis-headless-start :thema-ids '(101 103) :limit 2))
           (token (plist-get start :token)))
      (gnosis-headless-answer token "axilliary nerve")
      (let ((accepted (gnosis-headless-accept token t)))
        (should (equal "success" (plist-get (plist-get accepted :accepted) :outcome)))
        (should (eq t (plist-get (plist-get accepted :accepted) :overridden)))
        (should (string-prefix-p "Counted as correct" (plist-get accepted :text)))
        (setq token (plist-get accepted :token)))
      (let ((encounter (gnosis-get 'data 'practice-encounters)))
        (should (equal "failure" (plist-get encounter :original-outcome))))
      ;; MCQ by number; invalid input does not consume the attempt.
      (let ((invalid (gnosis-headless-answer token "7")))
        (should (equal "invalid" (plist-get invalid :status)))
        (should (= 1 (gnosis-test-headless--practice-count))))
      (should (eq :false (plist-get (gnosis-headless-answer token "1") :correct)))
      (let ((accepted (gnosis-headless-accept token)))
        ;; A failed card returns once later in the same batch, marked as a retry.
        (should (equal 103 (plist-get (plist-get accepted :question) :thema-id)))
        (should (eq t (plist-get (plist-get accepted :question) :retry)))
        (should (= 2 (plist-get (plist-get accepted :question) :total)))
        (should (string-prefix-p "Again\n" (plist-get accepted :text)))
        (setq token (plist-get accepted :token)))
      (should (eq t (plist-get (gnosis-headless-answer token "supraspinatus") :correct)))
      (let ((done (gnosis-headless-accept token)))
        (should (eq t (plist-get done :done)))
        (should (equal :false (plist-get done :schedule-updated)))
        (let ((summary (plist-get done :summary)))
          (should (= 2 (plist-get summary :unique)))
          (should (= 1 (plist-get summary :first-success)))
          (should (= 1 (plist-get summary :retry-success))))))))

(ert-deftest gnosis-headless-cloze-blanks-and-mc-cloze ()
  (gnosis-test-headless
    (gnosis-test-headless--deck)
    (let* ((start (gnosis-headless-start :thema-ids '(102 104) :limit 2))
           (question (plist-get start :question))
           (token (plist-get start :token)))
      (should (= 2 (plist-get question :blanks)))
      (should-not (string-search "C5" (plist-get question :prompt)))
      (should (string-search "(...)" (plist-get question :prompt)))
      (should (eq t (plist-get (gnosis-headless-answer token "c6; C5") :correct)))
      (setq token (plist-get (gnosis-headless-accept token) :token))
      (let ((encounter (gnosis-get 'data 'practice-encounters)))
        (should (equal "blanks" (plist-get (plist-get encounter :response) :kind)))
        (should (= 2 (length (plist-get (plist-get encounter :response) :inputs)))))
      (let ((q (plist-get (gnosis-headless-next) :question)))
        (should (equal "choice" (plist-get q :answer-format)))
        (should (equal ["radial" "ulnar" "median"] (plist-get q :choices)))
        (should-not (string-search "radial" (plist-get q :prompt))))
      (should (eq t (plist-get (gnosis-headless-answer token "Radial") :correct))))))

(ert-deftest gnosis-headless-cloze-requires-one-answer-per-blank ()
  (gnosis-test-headless
    (gnosis-test-headless--deck)
    (let ((token (plist-get (gnosis-headless-start :thema-ids '(102) :limit 1) :token)))
      ;; Missing, extra (wrong or right) and padded replies are invalid input:
      ;; nothing is consumed and nothing is certified as a complete answer.
      (dolist (reply '("C5" "C5; C6; wrong" "C5; C6; C5" "C5;;C6;x" ""))
        (let ((verdict (gnosis-headless-answer token reply)))
          (should (equal "invalid" (plist-get verdict :status)))
          (should (string-search "2 answers" (plist-get verdict :text)))))
      (should-not (plist-get (gnosis-headless--record
                              (car (gnosis-headless--parse-token token)))
                             :pending))
      ;; Repeated separators around exactly two parts are fine.
      (let ((verdict (gnosis-headless-answer token " C6 ;; c5 ")))
        (should (eq t (plist-get verdict :correct))))
      (gnosis-headless-accept token)
      (let ((response (plist-get (gnosis-get 'data 'practice-encounters) :response)))
        (should (equal " C6 ;; c5 " (plist-get response :text)))
        (should (= 2 (length (plist-get response :inputs)))))
      (should (= 1 (gnosis-test-headless--practice-count))))))

(ert-deftest gnosis-headless-cloze-wrong-blank-keeps-every-part ()
  (gnosis-test-headless
    (gnosis-test-headless--deck)
    (let ((token (plist-get (gnosis-headless-start :thema-ids '(102) :limit 1) :token)))
      (should (eq :false (plist-get (gnosis-headless-answer token "C7; C5") :correct)))
      (gnosis-headless-accept token)
      (let ((inputs (plist-get (plist-get (gnosis-get 'data 'practice-encounters) :response)
                               :inputs)))
        (should (equal '("C7" "C5") (mapcar (lambda (i) (plist-get i :text)) inputs)))
        (should (equal '(nil 0) (mapcar (lambda (i) (plist-get i :matched-blank-index))
                                        inputs)))))))

(ert-deftest gnosis-headless-cloze-never-shows-its-answer ()
  (gnosis-test-headless
    ;; Native writer: answer-matching destination, partial destination,
    ;; repeated answer, several blanks, and an mc-cloze variant.
    (let ((ids (list (gnosis-add-thema-fields "cloze" "See [[id:C5][source]]: C5 root."
                                              '("") '("C5") "" '("upper_limb") 0 nil)
                     (gnosis-add-thema-fields "cloze" "Per [[id:xC5y][notes]], C5 and C6."
                                              '("" "") '("C5" "C6") "" '("upper_limb") 0 nil)
                     (gnosis-add-thema-fields "mc-cloze" "[[id:radial][Atlas]]: radial nerve."
                                              '("radial" "ulnar") '("radial") ""
                                              '("upper_limb") 0 nil)
                     (gnosis-add-thema-fields "cloze" "C5 then C5 again."
                                              '("") '("C5") "" '("upper_limb") 0 nil))))
      (let* ((start (gnosis-headless-start :tags '("upper_limb") :limit 10))
             (skipped (append (plist-get start :skipped) nil)))
        ;; The repeated answer cannot be fully masked: skipped, not leaked.
        (should (equal (list (nth 3 ids)) (mapcar (lambda (s) (plist-get s :thema-id)) skipped)))
        (should (= 3 (plist-get start :selected)))
        (let ((token (plist-get start :token)) (seen nil))
          (while token
            (let* ((turn (gnosis-headless-next))
                   (question (plist-get turn :question))
                   (row (gnosis-headless--row (plist-get question :thema-id)))
                   ;; Opaque hex tokens/IDs may contain "c5" by chance.
                   (wire (json-serialize
                          (cl-loop for (key value) on question by #'cddr
                                   unless (memq key '(:token :session-id))
                                   append (list key value))
                          :false-object :false :null-object nil))
                   (expected (if (equal (car row) "mc-cloze") (list (car (nth 3 row)))
                               (nth 3 row))))
              (push (plist-get question :thema-id) seen)
              (dolist (answer expected)
                (should-not (string-search (downcase answer)
                                           (downcase (plist-get question :prompt))))
                ;; Choices name the answer by design; the rest never does.
                (unless (equal (car row) "mc-cloze")
                  (should-not (string-search (downcase answer)
                                             (downcase (plist-get turn :text))))))
              (should (string-search "(...)" (plist-get question :prompt)))
              (unless (equal (car row) "mc-cloze")
                (dolist (answer expected)
                  (should-not (string-search (downcase answer) (downcase wire)))))
              ;; Grading retains the original authored answers.
              (let ((reply (if (equal (car row) "mc-cloze") "1"
                             (mapconcat #'identity (nth 3 row) ";"))))
                (should (eq t (plist-get (gnosis-headless-answer token reply) :correct))))
              (setq token (plist-get (gnosis-headless-accept token) :token))))
          (should (= 3 (length seen))))
        (should (string-search "See source: (...) root."
                               (car (gnosis-headless--cloze-prompt
                                     (gnosis-headless--row (nth 0 ids))))))))))

(ert-deftest gnosis-headless-numbers-and-short-codes-match-exactly ()
  (gnosis-test-headless
    (gnosis-test-headless--add "basic" "Year?" '("") '("1984") 101)
    (gnosis-test-headless--add "cloze" "Erb palsy injures C5." '("") '("C5") 102)
    (gnosis-test-headless--add "basic" "Nerve?" '("") '("axillary") 103)
    (gnosis-test-headless--add "basic" "Dose?" '("") '("0.5 mg") 104)
    (let ((token (plist-get (gnosis-headless-start :thema-ids '(101 102 103 104)) :token)))
      (should (eq :false (plist-get (gnosis-headless-answer token "1985") :correct)))
      (setq token (plist-get (gnosis-headless-accept token t) :token))
      (should (eq :false (plist-get (gnosis-headless-answer token "C6") :correct)))
      (setq token (plist-get (gnosis-headless-accept token) :token))
      ;; Longer words keep the native typo tolerance.
      (should (eq t (plist-get (gnosis-headless-answer token "axilary") :correct)))
      (setq token (plist-get (gnosis-headless-accept token) :token))
      (should (eq :false (plist-get (gnosis-headless-answer token "0.6 mg") :correct)))
      (gnosis-headless-accept token)
      ;; The effective tolerance is recorded with each encounter.
      (should (equal '(0 0 0 1)
                     (sort (mapcar (lambda (row)
                                     (plist-get (plist-get (car row) :match-rule) :tolerance))
                                   (gnosis-select 'data 'practice-encounters))
                           #'<))))))

(ert-deftest gnosis-headless-reverse-override-says-wrong ()
  (gnosis-test-headless
    (gnosis-test-headless--deck)
    (let ((token (plist-get (gnosis-headless-start :thema-ids '(101 103)) :token)))
      (should (eq t (plist-get (gnosis-headless-answer token "axillary") :correct)))
      (let ((accepted (gnosis-headless-accept token t)))
        (should (equal "failure" (plist-get (plist-get accepted :accepted) :outcome)))
        (should (string-prefix-p "Counted as wrong." (plist-get accepted :text)))))))

(ert-deftest gnosis-headless-refuses-live-review-and-unfinished-batches ()
  (gnosis-test-headless
    (gnosis-test-headless--deck)
    (let ((gnosis-review--running "desk-session"))
      (should (equal "review-running"
                     (plist-get (gnosis-headless-start :tags '("upper_limb")) :reason)))
      (should-not (gnosis-review--read-session)))
    (let* ((desk (cl-letf (((symbol-function 'run-with-timer) (lambda (&rest _) nil)))
                   (plist-get (gnosis-agent-start-practice :thema-ids '(101 103) :limit 2)
                              :session-id)))
           (refused (gnosis-headless-start :tags '("upper_limb"))))
      (should (equal "unfinished-batch" (plist-get refused :reason)))
      (should (= 2 (plist-get refused :remaining)))
      (should (equal desk (gnosis-review-state-session-id (gnosis-review--read-session))))
      ;; The desktop batch cannot be driven headlessly.
      (should (equal "not-current" (plist-get (gnosis-headless-next desk) :reason)))
      (let ((started (gnosis-headless-start :tags '("upper_limb") :replace t)))
        (should (equal "started" (plist-get started :status)))
        (should (equal "cancelled" (plist-get (gnosis-agent-status desk) :status)))
        (let ((gnosis-review--running "desk-session"))
          (should (equal "review-running"
                         (plist-get (gnosis-headless-answer (plist-get started :token) "x")
                                    :reason)))
          (should (equal "review-running"
                         (plist-get (gnosis-headless-accept (plist-get started :token))
                                    :reason))))))))

(ert-deftest gnosis-headless-desktop-progress-retires-token ()
  (gnosis-test-headless
    (gnosis-test-headless--deck)
    (let* ((start (gnosis-headless-start :thema-ids '(101 103) :limit 2))
           (token (plist-get start :token)))
      (gnosis-headless-answer token "axillary")
      ;; The learner answered the same card at the desk meanwhile.
      (let* ((gnosis-review--state (gnosis-review--read-session)))
        (gnosis-review-result 101 t (gnosis-review-algorithm 101 t)))
      ;; The same presentation was counted there: never count it twice.
      (let ((counted (gnosis-headless-accept token)))
        (should (equal "already-accepted" (plist-get counted :reason)))
        (should (equal 103 (plist-get (plist-get counted :question) :thema-id))))
      (should (= 1 (gnosis-test-headless--practice-count)))
      ;; Two desk answers later, the old token is plainly stale.
      (let ((gnosis-review--state (gnosis-review--read-session)))
        (gnosis-review-result 103 t (gnosis-review-algorithm 103 t)))
      (should (equal "stale" (plist-get (gnosis-headless-answer token "x") :reason)))
      (should (= 2 (gnosis-test-headless--practice-count))))))

(ert-deftest gnosis-headless-content-change-represents-question ()
  (gnosis-test-headless
    (gnosis-test-headless--deck)
    (let ((token (plist-get (gnosis-headless-start :thema-ids '(101) :limit 1) :token)))
      (gnosis-sqlite-execute gnosis-db "UPDATE themata SET keimenon = ? WHERE id = 101"
                             '("Which nerve supplies teres minor?"))
      (let ((changed (gnosis-headless-answer token "axillary")))
        (should (equal "changed" (plist-get changed :reason)))
        (should (string-search "teres minor" (plist-get (plist-get changed :question) :prompt)))
        (should-not (equal token (plist-get changed :token)))
        ;; The old token stays stale; the new one grades the new text.
        (should (equal "changed" (plist-get (gnosis-headless-answer token "axillary") :reason)))
        (should (eq t (plist-get (gnosis-headless-answer (plist-get changed :token) "axillary")
                                 :correct)))))))

(ert-deftest gnosis-headless-edited-card-retires-old-token ()
  (dolist (edit '(("keimenon" "Which nerve supplies interossei?")
                  ("answer" ("ulnar"))
                  ("hypothesis" ("a hint"))
                  ("type" "mcq")))
    (gnosis-test-headless
      (gnosis-test-headless--deck)
      (let* ((fsrs (gnosis-test-headless--fsrs))
             (start (gnosis-headless-start :thema-ids '(101 103)))
             (old (plist-get start :token)))
        ;; Edit between presentation and answer, and again after a pending answer.
        (gnosis-sqlite-execute gnosis-db (format "UPDATE themata SET %s = ? WHERE id = 101"
                                                 (car edit))
                               (cdr edit))
        (let ((new (plist-get (gnosis-headless-next) :token)))
          (should-not (equal old new))
          (dotimes (_ 2)
            (should (equal "changed" (plist-get (gnosis-headless-answer old "axillary") :reason)))
            (should (equal "changed" (plist-get (gnosis-headless-accept old) :reason))))
          ;; Process restart: caches lost, the old token is still stale.
          (clrhash gnosis-headless--sessions)
          (sqlite-close gnosis-db)
          (setq gnosis-db nil)
          (should (equal "changed" (plist-get (gnosis-headless-answer old "axillary") :reason)))
          (should (equal new (plist-get (gnosis-headless-next) :token))))
        (should (zerop (gnosis-test-headless--practice-count)))
        (should (equal fsrs (gnosis-test-headless--fsrs)))))))

(ert-deftest gnosis-headless-edit-after-answer-discards-pending ()
  (gnosis-test-headless
    (gnosis-test-headless--deck)
    (let* ((old (plist-get (gnosis-headless-start :thema-ids '(101 103)) :token)))
      (should (eq t (plist-get (gnosis-headless-answer old "axillary") :correct)))
      (gnosis-sqlite-execute gnosis-db "UPDATE themata SET keimenon = ?, answer = ? WHERE id = 101"
                             (list "Which nerve supplies interossei?" '("ulnar")))
      (dotimes (_ 2)
        (should (equal "changed" (plist-get (gnosis-headless-accept old) :reason)))
        (should (equal "changed" (plist-get (gnosis-headless-answer old "axillary") :reason))))
      (should (zerop (gnosis-test-headless--practice-count)))
      (let ((new (plist-get (gnosis-headless-next) :token)))
        (should (eq :false (plist-get (gnosis-headless-answer new "axillary") :correct)))
        (should (equal "ulnar" (aref (plist-get (gnosis-headless-answer new "axillary")
                                                :expected)
                                     0)))))))

(ert-deftest gnosis-headless-subscribers-cannot-reenter-or-mix-sessions ()
  ;; A nested API call from an accepted subscriber refuses.
  (gnosis-test-headless
    (gnosis-test-headless--deck)
    (let* ((start (gnosis-headless-start :thema-ids '(101 103)))
           (token (plist-get start :token))
           (nested nil))
      (gnosis-headless-answer token "axillary")
      (let* ((gnosis-review-event-functions
              (list (lambda (e)
                      (when (eq (plist-get e :type) 'accepted)
                        (push (gnosis-headless-start :thema-ids '(104) :replace t) nested)
                        (push (gnosis-headless-accept token) nested)))))
             (accepted (gnosis-headless-accept token)))
        (should (equal '("reentrant" "reentrant") (mapcar (lambda (n) (plist-get n :reason))
                                                          nested)))
        (should (equal (plist-get start :session-id)
                       (plist-get (plist-get accepted :question) :session-id)))
        (should (equal 103 (plist-get (plist-get accepted :question) :thema-id))))
      (should (= 1 (gnosis-test-headless--practice-count)))))
  ;; A subscriber that replaces the batch by other means: no successor
  ;; question, cache entry or completion on the old call's behalf.
  (dolist (hook-type '(accepted presented))
    (gnosis-test-headless
      (gnosis-test-headless--deck)
      (let* ((start (gnosis-headless-start :thema-ids '(101 103)))
             (old-session (plist-get start :session-id))
             (token (plist-get start :token))
             (successor nil)
             (completed nil)
             (gnosis-practice-completed-hook (list (lambda (e) (push e completed)))))
        (gnosis-headless-answer token "axillary")
        (let* ((gnosis-review-event-functions
                (list (lambda (e)
                        (when (and (eq (plist-get e :type) hook-type) (null successor))
                          (setq successor
                                (gnosis-review--reserve-batch
                                 '(104) 'practice gnosis-headless-policy
                                 (list :headless t) (gnosis-review--session-target)))))))
               (accepted (gnosis-headless-accept token)))
          (should (equal "accepted" (plist-get accepted :status)))
          (should (equal old-session (plist-get accepted :session-id)))
          (should (equal "replaced" (plist-get accepted :continuation)))
          (should-not (plist-get accepted :question))
          (should-not (plist-get accepted :token))
          (should-not (string-search "Wrist drop" (format "%S" accepted)))
          (should-not (string-search (gnosis-review-state-session-id successor)
                                     (format "%S" accepted)))
          ;; Replay returns the same receipt, still without a successor.
          (should (equal accepted (gnosis-test-headless--strip-duplicate
                                   (gnosis-headless-accept token)))))
        (should-not completed)
        (should (= 1 (gnosis-test-headless--practice-count))))))
  ;; Replaced before acceptance by an answered or overridden subscriber: nothing counts.
  (dolist (hook-type '(answered overridden))
    (gnosis-test-headless
      (gnosis-test-headless--deck)
      (let* ((start (gnosis-headless-start :thema-ids '(101 103)))
             (token (plist-get start :token))
             (fired nil))
        (let ((gnosis-review-event-functions
               (list (lambda (e)
                       (when (and (eq (plist-get e :type) hook-type) (not fired))
                         (setq fired t)
                         (gnosis-review--reserve-batch
                          '(104) 'practice gnosis-headless-policy
                          (list :headless t) (gnosis-review--session-target)))))))
          (let ((answered (gnosis-headless-answer token "ulnar")))
            (when (eq hook-type 'answered)
              (should (equal "replaced" (plist-get answered :reason)))))
          (when (eq hook-type 'overridden)
            (should (equal "replaced" (plist-get (gnosis-headless-accept token t) :reason)))))
        (should fired)
        (should (zerop (gnosis-test-headless--practice-count)))))))

(ert-deftest gnosis-headless-restart-recovers-from-checkpoint ()
  (gnosis-test-headless
    (gnosis-test-headless--deck)
    (let* ((start (gnosis-headless-start :thema-ids '(101 103) :limit 2))
           (token (plist-get start :token)))
      (gnosis-headless-answer token "axillary")
      (let ((next-token (plist-get (gnosis-headless-accept token) :token)))
        ;; Emacs restarted: process-local caches are gone.
        (clrhash gnosis-headless--sessions)
        (let ((again (gnosis-headless-accept token)))
          (should (equal "already-accepted" (plist-get again :reason)))
          (should (equal next-token (plist-get again :token))))
        (let ((current (gnosis-headless-next)))
          (should (equal next-token (plist-get current :token)))
          (should (equal 103 (plist-get (plist-get current :question) :thema-id)))))
      (should (= 1 (gnosis-test-headless--practice-count))))))

(ert-deftest gnosis-headless-end-summarizes-and-keeps-batch-resumable ()
  (gnosis-test-headless
    (gnosis-test-headless--deck)
    (let* ((fsrs (gnosis-test-headless--fsrs))
           (start (gnosis-headless-start :tags '("upper_limb") :limit 4))
           (session (plist-get start :session-id))
           (token (plist-get start :token)))
      (gnosis-headless-answer token "ulnar")
      (let ((ended (gnosis-test-headless--wire (gnosis-headless-end session))))
        (should (equal "ended" (plist-get ended :status)))
        (should (eq t (plist-get ended :resumable)))
        (should (= 1 (plist-get (plist-get ended :summary) :needs-work)))
        (should (string-search "schedule unchanged" (plist-get ended :text))))
      ;; The pending verdict was accepted; the batch stays on the desk.
      (should (= 1 (gnosis-test-headless--practice-count)))
      (should (equal "unfinished" (plist-get (gnosis-agent-status session) :status)))
      (let ((cancelled (gnosis-headless-end session :cancel t)))
        (should (eq :false (plist-get cancelled :resumable)))
        (should (equal "cancelled" (plist-get (gnosis-agent-status session) :status))))
      (should (equal fsrs (gnosis-test-headless--fsrs))))))

(ert-deftest gnosis-headless-completion-notifies-once ()
  (gnosis-test-headless
    (gnosis-test-headless--deck)
    (let* ((seen nil)
           (gnosis-practice-completed-hook (list (lambda (e) (push e seen))))
           (token (plist-get (gnosis-headless-start :thema-ids '(103) :limit 1) :token)))
      (gnosis-headless-answer token "2")
      (should (eq t (plist-get (gnosis-headless-accept token) :done)))
      (gnosis-headless-accept token)
      (should (= 1 (length seen)))
      (should (equal "practice" (plist-get (car seen) :mode)))
      (should (eq t (plist-get (gnosis-headless-next) :done))))))

(ert-deftest gnosis-headless-selectors ()
  (gnosis-test-headless
    (gnosis-test-headless--deck)
    (gnosis-test-headless--add "basic" "Unrelated" '("") '("x") 106 '("other"))
    ;; Weak: a lapsed card ranks first; unlapsed cards are excluded.
    (gnosis-sqlite-execute gnosis-db
                           "UPDATE scheduler_state SET lapses = 3, reps = 4 WHERE thema_id = 103")
    (let ((weak (gnosis-headless-start :weak t :limit 5)))
      (should (equal 103 (plist-get (plist-get weak :question) :thema-id)))
      (should (= 1 (plist-get weak :selected))))
    (gnosis-headless-end (gnosis-review-state-session-id (gnosis-review--read-session)) :cancel t)
    ;; Due: every new card is due today; models are still skipped.
    (let ((due (gnosis-headless-start :due t :limit 50)))
      (should (= 5 (plist-get due :selected)))
      (should (= 1 (length (plist-get due :skipped))))
      (gnosis-headless-end (plist-get due :session-id) :cancel t))
    ;; Exam: scope by exam tags.
    (require 'gnosis-exam)
    (gnosis-exam-save "Anatomy" 20991231 '(("other")))
    (let ((exam (gnosis-headless-start :exam "Anatomy" :limit 5)))
      (should (= 1 (plist-get exam :selected)))
      (should (equal 106 (plist-get (plist-get exam :question) :thema-id)))
      (gnosis-headless-end (plist-get exam :session-id) :cancel t))
    (should (equal "empty" (plist-get (gnosis-headless-start :tags '("missing")) :reason)))
    (should-error (gnosis-headless-start :tags '("a") :due t) :type 'user-error)
    (should-error (gnosis-headless-start :limit 5) :type 'user-error)
    (should-error (gnosis-headless-start :due t :limit 0) :type 'user-error)
    (should-error (gnosis-headless-start :exam "Nope") :type 'user-error)))

(provide 'gnosis-test-headless)
;;; gnosis-test-headless.el ends here
