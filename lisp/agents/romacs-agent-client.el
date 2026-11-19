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
;; This part mirrors the session list: its rows by id and its revision,
;; each `agents/sessions.changed' applied in order, a gap or a stale
;; notice answered by watching again from the revision held.

;;; Code:

(require 'cl-lib)
(require 'romacs-agent-rpc)

(defvar romacs-agent-client-rows-functions nil
  "Functions called with a client, the ids of the rows that changed and
those removed, after its list changed.")

(cl-defstruct (romacs-agent-client (:constructor romacs-agent-client--make) (:copier nil))
  "A connection to a service: its RPC, the SERVER's welcome, the list's
ROWS by id and REVISION, the requests waiting on a reply (PENDING, id to
callback), the next request id, and the list notifications held while a
paged list reply is being read (HELD; t while reading, else nil)."
  rpc server
  (rows (make-hash-table :test #'equal))
  (revision nil)
  (pending (make-hash-table :test #'eql))
  (next 0)
  (held nil))

(defun romacs-agent-client-connect (socket &optional instance)
  "Connect to the service at SOCKET, say hello (with INSTANCE, a name this
client keeps across connections) and watch its list.  The client."
  (let* ((client (romacs-agent-client--make))
         (process (make-network-process :name "romacs-agent-client" :family 'local :service socket
                                        :noquery t :coding 'no-conversion
                                        :filter #'romacs-agent-client--filter
                                        :sentinel #'ignore)))
    (setf (romacs-agent-client-rpc client) (romacs-agent-rpc-make process client))
    (process-put process 'romacs-agent-client client)
    (romacs-agent-client-request
     client "agents/hello"
     (romacs-agent-rpc-object "protocol" "romacs-agents" "version" [1 0]
                              "client" (romacs-agent-rpc-object "name" "romacs" "version" emacs-version
                                                                "instance" (or instance (system-name)))
                              "capabilities" [])
     (lambda (result error)
       (if error (romacs-agent-client-close client)
         (setf (romacs-agent-client-server client) result)
         (romacs-agent-client--watch client nil))))
    client))

(defun romacs-agent-client-live-p (client)
  (romacs-agent-rpc-live-p (romacs-agent-client-rpc client)))

(defun romacs-agent-client-close (client)
  (romacs-agent-rpc-close (romacs-agent-client-rpc client)))

(defun romacs-agent-client-request (client method params callback)
  "Ask CLIENT's service METHOD with PARAMS; CALLBACK is called with the
result and nil, or nil and the error, once its reply comes."
  (let ((id (cl-incf (romacs-agent-client-next client))))
    (puthash id callback (romacs-agent-client-pending client))
    (romacs-agent-rpc-send (romacs-agent-client-rpc client) (romacs-agent-rpc-request id method params))
    id))

(defun romacs-agent-client--filter (process chunk)
  (when-let* ((client (process-get process 'romacs-agent-client)))
    (dolist (line (romacs-agent-rpc-lines (romacs-agent-client-rpc client) chunk))
      (when-let* ((message (romacs-agent-rpc-parse line)))
        (romacs-agent-client--receive client message)))))

(defun romacs-agent-client--receive (client message)
  (let ((id (gethash "id" message)) (method (gethash "method" message)))
    (cond
     ((and (integerp id) (gethash id (romacs-agent-client-pending client)))
      (let ((callback (gethash id (romacs-agent-client-pending client))))
        (remhash id (romacs-agent-client-pending client))
        (funcall callback (gethash "result" message) (gethash "error" message))))
     ((equal method "agents/sessions.changed")
      (let ((params (gethash "params" message)))
        (if (romacs-agent-client-held client)
            (setf (romacs-agent-client-held client)
                  (append (and (consp (romacs-agent-client-held client)) (romacs-agent-client-held client))
                          (list params)))
          (romacs-agent-client--changed client params)))))))

;;; The list

(defun romacs-agent-client--watch (client since)
  "Watch CLIENT's list from the revision SINCE (all of it with nil)."
  (setf (romacs-agent-client-held client) t)
  (romacs-agent-client-request
   client "agents/sessions.watch"
   (if since (romacs-agent-rpc-object "since" since) (romacs-agent-rpc-object))
   (lambda (result error) (romacs-agent-client--listed client result error nil nil))))

(defun romacs-agent-client--listed (client result error changed removed)
  "A part of a list reply came: RESULT, or ERROR.  CHANGED and REMOVED, the
ids the reply's earlier parts changed and removed."
  (if error
      (progn (setf (romacs-agent-client-held client) nil)
             (romacs-agent-client--watch client nil))
    (let ((rows (romacs-agent-client-rows client)))
      (when (eq (gethash "reset" result) t)
        (setq removed (append removed (hash-table-keys rows)))
        (clrhash rows))
      (seq-doseq (row (gethash "rows" result))
        (puthash (gethash "id" row) row rows)
        (push (gethash "id" row) changed))
      (seq-doseq (id (gethash "removed" result))
        (remhash id rows)
        (push id removed))
      (if (stringp (gethash "more" result))
          (romacs-agent-client-request
           client "agents/page" (romacs-agent-rpc-object "more" (gethash "more" result))
           (lambda (result error) (romacs-agent-client--listed client result error changed removed)))
        ;; Whole: its revision held, then what came meanwhile applied.
        (let ((held (romacs-agent-client-held client)))
          (setf (romacs-agent-client-revision client) (gethash "revision" result)
                (romacs-agent-client-held client) nil)
          (run-hook-with-args 'romacs-agent-client-rows-functions client changed
                              (cl-remove-if (lambda (id) (gethash id rows)) removed))
          (when (consp held)
            (dolist (params held) (romacs-agent-client--changed client params))))))))

(defun romacs-agent-client--changed (client params)
  "Apply `agents/sessions.changed' PARAMS: the next revision applied, an
earlier one ignored, a later one (a gap) or a stale notice watched again."
  (let ((revision (gethash "revision" params)) (held (romacs-agent-client-revision client)))
    (cond
     ((or (null held) (not (integerp revision)) (<= revision held)))
     ((or (/= revision (1+ held)) (eq (gethash "stale" params) t))
      (romacs-agent-client--watch client held))
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

(provide 'romacs-agent-client)
;;; romacs-agent-client.el ends here
