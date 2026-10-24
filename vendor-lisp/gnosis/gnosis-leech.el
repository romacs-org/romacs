;;; gnosis-leech.el --- Leech detection and tutor rework requests -*- lexical-binding: t; -*-

;; Copyright (C) 2026 Free Software Foundation, Inc.
;; Author: Thanos Apollo <public@thanosapollo.org>
;; SPDX-License-Identifier: GPL-3.0-or-later

;;; Commentary:

;; A leech is a card the learner keeps failing.  Grinding it wastes
;; reviews; fixing the item usually works better: split it, add a
;; mnemonic, or link the missing prerequisite.
;;
;; Detection is a pure rule over FSRS scheduler state
;; (`gnosis-leech-reasons').  `gnosis-leeches' is the agent query: plain
;; plists with the card content, its scheduler evidence, its content
;; revision, the learner's recent wrong answers (practice encounters, and
;; scheduled review encounters when that table exists) and the inbox
;; state of requests and proposals for it.
;;
;; Review feedback after a wrong answer that makes the card a leech shows
;; one line, \"⚑ leech · 5 lapses · w ask tutor to rework\".  The `w' key
;; appends a content-free request stub to the inbox (GNOSIS_STATE
;; requested); requests never appear as drafts to approve.
;;
;; The tutor (an agent) reads requests with `gnosis-leech-batch-report',
;; which opens the database read-only and never migrates it, or with
;; `gnosis-leeches' inside a running Emacs.  It proposes reworks through
;; `gnosis-leech-propose' or `gnosis-leech-batch-propose', which queue
;; native drafts with `gnosis-inbox-propose-rework' and close the
;; request.  The learner approves or rejects them in `gnosis-inbox'.
;; Agents never apply anything.

;;; Code:

(require 'cl-lib)
(require 'gnosis-db)
(require 'gnosis-sqlite)

(defvar gnosis-review--display-buffer)
(defvar gnosis-review--feedback)
(defvar gnosis-review-buffer-name)
(defvar gnosis-review-feedback-mode)
(declare-function gnosis-review--append-section "gnosis-review")
(declare-function gnosis-review--display-validator "gnosis-review")
(declare-function gnosis-review--feedback-check "gnosis-review")
(declare-function gnosis-confusion-note "gnosis-confusion")
(declare-function gnosis-get-tags-for-ids "gnosis-tags")
(declare-function gnosis-inbox-file "gnosis-inbox")
(declare-function gnosis-inbox-parse "gnosis-inbox")
(declare-function gnosis-inbox-propose-rework "gnosis-inbox")
(declare-function gnosis-inbox--read "gnosis-inbox")
(declare-function gnosis-inbox--read-input "gnosis-inbox")
(declare-function gnosis-inbox--hash "gnosis-inbox")
(declare-function gnosis-inbox--write "gnosis-inbox")
(declare-function gnosis-inbox--preamble "gnosis-inbox")
(declare-function gnosis-inbox--replace-entry "gnosis-inbox")
(declare-function gnosis-inbox--ledger-text "gnosis-inbox")
(declare-function gnosis-inbox--revision "gnosis-inbox")
(declare-function gnosis-inbox--draft-text "gnosis-inbox")
(declare-function gnosis-inbox--thema-draft "gnosis-inbox")
(declare-function gnosis-inbox--one-line "gnosis-inbox")

(defgroup gnosis-leech nil
  "Leech detection and tutor rework requests."
  :group 'gnosis)

(defcustom gnosis-leech-lapses 4
  "Lapses at which a card is a leech."
  :type 'natnum)

(defcustom gnosis-leech-hard-lapses 3
  "Lapses at which a card with high difficulty is a leech.
See `gnosis-leech-difficulty'."
  :type 'natnum)

(defcustom gnosis-leech-difficulty 8.0
  "FSRS difficulty (1-10) that makes `gnosis-leech-hard-lapses' enough."
  :type 'number)

(defcustom gnosis-leech-stuck-reps 8
  "Reviews after which a low stability marks a leech.
See `gnosis-leech-stuck-stability'."
  :type 'natnum)

(defcustom gnosis-leech-stuck-stability 2.0
  "FSRS stability in days below which a much-reviewed card is a leech."
  :type 'number)

(defcustom gnosis-leech-recent-limit 10
  "Maximum number of recent wrong answers reported per leech."
  :type 'natnum)

(defface gnosis-leech-note '((t :inherit shadow))
  "Face of the leech line in review feedback.")

(defface gnosis-leech-mark '((t :inherit warning))
  "Face of the leech flag in review feedback.")

;;; The rule

(defun gnosis-leech-reasons (state)
  "Return why scheduler STATE is a leech, as a list of symbols, or nil.
STATE is a plist with :lapses, :reps, :difficulty and :stability.  The
reasons are `lapses' (at least `gnosis-leech-lapses'), `difficulty'
\(at least `gnosis-leech-hard-lapses' and difficulty at least
`gnosis-leech-difficulty') and `stability' (at least
`gnosis-leech-stuck-reps' reviews and stability below
`gnosis-leech-stuck-stability' days).  Pure."
  (let ((lapses (or (plist-get state :lapses) 0))
        (reps (or (plist-get state :reps) 0))
        (difficulty (plist-get state :difficulty))
        (stability (plist-get state :stability)))
    (delq nil
          (list (and (>= lapses gnosis-leech-lapses) 'lapses)
                (and (>= lapses gnosis-leech-hard-lapses)
                     (numberp difficulty) (>= difficulty gnosis-leech-difficulty)
                     'difficulty)
                (and (>= reps gnosis-leech-stuck-reps)
                     (numberp stability) (< stability gnosis-leech-stuck-stability)
                     'stability)))))

(defun gnosis-leech-p (state)
  "Return non-nil if scheduler STATE plist is a leech; see `gnosis-leech-reasons'."
  (and (gnosis-leech-reasons state) t))

(defun gnosis-leech--summary (state)
  "Return a short evidence line for scheduler STATE."
  (let ((lapses (or (plist-get state :lapses) 0))
        (difficulty (plist-get state :difficulty)))
    (concat (format "%d lapse%s" lapses (if (= lapses 1) "" "s"))
            (if (and (numberp difficulty) (>= difficulty gnosis-leech-difficulty))
                (format " · difficulty %.1f" difficulty)
              ""))))

;;; Reading evidence

(defun gnosis-leech--db (db)
  "Return DB or the current Gnosis connection."
  (or db (gnosis--ensure-db)))

(defun gnosis-leech--state-plist (row)
  "Return a state plist from scheduler ROW (ID LAPSES REPS STAB DIFF SUSPENDED)."
  (pcase-let ((`(,id ,lapses ,reps ,stability ,difficulty ,suspended) row))
    (list :id id :lapses lapses :reps reps :stability stability
          :difficulty difficulty :suspended (eql suspended 1))))

(defun gnosis-leech-state (id &optional db)
  "Return the scheduler state plist of thema ID on DB, or nil."
  (when-let* ((row (car (gnosis-sqlite-select
                         (gnosis-leech--db db)
                         "SELECT thema_id, lapses, reps, stability, difficulty, suspended
                            FROM scheduler_state WHERE thema_id = ?"
                         (list id)))))
    (gnosis-leech--state-plist row)))

(defun gnosis-leech--table-p (db table)
  "Return non-nil if TABLE exists in DB."
  ;; Raw binding: the Gnosis encoder would quote the name.
  (sqlite-select db "SELECT 1 FROM sqlite_master WHERE type = 'table' AND name = ?"
                 (list table)))

(defun gnosis-leech--response-text (response)
  "Return learner RESPONSE plist as display text, or nil."
  (let ((text (pcase (plist-get response :kind)
                ((or "text" "agent-eval") (plist-get response :text))
                ("choice" (plist-get response :selected))
                ("blanks" (mapconcat (lambda (input) (format "%s" input))
                                     (plist-get response :inputs) " · "))
                ((or "surface" "region") (plist-get response :selected-target))
                ("self-grade" "not recalled"))))
    (when (and text (not (string-empty-p (string-trim (format "%s" text)))))
      (format "%s" text))))

(defun gnosis-leech-recent-wrong (id &optional limit db)
  "Return up to LIMIT recent wrong answers to thema ID on DB, newest first.
LIMIT defaults to `gnosis-leech-recent-limit'.  Each is a plist with
:source (`practice' or `scheduled'), :at-us and :response text.  Read
effective failed practice events with captured encounters, and failed
scheduled reviews from `review_encounters' only when that table exists.
Never writes."
  (let* ((db (gnosis-leech--db db))
         (limit (or limit gnosis-leech-recent-limit))
         (sources '((practice "practice_events" "practice_encounters" "practice_voids")
                    (scheduled "review_events" "review_encounters" "review_voids")))
         rows)
    (pcase-dolist (`(,source ,events ,encounters ,voids) sources)
      (when (gnosis-leech--table-p db encounters)
        (pcase-dolist (`(,at ,data)
                       (gnosis-sqlite-select
                        db (format "SELECT e.reviewed_at_us, c.data FROM %s e
                                      JOIN %s c ON c.event_id = e.event_id
                                      LEFT JOIN %s v ON v.event_id = e.event_id
                                     WHERE e.thema_id = ? AND e.rating = 1
                                       AND v.event_id IS NULL
                                     ORDER BY e.reviewed_at_us DESC LIMIT ?"
                                   events encounters voids)
                        (list id limit)))
          (when-let* ((text (gnosis-leech--response-text
                             (and (proper-list-p data) (plist-get data :response)))))
            (push (list :source source :at-us at :response text) rows)))))
    (seq-take (sort rows (lambda (a b) (> (plist-get a :at-us) (plist-get b :at-us))))
              limit)))

;;; Inbox state

(defun gnosis-leech--entries ()
  "Return parsed inbox entries."
  (require 'gnosis-inbox)
  (gnosis-inbox-parse (gnosis-inbox--read (gnosis-inbox-file))))

(defun gnosis-leech--target-entries (id entries)
  "Return ENTRIES whose rework target is ID."
  (seq-filter (lambda (entry) (eql (plist-get entry :target) id)) entries))

(defun gnosis-leech--request-entry (id entries)
  "Return the open request for ID among ENTRIES, or nil."
  (seq-find (lambda (entry) (eq (plist-get entry :state) 'requested))
            (gnosis-leech--target-entries id entries)))

(defun gnosis-leech--proposals (id entries)
  "Return the number of pending rework drafts for ID among ENTRIES."
  (cl-count-if (lambda (entry) (eq (plist-get entry :state) 'pending))
               (gnosis-leech--target-entries id entries)))

(defun gnosis-leech--status (id &optional entries)
  "Return `requested', `proposed' or nil for thema ID's inbox state.
Read inbox ENTRIES, defaulting to the parsed inbox file."
  (let ((entries (or entries (gnosis-leech--entries))))
    (cond ((> (gnosis-leech--proposals id entries) 0) 'proposed)
          ((gnosis-leech--request-entry id entries) 'requested))))

;;; Agent query

(defun gnosis-leech--content (id db)
  "Return the content plist of thema ID on DB, or nil."
  (when-let* ((row (car (gnosis-sqlite-select
                         db "SELECT type, keimenon, hypothesis, answer, accepted_aliases, rubric
                               FROM themata WHERE id = ?"
                         (list id)))))
    (pcase-let ((`(,type ,keimenon ,hypothesis ,answer ,aliases ,rubric) row))
      (list :type (downcase type) :keimenon keimenon
            :hypothesis (remove "" hypothesis) :answer answer
            :aliases aliases :rubric rubric
            :parathema (or (caar (gnosis-sqlite-select
                                  db "SELECT parathema FROM extras WHERE id = ?" (list id)))
                           "")
            :tags (mapcar #'car (gnosis-sqlite-select
                                 db "SELECT tag FROM thema_tag WHERE thema_id = ? ORDER BY tag"
                                 (list id)))))))

(defun gnosis-leech-annotation-draft (id &optional addition)
  "Return a draft plist of thema ID's current content, ADDITION appended.
ADDITION is appended to the explanation (parathema) after a blank line.
The draft carries ID's current :target-revision.
Use it with `gnosis-leech-propose' and kind \"rework-annotate\" for a
mnemonic or a prerequisite link: approval appends the text to ID and
keeps its schedule and history."
  (require 'gnosis-inbox)
  (let* ((content (or (gnosis-leech--content id (gnosis--ensure-db))
                      (user-error "No thema %s" id)))
         (old (string-trim (plist-get content :parathema))))
    (when addition
      (setq content (plist-put content :parathema
                               (if (string-empty-p old) (string-trim addition)
                                 (concat old "\n\n" (string-trim addition))))))
    (append content (list :target-revision (gnosis-inbox--revision id)))))

(defun gnosis-leech--draft-text (id content revision)
  "Return native Org text of CONTENT for ID at REVISION, ready for editing.
It carries rework provenance except GNOSIS_AGENT and GNOSIS_REASON."
  (require 'gnosis-inbox)
  (gnosis-inbox--draft-text
   (append (list :kind "rework-annotate" :source (format "thema:%d" id)
                 :target id :target-revision revision)
           content)))

(defun gnosis-leech--rejected (id entries)
  "Return rejected proposals for ID among ENTRIES as (:kind :reason) plists."
  (cl-loop for entry in (gnosis-leech--target-entries id entries)
           when (eq (plist-get entry :state) 'rejected)
           collect (list :kind (plist-get entry :kind)
                         :reason (plist-get entry :reject-reason))))

(defun gnosis-leech--describe (state db entries)
  "Return the agent plist for scheduler STATE on DB with inbox ENTRIES."
  (let* ((id (plist-get state :id))
         (content (gnosis-leech--content id db))
         (revision (let ((gnosis-db db))
                     (require 'gnosis-inbox)
                     (gnosis-inbox--revision id)))
         (request (gnosis-leech--request-entry id entries))
         (draft (condition-case err
                    (cons (gnosis-leech--draft-text id content revision) nil)
                  (error (cons nil (error-message-string err))))))
    (append (list :id id)
            content
            (list :lapses (plist-get state :lapses) :reps (plist-get state :reps)
                  :difficulty (plist-get state :difficulty)
                  :stability (plist-get state :stability)
                  :suspended (plist-get state :suspended)
                  :reasons (gnosis-leech-reasons state)
                  :revision revision
                  :recent-wrong (gnosis-leech-recent-wrong id nil db)
                  :request (and request
                                (list :state 'requested :reason (plist-get request :reason)
                                      :created (plist-get request :created)))
                  :proposals (gnosis-leech--proposals id entries)
                  :rejected (gnosis-leech--rejected id entries)
                  :draft (car draft)
                  :draft-error (cdr draft)))))

(cl-defun gnosis-leeches (&key ids include-suspended all db)
  "Return leeches as plists, most lapses first.  Never writes.
Each plist has :id, the content (:type :keimenon :hypothesis :answer
:aliases :rubric :parathema :tags), scheduler evidence (:lapses :reps
:difficulty :stability :suspended :reasons), :revision (the content
revision rework proposals must carry), :recent-wrong (see
`gnosis-leech-recent-wrong'), inbox state (:request, :proposals, the
number of pending rework drafts, and :rejected, earlier rejected
proposals with their reasons) and :draft, the card as native Org text
with rework provenance, for the tutor to edit into a proposal, or nil
with the reason in :draft-error when the card cannot be written as Org.
IDS restricts the query.  INCLUDE-SUSPENDED keeps suspended cards.  ALL
returns IDS even when they are not leeches.  DB defaults to the current
connection."
  (let* ((db (gnosis-leech--db db))
         (entries (gnosis-leech--entries))
         (rows (gnosis-sqlite-select
                db "SELECT thema_id, lapses, reps, stability, difficulty, suspended
                      FROM scheduler_state
                     ORDER BY lapses DESC, difficulty DESC, thema_id")))
    (cl-loop for row in rows
             for state = (gnosis-leech--state-plist row)
             when (and (or (null ids) (memql (plist-get state :id) ids))
                       (or include-suspended (not (plist-get state :suspended)))
                       (or (and all ids) (gnosis-leech-p state)))
             collect (gnosis-leech--describe state db entries))))

;;; Requests

(defun gnosis-leech-requests ()
  "Return open rework requests from the inbox, oldest first.
Each is a plist with :target, :revision, :reason, :created and :key.
Never opens the database."
  (cl-loop for entry in (gnosis-leech--entries)
           when (eq (plist-get entry :state) 'requested)
           collect (list :target (plist-get entry :target)
                         :revision (plist-get entry :target-revision)
                         :reason (plist-get entry :reason)
                         :created (plist-get entry :created)
                         :key (plist-get entry :key))))

(defun gnosis-leech--request-text (id revision reason)
  "Return the inbox request stub for thema ID at REVISION with REASON."
  (let ((created (format-time-string "%FT%T%z")))
    (concat (format "* Rework request #%d\n:PROPERTIES:\n" id)
            ":GNOSIS_STATE: requested\n"
            ":GNOSIS_PROPOSAL: leech\n"
            (format ":GNOSIS_SOURCE: thema:%d\n" id)
            (format ":GNOSIS_TARGET: %d\n" id)
            (format ":GNOSIS_TARGET_REVISION: %s\n" revision)
            ":GNOSIS_AGENT: learner\n"
            (format ":GNOSIS_REASON: %s\n" (or (gnosis-inbox--one-line reason) "leech"))
            (format ":GNOSIS_CREATED: %s\n" created)
            (format ":GNOSIS_KEY: %s\n"
                    (secure-hash 'sha1 (format "gnosis-leech-request\0%d\0%s\0%s"
                                               id revision created)))
            ":END:\n")))

(defun gnosis-leech-request (id &optional reason)
  "Ask the tutor to rework thema ID, recording REASON.
Append a content-free request stub to the inbox unless ID already has an
open request or pending rework drafts.  Return (:status STATUS) where
STATUS is `added' or `exists'.  Writes only the inbox file."
  (require 'gnosis-inbox)
  (unless (gnosis-sqlite-select (gnosis--ensure-db) "SELECT 1 FROM themata WHERE id = ?"
                                (list id))
    (user-error "No thema %s" id))
  (let* ((file (gnosis-inbox-file))
         (text (gnosis-inbox--read file))
         (entries (gnosis-inbox-parse text)))
    (if (gnosis-leech--status id entries)
        (list :status 'exists)
      (gnosis-inbox--write
       file (gnosis-inbox--hash text)
       (concat (if (string-empty-p text) (gnosis-inbox--preamble)
                 (concat (string-trim-right text) "\n"))
               "\n" (gnosis-leech--request-text id (gnosis-inbox--revision id) reason)))
      (list :status 'added))))

(defun gnosis-leech--close-requests (id state)
  "Close every open request for thema ID with ledger STATE."
  (while-let ((text (gnosis-inbox--read (gnosis-inbox-file)))
              (entry (gnosis-leech--request-entry id (gnosis-inbox-parse text))))
    (gnosis-inbox--replace-entry (gnosis-inbox-file) (gnosis-inbox--hash text) entry
                                 (gnosis-inbox--ledger-text entry state))))

(cl-defun gnosis-leech-propose (target drafts &key (kind "rework-split") agent reason)
  "Propose rework DRAFTS for leech TARGET and close its open request.
KIND is \"rework-split\" (DRAFTS replace TARGET, which is suspended on
approval), \"rework-replace\" (one draft) or \"rework-annotate\" (one
draft from `gnosis-leech-annotation-draft': its explanation is appended
to TARGET, whose schedule is kept).  AGENT and REASON are provenance.
Return `gnosis-inbox-propose-rework' results.

Every draft must carry the :target-revision of the content it was
written against (`gnosis-leech-annotation-draft' and the batch report's
drafts do).  If TARGET changed since, signal `user-error' and leave the
inbox and its requests unchanged.  A draft's default identity covers
KIND, question, answers and explanation, so distinct repairs of one
revision are distinct proposals.  The request closes only when a
proposal for TARGET is now pending or was applied; a draft that merely
repeats an earlier rejected or closed proposal leaves it open."
  (require 'gnosis-inbox)
  (dolist (draft drafts)
    (unless (plist-get draft :target-revision)
      (user-error "Rework drafts for #%s need the :target-revision they were written against"
                  target)))
  (let* ((results (gnosis-inbox-propose-rework
                   target
                   (mapcar (lambda (draft)
                             (if (plist-get draft :span) draft
                               (append (list :span (mapconcat
                                                    #'identity
                                                    (append (list kind (plist-get draft :keimenon))
                                                            (plist-get draft :answer)
                                                            (list (or (plist-get draft :parathema)
                                                                      "")))
                                                    "\n"))
                                       draft)))
                           drafts)
                   :kind kind :agent agent :reason reason))
         (entries (gnosis-leech--entries)))
    (when (cl-find-if
           (lambda (result)
             (when-let* ((entry (seq-find (lambda (e) (equal (plist-get e :key)
                                                             (plist-get result :key)))
                                          entries)))
               (and (memq (plist-get result :status) '(added duplicate))
                    (eql (plist-get entry :target) target)
                    (memq (plist-get entry :state) '(pending applied)))))
           results)
      (gnosis-leech--close-requests target 'proposed))
    results))

;;; Review feedback

(defvar-local gnosis-leech--shown nil
  "Thema ID whose leech line this review buffer shows, or nil.")

(defun gnosis-leech--projected (id result)
  "Return thema ID's scheduler state after pending RESULT is accepted."
  (if-let* ((preview (plist-get result :preview)))
      (list :lapses (plist-get preview :lapses-after)
            :reps (plist-get preview :reps-after)
            :difficulty (plist-get preview :difficulty)
            :stability (plist-get preview :stability))
    ;; Practice never changes the schedule.
    (gnosis-leech-state id)))

(defun gnosis-leech-note (id result)
  "Return the feedback line for wrong pending RESULT of thema ID, or nil.
Show it only for collection themata whose state after this answer is a
leech, and not when a confusion note applies (that line takes the slot)."
  (unless (plist-get result :local-owner)
    (let ((state (gnosis-leech--projected id result)))
      (when (and (gnosis-leech-p state)
                 (not (and (fboundp 'gnosis-confusion-note)
                           (gnosis-confusion-note result))))
        (concat (propertize "⚑ leech" 'face 'gnosis-leech-mark)
                (propertize
                 (concat " · " (gnosis-leech--summary state) " · "
                         (pcase (gnosis-leech--status id)
                           ('proposed "rework waiting in inbox")
                           ('requested "rework requested")
                           (_ "w ask tutor to rework")))
                 'face 'gnosis-leech-note))))))

(defun gnosis-leech-display (id result)
  "Append the leech line for wrong pending RESULT of thema ID to feedback.
Errors while reading evidence are reported and never interrupt review."
  (when-let* ((note (condition-case err (gnosis-leech-note id result)
                      (error (message "Gnosis leech note unavailable: %s"
                                      (error-message-string err))
                             nil))))
    (with-current-buffer (or gnosis-review--display-buffer gnosis-review-buffer-name)
      (setq gnosis-leech--shown id)
      (gnosis-review--append-section (propertize note 'gnosis-leech-note t)
                                     (gnosis-review--display-validator) "\n\n"))))

(defun gnosis-leech--note-region ()
  "Return (START . END) of the leech line in the current buffer, or nil."
  (when-let* ((start (text-property-any (point-min) (point-max) 'gnosis-leech-note t)))
    (cons start (or (next-single-property-change start 'gnosis-leech-note) (point-max)))))

;;;###autoload
(defun gnosis-leech-feedback-p ()
  "Return non-nil when a leech line is in the current feedback."
  (and (bound-and-true-p gnosis-review--feedback)
       gnosis-leech--shown
       (eql gnosis-leech--shown (plist-get gnosis-review--feedback :id))
       (gnosis-leech--note-region)
       t))

;;;###autoload
(defun gnosis-leech-request-rework ()
  "Ask the tutor to rework the leech shown in this review feedback.
The pending answer is unchanged; continue reviewing as usual."
  (interactive nil gnosis-review-feedback-mode)
  (let* ((context (gnosis-review--feedback-check))
         (id (plist-get context :id))
         (result (plist-get context :result)))
    (when (plist-get result :local-owner)
      (user-error "Session-local question; save it before asking for a rework"))
    (unless (gnosis-leech-feedback-p)
      (user-error "This card is not a leech"))
    (let* ((state (gnosis-leech--projected id result))
           (status (plist-get (gnosis-leech-request id (gnosis-leech--summary state))
                              :status)))
      (when-let* ((region (gnosis-leech--note-region))
                  (note (gnosis-leech-note id result)))
        (let ((inhibit-read-only t))
          (save-excursion
            (goto-char (car region))
            (delete-region (car region) (cdr region))
            (insert (propertize note 'gnosis-leech-note t)))))
      (message (if (eq status 'added)
                   "Rework requested; the tutor's proposal will wait in M-x gnosis-inbox"
                 "Rework already requested")))))

;;; Tutor template

(defconst gnosis-leech-tutor-instructions
  "You are the learner's tutor.  Each card below is a leech: the learner keeps
failing it.  Fix the item, do not grind it.  For each card choose at most
one proposal, and propose nothing when the card is sound:

- rework-split: the card tests two or more facts, or a long cloze hides
  several answers.  Write 2-4 smaller cards, one fact each, that together
  cover the original.  Approval suspends the original.
- rework-annotate (mnemonic): the fact is atomic but arbitrary.  Append a
  short mnemonic or memorable explanation to Parathema.  Keep every other
  field byte for byte.  Approval keeps the card's schedule and history.
- rework-annotate (prerequisite): the recent wrong answers show a missing
  concept.  Append one line naming the prerequisite, with an [[id:...]]
  link to an existing note when one exists.
- rework-replace: the wording is ambiguous or the answer is wrong.  Write
  one corrected card.  Approval suspends the original.

Use the recent wrong answers: they show what the learner confuses.  Never
re-propose a kind listed under rejected.  Be medically correct and cite
sources in Parathema where you add facts.

Output: an Org file of level-1 native themata.  Start from each card's
:draft, which already carries GNOSIS_PROPOSAL, GNOSIS_SOURCE, GNOSIS_TARGET
and GNOSIS_TARGET_REVISION.  Set GNOSIS_PROPOSAL to your kind, add
GNOSIS_AGENT (for example hermes/tutor) and GNOSIS_REASON (one line), and
keep GNOSIS_ID NEW.  For a split, copy the drafted heading once per new
card.  Queue it with:
  GNOSIS_DIR=DIR emacs -Q --batch -L lisp -L keymap-popup -l gnosis-leech \\
    -f gnosis-leech-batch-propose DRAFTS.org
The learner approves or rejects each proposal in M-x gnosis-inbox."
  "Instructions given to the tutor agent with the leech report.")

;;; Batch entry points

(defun gnosis-leech--open-read-only (dir)
  "Open DIR's existing database read-only, refusing unsupported versions.
Never creates, initializes or migrates a database."
  (let ((file (expand-file-name "gnosis.db" dir)))
    (unless (file-regular-p file)
      (user-error "No Gnosis database in %s" dir))
    (let ((db (sqlite-open file)) ok)
      (unwind-protect
          (let ((version (caar (sqlite-select db "PRAGMA user_version"))))
            (unless (and (integerp version)
                         (<= gnosis-db-min-version version gnosis-db-version))
              (user-error "Unsupported Gnosis database version %s" version))
            (sqlite-execute db "PRAGMA query_only = ON")
            (setq ok t)
            db)
        (unless ok (sqlite-close db))))))

(defun gnosis-leech--json-value (value)
  "Return VALUE converted for `json-serialize'."
  (cond ((null value) :null)
        ((eq value t) t)
        ((keywordp value) (substring (symbol-name value) 1))
        ((symbolp value) (symbol-name value))
        ((and (consp value) (keywordp (car value)))
         (cl-loop for (key item) on value by #'cddr
                  append (list key (gnosis-leech--json-value item))))
        ((or (consp value) (vectorp value))
         (vconcat (mapcar #'gnosis-leech--json-value value)))
        (t value)))

(defun gnosis-leech--batch-print (plist)
  "Print PLIST as one JSON object of UTF-8 text on standard output."
  (let ((coding-system-for-write 'utf-8-unix))
    (princ (decode-coding-string
            (json-serialize (gnosis-leech--json-value plist)
                            :null-object :null :false-object :false)
            'utf-8))
    (terpri)))

(defmacro gnosis-leech--batch (&rest body)
  "Run BODY as a batch entry point, printing its plist as JSON.
BODY sees `gnosis-dir' from GNOSIS_DIR and returns (EXIT . PLIST).  On a
usage, read or write error print the error and exit 2."
  (declare (indent 0) (debug t))
  `(progn
     (unless noninteractive (user-error "Batch entry point; use the Lisp API interactively"))
     (let* ((dir (getenv "GNOSIS_DIR"))
            (gnosis-dir (and dir (not (string-empty-p dir)) (file-name-as-directory dir)))
            (gnosis-db nil)
            (inbox (and gnosis-dir (progn (require 'gnosis-inbox) (gnosis-inbox-file)))))
       (condition-case err
           (pcase-let ((`(,exit . ,plist)
                        (progn
                          (unless gnosis-dir
                            (user-error "Set GNOSIS_DIR to the Gnosis directory"))
                          (setq gnosis-db (gnosis-leech--open-read-only gnosis-dir))
                          (unwind-protect (progn ,@body)
                            (sqlite-close gnosis-db)))))
             (gnosis-leech--batch-print (append (list :inbox inbox) plist))
             (kill-emacs exit))
         (error
          (gnosis-leech--batch-print (list :inbox inbox :error (error-message-string err)))
          (kill-emacs 2))))))

(defun gnosis-leech-batch-report ()
  "Batch entry point: print open requests and leeches as JSON for the tutor.
Run Emacs in batch mode with this package and keymap-popup on the load
path and GNOSIS_DIR naming the Gnosis directory.  The database is opened
read-only and never created or migrated.  The JSON object has
\"instructions\" (`gnosis-leech-tutor-instructions'), \"requests\"
\(requested cards, leech or not) and \"leeches\" (all unsuspended
leeches), each card as described in `gnosis-leeches'.  Exit 0, or 2 with
\"error\" on a usage or read failure."
  (setq command-line-args-left nil)
  (gnosis-leech--batch
    (let ((targets (mapcar (lambda (r) (plist-get r :target)) (gnosis-leech-requests))))
      (cons 0 (list :instructions gnosis-leech-tutor-instructions
                    :requests (and targets (gnosis-leeches :ids targets :all t
                                                           :include-suspended t
                                                           :db gnosis-db))
                    :leeches (gnosis-leeches :db gnosis-db))))))

(defun gnosis-leech--entry-draft (entry)
  "Return the draft plist of parsed pending inbox-format ENTRY."
  (gnosis-inbox--thema-draft (plist-get entry :thema)
                             (list :span (plist-get entry :span)
                                   :target-revision (plist-get entry :target-revision))))

(defun gnosis-leech-batch-propose ()
  "Batch entry point: queue the tutor's rework drafts from DRAFTS.org.
DRAFTS.org, the only command-line argument, holds level-1 native themata
with rework provenance (see `gnosis-leech-tutor-instructions').  A draft
whose GNOSIS_TARGET_REVISION no longer matches the card is refused, so a
proposal never applies to content the tutor did not see.  Each target's
open request closes once a draft is queued.  Print JSON with \"results\";
exit 0, 1 when any draft is invalid, or 2 on a usage or read failure."
  (let ((args command-line-args-left))
    (setq command-line-args-left nil)
    (gnosis-leech--batch
      (unless (= (length args) 1)
        (user-error "Usage: Emacs --batch ... -f gnosis-leech-batch-propose DRAFTS.org"))
      (let* ((entries (gnosis-inbox-parse
                       (gnosis-inbox--read-input (expand-file-name (car args)))))
             results)
        (unless entries (user-error "No drafts in %s" (car args)))
        (dolist (entry entries)
          (let ((target (plist-get entry :target)))
            (push
             (cond
              ((not (plist-get entry :thema))
               (list :status 'invalid :target target
                     :error (or (plist-get entry :error) "Not a pending draft")))
              ((not (string-prefix-p "rework-" (plist-get entry :kind)))
               (list :status 'invalid :target target :error "Not a rework draft"))
              ((not (gnosis-leech--content target gnosis-db))
               (list :status 'invalid :target target :error (format "No thema %s" target)))
              ((not (equal (plist-get entry :target-revision) (gnosis-inbox--revision target)))
               (list :status 'invalid :target target
                     :error "Card changed since the report; report again"))
              (t
               (let ((result (car (gnosis-leech-propose
                                   target (list (gnosis-leech--entry-draft entry))
                                   :kind (plist-get entry :kind)
                                   :agent (plist-get entry :agent)
                                   :reason (plist-get entry :reason)))))
                 (append (list :target target) result))))
             results)))
        (setq results (nreverse results))
        (cons (if (cl-find 'invalid results :key (lambda (r) (plist-get r :status))) 1 0)
              (list :results results))))))

(provide 'gnosis-leech)
;;; gnosis-leech.el ends here
