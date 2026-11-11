;;; romacs-backtrace.el --- backtrace frames in one walk  -*- lexical-binding: t -*-
;; SPDX-License-Identifier: GPL-3.0-or-later
;; Maintainer: Thanos Apollo <public@thanosapollo.org>

;;; Commentary:

;; GNU's `backtrace-get-frames' (backtrace.el) collects the frames with
;; `mapbacktrace', then asks `backtrace--locals' for each one and indexes
;; the list with `nth', quadratic in the stack's depth; every *Backtrace*
;; buffer is built through it.  This one takes every frame with its locals
;; from one walk (`backtrace--frames-and-locals'), and keeps GNU's
;; constructor protocol and eval-buffer positions (DIVERGENCES DBG-GET-FRAMES).

;;; Code:

(require 'backtrace)

(cl-defun backtrace-get-frames
    (&optional base &key (constructor #'backtrace-make-frame))
  "Collect all frames of current backtrace into a list.
The list will contain objects made by CONSTRUCTOR, which
defaults to `backtrace-make-frame' and which, if provided, should
be the constructor of a structure which includes
`backtrace-frame'.  If non-nil, BASE should be a function, and
frames before its nearest activation frame are discarded."
  (let ((eval-buffers eval-buffer-list)
        (frames nil))
    (dolist (entry (backtrace--frames-and-locals (or base 'backtrace-get-frames)))
      (let ((frame (funcall constructor
                            :evald (nth 0 entry) :fun (nth 1 entry)
                            :args (nth 2 entry) :flags (nth 3 entry))))
        (setf (backtrace-frame-locals frame) (nth 4 entry))
        (when (and eval-buffers (memq (backtrace-frame-fun frame)
                                      '(eval-buffer eval-region)))
          (with-current-buffer (pop eval-buffers)
            (setf (backtrace-frame-buffer frame) (current-buffer))
            (setf (backtrace-frame-pos frame) (point))))
        (push frame frames)))
    (nreverse frames)))

(provide 'romacs-backtrace)
;;; romacs-backtrace.el ends here
