(progn
;;; dired.el --- VM2: the Lisp half of dired.c's file_name_completion  -*- lexical-binding: t -*-

;; SPDX-License-Identifier: GPL-3.0-or-later
;; Maintainer: Thanos Apollo <public@thanosapollo.org>

;;; Commentary:

;; Evaluated once per VM by prims/filename.rs, the first time
;; `file-name-completion' runs.  The Rust half scans the directory; this
;; is file_name_completion's loop over the entries that begin with FILE,
;; which binds `default-directory' to the directory around the PREDICATE
;; calls, so it is Lisp.

;;; Code:

(defun romacs--file-name-completion (file directory entries predicate)
  "dired.c file_name_completion for `file-name-completion' after its scan.
ENTRIES are the entries of DIRECTORY that begin with FILE, in directory
order, each [NAME EXCLUDABLE REGEXPS DIRECTORYP]: whether it may be left
out when anything else matches, whether it matches
`completion-regexp-list', and whether it is a directory.  Return the
longest common prefix of the matches, t for a sole exact match, or nil."
  (let ((default-directory directory)
        (fold completion-ignore-case)
        (includeall t) (bestmatch nil) (bestmatchsize 0) (matchcount 0))
    (while entries
      (let* ((entry (car entries))
             (name (aref entry 0)) (excludable (aref entry 1)) (directoryp (aref entry 3)))
        (setq entries (cdr entries))
        (unless (and excludable (not includeall))
          ;; One file that is not excludable excludes the excludable ones,
          ;; including any already matched.
          (when (and includeall (not excludable))
            (setq includeall nil bestmatch nil bestmatchsize 0 matchcount 0))
          (when directoryp (setq name (file-name-as-directory name)))
          (when (and (aref entry 2) (or (null predicate) (funcall predicate name)))
            (when (<= matchcount 1) (setq matchcount (1+ matchcount)))
            (if (null bestmatch)
                (setq bestmatch name bestmatchsize (length name))
              (let* ((compare (min bestmatchsize (length name)))
                     (cmp (compare-strings bestmatch 0 compare name 0 compare fold))
                     (matchsize (if (eq cmp t) compare (1- (abs cmp))))
                     (exact (= matchsize (length name)))
                     (best-exact (= (+ matchsize (if directoryp 1 0)) (length bestmatch))))
                ;; Ignoring case, prefer an exact match, then one that keeps
                ;; the input's case.
                (when (and fold
                           (or (and exact (< (+ matchsize (if directoryp 1 0)) (length bestmatch)))
                               (and (eq exact best-exact)
                                    (eq t (compare-strings name 0 (length file) file 0 nil))
                                    (not (eq t (compare-strings bestmatch 0 (length file) file 0 nil))))))
                  (setq bestmatch name))
                (setq bestmatchsize matchsize)
                ;; Reduced to the input already: nothing can extend it.
                (when (and (<= matchsize (length file)) (not includeall)
                           (or (not fold) (= matchsize 0)) (> matchcount 1))
                  (setq entries nil))))))))
    (cond ((null bestmatch) nil)
          ((and (= matchcount 1) (equal bestmatch file)) t)
          (t (substring bestmatch 0 bestmatchsize)))))

)
;;; dired.el ends here
