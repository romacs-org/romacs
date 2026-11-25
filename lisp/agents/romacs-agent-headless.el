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
;; A batch start reads no init file, so it loads the user's own first: the
;; service runs their approval policy, providers and options, as their
;; editor would.

;;; Code:

(require 'romacs-agent)
(require 'romacs-agent-service)

(defconst romacs-agent-headless--wait 1
  "Seconds one wait for input lasts before the loop waits again.")

(defun romacs-agent-headless--init-files ()
  "The user's init files that exist, as an editor's start finds them:
early-init.el in `user-emacs-directory', then ~/.emacs, ~/.emacs.el or
that directory's init.el, the first there."
  (let* ((dir user-emacs-directory) (early (expand-file-name "early-init.el" dir)))
    (delq nil (list (and (file-exists-p early) early)
                    (seq-find #'file-exists-p
                              (list "~/.emacs" "~/.emacs.el" (expand-file-name "init.el" dir)))))))

(defun romacs-agent-headless--load-init ()
  "Load the user's init files (`romacs-agent-headless--init-files'), the
last one as `user-init-file'; an error in one is reported and the next
goes on."
  (dolist (file (romacs-agent-headless--init-files))
    (setq user-init-file (expand-file-name file))
    (with-demoted-errors "romacs-agent-headless: loading the init file: %S"
      (load file nil t t))))

;;;###autoload
(defun romacs-agent-headless-run ()
  "Serve this machine's agent sessions with no frame, until killed.
Load the user's init files (`romacs-agent-headless--load-init'), start
the agents service on its sockets unless the init did
\(`romacs-agent-service-start'), restore the recorded sessions a turn at
a time, attaching to each whose host runs (`romacs-agent-restore-soon';
a failure is reported and serving goes on), then wait for input forever,
so that connections, hosts and timers are served.  A socket a kill left
is replaced as stale at the next start."
  (romacs-agent-headless--load-init)
  (unless romacs-agent-service--listeners
    (romacs-agent-service-start))
  (message "romacs-agent-headless: serving %s" (romacs-agent-service-socket))
  ;; A restore that fails is reported; the service goes on serving.
  (with-demoted-errors "romacs-agent-headless: restoring the sessions: %S" (romacs-agent-restore-soon))
  (while t (accept-process-output nil romacs-agent-headless--wait)))

(provide 'romacs-agent-headless)
;;; romacs-agent-headless.el ends here
