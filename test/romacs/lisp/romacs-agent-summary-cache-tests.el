;;; romacs-agent-summary-cache-tests.el --- A summary kept until its session changes -*- lexical-binding: t; -*-
;; SPDX-License-Identifier: GPL-3.0-or-later
;; Maintainer: Thanos Apollo <public@thanosapollo.org>

;;; Commentary:
;; Each session's summary is made once and kept (`romacs-agent--summaries')
;; until a fact of it changes.  After each kind of change, made through
;; the path the editor takes, every kept summary equals one made anew.

;;; Code:

(require 'ert)
(require 'cl-lib)
(let ((load-path (cons (file-name-directory load-file-name) load-path)))
  (require 'romacs-agent-test-stored))
(require 'romacs-agent-summary)
(require 'romacs-agent-board)
(require 'romacs-agent-watch)
(require 'romacs-agent-codex)

(defconst romacs-agent-summary-cache-tests--ids
  '("ca000000-0000-4000-8000-000000000001" "ca000000-0000-4000-8000-000000000002"
    "ca000000-0000-4000-8000-000000000003" "ca000000-0000-4000-8000-000000000004")
  "The lead, its worker, another session, and one known by its row alone.")

(defmacro romacs-agent-summary-cache-tests--with (&rest body)
  "Run BODY with a private store and sessions `lead', `worker', `other'
loaded and `stored' a row alone."
  (declare (indent 0))
  `(romacs-agent-test-with-store
     (let* ((romacs-agent--sessions (make-hash-table :test #'equal))
            (romacs-agent--children (make-hash-table :test #'equal))
            (romacs-agent--summaries (make-hash-table :test #'equal))
            (romacs-agent-ui--children-changed (make-hash-table :test #'eq))
            (romacs-agent-ui--children-timer nil)
            (romacs-agent-ui--unloaded-waiting (make-hash-table :test #'equal))
            (romacs-agent-watch--watches (make-hash-table :test #'equal))
            (romacs-agent-log--chats (make-hash-table :test #'equal))
            (romacs-agent-writer-queued nil)
            (romacs-agent-event-functions (list #'romacs-agent-ui--on-event))
            (romacs-agent-approval-policy 'ask)
            (made (cl-mapcar
                   (lambda (id fields)
                     (let ((session (apply #'romacs-agent--make
                                           :id id :harness 'codex :provider "codex" :cwd "/p/cache/" :state 'idle
                                           :title (substring id -1) :dir (expand-file-name id (romacs-agent--state-root))
                                           fields)))
                       (make-directory (romacs-agent-dir session) t)
                       session))
                   romacs-agent-summary-cache-tests--ids
                   (list '(:options (:plan t) :seq 10)
                         (list :options (list :parent (car romacs-agent-summary-cache-tests--ids)) :done-seq 2)
                         '(:done-seq 4 :seen-seq 4)
                         '(:done-seq 1))))
            (lead (nth 0 made)) (worker (nth 1 made)) (other (nth 2 made)) (stored (nth 3 made)))
       (ignore worker other)
       (dolist (session (list lead worker other))
         (puthash (romacs-agent-id session) session romacs-agent--sessions)
         (romacs-agent--index session))
       (should (romacs-agent-store--put stored nil))
       (cl-letf (((symbol-function 'romacs-agent--note) #'ignore)
                 ((symbol-function 'romacs-agent--manifest-put) #'ignore)
                 ((symbol-function 'romacs-agent--keep-soon) #'ignore))
         (unwind-protect (progn (romacs-agent-summary-cache-tests--same) ,@body)
           (when romacs-agent-ui--children-timer (cancel-timer romacs-agent-ui--children-timer)))))))

(defun romacs-agent-summary-cache-tests--anew (id)
  "Session ID's summary (every one's with ID nil), none of them kept."
  (let ((romacs-agent--summaries (make-hash-table :test #'equal)))
    (romacs-agent-summary id)))

(defun romacs-agent-summary-cache-tests--same ()
  "Once the batch of rows runs, every session's kept summary, and the list
of all, equal those made anew; each is kept again for the next change."
  (romacs-agent-ui--children-flush)
  (dolist (id romacs-agent-summary-cache-tests--ids)
    (ert-info ((format "Session %s" id))
      (should (equal (romacs-agent-summary id) (romacs-agent-summary-cache-tests--anew id)))))
  (should (equal (romacs-agent-summary nil) (romacs-agent-summary-cache-tests--anew nil))))

(ert-deftest romacs-agent-summary-cache-follows-events ()
  (romacs-agent-summary-cache-tests--with
    (romacs-agent--raise lead '(:type input :seq 11 :request "p11" :text "Go"))
    (romacs-agent-summary-cache-tests--same)
    (romacs-agent--raise lead '(:type tool :state started :name "Read" :id "t1" :seq 12))
    (romacs-agent-summary-cache-tests--same)
    (puthash "r1" t (romacs-agent-approvals lead))
    (romacs-agent--raise lead '(:type approval :request-id "r1" :tool "Edit" :seq 13))
    (romacs-agent-summary-cache-tests--same)
    (romacs-agent--raise lead '(:type done :seq 14))
    (romacs-agent-summary-cache-tests--same)
    (romacs-agent--raise lead '(:type usage :context-used 900 :seq 15))
    (romacs-agent-summary-cache-tests--same)))

(ert-deftest romacs-agent-summary-cache-follows-the-plan-offer ()
  (romacs-agent-summary-cache-tests--with
    (let ((event (car (romacs-agent--codex-item
                       lead (romacs-mcp--object "type" "plan" "id" "plan-1" "text" "1. Do it") t))))
      (romacs-agent--raise lead (append event '(:seq 20))))
    (romacs-agent-summary-cache-tests--same)
    (should (eql (romacs-agent-summary-plan-offer (romacs-agent-summary (romacs-agent-id lead))) 20))
    (romacs-agent-set-plan lead nil)
    (romacs-agent-summary-cache-tests--same)))

(ert-deftest romacs-agent-summary-cache-follows-commands ()
  (romacs-agent-summary-cache-tests--with
    (romacs-agent--set-options other '(:effort "high"))
    (romacs-agent-summary-cache-tests--same)
    ;; Its views told only once drafts are answered: its summary at once.
    (let ((romacs-agent--held (list nil nil nil)))
      (romacs-agent--set-options other '(:effort "low"))
      (romacs-agent-summary-cache-tests--same))
    (romacs-agent-rename (romacs-agent-id other) "Renamed")
    (romacs-agent-summary-cache-tests--same)
    (romacs-agent-settle other)
    (romacs-agent-summary-cache-tests--same)
    (romacs-agent-settle other t)
    (setf (romacs-agent-done-seq other) 6)
    (romacs-agent-mark-seen other)
    (romacs-agent-summary-cache-tests--same)
    (romacs-agent--enqueue other (romacs-agent-item "Later"))
    (romacs-agent-summary-cache-tests--same)
    (romacs-agent-select other "next-model" "low")
    (romacs-agent-summary-cache-tests--same)
    (let ((connection (make-pipe-process :name "summary-cache" :noquery t)))
      (unwind-protect
          (progn (setf (romacs-agent-connection other) connection)
                 (romacs-agent--kept other connection)
                 (romacs-agent-summary-cache-tests--same))
        (delete-process connection)))))

(ert-deftest romacs-agent-summary-cache-follows-workers-and-watches ()
  (romacs-agent-summary-cache-tests--with
    (romacs-agent--raise worker '(:type input :seq 3 :request "w3" :text "Work"))
    (romacs-agent-summary-cache-tests--same)
    (should (romacs-agent-summary-children-rows (romacs-agent-summary (romacs-agent-id lead))))
    (romacs-agent--raise worker '(:type done :seq 4))
    (romacs-agent-summary-cache-tests--same)
    ;; No view of it following: its watch alone.
    (let ((romacs-agent-event-functions nil))
      (romacs-agent-watch--set other '((:unit "fixture.service")))
      (romacs-agent-summary-cache-tests--same))
    (should (equal (romacs-agent-summary-watch-unit (romacs-agent-summary (romacs-agent-id other))) "fixture.service"))))

(ert-deftest romacs-agent-summary-cache-follows-rows-and-loads ()
  (romacs-agent-summary-cache-tests--with
    (let ((id (romacs-agent-id stored)))
      ;; Its row written by any process: what its summary is made from.
      (sqlite-execute (romacs-agent-store--open) "update sessions set title = 'From elsewhere' where id = ?" (list id))
      (romacs-agent-summary-cache-tests--same)
      (should (equal (romacs-agent-summary-title (romacs-agent-summary id)) "From elsewhere"))
      (romacs-agent--write-json
       (expand-file-name "manifest.json" (romacs-agent-dir stored))
       (romacs-mcp--object "id" id "harness" "codex" "cwd" "/p/cache/" "title" "Loaded"))
      (should (romacs-agent-load-session id))
      (romacs-agent-summary-cache-tests--same)
      (should (equal (romacs-agent-summary-title (romacs-agent-summary id)) "Loaded")))))

(ert-deftest romacs-agent-summary-cache-follows-what-all-are-made-under ()
  (romacs-agent-summary-cache-tests--with
    (puthash "r1" t (romacs-agent-approvals other))
    (romacs-agent--changed other)
    (romacs-agent-summary-cache-tests--same)
    (setq romacs-agent-approval-policy 'allow)
    (romacs-agent-summary-cache-tests--same)
    (let ((romacs-agent-models '((codex ("m" :default-effort "low" :efforts ("low"))))))
      (romacs-agent-summary-cache-tests--same))))

;;; What a task record reads

(ert-deftest romacs-agent-summary-cache-follows-task-facts ()
  ;; The facts MCP reads of a task (`romacs-agent-tools--task'), as each
  ;; is written: a turn's end and its reply, a move armed at a limit and
  ;; let go (a move's wait is no fact of its summary), a renewal's new
  ;; conversation and its commit, its wake rule.
  (romacs-agent-summary-cache-tests--with
    (romacs-agent--raise worker '(:type input :seq 3 :request "w3" :text "Work"))
    (romacs-agent--raise worker '(:type done :seq 4 :text "Reply"))
    (romacs-agent-summary-cache-tests--same)
    (setf (romacs-agent-limited worker) '(:provider "codex" :text "Spent"))
    (romacs-agent--changed worker)
    (cl-letf (((symbol-function 'run-at-time) #'ignore))
      (romacs-agent-tools--arm-move worker))
    (romacs-agent-summary-cache-tests--same)
    (romacs-agent-tools--not-moved worker nil)
    (romacs-agent-summary-cache-tests--same)
    (romacs-agent--raise other '(:type renewed :to "native-2" :seq 5))
    (romacs-agent-summary-cache-tests--same)
    (should (equal (romacs-agent-summary-conversation (romacs-agent-summary (romacs-agent-id other))) "native-2"))
    (romacs-agent-renew--commit other "native-2")
    (romacs-agent-summary-cache-tests--same)
    (romacs-agent--set-options worker (plist-put (copy-sequence (romacs-agent-options worker)) :wake "none"))
    (romacs-agent-summary-cache-tests--same)
    (should (equal (romacs-agent-summary-wake-rule (romacs-agent-summary (romacs-agent-id worker))) "none"))))

;;; A resume at a limit's reset that goes nowhere

(defun romacs-agent-summary-cache-tests--resume-goes-nowhere (session before)
  "Arm SESSION's resume at its limit's reset, read its summary, call
BEFORE, let the resume fall due, and check its kept summary, and its
summary, say what one made anew does."
  (let* ((romacs-agent-tools--resumes (make-hash-table :test #'eq :weakness 'key))
         (id (romacs-agent-id session))
         (time (+ (float-time) 3600)))
    (romacs-agent-tools--arm-resume session time)
    (let ((timer (cdr (gethash session romacs-agent-tools--resumes))))
      (unwind-protect
          (progn
            (funcall before)
            (should (equal (romacs-agent-summary-resume-at (romacs-agent-summary id)) time))
            (romacs-agent-tools--resume session time)
            (let ((anew (romacs-agent-summary-cache-tests--anew id))
                  (kept (romacs-agent-summary-kept id)))
              (should-not (romacs-agent-summary-resume-at anew))
              (should (or (null kept) (equal kept anew)))
              (should (equal (romacs-agent-summary id) anew))
              (should (equal (romacs-agent-summary-kept id) anew))))
        (cancel-timer timer)))))

(ert-deftest romacs-agent-summary-cache-resume-settled-since ()
  (romacs-agent-summary-cache-tests--with
    (romacs-agent-summary-cache-tests--resume-goes-nowhere other (lambda () (romacs-agent-settle other)))))

(ert-deftest romacs-agent-summary-cache-resume-turned-off ()
  (romacs-agent-summary-cache-tests--with
    (let ((romacs-agent-resume-at-reset t))
      (romacs-agent-summary-cache-tests--resume-goes-nowhere
       other (lambda () (setq romacs-agent-resume-at-reset nil))))))

(ert-deftest romacs-agent-summary-cache-resume-no-longer-limited ()
  (romacs-agent-summary-cache-tests--with
    (let ((romacs-agent-resume-at-reset t))
      (should-not (romacs-agent-limited other))
      (romacs-agent-summary-cache-tests--resume-goes-nowhere other #'ignore))))

;;; One write path for each outside fact a summary reads

(defconst romacs-agent-summary-cache-tests--writers
  '((romacs-agent-tools--resumes . romacs-agent-tools--put-resume)
    (romacs-agent-watch--watches . romacs-agent-watch--put)
    (romacs-agent-ui--unloaded-waiting . romacs-agent-ui--put-unloaded-waiting)
    (romacs-agent-children-rows . romacs-agent-ui--summarize-children)
    (romacs-agent-children-summary . romacs-agent-ui--summarize-children)
    (romacs-agent-kept . romacs-agent--kept))
  "Each fact a summary reads from beside its session's record or event
path (a table, a slot, a process property), with the one function that
writes it and drops the summary (`romacs-agent--summary-stale').")

(defun romacs-agent-summary-cache-tests--written (form)
  "The facts of `romacs-agent-summary-cache-tests--writers' FORM writes
anywhere within it."
  (let ((facts (mapcar #'car romacs-agent-summary-cache-tests--writers)) (found nil))
    (cl-labels ((walk (x)
                  (when (consp x)
                    (let ((args (and (listp (cdr x)) (cdr x))))
                      (pcase (car x)
                        ((or 'puthash 'remhash 'clrhash)
                         (dolist (arg args) (when (memq arg facts) (push arg found))))
                        ((or 'setq 'setq-local 'set)
                         (dolist (arg args) (when (memq arg facts) (push arg found))))
                        ('setf (cl-loop for place in args by #'cddr
                                        do (when (memq (car-safe place) facts) (push (car place) found))
                                        (when (eq (car-safe place) 'gethash)
                                          (dolist (arg place) (when (memq arg facts) (push arg found))))))
                        ('process-put (when (equal (nth 1 args) '(quote romacs-agent-kept))
                                        (push 'romacs-agent-kept found)))))
                    (while (consp x) (walk (pop x))))))
      (walk form))
    found))

(ert-deftest romacs-agent-summary-cache-one-writer-each ()
  ;; A summary is kept until a fact of it changes: each fact it reads from
  ;; beside its record is written by one function, which drops it.
  (let ((dir (file-name-directory (locate-library "romacs-agent")))
        (writes nil))
    (dolist (file (directory-files dir t "\\`romacs-.*\\.el\\'"))
      (with-temp-buffer
        (insert-file-contents file)
        (goto-char (point-min))
        (condition-case nil
            (while t
              (let* ((form (read (current-buffer)))
                     (name (if (memq (car-safe form) '(defun defmacro defsubst cl-defun cl-defmethod))
                               (nth 1 form)
                             (car-safe form))))
                (dolist (fact (romacs-agent-summary-cache-tests--written form))
                  (push (list fact name (file-name-nondirectory file)) writes))))
          (end-of-file nil))))
    (dolist (writer romacs-agent-summary-cache-tests--writers)
      (should (assq (car writer) writes)))
    (dolist (write writes)
      (ert-info ((format "%S written in %S (%s)" (nth 0 write) (nth 1 write) (nth 2 write)))
        (should (eq (nth 1 write) (alist-get (nth 0 write) romacs-agent-summary-cache-tests--writers)))))))

(provide 'romacs-agent-summary-cache-tests)
;;; romacs-agent-summary-cache-tests.el ends here
