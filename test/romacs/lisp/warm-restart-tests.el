;;; warm-restart-tests.el --- A session image's windows laid out again -*- lexical-binding: t; -*-
;; SPDX-License-Identifier: GPL-3.0-or-later
;; Maintainer: Thanos Apollo <public@thanosapollo.org>
(require 'ert)
(require 'cl-lib)
(require 'warm-restart)

(ert-deftest warm-restart-a-rebuilt-buffers-window-has-a-start ()
  ;; A window on a buffer its handler rebuilt (an agent chat, empty until
  ;; shown) is laid out from that buffer's own beginning and point, never
  ;; from no start: `window-state-put' would then leave the window's start
  ;; marker pointing nowhere, and the first `posn-at-point' on it (the
  ;; restore's own warning finding a window for itself) signalled,
  ;; cutting `normal-top-level' short.
  (let ((rebuilt (generate-new-buffer " *rebuilt*")))
    (unwind-protect
        (save-window-excursion
          (with-current-buffer rebuilt (insert "short") (goto-char 3))
          (delete-other-windows)
          (let* ((names (let ((table (make-hash-table :test #'equal)))
                          (puthash "old name" (cons rebuilt t) table)
                          table))
                 ;; The state as the image carried it: the old buffer's name,
                 ;; and the old text's start and point, far past this one's end.
                 (state (progn (set-window-buffer nil rebuilt) (window-state-get nil t)))
                 (carried (cons (car state)
                                (mapcar (lambda (item)
                                          (if (eq (car-safe item) 'buffer)
                                              `(buffer "old name" (start . 4000000) (point . 5859891)
                                                       ,@(cl-remove-if (lambda (p) (memq (car-safe p) '(start point)))
                                                                       (cddr item)))
                                            item))
                                        (cdr state))))
                 (live (cons (car carried) (warm-restart--live-tree (cdr carried) names)))
                 (props (cddr (assq 'buffer (cdr live)))))
            (should (equal (cadr (assq 'buffer (cdr live))) (buffer-name rebuilt)))
            (should (equal (alist-get 'start props) 1))
            (should (equal (alist-get 'point props) 3))
            (set-window-buffer nil (get-buffer-create "*scratch*"))
            (window-state-put live (selected-window) 'safe)
            (should (eq (window-buffer) rebuilt))
            (should (integerp (window-start)))
            (should-not (condition-case err (progn (posn-at-point 1) nil) (error err)))))
      (kill-buffer rebuilt))))

(ert-deftest warm-restart-a-start-restores-once-it-serves ()
  ;; `--restore' at a start applies nothing while the start runs: the
  ;; records come back from the command loop, a turn each, and the
  ;; `restore' step says how far it is, then that it is done.
  (let* ((warm-restart-directory (make-temp-file "romacs-wr-" t))
         (buffer (get-buffer-create "wr-carried"))
         (events nil) image)
    (unwind-protect
        (progn
          (with-current-buffer buffer (text-mode) (insert "carried\n"))
          (setq image (warm-restart-write))
          (kill-buffer buffer)
          (let ((romacs-progress-functions
                 (list (lambda (step state) (when (eq step 'restore) (push (plist-get state :status) events))))))
            (should-not (warm-restart-restore-image image t))
            (should-not (get-buffer "wr-carried"))
            (with-timeout (20 (ert-fail "the restore did not end"))
              (while (not (memq 'done events)) (accept-process-output nil 0.05)))
            (should (memq 'running events))
            (should (equal (with-current-buffer "wr-carried" (buffer-string)) "carried\n"))
            (should (eq (plist-get (alist-get 'restore romacs-progress-state) :status) 'done))))
      (ignore-errors (kill-buffer "wr-carried"))
      (delete-directory warm-restart-directory t))))

;;; warm-restart-tests.el ends here
