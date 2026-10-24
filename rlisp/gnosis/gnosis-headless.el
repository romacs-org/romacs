;;; gnosis-headless.el --- Headless text practice for chat agents -*- lexical-binding: t; -*-

;; Copyright (C) 2026 Free Software Foundation, Inc.
;; Author: Thanos Apollo <public@thanosapollo.org>
;; SPDX-License-Identifier: GPL-3.0-or-later

;;; Commentary:

;; Practice away from the desk.  A chat agent (Hermes over XMPP, say)
;; calls these functions in the learner's running Emacs, one call per chat
;; turn, and relays plain text.  There is no buffer, no recursive input and
;; no second database.
;;
;;   (gnosis-headless-start :tags \\='("upper_limb") :limit 10)
;;   (gnosis-headless-answer TOKEN "axillary")   ; verdict, not yet counted
;;   (gnosis-headless-accept TOKEN &optional OVERRIDE) ; counted, next question
;;   (gnosis-headless-next &optional SESSION-ID) ; re-send current turn
;;   (gnosis-headless-end SESSION-ID &key cancel discard-pending)
;;
;; A headless batch is an ordinary practice batch: it occupies the single
;; active batch row, its accepted answers are practice events with
;; encounter evidence, and the desktop can resume it.  Practice never
;; writes FSRS state; scheduled review away from the desk is deliberately
;; not offered.  Only basic, cloze, mcq and mc-cloze themata without
;; images are selected; others are reported as skipped.
;;
;; Every call refuses while native review input is running, and nested
;; calls (from review event subscribers) refuse too.  Tokens are
;; SESSION/ATTEMPT/REVISION: they name one presentation of one content
;; revision and go stale as soon as the batch moves on, including from the
;; desktop, or the card is edited.  Repeating a call with the same token is
;; harmless.  Results are JSON-safe keyword plists: use
;; `json-serialize' with :false-object :false and :null-object nil.

;;; Code:

(require 'cl-lib)
(require 'gnosis-agent)
(require 'gnosis-cloze)

(declare-function gnosis-exam-list "gnosis-exam" (&optional db))
(declare-function gnosis-exam-exam-coverage "gnosis-exam" (exam &optional db today))
(declare-function gnosis-exam-cram-ids "gnosis-exam" (coverage &optional limit))

(defgroup gnosis-headless nil
  "Headless text practice for chat agents."
  :group 'gnosis)

(defcustom gnosis-headless-explanation-limit 280
  "Maximum characters of explanation returned after an answer."
  :type 'natnum)

(defconst gnosis-headless-kinds '("basic" "cloze" "mcq" "mc-cloze")
  "Thema kinds that can be practised as plain text.")

(defconst gnosis-headless-policy
  '(:successes-required 1 :successes-after-failure 1
    :consecutive t :max-attempts-per-thema 2)
  "Frozen practice policy: one correct answer, one retry after a miss.")

(defvar gnosis-headless--active nil
  "Non-nil while a public headless call runs; nested calls refuse.")

(defvar gnosis-headless--sessions (make-hash-table :test 'equal)
  "Process-local turn caches keyed by session ID.
Each value is a plist: :db, :token, :thema, :content, :hints, :pending and
:accepted (an alist from token to payload).  Losing it loses only an
unaccepted verdict; the durable checkpoint owns progress.")

;;; Plain text

(defun gnosis-headless--plain (text)
  "Return Org TEXT as one plain line, keeping link descriptions."
  (if (not (stringp text)) ""
    (let ((s (substring-no-properties text)))
      (setq s (replace-regexp-in-string "\\[\\[[^]]*\\]\\[\\([^]]*\\)\\]\\]" "\\1" s t))
      (setq s (replace-regexp-in-string
               "\\[\\[\\(?:id:\\|gnosis-image:\\)[^]]*\\]\\]" "" s t t))
      (setq s (replace-regexp-in-string "\\[\\[\\([^]]*\\)\\]\\]" "\\1" s t))
      (string-trim (replace-regexp-in-string "[ \t\n]+" " " s)))))

(defun gnosis-headless--short (text limit)
  "Return plain TEXT cut to LIMIT characters at a sentence end if possible."
  (let ((s (gnosis-headless--plain text)))
    (if (<= (length s) limit) s
      (let ((cut (string-match-p "[.!?] [^.!?]*\\'" (substring s 0 limit))))
        (if (and cut (> cut (/ limit 3)))
            (substring s 0 (1+ cut))
          (concat (string-trim-right (substring s 0 (1- limit))) "…"))))))

;;; Refusals and identity

(defun gnosis-headless--refusal (reason text &rest properties)
  "Return a refusal for REASON with chat TEXT and extra PROPERTIES."
  (append (list :api-version 1 :status "refused" :reason reason :text text)
          properties))

(defun gnosis-headless--busy ()
  "Return a refusal if native review input is active, else nil."
  (when gnosis-review--running
    (gnosis-headless--refusal
     "review-running" "Gnosis is in a review at your desk.  Finish it there first.")))

(defun gnosis-headless--parse-token (token)
  "Return (SESSION-ID EVENT-ID REVISION) from TOKEN, refusing malformed input.
REVISION is nil for a token without one, which never matches."
  (unless (and (stringp token)
               (string-match "\\`\\([^/]+\\)/\\([^/]+\\)\\(?:/\\([^/]+\\)\\)?\\'" token))
    (user-error "Malformed headless token"))
  (list (match-string 1 token) (match-string 2 token) (match-string 3 token)))

(defun gnosis-headless--revision (id)
  "Return a short digest naming thema ID's current content revision."
  (if (null id) "0"
    (substring (secure-hash 'sha256 (prin1-to-string (gnosis-review--content-thema id)))
               0 16)))

(defun gnosis-headless--token (state)
  "Return the token of STATE's current presentation.
It names the session, the attempt and the head's content revision, so an
edited card gets a fresh token and replies to the old text go stale."
  (format "%s/%s/%s" (gnosis-review-state-session-id state)
          (gnosis-review-state-event-id state)
          (gnosis-headless--revision (car (gnosis-review-state-remaining state)))))

(defun gnosis-headless--owner (db session-id)
  "Return the state of SESSION-ID if DB and its batch still own the desk.
Event subscribers may run arbitrary code; check this after every emit."
  (and (not gnosis-review--running)
       (eq db (gnosis--ensure-db))
       (gnosis-headless--current session-id)))

(defun gnosis-headless--replaced (session-id)
  "Return the refusal for SESSION-ID after its batch was replaced mid-call."
  (gnosis-headless--refusal
   "replaced" "That practice batch was replaced while answering; nothing more here."
   :session-id session-id))

(defun gnosis-headless--current (&optional session-id)
  "Return the active headless practice state, or nil.
With SESSION-ID, require that exact session."
  (when-let* ((state (gnosis-review--read-session)))
    (and (eq 'practice (gnosis-review-state-mode state))
         (not (gnosis-review-state-cancelled-p state))
         (null (gnosis-review-state-agent-review state))
         (eq t (plist-get (gnosis-review-state-selection state) :headless))
         (or (null session-id)
             (equal session-id (gnosis-review-state-session-id state)))
         state)))

(defun gnosis-headless--record (session-id)
  "Return the turn cache for SESSION-ID on the current database."
  (let ((record (gethash session-id gnosis-headless--sessions)))
    (unless (and record (eq (plist-get record :db) (gnosis--ensure-db)))
      (setq record (list :db (gnosis--ensure-db)))
      (puthash session-id record gnosis-headless--sessions))
    record))

(defun gnosis-headless--update (session-id &rest properties)
  "Set PROPERTIES in SESSION-ID's turn cache and return it."
  (let ((record (gnosis-headless--record session-id)))
    (cl-loop for (key value) on properties by #'cddr
             do (setq record (plist-put record key value)))
    (puthash session-id record gnosis-headless--sessions)
    record))

(defmacro gnosis-headless--isolated (&rest body)
  "Run BODY outside any review buffer's local state, refusing reentry.
A review event subscriber that calls back into this API gets a refusal
instead of replacing the batch under the outer call."
  (declare (indent 0) (debug t))
  `(if gnosis-headless--active
       (gnosis-headless--refusal
        "reentrant" "Another practice call is still running; try again.")
     (let ((gnosis-headless--active t))
       (with-temp-buffer ,@body))))

;;; Content

(defun gnosis-headless--row (id)
  "Return thema ID's content row in `gnosis-review--content-thema' order."
  (car (gnosis-review--content-thema id)))

(defun gnosis-headless--leaks-p (text answers)
  "Return non-nil if any of ANSWERS still appears in TEXT, ignoring case."
  (let ((haystack (downcase text)))
    (seq-some (lambda (answer)
                (string-search (downcase (string-trim (gnosis-utils-trim-quotes answer)))
                               haystack))
              answers)))

(defun gnosis-headless--mask (row remaining hints)
  "Return (PROMPT . SHOWN-HINTS) for ROW's REMAINING blanks masked, or nil.
Masks show their HINTS where present.  Mask the plain text actually
shown, with the review's canonical cloze renderer, so a hidden link
destination cannot absorb a mask.  Return nil when a masked answer would
still be visible, rather than leak it."
  (let* ((answers (mapcar #'gnosis-headless--plain (nth 3 row)))
         (plain (gnosis-headless--plain (nth 1 row)))
         (bare (gnosis-headless--plain
                (car (gnosis-cloze--render plain answers remaining nil)))))
    (unless (gnosis-headless--leaks-p bare (mapcar (lambda (i) (nth i answers)) remaining))
      (let ((rendered (gnosis-cloze--render plain answers remaining hints)))
        (cons (gnosis-headless--plain (car rendered)) (cdr rendered))))))

(defun gnosis-headless--cloze-prompt (row)
  "Return (PROMPT . SHOWN-HINTS) for cloze or mc-cloze ROW, or nil if unsafe."
  (if (equal (nth 0 row) "mc-cloze")
      (gnosis-headless--mask row '(0) nil)
    (gnosis-headless--mask row (number-sequence 0 (1- (length (nth 3 row))))
                           (nth 2 row))))

(defun gnosis-headless--skip-reason (row)
  "Return why content ROW cannot be practised as text, or nil."
  (let ((kind (nth 0 row)))
    (cond ((not (member kind gnosis-headless-kinds))
           (format "%s cards need the desktop" kind))
          ((seq-some #'gnosis-image-content-p (list (nth 1 row) (nth 2 row) (nth 3 row)))
           "image questions need the desktop")
          ((and (member kind '("cloze" "mc-cloze"))
                (null (gnosis-headless--cloze-prompt row)))
           "cloze would show its answer in plain text"))))

(defun gnosis-headless--question (state row)
  "Return the answer-free question payload for STATE's head with content ROW."
  (let* ((id (car (gnosis-review-state-remaining state)))
         (kind (nth 0 row))
         (hint (and (equal kind "basic") (car (nth 2 row))))
         (hint (and (stringp hint) (not (string-empty-p hint)) (gnosis-headless--plain hint)))
         (cloze (and (member kind '("cloze" "mc-cloze")) (gnosis-headless--cloze-prompt row)))
         (prompt (if cloze (car cloze) (gnosis-headless--plain (nth 1 row))))
         (choices (when (member kind '("mcq" "mc-cloze"))
                    (vconcat (mapcar #'gnosis-headless--plain (nth 2 row)))))
         (blanks (when (equal kind "cloze") (length (nth 3 row))))
         (outcomes (gnosis-review-state-outcomes state))
         (retry (and (assq id outcomes) t))
         (total (or (plist-get (gnosis-review-state-selection state) :selected)
                    (gnosis-review-state-total state)))
         (position (+ (length (delete-dups (mapcar #'car outcomes))) (if retry 0 1))))
    (list :token (gnosis-headless--token state)
          :session-id (gnosis-review-state-session-id state)
          :thema-id id :kind kind :position position :total total
          :retry (if retry t :false)
          :prompt prompt :hint hint :choices choices :blanks blanks
          :answer-format (cond (choices "choice") (blanks "blanks") (t "text"))
          :hints-shown (vconcat (cond (hint (list hint))
                                      ((equal kind "cloze") (cdr cloze))))
          :text (concat
                 (if retry (format "Again\n%s" prompt)
                   (format "%d/%d\n%s" position total prompt))
                 (when hint (format "\nHint: %s" hint))
                 (when choices
                   (mapconcat #'identity
                              (cl-loop for choice across choices for n from 1
                                       collect (format "\n%d. %s" n choice))
                              ""))
                 (when (and blanks (> blanks 1))
                   (format "\n(%d blanks, separate with ;)" blanks))))))

;;; Turns

(defun gnosis-headless--present (state &optional quiet)
  "Present STATE's head as a question payload and cache its content.
Skip unavailable heads through the native skip path.  QUIET suppresses the
presented event, as for a re-sent turn.  A re-sent turn keeps its content
snapshot and pending verdict; an edited card has a new token and starts
afresh.  If a presented subscriber replaces the batch, refuse."
  (let ((gnosis-review--state state))
    (while (and (gnosis-review-state-remaining state)
                (not (gnosis-study-eligible-p (car (gnosis-review-state-remaining state)))))
      (gnosis-review--skip state (car (gnosis-review-state-remaining state))))
    (if (not (gnosis-review--work-p state))
        (gnosis-headless--done state)
      (let* ((session-id (gnosis-review-state-session-id state))
             (id (car (gnosis-review-state-remaining state)))
             (row (gnosis-headless--row id))
             (token (gnosis-headless--token state))
             (record (gnosis-headless--record session-id)))
        (if-let* ((reason (gnosis-headless--skip-reason row)))
            (gnosis-headless--refusal
             "needs-desktop" (format "The next card (%d) %s.  Continue at your desk." id reason)
             :session-id session-id :thema-id id)
          (let ((question (gnosis-headless--question state row))
                (db (gnosis--ensure-db))
                (event (gnosis-review-state-event-id state)))
            (unless (or quiet (equal token (plist-get record :presented)))
              (gnosis-review--emit 'presented id :source "headless"))
            (if (not (when-let* ((owner (gnosis-headless--owner db session-id)))
                       (equal event (gnosis-review-state-event-id owner))))
                (gnosis-headless--replaced session-id)
              (unless (equal token (plist-get record :token))
                (gnosis-headless--update session-id :token token :thema id
                                         :content (gnosis-review--content-thema id)
                                         :hints (append (plist-get question :hints-shown) nil)
                                         :pending nil))
              (gnosis-headless--update session-id :presented token)
              (list :api-version 1 :status "question" :token token
                    :session-id session-id :question question
                    :schedule-updated :false :text (plist-get question :text)))))))))

(defun gnosis-headless--choice (response choices)
  "Return the member of CHOICES chosen by RESPONSE (number or text), or nil."
  (let ((reply (string-trim response)))
    (if (string-match-p "\\`[0-9]+\\'" reply)
        (nth (1- (string-to-number reply)) choices)
      (seq-find (lambda (choice)
                  (string= (downcase (string-trim (gnosis-headless--plain choice)))
                           (downcase reply)))
                choices))))

(defun gnosis-headless--tolerance (answer)
  "Return the typo tolerance for ANSWER in headless practice.
Numbers, codes such as C5 and answers of three characters or fewer must
match exactly after normalization; a near miss there is a different answer."
  (let ((text (replace-regexp-in-string
               "\\s-" "" (gnosis-utils-trim-quotes (gnosis-headless--plain answer)))))
    (if (or (string-match-p "[0-9]" text) (<= (length text) 3)) 0
      gnosis-string-difference)))

(defun gnosis-headless--match (answer input &optional aliases)
  "Return non-nil if INPUT matches ANSWER or ALIASES with its tolerance."
  (gnosis-answer-match-p answer input aliases (gnosis-headless--tolerance answer)))

(defun gnosis-headless--grade (row response)
  "Grade RESPONSE for content ROW with the native matching rules.
Numbers and short codes match exactly.  Return (SUCCESS RESPONSE-EVIDENCE
TOLERANCE), or a string for invalid input, which consumes nothing."
  (let ((kind (nth 0 row)) (answers (nth 3 row)))
    (pcase kind
      ("basic"
       (list (and (gnosis-headless--match (car answers) response (nth 4 row)) t)
             (list :kind "text" :text response)
             (gnosis-headless--tolerance (car answers))))
      ((or "mcq" "mc-cloze")
       (if-let* ((choice (gnosis-headless--choice response (nth 2 row))))
           (list (string= choice (car answers))
                 (list :kind "choice" :selected choice :choices (vconcat (nth 2 row)))
                 nil)
         (format "Reply with a number from 1 to %d." (length (nth 2 row)))))
      ("cloze"
       (let ((parts (seq-remove #'string-empty-p
                                (mapcar #'string-trim (split-string response "[;\n]"))))
             (count (length answers)))
         (if (/= (length parts) count)
             (if (= count 1) "Reply with one answer (? if you don't know)."
               (format "Reply with %d answers separated by ; (? for any you don't know)."
                       count))
           ;; Exactly one part per blank: every part is graded and retained.
           (let ((indices (number-sequence 0 (1- count)))
                 (tolerances (mapcar #'gnosis-headless--tolerance answers))
                 (success t) inputs)
             (dolist (input parts)
               (let* ((remaining (mapcar (lambda (i) (nth i answers)) indices))
                      (position (cl-position input remaining
                                             :test (lambda (in answer)
                                                     (gnosis-headless--match answer in)))))
                 (push (list :text input :remaining-blank-indices (vconcat indices)
                             :matched-blank-index (and position (nth position indices)))
                       inputs)
                 (if position
                     (setq indices (remq (nth position indices) indices))
                   (setq success nil))))
             (list success
                   (list :kind "blanks" :text response :inputs (vconcat (nreverse inputs)))
                   (if (cl-every (lambda (tol) (eql tol (car tolerances))) tolerances)
                       (car tolerances)
                     (vconcat tolerances))))))))))

(defun gnosis-headless--verdict-text (row success response)
  "Return chat text for SUCCESS on content ROW after RESPONSE."
  (let* ((kind (nth 0 row))
         (expected (mapconcat #'gnosis-headless--plain (nth 3 row) "; "))
         (why (gnosis-headless--short (nth 5 row) gnosis-headless-explanation-limit)))
    (concat (if success (format "✓ %s" expected)
              (format "✗ You: %s\nAnswer: %s"
                      (if (string-empty-p (string-trim response)) "(blank)"
                        (if (member kind '("mcq" "mc-cloze"))
                            (or (gnosis-headless--choice response (nth 2 row)) response)
                          response))
                      expected))
            (unless (string-empty-p why) (concat "\n" why))
            (unless success "\nReply o if you were right, or anything to go on."))))

(defun gnosis-headless--locate (token)
  "Return (STATE RECORD) for current TOKEN, or a refusal payload.
Stale, edited and already-accepted tokens refuse with the current turn
attached, which carries a fresh token."
  (pcase-let ((`(,session-id ,event ,_) (gnosis-headless--parse-token token)))
    (let ((state (gnosis-headless--current session-id)))
      (cond
       ((null state)
        (gnosis-headless--refusal
         "not-current" "That practice batch is no longer active." :session-id session-id))
       ((equal token (gnosis-headless--token state))
        (list state (gnosis-headless--record session-id)))
       (t
        (let* ((reason (cond ((equal event (gnosis-review-state-event-id state)) "changed")
                             ((equal event (gnosis-review-state-last-event state))
                              "already-accepted")
                             (t "stale")))
               (turn (gnosis-headless--present state t)))
          (append (gnosis-headless--refusal
                   reason
                   (concat (pcase reason
                             ("changed" "That card was just edited; here it is again.")
                             ("already-accepted" "Already counted.")
                             (_ "That question moved on (maybe at your desk)."))
                           "\n\n" (plist-get turn :text)))
                  (list :token (plist-get turn :token) :question (plist-get turn :question)
                        :done (or (plist-get turn :done) :false)))))))))

(defun gnosis-headless--cached (token)
  "Return TOKEN's retained acceptance payload, or nil."
  (when-let* ((session-id (car (gnosis-headless--parse-token token)))
              (record (gethash session-id gnosis-headless--sessions))
              ((eq (plist-get record :db) (gnosis--ensure-db))))
    (cdr (assoc token (plist-get record :accepted)))))

;;; Summary

(defun gnosis-headless--summary (session-id)
  "Return summary payload fields for exact SESSION-ID."
  (let* ((status (gnosis-agent-status session-id))
         (summary (plist-get status :summary))
         (state (gnosis-agent--session session-id))
         (last (let (seen out)
                 (dolist (row (gnosis-review-state-outcomes state) (nreverse out))
                   (unless (memq (car row) seen)
                     (push (car row) seen)
                     (unless (cdr row) (push (car row) out))))))
         (revisit (mapcar (lambda (id)
                            (let ((row (gnosis-headless--row id)))
                              (list :thema-id id
                                    :prompt (when row
                                              (gnosis-headless--short
                                               (if (member (car row) '("cloze" "mc-cloze"))
                                                   (or (car (gnosis-headless--cloze-prompt row))
                                                       (format "Card %d" id))
                                                 (nth 1 row))
                                               70)))))
                          last))
         (remaining (length (gnosis-review-state-remaining state))))
    (list :summary summary :revisit (vconcat revisit) :remaining remaining
          :schedule-updated :false
          :summary-text
          (concat (format "%d answered · %d ✓ first try"
                          (plist-get summary :unique) (plist-get summary :first-success))
                  (let ((fixed (plist-get summary :retry-success)))
                    (if (> fixed 0) (format " · %d fixed on retry" fixed) ""))
                  (if revisit (format " · %d to revisit" (length revisit)) "")
                  (mapconcat (lambda (item) (format "\n- %s" (plist-get item :prompt)))
                             revisit "")
                  (if (> remaining 0) (format "\n%d left; resume here or at your desk." remaining) "")
                  "\nPractice only, schedule unchanged."))))

(defun gnosis-headless--done (state)
  "Return the completion payload for finished STATE."
  (let ((summary (gnosis-headless--summary (gnosis-review-state-session-id state))))
    (append (list :api-version 1 :status "done" :done t
                  :session-id (gnosis-review-state-session-id state))
            summary
            (list :text (concat "Done · " (plist-get summary :summary-text))))))

;;; Public API

(defun gnosis-headless--select (tags due weak exam thema-ids)
  "Return ordered candidate IDs for one of TAGS, DUE, WEAK, EXAM or THEMA-IDS."
  (cond
   (tags (sort (delete-dups (apply #'append (mapcar #'gnosis-get-tag-themata tags))) #'<))
   (due (gnosis-review-get-due-themata))
   (weak
    (mapcar #'car
            (gnosis-sqlite-select
             (gnosis--ensure-db)
             "SELECT s.thema_id FROM scheduler_state s
                LEFT JOIN (SELECT thema_id, COUNT(*) AS fails FROM practice_events
                            WHERE rating = 1 AND reviewed_at_us >= ?
                              AND event_id NOT IN (SELECT event_id FROM practice_voids)
                            GROUP BY thema_id) p ON p.thema_id = s.thema_id
               WHERE s.lapses > 0 OR COALESCE(p.fails, 0) > 0
               ORDER BY s.lapses + COALESCE(p.fails, 0) DESC,
                        COALESCE(s.stability, 0), s.thema_id"
             (list (- (car (time-convert nil 1000000)) (* 30 86400 1000000))))))
   (exam
    (require 'gnosis-exam)
    (let* ((record (or (seq-find (lambda (e) (equal exam (plist-get e :name)))
                                 (gnosis-exam-list))
                       (user-error "Unknown exam: %s" exam)))
           (coverage (gnosis-exam-exam-coverage record)))
      (delete-dups (append (gnosis-exam-cram-ids coverage)
                           (plist-get coverage :unseen)))))
   (t (delete-dups (copy-sequence thema-ids)))))

;;;###autoload
(cl-defun gnosis-headless-start (&key tags due weak exam thema-ids (limit 10) replace)
  "Start a headless practice batch and return its first question.
Supply exactly one selector: TAGS (list of tag strings, ascending IDs),
DUE (t: cards due today), WEAK (t: lapsed cards and recent practice
misses, worst first), EXAM (an exam name: due, at risk, then unseen) or
THEMA-IDS (integers, in order).  LIMIT is 1..50 text cards (default 10).

Only basic, cloze, mcq and mc-cloze cards without images are selected;
others are reported in :skipped.  Refuse while native review input runs,
and while another batch is unfinished unless REPLACE is non-nil, which
ends that batch early and keeps its accepted evidence.  Return a payload
with :status \"started\", :session-id, :selected, :skipped, :token,
:question and chat :text, or a refusal with :reason \"review-running\",
\"unfinished-batch\" or \"empty\".  Practice never changes FSRS."
  (unless (and (integerp limit) (<= 1 limit 50)
               (= 1 (seq-count #'identity (list tags due weak exam thema-ids)))
               (or (null tags) (and (proper-list-p tags)
                                    (seq-every-p (lambda (tag) (and (stringp tag)
                                                                    (not (string-empty-p tag))))
                                                 tags)))
               (or (null thema-ids) (and (proper-list-p thema-ids)
                                         (seq-every-p #'integerp thema-ids)))
               (or (null exam) (stringp exam)))
    (user-error "Supply one selector (:tags, :due, :weak, :exam or :thema-ids) and :limit 1..50"))
  (gnosis-headless--isolated
    (or
     (gnosis-headless--busy)
     (let* ((target (gnosis-review--session-target))
            (candidates (gnosis-headless--select tags due weak exam thema-ids))
            (rows (when candidates
                    (gnosis-sqlite-select-batch
                     (gnosis--ensure-db)
                     "SELECT t.id, s.suspended, s.due_day FROM themata t
                       JOIN scheduler_state s ON s.thema_id = t.id WHERE t.id IN (%s)"
                     candidates)))
            (eligible (gnosis-study--eligible-ids candidates rows nil))
            (text nil) (skipped nil))
       (dolist (id eligible)
         (let* ((row (gnosis-headless--row id))
                (reason (gnosis-headless--skip-reason row)))
           (if reason
               (push (list :thema-id id :kind (nth 0 row) :reason reason) skipped)
             (push id text))))
       (let* ((ids (seq-take (nreverse text) limit))
              (skipped (vconcat (nreverse skipped)))
              (selector (cond (tags (list :kind "tags" :tags (vconcat tags)))
                              (due (list :kind "due"))
                              (weak (list :kind "weak"))
                              (exam (list :kind "exam" :exam exam))
                              (t (list :kind "thema-ids"))))
              (current (and (cdr target) (gnosis-review--read-session))))
         (cond
          ((null ids)
           (gnosis-headless--refusal
            "empty" (format "Nothing to practise there%s."
                            (if (> (length skipped) 0)
                                (format " (%d cards need the desktop)" (length skipped))
                              ""))
            :skipped skipped))
          ((and current (not replace) (gnosis-review--work-p current)
                (not (gnosis-review-state-cancelled-p current)))
           (let ((left (length (gnosis-review-state-remaining current))))
             (gnosis-headless--refusal
              "unfinished-batch"
              (format "You have an unfinished batch (%d left).  End it early and start this one?"
                      left)
              :session-id (gnosis-review-state-session-id current)
              :remaining left
              :headless (if (gnosis-headless--current) t :false))))
          (t
           (let* ((selection (list :headless t :selector selector :limit limit
                                   :candidates (length candidates)
                                   :eligible (length eligible) :selected (length ids)
                                   :skipped-ids (vconcat (mapcar (lambda (s) (plist-get s :thema-id))
                                                                 skipped))))
                  (state (gnosis-review--reserve-batch
                          ids 'practice gnosis-headless-policy selection target))
                  (turn (gnosis-headless--present state)))
             (if (not (member (plist-get turn :status) '("question" "done")))
                 turn
             (append (list :api-version 1 :status "started"
                           :session-id (gnosis-review-state-session-id state)
                           :selected (length ids) :skipped skipped
                           :schedule-updated :false
                           :token (plist-get turn :token) :question (plist-get turn :question))
                     (list :text (concat
                                  (format "Practice · %d cards" (length ids))
                                  (if (> (length skipped) 0)
                                      (format " (%d skipped, need the desktop)" (length skipped))
                                    "")
                                  " · schedule unchanged\n\n"
                                  (plist-get turn :text)))))))))))))

;;;###autoload
(defun gnosis-headless-next (&optional session-id)
  "Return the current turn of the active headless batch.
With SESSION-ID, require that batch.  Return the open question (same
token), the pending verdict if an answer awaits acceptance, or the done
summary.  Use this to re-send a turn or to recover after a restart."
  (gnosis-headless--isolated
    (or (gnosis-headless--busy)
        (let ((state (gnosis-headless--current session-id)))
          (if (null state)
              (gnosis-headless--refusal
               "not-current" "No headless practice batch is active." :session-id session-id)
            (let* ((record (gnosis-headless--record (gnosis-review-state-session-id state)))
                   (pending (plist-get record :pending)))
              (if (and pending (equal (plist-get record :token) (gnosis-headless--token state)))
                  (plist-get pending :payload)
                (gnosis-headless--present state t))))))))

;;;###autoload
(defun gnosis-headless-answer (token response)
  "Grade RESPONSE to the question named by TOKEN, without counting it yet.
RESPONSE is the learner's text: typed answer for basic, exactly one answer
per blank separated by \";\" for cloze, a choice number or exact choice
text for mcq/mc-cloze.  Return :status \"pending\" with :correct,
:expected, :explanation and chat :text; then call `gnosis-headless-accept'.
The same RESPONSE again returns the same verdict with :duplicate t; a
different one is refused.  Invalid input (a bad choice or the wrong number
of blanks) returns :status \"invalid\" and consumes nothing.  A token for
an edited card is refused with reason \"changed\" and the new question."
  (unless (stringp response) (user-error "Response must be a string"))
  (gnosis-headless--isolated
    (or
     (gnosis-headless--busy)
     (let ((located (gnosis-headless--locate token)))
       (if (plist-get located :status) located
         (pcase-let* ((`(,state ,record) located)
                      (session-id (gnosis-review-state-session-id state))
                      (db (gnosis--ensure-db)))
           (unless (equal token (plist-get record :token))
             (setq located (gnosis-headless--present state t))
             (setq record (gnosis-headless--record session-id)))
           (if (not (equal token (plist-get record :token)))
               located
             (let ((pending (plist-get record :pending)))
               (cond
                ((and pending (equal response (plist-get pending :response)))
                 (append (list :duplicate t) (plist-get pending :payload)))
                (pending
                 (append (gnosis-headless--refusal
                          "already-answered"
                          "Already answered.  Reply o if you were right, or anything to go on.")
                         (list :token token :verdict (plist-get pending :payload))))
                (t
                 (let* ((id (car (gnosis-review-state-remaining state)))
                        (row (car (plist-get record :content)))
                        (graded (gnosis-headless--grade row response)))
                   (if (stringp graded)
                       (list :api-version 1 :status "invalid" :token token :text graded)
                     (let* ((success (nth 0 graded))
                            (result (let ((gnosis-review--state state))
                                      (gnosis-review--encounter
                                       (gnosis-review-algorithm id success)
                                       row (nth 1 graded) (plist-get record :hints)
                                       (nth 2 graded))))
                            (payload
                             (list :api-version 1 :status "pending" :token token
                                   :session-id session-id :thema-id id
                                   :correct (if success t :false)
                                   :expected (vconcat (mapcar #'gnosis-headless--plain
                                                              (nth 3 row)))
                                   :response response
                                   :explanation (gnosis-headless--short
                                                 (nth 5 row)
                                                 gnosis-headless-explanation-limit)
                                   :schedule-updated :false
                                   :text (gnosis-headless--verdict-text
                                          row success response))))
                       (let ((gnosis-review--state state))
                         (gnosis-review--emit 'answered id :success success :source "headless"))
                       (if (not (when-let* ((owner (gnosis-headless--owner db session-id)))
                                  (equal token (gnosis-headless--token owner))))
                           (gnosis-headless--replaced session-id)
                         (gnosis-headless--update session-id
                                                  :pending (list :response response
                                                                 :success success
                                                                 :result result
                                                                 :payload payload))
                         payload))))))))))))))

(defun gnosis-headless--accept (token override)
  "Accept TOKEN's pending answer, flipped when OVERRIDE; see the public API.
Capture the database, session and attempt before any subscriber runs.  If
a subscriber replaces the batch, return the committed acceptance without
presenting, caching or completing anything on the successor's behalf."
  (or
   (when-let* ((cached (gnosis-headless--cached token)))
     (append (list :duplicate t) cached))
   (let ((located (gnosis-headless--locate token)))
     (if (plist-get located :status) located
       (pcase-let* ((`(,state ,record) located)
                    (db (gnosis--ensure-db))
                    (session-id (gnosis-review-state-session-id state))
                    (event (gnosis-review-state-event-id state))
                    (pending (and (equal token (plist-get record :token))
                                  (plist-get record :pending))))
         (if (null pending)
             (gnosis-headless--refusal "no-answer" "Answer the question first." :token token)
           (let* ((id (car (gnosis-review-state-remaining state)))
                  (success (if override (not (plist-get pending :success))
                             (plist-get pending :success)))
                  (result (if override
                              (gnosis-review--override-result (plist-get pending :result) success)
                            (plist-get pending :result)))
                  (receipt (list :thema-id id
                                 :outcome (if success "success" "failure")
                                 :overridden (if override t :false)))
                  (counted (concat (if override
                                       (if success "Counted as correct." "Counted as wrong.")
                                     "Counted.")
                                   "\n\n")))
             (if (not (progn
                        (when override
                          (let ((gnosis-review--state state))
                            (gnosis-review--emit 'overridden id :success success
                                                 :source "headless")))
                        (when-let* ((owner (gnosis-headless--owner db session-id)))
                          (equal token (gnosis-headless--token owner)))))
                 ;; Nothing was written: the batch is no longer this call's.
                 (gnosis-headless--replaced session-id)
               (let ((gnosis-review--state state))
                 (gnosis-review-result id success result)
                 (gnosis-review--emit 'accepted id :success success :source "headless"))
               (let* ((fresh (gnosis-headless--owner db session-id))
                      (owned (and fresh (equal event (gnosis-review-state-last-event fresh))))
                      (turn (when owned (gnosis-headless--present fresh)))
                      (done (equal "done" (plist-get turn :status)))
                      (next (equal "question" (plist-get turn :status)))
                      (payload
                       (append
                        (list :api-version 1 :status "accepted" :session-id session-id
                              :accepted receipt
                              :done (if done t :false) :schedule-updated :false)
                        (cond (done (list :summary (plist-get turn :summary)
                                          :revisit (plist-get turn :revisit)))
                              (next (list :token (plist-get turn :token)
                                          :question (plist-get turn :question)))
                              (t (list :continuation
                                       (or (plist-get turn :reason) "replaced"))))
                        (list :text
                              (concat (if (or override (not turn)) counted "")
                                      (if turn (plist-get turn :text)
                                        "That practice batch was replaced; nothing more here."))))))
                 (when (eq db (gnosis--ensure-db))
                   (let ((record (gnosis-headless--record session-id)))
                     (gnosis-headless--update
                      session-id :pending nil
                      :accepted (seq-take (cons (cons token payload)
                                                (plist-get record :accepted))
                                          8))))
                 (when (and done (gnosis-headless--owner db session-id))
                   (when-let* ((event (gnosis-review--completion-event fresh)))
                     (gnosis-review--notify-completion event)))
                 payload)))))))))

;;;###autoload
(defun gnosis-headless-accept (token &optional override)
  "Count the pending answer to TOKEN as practice and return the next turn.
With OVERRIDE non-nil, flip the verdict first (\"I was right\" or \"I was
wrong\").  Record a practice event with encounter evidence; FSRS state is
never changed.  Return :status \"accepted\", :accepted (:thema-id :outcome
:overridden), :done, and either :token/:question for the next card,
:summary, or :continuation \"replaced\" when an event subscriber replaced
the batch.  Repeating the call returns the same payload with :duplicate t."
  (gnosis-headless--isolated
    (or (gnosis-headless--busy)
        (gnosis-headless--accept token override))))

;;;###autoload
(cl-defun gnosis-headless-end (session-id &key cancel discard-pending)
  "Stop driving headless SESSION-ID and return its summary.
A pending verdict is accepted first unless DISCARD-PENDING is non-nil.
Without CANCEL the batch stays resumable here or at the desk; with CANCEL
it ends early, keeping accepted evidence.  Return :status \"ended\",
:summary, :revisit, :remaining, :resumable and chat :text."
  (gnosis-headless--isolated
    (or
     (gnosis-headless--busy)
     (progn
       (gnosis-agent--session session-id)
       (when-let* ((state (gnosis-headless--current session-id))
                   (record (gethash session-id gnosis-headless--sessions))
                   ((plist-get record :pending))
                   ((not discard-pending))
                   ((equal (plist-get record :token) (gnosis-headless--token state))))
         (gnosis-headless--accept (plist-get record :token) nil))
       (when-let* ((state (gnosis-headless--current session-id))
                   (cancel)
                   ((gnosis-review--work-p state)))
         (gnosis-agent-cancel session-id))
       (remhash session-id gnosis-headless--sessions)
       (let* ((state (gnosis-agent--session session-id))
              (resumable (and (gnosis-headless--current session-id)
                              (gnosis-review--work-p state)))
              (summary (gnosis-headless--summary session-id)))
         (append (list :api-version 1 :status "ended" :session-id session-id
                       :resumable (if resumable t :false))
                 summary
                 (list :text (concat "Stopped · " (plist-get summary :summary-text)))))))))

(provide 'gnosis-headless)
;;; gnosis-headless.el ends here
