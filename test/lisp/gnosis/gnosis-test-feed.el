;;; gnosis-test-feed.el --- Feed bursts -*- lexical-binding: t; -*-

;; Copyright (C) 2026 Free Software Foundation, Inc.

;;; Commentary:
;; The feed changes presentation only.  These tests drive real scheduled
;; batches through the shared review loop.

;;; Code:

(require 'ert)
(require 'gnosis-review-test-support)
(require 'gnosis-feed)

(defmacro gnosis-test-feed (&rest body)
  "Run BODY with isolated data, three basic themata and owned buffers."
  (declare (indent 0) (debug t))
  `(gnosis-test-with-db
     (let ((buffers (buffer-list))
           (gnosis-review-buffer-name "*Gnosis Feed Test*")
           (gnosis-review--running nil)
           (gnosis-new-themata-limit nil)
           (gnosis-center-content nil)
           (gnosis-monkeytype-enable nil)
           (gnosis-feed-flash-duration nil)
           (gnosis-feed--current nil)
           (gnosis-feed--end-buffer nil)
           (gnosis-feed--dwell-supported t)
           (gnosis-feed--hints (cons nil nil))
           (register-alist nil))
       (save-window-excursion
         (unwind-protect
             (progn
               (gnosis-test--add-basic-thema "Alpha question" "alpha" nil "Alpha note" 101)
               (gnosis-test--add-basic-thema "Beta question" "beta" nil nil 102)
               (gnosis-test--add-basic-thema "Gamma question" "gamma" nil nil 103)
               ,@body)
           (when gnosis-feed--current (gnosis-feed--revoke))
           (dolist (buffer (seq-difference (buffer-list) buffers))
             (when (buffer-live-p buffer) (kill-buffer buffer))))))))

(defun gnosis-test-feed--ratings ()
  "Return accepted scheduled ratings by thema."
  (sqlite-select gnosis-db "SELECT thema_id, rating FROM review_events ORDER BY rowid"))

(defun gnosis-test-feed--text ()
  "Return the review buffer text."
  (with-current-buffer gnosis-review-buffer-name
    (buffer-substring-no-properties (point-min) (point-max))))

(defun gnosis-test-feed--run (answers keys)
  "Run one feed burst over 101..103 with ANSWERS and KEYS."
  (cl-letf (((symbol-function 'gnosis-feed--select) (lambda (_) '(101 102 103))))
    (gnosis-test-drive answers keys #'gnosis-feed)))

(ert-deftest gnosis-test-feed-plan-interleaves ()
  "Plans interleave topics, pace new themata and replay by seed."
  (let* ((topics '((1 . "a") (2 . "a") (3 . "a") (4 . "b") (5 . "b") (6 . "c")))
         (plan (gnosis-feed-plan '(1 2 3 4 5 6) '(90 91) topics 10 7 4)))
    (should (equal plan (gnosis-feed-plan '(1 2 3 4 5 6) '(90 91) topics 10 7 4)))
    (should (equal 8 (length plan)))
    (should (equal '(90) (seq-filter (lambda (id) (> id 50)) (seq-take plan 5))))
    (should (> (seq-position plan 90) 3))
    ;; The first three cards come from three different topics.
    (should (= 3 (length (delete-dups (mapcar (lambda (id) (alist-get id topics))
                                              (seq-take plan 3))))))
    (should (= 2 (length (gnosis-feed-plan '(1 2 3) nil nil 2 1 nil))))
    (should (equal '(90 91) (gnosis-feed-plan nil '(90 91) nil 10 1 4)))))

(ert-deftest gnosis-test-feed-classic-unchanged ()
  "Classic review keeps its popup, labels and header outside the feed."
  (gnosis-test-feed
    (let ((popups 0) text)
      (cl-letf (((symbol-function 'gnosis-review--feedback-show) (lambda () (cl-incf popups))))
        (gnosis-test-drive '("alpha") (list (lambda () (setq text (gnosis-test-feed--text))) "n")
                           (lambda () (gnosis-review-loop '(101) 'due))))
      (should (= 1 popups))
      (should (string-match-p "Answer: alpha" text))
      (should (string-match-p "Next review: " text))
      (should-not (string-match-p "✓" text)))))

(ert-deftest gnosis-test-feed-rhythm ()
  "A burst shows verdict lines, no popup, dots and the burst-end screen."
  (gnosis-test-feed
    (let ((popups 0) texts headers)
      (cl-letf (((symbol-function 'gnosis-review--feedback-show) (lambda () (cl-incf popups))))
        (gnosis-test-feed--run
         '("alpha" "nope" "gamma" "beta")
         (let ((look (lambda ()
                       (push (gnosis-test-feed--text) texts)
                       (with-current-buffer gnosis-review-buffer-name
                         (push (substring-no-properties (gnosis-feed--header)) headers)
                         ;; Let the wrong guard elapse without waiting.
                         (gnosis-feed--card-put :feedback-at (time-subtract nil 5))))))
           (list look "SPC" look "SPC" look "n" look "SPC"))))
      (setq texts (nreverse texts) headers (nreverse headers))
      (should (= 0 popups))
      (should (string-match-p "^✓  alpha    next " (nth 0 texts)))
      (should-not (string-match-p "Answer:\\|Next review:" (nth 0 texts)))
      ;; The verdict sits before the explanation.
      (should (< (string-match "✓" (nth 0 texts)) (string-match "Alpha note" (nth 0 texts))))
      (should (string-match-p "^✗  beta    you typed  nope\nagain later this burst · then " (nth 1 texts)))
      (should (string-match-p "^ ○○○ .*SPC next" (nth 0 headers)))
      (should (string-match-p "^ ●○○ " (nth 1 headers)))
      (should (string-match-p "^ ●◌○ " (nth 2 headers)))
      (should (string-match-p "^ ●◌● " (nth 3 headers)))
      (should (equal '((101 3) (102 1) (103 3) (102 3)) (gnosis-test-feed--ratings)))
      (with-current-buffer gnosis-feed--end-buffer
        (should (string-match-p "●◍●" (buffer-string)))
        (should (string-match-p "3 cards · 2 ✓ · 1 fixed" (buffer-string))))
      (should (equal "burst-end" (plist-get (gnosis-feed-snapshot) :burst-state))))))

(ert-deftest gnosis-test-feed-override-current-outcome ()
  "Override repaints the verdict with the current outcome and date in place."
  (gnosis-test-feed
    (let (before after)
      (gnosis-test-feed--run
       '("nope" "beta" "gamma")
       (list (lambda () (setq before (gnosis-test-feed--text)))
             "o"
             (lambda () (setq after (gnosis-test-feed--text)))
             "SPC" "SPC" "SPC"))
      (should (string-match-p "^✗  alpha    you typed  nope\nagain later" before))
      (should (string-match-p "^✓  alpha  · overridden    next " after))
      ;; Same explanation, one verdict: nothing was appended.
      (should (= 1 (with-temp-buffer (insert after) (how-many "Alpha note" (point-min)))))
      (should (equal 3 (cadr (car (gnosis-test-feed--ratings)))))
      ;; The summary accounts for the overridden card.
      (with-current-buffer gnosis-feed--end-buffer
        (should (string-match-p "3 cards · 2 ✓ · 1 overridden" (buffer-string))))
      ;; Overridden successes do not count as first-try correct.
      (should (eq 'override (alist-get 101 (gnosis-feed--burst-outcomes gnosis-feed--current)))))))

(ert-deftest gnosis-test-feed-wrong-guard ()
  "SPC right after a wrong answer accepts nothing."
  (gnosis-test-feed
    (let ((gnosis-feed-wrong-guard 60))
      (gnosis-test-feed--run
       '("nope" "beta" "gamma" "alpha")
       (list "SPC"
             (lambda () (should (null (gnosis-test-feed--ratings)))
               (gnosis-feed--card-put :feedback-at (time-subtract nil 120)))
             "SPC" "SPC" "SPC" "SPC"))
      (should (equal '((101 1) (102 3) (103 3) (101 3)) (gnosis-test-feed--ratings))))))

(ert-deftest gnosis-test-feed-dwell-advance ()
  "Correct answers queue one advance event; any command stops the clock."
  (gnosis-test-feed
    (let ((gnosis-feed-correct-dwell 0.3) queued)
      (gnosis-test-feed--run
       '("alpha" "beta" "gamma")
       (list
        ;; The dwell's timer only queues input; the reader dispatches it.
        (lambda ()
          (should (timerp (gnosis-feed--burst-dwell gnosis-feed--current)))
          (gnosis-feed--card-put :feedback-at (time-subtract nil 1))
          (let ((unread-command-events nil))
            (gnosis-feed--tick gnosis-feed--current (gnosis-feed--card-get :token))
            ;; Later ticks keep the reader awake without queueing again.
            (gnosis-feed--tick gnosis-feed--current (gnosis-feed--card-get :token))
            (setq queued unread-command-events))
          (should (equal 1 (length queued)))
          (should (null (gnosis-test-feed--ratings)))
          (with-current-buffer gnosis-review-buffer-name
            (should (eq #'gnosis-feed--advance (key-binding (vector (caar queued)))))
            (gnosis-feed--advance (car queued))))
        ;; A stale event from an earlier reader does nothing.
        (lambda ()
          (with-current-buffer gnosis-review-buffer-name
            (gnosis-feed--advance (car queued))
            (should (equal 1 (length (gnosis-test-feed--ratings))))
            (let ((this-command 'forward-char)) (gnosis-feed--pre-command))
            (should (null (gnosis-feed--burst-dwell gnosis-feed--current)))))
        "SPC" "SPC"))
      (should (equal '((101 3) (102 3) (103 3)) (gnosis-test-feed--ratings))))))

(ert-deftest gnosis-test-feed-streak ()
  "The streak appears from five first-try successes and vanishes on a miss."
  (let ((burst (gnosis-feed--burst-create :ids '(1 2 3 4 5 6 7) :started (current-time)))
        (gnosis-review--state nil))
    (let ((gnosis-feed--current burst))
      (dolist (id '(1 2 3 4 5)) (gnosis-feed--outcome burst id t))
      (should (= 5 (gnosis-feed--burst-streak burst)))
      (let ((gnosis-review--feedback '(:success t)))
        (should (string-match-p "✓ 5 in a row" (gnosis-feed--header))))
      (gnosis-feed--outcome burst 6 nil)
      (let ((gnosis-review--feedback '(:success t)))
        (should-not (string-match-p "in a row" (gnosis-feed--header)))))))

(ert-deftest gnosis-test-feed-header-never-leaks ()
  "Question headers show dots and hints, never tags or answers."
  (gnosis-test-feed
    (let (header)
      (gnosis-test--add-basic-thema "Pancreas question" "pancreas" '("pancreas") nil 104)
      (let ((gnosis-review-event-functions
             (list (lambda (event)
                     (when (eq 'presented (plist-get event :type))
                       (with-current-buffer gnosis-review-buffer-name
                         (setq header (format "%s" (gnosis-feed--header)))))))))
        (cl-letf (((symbol-function 'gnosis-feed--select) (lambda (_) '(104))))
          (gnosis-test-drive '("pancreas") '("SPC") #'gnosis-feed)))
      (should (string-match-p "○" header))
      (should (string-match-p "RET answer" header))
      (should-not (string-match-p "pancreas" header)))))

(defconst gnosis-test-feed--native-recursive-edit (symbol-function 'recursive-edit)
  "The native `recursive-edit', before any test replaces it.")

(defun gnosis-test-feed--command-loop (events)
  "Run EVENTS through the native command loop, with its hooks, and return."
  (let ((overriding-terminal-local-map (make-sparse-keymap)))
    (define-key overriding-terminal-local-map [gnosis-test-feed-done]
                (lambda () (interactive) (throw 'gnosis-test-feed-done nil)))
    (unwind-protect
        (catch 'gnosis-test-feed-done
          (setq unread-command-events (append events '(gnosis-test-feed-done)))
          (funcall gnosis-test-feed--native-recursive-edit))
      (setq unread-command-events nil))))

(defun gnosis-test-feed--queue ()
  "Let the running dwell finish and return the advance event it queued."
  (gnosis-feed--card-put :feedback-at (time-subtract nil 1))
  (let ((unread-command-events nil))
    (gnosis-feed--tick gnosis-feed--current (gnosis-feed--card-get :token))
    (car unread-command-events)))

(ert-deftest gnosis-test-feed-queued-advance-revoked ()
  "A command run before a queued advance event revokes that event."
  (gnosis-test-feed
    (let ((gnosis-feed-correct-dwell 0.3) event)
      (gnosis-test-feed--run
       '("alpha" "beta" "gamma")
       (list
        (lambda ()
          (setq event (gnosis-test-feed--queue))
          (should event)
          ;; Navigation already read runs first, then the queued event.
          (with-current-buffer gnosis-review-buffer-name
            (gnosis-test-feed--command-loop (list ?\C-a event))
            (should (null (gnosis-test-feed--ratings)))
            (should-not (gnosis-feed--burst-dwell gnosis-feed--current))
            ;; Direct, nested and foreign deliveries are refused too.
            (gnosis-feed--advance event)
            (let ((gnosis-test-drive--depth (1+ gnosis-test-drive--depth)))
              (ignore-errors (gnosis-feed--advance event))))
          (with-temp-buffer (gnosis-feed--advance event))
          (should (null (gnosis-test-feed--ratings))))
        "SPC"
        ;; Uninterrupted native delivery accepts exactly once.
        (lambda ()
          (setq event (gnosis-test-feed--queue))
          (with-current-buffer gnosis-review-buffer-name
            (gnosis-test-feed--command-loop (list event event))))
        ;; An override, even one flipped back, refuses the earlier event.
        (lambda ()
          (setq event (gnosis-test-feed--queue)))
        "o" "o"
        (lambda ()
          (with-current-buffer gnosis-review-buffer-name
            (gnosis-feed--advance event))
          (should (equal 2 (length (gnosis-test-feed--ratings)))))
        "SPC"))
      (should (equal '((101 3) (102 3) (103 3)) (gnosis-test-feed--ratings))))))

(ert-deftest gnosis-test-feed-dwell-unsupported ()
  "Without input wakeups the feed waits for SPC and leaves auto-save alone."
  (gnosis-test-feed
    (let ((gnosis-feed-correct-dwell 0.3)
          (gnosis-feed--dwell-supported nil)
          (auto-save-timeout 37)
          seen)
      (gnosis-test-feed--run
       '("alpha" "beta" "gamma")
       (list (lambda ()
               (push (list (gnosis-feed--burst-dwell gnosis-feed--current)
                           (gnosis-feed--card-get :token)
                           auto-save-timeout)
                     seen))
             "SPC" "SPC" "SPC"))
      (should (equal '((nil nil 37)) seen)))))

(defun gnosis-test-feed--probe (delay &optional signal)
  "Run the dwell probe with a reader that waits DELAY seconds.
The reader then signals SIGNAL or returns the next unread event.  Return a plist
of the outcome, the residue and the probe's leftover timers."
  (let ((before (copy-sequence timer-list))
        (noninteractive nil)
        (executing-kbd-macro nil)
        (defining-kbd-macro nil)
        (inhibit-message t)
        caught result)
    (cl-letf (((symbol-function 'read-event)
               (lambda (&rest _)
                 (sleep-for delay)
                 (if signal (signal signal nil) (pop unread-command-events)))))
      (condition-case err
          (setq result (gnosis-feed--dwell-supported-p))
        ((quit error) (setq caught (car err)))))
    (let ((leaked (seq-difference timer-list before)))
      (sleep-for 0.06)
      (mapc #'cancel-timer leaked)
      (list :caught caught :result result :timers (length leaked)
            :queue (copy-sequence unread-command-events)
            :cached gnosis-feed--dwell-supported))))

(ert-deftest gnosis-test-feed-dwell-probe-interrupted ()
  "A quit or error in the probe leaves no timer, event or cached verdict."
  (dolist (case '((0 quit) (0.06 quit) (0 error) (0.06 error)))
    (let ((gnosis-feed--dwell-supported 'unknown)
          (unread-command-events (list ?x 'mine)))
      (let ((out (gnosis-test-feed--probe (car case) (cadr case))))
        (should (eq (cadr case) (plist-get out :caught)))
        (should (equal 0 (plist-get out :timers)))
        (should (equal (list ?x 'mine) (plist-get out :queue)))
        (should (eq 'unknown (plist-get out :cached))))
      ;; An old probe event left in the queue never proves a later probe.
      (setq unread-command-events (list 'gnosis-feed--wakeup))
      (let ((out (cl-letf (((symbol-function 'sleep-for) #'ignore))
                   (gnosis-test-feed--probe 0))))
        (should-not (plist-get out :result))
        (should (equal '(gnosis-feed--wakeup) (plist-get out :queue)))))))

(ert-deftest gnosis-test-feed-dwell-probe-outcomes ()
  "Supported, unsupported and typed-over probes leave only foreign input."
  ;; Supported: the reader returns the probe's own event.
  (let ((gnosis-feed--dwell-supported 'unknown)
        (unread-command-events nil))
    (let ((out (gnosis-test-feed--probe 0.06)))
      (should (eq t (plist-get out :result)))
      (should (equal 0 (plist-get out :timers)))
      (should-not (plist-get out :queue))
      (should (eq t (plist-get out :cached)))))
  ;; Unsupported: the event is still queued at the timeout.
  (let ((gnosis-feed--dwell-supported 'unknown)
        (unread-command-events nil))
    (cl-letf (((symbol-function 'read-event)
               (lambda (&rest _) (sleep-for 0.06) nil)))
      (let ((noninteractive nil) (inhibit-message t))
        (should-not (gnosis-feed--dwell-supported-p))))
    (should-not unread-command-events)
    (should-not gnosis-feed--dwell-supported))
  ;; Typed over: the key returns in order, the verdict stays unknown.
  (let ((gnosis-feed--dwell-supported 'unknown)
        (unread-command-events (list ?a ?b)))
    (let ((out (gnosis-test-feed--probe 0.06)))
      (should-not (plist-get out :result))
      (should (equal 0 (plist-get out :timers)))
      (should (equal (list ?a ?b) (plist-get out :queue)))
      (should (eq 'unknown (plist-get out :cached)))))
  ;; Delayed timer: nothing queued yet, unknown, nothing left behind.
  (let ((gnosis-feed--dwell-supported 'unknown)
        (unread-command-events (list 'mine)))
    (cl-letf (((symbol-function 'read-event) (lambda (&rest _) nil)))
      (let ((noninteractive nil))
        (should-not (gnosis-feed--dwell-supported-p))))
    (sleep-for 0.06)
    (should (equal '(mine) unread-command-events))
    (should (eq 'unknown gnosis-feed--dwell-supported))))

(defvar-keymap gnosis-test-feed--custom-map
  :doc "A pre-existing overriding map the feed must preserve."
  "z" #'ignore)

(defun gnosis-test-feed--classic-keys ()
  "Run a classic review of 101 in the reused buffer; return its feedback keys."
  (let (facts)
    (gnosis-test-drive
     '("alpha")
     (list (lambda ()
             (with-current-buffer gnosis-review-buffer-name
               (setq facts
                     (list :spc (key-binding (kbd "SPC"))
                           :n (key-binding (kbd "n"))
                           :ret (key-binding (kbd "RET"))
                           :hook (and (memq #'gnosis-feed--pre-command pre-command-hook) t)
                           :read-only buffer-read-only
                           :custom (eq gnosis-test-feed--custom-map
                                       (alist-get 'gnosis-test-feed-custom
                                                  minor-mode-overriding-map-alist))
                           :entries (length minor-mode-overriding-map-alist)))))
           "n")
     (lambda () (gnosis-review-loop '(101) 'due)))
    facts))

(ert-deftest gnosis-test-feed-keys-scoped-to-burst ()
  "Every burst exit restores classic feedback keys in the reused buffer."
  (dolist (exit (list '("SPC" "SPC" "SPC") '("q") '("C-g")
                      (list (lambda () (error "Burst failure")))))
    (gnosis-test-feed
      (with-current-buffer (get-buffer-create gnosis-review-buffer-name)
        (gnosis-mode)
        (setq-local minor-mode-overriding-map-alist
                    (list (cons 'gnosis-test-feed-custom gnosis-test-feed--custom-map))))
      (ignore-errors
        (condition-case nil
            (gnosis-test-feed--run '("alpha" "beta" "gamma") exit)
          (quit nil)))
      (let ((facts (gnosis-test-feed--classic-keys)))
        (should-not (eq 'gnosis-feed-next (plist-get facts :spc)))
        (should (eq 'gnosis-review-feedback-next (plist-get facts :n)))
        (should (eq 'gnosis-review-feedback-menu (plist-get facts :ret)))
        (should-not (plist-get facts :hook))
        (should (plist-get facts :read-only))
        (should (plist-get facts :custom))
        (should (= 1 (plist-get facts :entries))))))
  ;; Retiring the buffer mid-burst leaves no feed keys or hooks behind.
  (gnosis-test-feed
    (let (buffer)
      (ignore-errors
        (condition-case nil
            (gnosis-test-feed--run
             '("alpha")
             (list (lambda ()
                     (setq buffer (get-buffer gnosis-review-buffer-name))
                     (with-current-buffer buffer (fundamental-mode))
                     (signal 'quit nil))))
          (quit nil)))
      (with-current-buffer buffer
        (should-not (rassq gnosis-feed-feedback-map minor-mode-overriding-map-alist))
        (should-not (memq #'gnosis-feed--pre-command pre-command-hook))))))

(ert-deftest gnosis-test-feed-summary-owned-buffer ()
  "Burst ends never claim a buffer the feed did not create and still own."
  (gnosis-test-feed
    (let ((draft (get-buffer-create "*Gnosis Feed*"))
          (run (lambda () (gnosis-test-feed--run '("alpha" "beta" "gamma")
                                                 '("SPC" "SPC" "SPC")))))
      (with-current-buffer draft (text-mode) (insert "UNSAVED USER DRAFT"))
      (funcall run)
      (with-current-buffer draft
        (should (eq 'text-mode major-mode))
        (should (equal "UNSAVED USER DRAFT" (buffer-string))))
      (let ((first gnosis-feed--end-buffer))
        (should (buffer-live-p first))
        (should-not (eq first draft))
        (with-current-buffer first
          (should (derived-mode-p 'gnosis-feed-end-mode))
          (should (string-match-p "3 cards" (buffer-string))))
        ;; Repeated summaries reuse the owned buffer.
        (funcall run)
        (should (eq first gnosis-feed--end-buffer))
        ;; A repurposed summary is never reclaimed.
        (with-current-buffer first
          (text-mode)
          (let ((inhibit-read-only t)) (erase-buffer) (insert "MINE")))
        (funcall run)
        (should-not (eq first gnosis-feed--end-buffer))
        (with-current-buffer first
          (should (equal "MINE" (buffer-string)))
          (should (eq 'text-mode major-mode))))
      ;; Neither is a renamed or file-visiting one.
      (let ((owned gnosis-feed--end-buffer))
        (with-current-buffer owned (rename-buffer "renamed summary"))
        (funcall run)
        (should-not (eq owned gnosis-feed--end-buffer)))
      (let ((owned gnosis-feed--end-buffer))
        (with-current-buffer owned
          (set-visited-file-name (expand-file-name "notes.txt" gnosis-dir) t)
          (set-buffer-modified-p nil))
        (funcall run)
        (should-not (eq owned gnosis-feed--end-buffer))
        (with-current-buffer owned (set-visited-file-name nil t))))))

(ert-deftest gnosis-test-feed-burst-subscribers-contained ()
  "Burst subscribers' errors, quits and buffer changes are contained."
  (gnosis-test-feed
    (let* ((seen nil)
           (gnosis-feed-burst-functions
            (list (lambda (_) (signal 'quit nil))
                  (lambda (_) (error "Subscriber failure"))
                  (lambda (facts)
                    (setq seen facts)
                    (set-buffer (get-buffer-create "*elsewhere*"))))))
      (gnosis-test-feed--run '("alpha" "beta" "gamma") '("SPC" "SPC" "SPC"))
      (should (eq gnosis-feed--end-buffer (current-buffer)))
      (should (equal 3 (plist-get seen :size)))
      (should (equal 3 (plist-get seen :reviewed)))
      (should (equal 3 (length (gnosis-test-feed--ratings)))))))

(ert-deftest gnosis-test-feed-early-stop-summary ()
  "An early stop reports reviewed cards out of the planned burst."
  (gnosis-test-feed
    (gnosis-test-feed--run '("alpha") '("q"))
    (with-current-buffer gnosis-feed--end-buffer
      (should (string-match-p "1 of 3 cards · 1 ✓" (buffer-string)))
      (should (string-match-p "Stopped early" (buffer-string))))
    (should (equal 1 (plist-get (gnosis-feed-snapshot) :reviewed)))))

(provide 'gnosis-test-feed)
;;; gnosis-test-feed.el ends here
