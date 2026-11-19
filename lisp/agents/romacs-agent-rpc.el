;;; romacs-agent-rpc.el --- JSON-RPC lines over a socket, never waiting on a peer  -*- lexical-binding: t -*-
;; SPDX-License-Identifier: GPL-3.0-or-later
;; Maintainer: Thanos Apollo <public@thanosapollo.org>

;;; Commentary:
;; The agent session protocol's framing (doc/agent-protocol.org,
;; Framing), shared by the agents service and its clients: one JSON-RPC
;; 2.0 message a line, each at most `romacs-agent-rpc-message-limit'
;; bytes.  Sending never waits on the peer: a message the socket does not
;; take at once waits in its connection's queue, written as the peer
;; reads, and a peer that leaves more than `romacs-agent-rpc-queue-limit'
;; bytes unread is closed (the one overflow signal that cannot be lost).

;;; Code:

(require 'cl-lib)

(defconst romacs-agent-rpc-message-limit (* 1024 1024)
  "The most bytes of one message, its newline left out.")

(defconst romacs-agent-rpc-queue-limit (* 4 1024 1024)
  "The most bytes a connection may hold unsent before it is closed.")

(cl-defstruct (romacs-agent-rpc (:constructor romacs-agent-rpc--make) (:copier nil))
  "One connection: its PROCESS, the PARTIAL line read last, the messages
waiting to be sent (QUEUE, oldest first, and their BYTES), whether the
socket still holds part of one (HELD), and what its owner keeps (DATA)."
  process (partial "") queue (bytes 0) held data)

(defvar romacs-agent-rpc--held nil "The connections with something unsent.")
(defvar romacs-agent-rpc--timer nil "Writes what the held connections hold.")

(defun romacs-agent-rpc-make (process &optional data)
  "A connection on PROCESS, keeping DATA."
  (romacs-agent-rpc--make :process process :data data))

(defun romacs-agent-rpc-live-p (rpc)
  (process-live-p (romacs-agent-rpc-process rpc)))

(defun romacs-agent-rpc-lines (rpc chunk)
  "The whole lines CHUNK completes on RPC, in order; the rest is kept."
  (let* ((text (concat (romacs-agent-rpc-partial rpc) chunk))
         (lines (split-string text "\n")))
    (setf (romacs-agent-rpc-partial rpc) (car (last lines)))
    (delete "" (butlast lines))))

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
  (romacs-agent-rpc-object "jsonrpc" "2.0" "id" id "result" result))

(defun romacs-agent-rpc-error (id code kind message &optional data)
  "An error reply to ID: CODE and its KIND (`data.kind'), MESSAGE, and
DATA's fields beside the kind."
  (let ((fields (or data (make-hash-table :test #'equal))))
    (puthash "kind" kind fields)
    (romacs-agent-rpc-object "jsonrpc" "2.0" "id" (or id :null)
                             "error" (romacs-agent-rpc-object "code" code "message" message "data" fields))))

(defun romacs-agent-rpc-notification (method params)
  (romacs-agent-rpc-object "jsonrpc" "2.0" "method" method "params" params))

(defun romacs-agent-rpc-request (id method params)
  (romacs-agent-rpc-object "jsonrpc" "2.0" "id" id "method" method "params" params))

(defun romacs-agent-rpc-encode (message)
  "MESSAGE as its line, newline included: UTF-8 bytes."
  (concat (json-serialize message) "\n"))

(defun romacs-agent-rpc-send (rpc message)
  "Send MESSAGE (a hash table) on RPC, never waiting: what the socket does
not take now is queued, behind whatever already is.  A message over the
limit signals, never sent broken; a connection whose queue is over its
limit is closed.  Non-nil while RPC is live."
  (let ((line (romacs-agent-rpc-encode message)))
    (when (> (1- (string-bytes line)) romacs-agent-rpc-message-limit)
      (error "Agent protocol message of %d bytes is over the limit" (1- (string-bytes line))))
    (when (romacs-agent-rpc-live-p rpc)
      (if (or (romacs-agent-rpc-held rpc) (romacs-agent-rpc-queue rpc))
          (romacs-agent-rpc--enqueue rpc line)
        (romacs-agent-rpc--write rpc line))
      (romacs-agent-rpc-live-p rpc))))

(defun romacs-agent-rpc--write (rpc line)
  "Hand LINE to RPC's socket; held when it does not take all of it."
  (condition-case nil
      (unless (romacs-process-send-nowait (romacs-agent-rpc-process rpc) line)
        (setf (romacs-agent-rpc-held rpc) t)
        (romacs-agent-rpc--hold rpc))
    (error (romacs-agent-rpc-close rpc))))

(defun romacs-agent-rpc--enqueue (rpc line)
  (setf (romacs-agent-rpc-queue rpc) (nconc (romacs-agent-rpc-queue rpc) (list line))
        (romacs-agent-rpc-bytes rpc) (+ (romacs-agent-rpc-bytes rpc) (string-bytes line)))
  (if (> (romacs-agent-rpc-bytes rpc) romacs-agent-rpc-queue-limit)
      (romacs-agent-rpc-close rpc)
    (romacs-agent-rpc--hold rpc)))

(defun romacs-agent-rpc--hold (rpc)
  (unless (memq rpc romacs-agent-rpc--held) (push rpc romacs-agent-rpc--held))
  (unless (timerp romacs-agent-rpc--timer)
    (setq romacs-agent-rpc--timer (run-at-time 0.02 0.02 #'romacs-agent-rpc--drain))))

(defun romacs-agent-rpc--drain ()
  "Write what each held connection holds, as far as its socket takes it."
  (dolist (rpc romacs-agent-rpc--held)
    (if (not (romacs-agent-rpc-live-p rpc))
        (setq romacs-agent-rpc--held (delq rpc romacs-agent-rpc--held))
      (condition-case nil
          (progn
            (when (and (romacs-agent-rpc-held rpc) (romacs-process-send-nowait (romacs-agent-rpc-process rpc)))
              (setf (romacs-agent-rpc-held rpc) nil))
            (while (and (not (romacs-agent-rpc-held rpc)) (romacs-agent-rpc-queue rpc))
              (let ((line (pop (romacs-agent-rpc-queue rpc))))
                (setf (romacs-agent-rpc-bytes rpc) (- (romacs-agent-rpc-bytes rpc) (string-bytes line)))
                (unless (romacs-process-send-nowait (romacs-agent-rpc-process rpc) line)
                  (setf (romacs-agent-rpc-held rpc) t))))
            (unless (or (romacs-agent-rpc-held rpc) (romacs-agent-rpc-queue rpc))
              (setq romacs-agent-rpc--held (delq rpc romacs-agent-rpc--held))))
        (error (romacs-agent-rpc-close rpc)))))
  (when (and (null romacs-agent-rpc--held) (timerp romacs-agent-rpc--timer))
    (cancel-timer romacs-agent-rpc--timer)
    (setq romacs-agent-rpc--timer nil)))

(defun romacs-agent-rpc-close (rpc)
  "Close RPC, dropping what it held unsent."
  (setq romacs-agent-rpc--held (delq rpc romacs-agent-rpc--held))
  (setf (romacs-agent-rpc-queue rpc) nil (romacs-agent-rpc-bytes rpc) 0 (romacs-agent-rpc-held rpc) nil)
  (when (process-live-p (romacs-agent-rpc-process rpc))
    (delete-process (romacs-agent-rpc-process rpc))))

(provide 'romacs-agent-rpc)
;;; romacs-agent-rpc.el ends here
