;;; world-digest.el --- What a VM's Lisp observes, digested root by root  -*- lexical-binding: t -*-
;; SPDX-License-Identifier: GPL-3.0-or-later

;;; Commentary:
;; The differential checker of the world snapshots (DESIGN-M1 §10, E2):
;; loaded at the same point in a VM booted from source and in one restored
;; from a world, it writes one line per Lisp-visible root to the file
;; `world-digest-file' (or $WORLD_DIGEST_OUT), ROOT TAB NAME TAB SHA-1 of
;; the root's printed value.  gnu-tests/tools/world_diff.py compares two
;; such files.  It reads through Lisp only, independently of how a world
;; is encoded: every interned symbol's default value, function and plist,
;; the obarray's members in the order symbols were made, the features,
;; load history and load path, every buffer's text,
;; properties, point, narrowing, locals, overlays and modification state,
;; every frame's parameters and window tree, the faces, the charset and
;; coding priorities and the match data.
;;
;; Values print with `print-circle', so sharing and cycles are part of
;; the text, and with every length and level limit off.  An ordinary
;; timer's due time is the clock's, not the state's: it prints as
;; `world-digest--time', and the rest of the timer, its function,
;; arguments and repeat, as it is.  An idle timer's time fields hold its
;; delay, not a clock reading, and print as they are.  It collects first:
;; what a weak table still holds of keys nothing else reaches follows when
;; the last collection ran, not the state; what is reachable is compared.  The last line counts the
;; ones before it, so a digest cut short is no digest.

;;; Code:

(defvar world-digest-file (getenv-internal "WORLD_DIGEST_OUT")
  "The file `world-digest' writes.")

(defvar world-digest-show nil
  "Names of roots whose printed text `world-digest' also writes in full,
to `world-digest-file' with \".show\" appended, to see why they differ;
from $WORLD_DIGEST_SHOW when nil.")

(defvar world-digest--shown nil)

(defun world-digest--text (value)
  "VALUE as `prin1' prints it with sharing shown and no limits."
  (let ((print-circle t) (print-length nil) (print-level nil)
        (print-escape-newlines t) (print-escape-control-characters t)
        (print-quoted t) (print-gensym t))
    (condition-case err
        (prin1-to-string value)
      (error (format "#<unprintable %S>" err)))))

(defmacro world-digest--safely (form)
  "FORM's value, or the error it signals: a root Lisp cannot read is
digested as that error, alike in both VMs when they fail alike."
  `(condition-case err ,form (error (list 'world-digest--error err))))

(defun world-digest--timer-p (value)
  "Whether VALUE is a timer as timer.el makes it: a vector of triggered,
high, low and micro seconds, repeat, function, arguments, idle delay,
picoseconds and integral multiple."
  (and (vectorp value) (= (length value) 10) (memq (aref value 0) '(nil t))
       (aref value 5)
       (let ((ok t))
         (dolist (slot '(1 2 3 8) ok)
           (unless (or (null (aref value slot)) (integerp (aref value slot)))
             (setq ok nil))))))

(defun world-digest--timeless (value)
  "VALUE with the due time of each ordinary timer it is, or lists,
replaced.  An idle timer, its idle slot set, keeps its delay there."
  (cond ((and (world-digest--timer-p value) (null (aref value 7)))
         (let ((copy (copy-sequence value)))
           (dolist (slot '(1 2 3 8) copy)
             (aset copy slot 'world-digest--time))))
        ((and (consp value) (proper-list-p value) (world-digest--any-timer-p value))
         (mapcar #'world-digest--timeless value))
        (t value)))

(defun world-digest--any-timer-p (list)
  "Whether LIST, a proper list, has a timer among its elements."
  (catch 'found
    (dolist (element list)
      (when (world-digest--timer-p element)
        (throw 'found t)))))

(defvar world-digest--lines nil
  "The lines so far, last first.")

(defun world-digest--line (root name value)
  "Add ROOT, NAME and the digest of VALUE's text as a line."
  (let ((text (world-digest--text (world-digest--timeless value))))
    (when (member name world-digest-show)
      (push (format "%s %s %s\n" root name text) world-digest--shown))
    (push (concat root "\t" name "\t" (secure-hash 'sha1 (encode-coding-string text 'utf-8-emacs)) "\n")
          world-digest--lines)))

(defun world-digest--interned ()
  "Every interned symbol's name in `mapatoms' order, which is the order
symbols were made, but this file's own."
  (let (names)
    (mapatoms (lambda (symbol)
                (unless (string-prefix-p "world-digest" (symbol-name symbol))
                  (push (symbol-name symbol) names))))
    (nreverse names)))

(defun world-digest--symbols ()
  "Every interned symbol, by name, but this file's own."
  (let (symbols)
    (mapatoms (lambda (symbol)
                (unless (string-prefix-p "world-digest" (symbol-name symbol))
                  (push symbol symbols))))
    (sort symbols (lambda (a b) (string< (symbol-name a) (symbol-name b))))))

(defun world-digest--buffer (buffer)
  "What Lisp observes of BUFFER, as one list."
  (with-current-buffer buffer
    (save-restriction
      (let ((narrowing (list (point-min) (point-max))))
        (widen)
        (list (buffer-name) narrowing (point) (mark t)
              (buffer-substring (point-min) (point-max))
              (buffer-modified-p) (buffer-modified-tick) (buffer-chars-modified-tick)
              enable-multibyte-characters major-mode
              (sort (mapcar (lambda (local) (if (consp local) local (list local)))
                            (buffer-local-variables))
                    (lambda (a b) (string< (symbol-name (car a)) (symbol-name (car b)))))
              (mapcar (lambda (o) (list (overlay-start o) (overlay-end o) (overlay-properties o)))
                      (overlays-in (point-min) (point-max)))
              (syntax-table) (current-local-map) (current-case-table))))))

(defun world-digest ()
  "Write the digest of every root to `world-digest-file'.
No buffer is made until every root is read."
  (garbage-collect)
  (let ((symbols (world-digest--symbols)))
    (setq world-digest--lines nil world-digest--shown nil)
    (unless world-digest-show
      (setq world-digest-show (split-string (or (getenv-internal "WORLD_DIGEST_SHOW") ""))))
    (dolist (symbol symbols)
      (let ((name (symbol-name symbol)))
        (world-digest--line "value" name
                            (if (default-boundp symbol) (default-value symbol) 'world-digest--void))
        (world-digest--line "function" name
                            (if (fboundp symbol) (symbol-function symbol) 'world-digest--void))
        (world-digest--line "plist" name (symbol-plist symbol))
        (world-digest--line "special" name (special-variable-p symbol))))
    (world-digest--line "obarray" "names" (mapcar #'symbol-name symbols))
    (world-digest--line "obarray" "order" (world-digest--interned))
    (world-digest--line "features" "features" features)
    (world-digest--line "load-history" "load-history" load-history)
    (world-digest--line "load-path" "load-path" load-path)
    (dolist (buffer (buffer-list))
      (world-digest--line "buffer" (buffer-name buffer) (world-digest--safely (world-digest--buffer buffer))))
    (dolist (frame (frame-list))
      (world-digest--line "frame" (format "%S" (frame-parameter frame 'name))
                          (list (world-digest--safely (frame-parameters frame))
                                (world-digest--safely (window-tree frame)))))
    (dolist (face (face-list))
      (world-digest--line "face" (symbol-name face)
                          (world-digest--safely (face-all-attributes face (selected-frame)))))
    (world-digest--line "charset" "priority" (world-digest--safely (charset-priority-list)))
    (world-digest--line "coding" "priority" (world-digest--safely (coding-system-priority-list)))
    (world-digest--line "match-data" "match-data" (match-data))
    (let ((lines (nreverse world-digest--lines)) (shown (nreverse world-digest--shown)))
      (setq world-digest--lines nil world-digest--shown nil)
      (with-temp-file world-digest-file
        (dolist (line lines) (insert line))
        (insert (format "end\tlines\t%d\n" (length lines))))
      (when shown
        (with-temp-file (concat world-digest-file ".show")
          (dolist (line shown) (insert line)))))
    (length symbols)))

(when world-digest-file
  (message "WORLD-DIGEST symbols=%d file=%s" (world-digest) world-digest-file))

(provide 'world-digest)
;;; world-digest.el ends here
