;;; gnosis-test-timer-entry.el --- Timer entry refusal -*- lexical-binding: t; -*-

;; Copyright (C) 2026 Free Software Foundation, Inc.

;;; Commentary:
;; A timer must return, rather than retain a native review reader on its stack.

;;; Code:

(require 'ert)
(require 'gnosis-test-linked-native)

(defun gnosis-test-timer-entry--call (function)
  "Call FUNCTION from a real timer, returning its condition or nil."
  (let (done condition timer)
    (unwind-protect
        (progn
          (setq timer
                (run-at-time
                 0 nil (lambda ()
                         (condition-case err (funcall function)
                           ((error quit) (setq condition err)))
                         (setq done t))))
          (let ((deadline (+ (float-time) 3)))
            (while (and (not done) (< (float-time) deadline))
              (accept-process-output nil 0.01)))
          (should done)
          condition)
      (when timer (cancel-timer timer)))))

(ert-deftest gnosis-timer-entry-resume-preserves-pending-checkpoint ()
  (gnosis-test-linked--with (vector (gnosis-test-linked--question "q1"))
    ;; Capture an actual unaccepted native answer, not a fabricated checkpoint.
    (cl-letf (((symbol-function 'gnosis--read-string-with-input-method)
               (lambda (&rest _) "opens"))
              ((symbol-function 'gnosis-review--read-action)
               (lambda (_) (signal 'quit nil))))
      (should (eq 'paused
                  (condition-case nil (gnosis-agent-review--question-input state)
                    (quit 'paused)))))
    (setf (gnosis-review-state-launch-token state) "reserved-launch")
    (gnosis-review--save-session state)
    (let ((before (gnosis-review--session-target))
          (history (gnosis-select '* 'study-history))
          (buffers (buffer-list))
          reached)
      (cl-letf (((symbol-function 'gnosis-review--run-state)
                 (lambda (&rest _) (setq reached t))))
        (let ((condition (gnosis-test-timer-entry--call #'gnosis-review-resume)))
          (should (eq 'user-error (car condition)))
          (should (string-match-p "command loop" (error-message-string condition)))))
      (should-not reached)
      (should (equal before (gnosis-review--session-target)))
      (should (equal history (gnosis-select '* 'study-history)))
      (should (equal buffers (buffer-list)))
      (gnosis-test-linked--empty-evidence))))

(ert-deftest gnosis-timer-entry-internal-resume-refuses-before-validation ()
  (let (validated)
    (should (eq 'user-error
                (car (gnosis-test-timer-entry--call
                      (lambda ()
                        (gnosis-review--resume nil (lambda () (setq validated t))))))))
    (should-not validated)))

(ert-deftest gnosis-timer-entry-loop-refuses-before-selection ()
  (let (selected validated)
    (let ((condition
           (gnosis-test-timer-entry--call
            (lambda ()
              (gnosis-review-loop
               (lambda () (setq selected t) nil) 'practice nil
               (lambda () (setq validated t)))))))
      (should (eq 'user-error (car condition))))
    (should-not selected)
    (should-not validated)))

(ert-deftest gnosis-timer-entry-reader-refuses-before-context-mutation ()
  (with-temp-buffer
    (let* ((context (list :id "q1" :success t :result '(:mode practice)
                          :alternate nil))
           (before (copy-tree context))
           reached)
      (cl-letf (((symbol-function 'gnosis-review--override-result)
                 (lambda (&rest _) (setq reached t) (error "Unsafe preview"))))
        (should (eq 'user-error
                    (car (gnosis-test-timer-entry--call
                          (lambda () (gnosis-review--read-action context)))))))
      (should-not reached)
      (should (equal before context))
      (should-not gnosis-review--feedback)
      (should-not gnosis-review-feedback-mode))))

(ert-deftest gnosis-timer-entry-native-ordinary-feedback-services-events ()
  (skip-unless (not noninteractive))
  (gnosis-test-linked--with (vector (gnosis-test-linked--question "q1"))
    (let (timer driver child seen pending before observed)
      (cl-letf (((symbol-function 'gnosis--read-string-with-input-method)
                 (lambda (&rest _) "opens"))
                ((symbol-function 'gnosis-review-actions)
                 (lambda (_success _id result) (setq pending result))))
        (gnosis-agent-review--question-input state))
      (setq before (gnosis-review--session-target))
      (unwind-protect
          (progn
            (setq timer (run-at-time 0.05 nil (lambda () (push 'timer seen))))
            (setq child
                  (make-process
                   :name "gnosis-timer-entry-test" :connection-type 'pipe
                   :command '("sh" "-c" "sleep 0.1; printf ready") :noquery t
                   :filter (lambda (_process text)
                             (when (string-match-p "ready" text) (push 'filter seen)))
                   :sentinel (lambda (process _event)
                               (when (eq (process-status process) 'exit)
                                 (push 'sentinel seen)))))
            (let ((deadline (+ (float-time) 3)))
              (setq driver
                    (run-at-time
                     0.02 0.02
                     (lambda ()
                       (when (or (= (length seen) 3) (> (float-time) deadline))
                         (with-current-buffer review
                           (when gnosis-review-feedback-mode
                             (setq observed (list (copy-sequence seen)
                                                  (equal before (gnosis-review--session-target))))
                             (gnosis-review-feedback-cancel))))))))
            (should (eq 'paused
                        (condition-case nil
                            (gnosis-review--read-action
                             (list :id "q1" :success t :result pending :alternate nil))
                          (quit 'paused))))
            (should (equal (sort (car observed) #'string-lessp) '(filter sentinel timer)))
            (should (cadr observed))
            (should (equal before (gnosis-review--session-target)))
            (gnosis-test-linked--empty-evidence))
        (when timer (cancel-timer timer))
        (when driver (cancel-timer driver))
        (when (process-live-p child) (delete-process child))))))

(ert-deftest gnosis-timer-entry-queue-order-and-one-shot ()
  (gnosis-test-linked--with (vector (gnosis-test-linked--question "q1"))
    (let ((gnosis-agent--launches nil) (unread-command-events '(left right)) calls)
      (unwind-protect
          (progn
            (gnosis-agent-resume (gnosis-review-state-session-id state))
            (let* ((record (car gnosis-agent--launches))
                   (before (gnosis-review--session-target)))
              (should-not (gnosis-test-timer-entry--call
                           (lambda () (gnosis-agent--enqueue record))))
              (gnosis-agent--enqueue record)
              (should (equal '(left right) (seq-take unread-command-events 2)))
              (should (= 3 (length unread-command-events)))
              (should (equal before (gnosis-review--session-target)))
              (let ((event (nth 2 unread-command-events)))
                (should (eq record (cadr event)))
                (should-not (lookup-key special-event-map [gnosis-agent-launch]))
                (setq unread-command-events nil)
                (cl-letf (((symbol-function 'gnosis-review--resume)
                           (lambda (_target validate)
                             (funcall validate)
                             (push gnosis-review-buffer-name calls))))
                  (execute-kbd-macro (vector event event)))
                (should (equal calls (list review)))
                (should-not gnosis-agent--launches)
                (should (equal before (gnosis-review--session-target))))))
        (mapc #'gnosis-agent--release (copy-sequence gnosis-agent--launches))))))

(ert-deftest gnosis-timer-entry-stale-launch-preserves-pending ()
  (dolist (fault '(database directory token session killed created setup mode file state depth frame release cancel))
    (gnosis-test-linked--with (vector (gnosis-test-linked--question "q1"))
      (cl-letf (((symbol-function 'gnosis--read-string-with-input-method)
                 (lambda (&rest _) "opens"))
                ((symbol-function 'gnosis-review--read-action)
                 (lambda (_) (signal 'quit nil))))
        (condition-case nil (gnosis-agent-review--question-input state) (quit nil)))
      (let ((gnosis-agent--launches nil) (unread-command-events nil) reached replacement)
        (unwind-protect
            (progn
              (when (eq fault 'created) (kill-buffer review))
              (gnosis-agent-resume (gnosis-review-state-session-id state))
              (let ((record (car gnosis-agent--launches)))
                (gnosis-agent--enqueue record)
                (pcase fault
                  ('token (setf (plist-get record :token) "stale"))
                  ('session (setf (plist-get record :session-id) "stale"))
                  ('killed (kill-buffer review)
                           (setq replacement (get-buffer-create gnosis-review-buffer-name)))
                  ('created (setq replacement (get-buffer-create gnosis-review-buffer-name)))
                  ('setup (gnosis-review--setup-buffer nil))
                  ('mode (with-current-buffer review (fundamental-mode) (gnosis-mode)))
                  ('file (with-current-buffer review
                           (set-visited-file-name (expand-file-name "foreign" gnosis-dir) t)
                           (set-visited-file-name nil t)))
                  ('state (with-current-buffer review (setq gnosis-review--state nil)))
                  ('depth (setf (plist-get record :depth) (1+ (recursion-depth))))
                  ('frame (setf (plist-get record :frame) nil))
                  ('release (gnosis-agent--release record))
                  ('cancel (gnosis-agent-cancel (gnosis-review-state-session-id state))))
                (let ((before (gnosis-review--session-target))
                      (history (gnosis-select '* 'study-history))
                      (gnosis-db (if (eq fault 'database) nil gnosis-db))
                      (gnosis-dir (if (eq fault 'directory)
                                      (expand-file-name "other" gnosis-dir) gnosis-dir)))
                  (cl-letf (((symbol-function 'gnosis-review--resume)
                             (lambda (&rest _) (setq reached t))))
                    (let ((event (pop unread-command-events)))
                      (execute-kbd-macro (vector event event))))
                  (should-not reached)
                  (let ((gnosis-db (plist-get record :db)))
                    (should (equal before (gnosis-review--session-target)))
                    (should (equal history (gnosis-select '* 'study-history)))
                    (gnosis-test-linked--empty-evidence)))))
          (mapc #'gnosis-agent--release (copy-sequence gnosis-agent--launches))
          (when (buffer-live-p replacement) (kill-buffer replacement)))))))

(ert-deftest gnosis-timer-entry-renamed-owner-not-name-replacement ()
  (gnosis-test-linked--with (vector (gnosis-test-linked--question "q1"))
    (let ((gnosis-agent--launches nil) replacement destination)
      (unwind-protect
          (progn
            (gnosis-agent-resume (gnosis-review-state-session-id state))
            (with-current-buffer review (rename-buffer (generate-new-buffer-name "*renamed review*")))
            (setq replacement (get-buffer-create gnosis-review-buffer-name))
            (with-current-buffer replacement (insert "Foreign draft"))
            (cl-letf (((symbol-function 'gnosis-review--run-state)
                       (lambda (buffer _state) (setq destination buffer))))
              (gnosis-agent--launch (car gnosis-agent--launches)))
            (should (eq destination review))
            (should (equal "Foreign draft" (with-current-buffer replacement (buffer-string)))))
        (mapc #'gnosis-agent--release (copy-sequence gnosis-agent--launches))
        (when (buffer-live-p replacement) (kill-buffer replacement))))))

(ert-deftest gnosis-timer-entry-native-foreign-reader-preserves-launch ()
  (skip-unless (not noninteractive))
  (gnosis-test-linked--with (vector (gnosis-test-linked--question "q1"))
    (let ((gnosis-agent--launches nil) (unread-command-events nil) reached returned event)
      (unwind-protect
          (progn
            (gnosis-agent-resume (gnosis-review-state-session-id state))
            (let ((before (gnosis-review--session-target)))
              (with-temp-buffer
                (insert "Foreign draft")
                ;; The native D-Bus reader likewise preserves ordinary input
                ;; after reading it with redisplay inhibited in a foreign buffer.
                (setq event (let ((inhibit-redisplay t)) (read-event nil nil 2)))
                (setq returned t)
                (should (equal "Foreign draft" (buffer-string))))
              (should (eq (car-safe event) 'gnosis-agent-launch))
              (should (equal before (gnosis-review--session-target)))
              (cl-letf (((symbol-function 'gnosis-review--resume)
                         (lambda (_target validate)
                           (should returned)
                           (should-not (backtrace-frame 0 'timer-event-handler))
                           (should (eq this-command 'gnosis-agent--dispatch))
                           (funcall validate)
                           (setq reached t))))
                (execute-kbd-macro (vector event)))
              (should reached)
              (should-not gnosis-agent--launches)
              (should (equal before (gnosis-review--session-target)))))
        (mapc #'gnosis-agent--release (copy-sequence gnosis-agent--launches))))))

(provide 'gnosis-test-timer-entry)
;;; gnosis-test-timer-entry.el ends here
