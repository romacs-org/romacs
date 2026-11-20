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
     (cl-letf (((symbol-function 'romacs-agent-store-unloaded) #'ignore))
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
                         (symbol-name (romacs-agent-board--key (romacs-agent-get id))))))
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
          (run-hook-with-args 'romacs-agent-event-functions busy '(:type changed))
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
    (let ((stored (list (cons "p" (romacs-agent--make :id "p" :harness 'claude :state 'idle :cwd "/p/"))
                        (cons "c" (romacs-agent--make :id "c" :harness 'claude :state 'idle :cwd "/p/"
                                                      :done-seq 4 :options '(:parent "p"))))))
      (cl-letf (((symbol-function 'romacs-agent-store-row-session) (lambda (id) (cdr (assoc id stored)))))
        (let ((updates (romacs-agent-service--rows-of romacs-agent-service--list '("c" "p"))))
          (should (equal (gethash "key" (cdr (assoc "c" updates))) "completed")))))))

(ert-deftest romacs-agent-service-leaves-the-board-its-own-view ()
  ;; The service's whole pass makes its own view of the unloaded sessions:
  ;; the board's, whose shown rows hold its sessions, stays as it was.
  ;; Its keys read its own: an unloaded parent's finished child reads
  ;; completed, as on the board.
  (romacs-agent-service-tests--with
    (let ((romacs-agent-board--unloaded 'the-boards)
          (stored (list (romacs-agent--make :id "p" :harness 'claude :state 'idle :cwd "/p/")
                        (romacs-agent--make :id "c" :harness 'claude :state 'idle :cwd "/p/"
                                            :done-seq 4 :options '(:parent "p")))))
      (cl-letf (((symbol-function 'romacs-agent-store-unloaded) (lambda () stored)))
        (romacs-agent-service-start socket)
        (romacs-agent-service--refresh t))
      (should (eq romacs-agent-board--unloaded 'the-boards))
      (should (equal (gethash "key" (gethash "c" (romacs-agent-service--list-rows romacs-agent-service--list)))
                     "completed")))))

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
        (cl-letf (((symbol-function 'romacs-agent-store-unloaded) #'ignore))
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
        (run-hook-with-args 'romacs-agent-event-functions busy '(:type changed))
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
              (run-hook-with-args 'romacs-agent-event-functions session
                                  (list :type 'status :text "working" :seq 21 :last t :time (float-time)))
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
                  (should-error (romacs-agent-board-settle-finished) :type 'user-error)
                  (should-error (romacs-agent-board-new) :type 'user-error)))
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
      (setq source (romacs-agent-remote-connect socket) client (romacs-agent-remote-client source)
            local (romacs-agent-log-chat session))
      (unwind-protect
          (progn
            (should (wait (romacs-agent-client-revision client)))
            (setq remote (romacs-agent-remote-chat id))
            (should (wait (string-search "Tea or coffee?" (funcall text remote))))
            (should (equal (funcall text remote) (funcall text local)))
            (let ((follow (gethash id (romacs-agent-client-follows client))))
              (should (= (hash-table-count (romacs-agent-client--follow-pending follow)) 1))
              ;; One asked live: its approval event, and it waits.
              (funcall question "q-new" "Milk?")
              (run-hook-with-args 'romacs-agent-event-functions session
                                  (list :type 'approval :request-id "q-new" :tool "AskUserQuestion" :user t
                                        :input (romacs-agent-rpc-object)
                                        :questions '(("Milk?" :header nil :options ("Tea" "Coffee") :notes nil :multi nil))
                                        :seq 20 :last t :time (float-time)))
              (should (wait (= (hash-table-count (romacs-agent-client--follow-pending follow)) 2)))
              (should (string-search "Milk?" (funcall text remote)))
              ;; Answered: it leaves, answered, by the user.
              (let ((resolved nil))
                (add-hook 'romacs-agent-client-session-functions
                          (lambda (_c _id notice) (when (eq (car notice) 'pending-resolved) (setq resolved (cadr notice)))))
                (remhash "q-new" (romacs-agent-approvals session))
                (run-hook-with-args 'romacs-agent-event-functions session
                                    (list :type 'attributed :request-id "q-new" :by "user" :decision "allow" :seq 21 :last nil :time (float-time)))
                (run-hook-with-args 'romacs-agent-event-functions session
                                    (list :type 'answered :request-id "q-new" :behavior "allow" :seq 21 :last t :time (float-time)))
                (should (wait resolved))
                (should (equal (gethash "how" resolved) "answered"))
                (should (equal (gethash "kind" (gethash "actor" resolved)) "user"))
                (should (= (hash-table-count (romacs-agent-client--follow-pending follow)) 1)))))
        (romacs-agent-remote-disconnect)
        (kill-buffer local)
        (when remote (kill-buffer remote))))))

(ert-deftest romacs-agent-service-runs-a-command-once ()
  ;; A command runs once by its cid: again, its kept result; the cid with
  ;; other params, refused.  An answer reserves its request: a second
  ;; finds it gone.  A remote chat's keys are commands.
  (require 'romacs-agent-remote)
  (romacs-agent-service-tests--with-stored
    (romacs-agent-service-start socket)
    (let* ((romacs-agent-remote--source nil) (romacs-agent-approval-policy 'ask) (calls nil)
           (romacs-agent-service--commands (make-hash-table :test #'equal))
           (romacs-agent-service--answering (make-hash-table :test #'equal))
           (source (romacs-agent-remote-connect socket)) (client (romacs-agent-remote-client source))
           (remote nil)
           (ask (lambda (method params)
                  (let ((answer nil))
                    (romacs-agent-client-request client method params (lambda (r e) (setq answer (list r e))))
                    (wait answer)
                    answer))))
      (cl-letf (((symbol-function 'romacs-agent-send) (lambda (_s text &rest _) (push (list 'send text) calls) 'item))
                ((symbol-function 'romacs-agent-send-later) (lambda (_s text &rest _) (push (list 'later text) calls) 'item))
                ((symbol-function 'romacs-agent-interrupt) (lambda (_s) (push '(interrupt) calls)))
                ((symbol-function 'romacs-agent-respond)
                 (lambda (_s id allow reason _by) (push (list 'respond id allow reason) calls))))
        (unwind-protect
            (progn
              (should (wait (romacs-agent-client-revision client)))
              (let ((send (romacs-agent-rpc-object "cid" "c1" "id" id "text" "hi" "mode" "now")))
                (should (equal (gethash "state" (car (funcall ask "agents/send" send))) "written"))
                (should (equal (gethash "state" (car (funcall ask "agents/send" send))) "written"))
                (should (equal calls '((send "hi"))))
                ;; Kept in the store too: a service started again runs it no second time.
                (clrhash romacs-agent-service--commands)
                (should (equal (gethash "state" (car (funcall ask "agents/send" send))) "written"))
                (should (equal calls '((send "hi"))))
                (puthash "text" "other" send)
                (should (equal (gethash "kind" (gethash "data" (cadr (funcall ask "agents/send" send)))) "cid-conflict")))
              ;; A question waits; answered by its handle, then again.
              (let ((request (make-hash-table :test #'equal)))
                (puthash "tool_name" "AskUserQuestion" request)
                (puthash "input" (json-parse-string "{\"questions\":[{\"question\":\"Tea or coffee?\",\"options\":[{\"label\":\"Tea\"},{\"label\":\"Coffee\"}]}]}") request)
                (puthash "q-1" request (romacs-agent-approvals session)))
              (let* ((handle (romacs-agent-wire-handle nil "q-1"))
                     (answer (lambda (cid)
                               (funcall ask "agents/answer"
                                        (romacs-agent-rpc-object "cid" cid "id" id "handle" handle "decision" "allow"
                                                                 "answers" (romacs-agent-rpc-object
                                                                            "q1" (romacs-agent-rpc-object "options" ["o2"] "text" :null)))))))
                (should (equal (gethash "state" (car (funcall answer "c2"))) "written"))
                (should (equal (car calls) '(respond "q-1" (:answers (("Tea or coffee?" . "Coffee"))) nil)))
                (should (equal (gethash "kind" (gethash "data" (cadr (funcall answer "c3")))) "request-gone")))
              ;; The remote chat's keys: a send, and an answer through its layer.
              (setq remote (romacs-agent-remote-chat id))
              (should (wait (string-search "answer 3" (with-current-buffer remote (buffer-string)))))
              (with-current-buffer remote
                (goto-char (point-max))
                (insert "from afar")
                (romacs-agent-log-send))
              (should (wait (equal (car calls) '(send "from afar")))))
          (romacs-agent-remote-disconnect)
          (when remote (kill-buffer remote)))))))

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

;;; romacs-agent-service-tests.el ends here
