;;; romacs-agent-test-attached.el --- Sessions attached as a live harness leaves them, for tests -*- lexical-binding: t; -*-
;; SPDX-License-Identifier: GPL-3.0-or-later
;; Maintainer: Thanos Apollo <public@thanosapollo.org>

;;; Commentary:
;; A message goes to a persistent harness only on a live attachment that
;; has replayed its records (`romacs-agent--may-write-p').  Tests that send
;; with no supervisor give a session such an attachment: a pipe nothing
;; reads, its writes caught by stubbing `romacs-agent--line'.

;;; Code:

(require 'romacs-agent)

(defun romacs-agent-test-attach (session &optional ready)
  "Give SESSION a live, caught-up attachment, its driver READY when given.
Return SESSION."
  (setf (romacs-agent-connection session) (make-pipe-process :name "romacs-agent-test" :noquery t)
        (romacs-agent-caught-up session) t)
  (when ready
    (setf (romacs-agent-driver-state session) (plist-put (romacs-agent-driver-state session) :ready t)))
  session)

(provide 'romacs-agent-test-attached)
;;; romacs-agent-test-attached.el ends here
