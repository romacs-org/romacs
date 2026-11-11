;;; gnosis-fsrs.el --- Binary FSRS-6 scheduler  -*- lexical-binding: t; -*-
;; Maintainer: Thanos Apollo <public@thanosapollo.org>

;; Copyright (C) 2026  Free Software Foundation, Inc.

;; Author: Thanos Apollo <public@thanosapollo.org>
;; Keywords: extensions
;; URL: https://git.thanosapollo.org/gnosis
;; Version: 0.0.1

;; This program is free software; you can redistribute it and/or modify
;; it under the terms of the GNU General Public License as published by
;; the Free Software Foundation, either version 3 of the License, or
;; (at your option) any later version.

;;; Commentary:

;; FSRS-6 transitions for Gnosis binary review outcomes: failure selects
;; Again and success Good.  The arithmetic is fsrs-rs 6.6.1, the library
;; Anki uses, through Romacs's `romacs-fsrs-next-state',
;; `romacs-fsrs-interval' and `romacs-fsrs-retrievability'.  PARAMETERS
;; arguments are a scheduler configuration's weights, nil meaning the
;; defaults below.

;;; Code:

(defconst gnosis-fsrs--algorithm "fsrs"
  "Scheduler algorithm identity stored with FSRS configuration.")

(defconst gnosis-fsrs--model "gnosis-fsrs6-v1"
  "Gnosis FSRS model identity stored with scheduler configuration.")

(defconst gnosis-fsrs--implementation "fsrs-rs-6.6.1"
  "Reference implementation identity for the pinned FSRS model.")

(defconst gnosis-fsrs-default-parameters
  [0.212 1.2931 2.3065 8.2956 6.4133 0.8334 3.0194 0.001 1.8722
   0.1666 0.796 1.4835 0.0614 0.2629 1.6483 0.6014 1.8729 0.5425
   0.0912 0.0658 0.1542]
  "FSRS-6 default parameters of fsrs-rs 6.6.1.")

(defconst gnosis-fsrs-default-retention 0.9
  "Default desired retention for FSRS reviews.")

(defconst gnosis-fsrs--maximum-stability 36500.0
  "Maximum stability in days of the FSRS-6 model.")

(defun gnosis-fsrs--finite-number-p (value)
  "Return non-nil when VALUE is a finite number."
  (and (numberp value)
       (= value value)
       (<= (abs value) 1.0e100)))

(defun gnosis-fsrs--validate-input
    (prior-state elapsed-days outcome desired-retention)
  "Validate PRIOR-STATE, ELAPSED-DAYS, OUTCOME, and DESIRED-RETENTION."
  (unless (and (integerp elapsed-days) (>= elapsed-days 0)
               (memq outcome '(failure success))
               (gnosis-fsrs--finite-number-p desired-retention)
               (< 0 desired-retention 1))
    (error "Invalid FSRS transition input"))
  (if prior-state
      (let ((stability (plist-get prior-state :stability))
            (difficulty (plist-get prior-state :difficulty)))
        (unless (and (gnosis-fsrs--finite-number-p stability)
                     (> stability 0)
                     (gnosis-fsrs--finite-number-p difficulty)
                     (<= 1 difficulty 10))
          (error "Invalid FSRS memory state")))
    (unless (zerop elapsed-days)
      (error "New FSRS state must have zero elapsed days"))))

(defun gnosis-fsrs-transition
    (prior-state elapsed-days outcome &optional desired-retention parameters)
  "Return the FSRS-6 transition for binary OUTCOME.

PRIOR-STATE is nil or a plist containing :stability and :difficulty.
ELAPSED-DAYS is the number of logical days since the prior review.
OUTCOME is either `failure' (Again) or `success' (Good).
DESIRED-RETENTION defaults to `gnosis-fsrs-default-retention'.
PARAMETERS are the model weights, nil for the defaults.
The result has :stability, :difficulty, the unrounded
:raw-interval-days and whole :calendar-interval-days, at least one."
  (let ((retention (or desired-retention gnosis-fsrs-default-retention)))
    (gnosis-fsrs--validate-input prior-state elapsed-days outcome retention)
    (let* ((memory (and prior-state
                        (cons (plist-get prior-state :stability)
                              (plist-get prior-state :difficulty))))
           (next (romacs-fsrs-next-state parameters memory elapsed-days
                                         (if (eq outcome 'failure) 1 3)
                                         retention))
           (interval (plist-get next :interval)))
      (list :stability (plist-get next :stability)
            :difficulty (plist-get next :difficulty)
            :raw-interval-days interval
            :calendar-interval-days (max 1 (round interval))))))

(defun gnosis-fsrs-interval (stability retention &optional parameters)
  "Return the unrounded interval in days for STABILITY at RETENTION.
PARAMETERS are the model weights, nil for the defaults."
  (romacs-fsrs-interval parameters stability retention))

(defun gnosis-fsrs-retrievability (elapsed-days stability &optional parameters)
  "Return recall probability ELAPSED-DAYS after a review at STABILITY.
PARAMETERS are the model weights, nil for the defaults."
  (romacs-fsrs-retrievability parameters stability elapsed-days))

(provide 'gnosis-fsrs)
;;; gnosis-fsrs.el ends here
