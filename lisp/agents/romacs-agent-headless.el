;;; romacs-agent-headless.el --- The agents service with no frame  -*- lexical-binding: t -*-
;; SPDX-License-Identifier: GPL-3.0-or-later
;; Maintainer: Thanos Apollo <public@thanosapollo.org>

;;; Commentary:
;; The headless agents service (doc/agent-protocol.org, Transports): a
;; batch Romacs that serves this machine's agent sessions and nothing
;; else, for a machine no editor runs them on.
;;
;;   romacs-vm-gui --batch -l romacs-agent-headless -f romacs-agent-headless-run
;;
;; `romacs-relay stdio' starts it so, as the user unit
;; `romacs-agents-service', when nothing answers at the service's socket.

;;; Code:

(require 'romacs-agent)
(require 'romacs-agent-service)

(defconst romacs-agent-headless--wait 1
  "Seconds one wait for input lasts before the loop waits again.")

;;;###autoload
(defun romacs-agent-headless-run ()
  "Serve this machine's agent sessions with no frame, until killed.
Start the agents service on its sockets (`romacs-agent-service-start'),
restore the recorded sessions a turn at a time, attaching to each whose
host runs (`romacs-agent-restore-soon'; a failure is reported and
serving goes on), then wait for input forever, so
that connections, hosts and timers are served.  The service's sockets
are removed as the process is killed (`kill-emacs-hook')."
  (let ((socket (romacs-agent-service-start)))
    (message "romacs-agent-headless: serving %s" socket))
  ;; A restore that fails is reported; the service goes on serving.
  (with-demoted-errors "romacs-agent-headless: restoring the sessions: %S" (romacs-agent-restore-soon))
  (while t (accept-process-output nil romacs-agent-headless--wait)))

(provide 'romacs-agent-headless)
;;; romacs-agent-headless.el ends here
