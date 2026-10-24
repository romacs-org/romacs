;;; gnosis-feed.el --- Flow-paced review bursts  -*- lexical-binding: t; -*-

;; Copyright (C) 2026 Free Software Foundation, Inc.

;; This file is part of Gnosis.

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

;; `gnosis-feed' reviews due themata in short bursts with a quiet rhythm:
;; answer, see, SPC.  Each burst is one ordinary finite scheduled batch run
;; by `gnosis-review-loop', so grading, acceptance, overrides, retries and
;; ownership checks are exactly those of classic review.  The feed changes
;; only presentation, through the seams in `gnosis-review':
;;
;; - the action menu opens on request (? or h), not after every answer;
;; - one verdict line shows the answer and the current outcome and date,
;;   and repaints in place after an override;
;; - the header shows progress dots, a streak from five, and a dwell bar;
;; - correct answers advance after a short dwell through one ordinary input
;;   event, never from a timer-owned reader; any other command revokes it;
;; - SPC and n ignore a reflexive double tap just after a wrong answer;
;; - the burst ends on a one-line summary offering another burst.

;;; Code:

(require 'cl-lib)
(require 'subr-x)
(require 'face-remap)
(require 'gnosis-review)

(defvar gnosis-monkeytype-enable)

(defgroup gnosis-feed nil
  "Flow-paced review bursts."
  :group 'gnosis)

(defcustom gnosis-feed-burst-size 10
  "Number of themata in one feed burst."
  :type 'natnum)

(defcustom gnosis-feed-correct-dwell 1.5
  "Seconds a correct answer stays before advancing, or nil to wait for SPC.
The feed waits for SPC anyway when this Emacs does not wake a waiting
reader for input queued by a timer."
  :type '(choice (number :tag "Seconds") (const :tag "Never" nil)))

(defcustom gnosis-feed-wrong-guard 0.5
  "Seconds after a wrong answer during which SPC and n do nothing."
  :type 'number)

(defcustom gnosis-feed-new-every 4
  "Interleave one new thema after this many due reviews, or nil for none.
New themata stay within `gnosis-new-themata-limit'."
  :type '(choice natnum (const nil)))

(defcustom gnosis-feed-streak-threshold 5
  "Smallest run of first-try correct answers shown in the header."
  :type 'natnum)

(defcustom gnosis-feed-flash-duration 0.18
  "Seconds the header flashes after a correct answer, or nil for none."
  :type '(choice number (const nil)))

(defcustom gnosis-feed-monkeytype nil
  "Non-nil keeps `gnosis-monkeytype-enable' in the feed.
Typing drills after wrong answers break the answer, see, SPC rhythm, so the
feed turns them off unless this is non-nil."
  :type 'boolean)

(defcustom gnosis-feed-hint-presentations 3
  "Presentations of a card kind per day after which its key hint fades."
  :type 'natnum)

(defface gnosis-feed-correct
  '((t :inherit success :weight bold))
  "Face for a correct answer on the verdict line.")

(defface gnosis-feed-wrong
  '((t :inherit error :weight bold))
  "Face for a wrong mark and the learner's response.")

(defface gnosis-feed-answer
  '((t :weight bold))
  "Face for the correct answer after a wrong response.")

(defface gnosis-feed-fixed
  '((t :inherit warning))
  "Face for retries fixed in this burst and overridden outcomes.")

(defface gnosis-feed-dim
  '((t :inherit shadow))
  "Face for schedules, pending dots and hints.")

(defface gnosis-feed-flash
  '((((background dark)) :background "#1d4d32")
    (t :background "#c9ebd2"))
  "Face remapped onto the header line briefly after a correct answer.")

(defvar gnosis-feed-burst-functions nil
  "Abnormal hook run with one plist when a feed burst ends.
The plist holds :api 1, :session-id, :size (planned), :reviewed,
:correct, :fixed, :wrong, :overridden, :seconds and :finished.  It never
contains thema text.  Errors and quits in a function are reported and
contained.")

;;; Burst state

(cl-defstruct (gnosis-feed--burst (:constructor gnosis-feed--burst-create))
  "One feed burst; display state only, never review authority."
  ids                 ; planned thema IDs, in order
  (outcomes nil)      ; alist of (ID . correct|override|retry|fixed|wrong)
  (streak 0)
  (started nil)
  (latencies nil)
  (card nil)          ; plist of the presented card
  (dwell nil)         ; timer of the running dwell
  buffer
  (session-id nil)
  (finished nil))

(defvar gnosis-feed--current nil
  "The running or last finished `gnosis-feed--burst'.")

(defvar gnosis-feed--count 0
  "Bursts started in this Emacs session, mixed into the order seed.")

(defvar gnosis-feed--token 0
  "Counter distinguishing feedback readers for queued advance events.")

(defvar gnosis-feed--dwell-supported 'unknown
  "Whether input queued by a timer wakes a waiting reader: t, nil or `unknown'.")

(defvar gnosis-feed--hints (cons nil nil)
  "Cons of (DAY . ALIST) counting presentations per card kind today.")

(defun gnosis-feed--active-p ()
  "Return the running burst if its review buffer is current."
  (let ((burst gnosis-feed--current))
    (and burst (not (gnosis-feed--burst-finished burst))
         (eq (current-buffer) (gnosis-feed--burst-buffer burst))
         burst)))

;;; Planning

(defun gnosis-feed--random (seed)
  "Return a deterministic generator of integers below N seeded by SEED."
  (let ((x (logand (abs seed) #x7fffffff)))
    (lambda (n)
      (setq x (logand (+ (* x 1103515245) 12345) #x7fffffff))
      (% (ash x -8) n))))

(defun gnosis-feed--shuffle (list random)
  "Return LIST shuffled with generator RANDOM."
  (let ((v (vconcat list)))
    (cl-loop for i from (1- (length v)) downto 1
             do (cl-rotatef (aref v i) (aref v (funcall random (1+ i)))))
    (append v nil)))

(defun gnosis-feed-plan (due new topics size seed &optional new-every)
  "Return at most SIZE IDs interleaving DUE and NEW deterministically.
TOPICS maps an ID to its topic key.  Due IDs are shuffled within each
topic by SEED and dealt round-robin across topics; one NEW ID follows
every NEW-EVERY due IDs, and new IDs fill the burst when due run out."
  (let* ((random (gnosis-feed--random seed))
         (groups nil))
    (dolist (id due)
      (let* ((topic (alist-get id topics))
             (cell (assoc topic groups)))
        (if cell (push id (cdr cell)) (push (list topic id) groups))))
    (let* ((queues (gnosis-feed--shuffle
                    (mapcar (lambda (group)
                              (gnosis-feed--shuffle (reverse (cdr group)) random))
                            (reverse groups))
                    random))
           (dealt nil)
           (plan nil)
           (since-new 0))
      (while queues
        (setq queues (delq nil (mapcar (lambda (queue) (push (car queue) dealt) (cdr queue))
                                       queues))))
      (setq dealt (nreverse dealt))
      (while (and (< (length plan) size) (or dealt new))
        (if (or (null dealt)
                (and new new-every (> new-every 0) (>= since-new new-every)))
            (progn (push (pop new) plan) (setq since-new 0))
          (push (pop dealt) plan)
          (cl-incf since-new)))
      (nreverse plan))))

(defun gnosis-feed--candidates ()
  "Return (DUE NEW) thema IDs eligible now, new ones within the limit."
  (let* ((db (gnosis--ensure-db))
         (today (gnosis--today-int))
         (rows (gnosis-sqlite-select
                db "SELECT thema_id, reps FROM scheduler_state
                     WHERE suspended = 0 AND due_day <= ?
                     ORDER BY due_day, thema_id" (list today)))
         (due (mapcar #'car (seq-filter (lambda (row) (> (cadr row) 0)) rows)))
         (new (mapcar #'car (seq-filter (lambda (row) (= (cadr row) 0)) rows))))
    (list due (if gnosis-new-themata-limit
                  (seq-take new (max 0 gnosis-new-themata-limit))
                new))))

(defun gnosis-feed--topics (ids)
  "Return an alist of IDS to their first tag."
  (when ids
    (let (topics)
      (dolist (row (gnosis-sqlite-select
                    (gnosis--ensure-db)
                    (format "SELECT thema_id, tag FROM thema_tag WHERE thema_id IN (%s)
                             ORDER BY rowid DESC"
                            (mapconcat #'number-to-string
                                       (seq-filter #'integerp ids) ","))))
        (setf (alist-get (car row) topics) (cadr row)))
      topics)))

(defun gnosis-feed--select (size)
  "Return the planned IDs of the next burst of SIZE."
  (pcase-let ((`(,due ,new) (gnosis-feed--candidates)))
    (gnosis-feed-plan due new (gnosis-feed--topics due) size
                      (sxhash-equal (list (gnosis--today-int) gnosis-feed--count))
                      gnosis-feed-new-every)))

;;; Verdict line

(defun gnosis-feed--slot (&optional newline)
  "Return the schedule slot, preceded by NEWLINE line breaks."
  (concat (make-string (or newline 0) ?\n)
          (propertize "\n" 'gnosis-review-status-slot t)))

(defun gnosis-feed--date (date)
  "Return a short label for schedule DATE, a (YEAR MONTH DAY) list."
  (let* ((time (encode-time (list 0 0 12 (nth 2 date) (nth 1 date) (nth 0 date))))
         (days (- (time-to-days time) (time-to-days (current-time)))))
    (cond ((<= days 0) "today")
          ((= days 1) "tomorrow")
          (t (format "%s (+%d d)" (format-time-string "%b %-d" time) days)))))

(defun gnosis-feed--clean (text)
  "Return TEXT on one line without Org markup noise."
  (when (and (stringp text) (not (string-empty-p (string-trim text))))
    (string-join (split-string (substring-no-properties text) "[\n\r]+" t "[ \t]+") " ")))

(defun gnosis-feed--card-put (property value)
  "Set PROPERTY of the presented card to VALUE."
  (when-let* ((burst gnosis-feed--current))
    (setf (gnosis-feed--burst-card burst)
          (plist-put (gnosis-feed--burst-card burst) property value))))

(defun gnosis-feed--card-get (property)
  "Return PROPERTY of the presented card."
  (and gnosis-feed--current
       (plist-get (gnosis-feed--burst-card gnosis-feed--current) property)))

(defun gnosis-feed--retry-pending-p (id)
  "Return non-nil when a failure of ID now would be retried in this burst."
  (when-let* ((buffer (get-buffer gnosis-review-buffer-name))
              (state (buffer-local-value 'gnosis-review--state buffer)))
    (and (eq (gnosis-review-state-mode state) 'due)
         (not (member id (gnosis-review-state-requeued state))))))

(defun gnosis-feed--requeued-p (id)
  "Return non-nil when ID has been requeued in the running batch."
  (when-let* ((buffer (get-buffer gnosis-review-buffer-name))
              (state (buffer-local-value 'gnosis-review--state buffer)))
    (member id (gnosis-review-state-requeued state))))

(defun gnosis-feed--schedule-line (plist)
  "Return the verdict line for schedule PLIST and the stashed card."
  (let* ((success (plist-get plist :success))
         (date (plist-get plist :date))
         (answer (gnosis-feed--clean (gnosis-feed--card-get :answer)))
         (response (gnosis-feed--clean (gnosis-feed--card-get :response)))
         (overridden (plist-get plist :replace))
         (mark (if success (propertize "✓" 'face 'gnosis-feed-correct)
                 (propertize "✗" 'face 'gnosis-feed-wrong)))
         (when-line (cond ((null date) "practice · schedule unchanged")
                          (success (concat "next " (gnosis-feed--date date)))
                          ((gnosis-feed--retry-pending-p (plist-get plist :thema-id))
                           (concat "again later this burst · then " (gnosis-feed--date date)))
                          (t (concat "next " (gnosis-feed--date date))))))
    (concat
     mark "  "
     (cond ((and answer success) (propertize answer 'face 'gnosis-feed-correct))
           (answer (propertize answer 'face 'gnosis-feed-answer))
           (t ""))
     (when (and response (not success) (not (equal response answer)))
       (concat (if answer "    " "")
               (propertize (if (gnosis-feed--card-get :picked) "you picked  " "you typed  ")
                           'face 'gnosis-feed-dim)
               (propertize response 'face 'gnosis-feed-wrong)))
     (when overridden (propertize "  · overridden" 'face 'gnosis-feed-fixed))
     (if success "    " "\n")
     (propertize when-line 'face 'gnosis-feed-dim))))

(defun gnosis-feed--verdict (plist)
  "Format feedback PLIST for the feed; see `gnosis-review-verdict-function'."
  (let ((model (equal (gnosis-feed--card-get :kind) "model")))
    (pcase (plist-get plist :part)
      ('answer
       (gnosis-feed--card-put :answer (plist-get plist :answer))
       (gnosis-feed--card-put :response (plist-get plist :response))
       (gnosis-feed--card-put :picked (and model (not (plist-get plist :success))))
       (gnosis-feed--slot (if model 1 2)))
      ('choice
       (gnosis-feed--card-put :answer (plist-get plist :answer))
       (gnosis-feed--card-put :response (plist-get plist :response))
       (gnosis-feed--card-put :picked t)
       (gnosis-feed--slot 2))
      ('response
       (gnosis-feed--card-put :response (plist-get plist :response))
       (gnosis-feed--slot 2))
      ('parathema
       (unless (text-property-any (point-min) (point-max) 'gnosis-review-status-slot t)
         (gnosis-feed--slot 1)))
      ('schedule (gnosis-feed--schedule-line plist)))))

;;; Header line

(defun gnosis-feed--dots (burst)
  "Return the progress dots of BURST."
  (mapconcat
   (lambda (id)
     (pcase (alist-get id (gnosis-feed--burst-outcomes burst))
       ('correct (propertize "●" 'face 'gnosis-feed-correct))
       ('override (propertize "●" 'face 'gnosis-feed-dim))
       ('fixed (propertize "◍" 'face 'gnosis-feed-fixed))
       ('retry (propertize "◌" 'face 'gnosis-feed-wrong))
       ('wrong (propertize "●" 'face 'gnosis-feed-wrong))
       (_ (propertize "○" 'face 'gnosis-feed-dim))))
   (gnosis-feed--burst-ids burst) ""))

(defun gnosis-feed--dwell-bar (burst)
  "Return the dwell bar of BURST's running clock, or nil."
  (when-let* ((dwell gnosis-feed-correct-dwell)
              (_ (eq burst gnosis-feed--current))
              (_ (gnosis-feed--card-get :token))
              (start (gnosis-feed--card-get :feedback-at)))
    (let* ((fraction (min 1.0 (/ (float-time (time-since start)) dwell)))
           (filled (round (* 5 fraction))))
      (propertize (concat (make-string filled ?▰) (make-string (- 5 filled) ?▱))
                  'face 'gnosis-feed-dim))))

(defun gnosis-feed--kind-hint (kind)
  "Return the question hint for KIND unless it has faded today."
  (let ((seen (alist-get kind (cdr gnosis-feed--hints) 0 nil #'equal)))
    (when (<= seen gnosis-feed-hint-presentations)
      (pcase kind
        ("model" "click · RET submit")
        ((or "mcq" "mc-cloze") "choose · RET")
        (_ "RET answer")))))

(defun gnosis-feed--right (left right)
  "Return header LEFT with RIGHT aligned to the window's right edge."
  (if (not right) left
    (concat left
            (propertize " " 'display `(space :align-to (- right ,(1+ (string-width right)))))
            right)))

(defun gnosis-feed--model-status ()
  "Return the model status when it is not ready, or nil."
  (when-let* ((context gnosis-review--model-context))
    (let ((attached (plist-get context :attachment)))
      (cond ((plist-get context :error)
             (propertize "Unavailable · RET details" 'face 'error))
            ((not attached) (propertize "Loading…" 'face 'gnosis-feed-dim))
            ((not (gnosis-model-backend-get :live))
             (propertize "Unavailable" 'face 'error))))))

(defun gnosis-feed--header ()
  "Return the feed header line; never topics, tags or answers."
  (if-let* ((burst gnosis-feed--current))
      (let* ((feedback gnosis-review--feedback)
             (dots (gnosis-feed--dots burst))
             (streak (gnosis-feed--burst-streak burst))
             (guarded (and feedback (gnosis-feed--guarded-p)))
             (left (concat
                    " " dots
                    (when (and feedback (>= streak gnosis-feed-streak-threshold))
                      (concat "   " (propertize (format "✓ %d in a row" streak)
                                                'face 'gnosis-feed-correct)))
                    (when-let* ((bar (and feedback (gnosis-feed--dwell-bar burst))))
                      (concat "   " bar))))
             (right (cond (feedback (gnosis-feed--keys-hint guarded))
                          ((gnosis-feed--model-status))
                          ((gnosis-feed--kind-hint (gnosis-feed--card-get :kind))
                           (propertize (gnosis-feed--kind-hint (gnosis-feed--card-get :kind))
                                       'face 'gnosis-feed-dim)))))
        (gnosis-feed--right left right))
    (gnosis-review--header-line)))

;;; Feedback rhythm

(defun gnosis-feed--keys-hint (guarded)
  "Return the feedback key hint, all dimmed when GUARDED."
  (mapconcat (lambda (pair)
               (concat (propertize (car pair) 'face (if guarded 'gnosis-feed-dim
                                                      'help-key-binding))
                       " " (propertize (cdr pair) 'face 'gnosis-feed-dim)))
             '(("SPC" . "next") ("o" . "override") ("?" . "more"))
             "  "))

(defun gnosis-feed--guarded-p ()
  "Return non-nil while a wrong answer's guard is running."
  (when-let* ((at (gnosis-feed--card-get :feedback-at)))
    (and (not (gnosis-feed--card-get :success))
         (not (gnosis-feed--card-get :overridden))
         (< (float-time (time-since at)) gnosis-feed-wrong-guard))))

(defun gnosis-feed--revoke ()
  "Stop the dwell of the presented card and revoke its advance event.
A queued event outlives its timer, so authorization lives in the card's
token: once cleared, a delivered event accepts nothing."
  (when-let* ((burst gnosis-feed--current))
    (when-let* ((timer (gnosis-feed--burst-dwell burst)))
      (cancel-timer timer)
      (setf (gnosis-feed--burst-dwell burst) nil))
    (when (gnosis-feed--card-get :token)
      (gnosis-feed--card-put :token nil)
      (setq unread-command-events
            (seq-remove (lambda (event) (eq (car-safe event) 'gnosis-feed-advance))
                        unread-command-events))
      (when (buffer-live-p (gnosis-feed--burst-buffer burst))
        (with-current-buffer (gnosis-feed--burst-buffer burst)
          (force-mode-line-update))))))

(defun gnosis-feed--tick (burst token)
  "Repaint BURST's dwell bar; enqueue one advance event for TOKEN when due.
Runs from a timer, so it only queues ordinary input and never reads it."
  (when (and (eq burst gnosis-feed--current)
             token (eq token (gnosis-feed--card-get :token))
             (buffer-live-p (gnosis-feed--burst-buffer burst)))
    (with-current-buffer (gnosis-feed--burst-buffer burst)
      (force-mode-line-update))
    (when (and (>= (float-time (time-since (gnosis-feed--card-get :feedback-at)))
                   gnosis-feed-correct-dwell)
               (not (gnosis-feed--card-get :queued)))
      (gnosis-feed--card-put :queued t)
      (when-let* ((timer (gnosis-feed--burst-dwell burst)))
        (cancel-timer timer)
        (setf (gnosis-feed--burst-dwell burst) nil))
      (setq unread-command-events
            (append unread-command-events (list (list 'gnosis-feed-advance token)))))))

(defun gnosis-feed--advance (event)
  "Accept the correct answer EVENT's dwell finished for, if still authorized."
  (interactive "e")
  (let ((token (gnosis-feed--card-get :token)))
    (when (and token (gnosis-feed--active-p)
               (eq token (car-safe (cdr-safe event)))
               gnosis-review--feedback
               (gnosis-feed--card-get :success)
               (not (gnosis-feed--card-get :overridden)))
      (gnosis-feed--card-put :token nil)
      (gnosis-review-feedback-next))))

;; Stale advance events reaching other readers do nothing.
(define-key global-map [gnosis-feed-advance] #'ignore)

(defun gnosis-feed-next ()
  "Accept the pending answer and continue, unless a wrong answer is fresh."
  (interactive nil gnosis-review-feedback-mode)
  (if (and (gnosis-feed--active-p) (gnosis-feed--guarded-p))
      (progn (force-mode-line-update) (message "Take a look first"))
    (gnosis-review-feedback-next)))

(defvar-keymap gnosis-feed-feedback-map
  :doc "Feed keys over `gnosis-review-feedback-mode-map'."
  :parent gnosis-review-feedback-mode-map
  "SPC" #'gnosis-feed-next
  "n" #'gnosis-feed-next
  "<gnosis-feed-advance>" #'gnosis-feed--advance)

(defun gnosis-feed--pre-command ()
  "Revoke the dwell on any command but the queued advance."
  (unless (eq this-command 'gnosis-feed--advance)
    (gnosis-feed--revoke)))

(defun gnosis-feed--flash ()
  "Flash the header line of the current buffer briefly."
  (when gnosis-feed-flash-duration
    (let ((buffer (current-buffer))
          (cookie (face-remap-add-relative 'header-line 'gnosis-feed-flash)))
      (force-mode-line-update)
      (run-with-timer gnosis-feed-flash-duration nil
                      (lambda ()
                        (when (buffer-live-p buffer)
                          (with-current-buffer buffer
                            (face-remap-remove-relative cookie)
                            (force-mode-line-update))))))))

(defvar-local gnosis-feed--override nil
  "The `minor-mode-overriding-map-alist' entry the feed added here, or nil.")

(define-minor-mode gnosis-feed--keys-mode
  "Feed feedback keys and dwell revocation for one burst's review buffer.
Disabling removes exactly the entry and hook it added."
  :interactive nil
  :lighter nil
  (if gnosis-feed--keys-mode
      (unless gnosis-feed--override
        (setq gnosis-feed--override
              (cons 'gnosis-review-feedback-mode gnosis-feed-feedback-map))
        (setq-local minor-mode-overriding-map-alist
                    (cons gnosis-feed--override minor-mode-overriding-map-alist))
        (add-hook 'pre-command-hook #'gnosis-feed--pre-command nil t))
    (when gnosis-feed--override
      (setq-local minor-mode-overriding-map-alist
                  (remq gnosis-feed--override minor-mode-overriding-map-alist))
      (setq gnosis-feed--override nil))
    (remove-hook 'pre-command-hook #'gnosis-feed--pre-command t)))

(defun gnosis-feed--finish (burst)
  "Mark BURST finished and remove its keys from its review buffer."
  (when (eq burst gnosis-feed--current) (gnosis-feed--revoke))
  (setf (gnosis-feed--burst-finished burst) t)
  (let ((buffer (gnosis-feed--burst-buffer burst)))
    (when (and (buffer-live-p buffer)
               (buffer-local-value 'gnosis-feed--keys-mode buffer))
      (with-current-buffer buffer (gnosis-feed--keys-mode -1)))))

(defun gnosis-feed--outcome (burst id success)
  "Record BURST's accepted SUCCESS for ID and update the streak."
  (let* ((outcomes (gnosis-feed--burst-outcomes burst))
         (before (alist-get id outcomes))
         (overridden (gnosis-feed--card-get :overridden))
         (after (cond ((and success (eq before 'retry)) 'fixed)
                      ((and success overridden) 'override)
                      (success 'correct)
                      ;; Acceptance has already requeued a first failure.
                      ((and (null before) (gnosis-feed--requeued-p id)) 'retry)
                      (t 'wrong))))
    (setf (alist-get id (gnosis-feed--burst-outcomes burst)) after)
    (setf (gnosis-feed--burst-streak burst)
          (if (eq after 'correct) (1+ (gnosis-feed--burst-streak burst)) 0))))

(defun gnosis-feed--observe (event)
  "Update the running burst's display from review EVENT."
  (when-let* ((burst gnosis-feed--current)
              (_ (not (gnosis-feed--burst-finished burst))))
    (let ((id (plist-get event :thema-id))
          (buffer (get-buffer gnosis-review-buffer-name)))
      (setf (gnosis-feed--burst-session-id burst) (plist-get event :session-id))
      (pcase (plist-get event :type)
        ('presented
         (gnosis-feed--revoke)
         (let ((old (gnosis-feed--burst-buffer burst)))
           (when (and (buffer-live-p old) (not (eq old buffer)))
             (with-current-buffer old (gnosis-feed--keys-mode -1))))
         (setf (gnosis-feed--burst-buffer burst) buffer)
         (when (and buffer (not (buffer-local-value 'gnosis-feed--keys-mode buffer)))
           (with-current-buffer buffer (gnosis-feed--keys-mode 1)))
         (let ((kind (plist-get event :kind)) (today (gnosis--today-int)))
           (unless (equal (car gnosis-feed--hints) today)
             (setq gnosis-feed--hints (cons today nil)))
           (cl-incf (alist-get kind (cdr gnosis-feed--hints) 0 nil #'equal))
           (setf (gnosis-feed--burst-card burst) (list :id id :kind kind))))
        ('answered
         (let ((success (plist-get event :success)))
           (gnosis-feed--revoke)
           (push (plist-get event :latency-ms) (gnosis-feed--burst-latencies burst))
           (gnosis-feed--card-put :success success)
           (gnosis-feed--card-put :feedback-at (current-time))
           (when (and success (buffer-live-p buffer))
             (with-current-buffer buffer (gnosis-feed--flash))
             (when gnosis-feed-correct-dwell
               (let ((token (cl-incf gnosis-feed--token)))
                 (gnosis-feed--card-put :token token)
                 (setf (gnosis-feed--burst-dwell burst)
                       (run-with-timer 0.1 0.1 #'gnosis-feed--tick burst token)))))))
        ('overridden
         (gnosis-feed--revoke)
         (gnosis-feed--card-put :overridden (not (gnosis-feed--card-get :overridden)))
         (gnosis-feed--card-put :success (plist-get event :success)))
        ('accepted
         (gnosis-feed--revoke)
         (gnosis-feed--outcome burst id (plist-get event :success)))
        ('cancelled (gnosis-feed--revoke))))))

;;; Burst end

(defvar-keymap gnosis-feed-end-mode-map
  :doc "Keymap at the end of a feed burst."
  :parent special-mode-map
  "SPC" #'gnosis-feed
  "RET" #'gnosis-feed
  "q" #'quit-window)

(defvar-local gnosis-feed--end-width 0
  "Widest line of the burst-end screen.")

(defvar gnosis-feed--end-buffer nil
  "The burst-end buffer the feed created, or nil.")

(defvar-local gnosis-feed--end-owned nil
  "Name the feed gave this burst-end buffer; cleared by any mode change.")

(defun gnosis-feed--end-buffer ()
  "Return the feed's own burst-end buffer, or a new one.
Reuse only the buffer the feed created, still in its mode, under its name
and visiting no file; never claim a foreign buffer of the same name."
  (let ((buffer gnosis-feed--end-buffer))
    (if (and (buffer-live-p buffer)
             (with-current-buffer buffer
               (and (derived-mode-p 'gnosis-feed-end-mode)
                    (null buffer-file-name)
                    (equal gnosis-feed--end-owned (buffer-name)))))
        buffer
      (setq gnosis-feed--end-buffer (generate-new-buffer "*Gnosis Feed*")))))

(defun gnosis-feed--center-end (window &rest _)
  "Center the burst-end text block in WINDOW."
  (when (window-live-p window)
    (with-current-buffer (window-buffer window)
      (when (derived-mode-p 'gnosis-feed-end-mode)
        (setq-local line-prefix
                    (make-string (max 2 (/ (- (window-body-width window) gnosis-feed--end-width) 2))
                                 ?\s))))))

(define-derived-mode gnosis-feed-end-mode special-mode "Gnosis Feed"
  "Major mode for the end of a feed burst."
  :interactive nil
  (setq-local cursor-type nil)
  (setq header-line-format nil)
  (add-hook 'window-size-change-functions #'gnosis-feed--center-end nil t))

(defun gnosis-feed--summary-plist (burst state)
  "Return the burst-end facts of BURST and review STATE."
  (let ((outcomes (mapcar #'cdr (gnosis-feed--burst-outcomes burst))))
    (list :api 1
          :session-id (gnosis-review-state-session-id state)
          :size (length (gnosis-feed--burst-ids burst))
          :reviewed (length outcomes)
          :correct (cl-count 'correct outcomes)
          :fixed (cl-count 'fixed outcomes)
          :wrong (+ (cl-count 'wrong outcomes) (cl-count 'retry outcomes))
          :overridden (cl-count 'override outcomes)
          :seconds (round (float-time (time-since (gnosis-feed--burst-started burst))))
          :finished (null (gnosis-review-state-remaining state)))))

(defun gnosis-feed--tally (facts)
  "Return the one-line tally of burst FACTS."
  (let ((size (plist-get facts :size))
        (reviewed (plist-get facts :reviewed))
        (seconds (plist-get facts :seconds)))
    (concat
     (if (= reviewed size)
         (format "%d cards" size)
       (format "%d of %d cards" reviewed size))
     (format " · %d ✓" (plist-get facts :correct))
     (mapconcat (lambda (pair)
                  (let ((n (plist-get facts (car pair))))
                    (if (> n 0) (format " · %d %s" n (cdr pair)) "")))
                '((:fixed . "fixed") (:overridden . "overridden") (:wrong . "to revisit"))
                "")
     (format " · %d min %02d s" (/ seconds 60) (% seconds 60)))))

(defun gnosis-feed--median (numbers)
  "Return the median of NUMBERS, or nil."
  (when-let* ((sorted (sort (delq nil (copy-sequence numbers)) #'<)))
    (nth (/ (length sorted) 2) sorted)))

(defun gnosis-feed--end (state)
  "Present the end of the running burst for review STATE."
  (let* ((burst gnosis-feed--current)
         (facts (gnosis-feed--summary-plist burst state))
         (median (gnosis-feed--median (gnosis-feed--burst-latencies burst)))
         (remaining (let ((gnosis-db (or (gnosis-review-state-database state) gnosis-db)))
                      (gnosis-feed--candidates)))
         (due (length (car remaining)))
         (new (length (cadr remaining)))
         (buffer (gnosis-feed--end-buffer)))
    (gnosis-feed--finish burst)
    (with-current-buffer buffer
      (let ((inhibit-read-only t)
            (lines
             (list
              (gnosis-feed--dots burst)
              ""
              (concat
               (propertize (gnosis-feed--tally facts) 'face 'bold)
               (if median
                   (propertize (format "   median answer %.1f s" (/ median 1000.0))
                               'face 'gnosis-feed-dim)
                 ""))
              (propertize
               (cond ((not (plist-get facts :finished)) "Stopped early; accepted answers are kept.")
                     ((> due 0) (format "%d due · %d new available" due new))
                     ((> new 0) (format "Nothing due · %d new within today's limit" new))
                     (t "Nothing due.  Done for now."))
               'face 'gnosis-feed-dim)
              ""
              (concat (if (> (+ due new) 0)
                          (concat (propertize "SPC" 'face 'help-key-binding) " next burst   ")
                        "")
                      (propertize "q" 'face 'help-key-binding) " stop"))))
        (gnosis-feed-end-mode)
        (setq gnosis-feed--end-owned (buffer-name))
        (erase-buffer)
        (insert "\n\n\n" (mapconcat #'identity lines "\n") "\n")
        (setq-local gnosis-feed--end-width (apply #'max (mapcar #'string-width lines)))
        (goto-char (point-min))))
    (pop-to-buffer-same-window buffer)
    (gnosis-feed--center-end (selected-window))
    (gnosis-review--deliver 'gnosis-feed-burst-functions facts "Gnosis feed")))

;;; Entry points

(defun gnosis-feed-snapshot ()
  "Return a JSON-safe plist of the feed and review state, without text."
  (let ((burst gnosis-feed--current))
    (append
     (gnosis-review-snapshot)
     (when burst
       (list :burst-state (if (gnosis-feed--burst-finished burst) "burst-end" "running")
             :burst-size (length (gnosis-feed--burst-ids burst))
             :dots (substring-no-properties (gnosis-feed--dots burst))
             :streak (gnosis-feed--burst-streak burst)
             :dwell (and (gnosis-feed--card-get :token) t))))))

(defun gnosis-feed--dwell-supported-p ()
  "Return non-nil when input queued by a timer wakes a waiting reader.
Auto-advance relies on it.  Probe once with a short timed read: a runtime
that keeps sleeping returns nil with the event still queued.  Batch runs
are assumed capable; keyboard macros, typing and a quit during the probe
defer it."
  (when (eq gnosis-feed--dwell-supported 'unknown)
    (cond
     (noninteractive (setq gnosis-feed--dwell-supported t))
     ((not (or executing-kbd-macro defining-kbd-macro))
      ;; A fresh uninterned symbol per probe: only this probe's own
      ;; delivery counts, and cleanup removes nothing else.
      (let* ((wakeup (make-symbol "gnosis-feed--wakeup"))
             (timer (run-with-timer
                     0.02 nil
                     (lambda ()
                       (setq unread-command-events
                             (append unread-command-events (list wakeup))))))
             event queued)
        (unwind-protect
            (setq event (read-event nil nil 0.25)
                  queued (memq wakeup unread-command-events))
          (cancel-timer timer)
          (setq unread-command-events (delq wakeup unread-command-events)))
        (cond ((eq event wakeup) (setq gnosis-feed--dwell-supported t))
              (event (push event unread-command-events))
              (queued
               (setq gnosis-feed--dwell-supported nil)
               (message "Feed: this Emacs does not wake for queued input; SPC advances")))))))
  (eq gnosis-feed--dwell-supported t))

;;;###autoload
(defun gnosis-feed (&optional size)
  "Review the next burst of due themata at a calm, quick rhythm.
SIZE defaults to `gnosis-feed-burst-size'; with a prefix argument, ask.
Answer, look at the verdict, press SPC.  Correct answers move on by
themselves after `gnosis-feed-correct-dwell'.  Press ? for all actions."
  (interactive (list (when current-prefix-arg
                       (read-number "Burst size: " gnosis-feed-burst-size))))
  (gnosis-review--check-input-entry)
  (when gnosis-review--running (user-error "Finish the active review first"))
  (let* ((size (or size gnosis-feed-burst-size))
         (ids (progn (cl-incf gnosis-feed--count) (gnosis-feed--select size))))
    (if (null ids)
        (message "Nothing due.  Done for now.")
      (let ((burst (gnosis-feed--burst-create :ids ids :started (current-time)))
            (dwell (and gnosis-feed-correct-dwell (gnosis-feed--dwell-supported-p)
                        gnosis-feed-correct-dwell)))
        (when gnosis-feed--current (gnosis-feed--finish gnosis-feed--current))
        (setq gnosis-feed--current burst)
        (unwind-protect
            (let ((gnosis-review-feedback-menu-style 'on-demand)
                  (gnosis-review-verdict-function #'gnosis-feed--verdict)
                  (gnosis-review-header-function #'gnosis-feed--header)
                  (gnosis-review-model-header-function #'gnosis-feed--header)
                  (gnosis-review-summary-function #'gnosis-feed--end)
                  (gnosis-feed-correct-dwell dwell)
                  (gnosis-monkeytype-enable (and gnosis-feed-monkeytype
                                                 gnosis-monkeytype-enable))
                  (gnosis-review-event-functions
                   (cons #'gnosis-feed--observe gnosis-review-event-functions)))
              (gnosis-review-loop ids 'due))
          (gnosis-feed--finish burst))))))

(provide 'gnosis-feed)
;;; gnosis-feed.el ends here
