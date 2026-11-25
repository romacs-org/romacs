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
;; A batch start reads no init file, so it takes the GUI's steps first
;; (saved settings, packages, the init): the service runs the user's
;; approval policy, providers and options, as their editor would.

;;; Code:

(require 'romacs-agent)
(require 'romacs-agent-service)

(defconst romacs-agent-headless--wait 1
  "Seconds one wait for input lasts before the loop waits again.")

(require 'romacs-init-load)

(declare-function romacs-saved-settings-start "romacs-saved-settings" ())
(declare-function romacs--activate-packages "romacs-loadup-bootstrap" ())
(declare-function romacs--after-init "romacs-loadup-bootstrap" ())
(defvar romacs-loadup-delayed-init-variables)

(defconst romacs-agent-headless--failure-most 4096
  "The most characters of what failed that hello tells: a welcome stays
far below the protocol's message limit.")

(defun romacs-agent-headless--failed (what said)
  "Say that WHAT failed, SAID why, on stderr and to every client of the
service (`romacs-agent-service-init-failure', cut to
`romacs-agent-headless--failure-most' characters)."
  (let* ((line (format "%s: %s" what said))
         (all (if romacs-agent-service-init-failure (concat romacs-agent-service-init-failure "\n" line) line)))
    (message "romacs-agent-headless: %s" line)
    (setq romacs-agent-service-init-failure
          (substring all 0 (min (length all) romacs-agent-headless--failure-most)))))

(defmacro romacs-agent-headless--step (what &rest body)
  "Run BODY, a startup step; an error in it is said as WHAT failing
\(`romacs-agent-headless--failed') and the next step goes on."
  (declare (indent 1))
  `(condition-case err (progn ,@body)
     (error (romacs-agent-headless--failed ,what (error-message-string err)))))

(defun romacs-agent-headless--load-init ()
  "Take the GUI's per-user steps before its first frame: its per-user
directory (`romacs-init-directory') as `user-emacs-directory', the
settings computed from it again (`romacs-loadup-delayed-init-variables',
the packages' directory among them), the saved settings, the installed
packages, its init.el through `romacs-init-load', then the end of
initialization (`romacs--after-init'), which a batch start took before
any of them: only what they put on `after-init-hook' runs now.  A step
that fails is said (`romacs-agent-headless--failed') and the next goes
on."
  (let ((file (expand-file-name "init.el" (romacs-init-directory)))
        (after-init-hook nil))
    (setq user-emacs-directory (romacs-init-directory) user-init-file file)
    (romacs-agent-headless--step "per-user settings"
      (let ((user-emacs-directory-warning nil))
        (mapc #'custom-reevaluate-setting romacs-loadup-delayed-init-variables)))
    (romacs-agent-headless--step "saved settings"
      (when (fboundp 'romacs-saved-settings-start) (romacs-saved-settings-start)))
    (romacs-agent-headless--step "packages"
      (when (fboundp 'romacs--activate-packages) (romacs--activate-packages)))
    (when (file-exists-p file)
      (when-let* ((said (romacs-init-load file)))
        (romacs-agent-headless--failed "init" said)))
    (romacs-agent-headless--step "after-init-hook" (romacs--after-init))))

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
