;;; romacs-agent-limits.el --- Each account's usage limits, at a glance  -*- lexical-binding: t -*-
;; SPDX-License-Identifier: GPL-3.0-or-later
;; Maintainer: Thanos Apollo <public@thanosapollo.org>

;;; Commentary:
;; What an orchestrator needs to place work: each harness's accounts in
;; their pool's order, each with its limit windows (five-hour, weekly) as
;; its harness last reported them (`romacs-agent--room'), its live
;; sessions, and whether new work draws on it now; then a line saying
;; what to do.  Read from the kept reports alone: turns make them, and an
;; account at its limit is asked again (`romacs-agent-check-room').
;; As T3 Code's Limits view, one row per window.

;;; Code:

(require 'cl-lib)
(require 'romacs-mcp)
(require 'romacs-agent)

(defconst romacs-agent-limits--labels
  '(("five_hour" . "5h") ("seven_day" . "week") ("seven_day_opus" . "week opus")
    ("seven_day_sonnet" . "week sonnet"))
  "Short names of the limit windows harnesses report.")

(defun romacs-agent-limits--label (name)
  (or (cdr (assoc name romacs-agent-limits--labels)) name))

(defun romacs-agent-limits--in (seconds)
  "SECONDS from now, compactly: 2d4h, 1h12m, 9m."
  (let* ((s (max 0 (round seconds))) (d (/ s 86400)) (h (/ (% s 86400) 3600)) (m (/ (% s 3600) 60)))
    (cond ((> d 0) (format "%dd%dh" d h)) ((> h 0) (format "%dh%02dm" h m)) (t (format "%dm" m)))))

(defun romacs-agent-limits--reset (resets now)
  "When RESETS (seconds since the epoch) is, from NOW: its local time and
how long until it, or that it is past."
  (cond ((not (numberp resets)) nil)
        ((<= resets now) "reset since")
        (t (format "%s (in %s)" (format-time-string (if (< (- resets now) 86400) "%H:%M" "%a %H:%M") resets)
                   (romacs-agent-limits--in (- resets now))))))

(defun romacs-agent-limits--windows (room now)
  "ROOM's windows (an instance's kept report) as rows: name, use in
percent, status and reset; a window past its reset is said to be."
  (let ((windows (or (plist-get room :windows)
                     ;; Kept before windows were: its one report.
                     (and room (list (cons (or (plist-get room :window) "limit") room))))))
    (mapcar (lambda (window)
              (let* ((report (cdr window)) (resets (plist-get report :resets))
                     (past (and (numberp resets) (<= resets now))))
                (romacs-mcp--object
                 "window" (romacs-agent-limits--label (car window))
                 "used" (if (and (numberp (plist-get report :used)) (not past))
                            (round (* 100 (plist-get report :used)))
                          :null)
                 "status" (if past "reset" (symbol-name (or (plist-get report :status) 'allowed)))
                 "resets" (or (romacs-agent-limits--reset resets now) :null))))
            (sort (copy-sequence windows) (lambda (a b) (string< (car a) (car b)))))))

(defun romacs-agent-limits--sessions (name)
  "The sessions running under instance NAME now: how many, and how many
of them are in a turn."
  (let ((live 0) (busy 0))
    (dolist (session (romacs-agent-sessions))
      (when (and (equal (romacs-agent-provider session) name)
                 (process-live-p (romacs-agent-connection session)))
        (cl-incf live)
        (when (memq (romacs-agent-state session) '(running waiting-approval)) (cl-incf busy))))
    (cons live busy)))

(defun romacs-agent-limits--fullest (room now)
  "ROOM's fullest window not past its reset: (LABEL USED RESETS), or nil."
  (let ((best nil))
    (pcase-dolist (`(,name . ,report) (plist-get room :windows))
      (let ((used (plist-get report :used)) (resets (plist-get report :resets)))
        (when (and (numberp used) (not (and (numberp resets) (<= resets now)))
                   (or (null best) (> used (nth 1 best))))
          (setq best (list (romacs-agent-limits--label name) used resets)))))
    best))

(defun romacs-agent-limits--suggestion (harness accounts pick now)
  "A line saying where HARNESS's new work goes, ACCOUNTS its pool's
instances, PICK the pool's pick, as of NOW."
  (let* ((rooms (mapcar (lambda (i) (cons (plist-get i :name) (romacs-agent-room-of (plist-get i :name)))) accounts))
         (spent (cl-remove-if (lambda (r) (romacs-agent-room (car r))) rooms))
         (pick-name (plist-get pick :name)))
    (cond
     ((null accounts) (format "No %s account is enabled." harness))
     ((cl-every (lambda (r) (null (cdr r))) rooms)
      (format "No limit reported yet: new work goes to %s." pick-name))
     ((= (length spent) (length rooms))
      (let ((first (car (sort (copy-sequence spent)
                              (lambda (a b) (< (or (plist-get (cdr a) :resets) most-positive-fixnum)
                                               (or (plist-get (cdr b) :resets) most-positive-fixnum)))))))
        (format "Every %s account is at its limit: %s is back first%s." harness (car first)
                (if (numberp (plist-get (cdr first) :resets))
                    (format ", in %s" (romacs-agent-limits--in (- (plist-get (cdr first) :resets) now)))
                  ""))))
     (t
      (let ((full (romacs-agent-limits--fullest (romacs-agent-room-of pick-name) now)))
        (concat (format "New work goes to %s" pick-name)
                (if (and full (>= (nth 1 full) 0.8))
                    (format " (%s at %d%%%s)" (nth 0 full) (round (* 100 (nth 1 full)))
                            (if (numberp (nth 2 full)) (format ", resets in %s" (romacs-agent-limits--in (- (nth 2 full) now))) ""))
                  "")
                (if spent
                    (format "; %s at the limit" (mapconcat #'car spent ", "))
                  "")
                "."))))))

(defun romacs-agent-limits (&optional harnesses)
  "Each harness's accounts and their limits (`romacs-agent-limits--windows'),
of HARNESSES (symbols; default every harness with an enabled account):
a list of tables, one a harness."
  (let* ((now (float-time))
         (harnesses (or harnesses
                        (delete-dups (mapcar (lambda (i) (plist-get i :harness))
                                             (cl-remove-if-not (lambda (i) (plist-get i :enabled)) (romacs-agent-instances)))))))
    (mapcar
     (lambda (harness)
       (let* ((accounts (romacs-agent-pool harness))
              (pick (and accounts (romacs-agent-pool-pick harness))))
         (romacs-mcp--object
          "harness" (symbol-name harness)
          "suggestion" (romacs-agent-limits--suggestion harness accounts pick now)
          "accounts" (vconcat
                      (mapcar (lambda (instance)
                                (let* ((name (plist-get instance :name)) (room (romacs-agent-room-of name))
                                       (sessions (romacs-agent-limits--sessions name)))
                                  (romacs-mcp--object
                                   "account" name
                                   "pick" (if (equal name (plist-get pick :name)) t :false)
                                   "room" (if (romacs-agent-room name) t :false)
                                   "sessions" (car sessions) "busy" (cdr sessions)
                                   "windows" (vconcat (romacs-agent-limits--windows room now))
                                   "reported" (if (plist-get room :time)
                                                  (format-time-string "%FT%T%z" (plist-get room :time))
                                                :null))))
                              accounts)))))
     harnesses)))

(defun romacs-agent-limits--tool (arguments)
  (romacs-mcp--instance arguments)
  (let ((harness (gethash "harness" arguments)))
    (when (and harness (not (gethash (intern-soft harness) romacs-agent--driver-table)))
      (user-error "No harness %s" harness))
    (romacs-mcp--object "harnesses" (vconcat (romacs-agent-limits (and harness (list (intern harness))))))))

(romacs-mcp-register-tool
 "romacs_limits"
 "Each harness's accounts in their pool's order: each limit window (5h, week...) with its use in percent, status \
and reset, the account's live sessions (and how many are in a turn), whether new work draws on it now (pick), and \
a line per harness saying where new work goes. Read from the limit reports kept: turns make them, and an account \
at its limit is asked again every few minutes when its harness can say (claude), so one that reset early gets room. harness: one harness (claude, codex)."
 (romacs-mcp--schema '(("instance" . "string") ("harness" . "string")) nil)
 #'romacs-agent-limits--tool
 (romacs-mcp--object "readOnlyHint" t))

(provide 'romacs-agent-limits)
;;; romacs-agent-limits.el ends here
