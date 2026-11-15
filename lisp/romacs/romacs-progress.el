;;; romacs-progress.el --- How far a start's long steps are  -*- lexical-binding: t -*-

;; SPDX-License-Identifier: GPL-3.0-or-later
;; Maintainer: Thanos Apollo <public@thanosapollo.org>

;;; Commentary:

;; A start's long steps say how far they are as they go: the session
;; image's restore, the agent sessions' load.  `romacs-progress' keeps each
;; step's latest state in `romacs-progress-state', where a restart's
;; supervisor reads it over the server's socket, and runs
;; `romacs-progress-functions', which a loading screen shows.

;;; Code:

(defvar romacs-progress-functions nil
  "Abnormal hook run with STEP and STATE as a start's step goes on.
STEP is a symbol: `restore' (the session image), `agents' (agent
sessions loading).  STATE is a plist: :status `running', `done' or
`failed'; :done and :of, counts so far and in all; :name, what the step
is at; :error, why it failed; :seconds, since the step began.")

(defvar romacs-progress-state nil
  "Each step's latest state, as (STEP . STATE); see `romacs-progress-functions'.")

(defun romacs-progress (step &rest state)
  "Say STEP is at STATE (`romacs-progress-functions'); a function that
fails is reported, and the step goes on."
  (setf (alist-get step romacs-progress-state) state)
  (with-demoted-errors "romacs-progress: %S"
    (run-hook-with-args 'romacs-progress-functions step state)))

(provide 'romacs-progress)
;;; romacs-progress.el ends here
