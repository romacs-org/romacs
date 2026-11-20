;;; romacs-agent-client.el --- A client of an agents service: a passive mirror  -*- lexical-binding: t -*-
;; SPDX-License-Identifier: GPL-3.0-or-later
;; Maintainer: Thanos Apollo <public@thanosapollo.org>

;;; Commentary:
;; A client of the agent session protocol (doc/agent-protocol.org,
;; Clients): it connects to a service's socket (this machine's, or the
;; local end of `romacs-relay connect' to another's), says hello, and
;; mirrors what the service serves.  It folds nothing and has no policy
;; and no timer that acts: it shows what the service says.
;;
;; It mirrors the session list, and each session it opens.
;;
;; The list: its rows by id and its revision.
;; One watch at a time reads the list: its pages are staged apart and
;; the rows and revision published together once it is whole, then the
;; notifications that came meanwhile are applied.  A gap or a stale
;; notice is answered by watching again from the revision held, once the
;; watch under way, if any, is done.

;;; Code:

(require 'cl-lib)
(require 'romacs-agent-rpc)

(defvar romacs-agent-client-rows-functions nil
  "Functions called with a client, the ids of the rows that changed and
those removed, after its list changed.")

(defvar romacs-agent-client-session-functions nil
  "Functions called with a client, a session's id, and what came of it:
\=(opened REPLY), (event PARAMS) or (changed PARAMS), each in the order
its revisions give, or (failed ERROR) when an open, the first or one to
recover, is refused: nothing more comes of it until it is opened again.")

(defvar romacs-agent-client-instance nil
  "This process's name as a client, kept across its connections: a UUID
made once.")

(cl-defstruct (romacs-agent-client (:constructor romacs-agent-client--make) (:copier nil))
  "A connection to a service: its RPC, the SERVER's welcome, the list's
ROWS by id and REVISION, the requests waiting on a reply (PENDING, id to
callback), the next request id, the WATCH under way (nil, or (STAGE .
NOTICES): the rows its pages staged and the notifications held), and
whether another must follow it (AGAIN: t, or `whole' for all the list)."
  rpc server
  (rows (make-hash-table :test #'equal))
  (revision nil)
  (pending (make-hash-table :test #'eql))
  (next 0)
  watch again
  (sessions (make-hash-table :test #'equal)))

(defun romacs-agent-client--instance ()
  (or romacs-agent-client-instance
      (setq romacs-agent-client-instance
            (format "%04x%04x-%04x-4%03x-%04x-%04x%04x%04x"
                    (random 65536) (random 65536) (random 65536) (random 4096)
                    (logior #x8000 (random 16384)) (random 65536) (random 65536) (random 65536)))))

(defun romacs-agent-client-connect (socket)
  "Connect to the service at SOCKET, say hello and watch its list.  The client."
  (let* ((client (romacs-agent-client--make))
         (process (make-network-process :name "romacs-agent-client" :family 'local :service socket
                                        :noquery t :coding 'no-conversion
                                        :filter #'romacs-agent-client--filter
                                        :sentinel #'romacs-agent-client--sentinel)))
    (setf (romacs-agent-client-rpc client) (romacs-agent-rpc-make process client))
    (process-put process 'romacs-agent-client client)
    (romacs-agent-client-request
     client "agents/hello"
     (romacs-agent-rpc-object "protocol" "romacs-agents" "version" [1 0]
                              "client" (romacs-agent-rpc-object "name" "romacs" "version" emacs-version
                                                                "instance" (romacs-agent-client--instance))
                              "capabilities" [])
     (lambda (result error)
       (if error (romacs-agent-client-close client)
         (setf (romacs-agent-client-server client) result)
         (romacs-agent-client--watch client))))
    client))

(defun romacs-agent-client-live-p (client)
  (romacs-agent-rpc-live-p (romacs-agent-client-rpc client)))

(defun romacs-agent-client-close (client)
  (romacs-agent-rpc-close (romacs-agent-client-rpc client)))

(defun romacs-agent-client--sentinel (process _event)
  "The connection ended: every request waiting on it is told so."
  (unless (process-live-p process)
    (when-let* ((client (process-get process 'romacs-agent-client)))
      (let ((callbacks (hash-table-values (romacs-agent-client-pending client))))
        (clrhash (romacs-agent-client-pending client))
        (dolist (callback callbacks)
          (ignore-errors (funcall callback nil (romacs-agent-client--gone))))))))

(defun romacs-agent-client--gone ()
  (romacs-agent-rpc-object "code" -32603 "message" "The service connection ended"
                           "data" (romacs-agent-rpc-object "kind" "disconnected")))

(defun romacs-agent-client-request (client method params callback)
  "Ask CLIENT's service METHOD with PARAMS; CALLBACK is called with the
result and nil, or nil and the error, once its reply comes."
  (let ((id (cl-incf (romacs-agent-client-next client))))
    (if (not (romacs-agent-client-live-p client))
        (funcall callback nil (romacs-agent-client--gone))
      (puthash id callback (romacs-agent-client-pending client))
      (condition-case err
          (romacs-agent-rpc-send (romacs-agent-client-rpc client) (romacs-agent-rpc-request id method params))
        (error (remhash id (romacs-agent-client-pending client)) (signal (car err) (cdr err)))))
    id))

(defun romacs-agent-client--filter (process chunk)
  (when-let* ((client (process-get process 'romacs-agent-client)))
    (let ((lines (romacs-agent-rpc-lines (romacs-agent-client-rpc client) chunk)))
      (if (eq lines 'over)
          (romacs-agent-client-close client)
        (dolist (line lines)
          (when-let* ((message (romacs-agent-rpc-parse line)))
            (romacs-agent-client--receive client message)))))))

(defun romacs-agent-client--receive (client message)
  (let ((id (gethash "id" message)) (method (gethash "method" message)))
    (cond
     ((and (integerp id) (gethash id (romacs-agent-client-pending client)))
      (let ((callback (gethash id (romacs-agent-client-pending client))))
        (remhash id (romacs-agent-client-pending client))
        (funcall callback (gethash "result" message) (gethash "error" message))))
     ((member method '("agents/event" "agents/session.changed"))
      (romacs-agent-client--session-notice client method (gethash "params" message)))
     ((equal method "agents/sessions.changed")
      (let ((params (gethash "params" message)) (watch (romacs-agent-client-watch client)))
        (if watch
            (setcdr watch (nconc (cdr watch) (list params)))
          (romacs-agent-client--changed client params)))))))

;;; The list

(defun romacs-agent-client--watch (client &optional whole)
  "Watch CLIENT's list from the revision it holds (all of it with none, or
when WHOLE), unless a watch is under way: then once it is done."
  (if (romacs-agent-client-watch client)
      (setf (romacs-agent-client-again client) (if (or whole (eq (romacs-agent-client-again client) 'whole)) 'whole t))
    (let ((since (and (not whole) (romacs-agent-client-revision client)))
          (generation (gethash "generation" (gethash "server" (romacs-agent-client-server client)))))
      (setf (romacs-agent-client-watch client) (list (make-hash-table :test #'equal))
            (romacs-agent-client-again client) nil)
      (romacs-agent-client-request
       client "agents/sessions.watch"
       (if since (romacs-agent-rpc-object "since" since "generation" generation) (romacs-agent-rpc-object))
       (lambda (result error) (romacs-agent-client--listed client result error nil))))))

(defun romacs-agent-client--listed (client result error reset)
  "A part of a list reply came: RESULT, or ERROR.  RESET, whether its
first part said the list is all in the reply.  Its rows are staged
\(an id staged as `:removed' goes), published once it is whole."
  (let ((stage (car (romacs-agent-client-watch client))))
    (cond
     ((and error (equal (gethash "kind" (gethash "data" error)) "stale"))
      ;; Its continuation expired: read the list again, whole; what is
      ;; shown stays, rows and revision together, until that is in.
      (setf (romacs-agent-client-watch client) nil)
      (romacs-agent-client--watch client 'whole))
     (error
      ;; Not to be had: this client stops mirroring rather than retry.
      (setf (romacs-agent-client-watch client) nil)
      (romacs-agent-client-close client))
     (t
      (let ((reset (or reset (eq (gethash "reset" result) t))))
        (seq-doseq (row (gethash "rows" result)) (puthash (gethash "id" row) row stage))
        (seq-doseq (id (gethash "removed" result)) (puthash id :removed stage))
        (if (stringp (gethash "more" result))
            (romacs-agent-client-request
             client "agents/page" (romacs-agent-rpc-object "more" (gethash "more" result))
             (lambda (result error) (romacs-agent-client--listed client result error reset)))
          (romacs-agent-client--publish client stage reset (gethash "revision" result))))))))

(defun romacs-agent-client--publish (client stage reset revision)
  "Make STAGE CLIENT's rows (all of them when RESET) at REVISION, then
apply what came meanwhile, and watch again when that was asked for."
  (let ((rows (romacs-agent-client-rows client)) (notices (cdr (romacs-agent-client-watch client)))
        (changed nil) (removed nil))
    (when reset
      (maphash (lambda (id _) (unless (gethash id stage) (push id removed))) rows)
      (clrhash rows))
    (maphash (lambda (id row)
               (if (eq row :removed)
                   (when (gethash id rows) (remhash id rows) (push id removed))
                 (puthash id row rows)
                 (push id changed)))
             stage)
    (setf (romacs-agent-client-revision client) revision (romacs-agent-client-watch client) nil)
    (run-hook-with-args 'romacs-agent-client-rows-functions client changed removed)
    (dolist (params notices) (romacs-agent-client--changed client params))
    (when-let* ((again (romacs-agent-client-again client)))
      (romacs-agent-client--watch client (eq again 'whole)))))

(defun romacs-agent-client--changed (client params)
  "Apply `agents/sessions.changed' PARAMS: the next revision applied, an
earlier one ignored, a later one (a gap) or a stale notice watched again."
  (let ((revision (gethash "revision" params)) (held (romacs-agent-client-revision client)))
    (cond
     ((romacs-agent-client-watch client)
      (setcdr (romacs-agent-client-watch client) (nconc (cdr (romacs-agent-client-watch client)) (list params))))
     ((or (null held) (not (integerp revision)) (<= revision held)))
     ((or (/= revision (1+ held)) (eq (gethash "stale" params) t))
      (romacs-agent-client--watch client))
     (t
      (let ((rows (romacs-agent-client-rows client)) (changed nil) (removed nil))
        (seq-doseq (row (gethash "rows" params))
          (puthash (gethash "id" row) row rows)
          (push (gethash "id" row) changed))
        (seq-doseq (id (gethash "removed" params))
          (remhash id rows)
          (push id removed))
        (setf (romacs-agent-client-revision client) revision)
        (run-hook-with-args 'romacs-agent-client-rows-functions client changed removed))))))

;;; Sessions

;; Each session opened is, by id, the state of its open: (opening) while
;; it is asked, then [REVISION] once it is answered.  A reply
;; is applied only while the state it was asked with is still the
;; session's; a notice before it is an earlier subscription's, as the
;; service answers an open before it sends anything of it, and is dropped.

(defun romacs-agent-client-open (client id)
  "Open session ID on CLIENT: its reply, then each event and change of it,
reach `romacs-agent-client-session-functions'."
  (let ((asked (list 'opening)))
    (puthash id asked (romacs-agent-client-sessions client))
    (romacs-agent-client-request
     client "agents/session.open" (romacs-agent-rpc-object "id" id)
     (lambda (result error)
       (when (eq (gethash id (romacs-agent-client-sessions client)) asked)
         (if error
             (progn (remhash id (romacs-agent-client-sessions client))
                    (run-hook-with-args 'romacs-agent-client-session-functions client id (list 'failed error)))
           (puthash id (vector (gethash "revision" result)) (romacs-agent-client-sessions client))
           (run-hook-with-args 'romacs-agent-client-session-functions client id (list 'opened result))))))))

(defun romacs-agent-client-close-session (client id)
  "Close session ID on CLIENT."
  (remhash id (romacs-agent-client-sessions client))
  (romacs-agent-client-request client "agents/session.close" (romacs-agent-rpc-object "id" id) #'ignore))

(defun romacs-agent-client--session-notice (client method params)
  "Apply METHOD's PARAMS to their session: dropped while it opens, the next
revision passed on (an event's `gap', records it was never sent live,
for its reader to pull as history), an earlier one ignored, a missed one
answered by opening it again.  Its history's epoch is checked where
history is read, against the open's."
  (let* ((id (gethash "session" params)) (state (gethash id (romacs-agent-client-sessions client)))
         (revision (gethash "revision" params)))
    (cond
     ((null state))
     ((eq (car-safe state) 'opening))
     ((not (integerp revision)))
     ((<= revision (aref state 0)))
     ((/= revision (1+ (aref state 0)))
      (romacs-agent-client-open client id))
     (t
      (aset state 0 revision)
      (run-hook-with-args 'romacs-agent-client-session-functions client id
                          (list (if (equal method "agents/event") 'event 'changed) params))))))

(defun romacs-agent-client-history (client id params callback &optional expect)
  "Ask CLIENT's service for session ID's history (`agents/session.history'
with PARAMS: upto, after or turns), every page of it; CALLBACK is called
with (UNITS CURSOR RESET), UNITS the record units in order, or with
\=(error ERROR).  With EXPECT, the cursor (a table) the history already
held was read under, a reply of another epoch or adapter is a reset: its
units are not to be joined to that history."
  (let ((units nil))
    (cl-labels ((part (result error)
                  (if error (funcall callback (list 'error error))
                    (setq units (append units (append (gethash "units" result) nil)))
                    (if (stringp (gethash "more" result))
                        (romacs-agent-client-request client "agents/page"
                                                     (romacs-agent-rpc-object "more" (gethash "more" result)) #'part)
                      (let* ((cursor (gethash "cursor" result))
                             (reset (or (eq (gethash "reset" result) t)
                                        (and expect (not (and (equal (gethash "epoch" cursor) (gethash "epoch" expect))
                                                              (equal (gethash "adapter" cursor) (gethash "adapter" expect))))))))
                        (funcall callback (list (and (not reset) units) cursor reset)))))))
      (let ((request (copy-hash-table params)))
        (puthash "id" id request)
        (romacs-agent-client-request client "agents/session.history" request #'part)))))

(provide 'romacs-agent-client)
;;; romacs-agent-client.el ends here
