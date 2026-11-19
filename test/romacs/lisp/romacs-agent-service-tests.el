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
  (romacs-agent-service-tests--with
    (let ((romacs-agent-board--unloaded 'the-boards))
      (romacs-agent-service-start socket)
      (romacs-agent-service--refresh t)
      (should (eq romacs-agent-board--unloaded 'the-boards)))))

(ert-deftest romacs-agent-service-runs-its-whole-pass-when-quiet ()
  (romacs-agent-service-tests--with
    (romacs-agent-service-start socket)
    (setq romacs-agent-service--whole-at 0 romacs-agent-service--dirty nil)
    (when (timerp romacs-agent-service--refresh-timer) (cancel-timer romacs-agent-service--refresh-timer))
    (setq romacs-agent-service--refresh-timer nil)
    (romacs-agent-service--poll)
    (should (timerp romacs-agent-service--refresh-timer))))

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

;;; romacs-agent-service-tests.el ends here
