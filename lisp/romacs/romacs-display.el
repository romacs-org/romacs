;;; romacs-display.el --- the switches of redisplay -*- lexical-binding: t -*-

;; SPDX-License-Identifier: GPL-3.0-or-later
;; Maintainer: Thanos Apollo <public@thanosapollo.org>

;;; Commentary:

;; Redisplay answers a window from its last snapshot while what that
;; snapshot read is unchanged (crates/romacs-vm/src/display.rs,
;; `reuse_shown').  `romacs--display-stats' counts what it did.

;;; Code:

(defvar romacs-display-check nil
  "Non-nil makes redisplay build in full the text of each window it reuses,
and compare the two: text, face runs, start, point, stretches, images and
line numbers.  A difference is an input the reuse missed; it is counted
\(`check-mismatches' in `romacs--display-stats') and logged once per window
and cause as a display diagnostic.  For tests: it costs a full build of
every reused window.")

(provide 'romacs-display)
;;; romacs-display.el ends here
