;;; gnosis-test-review-seams.el --- Presentation seams -*- lexical-binding: t; -*-

;; Copyright (C) 2026 Free Software Foundation, Inc.

;;; Commentary:
;; The review presentation seams default to the classic review exactly and
;; never change grading, acceptance or ownership when bound.

;;; Code:

(require 'ert)
(require 'gnosis-review-test-support)

(defmacro gnosis-test-seams (&rest body)
  "Run BODY with isolated data, two basic themata and owned buffers."
  (declare (indent 0) (debug t))
  `(gnosis-test-with-db
     (let ((buffers (buffer-list))
           (gnosis-review-buffer-name "*Gnosis Seams Test*")
           (gnosis-review--running nil)
           (gnosis-new-themata-limit nil)
           (gnosis-center-content nil)
           (gnosis-monkeytype-enable nil)
           (register-alist nil))
       (save-window-excursion
         (unwind-protect
             (progn
               (gnosis-test--add-basic-thema "Alpha question" "alpha" nil "Alpha note" 101)
               (gnosis-test--add-basic-thema "Beta question" "beta" nil nil 102)
               ,@body)
           (dolist (buffer (seq-difference (buffer-list) buffers))
             (when (buffer-live-p buffer) (kill-buffer buffer))))))))

(defun gnosis-test-seams--ratings ()
  "Return accepted scheduled ratings by thema."
  (sqlite-select gnosis-db "SELECT thema_id, rating FROM review_events ORDER BY rowid"))

(ert-deftest gnosis-test-review-seams-defaults ()
  "Defaults reproduce the classic header, summary, menu and labels."
  (should (eq gnosis-review-feedback-menu-style 'popup))
  (should (null gnosis-review-verdict-function))
  (should (null gnosis-review-event-functions))
  (should (eq gnosis-review-summary-function #'gnosis-review--show-summary))
  (gnosis-test-seams
    (with-current-buffer (gnosis-review--setup-buffer '(101))
      (should (equal header-line-format '(:eval (gnosis-review--header-line))))))
  (with-temp-buffer
    (gnosis-mode)
    (let ((gnosis-review-buffer-name (buffer-name))
          (gnosis-center-content nil))
      (gnosis-display-basic-answer "alpha" nil "beta")
      (gnosis-display-parathema "Note")
      (gnosis-display-next-review '(2026 10 20) nil)
      (should (equal (buffer-string)
                     "\n\nAnswer: alpha\nYour answer: beta\nNote\n\n\nNext review: 2026-10-20")))))

(ert-deftest gnosis-test-review-seams-nil-verdict-is-classic ()
  "A verdict function returning nil renders byte-identical feedback."
  (let (classic seamed)
    (dolist (function (list nil (lambda (_) nil)))
      (with-temp-buffer
        (gnosis-mode)
        (let ((gnosis-review-buffer-name (buffer-name))
              (gnosis-center-content nil)
              (gnosis-review-verdict-function function))
          (gnosis-display-basic-answer "alpha" nil "beta")
          (gnosis-display-correct-answer-mcq "one" "two")
          (gnosis-display-cloze-user-answer "three" t)
          (gnosis-display-parathema "Note")
          (gnosis-display-next-review '(2026 10 20) nil)
          (gnosis-display-next-review '(2026 10 30) t)
          (if function (setq seamed (buffer-string)) (setq classic (buffer-string))))))
    (should (equal-including-properties classic seamed))))

(ert-deftest gnosis-test-review-seams-verdict-slot-and-replace ()
  "Custom lines replace labels; schedules fill the slot and repaint in place."
  (with-temp-buffer
    (gnosis-mode)
    (let* ((gnosis-review-buffer-name (buffer-name))
           (gnosis-center-content nil)
           parts
           (gnosis-review-verdict-function
            (lambda (plist)
              (push plist parts)
              (pcase (plist-get plist :part)
                ('answer (concat "\n\nA:" (plist-get plist :answer)
                                 (propertize " " 'gnosis-review-status-slot t)))
                ('schedule (format "[%s %s]" (if (plist-get plist :success) "ok" "no")
                                   (gnosis-review--date-string (plist-get plist :date))))))))
      (gnosis-display-basic-answer "alpha" nil "beta")
      (gnosis-display-parathema "Note")
      (gnosis-display-next-review '(2026 10 13) nil)
      (should (equal (buffer-string) "\n\nA:alpha[no 2026-10-13] \nNote\n"))
      (gnosis-display-next-review '(2026 10 21) t)
      (should (equal (buffer-string) "\n\nA:alpha[ok 2026-10-21] \nNote\n"))
      (should (plist-get (car parts) :replace))
      (should (equal '(schedule schedule parathema answer)
                     (mapcar (lambda (p) (plist-get p :part)) parts)))
      (should-not (string-match-p "Answer:" (buffer-string))))))

(ert-deftest gnosis-test-review-seams-parathema-slot ()
  "A parathema prefix places the schedule before the explanation."
  (with-temp-buffer
    (gnosis-mode)
    (let ((gnosis-review-buffer-name (buffer-name))
          (gnosis-center-content nil)
          (gnosis-review-verdict-function
           (lambda (plist)
             (pcase (plist-get plist :part)
               ('parathema (concat "\n" (propertize " " 'gnosis-review-status-slot t)))
               ('schedule "S")))))
      (insert "Q")
      (gnosis-display-parathema "Note")
      (gnosis-display-next-review '(2026 10 13) t)
      (should (equal (buffer-string) "Q\nS \nNote\n")))))

(ert-deftest gnosis-test-review-seams-picture-reserve ()
  "On-demand menus reserve two lines instead of the popup's height."
  (with-temp-buffer
    (insert (propertize "P" 'gnosis-model-picture t) "\ntext")
    (cl-letf (((symbol-function 'window-text-pixel-size)
               (lambda (_window from to &rest _)
                 (cons 0 (if (and (= from 1) (= to 2)) 50 100))))
              ((symbol-function 'window-body-height) (lambda (&rest _) 1000))
              ((symbol-function 'window-body-width) (lambda (&rest _) 2000))
              ((symbol-function 'frame-char-height) (lambda (&rest _) 10)))
      (let ((gnosis-review--feedback-popup-height nil))
        (should (= 810 (gnosis-review--feedback-picture-size (selected-window))))
        (let ((gnosis-review-feedback-menu-style 'on-demand))
          (should (= 920 (gnosis-review--feedback-picture-size (selected-window)))))))))

(defun gnosis-test-seams--run (style answers keys)
  "Review both themata with menu STYLE, ANSWERS and KEYS.
Return (POPUPS EVENTS STATE)."
  (let* ((popups 0) events
         (gnosis-review-feedback-menu-style style)
         (gnosis-review-event-functions
          (list (lambda (_event) (error "Subscriber failure"))
                (lambda (event) (push event events)))))
    (cl-letf (((symbol-function 'gnosis-review--feedback-show)
               (lambda () (cl-incf popups))))
      (let ((state (gnosis-test-drive answers keys
                                      (lambda () (gnosis-review-loop '(101 102) 'due)))))
        (list popups (nreverse events) state)))))

(ert-deftest gnosis-test-review-seams-menu-style ()
  "On-demand feedback opens no popup and accepts only chosen actions."
  (gnosis-test-seams
    (pcase-let ((`(,popups ,_ ,_) (gnosis-test-seams--run 'popup '("alpha" "beta") '("n" "n"))))
      (should (= 2 popups)))
    (should (equal '((101 3) (102 3)) (gnosis-test-seams--ratings))))
  (gnosis-test-seams
    (pcase-let ((`(,popups ,_ ,_) (gnosis-test-seams--run 'on-demand '("alpha" "beta") '("?" "n" "n"))))
      ;; Only the explicit request opened the menu.
      (should (= 1 popups)))
    (should (equal '((101 3) (102 3)) (gnosis-test-seams--ratings)))))

(ert-deftest gnosis-test-review-seams-events ()
  "Events are typed, ordered, contained and carry no answer text."
  (gnosis-test-seams
    (pcase-let ((`(,_ ,events ,state) (gnosis-test-seams--run 'popup '("wrong" "beta") '("o" "n" "n"))))
      (should (equal '((presented 101) (answered 101) (overridden 101) (accepted 101)
                       (presented 102) (answered 102) (accepted 102))
                     (mapcar (lambda (e) (list (plist-get e :type) (plist-get e :thema-id)))
                             events)))
      (dolist (event events)
        (should (equal 1 (plist-get event :api)))
        (should (equal (gnosis-review-state-session-id state) (plist-get event :session-id)))
        (should (equal gnosis-test--db-file (plist-get event :database)))
        (should (eq 'due (plist-get event :mode)))
        (should (equal "basic" (plist-get event :kind)))
        (should (integerp (plist-get event :t-ms)))
        (should-not (string-match-p "alpha\\|beta\\|wrong\\|question"
                                    (prin1-to-string event))))
      (should (eq :false (or (plist-get (nth 1 events) :success) :false)))
      (should (integerp (plist-get (nth 1 events) :latency-ms)))
      (should (eq t (plist-get (nth 2 events) :success)))
      ;; The override was accepted as the current outcome.
      (should (eq t (plist-get (nth 3 events) :success)))
      (should (equal '((101 3) (102 3)) (gnosis-test-seams--ratings))))))

(ert-deftest gnosis-test-review-seams-header-and-summary ()
  "Header and summary functions are installed by the shared runner."
  (gnosis-test-seams
    (let* ((summaries nil) header
           (gnosis-review-header-function (lambda () "custom header"))
           (gnosis-review-summary-function (lambda (state) (push state summaries))))
      (gnosis-test-drive '("alpha" "beta")
                         (list (lambda () (with-current-buffer gnosis-review-buffer-name (setq header (eval (cadr header-line-format) t))))
                               "n" "n")
                         (lambda () (gnosis-review-loop '(101 102) 'due)))
      (should (equal "custom header" header))
      (should (= 1 (length summaries)))
      (should (= 2 (gnosis-review-state-reviewed (car summaries))))
      (should-not (get-buffer "*Gnosis Study Summary*")))))

(ert-deftest gnosis-test-review-seams-snapshot ()
  "Snapshots describe the feedback reader without answer text."
  (gnosis-test-seams
    (should (equal "idle" (plist-get (gnosis-review-snapshot) :state)))
    (let (seen)
      (gnosis-test-drive
       '("wrong" "beta" "alpha")
       (list (lambda () (setq seen (gnosis-review-snapshot)))
             "n" "n" "n")
       (lambda () (gnosis-review-loop '(101 102) 'due)))
      (should (equal "feedback" (plist-get seen :state)))
      (should (equal 101 (plist-get seen :thema-id)))
      (should (eq :false (plist-get seen :success)))
      (should (stringp (plist-get seen :date)))
      (should (equal "basic" (plist-get seen :kind)))
      (should-not (string-match-p "alpha" (prin1-to-string seen))))))

(ert-deftest gnosis-test-review-seams-subscriber-quit-contained ()
  "Error and quit in a subscriber never stop delivery or review."
  (dolist (signal-type '(error quit))
    (gnosis-test-seams
      (let* ((seen nil)
             (gnosis-review-event-functions
              (list (lambda (_) (signal signal-type '("subscriber")))
                    (lambda (event) (push (plist-get event :type) seen)))))
        (gnosis-test-drive '("wrong" "beta") '("o" "n" "n")
                           (lambda () (gnosis-review-loop '(101 102) 'due)))
        (should (equal '(presented answered overridden accepted
                         presented answered accepted)
                       (nreverse seen)))
        (should (equal '((101 3) (102 3)) (gnosis-test-seams--ratings)))))))

(ert-deftest gnosis-test-review-seams-event-index-and-isolation ()
  "Accepted events count attempts before them; payloads never alias state."
  (gnosis-test-seams
    (let* ((indexes nil) (downstream nil)
           (gnosis-review-event-functions
            (list (lambda (event) (aset (plist-get event :session-id) 0 ?X))
                  (lambda (event)
                    (push (list (plist-get event :type) (plist-get event :index)) indexes)
                    (setq downstream (plist-get event :session-id)))))
           (state (gnosis-test-drive '("alpha" "beta") '("n" "n")
                                     (lambda () (gnosis-review-loop '(101 102) 'due)))))
      (should (equal '((presented 0) (answered 0) (accepted 0)
                       (presented 1) (answered 1) (accepted 1))
                     (nreverse indexes)))
      (should-not (string-prefix-p "X" (gnosis-review-state-session-id state)))
      (should-not (string-prefix-p "X" downstream)))))

(ert-deftest gnosis-test-review-seams-standalone-validator-first ()
  "A standalone formatter that repurposes the buffer gets no stale insert."
  (with-temp-buffer
    (gnosis-mode)
    (let ((gnosis-review-buffer-name (buffer-name))
          (gnosis-review-verdict-function
           (lambda (_) (text-mode) (insert "successor") "OLD-FEEDBACK")))
      (should-error (gnosis-display-basic-answer "answer" t nil))
      (should (equal "successor" (buffer-string))))))

(ert-deftest gnosis-test-review-seams-snapshot-owned ()
  "Snapshots use only the retained owner and never open a database."
  (gnosis-test-seams
    (let* ((buf (gnosis-review--setup-buffer '(101)))
           (state (buffer-local-value 'gnosis-review--state buf))
           (session (gnosis-review-state-session-id state)))
      ;; Another session running does not make this buffer current.
      (let* ((gnosis-review--running "OTHER-SESSION")
             (snap (gnosis-review-snapshot buf)))
        (should (equal "idle" (plist-get snap :state)))
        (should-not (plist-get snap :running)))
      ;; The kind comes from the owner's connection, not a colliding database.
      (let ((gnosis-review--running session))
        (gnosis-test-with-db
          (gnosis-add-thema-fields "cloze" "New {answer}" nil '("answer") "" nil 0 nil nil 101)
          (let ((snap (gnosis-review-snapshot buf)))
            (should (equal "question" (plist-get snap :state)))
            (should (equal "basic" (plist-get snap :kind)))
            ;; The session identity is a copy.
            (aset (plist-get snap :session-id) 0 ?X)
            (should (equal session (gnosis-review-state-session-id state))))))
      ;; No current connection: nothing is created or opened.
      (let* ((dir (make-temp-file "gnosis-snapshot-" t))
             (gnosis-dir (file-name-as-directory dir))
             (gnosis-db nil)
             (gnosis-review--running session))
        (unwind-protect
            (progn
              (gnosis-review-snapshot buf)
              (gnosis-review-snapshot)
              (should-not (file-exists-p (expand-file-name "gnosis.db" dir)))
              (should-not gnosis-db))
          (delete-directory dir t)))
      ;; A retired owner is idle and not running.
      (let ((gnosis-review--running session))
        (with-current-buffer buf
          (set-visited-file-name (expand-file-name "successor.txt" gnosis-dir) t)
          (let ((snap (gnosis-review-snapshot buf)))
            (should (equal "idle" (plist-get snap :state)))
            (should-not (plist-get snap :running)))
          (set-buffer-modified-p nil))))))

(ert-deftest gnosis-test-review-seams-snapshot-closed-owner ()
  "A closed owner connection yields an unknown kind, not another database."
  (gnosis-test-seams
    (let* ((buf (gnosis-review--setup-buffer '(101)))
           (state (buffer-local-value 'gnosis-review--state buf))
           (gnosis-review--running (gnosis-review-state-session-id state))
           (other (sqlite-open)))
      (unwind-protect
          (progn
            (setf (gnosis-review-state-database state) other)
            (sqlite-close other)
            (let ((snap (gnosis-review-snapshot buf)))
              (should (equal 101 (plist-get snap :thema-id)))
              (should-not (plist-get snap :kind))))
        (setf (gnosis-review-state-database state) gnosis-db)))))

(provide 'gnosis-test-review-seams)
;;; gnosis-test-review-seams.el ends here
