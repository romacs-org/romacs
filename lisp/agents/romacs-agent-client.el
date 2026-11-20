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
\=(opening) as it is asked to open, (opened REPLY), (event PARAMS),
\(changed PARAMS), (pending-added PARAMS) or (pending-resolved PARAMS),
each in the order its revisions give, or (failed ERROR) when an open,
the first or one to recover, is refused: nothing more comes of it until
it is opened again.")

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
  (sessions (make-hash-table :test #'equal))
  (follows (make-hash-table :test #'equal)))

(defun romacs-agent-client--instance ()
  (or romacs-agent-client-instance (setq romacs-agent-client-instance (romacs-agent-client--uuid))))

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
  "The connection ended: every request waiting on it is told so, and so is
whatever follows a session through it."
  (unless (process-live-p process)
    (when-let* ((client (process-get process 'romacs-agent-client)))
      (let ((callbacks (hash-table-values (romacs-agent-client-pending client))))
        (clrhash (romacs-agent-client-pending client))
        (dolist (callback callbacks)
          (ignore-errors (funcall callback nil (romacs-agent-client--gone)))))
      (dolist (id (hash-table-keys (romacs-agent-client-follows client)))
        (run-hook-with-args 'romacs-agent-client-follow-functions client id '(ended))))))

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

(defun romacs-agent-client-command (client method params callback)
  "Ask CLIENT's service to run the command METHOD with PARAMS, named by a
client id (cid) of its own, made here: CALLBACK as for
`romacs-agent-client-request'.  The cid, which a repeat of the command
names it by."
  (let ((cid (romacs-agent-client--uuid)) (params (copy-hash-table params)))
    (puthash "cid" cid params)
    (romacs-agent-client-request client method params callback)
    cid))

(defun romacs-agent-client--uuid ()
  (format "%04x%04x-%04x-4%03x-%04x-%04x%04x%04x"
          (random 65536) (random 65536) (random 65536) (random 4096)
          (logior #x8000 (random 16384)) (random 65536) (random 65536) (random 65536)))

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
     ((member method '("agents/event" "agents/session.changed" "agents/pending.added" "agents/pending.resolved"))
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
    (run-hook-with-args 'romacs-agent-client-session-functions client id '(opening))
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
                          (list (pcase method
                                  ("agents/event" 'event) ("agents/pending.added" 'pending-added)
                                  ("agents/pending.resolved" 'pending-resolved) (_ 'changed))
                                params))))))

(defun romacs-agent-client-history (client id params callback &optional expect)
  "Ask CLIENT's service for session ID's history (`agents/session.history'
with PARAMS: upto, after or turns), every page of it; CALLBACK is called
with (UNITS CURSOR RESET FROM), UNITS the record units in order, FROM
the seq they begin at when earlier records are not read, or with
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
                        (funcall callback (list (and (not reset) units) cursor reset
                                                (and (integerp (gethash "from" result)) (gethash "from" result)))))))))
      (let ((request (copy-hash-table params)))
        (puthash "id" id request)
        (romacs-agent-client-request client "agents/session.history" request #'part)))))

;;; Following a session

;; A session followed is drawn whole and kept whole: its units in order,
;; each once, whatever the connection misses read as history.  Each open,
;; the first and one to recover, is a generation: what it reads is drawn
;; afresh, and a reply of an earlier generation is dropped.  While
;; history is read, the live units that come are held, then applied.
;; History read under another epoch or adapter than the open's, or a
;; reset, opens the session again.

(defvar romacs-agent-client-follow-functions nil
  "Functions called with a client, a session's id and what came of
following it: (drawn UNITS FROM), all of it to draw afresh, its units
beginning at seq FROM (nil, at its start); (more UNITS), units that
follow those drawn; (pending REQUESTS), what it waits on now, a table of
handle id to request, told in order with its units; (row ROW), its row
now; (failed ERROR), not followed; (ended), the connection gone.")

(cl-defstruct (romacs-agent-client--follow (:constructor romacs-agent-client--follow-make) (:copier nil))
  "A session followed: the TURNS and RECORDS its drawing begins with, the
GENERATION of its open, the CURSOR the open answered (its epoch and
adapter what history must be read under), the SEQ of the last record
drawn, the requests it waits on (PENDING, by handle id), and while
history is read (PULLING), the live notices HELD, newest first, each
\(KIND . PARAMS)."
  turns records (generation 0) cursor (seq 0) (pending (make-hash-table :test #'equal)) pulling held)

(defun romacs-agent-client-follow (client id turns records)
  "Follow session ID on CLIENT, drawn from its last TURNS turns, no more
than RECORDS records: `romacs-agent-client-follow-functions' is told."
  (puthash id (romacs-agent-client--follow-make :turns turns :records records) (romacs-agent-client-follows client))
  (romacs-agent-client-open client id))

(defun romacs-agent-client-unfollow (client id)
  "Stop following session ID on CLIENT."
  (when (gethash id (romacs-agent-client-follows client))
    (remhash id (romacs-agent-client-follows client))
    (when (romacs-agent-client-live-p client) (romacs-agent-client-close-session client id))))

(defun romacs-agent-client--tell (client id notice)
  (run-hook-with-args 'romacs-agent-client-follow-functions client id notice))

(defconst romacs-agent-client--catch-up 4000
  "The most records the service reads after a place (Pages): a read on
from where a reply stopped asks no more at a time.")

(defun romacs-agent-client--end-follow (client id error)
  "Following session ID on CLIENT ends with ERROR: its subscription closed."
  (remhash id (romacs-agent-client-follows client))
  (when (romacs-agent-client-live-p client) (romacs-agent-client-close-session client id))
  (romacs-agent-client--tell client id (list 'failed error)))

(defun romacs-agent-client--followed (client id notice)
  "What came of session ID's open on CLIENT, applied to its following."
  (when-let* ((follow (gethash id (romacs-agent-client-follows client))))
    (pcase notice
      ('(opening)
       ;; What the open before it reads is no longer drawn.
       (cl-incf (romacs-agent-client--follow-generation follow))
       (setf (romacs-agent-client--follow-pulling follow) t (romacs-agent-client--follow-held follow) nil))
      (`(opened ,reply)
       (let ((generation (cl-incf (romacs-agent-client--follow-generation follow))))
         (setf (romacs-agent-client--follow-cursor follow) (gethash "cursor" reply)
               (romacs-agent-client--follow-seq follow) 0
               (romacs-agent-client--follow-pulling follow) t
               (romacs-agent-client--follow-held follow) nil)
         (when (hash-table-p (gethash "row" reply)) (romacs-agent-client--tell client id (list 'row (gethash "row" reply))))
         (let ((pending (romacs-agent-client--follow-pending follow)))
           (clrhash pending)
           (seq-doseq (request (gethash "pending" reply)) (puthash (gethash "id" (gethash "handle" request)) request pending))
           (romacs-agent-client--tell client id (list 'pending pending)))
         (romacs-agent-client--read
          client id follow generation 'drawn (gethash "seq" (gethash "cursor" reply))
          (romacs-agent-rpc-object "upto" (gethash "seq" (gethash "cursor" reply))
                                   "turns" (romacs-agent-client--follow-turns follow)
                                   "records" (romacs-agent-client--follow-records follow)))))
      (`(,(and kind (or 'event 'pending-added 'pending-resolved)) ,params)
       (if (romacs-agent-client--follow-pulling follow)
           (push (cons kind params) (romacs-agent-client--follow-held follow))
         (romacs-agent-client--apply client id follow (cons kind params))))
      (`(changed ,params) (romacs-agent-client--tell client id (list 'row (gethash "row" params))))
      (`(failed ,error)
       (remhash id (romacs-agent-client-follows client))
       (romacs-agent-client--tell client id (list 'failed error))))))

(add-hook 'romacs-agent-client-session-functions #'romacs-agent-client--followed)

(defun romacs-agent-client--apply (client id follow notice)
  "Apply NOTICE, (KIND . PARAMS), live of session ID, to its FOLLOW: a
request it waits on or no longer; an event's gap read as history, a unit
past what is drawn passed on."
  (let* ((kind (car notice)) (params (cdr notice)) (gap (gethash "gap" params)) (seq (gethash "seq" params))
         (pending (romacs-agent-client--follow-pending follow)))
    (cond
     ((eq kind 'pending-added)
      (puthash (gethash "id" (gethash "handle" (gethash "request" params))) (gethash "request" params) pending)
      (romacs-agent-client--tell client id (list 'pending pending)))
     ((eq kind 'pending-resolved)
      (remhash (gethash "id" (gethash "handle" params)) pending)
      (romacs-agent-client--tell client id (list 'pending pending)))
     ((hash-table-p gap)
      (let ((after (max (romacs-agent-client--follow-seq follow) (gethash "after" gap))) (upto (gethash "upto" gap)))
        (when (< after upto)
          (setf (romacs-agent-client--follow-pulling follow) t)
          (romacs-agent-client--read client id follow (romacs-agent-client--follow-generation follow) 'more upto
                                     (romacs-agent-rpc-object "after" after "upto" upto)))))
     ((and (integerp seq) (> seq (romacs-agent-client--follow-seq follow)))
      (setf (romacs-agent-client--follow-seq follow) seq)
      (romacs-agent-client--tell client id (list 'more (list params)))))))

(defun romacs-agent-client--read (client id follow generation kind upto params &optional earlier)
  "Read session ID's history PARAMS asks for, to UPTO, a reply that stops
short read on from where it stopped; then tell it as KIND (`drawn' or
`more') and apply the live notices held.  EARLIER is what was read
before it stopped, (UNITS . FROM).  A reset, or history of another epoch
or adapter, opens it again."
  (romacs-agent-client-history
   client id params
   (lambda (result)
     (when (and (eq (gethash id (romacs-agent-client-follows client)) follow)
                (= generation (romacs-agent-client--follow-generation follow)))
       (pcase result
         (`(error ,error) (romacs-agent-client--end-follow client id error))
         (`(,_ ,_ t ,_) (romacs-agent-client-open client id))
         (`(,read ,cursor ,_ ,from)
          (let ((units (append (car earlier) read)) (from (if earlier (cdr earlier) from))
                (stopped (gethash "seq" cursor)))
            (if (and (integerp stopped) (< stopped upto) read)
                (romacs-agent-client--read client id follow generation kind upto
                                           (romacs-agent-rpc-object "after" stopped
                                                                    "upto" (min upto (+ stopped romacs-agent-client--catch-up)))
                                           (cons units from))
              (setf (romacs-agent-client--follow-seq follow) (max upto (romacs-agent-client--follow-seq follow)))
              (romacs-agent-client--tell client id (if (eq kind 'drawn) (list 'drawn units from) (list 'more units)))
              (romacs-agent-client--release client id follow)))))))
   (romacs-agent-client--follow-cursor follow)))

(defun romacs-agent-client--release (client id follow)
  "Apply the live notices FOLLOW held while its history was read, in
order, until one reads history again: those after it held for then."
  (let ((held (reverse (romacs-agent-client--follow-held follow))))
    (setf (romacs-agent-client--follow-pulling follow) nil (romacs-agent-client--follow-held follow) nil)
    (while held
      (romacs-agent-client--apply client id follow (pop held))
      (when (romacs-agent-client--follow-pulling follow)
        (setf (romacs-agent-client--follow-held follow) (reverse held) held nil)))))

(defun romacs-agent-client-earlier (client id upto turns records callback)
  "Read session ID's TURNS turns up to seq UPTO, no more than RECORDS, on
CLIENT, under the epoch it is followed by, a reply that stops short read
on to UPTO: CALLBACK is called with (UNITS FROM), or (error ERROR), or
\(reset) when that history is gone; with nothing, should the session be
opened again meanwhile."
  (when-let* ((follow (gethash id (romacs-agent-client-follows client))))
    (let ((generation (romacs-agent-client--follow-generation follow)) (units nil) (from nil) (first t))
      (cl-labels ((read (params)
                    (romacs-agent-client-history
                     client id params
                     (lambda (result)
                       (when (and (eq (gethash id (romacs-agent-client-follows client)) follow)
                                  (= generation (romacs-agent-client--follow-generation follow)))
                         (pcase result
                           (`(error ,_) (funcall callback result))
                           (`(,_ ,_ t ,_) (funcall callback '(reset)))
                           (`(,read ,cursor ,_ ,read-from)
                            (when first (setq from read-from first nil))
                            (setq units (append units read))
                            (let ((stopped (gethash "seq" cursor)))
                              (if (and (integerp stopped) (< stopped upto) read)
                                  (read (romacs-agent-rpc-object "after" stopped
                                                                 "upto" (min upto (+ stopped romacs-agent-client--catch-up))))
                                (funcall callback (list units from))))))))
                     (romacs-agent-client--follow-cursor follow))))
        (read (romacs-agent-rpc-object "upto" upto "turns" turns "records" records))))))

(provide 'romacs-agent-client)
;;; romacs-agent-client.el ends here
