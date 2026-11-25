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

(declare-function romacs-saved-settings-start "romacs-saved-settings" ())
(declare-function romacs-init-load "romacs-init-load" (file))
(declare-function romacs--activate-packages "romacs-loadup-bootstrap" ())

(defun romacs-agent-headless--init-file ()
  "The user's Romacs init file, as the GUI names it
\(crates/romacs-vm-gui/src/user_init.rs): init.el in romacs/ under
XDG_CONFIG_HOME when that is absolute, else under ~/.config."
  (let ((config (getenv "XDG_CONFIG_HOME")))
    (expand-file-name "romacs/init.el" (if (and config (file-name-absolute-p config)) config "~/.config"))))

(defun romacs-agent-headless--load-init ()
  "Take the GUI's steps before its first frame: name the init file (as
`user-init-file', its directory `user-emacs-directory'), load the saved
settings, activate the installed packages, then load the init through
`romacs-init-load'.  A step that fails is reported and the next goes on."
  (let ((file (romacs-agent-headless--init-file)))
    (setq user-init-file file user-emacs-directory (file-name-directory file))
    (with-demoted-errors "romacs-agent-headless: saved settings: %S"
      (when (fboundp 'romacs-saved-settings-start) (romacs-saved-settings-start)))
    (with-demoted-errors "romacs-agent-headless: packages: %S"
      (when (fboundp 'romacs--activate-packages) (romacs--activate-packages)))
    (when (file-exists-p file)
      (let ((said (if (fboundp 'romacs-init-load) (romacs-init-load file)
                    (with-demoted-errors "%S" (load file nil t t) nil))))
        (when (stringp said) (message "romacs-agent-headless: init failed: %s" said))))))

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
