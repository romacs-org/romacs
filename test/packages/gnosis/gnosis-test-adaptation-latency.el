;;; gnosis-test-adaptation-latency.el --- Optional tutoring policy -*- lexical-binding: t; -*-

;; Copyright (C) 2026 Free Software Foundation, Inc.

(require 'gnosis-test-linked-native)

(ert-deftest gnosis-test-adaptation-latency-imminent-semantic ()
  "Leave imminent combined evaluation to teach from accepted native evidence."
  (dolist (prepared '(nil t))
    (gnosis-test-linked--with
        (vconcat (list (gnosis-test-linked--question "wrong"))
                 (when prepared (list (gnosis-test-linked--question "prepared")))
                 (list (gnosis-test-linked--question "semantic" "agent-eval")
                       (gnosis-test-linked--question "tail")))
      (gnosis-test-linked--answer state "closes")
      (let* ((before (gnosis-agent-review--copy (gnosis-review--state-data state)))
             (gnosis-agent-review-function
              (lambda (&rest _) (ert-fail "Avoidable optional request"))))
        (gnosis-agent-review--start-background state)
        (should-not gnosis-agent-review--background)
        ;; No invented attempt, result, error or plan for skipped work.
        (should (equal before (gnosis-review--state-data state)))
        (when prepared
          (gnosis-test-linked--answer state)
          (gnosis-agent-review--start-background state))
        (gnosis-agent-review--present state)
        (let* ((request (gnosis-agent-review--request state "evaluate" "semantic-request" "opens"))
               (transcript (plist-get request :transcript)))
          (should (equal "semantic" (plist-get (plist-get request :question) :id)))
          (should (= (if prepared 2 1) (length transcript)))
          (should (equal "failure" (plist-get (aref transcript 0) :outcome)))
          (should (equal "closes" (plist-get (aref transcript 0) :response)))
          (should (equal ["tail"] (vconcat (mapcar (lambda (q) (plist-get q :id))
                                                   (plist-get request :remaining))))))
        (should (zerop (length (plist-get (gnosis-agent-review--data state) :attempts)))))
      (gnosis-test-linked--empty-evidence))))

(ert-deftest gnosis-test-adaptation-latency-preserves-native-repair ()
  "Retain useful native-only adaptation and do not scan the entire queue."
  (dolist (distant-semantic '(nil t))
    (gnosis-test-linked--with
        (vector (gnosis-test-linked--question "wrong")
                (gnosis-test-linked--question "prepared")
                (gnosis-test-linked--question "tail")
                (gnosis-test-linked--question "distant" (if distant-semantic "agent-eval" "basic")))
      (gnosis-test-linked--answer state "closes")
      (let* (request
             (gnosis-agent-review-function
              (lambda (r resolve _reject)
                (setq request r)
                (funcall resolve (gnosis-test-linked--plan (gnosis-test-linked--question "repair")))
                #'ignore)))
        (gnosis-agent-review--start-background state)
        (should (equal "adapt" (plist-get request :phase)))
        (should (equal "failure" (plist-get (plist-get request :accepted) :outcome)))
        (gnosis-agent-review--present state)
        (let ((current (gnosis-agent-review--copy (plist-get (gnosis-agent-review--data state) :current)))
              (encounters (gnosis-agent-review--copy (plist-get (gnosis-agent-review--data state) :encounters))))
          (gnosis-agent-review--drain-background state)
          (should (equal '("prepared" "repair") (gnosis-review-state-remaining state)))
          (should (equal current (plist-get (gnosis-agent-review--data state) :current)))
          (should (equal encounters (plist-get (gnosis-agent-review--data state) :encounters))))
        (should (= 1 (length (plist-get (gnosis-agent-review--data state) :attempts)))))
      (gnosis-test-linked--empty-evidence))))

(provide 'gnosis-test-adaptation-latency)
;;; gnosis-test-adaptation-latency.el ends here
