;;; romacs-agent-limits-tests.el --- Each account's usage limits -*- lexical-binding: t; -*-
;; SPDX-License-Identifier: GPL-3.0-or-later
;; Maintainer: Thanos Apollo <public@thanosapollo.org>
(require 'ert)
(require 'cl-lib)
(require 'romacs-mcp)

;; Loading romacs-agent installs romacs-agent-tools: the endpoint's
;; registrations are left as they were.
(let ((romacs-mcp-call-function romacs-mcp-call-function)
      (romacs-mcp-tools (copy-alist romacs-mcp-tools)))
  (require 'romacs-agent)
  (require 'romacs-agent-claude)
  (require 'romacs-agent-codex)
  (require 'romacs-agent-limits))

(defmacro romacs-agent-limits-tests--with (&rest body)
  "BODY with Claude accounts claude and claude2 pooled, in a private state
root, their reports in a fresh table; `kept' counts writes to disk."
  (declare (indent 0))
  `(let* ((root (make-temp-file "romacs-limits-" t))
          (process-environment (cons (concat "XDG_STATE_HOME=" root "/state") process-environment))
          (romacs-agent-providers (list (cons "claude" (expand-file-name "one" root))
                                        (cons "claude2" (expand-file-name "two" root))))
          (romacs-agent-pools nil)
          (romacs-agent--sessions (make-hash-table :test #'equal))
          (romacs-agent--room (make-hash-table :test #'equal))
          (romacs-agent--room-loaded t)
          (kept 0))
     (dolist (dir '("one" "two")) (make-directory (expand-file-name dir root) t))
     (cl-letf (((symbol-function 'romacs-agent--keep-room) (lambda () (cl-incf kept))))
       (unwind-protect (progn ,@body) (delete-directory root t)))))

(defun romacs-agent-limits-tests--report (provider time &rest windows)
  "PROVIDER's harness reports WINDOWS at TIME: each (NAME STATUS USED RESETS)."
  (let ((session (romacs-agent--make :id (format "s-%s" provider) :provider provider :caught-up t)))
    (romacs-agent--note-room session (list :type 'rate-limit :time time
                                           :status (nth 1 (car windows)) :used (nth 2 (car windows))
                                           :resets (nth 3 (car windows)) :window (nth 0 (car windows))
                                           :windows (mapcar (lambda (w) (list :name (nth 0 w) :status (nth 1 w)
                                                                              :used (nth 2 w) :resets (nth 3 w)))
                                                            windows)))))

(ert-deftest romacs-agent-limits-drivers-report-every-window ()
  (let* ((info (romacs-mcp--object "status" "allowed" "rateLimitType" "five_hour" "resetsAt" 1000 "utilization" 0.8
                                   "unifiedWindows" (romacs-mcp--object
                                                     "five_hour" (romacs-mcp--object "status" "allowed" "utilization" 0.8
                                                                                     "resetsAt" 1000)
                                                     "seven_day" (romacs-mcp--object "status" "allowed_warning"
                                                                                     "utilization" 0.91 "resetsAt" 9000))))
         (windows (romacs-agent--claude-windows info)))
    (should (equal (mapcar (lambda (w) (plist-get w :name)) windows) '("five_hour" "seven_day")))
    (should (equal (plist-get (cadr windows) :status) 'warning))
    (should (equal (plist-get (cadr windows) :used) 0.91)))
  (should (equal (romacs-agent--codex-window (romacs-mcp--object "usedPercent" 100 "windowDurationMins" 10080 "resetsAt" 5))
                 '(:name "seven_day" :status rejected :used 1.0 :resets 5))))

(ert-deftest romacs-agent-limits-each-window-is-kept ()
  ;; Reports of one window then another: both kept, each its latest; the
  ;; disk written as a window's state or its use by 5% changes.
  (romacs-agent-limits-tests--with
    (romacs-agent-limits-tests--report "claude" 1.0 '("five_hour" allowed 0.40 1000))
    (romacs-agent-limits-tests--report "claude" 2.0 '("seven_day" allowed 0.60 9000))
    (romacs-agent-limits-tests--report "claude" 3.0 '("five_hour" allowed 0.41 1000))
    (let ((windows (plist-get (romacs-agent-room-of "claude") :windows)))
      (should (equal (sort (mapcar #'car windows) #'string<) '("five_hour" "seven_day")))
      (should (equal (plist-get (cdr (assoc "five_hour" windows)) :used) 0.41))
      (should (equal (plist-get (cdr (assoc "seven_day" windows)) :used) 0.60)))
    ;; The first two changed what is kept; a 1% step did not.
    (should (= kept 2))
    (romacs-agent-limits-tests--report "claude" 4.0 '("five_hour" allowed 0.47 1000))
    (should (= kept 3))))

(ert-deftest romacs-agent-limits-say-where-work-goes ()
  (romacs-agent-limits-tests--with
    (let* ((now (float-time))
           (live (make-process :name "romacs-limits-live" :command '("cat") :noquery t))
           (lane (romacs-agent--make :id "lane" :provider "claude" :state 'running :connection live)))
      (unwind-protect
          (progn
            (puthash "lane" lane romacs-agent--sessions)
            ;; Nothing reported yet.
            (should (equal (gethash "suggestion" (car (romacs-agent-limits '(claude))))
                           "No limit reported yet: new work goes to claude."))
            (romacs-agent-limits-tests--report "claude" now `("five_hour" allowed 0.85 ,(+ now 4320))
                                               `("seven_day" allowed 0.40 ,(+ now 300000)))
            (romacs-agent-limits-tests--report "claude2" now `("five_hour" rejected 1.0 ,(+ now 600)))
            (let* ((row (car (romacs-agent-limits '(claude))))
                   (accounts (append (gethash "accounts" row) nil))
                   (claude (car accounts)) (claude2 (cadr accounts)))
              (should (string-match-p "\\`New work goes to claude (5h at 85%, resets in 1h1[12]m); claude2 at the limit\\.\\'"
                                      (gethash "suggestion" row)))
              (should (eq (gethash "pick" claude) t))
              (should (eq (gethash "pick" claude2) :false))
              (should (= (gethash "sessions" claude) 1))
              (should (= (gethash "busy" claude) 1))
              (should (equal (mapcar (lambda (w) (list (gethash "window" w) (gethash "used" w) (gethash "status" w)))
                                     (gethash "windows" claude))
                             '(("5h" 85 "allowed") ("week" 40 "allowed"))))
              (should (string-match-p "(in 1h1[12]m)\\'" (gethash "resets" (aref (gethash "windows" claude) 0)))))
            ;; Both spent: who is back first.
            (romacs-agent-limits-tests--report "claude" (1+ now) `("five_hour" rejected 1.0 ,(+ now 4320)))
            (should (string-match-p "\\`Every claude account is at its limit: claude2 is back first, in [0-9]+m\\.\\'"
                                    (gethash "suggestion" (car (romacs-agent-limits '(claude))))))
            ;; A window past its reset: said to be, its use no longer told.
            (romacs-agent-limits-tests--report "claude2" (+ now 2) `("five_hour" rejected 1.0 ,(- now 5)))
            (let ((window (aref (gethash "windows" (aref (gethash "accounts" (car (romacs-agent-limits '(claude)))) 1)) 0)))
              (should (equal (gethash "status" window) "reset"))
              (should (eq (gethash "used" window) :null))))
        (delete-process live)))))

(ert-deftest romacs-agent-limits-the-tool ()
  (romacs-agent-limits-tests--with
    (let ((answer (romacs-agent-limits--tool (romacs-mcp--object "harness" "claude"))))
      (should (= (length (gethash "harnesses" answer)) 1)))
    (should-error (romacs-agent-limits--tool (romacs-mcp--object "harness" "nope")) :type 'user-error)))

;;; romacs-agent-limits-tests.el ends here
