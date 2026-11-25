;;; romacs-agent-service-tests.el --- The agents service and a client of it -*- lexical-binding: t; -*-
;; SPDX-License-Identifier: GPL-3.0-or-later
;; Maintainer: Thanos Apollo <public@thanosapollo.org>
(require 'ert)
(require 'cl-lib)
(require 'romacs-mcp)

(let ((romacs-mcp-call-function romacs-mcp-call-function)
      (romacs-mcp-tools (copy-alist romacs-mcp-tools)))
  (require 'romacs-agent)
  (require 'romacs-agent-service)
  (require 'romacs-agent-client))

(defmacro romacs-agent-service-tests--with (&rest body)
  "BODY with a registry of its own, no store, `mk' making a session in it
\(ID STATE &rest FIELDS), a service on a socket of its own (`socket'),
and `wait' waiting until its form holds (5 s at most)."
  (declare (indent 0))
  `(let* ((root (make-temp-file "rsv" t))
          (socket (expand-file-name "s.sock" root))
          (romacs-agent--sessions (make-hash-table :test #'equal))
          (romacs-agent--children (make-hash-table :test #'equal))
          (romacs-agent-event-functions nil)
          (romacs-agent-client-rows-functions nil)
          (now (float-time)))
     (cl-letf (((symbol-function 'romacs-agent-store-unloaded-rows) #'ignore)
               ((symbol-function 'romacs-agent-store-row) #'ignore))
       (cl-flet ((mk (id state &rest fields)
                   (let ((session (apply #'romacs-agent--make
                                         (append fields (list :id id :harness 'claude :provider "claude" :title id
                                                              :state state :cwd "/p/one/" :last-time now)))))
                     (puthash id session romacs-agent--sessions)
                     (romacs-agent--index session)
                     session)))
         (unwind-protect
             (cl-macrolet ((wait (form) `(let ((deadline (+ (float-time) 5)))
                                            (while (and (not ,form) (< (float-time) deadline))
                                              (accept-process-output nil 0.02))
                                            ,form)))
               ,@body)
           (romacs-agent-service-stop)
           (delete-directory root t))))))

(defun romacs-agent-service-tests--row (id &optional parent done)
  "A store row (`romacs-agent-store--row-columns') of an idle Claude
session ID in /p/, delegated by PARENT, its turns done through DONE."
  (list id nil "claude" nil nil nil "/p/" parent "idle" nil nil nil (or done 0)
        nil nil nil 0 nil nil nil nil 0 nil))

(defun romacs-agent-service-tests--raw (socket)
  "A raw connection to SOCKET whose lines are kept, in order, under its
`lines' property."
  (let ((process (make-network-process :name "rsv-raw" :family 'local :service socket :noquery t
                                       :coding 'no-conversion :sentinel #'ignore
                                       :filter (lambda (process chunk)
                                                 (process-put process 'partial (concat (process-get process 'partial) chunk))
                                                 (let ((lines (split-string (process-get process 'partial) "\n")))
                                                   (process-put process 'partial (car (last lines)))
                                                   (process-put process 'lines (append (process-get process 'lines)
                                                                                      (mapcar #'json-parse-string (butlast lines)))))))))
    process))

(ert-deftest romacs-agent-service-mirrors-the-board ()
  ;; A client's rows are the board's: each key made by the service, as the
  ;; board makes it; a running session reads running, from the service.
  (romacs-agent-service-tests--with
    (mk "busy" 'running)
    (mk "done" 'idle :done-seq 3)
    (mk "kid" 'running :options '(:parent "busy"))
    (romacs-agent-service-start socket)
    (let ((client (romacs-agent-client-connect socket)))
      (should (wait (romacs-agent-client-revision client)))
      (let ((rows (romacs-agent-client-rows client)))
        (should (= (hash-table-count rows) 3))
        (dolist (id '("busy" "done" "kid"))
          (should (equal (gethash "key" (gethash id rows))
                         (symbol-name (romacs-agent-summary-state-key (romacs-agent-summary id))))))
        (should (equal (gethash "key" (gethash "busy" rows)) "working"))
        (should (equal (gethash "state" (gethash "kid" rows)) "running"))
        (should (equal (gethash "children" (gethash "busy" rows)) 1))
        (should (equal (gethash "parent" (gethash "kid" rows)) "busy"))
        (should (eq (gethash "effort" (gethash "kid" rows)) :null))))))

(ert-deftest romacs-agent-service-tells-what-changed ()
  ;; A session's change reaches the client as the next revision, its row
  ;; alone; a session gone, by its id.
  (romacs-agent-service-tests--with
    (let ((busy (mk "busy" 'running)))
      (mk "other" 'idle)
      (romacs-agent-service-start socket)
      (let* ((client (romacs-agent-client-connect socket)) (seen nil))
        (should (wait (romacs-agent-client-revision client)))
        (let ((revision (romacs-agent-client-revision client)))
          (add-hook 'romacs-agent-client-rows-functions (lambda (_ changed removed) (push (list changed removed) seen)))
          (setf (romacs-agent-state busy) 'idle)
          ;; As the editor tells a change made beside its events.
          (romacs-agent--changed busy)
          (should (wait (eql (romacs-agent-client-revision client) (1+ revision))))
          (should (equal (gethash "state" (gethash "busy" (romacs-agent-client-rows client))) "idle"))
          (should (equal seen '((("busy") nil))))
          ;; Gone without an event: the next whole pass says so.
          (remhash "other" romacs-agent--sessions)
          (romacs-agent-service--refresh t)
          (should (wait (eql (romacs-agent-client-revision client) (+ 2 revision))))
          (should-not (gethash "other" (romacs-agent-client-rows client)))
          (should (equal (car seen) '(nil ("other")))))))))

(ert-deftest romacs-agent-service-counts-a-new-child ()
  ;; A task delegated after the list was made: its parent's row counts it,
  ;; though only the child raised an event.
  (romacs-agent-service-tests--with
    (mk "lead" 'running)
    (romacs-agent-service-start socket)
    (let ((client (romacs-agent-client-connect socket)))
      (should (wait (romacs-agent-client-revision client)))
      (should (equal (gethash "children" (gethash "lead" (romacs-agent-client-rows client))) 0))
      (let ((kid (mk "kid" 'running :options '(:parent "lead"))))
        (run-hook-with-args 'romacs-agent-event-functions kid '(:type changed)))
      (should (wait (gethash "kid" (romacs-agent-client-rows client))))
      (should (equal (gethash "children" (gethash "lead" (romacs-agent-client-rows client))) 1)))))

(ert-deftest romacs-agent-service-pages-a-list ()
  ;; A list over the reply's budget comes in pages of whole rows, at least
  ;; one each; a token used again, or never made, is stale.
  (romacs-agent-service-tests--with
    (dotimes (i 5) (mk (format "s%d" i) 'idle))
    (let ((romacs-agent-service--unit-budget 1))
      (romacs-agent-service-start socket)
      (let ((client (romacs-agent-client-connect socket)) (pages 0) (stale nil))
        (advice-add 'romacs-agent-client--listed :before (lambda (&rest _) (cl-incf pages)) '((name . rsv-count)))
        (unwind-protect
            (progn
              (should (wait (romacs-agent-client-revision client)))
              (should (= (hash-table-count (romacs-agent-client-rows client)) 5))
              (should (= pages 5)))
          (advice-remove 'romacs-agent-client--listed 'rsv-count))
        (romacs-agent-client-request client "agents/page" (romacs-agent-rpc-object "more" "nope")
                                     (lambda (_ error) (setq stale error)))
        (should (wait stale))
        (should (equal (gethash "code" stale) -32017))))))

(ert-deftest romacs-agent-service-wants-hello-first ()
  (romacs-agent-service-tests--with
    (romacs-agent-service-start socket)
    (let ((raw (romacs-agent-service-tests--raw socket)))
      (process-send-string raw "{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"agents/nothing\",\"params\":{}}\n")
      (process-send-string raw "{\"jsonrpc\":\"2.0\",\"id\":2,\"method\":\"agents/hello\",\"params\":{\"protocol\":\"romacs-agents\",\"version\":[2,0]}}\n")
      (process-send-string raw "not json\n")
      (should (wait (= (length (process-get raw 'lines)) 3)))
      (pcase-let ((`(,first ,second ,third) (process-get raw 'lines)))
        (should (equal (gethash "code" (gethash "error" first)) -32002))
        (should (equal (gethash "kind" (gethash "data" (gethash "error" second))) "unsupported"))
        (should (equal (gethash "code" (gethash "error" third)) -32700)))
      (delete-process raw))))

(ert-deftest romacs-agent-rpc-never-waits-on-a-peer-that-stops-reading ()
  ;; A peer that never reads: what its socket does not take is queued,
  ;; never waited for, and the connection is closed once the queue is
  ;; over its limit.
  (romacs-agent-service-tests--with
    (romacs-agent-service-start socket)
    (let ((raw (make-network-process :name "rsv-stuck" :family 'local :service socket :noquery t
                                     :coding 'no-conversion :sentinel #'ignore)))
      ;; The stuck peer never reads: its filter is nil and it is stopped.
      (set-process-filter raw nil)
      (stop-process raw)
      (should (wait (car romacs-agent-service--peers)))
      (let* ((rpc (car romacs-agent-service--peers))
             (chunk (make-string (* 512 1024) ?x))
             (start (float-time)) (sent 0))
        (while (and (romacs-agent-rpc-live-p rpc) (< sent 64))
          (romacs-agent-rpc-send rpc (romacs-agent-rpc-object "x" chunk))
          (cl-incf sent))
        (should (< (- (float-time) start) 1.0))
        (should-not (romacs-agent-rpc-live-p rpc))
        (should (< sent 64)))
      (delete-process raw))))

(ert-deftest romacs-agent-service-keeps-rows-small ()
  ;; A title of any length: a row carries its first 1,024 characters.
  (romacs-agent-service-tests--with
    (mk "long" 'idle :title (make-string 5000 ?t))
    (romacs-agent-service-start socket)
    (let ((row (gethash "long" (romacs-agent-service--list-rows romacs-agent-service--list))))
      (should (= (length (gethash "title" row)) 1024)))))

(ert-deftest romacs-agent-service-pages-removed-ids-too ()
  ;; Removed ids are units of a page as rows are: a page holds at least
  ;; one, never more than its budget, and the rest continue it.
  (let* ((romacs-agent-service--unit-budget 1)
         (rpc (romacs-agent-rpc-make nil (romacs-agent-service--peer-make)))
         (first (romacs-agent-service--paged rpc (list :revision 3 :reset nil :rows nil :removed '("a" "b" "c"))))
         (second (romacs-agent-service--paged
                  rpc (cddr (assoc (gethash "more" first) (romacs-agent-service--peer-tokens (romacs-agent-rpc-data rpc)))))))
    (should (equal (gethash "removed" first) ["a"]))
    (should (stringp (gethash "more" first)))
    (should (equal (gethash "removed" second) ["b"]))
    (should (eq (gethash "reset" second) :false))))

(ert-deftest romacs-agent-service-a-watch-lists-rows-as-registered ()
  ;; As recent, the rows come in the order the sessions were registered,
  ;; whatever their ids or directories.
  (romacs-agent-service-tests--with
    (dolist (id '("52" "51" "50" "4f" "4e" "4d")) (mk id 'idle :dir (expand-file-name id root)))
    (romacs-agent-service-start socket)
    (let ((raw (romacs-agent-service-tests--raw socket)))
      (process-send-string raw "{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"agents/hello\",\"params\":{\"protocol\":\"romacs-agents\",\"version\":[1,0]}}\n")
      (process-send-string raw "{\"jsonrpc\":\"2.0\",\"id\":2,\"method\":\"agents/sessions.watch\",\"params\":{\"since\":0}}\n")
      (should (wait (= (length (process-get raw 'lines)) 2)))
      (should (equal (mapcar (lambda (row) (gethash "id" row))
                             (gethash "rows" (gethash "result" (cadr (process-get raw 'lines)))))
                     '("52" "51" "50" "4f" "4e" "4d")))
      (delete-process raw))))

(ert-deftest romacs-agent-service-since-is-of-a-generation ()
  ;; A revision of another generation reads as none: the whole list.
  (romacs-agent-service-tests--with
    (mk "a" 'idle)
    (romacs-agent-service-start socket)
    (let ((raw (romacs-agent-service-tests--raw socket)))
      (process-send-string raw "{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"agents/hello\",\"params\":{\"protocol\":\"romacs-agents\",\"version\":[1,0]}}\n")
      (process-send-string raw (format "{\"jsonrpc\":\"2.0\",\"id\":2,\"method\":\"agents/sessions.watch\",\"params\":{\"since\":1,\"generation\":\"%s\"}}\n"
                                       romacs-agent-service--generation))
      (process-send-string raw "{\"jsonrpc\":\"2.0\",\"id\":3,\"method\":\"agents/sessions.watch\",\"params\":{\"since\":1,\"generation\":\"another\"}}\n")
      (should (wait (= (length (process-get raw 'lines)) 3)))
      (pcase-let ((`(,_ ,same ,other) (process-get raw 'lines)))
        (should (eq (gethash "reset" (gethash "result" same)) :false))
        (should (equal (gethash "rows" (gethash "result" same)) []))
        (should (eq (gethash "reset" (gethash "result" other)) t))
        (should (= (length (gethash "rows" (gethash "result" other))) 1)))
      (delete-process raw))))

(ert-deftest romacs-agent-service-remakes-a-parent-with-its-child ()
  ;; A worker's event remakes its parent's row too: its key follows its
  ;; workers'.
  (romacs-agent-service-tests--with
    (mk "lead" 'idle)
    (mk "kid" 'running :options '(:parent "lead"))
    (romacs-agent-service-start socket)
    (should (equal (sort (mapcar #'car (romacs-agent-service--rows-of romacs-agent-service--list '("kid"))) #'string<)
                   '("kid" "lead")))))

(ert-deftest romacs-agent-service-looks-up-a-batch-before-its-keys ()
  ;; An unloaded parent and its finished child found in one poll, the
  ;; child first: the child's key reads its parent known, as the board's.
  (romacs-agent-service-tests--with
    (romacs-agent-service-start socket)
    (let ((stored (list (cons "p" (romacs-agent-service-tests--row "p"))
                        (cons "c" (romacs-agent-service-tests--row "c" "p" 4)))))
      (cl-letf (((symbol-function 'romacs-agent-store-row) (lambda (id) (cdr (assoc id stored)))))
        (let ((updates (romacs-agent-service--rows-of romacs-agent-service--list '("c" "p"))))
          (should (equal (gethash "key" (cdr (assoc "c" updates))) "completed")))))))

(ert-deftest romacs-agent-service-leaves-the-board-its-own-view ()
  ;; The service's whole pass reads the sessions' summaries, as the board
  ;; does: an unloaded parent's finished child reads completed, as on the
  ;; board.
  (romacs-agent-service-tests--with
    (let ((stored (list (romacs-agent-service-tests--row "p") (romacs-agent-service-tests--row "c" "p" 4))))
      (cl-letf (((symbol-function 'romacs-agent-store-unloaded-rows) (lambda (&rest _) stored))
                ((symbol-function 'romacs-agent-store-row) (lambda (id) (assoc id stored))))
        (romacs-agent-service-start socket)
        (romacs-agent-service--refresh t))
      (should (equal (gethash "key" (gethash "c" (romacs-agent-service--list-rows romacs-agent-service--list)))
                     "completed")))))

(ert-deftest romacs-agent-service-remakes-a-child-when-its-parent-comes ()
  ;; A finished child's key reads its parent known: a parent come since
  ;; the child's row was made remakes the child's key, completed.
  (romacs-agent-service-tests--with
    (mk "c-late" 'idle :done-seq 4 :options '(:parent "p-late"))
    (let ((stored nil))
      (cl-letf (((symbol-function 'romacs-agent-store-unloaded-rows) (lambda (&rest _) stored))
                ((symbol-function 'romacs-agent-store-row) (lambda (id) (assoc id stored))))
        (romacs-agent-service-start socket)
        (romacs-agent-service--refresh t)
        (should (equal (gethash "key" (gethash "c-late" (romacs-agent-service--list-rows romacs-agent-service--list)))
                       "idle"))
        (push (romacs-agent-service-tests--row "p-late") stored)
        (let ((updates (romacs-agent-service--rows-of romacs-agent-service--list '("p-late"))))
          (should (equal (gethash "key" (cdr (assoc "c-late" updates))) "completed")))))))

(ert-deftest romacs-agent-service-a-child-first-in-a-batch-reads-its-parent-come ()
  ;; The child looked up before its parent in one batch: its key reads the
  ;; parent known all the same.
  (romacs-agent-service-tests--with
    (mk "c-first" 'idle :done-seq 4 :options '(:parent "p-first"))
    (let ((stored nil))
      (cl-letf (((symbol-function 'romacs-agent-store-unloaded-rows) (lambda (&rest _) stored))
                ((symbol-function 'romacs-agent-store-row) (lambda (id) (assoc id stored))))
        (romacs-agent-service-start socket)
        (romacs-agent-service--refresh t)
        (should (equal (gethash "key" (gethash "c-first" (romacs-agent-service--list-rows romacs-agent-service--list)))
                       "idle"))
        (push (romacs-agent-service-tests--row "p-first") stored)
        (let ((updates (romacs-agent-service--rows-of romacs-agent-service--list '("c-first" "p-first"))))
          (should (equal (gethash "key" (cdr (assoc "c-first" updates))) "completed")))))))

(ert-deftest romacs-agent-service-remakes-a-child-when-its-parent-goes-unsaid ()
  ;; A parent gone with no event: the next whole pass says so, and its
  ;; finished child's key no longer reads it known.
  (romacs-agent-service-tests--with
    (mk "p-gone" 'idle)
    (mk "c-gone" 'idle :done-seq 4 :options '(:parent "p-gone"))
    (romacs-agent-service-start socket)
    (romacs-agent-service--refresh t)
    (should (equal (gethash "key" (gethash "c-gone" (romacs-agent-service--list-rows romacs-agent-service--list)))
                   "completed"))
    (remhash "p-gone" romacs-agent--sessions)
    (romacs-agent-service--refresh t)
    (should-not (gethash "p-gone" (romacs-agent-service--list-rows romacs-agent-service--list)))
    (should (equal (gethash "key" (gethash "c-gone" (romacs-agent-service--list-rows romacs-agent-service--list)))
                   "idle"))))

(ert-deftest romacs-agent-service-runs-its-whole-pass-when-idle ()
  (romacs-agent-service-tests--with
    (romacs-agent-service-start socket)
    (setq romacs-agent-service--whole-at 0 romacs-agent-service--dirty nil)
    (when (timerp romacs-agent-service--refresh-timer) (cancel-timer romacs-agent-service--refresh-timer))
    (setq romacs-agent-service--refresh-timer nil)
    (romacs-agent-service--poll)
    ;; Due: once the editor is idle, never while the user types.
    (should (timerp romacs-agent-service--idle-timer))
    (should-not romacs-agent-service--refresh-timer)))

(ert-deftest romacs-agent-service-answers-a-long-request-in-bounds ()
  ;; A method name near a message's limit: answered, its error bounded.
  (romacs-agent-service-tests--with
    (romacs-agent-service-start socket)
    (let ((raw (romacs-agent-service-tests--raw socket)))
      (process-send-string raw "{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"agents/hello\",\"params\":{\"protocol\":\"romacs-agents\",\"version\":[1,0]}}\n")
      (process-send-string raw (format "{\"jsonrpc\":\"2.0\",\"id\":2,\"method\":\"%s\",\"params\":{}}\n"
                                       (make-string (- (* 1024 1024) 100) ?m)))
      (should (wait (= (length (process-get raw 'lines)) 2)))
      (should (equal (gethash "code" (gethash "error" (cadr (process-get raw 'lines)))) -32601))
      (delete-process raw))))

(ert-deftest romacs-agent-service-starts-beside-agents-already-running ()
  ;; The agents' runtime directory made by others with the usual umask:
  ;; the service's socket is in a directory of its own.
  (let* ((root (make-temp-file "rsv-run" t))
         (process-environment (cons (concat "XDG_RUNTIME_DIR=" root) process-environment))
         (romacs-agent--sessions (make-hash-table :test #'equal)))
    (unwind-protect
        (cl-letf (((symbol-function 'romacs-agent-store-unloaded-rows) #'ignore))
          (make-directory (romacs-agent--runtime-root) t)
          (set-file-modes (romacs-agent--runtime-root) #o755)
          (should (equal (romacs-agent-service-start) (romacs-agent-service-socket)))
          (should (file-exists-p (romacs-agent-service-socket))))
      (romacs-agent-service-stop)
      (delete-directory root t))))

(defun romacs-agent-service-tests--log (client)
  "What CLIENT hears of its sessions, oldest first, under its `log' property."
  (let ((log (list nil)))
    (add-hook 'romacs-agent-client-session-functions
              (lambda (c id notice)
                (when (and (eq c client) (not (equal notice '(opening))))
                  (setcar log (append (car log) (list (cons id notice)))))))
    log))

(ert-deftest romacs-agent-service-sends-a-session-s-record-units ()
  ;; Open: the row and the cursor live events follow from, the last
  ;; record kept; then each record unit past it, once whole, in revision
  ;; order.  A replay's copy of the session sends nothing, nor a unit at
  ;; or below the cursor (a supervisor's replay on attach).
  (romacs-agent-service-tests--with
    (let ((busy (mk "busy" 'running :seq 7)))
      (romacs-agent-service-start socket)
      (advice-add 'romacs-agent-service--last-seq :override (lambda (_) 7) '((name . rsv-kept)))
      (let* ((client (romacs-agent-client-connect socket)) (log (romacs-agent-service-tests--log client)))
        (should (wait (romacs-agent-client-revision client)))
        (romacs-agent-client-open client "busy")
        (should (wait (car log)))
        (pcase-let ((`("busy" opened ,reply) (car (car log))))
          (should (equal (gethash "seq" (gethash "cursor" reply)) 7))
          (should (equal (gethash "id" (gethash "row" reply)) "busy")))
        (run-hook-with-args 'romacs-agent-event-functions busy '(:type delta :seq 6 :message "r" :text "replayed" :last t))
        (run-hook-with-args 'romacs-agent-event-functions busy '(:type delta :seq 8 :message "m" :text "Hel"))
        (run-hook-with-args 'romacs-agent-event-functions (romacs-agent--scratch busy)
                            '(:type delta :seq 3 :message "m" :text "old" :last t))
        (run-hook-with-args 'romacs-agent-event-functions busy '(:type delta :seq 8 :message "m" :text "lo" :last t))
        (should (wait (cl-find 'event (car log) :key #'cadr)))
        (let* ((notices (seq-filter (lambda (entry) (eq (cadr entry) 'event)) (car log)))
               (params (nth 2 (car notices))))
          (should (= (length notices) 1))
          (should (equal (gethash "seq" params) 8))
          (should (equal (mapcar (lambda (event) (gethash "text" event)) (gethash "events" params)) '("Hel" "lo"))))
        (advice-remove 'romacs-agent-service--last-seq 'rsv-kept)))))

(ert-deftest romacs-agent-service-cuts-a-long-string ()
  ;; A tool's output over 64 KiB: its first 64 KiB, the event saying how
  ;; long it was.
  (let* ((sent (romacs-agent-service--bounded
                (list (list :type 'tool :seq 4 :id "t" :name "Bash" :state 'completed
                            :output (make-string 100000 ?o)))))
         (event (aref sent 0)))
    (should (= (string-bytes (gethash "output" event)) (* 64 1024)))
    (should (equal (gethash "/output" (gethash "truncated" event)) 100000))))

(ert-deftest romacs-agent-client-reopens-on-a-missed-revision ()
  ;; A notice past the next revision: something was missed, and the
  ;; session is opened again, the notice not applied.
  (romacs-agent-service-tests--with
    (mk "busy" 'running)
    (romacs-agent-service-start socket)
    (let* ((client (romacs-agent-client-connect socket)) (opened 0) (events 0))
      (add-hook 'romacs-agent-client-session-functions
                (lambda (_ _ notice) (pcase (car notice) ('opened (cl-incf opened)) ('event (cl-incf events)))))
      (should (wait (romacs-agent-client-revision client)))
      (romacs-agent-client-open client "busy")
      (should (wait (= opened 1)))
      (romacs-agent-client--session-notice client "agents/event"
                                           (romacs-agent-rpc-object "session" "busy" "revision" 3 "seq" 9 "events" []))
      (should (wait (= opened 2)))
      (should (= events 0)))))

(ert-deftest romacs-agent-client-applies-only-its-own-open ()
  ;; Open, close and open again before the first reply: the first reply
  ;; never takes the second open's place.
  (romacs-agent-service-tests--with
    (mk "busy" 'running)
    (romacs-agent-service-start socket)
    (let* ((client (romacs-agent-client-connect socket)) (opened 0))
      (add-hook 'romacs-agent-client-session-functions (lambda (_ _ notice) (when (eq (car notice) 'opened) (cl-incf opened))))
      (should (wait (romacs-agent-client-revision client)))
      (romacs-agent-client-open client "busy")
      (romacs-agent-client-close-session client "busy")
      (romacs-agent-client-open client "busy")
      (should (wait (= opened 1)))
      (accept-process-output nil 0.2)
      (should (= opened 1))
      (should (vectorp (gethash "busy" (romacs-agent-client-sessions client)))))))

(ert-deftest romacs-agent-service-tells-a-gap-it-never-sent ()
  ;; A session loaded past a subscription's cutoff with no events (a
  ;; worker's fold): told at once as a gap, its bounds the records never
  ;; sent live; then live units follow it.
  (romacs-agent-service-tests--with
    (let ((busy (mk "busy" 'running :seq 9)))
      (romacs-agent-service-start socket)
      (advice-add 'romacs-agent-service--last-seq :override (lambda (_) 7) '((name . rsv-kept)))
      (let* ((client (romacs-agent-client-connect socket)) (log (romacs-agent-service-tests--log client))
             (events (lambda () (mapcar (lambda (entry) (nth 2 entry))
                                        (seq-filter (lambda (entry) (eq (cadr entry) 'event)) (car log))))))
        (unwind-protect
            (progn
              (should (wait (romacs-agent-client-revision client)))
              (romacs-agent-client-open client "busy")
              (should (wait (car log)))
              (romacs-agent-service--loaded busy nil)
              (should (wait (funcall events)))
              (let ((gap (gethash "gap" (car (funcall events)))))
                (should (equal (list (gethash "after" gap) (gethash "upto" gap)) '(7 9)))
                (should (equal (gethash "events" (car (funcall events))) [])))
              (run-hook-with-args 'romacs-agent-event-functions busy '(:type delta :seq 10 :message "m" :text "x" :last t))
              (should (wait (= (length (funcall events)) 2)))
              (should-not (gethash "gap" (cadr (funcall events)))))
          (advice-remove 'romacs-agent-service--last-seq 'rsv-kept))))))

(ert-deftest romacs-agent-service-refuses-an-unknown-session ()
  (romacs-agent-service-tests--with
    (romacs-agent-service-start socket)
    (cl-letf (((symbol-function 'romacs-agent-store-row-session) #'ignore))
      (let ((client (romacs-agent-client-connect socket)) (answer nil))
        (should (wait (romacs-agent-client-revision client)))
        (romacs-agent-client-request client "agents/session.open" (romacs-agent-rpc-object "id" "nope")
                                     (lambda (_ error) (setq answer error)))
        (should (wait answer))
        (should (equal (gethash "code" answer) -32010))))))

(ert-deftest romacs-agent-service-tells-a-subscriber-its-row ()
  (romacs-agent-service-tests--with
    (let ((busy (mk "busy" 'running)))
      (romacs-agent-service-start socket)
      (let* ((client (romacs-agent-client-connect socket)) (log (romacs-agent-service-tests--log client)))
        (should (wait (romacs-agent-client-revision client)))
        (romacs-agent-client-open client "busy")
        (should (wait (car log)))
        (setf (romacs-agent-state busy) 'idle)
        ;; As the editor tells a change made beside its events.
        (romacs-agent--changed busy)
        (should (wait (cl-find 'changed (car log) :key #'cadr)))
        (should (equal (gethash "state" (gethash "row" (nth 2 (cl-find 'changed (car log) :key #'cadr)))) "idle"))))))

(ert-deftest romacs-agent-service-binds-a-relay-s-caller ()
  ;; On the relay's socket the first message says who called; hello then
  ;; answers that identity.  Anything else first closes the connection.
  (romacs-agent-service-tests--with
    (romacs-agent-service-start socket)
    (let* ((relay (expand-file-name "service-relay.sock" (file-name-directory socket)))
           (named (romacs-agent-service-tests--raw relay))
           (unnamed (romacs-agent-service-tests--raw relay)))
      (process-send-string named "{\"jsonrpc\":\"2.0\",\"method\":\"agents/transport\",\"params\":{\"via\":\"tailnet\",\"login\":\"me@example\",\"peer\":\"Me\"}}\n")
      (process-send-string named "{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"agents/hello\",\"params\":{\"protocol\":\"romacs-agents\",\"version\":[1,0]}}\n")
      (should (wait (process-get named 'lines)))
      (should (equal (json-serialize (gethash "identity" (gethash "result" (car (process-get named 'lines)))))
                     "{\"via\":\"tailnet\",\"login\":\"me@example\",\"peer\":\"Me\"}"))
      (process-send-string unnamed "{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"agents/hello\",\"params\":{\"protocol\":\"romacs-agents\",\"version\":[1,0]}}\n")
      (should (wait (not (process-live-p unnamed))))
      (should-not (process-get unnamed 'lines))
      (delete-process named))))

(ert-deftest romacs-agent-service-binds-an-ssh-caller ()
  ;; `romacs-relay stdio' says the caller came over ssh; a via the service
  ;; does not know closes the connection.
  (romacs-agent-service-tests--with
    (mk "listed" 'idle)
    (romacs-agent-service-start socket)
    (let* ((relay (expand-file-name "service-relay.sock" (file-name-directory socket)))
           (ssh (romacs-agent-service-tests--raw relay))
           (other (romacs-agent-service-tests--raw relay)))
      (process-send-string ssh "{\"jsonrpc\":\"2.0\",\"method\":\"agents/transport\",\"params\":{\"via\":\"ssh\",\"login\":\"me\",\"peer\":\"192.0.2.7 51234\"}}\n")
      (process-send-string ssh "{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"agents/hello\",\"params\":{\"protocol\":\"romacs-agents\",\"version\":[1,0]}}\n")
      (should (wait (process-get ssh 'lines)))
      (should (equal (json-serialize (gethash "identity" (gethash "result" (car (process-get ssh 'lines)))))
                     "{\"via\":\"ssh\",\"login\":\"me\",\"peer\":\"192.0.2.7 51234\"}"))
      (process-send-string other "{\"jsonrpc\":\"2.0\",\"method\":\"agents/transport\",\"params\":{\"via\":\"local\",\"login\":\"me\"}}\n")
      (should (wait (not (process-live-p other))))
      (delete-process ssh))))

;;; History

(defun romacs-agent-service-tests--line (seq key value)
  (concat (json-serialize (romacs-mcp--object key value "seq" seq "t" (+ 1794600000.25 seq))) "\n"))

(defun romacs-agent-service-tests--turn (seq n)
  "A Claude turn from SEQ: its prompt, its reply streamed in two deltas,
the whole reply, its result."
  (let ((id (format "m%d" n)) (text (format "answer %d" n)))
    (concat (romacs-agent-service-tests--line
             seq "in" (romacs-mcp--object "message" (romacs-mcp--object "content" (format "question %d" n) "role" "user")
                                          "romacsRequest" n "type" "user"))
            (romacs-agent-service-tests--line
             (+ seq 1) "out" (romacs-mcp--object "type" "stream_event" "parent_tool_use_id" :null "api_message_id" id
                                                 "event" (romacs-mcp--object "type" "message_start"
                                                                             "message" (romacs-mcp--object "id" id))))
            (romacs-agent-service-tests--line
             (+ seq 2) "out" (romacs-mcp--object "type" "stream_event" "parent_tool_use_id" :null "api_message_id" id
                                                 "event" (romacs-mcp--object "type" "content_block_delta" "index" 0
                                                                             "delta" (romacs-mcp--object "type" "text_delta" "text" "answer "))))
            (romacs-agent-service-tests--line
             (+ seq 3) "out" (romacs-mcp--object "type" "stream_event" "parent_tool_use_id" :null "api_message_id" id
                                                 "event" (romacs-mcp--object "type" "content_block_delta" "index" 0
                                                                             "delta" (romacs-mcp--object "type" "text_delta" "text" (number-to-string n)))))
            (romacs-agent-service-tests--line
             (+ seq 4) "out" (romacs-mcp--object "type" "assistant" "parent_tool_use_id" :null
                                                 "message" (romacs-mcp--object "id" id "content"
                                                                               (vector (romacs-mcp--object "type" "text" "text" text)))))
            (romacs-agent-service-tests--line (+ seq 5) "out" (romacs-mcp--object "type" "result" "result" text)))))

(defmacro romacs-agent-service-tests--with-stored (&rest body)
  "BODY with `session', a Claude session of three turns kept in SQLite, in
a state root of its own, and a service of its own on `socket'."
  (declare (indent 0))
  `(let* ((root (make-temp-file "rsv-store" t))
          (socket (expand-file-name "s.sock" root))
          (process-environment (append (list (concat "XDG_STATE_HOME=" root "/state")
                                             (concat "XDG_RUNTIME_DIR=" root "/run"))
                                       process-environment))
          (romacs-agent--sessions (make-hash-table :test #'equal))
          (romacs-agent--launched (make-hash-table :test #'equal))
          (romacs-agent--held-notes (make-hash-table :test #'equal))
          (romacs-agent-store--db nil) (romacs-agent-events--made nil)
          (romacs-agent-events--imported (make-hash-table :test #'equal))
          (romacs-agent-event-functions nil)
          (id "e7e70000-2222-4333-8444-555566667778")
          (dir (expand-file-name id (romacs-agent--state-root))))
     (unwind-protect
         (progn
           (make-directory dir t)
           (romacs-agent--write-json (expand-file-name "manifest.json" dir)
                                     (romacs-mcp--object "id" id "harness" "claude" "provider" "claude" "cwd" root
                                                         "title" "stored" "titleAuto" :false "argv" []
                                                         "options" (romacs-mcp--object)))
           (let ((coding-system-for-write 'utf-8-unix))
             (write-region (concat (romacs-agent-service-tests--line 1 "note" (romacs-mcp--object "type" "run"))
                                   (romacs-agent-service-tests--turn 2 1) (romacs-agent-service-tests--turn 8 2)
                                   (romacs-agent-service-tests--turn 14 3))
                           nil (expand-file-name "journal.jsonl" dir) nil 'silent))
           (romacs-agent--load dir)
           (let ((session (romacs-agent-get id)))
             (romacs-agent-events-import session)
             (require 'romacs-agent-turns)
             (romacs-agent-turns-fill session)
             (cl-macrolet ((wait (form) `(let ((deadline (+ (float-time) 5)))
                                            (while (and (not ,form) (< (float-time) deadline))
                                              (accept-process-output nil 0.02))
                                            ,form)))
               ,@body)))
       (romacs-agent-service-stop)
       (when (cdr romacs-agent-store--db) (ignore-errors (sqlite-close (cdr romacs-agent-store--db))))
       (delete-directory root t))))

(defun romacs-agent-service-tests--live (session)
  "SESSION's records as live delivery sends them: each unit (SEQ . JSON)."
  (let ((units nil) (unit nil))
    (dolist (event (romacs-agent-events session))
      (push event unit)
      (when (plist-get event :last)
        (let ((sent (romacs-agent-service--bounded (nreverse unit))))
          (unless (equal sent []) (push (cons (plist-get event :seq) (json-serialize sent)) units)))
        (setq unit nil)))
    (nreverse units)))

(defun romacs-agent-service-tests--history (client id params)
  "Session ID's history through CLIENT, each unit (SEQ . JSON), and its reset."
  (let ((answer nil))
    (romacs-agent-client-history client id params (lambda (result) (setq answer result)))
    (let ((deadline (+ (float-time) 5)))
      (while (and (not answer) (< (float-time) deadline)) (accept-process-output nil 0.02)))
    (cons (mapcar (lambda (unit) (cons (gethash "seq" unit) (json-serialize (gethash "events" unit)))) (car answer))
          (nth 2 answer))))

(ert-deftest romacs-agent-service-history-is-what-live-sent ()
  ;; A session's history is its records' units as live delivery sent
  ;; them; from a place mid-message, folded from its turn's start, its
  ;; text never comes twice; too far behind, a reset.
  (romacs-agent-service-tests--with-stored
    (romacs-agent-service-start socket)
    (let ((client (romacs-agent-client-connect socket)) (live (romacs-agent-service-tests--live session)))
      (should (wait (romacs-agent-client-revision client)))
      (should (> (length live) 9))
      (should (equal (car (romacs-agent-service-tests--history
                           client id (romacs-agent-rpc-object "upto" 19 "turns" 10)))
                     live))
      ;; The last turn alone.
      (should (equal (car (romacs-agent-service-tests--history client id (romacs-agent-rpc-object "upto" 19 "turns" 1)))
                     (seq-filter (lambda (unit) (>= (car unit) 14)) live)))
      ;; After seq 10, the first delta of turn 2: its second delta, then the
      ;; whole message raising no text, as live.
      (let ((after (car (romacs-agent-service-tests--history client id (romacs-agent-rpc-object "upto" 19 "after" 10)))))
        (should (equal after (seq-filter (lambda (unit) (> (car unit) 10)) live)))
        (should-not (string-search "answer 2" (cdr (assoc 12 after)))))
      (let ((romacs-agent-service--catch-up 5))
        (should (cdr (romacs-agent-service-tests--history client id (romacs-agent-rpc-object "upto" 19 "after" 2)))))
      ;; Where a chat begins is read from the turns' rows, an end given
      ;; or not: a running session's history read up to a seq before its
      ;; end starts as its chat does.
      (should (equal (romacs-agent-tail-start session 1 100) 14))
      (should (equal (romacs-agent-tail-start session 1 100 20) 14))
      (cl-letf (((symbol-function 'romacs-agent-events-tail) (lambda (&rest _) (error "Prompts scanned"))))
        (should (equal (romacs-agent-tail-start session 1 100 14) 8)))
      ;; A read from the end begins where a chat of its turns does
      ;; (`romacs-agent-tail-start'): a turn within RECORDS of the end from
      ;; its start (14, six records before 20), one longer from within it,
      ;; its first records left out; a catch-up folds from its turn's start
      ;; whatever RECORDS.
      (should (equal (romacs-agent-service-tests--history client id (romacs-agent-rpc-object "upto" 19 "turns" 1 "records" 6))
                     (cons (seq-filter (lambda (unit) (>= (car unit) 14)) live) nil)))
      (should (equal (romacs-agent-service-tests--history client id (romacs-agent-rpc-object "upto" 19 "turns" 3 "records" 6))
                     (cons (seq-filter (lambda (unit) (>= (car unit) 14)) live) nil)))
      (should (equal (romacs-agent-service-tests--history client id (romacs-agent-rpc-object "upto" 19 "turns" 1 "records" 3))
                     (cons (seq-filter (lambda (unit) (>= (car unit) 17)) live) nil)))
      (should (equal (car (romacs-agent-service-tests--history client id (romacs-agent-rpc-object "upto" 19 "after" 17)))
                     (seq-filter (lambda (unit) (> (car unit) 17)) live))))))

(ert-deftest romacs-agent-service-a-remote-chat-draws-as-the-local-one ()
  ;; A chat of the session drawn through the service (another machine's,
  ;; here this one's) is the chat this editor draws of it: its history,
  ;; then a unit that comes live.
  (require 'romacs-agent-remote)
  (romacs-agent-service-tests--with-stored
    (romacs-agent-service-start socket)
    (add-hook 'romacs-agent-event-functions #'romacs-agent-log--on-event)
    (let* ((romacs-agent-remote--source nil)
           (source (romacs-agent-remote-connect socket))
           (local (romacs-agent-log-chat session))
           (remote nil)
           (text (lambda (buffer) (with-current-buffer buffer (buffer-substring-no-properties (point-min) (point-max))))))
      (unwind-protect
          (progn
            (should (wait (romacs-agent-client-revision (romacs-agent-remote-client source))))
            (setq remote (romacs-agent-remote-chat id))
            (should-not (eq remote local))
            (should (wait (string-search "answer 3" (funcall text remote))))
            (should (equal (funcall text remote) (funcall text local)))
            (run-hook-with-args 'romacs-agent-event-functions session
                                (list :type 'status :text "live now" :seq 20 :last t :time (float-time)))
            (should (wait (string-search "live now" (funcall text remote))))
            (should (equal (funcall text remote) (funcall text local)))
            ;; Its board too: the rows, their keys the service's.
            (require 'romacs-agent-board)
            (cl-flet ((board (source)
                        (let ((romacs-agent-board-source source) (buffer (romacs-agent-board--buffer)))
                          (romacs-agent-board--render buffer 100)
                          (funcall text buffer))))
              (should (string-search "stored" (board source)))
              (should (equal (board source) (board romacs-agent-log-local)))
              ;; Running a tool: what it runs and how long, as drawn here.
              (setf (romacs-agent-state session) 'running (romacs-agent-activity session) "Grep"
                    (romacs-agent-turn-start session) (- (float-time) 75))
              ;; Raised, as the editor tells its views of an event.
              (romacs-agent--raise session (list :type 'status :text "working" :seq 21 :last t :time (float-time)))
              (should (wait (equal (gethash "activity" (gethash id (romacs-agent-client-rows (romacs-agent-remote-client source))))
                                   "Grep")))
              (should (string-search "Grep" (board source)))
              (should (equal (board source) (board romacs-agent-log-local)))
              ;; Shown, not acted on: no command of this editor's reaches it,
              ;; nor once the board's source is this editor's again, until
              ;; the board is drawn of its sessions.
              (let ((romacs-agent-board-source source))
                (with-current-buffer (romacs-agent-board--buffer)
                  (romacs-agent-board--render (current-buffer) 100)
                  (goto-char (point-min))
                  (romacs-agent-board-next 2)
                  (should-error (romacs-agent-board-settle) :type 'user-error)
                  (should-error (romacs-agent-board-settle-finished) :type 'user-error)))
              (with-current-buffer (romacs-agent-board--buffer)
                (should-error (romacs-agent-board-settle-finished) :type 'user-error))
              ;; The rows it serves are its own board's, whatever its board shows.
              (let ((romacs-agent-board-source source))
                (should (equal (gethash "key" (gethash id (romacs-agent-service--whole-rows)))
                               (let ((romacs-agent-board-source romacs-agent-log-local))
                                 (gethash "key" (gethash id (romacs-agent-service--whole-rows)))))))))
        (romacs-agent-remote-disconnect)
        (kill-buffer local)
        (when remote (kill-buffer remote))))))

(ert-deftest romacs-agent-service-a-remote-chat-reads-all-it-is-told ()
  ;; A remote chat is drawn whole when its history comes in replies cut
  ;; short; its earlier turns are read as the local chat's are; killed,
  ;; it follows the session no longer.
  (require 'romacs-agent-remote)
  (romacs-agent-service-tests--with-stored
    (romacs-agent-service-start socket)
    (let* ((romacs-agent-remote--source nil) (romacs-agent-log-turns 1)
           (source (romacs-agent-remote-connect socket))
           (client (romacs-agent-remote-client source))
           (local (romacs-agent-log-chat session))
           (remote nil)
           (text (lambda (buffer) (with-current-buffer buffer (buffer-substring-no-properties (point-min) (point-max))))))
      (unwind-protect
          (progn
            (should (wait (romacs-agent-client-revision client)))
            ;; Read a unit a reply, on in ranges the service reads whole.
            (let ((romacs-agent-service--history-bytes 1) (romacs-agent-service--catch-up 2)
                  (romacs-agent-client--catch-up 2))
              (setq remote (romacs-agent-remote-chat id))
              (should (wait (string-search "answer 3" (funcall text remote))))
              ;; Opened again while it reads: drawn once, afresh.
              (romacs-agent-client-open client id)
              (romacs-agent-client-open client id)
              (should (wait (let ((follow (gethash id (romacs-agent-client-follows client))))
                              (and follow (not (romacs-agent-client--follow-pulling follow)))))))
            (should (string-search "earlier turns" (funcall text remote)))
            (should (equal (funcall text remote) (funcall text local)))
            (with-current-buffer local (romacs-agent-log-earlier))
            (with-current-buffer remote
              (romacs-agent-log-earlier)
              ;; Asked again while they are read: once.
              (should-error (romacs-agent-log-earlier) :type 'user-error))
            (should (wait (string-search "answer 2" (funcall text remote))))
            (should (equal (funcall text remote) (funcall text local)))
            ;; The connection gone, the chat says so; connected again, it
            ;; is drawn afresh.
            (romacs-agent-client-close client)
            (should (wait (string-search "connection ended" (funcall text remote))))
            (setq client (romacs-agent-remote-client (romacs-agent-remote-connect socket)))
            (should (wait (not (string-search "connection ended" (funcall text remote)))))
            (should (wait (string-search "answer 3" (funcall text remote))))
            (with-current-buffer local (romacs-agent-log--clear) (romacs-agent-log-source-fill romacs-agent-log-local session))
            (should (equal (funcall text remote) (funcall text local)))
            (kill-buffer remote)
            (setq remote nil)
            (should-not (gethash id (romacs-agent-client-follows client)))
            (should (wait (null (romacs-agent-service--subscriptions id)))))
        (romacs-agent-remote-disconnect)
        (kill-buffer local)
        (when remote (kill-buffer remote))))))

(ert-deftest romacs-agent-client-speaks-over-a-command-s-pipes ()
  ;; A command's pipes carry the protocol as a socket does (`ssh HOST
  ;; romacs-relay stdio'): what it says on stderr stays out of the stream,
  ;; and its exit ends the connection.
  (let* ((stderr (generate-new-buffer " *rsv-stderr*")) (ended nil)
         (romacs-agent-client-ended-functions (list (lambda (client) (setq ended client))))
         (client (romacs-agent-client-connect
                  (list "sh" "-c" (concat "echo 'Warning: Permanently added' >&2; read hello; "
                                          "echo '{\"jsonrpc\":\"2.0\",\"id\":1,\"result\":"
                                          "{\"version\":[1,1],\"server\":{\"generation\":\"g\"}}}'; read watch"))
                  stderr)))
    (unwind-protect
        (let ((deadline (+ (float-time) 5)))
          (while (and (not ended) (< (float-time) deadline)) (accept-process-output nil 0.02))
          (should (eq ended client))
          (should (equal (romacs-agent-client-minor client) 1))
          (should (string-search "Warning" (with-current-buffer stderr (buffer-string)))))
      (romacs-agent-client-close client)
      (kill-buffer stderr))))

(ert-deftest romacs-agent-remote-runs-ssh-as-a-pipe ()
  ;; No pty, no prompt, a dead link found, a master connection reused; the
  ;; remote relay as the host names it, else the default.
  (require 'romacs-agent-remote)
  (let ((romacs-agent-remote-ssh "ssh") (romacs-agent-remote-relay-command "romacs-relay"))
    (should (equal (romacs-agent-remote--ssh-command '(:ssh "me@box" :relay "~/bin/romacs-relay") "/run/r/ssh")
                   (append '("ssh") romacs-agent-remote--ssh-options romacs-agent-remote--master-options
                           '("-o" "ControlPath=/run/r/ssh/%C" "me@box" "~/bin/romacs-relay" "stdio"))))
    (should (equal (last (romacs-agent-remote--ssh-command '(:ssh "box") "/run/r/ssh") 2) '("romacs-relay" "stdio")))
    ;; No place for a master's socket: none.
    (should (member "ControlPath=none" (romacs-agent-remote--ssh-command '(:ssh "box") nil)))
    (let ((romacs-agent-remote--socket-path-most 20)) (should-not (romacs-agent-remote--control-dir)))
    (should (member "BatchMode=yes" romacs-agent-remote--ssh-options))
    (should (member "-T" romacs-agent-remote--ssh-options))))

(ert-deftest romacs-agent-remote-a-machine-not-reached-is-tried-again ()
  ;; No ssh to run, or a service that never answers hello: each is a
  ;; connection that ended, tried again later, nothing left running.
  (require 'romacs-agent-remote)
  (let* ((romacs-agent-remote--source nil) (romacs-agent-remote--sources (make-hash-table :test #'equal))
         (romacs-agent-remote-hosts '(("far" :ssh "far"))) (romacs-agent-remote--answer-within 0.3)
         (inhibit-message t) (command '("/nonexistent/ssh"))
         (stderr-pipes (lambda () (seq-filter (lambda (p) (string-prefix-p "romacs-agent-client stderr" (process-name p)))
                                              (process-list))))
         (before (length (funcall stderr-pipes)))
         (source nil))
    (cl-letf (((symbol-function 'romacs-agent-remote--ssh-command) (lambda (_place _control) command)))
      (unwind-protect
          (progn
            (setq source (romacs-agent-remote-connect nil "far"))
            (should (timerp (romacs-agent-remote-retry source)))
            (should (= (length (funcall stderr-pipes)) before))
            ;; Reached, but no answer: closed once its time is up.
            (setq command '("sleep" "30"))
            (romacs-agent-remote--open source)
            (let ((client (romacs-agent-remote-client source)))
              (should (romacs-agent-client-live-p client))
              (let ((deadline (+ (float-time) 5)))
                (while (and (romacs-agent-client-live-p client) (< (float-time) deadline))
                  (accept-process-output nil 0.05)))
              (should-not (romacs-agent-client-live-p client))
              (should (timerp (romacs-agent-remote-retry source)))))
        (romacs-agent-remote-disconnect source)))))

(ert-deftest romacs-agent-remote-a-name-is-its-latest-machine-s ()
  ;; A name connected again at a new place is a new machine's; letting go
  ;; of an old source under it lets go of nothing newer.
  (require 'romacs-agent-remote)
  (let* ((romacs-agent-remote--sources (make-hash-table :test #'equal))
         (romacs-agent-remote--source nil)
         (old (romacs-agent-remote--host-source "n" "/a")))
    (should (eq (romacs-agent-remote--host-source "n" "/a") old))
    ;; Somewhere else: another machine's, the old one let go of.
    (should-not (eq (romacs-agent-remote--host-source "n" "/b") old))
    (should (equal (romacs-agent-remote-place (gethash "n" romacs-agent-remote--sources)) "/b"))
    ;; A place edited where it came from is another place too.
    (let* ((place (list :ssh "a")) (kept (romacs-agent-remote--host-source "p" place)))
      (setf (plist-get place :ssh) "b")
      (should-not (eq (romacs-agent-remote--host-source "p" place) kept)))
    (romacs-agent-remote-disconnect old)
    (let ((new (romacs-agent-remote--host-source "n" "/c")))
      (romacs-agent-remote-disconnect old)
      (should (eq (gethash "n" romacs-agent-remote--sources) new)))))

(ert-deftest romacs-agent-service-several-machines-keep-their-own-sessions ()
  ;; Two machines connected at once, each its own client, mirrors and
  ;; chats: a session id they share is two sessions here.
  (require 'romacs-agent-remote)
  (romacs-agent-service-tests--with
    (mk "same" 'running)
    (romacs-agent-service-start socket)
    (let ((romacs-agent-remote--source nil) (romacs-agent-remote--sources (make-hash-table :test #'equal))
          (a nil) (b nil))
      (unwind-protect
          (progn
            (setq a (romacs-agent-remote-connect socket "a") b (romacs-agent-remote-connect socket "b"))
            (should (eq romacs-agent-remote--source b))
            (should (wait (and (romacs-agent-client-revision (romacs-agent-remote-client a))
                               (romacs-agent-client-revision (romacs-agent-remote-client b)))))
            (should (eq (romacs-agent-remote--of (romacs-agent-remote-client a)) a))
            (should (eq (romacs-agent-remote--of (romacs-agent-remote-client b)) b))
            (let ((mirror-a (romacs-agent-board-source-live a "same"))
                  (mirror-b (romacs-agent-board-source-live b "same")))
              (should (and mirror-a mirror-b (not (eq mirror-a mirror-b)))))
            (should-not (eq (romacs-agent-log-source-chats a) (romacs-agent-log-source-chats b))))
        (when a (romacs-agent-remote-disconnect a))
        (when b (romacs-agent-remote-disconnect b))))))

(ert-deftest romacs-agent-service-a-remote-machine-is-connected-again ()
  ;; Its connection ended, a machine kept is connected again once its
  ;; service answers; let go, it is not.
  (require 'romacs-agent-remote)
  (romacs-agent-service-tests--with
    (mk "busy" 'running)
    (romacs-agent-service-start socket)
    (let* ((romacs-agent-remote--source nil) (romacs-agent-remote--sources (make-hash-table :test #'equal))
           (inhibit-message t)
           (source (romacs-agent-remote-connect socket))
           (first (romacs-agent-remote-client source)))
      (unwind-protect
          (progn
            (should (wait (romacs-agent-client-revision first)))
            (romacs-agent-service-stop)
            (should (wait (not (romacs-agent-client-live-p first))))
            (should (timerp (romacs-agent-remote-retry source)))
            (romacs-agent-service-start socket)
            (let ((deadline (+ (float-time) 8)))
              (while (and (not (let ((client (romacs-agent-remote-client source)))
                                 (and client (not (eq client first)) (romacs-agent-client-revision client))))
                          (< (float-time) deadline))
                (accept-process-output nil 0.05)))
            (should (gethash "busy" (romacs-agent-client-rows (romacs-agent-remote-client source))))
            (romacs-agent-remote-disconnect source)
            (should-not (gethash socket romacs-agent-remote--sources))
            (should-not (memq (romacs-agent-remote-retry source) timer-list)))
        (romacs-agent-remote-disconnect source)))))

(ert-deftest romacs-agent-service-a-remote-chat-shows-what-waits ()
  ;; A question the session waits on when its remote chat opens is drawn
  ;; there as here; one that comes live is added, then resolved with how
  ;; and by whom, the chat settling it.
  (require 'romacs-agent-remote)
  (romacs-agent-service-tests--with-stored
    (romacs-agent-service-start socket)
    (let* ((romacs-agent-remote--source nil) (romacs-agent-approval-policy 'ask)
           (question (lambda (id text)
                       (let ((request (make-hash-table :test #'equal)))
                         (puthash "tool_name" "AskUserQuestion" request)
                         (puthash "input" (json-parse-string (format "{\"questions\":[{\"question\":%S,\"options\":[{\"label\":\"Tea\"},{\"label\":\"Coffee\"}]}]}" text)) request)
                         (puthash id request (romacs-agent-approvals session)))))
           (text (lambda (buffer) (with-current-buffer buffer (buffer-substring-no-properties (point-min) (point-max)))))
           (source nil) (client nil) (local nil) (remote nil))
      (funcall question "q-old" "Tea or coffee?")
      (funcall question "q-old2" "Sugar?")
      (setq source (romacs-agent-remote-connect socket) client (romacs-agent-remote-client source)
            local (romacs-agent-log-chat session))
      (unwind-protect
          (progn
            (should (wait (romacs-agent-client-revision client)))
            ;; What waits, a request a reply: paged; told with its history,
            ;; never before it.
            (let ((romacs-agent-service--unit-budget 1) (told nil))
              (add-hook 'romacs-agent-client-follow-functions
                        (lambda (c id notice)
                          (unless (eq (car notice) 'row)
                            ;; With each notice, how far its history is read.
                            (push (cons (car notice) (romacs-agent-client--follow-seq (gethash id (romacs-agent-client-follows c))))
                                  told))))
              (setq remote (romacs-agent-remote-chat id))
              ;; Drawn whole: its history, then what it waits on.
              (should (wait (and (string-search "answer 3" (funcall text remote))
                                 (string-search "Tea or coffee?" (funcall text remote))
                                 (string-search "Sugar?" (funcall text remote)))))
              (should (equal (mapcar #'car (reverse told)) '(pending drawn)))
              (should (> (cdr (car (last told))) 0)))
            (should (equal (funcall text remote) (funcall text local)))
            (let ((follow (gethash id (romacs-agent-client-follows client))))
              (should (= (hash-table-count (romacs-agent-client--follow-pending follow)) 2))
              ;; One asked live: its approval event, and it waits.
              (funcall question "q-new" "Milk?")
              (run-hook-with-args 'romacs-agent-event-functions session
                                  (list :type 'approval :request-id "q-new" :tool "AskUserQuestion" :user t
                                        :input (romacs-agent-rpc-object)
                                        :questions '(("Milk?" :header nil :options ("Tea" "Coffee") :notes nil :multi nil))
                                        :seq 20 :last t :time (float-time)))
              (should (wait (= (hash-table-count (romacs-agent-client--follow-pending follow)) 3)))
              (should (string-search "Milk?" (funcall text remote)))
              ;; Answered: it leaves, answered, by the user.
              (let ((resolved nil))
                (add-hook 'romacs-agent-client-session-functions
                          (lambda (_c _id notice) (when (eq (car notice) 'pending-resolved) (setq resolved (cadr notice)))))
                ;; Who answered is said a unit before the answer: kept for it.
                (run-hook-with-args 'romacs-agent-event-functions session
                                    (list :type 'attributed :request-id "q-new" :decision "deny" :reason "no milk"
                                          :actor '(:kind "user" :login "ada" :client "x220")
                                          :seq 21 :last t :time (float-time)))
                (remhash "q-new" (romacs-agent-approvals session))
                (run-hook-with-args 'romacs-agent-event-functions session
                                    (list :type 'answered :request-id "q-new" :behavior "deny" :seq 22 :last t :time (float-time)))
                (should (wait resolved))
                (should (equal (gethash "how" resolved) "answered"))
                (should (equal (gethash "kind" (gethash "actor" resolved)) "user"))
                (should (equal (gethash "login" (gethash "actor" resolved)) "ada"))
                (should (equal (gethash "client" (gethash "actor" resolved)) "x220"))
                (should (equal (gethash "reason" resolved) "no milk"))
                (should (= (hash-table-count (romacs-agent-client--follow-pending follow)) 2)))))
        (romacs-agent-remote-disconnect)
        (kill-buffer local)
        (when remote (kill-buffer remote))))))

(ert-deftest romacs-agent-service-remote-send-keeps-a-refused-draft-and-its-id ()
  ;; Refused or unconfirmed, a remote draft stays in the composer under its
  ;; draft id: sent again, it names that id, which the service sends once.
  (require 'romacs-agent-remote)
  (let ((source (romacs-agent-remote--make
                 :client (romacs-agent-client--make :server (romacs-agent-rpc-object "version" [1 1]))))
        (session (romacs-agent--make :id "remote-test" :harness 'claude :state 'idle))
        (intent (list :draft-id "d1" :kind 'send :text "t" :attachments nil))
        (sent nil) (reply nil))
    ;; One that may have gone is unconfirmed; the others, refusals.
    (pcase-dolist (`(,kind . ,reason) '(("disconnected" . unconfirmed) ("not-kept" . unconfirmed)
                                        ("unknown-session" . no-session) ("not-live" . closed)
                                        ("bad-params" . invalid)))
      (let ((answer (romacs-agent-remote--submit-answer
                     intent nil (romacs-agent-rpc-object "message" "Test error"
                                                         "data" (romacs-agent-rpc-object "kind" kind)))))
        (should (eq (if (eq (plist-get answer :status) 'unconfirmed) 'unconfirmed (plist-get answer :reason))
                    reason))))
    (cl-letf (((symbol-function 'romacs-agent-client-live-p) (lambda (_) t))
              ((symbol-function 'romacs-agent-client-command)
               (lambda (_client _method params callback) (push params sent) (setq reply callback))))
      (with-temp-buffer
        (romacs-agent-log-mode)
        (setq-local romacs-agent-log--session session
                    romacs-agent-log--source source
                    romacs-agent-log--end (point-min-marker)
                    romacs-agent-log--input (point-min-marker)
                    romacs-agent-log--prompts nil)
        (set-marker-insertion-type romacs-agent-log--end t)
        (romacs-agent-log--separator t)
        (goto-char (point-max))
        (insert "sent prompt")
        (cl-flet ((answer (result error)
                    (funcall reply result error)
                    (let ((deadline (+ (float-time) 3)))
                      (while (and romacs-agent-log--sending (< (float-time) deadline))
                        (accept-process-output nil 0.01)))
                    (should-not romacs-agent-log--sending)))
          (romacs-agent-log-send)
          (answer nil (romacs-agent-rpc-object "message" "Test error"
                                               "data" (romacs-agent-rpc-object "kind" "disconnected")))
          (should (equal (romacs-agent-log--draft) "sent prompt"))
          (romacs-agent-log-send)
          (should (= (length sent) 2))
          (should (stringp (gethash "draftId" (car sent))))
          (should (equal (gethash "draftId" (car sent)) (gethash "draftId" (cadr sent))))
          (answer (romacs-agent-rpc-object "state" "written") nil)
          (should (equal (romacs-agent-log--draft) "")))))))

(ert-deftest romacs-agent-service-command-digest-orders-nested-objects ()
  (let ((a (json-parse-string "{\"cid\":\"a\",\"rows\":[{\"b\":2,\"a\":1},null]}"))
        (b (json-parse-string "{\"rows\":[{\"a\":1,\"b\":2},null],\"cid\":\"b\"}"))
        (c (json-parse-string "{\"rows\":[null,{\"a\":1,\"b\":2}]}")))
    (let ((digest-a (romacs-agent-service--command-digests "test" a))
          (digest-b (romacs-agent-service--command-digests "test" b))
          (digest-c (romacs-agent-service--command-digests "test" c)))
      (should (equal (car digest-a) (car digest-b)))
      (should-not (equal (car digest-a) (car digest-c)))
      (should (equal (cadr digest-a) (secure-hash 'sha1 "test {\"rows\":[{\"b\":2,\"a\":1},null]}"))))))

(ert-deftest romacs-agent-service-runs-a-command-once ()
  ;; A command runs once by its cid: again, its kept result; the cid with
  ;; other params, refused.  An answer reserves its request: a second
  ;; finds it gone.  A remote chat's keys are commands.
  (require 'romacs-agent-remote)
  (romacs-agent-service-tests--with-stored
    (romacs-agent-service-start socket)
    (let* ((romacs-agent-remote--source nil) (romacs-agent-approval-policy 'ask) (calls nil) (notes nil)
           (romacs-agent-service--commands (make-hash-table :test #'equal))
           (romacs-agent-service--answering (make-hash-table :test #'equal))
           (source (romacs-agent-remote-connect socket)) (client (romacs-agent-remote-client source))
           (remote nil)
           (ask (lambda (method params)
                  (let ((answer nil))
                    (romacs-agent-client-request client method params (lambda (r e) (setq answer (list r e))))
                    (wait answer)
                    answer))))
      (cl-letf (((symbol-function 'romacs-agent--ended-p) #'ignore)
                ((symbol-function 'romacs-agent--admit)
                 (lambda (_s item &rest _)
                   (push (list (if (eq (romacs-agent-item-kind item) 'next) 'later 'send) (romacs-agent-item-text item)) calls)
                   ;; As its write would say.
                   (romacs-agent--draft-written (romacs-agent-item-id item))
                   item))
                ((symbol-function 'romacs-agent-interrupt) (lambda (_s) (push '(interrupt) calls)))
                ;; Answered as the harness is written to: its line.
                ((symbol-function 'romacs-agent--line)
                 (lambda (_s line &rest _) (push (list 'line (if (stringp line) line (json-serialize line))) calls)))
                ((symbol-function 'romacs-agent--note) (lambda (_s note) (push note notes))))
        (unwind-protect
            (progn
              (should (wait (romacs-agent-client-revision client)))
              (let ((send (romacs-agent-rpc-object "cid" "c1" "id" id "text" "hi" "mode" "now"))
                    (repeat (romacs-agent-rpc-object "mode" "now" "text" "hi" "id" id "cid" "c1")))
                (should (equal (gethash "state" (car (funcall ask "agents/send" send))) "written"))
                (should (equal (gethash "state" (car (funcall ask "agents/send" repeat))) "written"))
                (should (equal calls '((send "hi"))))
                ;; Kept in the store too: a service started again runs it no second time.
                (clrhash romacs-agent-service--commands)
                (should (equal (gethash "state" (car (funcall ask "agents/send" repeat))) "written"))
                (should (equal calls '((send "hi"))))
                (puthash "text" "other" send)
                (should (equal (gethash "kind" (gethash "data" (cadr (funcall ask "agents/send" send)))) "cid-conflict"))
                ;; Another method by the same cid is another command: refused.
                (should (equal (gethash "kind" (gethash "data" (cadr (funcall ask "agents/interrupt"
                                                                              (romacs-agent-rpc-object "cid" "c1" "id" id)))))
                               "cid-conflict")))
              ;; A question waits; answered by its handle, then again.
              (let ((request (make-hash-table :test #'equal)))
                (puthash "tool_name" "AskUserQuestion" request)
                (puthash "input" (json-parse-string "{\"questions\":[{\"question\":\"Tea or coffee?\",\"options\":[{\"label\":\"Tea\"},{\"label\":\"Coffee\"}]}]}") request)
                (puthash "q-1" request (romacs-agent-approvals session)))
              (let* ((handle (romacs-agent-wire-handle (plist-get (romacs-agent-driver-state session) :run) "q-1"))
                     (stale (romacs-agent-wire-handle 999 "q-1"))
                     (answer (lambda (cid)
                               (funcall ask "agents/answer"
                                        (romacs-agent-rpc-object "cid" cid "id" id "handle" handle "decision" "allow"
                                                                 "answers" (romacs-agent-rpc-object
                                                                            "q1" (romacs-agent-rpc-object "options" ["o2"] "text" :null)))))))
                ;; Its handle whole: another run's is no request.
                (should (equal (gethash "kind" (gethash "data" (cadr (funcall ask "agents/answer"
                                                                              (romacs-agent-rpc-object "cid" "c9" "id" id "handle" stale
                                                                                                       "decision" "allow")))))
                               "request-gone"))
                (should (equal (gethash "state" (car (funcall answer "c2"))) "written"))
                ;; Credited to its caller: its login and its client's instance.
                (should (equal (gethash "login" (car notes)) (user-login-name)))
                (should (equal (gethash "client" (car notes)) (romacs-agent-client--instance)))
                (should (eq (car (car calls)) 'line))
                (should (string-search "Coffee" (cadr (car calls))))
                (should (equal (gethash "kind" (gethash "data" (cadr (funcall answer "c3")))) "request-gone")))
              ;; The remote chat's keys: a send, and an answer through its layer.
              (setq remote (romacs-agent-remote-chat id))
              (should (wait (string-search "answer 3" (with-current-buffer remote (buffer-string)))))
              (with-current-buffer remote
                (goto-char (point-max))
                (insert "from afar")
                (romacs-agent-log-send))
              (should (wait (equal (car calls) '(send "from afar"))))
              (should (wait (equal (with-current-buffer remote (romacs-agent-log--draft)) "")))
              ;; Refused: the text stays the composer's.
              (cl-letf (((symbol-function 'romacs-agent--admit) (lambda (&rest _) (user-error "Its harness ended"))))
                (with-current-buffer remote
                  (goto-char (point-max))
                  (insert "not sent")
                  (romacs-agent-log-send)
                  (should (equal (romacs-agent-log--draft) "not sent"))
                  (should (wait (not romacs-agent-log--sending)))
                  (should (equal (romacs-agent-log--draft) "not sent")))))
          (romacs-agent-remote-disconnect)
          (when remote (kill-buffer remote)))))))

(ert-deftest romacs-agent-service-starts-a-session-once ()
  ;; agents/start starts a session, its host tracked, once by its cid: a
  ;; repeat names the same session.  The lists say where and under what.
  (romacs-agent-service-tests--with-stored
    (romacs-agent-service-start socket)
    (let* ((home (make-temp-file "rsv-claude" t)) (cwd (make-temp-file "rsv-project" t))
           (romacs-agent-providers (list (cons "claude" home))) (romacs-agent-default-provider "claude")
           (romacs-agent-start-deadline 0.5)
           (romacs-agent-service--commands (make-hash-table :test #'equal))
           (client (romacs-agent-client-connect socket)) (launched nil)
           (ask (lambda (method params)
                  (let ((answer nil))
                    (romacs-agent-client-request client method params (lambda (r e) (setq answer (list r e))))
                    (wait answer)
                    answer))))
      (cl-letf (((symbol-function 'romacs-agent--launch-when-placed)
                 (lambda (session _launch) (push (romacs-agent-options session) launched)))
                ((symbol-function 'romacs-agent--check-companions) #'ignore))
        (should (wait (romacs-agent-client-revision client)))
        (let* ((params (romacs-agent-rpc-object "cid" "s1" "cwd" cwd "title" "far start"))
               (first (car (funcall ask "agents/start" params)))
               (again (car (funcall ask "agents/start" params))))
          (should (stringp (gethash "id" first)))
          ;; Its launch still waited for at its deadline: queued.
          (should (equal (gethash "state" first) "queued"))
          (should (equal (gethash "id" again) (gethash "id" first)))
          ;; Kept in the store: a service started again names the same one.
          (clrhash romacs-agent-service--commands)
          (should (equal (gethash "id" (car (funcall ask "agents/start" params))) (gethash "id" first)))
          ;; Found by the writer as it is admitted, the reader seeing none:
          ;; a repeat all the same.
          (clrhash romacs-agent-service--commands)
          (cl-letf (((symbol-function 'romacs-agent-store-command) #'ignore))
            (should (equal (gethash "id" (car (funcall ask "agents/start" params))) (gethash "id" first))))
          (should (= (length launched) 1))
          (should (plist-get (car launched) :tracked))
          (should (equal (romacs-agent-title (romacs-agent-loaded (gethash "id" first))) "far start")))
        (should (equal (gethash "kind" (gethash "data" (cadr (funcall ask "agents/start" (romacs-agent-rpc-object "cid" "s2" "cwd" "relative")))))
                       "bad-params"))
        ;; One that fails after the deadline: queued, then its repeats say so.
        (cl-letf (((symbol-function 'romacs-agent--launch-when-placed)
                   (lambda (session _launch)
                     (run-at-time 1 nil (lambda () (setf (romacs-agent-state session) 'unstarted))))))
          (let ((params (romacs-agent-rpc-object "cid" "s4" "cwd" cwd)))
            (should (equal (gethash "state" (car (funcall ask "agents/start" params))) "queued"))
            (should (wait (let ((error (cadr (funcall ask "agents/start" params))))
                            (and error (equal (gethash "kind" (gethash "data" error)) "not-live")))))))
        ;; A host that does not start: the start says so, and so does its repeat.
        (cl-letf (((symbol-function 'romacs-agent--launch-when-placed)
                   (lambda (session _launch) (setf (romacs-agent-state session) 'unstarted))))
          (let ((params (romacs-agent-rpc-object "cid" "s3" "cwd" cwd)))
            (should (equal (gethash "kind" (gethash "data" (cadr (funcall ask "agents/start" params)))) "not-live"))
            (should (equal (gethash "kind" (gethash "data" (cadr (funcall ask "agents/start" params)))) "not-live"))))
        (should (seq-some (lambda (p) (equal (gethash "cwd" p) root))
                          (gethash "projects" (car (funcall ask "agents/projects.list" (romacs-agent-rpc-object))))))
        (cl-letf (((symbol-function 'romacs-agent--want-models) #'ignore))
          (should (equal (mapcar (lambda (p) (gethash "name" p))
                                 (gethash "providers" (car (funcall ask "agents/providers.list" (romacs-agent-rpc-object)))))
                         '("claude"))))))))

(ert-deftest romacs-agent-service-history-stops-at-its-bound ()
  ;; Past its bytes' bound a reply stops, its cursor where; the client asks
  ;; on from there.  A reply under another epoch than the history held is
  ;; a reset for it.
  (romacs-agent-service-tests--with-stored
    (romacs-agent-service-start socket)
    (let ((client (romacs-agent-client-connect socket)) (live (romacs-agent-service-tests--live session)))
      (should (wait (romacs-agent-client-revision client)))
      (let ((romacs-agent-service--history-bytes 1) (answer nil))
        (romacs-agent-client-history client id (romacs-agent-rpc-object "upto" 19 "turns" 10) (lambda (r) (setq answer r)))
        (should (wait answer))
        (should (= (length (car answer)) 1))
        (should (equal (gethash "seq" (nth 1 answer)) (car (car live))))
        (should (equal (gethash "seq" (car (car answer))) (car (car live)))))
      (let ((answer nil))
        (romacs-agent-client-history client id (romacs-agent-rpc-object "upto" 19 "turns" 10) (lambda (r) (setq answer r))
                                     (romacs-agent-rpc-object "epoch" 99 "adapter" "other"))
        (should (wait answer))
        (should (nth 2 answer))
        (should-not (car answer))))))

(ert-deftest romacs-agent-replay-copies-touch-nothing-live ()
  ;; A replay's copy starts with no turn under way, whatever the session
  ;; does; folding a note there never lets go of the live session's held
  ;; note of the same id.
  (let* ((romacs-agent--held-notes (make-hash-table :test #'equal))
         (session (romacs-agent--make :id "live" :harness 'claude :input-seq 40 :completed-seq 30))
         (copy (romacs-agent-replay-copy session nil))
         (held (romacs-mcp--object "romacsNote" 1 "type" "selection")))
    (should (equal (list (romacs-agent-input-seq copy) (romacs-agent-completed-seq copy)) '(0 0)))
    (puthash "live" (list held) romacs-agent--held-notes)
    (romacs-agent-replay-record copy (json-parse-string "{\"note\":{\"romacsNote\":1,\"type\":\"selection\"},\"seq\":3,\"t\":1.0}"))
    (should (equal (gethash "live" romacs-agent--held-notes) (list held)))))

(ert-deftest romacs-agent-held-notes-of-two-runs-never-collide ()
  ;; A journal an older run wrote has numeric note ids, this run's are
  ;; UUIDs: replaying both lets go of each held note once, on its own echo.
  (let* ((romacs-agent--held-notes (make-hash-table :test #'equal))
         (session (romacs-agent--make :id "mixed" :harness 'claude :state 'idle))
         (record (lambda (id) (json-parse-string (format "{\"note\":{\"romacsNote\":%s,\"type\":\"selection\"},\"seq\":3}"
                                                         (if (numberp id) id (format "\"%s\"" id)))))))
    (cl-letf (((symbol-function 'romacs-agent--check-note) #'ignore)
              ((symbol-function 'romacs-agent--ensure-host) #'ignore)
              ((symbol-function 'romacs-agent--notes-written) #'ignore))
      (romacs-agent--note session (romacs-mcp--object "type" "selection"))
      (romacs-agent--note session (romacs-mcp--object "type" "handoff")))
    (let* ((held (gethash "mixed" romacs-agent--held-notes))
           (ids (mapcar (lambda (h) (gethash "romacsNote" h)) held)))
      (should (= (length ids) 2))
      (should (cl-every #'romacs-agent--uuid-p ids))
      (should-not (equal (car ids) (cadr ids)))
      ;; The older run's notes, numbered from 1: none is this run's echo.
      (dolist (old '(1 2 3)) (romacs-agent--held-echoed session (funcall record old)))
      (should (equal (gethash "mixed" romacs-agent--held-notes) held))
      ;; Each echo lets go of its own note, once; one seen again changes nothing.
      (romacs-agent--held-echoed session (funcall record (car ids)))
      (should (equal (gethash "mixed" romacs-agent--held-notes) (cdr held)))
      (romacs-agent--held-echoed session (funcall record (car ids)))
      (should (equal (gethash "mixed" romacs-agent--held-notes) (cdr held)))
      (romacs-agent--held-echoed session (funcall record (cadr ids)))
      (should-not (gethash "mixed" romacs-agent--held-notes)))))

;;; romacs-agent-service-tests.el ends here

(ert-deftest romacs-agent-service-keeps-unchanged-pending-projections ()
  (romacs-agent-service-tests--with
    (let* ((session (mk "followed" 'waiting-approval))
           (approvals (romacs-agent-approvals session))
           (subscription (romacs-agent-service--subscription-make :cutoff 0))
           (project (symbol-function 'romacs-agent-service--bounded-request))
           (projections 0) notices)
      (cl-flet ((ask (path)
                  (puthash "r" (romacs-agent-rpc-object "method" "can_use_tool"
                                                       "tool_name" "Read"
                                                       "input" (romacs-agent-rpc-object "path" path))
                           approvals)))
        (ask "/before")
        (setf (romacs-agent-service--subscription-pending subscription)
              (romacs-agent-service--requests session))
        (cl-letf (((symbol-function 'romacs-agent-service--subscriptions)
                   (lambda (_) (list (cons nil subscription))))
                  ((symbol-function 'romacs-agent-service--bounded-request)
                   (lambda (request) (cl-incf projections) (funcall project request)))
                  ((symbol-function 'romacs-agent-service--notify)
                   (lambda (_rpc _sub _id method params)
                     (when (equal method "agents/pending.added") (push (gethash "request" params) notices)))))
          (dotimes (index 3)
            (romacs-agent-service--unit-event "followed" session
                                              (list :type 'delta :text "output" :seq (1+ index) :last t)))
          (should (= projections 0))
          (should-not notices)
          (ask "/after")
          (romacs-agent-service--unit-event "followed" session
                                            '(:type approval :request-id "r" :tool "Read"
                                              :description "updated" :seq 4 :last t))
          (should (= projections (length notices) 1))
          (should (equal (gethash "reason" (gethash "tool" (car notices))) "updated")))))))
