;;; romacs-agent-rpc.el --- JSON-RPC lines over a socket, never waiting on a peer  -*- lexical-binding: t -*-
;; SPDX-License-Identifier: GPL-3.0-or-later
;; Maintainer: Thanos Apollo <public@thanosapollo.org>

;;; Commentary:
;; The agent session protocol's framing (doc/agent-protocol.org,
;; Framing), shared by the agents service and its clients: one JSON-RPC
;; 2.0 message a line, each at most `romacs-agent-rpc-message-limit'
;; bytes, split by the process layer as it reads (`set-process-framing').
;; Sending never waits on the peer: what the socket does not take at once
;; waits in its channel, written as the peer reads, and a peer that leaves
;; more than `romacs-agent-rpc-queue-limit' bytes unread is closed (the one
;; overflow signal that cannot be lost).

;;; Code:

(require 'cl-lib)

(defconst romacs-agent-rpc-message-limit (* 1024 1024)
  "The most bytes of one message, its newline left out.")

(defconst romacs-agent-rpc-queue-limit (* 4 1024 1024)
  "The most bytes a connection may hold unsent before it is closed.")

(cl-defstruct (romacs-agent-rpc (:constructor romacs-agent-rpc--make) (:copier nil))
  "One connection: its PROCESS and what its owner keeps (DATA)."
  process data)

(defun romacs-agent-rpc-make (process &optional data)
  "A connection on PROCESS, keeping DATA: PROCESS's output is framed in
lines of at most `romacs-agent-rpc-message-limit' bytes, so its filter
is called with the lines it read (`romacs-agent-rpc-lines')."
  (when (processp process)
    (set-process-framing process 'lines romacs-agent-rpc-message-limit))
  (romacs-agent-rpc--make :process process :data data))

(defun romacs-agent-rpc-live-p (rpc)
  "Non-nil while RPC's process is live."
  (process-live-p (romacs-agent-rpc-process rpc)))

(defun romacs-agent-rpc-lines (_rpc frames)
  "The lines of FRAMES, what RPC's filter was called with, in order, empty
ones dropped; `over' when one was over `romacs-agent-rpc-message-limit'
bytes (the caller closes the peer)."
  (if (memq 'over frames)
      'over
    (delete "" frames)))

(defun romacs-agent-rpc-parse (line)
  "LINE, UTF-8 bytes as read, as a message (a hash table), or nil when it
is no JSON object."
  (let ((message (ignore-errors (json-parse-string (if (multibyte-string-p line) line (decode-coding-string line 'utf-8))
                                                   :object-type 'hash-table
                                                   :null-object :null :false-object :false))))
    (and (hash-table-p message) message)))

(defun romacs-agent-rpc-object (&rest pairs)
  "A JSON object of PAIRS, keys and values alternating."
  (let ((object (make-hash-table :test #'equal)))
    (cl-loop for (key value) on pairs by #'cddr do (puthash key value object))
    object))

(defun romacs-agent-rpc-result (id result)
  "A JSON-RPC 2.0 success reply to request ID, carrying RESULT."
  (romacs-agent-rpc-object "jsonrpc" "2.0" "id" id "result" result))

(defun romacs-agent-rpc-error (id code kind message &optional data)
  "An error reply to ID (null when nil): CODE and its KIND (`data.kind'),
MESSAGE, and DATA's fields beside the kind.  DATA, a table, is the reply's
data itself: its \"kind\" is set in place."
  (let ((fields (or data (make-hash-table :test #'equal))))
    (puthash "kind" kind fields)
    (romacs-agent-rpc-object "jsonrpc" "2.0" "id" (or id :null)
                             "error" (romacs-agent-rpc-object "code" code "message" message "data" fields))))

(defun romacs-agent-rpc-notification (method params)
  "A JSON-RPC 2.0 notification of METHOD with PARAMS: it has no id, so
no reply answers it."
  (romacs-agent-rpc-object "jsonrpc" "2.0" "method" method "params" params))

(defun romacs-agent-rpc-request (id method params)
  "A JSON-RPC 2.0 request of METHOD with PARAMS, which its reply answers by ID."
  (romacs-agent-rpc-object "jsonrpc" "2.0" "id" id "method" method "params" params))

(defun romacs-agent-rpc-encode (message)
  "MESSAGE as its line, newline included: UTF-8 bytes."
  (concat (json-serialize message) "\n"))

(defun romacs-agent-rpc-send (rpc message)
  "Send MESSAGE (a hash table) on RPC, never waiting: what the socket does
not take now waits in its channel, behind whatever already does, written
as the peer reads.  A message over the limit signals, never sent broken;
a connection left holding more than `romacs-agent-rpc-queue-limit' bytes
unsent once its socket took what it could is closed.  Non-nil while RPC
is live."
  (let ((line (romacs-agent-rpc-encode message)))
    (when (> (1- (string-bytes line)) romacs-agent-rpc-message-limit)
      (error "Agent protocol message of %d bytes is over the limit" (1- (string-bytes line))))
    (when (romacs-agent-rpc-live-p rpc)
      (romacs-agent-rpc--write rpc line)
      ;; What the peer left unread, this message's rest included.
      (when (and (romacs-agent-rpc-live-p rpc)
                 (> (romacs-process-unsent-bytes (romacs-agent-rpc-process rpc)) romacs-agent-rpc-queue-limit))
        (romacs-agent-rpc-close rpc))
      (romacs-agent-rpc-live-p rpc))))

(defun romacs-agent-rpc--write (rpc line)
  "Hand LINE to RPC's channel, which writes it as the peer reads; a write
that fails closes RPC."
  (condition-case nil
      (romacs-process-send-nowait (romacs-agent-rpc-process rpc) line)
    (error (romacs-agent-rpc-close rpc))))

(defun romacs-agent-rpc-close (rpc)
  "Close RPC, dropping what it held unsent."
  (when (process-live-p (romacs-agent-rpc-process rpc))
    (delete-process (romacs-agent-rpc-process rpc))))

(provide 'romacs-agent-rpc)
;;; romacs-agent-rpc.el ends here
