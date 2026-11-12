;;; romacs-agent-launch-tests.el --- A session whose supervisor never comes up -*- lexical-binding: t; -*-
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
  (require 'romacs-agent-tools)
  (require 'romacs-agent-claude))

(defmacro romacs-agent-launch-tests--with (&rest body)
  "BODY with the idle session `parent' and its task `child', which has a
directory under `root' and no supervisor; `delivered' collects wakes."
  (declare (indent 0))
  `(let* ((root (make-temp-file "romacs-launch-" t))
          (romacs-agent--sessions (make-hash-table :test #'equal))
          (romacs-agent--children (make-hash-table :test #'equal))
          (romacs-agent-tools--pending (make-hash-table :test #'equal))
          (romacs-agent-tools--waking (make-hash-table :test #'equal))
          (romacs-agent-tools--answered (make-hash-table :test #'equal))
          (romacs-agent-tools--scheduled (make-hash-table :test #'equal))
          (parent (romacs-agent--make :id "pppppppp-0000-4000-8000-000000000000" :harness 'claude :state 'idle
                                      :dir (expand-file-name "parent" root)))
          (child (romacs-agent--make :id "cccccccc-0000-4000-8000-000000000000" :harness 'claude :title "janitor"
                                     :dir (expand-file-name "child" root) :cwd root
                                     :host-socket (expand-file-name "run/host.sock" root)
                                     :options (list :parent "pppppppp-0000-4000-8000-000000000000")))
          (delivered nil))
     (make-directory (romacs-agent-dir child) t)
     (dolist (s (list parent child)) (puthash (romacs-agent-id s) s romacs-agent--sessions))
     (romacs-agent--index child)
     (cl-letf (((symbol-function 'romacs-agent-tools--deliver)
                (lambda (_session text done &optional _valid) (push text delivered) (funcall done)))
               ((symbol-function 'run-at-time) #'ignore))
       (unwind-protect (progn ,@body) (delete-directory root t)))))

(ert-deftest romacs-agent-launch-a-missing-companion-is-an-error ()
  (let ((invocation-directory (file-name-as-directory (make-temp-file "romacs-bin-" t)))
        (exec-path nil))
    (unwind-protect
        (let ((err (should-error (romacs-agent--check-companions) :type 'user-error)))
          (should (string-match-p "romacs-agent-host is neither beside .* nor on PATH" (cadr err)))
          ;; Checked before anything is made: no half-made session.
          (let ((romacs-agent--sessions (make-hash-table :test #'equal))
                (romacs-agent-providers '(("claude" . nil))))
            (should-error (romacs-agent-start temporary-file-directory "go" "t" "claude") :type 'user-error)
            (should (= (hash-table-count romacs-agent--sessions) 0))))
      (delete-directory invocation-directory t))))

(ert-deftest romacs-agent-launch-never-started-is-a-failed-end ()
  (romacs-agent-launch-tests--with
    ;; The last look for its supervisor, with no record of one.
    (romacs-agent--attach child 0)
    (should (eq (romacs-agent-state child) 'unstarted))
    (let ((task (romacs-agent-tools--task child)))
      (should (equal (gethash "status" task) "failed"))
      (should (equal (gethash "state" task) "unstarted"))
      (should (string-match-p "never started" (gethash "result" task))))
    ;; Its parent is woken with the failure.
    (romacs-agent-tools--wake child)
    (romacs-agent-tools--flush parent)
    (should (equal (plist-get (romacs-agent-tools--wake-parse (car delivered)) :state) "failed"))
    ;; One that ran once is detached, as before.
    (let ((ran (romacs-agent--make :id "dddddddd-0000-4000-8000-000000000000" :harness 'claude
                                   :dir (expand-file-name "ran" root) :host-socket (expand-file-name "run/x.sock" root))))
      (make-directory (romacs-agent-dir ran))
      (write-region "" nil (expand-file-name "journal.jsonl" (romacs-agent-dir ran)))
      (romacs-agent--attach ran 0)
      (should (eq (romacs-agent-state ran) 'detached)))))

(ert-deftest romacs-agent-launch-a-delegate-that-never-starts-fails-the-call ()
  (romacs-agent-launch-tests--with
    (let ((poll (cadr (romacs-agent-tools--started child (lambda () (romacs-agent-tools--task child))))))
      ;; Starting: no answer yet.
      (setf (romacs-agent-state child) 'starting)
      (should-not (funcall poll))
      (romacs-agent--attach child 0)
      (should-error (funcall poll) :type 'user-error)
      ;; The caller has the failure: no wake besides.
      (romacs-agent-tools--wake child)
      (romacs-agent-tools--flush parent)
      (should-not delivered))))

(ert-deftest romacs-agent-launch-each-failure-wakes-and-says-failed ()
  (romacs-agent-launch-tests--with
    ;; The first failure the delegate's caller had; a retry's fails again.
    (romacs-agent--attach child 0)
    (romacs-agent-tools--answer child (romacs-agent-done-seq child))
    (romacs-agent-tools--wake child)
    (romacs-agent-tools--flush parent)
    (should-not delivered)
    (setf (romacs-agent-state child) 'starting)
    (romacs-agent--attach child 0)
    (romacs-agent-tools--wake child)
    (romacs-agent-tools--flush parent)
    (should (= (length delivered) 1))
    ;; A message waiting for it does not make it running.
    (romacs-agent--enqueue child (list "hello" nil nil nil))
    (should (equal (romacs-agent-tools--status child) "failed"))))

(ert-deftest romacs-agent-launch-waits-for-attach-as-long-as-it-tries ()
  (let ((scheduled nil) (session (romacs-agent--make :id "x" :harness 'claude)))
    (cl-letf (((symbol-function 'run-at-time) (lambda (_time _repeat _fn _s _f tries) (push tries scheduled))))
      (romacs-agent--when-attached session #'ignore))
    (should (equal scheduled (list (1- romacs-agent--launch-tries))))))

(ert-deftest romacs-agent-launch-a-restart-knows-it-never-started ()
  (romacs-agent-launch-tests--with
    (let ((dir (romacs-agent-dir child)))
      (romacs-agent--write-json (expand-file-name "manifest.json" dir)
                                (romacs-mcp--object "id" "eeeeeeee-0000-4000-8000-000000000000" "harness" "claude"
                                                    "cwd" root "title" "janitor"
                                                    "hostSocket" (expand-file-name "run/none.sock" root)))
      (let ((loaded (romacs-agent--load dir)))
        (should (eq (romacs-agent-state loaded) 'unstarted))
        (should (equal (romacs-agent-tools--status loaded) "failed"))))))

(ert-deftest romacs-agent-launch-resume-begins-a-conversation-that-never-was ()
  (romacs-agent-launch-tests--with
    (let ((resumed nil))
      (cl-letf (((symbol-function 'romacs-agent--argv) (lambda (_session resume) (push resume resumed) '("harness")))
                ((symbol-function 'romacs-agent--launch) #'ignore)
                ((symbol-function 'romacs-agent--attach) #'ignore)
                ((symbol-function 'romacs-agent--check-companions) #'ignore))
        (setf (romacs-agent-state child) 'unstarted)
        (romacs-agent-resume child)
        ;; A message reached it once: its conversation is there to resume.
        (setf (romacs-agent-input-seq child) 4)
        (romacs-agent-resume child))
      (should (equal resumed '(4 nil))))))

(provide 'romacs-agent-launch-tests)
;;; romacs-agent-launch-tests.el ends here
