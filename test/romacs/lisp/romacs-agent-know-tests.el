;;; romacs-agent-know-tests.el --- Knowing the sessions as a view opens -*- lexical-binding: t; -*-
;; SPDX-License-Identifier: GPL-3.0-or-later
;; Maintainer: Thanos Apollo <public@thanosapollo.org>
(require 'ert)
(require 'cl-lib)
(require 'romacs-mcp)

(let ((romacs-mcp-call-function romacs-mcp-call-function)
      (romacs-mcp-tools (copy-alist romacs-mcp-tools)))
  (require 'romacs-agent))

(defun romacs-agent-know-tests--live (id)
  "A socket where the supervisor of session ID listens, as one running has."
  (let ((socket (romacs-agent--host-socket id)))
    (make-directory (file-name-directory socket) t)
    (write-region "" nil socket nil 'silent)))

(defun romacs-agent-know-tests--record (root id)
  "A session ID recorded under ROOT, its supervisor's socket there too."
  (let ((dir (expand-file-name id root)))
    (make-directory dir t)
    (write-region (json-serialize (romacs-mcp--object "id" id "harness" "claude" "title" id "cwd" root))
                  nil (expand-file-name "manifest.json" dir) nil 'silent)
    (write-region "" nil (expand-file-name "journal.jsonl" dir) nil 'silent)
    (romacs-agent-know-tests--live id)))

(defun romacs-agent-know-tests--recorded-since (root id)
  "A session ID recorded under ROOT since the others, its row written as
the editor that made it writes one (its first kept state)."
  (romacs-agent-know-tests--record root id)
  (when (fboundp 'romacs-agent-store-fill) (romacs-agent-store-fill)))

(ert-deftest romacs-agent-know-sessions-restores-once ()
  ;; The first view restores every session, attaching where a supervisor
  ;; runs; a later one reads no known session's records and attaches none
  ;; again, but knows a session recorded since.
  (let* ((root (make-temp-file "romacs-know-" t))
         (romacs-agent--sessions (make-hash-table :test #'equal))
         (romacs-agent--children (make-hash-table :test #'equal))
         (romacs-agent-event-functions nil)
         (one "aaaaaaaa-0000-4000-8000-000000000001") (two "aaaaaaaa-0000-4000-8000-000000000002")
         (three "aaaaaaaa-0000-4000-8000-000000000003")
         (manifests nil) (attached nil))
    (unwind-protect
        (cl-letf* ((read-json (symbol-function 'romacs-agent--read-json))
                   ((symbol-function 'romacs-agent--state-root) (lambda () root))
                   ((symbol-function 'romacs-agent--runtime-root) (lambda () (file-name-as-directory (expand-file-name "run" root))))
                   ((symbol-function 'romacs-agent--read-json)
                    (lambda (file &rest args)
                      ;; A session's own record, in the state root.
                      (when (equal file (expand-file-name "manifest.json"
                                                          (expand-file-name (file-name-nondirectory (directory-file-name (file-name-directory file))) root)))
                        (cl-pushnew (file-name-nondirectory (directory-file-name (file-name-directory file))) manifests :test #'equal))
                      (apply read-json file args)))
                   ((symbol-function 'romacs-agent--attach) (lambda (s &rest _) (push (romacs-agent-id s) attached) nil))
                   ;; Each fixture socket stands for a supervisor that answers.
                   ((symbol-function 'romacs-agent--listening-p) #'file-exists-p)
                   ((symbol-function 'romacs-agent--endpoint) #'ignore)
                   ((symbol-function 'romacs-agent--sweep) #'ignore))
          (romacs-agent-know-tests--record root one)
          (romacs-agent-know-tests--record root two)
          (romacs-agent-know-sessions)
          (should (equal (sort manifests #'string<) (list one two)))
          (should (equal (sort (delete-dups (copy-sequence attached)) #'string<) (list one two)))
          (setq manifests nil attached nil)
          ;; Detached again, its supervisor still there: a view attaches nothing.
          (setf (romacs-agent-state (romacs-agent-get one)) 'detached)
          (romacs-agent-know-sessions)
          (should-not manifests)
          (should-not attached)
          ;; Recorded since: known, and attached as a new one is.
          (romacs-agent-know-tests--recorded-since root three)
          ;; What its row's writing read is not a view's.
          (setq manifests nil)
          (romacs-agent-know-sessions)
          (should (equal manifests (list three)))
          (should (romacs-agent-get three))
          (should (equal attached (list three)))
          ;; Another registry, as a test binds: restored anew.
          (let ((romacs-agent--sessions (make-hash-table :test #'equal)))
            (setq manifests nil)
            (romacs-agent-know-sessions)
            (should (= (length manifests) 3))))
      (delete-directory root t))))

(ert-deftest romacs-agent-know-sessions-new-ones-are-restored-as-such ()
  ;; A session recorded since the restore is known as a restore knows one:
  ;; the restore's hook runs for it; a view finding none new runs nothing.
  (let* ((root (make-temp-file "romacs-know-" t))
         (romacs-agent--sessions (make-hash-table :test #'equal))
         (romacs-agent--children (make-hash-table :test #'equal))
         (romacs-agent-event-functions nil)
         (restored 0)
         (romacs-agent-restored-hook (list (lambda () (cl-incf restored)))))
    (unwind-protect
        (cl-letf (((symbol-function 'romacs-agent--state-root) (lambda () root))
                  ((symbol-function 'romacs-agent--runtime-root) (lambda () (file-name-as-directory (expand-file-name "run" root))))
                  ((symbol-function 'romacs-agent--attach) (lambda (&rest _) nil))
                  ((symbol-function 'romacs-agent--endpoint) #'ignore)
                  ((symbol-function 'romacs-agent--sweep) #'ignore))
          (romacs-agent-know-tests--record root "aaaaaaaa-0000-4000-8000-000000000001")
          (romacs-agent-know-sessions)
          (should (= restored 1))
          (romacs-agent-know-sessions)
          (should (= restored 1))
          (romacs-agent-know-tests--recorded-since root "aaaaaaaa-0000-4000-8000-000000000002")
          (romacs-agent-know-sessions)
          (should (= restored 2)))
      (delete-directory root t))))

(ert-deftest romacs-agent-know-sessions-unreachable-supervisor-is-retried ()
  ;; A supervisor whose socket is there but that does not answer is tried
  ;; again later on its own, not by a view's opening.
  (let* ((root (make-temp-file "romacs-know-" t))
         (romacs-agent--sessions (make-hash-table :test #'equal))
         (romacs-agent--reattaching (make-hash-table :test #'eq))
         (romacs-agent-event-functions nil)
         (session (romacs-agent--make :id "aaaaaaaa-0000-4000-8000-000000000009" :harness 'claude :state 'detached
                                      :dir (expand-file-name "s" root) :host-socket (expand-file-name "host.sock" root)))
         (scheduled nil))
    (unwind-protect
        (progn
          (make-directory (romacs-agent-dir session))
          (write-region "{\"seq\":1,\"t\":1.0,\"in\":{\"prompt\":\"p\"}}\n" nil
                        (expand-file-name "journal.jsonl" (romacs-agent-dir session)) nil 'silent)
          (write-region "" nil (romacs-agent-host-socket session) nil 'silent)
          (puthash (romacs-agent-id session) session romacs-agent--sessions)
          (setf (romacs-agent-journal-start session) nil)
          (cl-letf (((symbol-function 'run-at-time)
                     (lambda (_delay _repeat function &rest _) (push function scheduled))))
            (romacs-agent--attach session 0))
          (should (memq #'romacs-agent--reattach scheduled)))
      (delete-directory root t))))

;;; romacs-agent-know-tests.el ends here
