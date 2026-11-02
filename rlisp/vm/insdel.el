(progn
;;; insdel.el --- VM2: the Lisp half of buffer text changes  -*- lexical-binding: t -*-

;; SPDX-License-Identifier: GPL-3.0-or-later
;; Maintainer: Thanos Apollo <public@thanosapollo.org>

;;; Commentary:

;; Evaluated once per VM by prims/insdel.rs, the first time a text change
;; has Lisp to run.  The changes themselves are native (insdel.c); these
;; run the hooks GNU runs around one, called only when one can run, with
;; the change's region in BUFFER; and cmds.c's self-insertion, which calls
;; Lisp per character.

;;; Code:

(defun romacs--overlay-hooks (overlays after beg end &optional length)
  "Run report_overlay_modification's hooks OVERLAYS, (PROP . OVERLAY)
pairs, for the overlays still in the current buffer: each function of
PROP with the overlay, AFTER, BEG and END, and after a change LENGTH."
  (while overlays
    (let ((overlay (cdr (car overlays)))
          (functions (car (car overlays))))
      (if (eq (overlay-buffer overlay) (current-buffer))
          (while (consp functions)
            (if after
                (funcall (car functions) overlay t beg end length)
              (funcall (car functions) overlay nil beg end))
            (setq functions (cdr functions)))))
    (setq overlays (cdr overlays))))

(defun romacs--relocated (positions function)
  "Call FUNCTION, then return POSITIONS where markers at them moved.
A nil position stays nil.  These are GNU's preserve and start/end markers
\(PRESERVE_VALUE, PRESERVE_START_END): not insertion types, unchained
after the hooks and left to the collector after a non-local exit.  A VM2
unibyte buffer holds no markers: there POSITIONS stay as they are."
  (if (not enable-multibyte-characters)
      (progn (funcall function) positions)
    (let ((markers (mapcar (lambda (p) (if p (copy-marker p))) positions)))
      (funcall function)
      (mapcar (lambda (m) (if m (prog1 (marker-position m) (set-marker m nil))))
              markers))))

(defun romacs--before-change (buffer beg end modification-hooks
                                     &optional undoable preserve)
  "Run what GNU runs before changing BEG..END of BUFFER.
prepare_to_modify_buffer first calls `undo-auto--undoable-change' when
UNDOABLE.  Then, unless `inhibit-modification-hooks':
verify_interval_modification calls each function of each
MODIFICATION-HOOKS value (the characters' `modification-hooks', in
order) with BEG and END; a visited file is locked on its first change;
`select-active-regions' saves the region; and signal_before_change runs
`first-change-hook' if BUFFER is unmodified and `before-change-functions'
with BEG and END where the first-change hook moved them, then finds the
hooks of the overlays at BEG..END where those moved them and calls them,
all with `inhibit-modification-hooks' bound to t.  A non-local exit from
`before-change-functions' resets it to nil, as reset_var_on_error does.

The value is (PRESERVE . OVERLAYS): the position PRESERVE (nil for none)
where the modification, first-change, before-change and overlay hooks
moved it, as GNU's preserve_ptr, and the overlays' hooks found, as
\(PROP . OVERLAY) pairs, which signal_after_change runs again."
  (save-current-buffer
    (set-buffer buffer)
    (if undoable (undo-auto--undoable-change))
    (if inhibit-modification-hooks
        (list preserve)
      (if modification-hooks
          (setq preserve
                (car (romacs--relocated
                      (list preserve)
                      (lambda ()
                        (let ((tail modification-hooks))
                          (while tail
                            (let ((functions (car tail)))
                              (while functions
                                (funcall (car functions) beg end)
                                (setq functions (cdr functions))))
                            (setq tail (cdr tail)))))))))
      ;; An indirect buffer's edit locks its base's file (insdel.c:2160).
      (let ((base (or (buffer-base-buffer) (current-buffer))))
        (if (and (not (buffer-modified-p)) (fboundp 'lock-file)
                 (buffer-local-value 'buffer-file-truename base)
                 (buffer-local-value 'buffer-file-name base))
            (lock-file (buffer-local-value 'buffer-file-truename base))))
      (if (and mark-active (boundp 'saved-region-selection)
               (not saved-region-selection) (boundp 'region-extract-function)
               (marker-buffer (mark-marker))
               (if (eq select-active-regions 'only)
                   (eq (car-safe transient-mark-mode) 'only)
                 (and select-active-regions transient-mark-mode)))
          (setq saved-region-selection (funcall region-extract-function nil)))
      (let ((inhibit-modification-hooks t)
            (overlays nil))
        (if (and first-change-hook (not (buffer-modified-p)))
            (let ((moved (romacs--relocated
                          (list beg end preserve)
                          (lambda () (run-hooks 'first-change-hook)))))
              (setq beg (car moved) end (nth 1 moved) preserve (nth 2 moved))))
        (if before-change-functions
            (let ((moved (romacs--relocated
                          (list beg end preserve)
                          (lambda ()
                            (let ((done nil))
                              (unwind-protect
                                  (progn (run-hook-with-args
                                          'before-change-functions beg end)
                                         (setq done t))
                                (or done (setq before-change-functions nil))))))))
              (setq beg (car moved) end (nth 1 moved) preserve (nth 2 moved))))
        ;; report_overlay_modification finds the hooks only now.
        (setq overlays (romacs--overlay-change-hooks beg end))
        (if overlays
            (setq preserve
                  (car (romacs--relocated
                        (list preserve)
                        (lambda () (romacs--overlay-hooks overlays nil beg end))))))
        (cons preserve overlays)))))

(defun romacs--print-insert (stream text insert)
  "Insert TEXT printed to STREAM, a buffer or marker, as print_finish does.
INSERT, the `romacs--insert-printed' subr, prepares, inserts and signals
a nonempty TEXT from point less its length; an empty one still signals
its after-change at point.  Output to a marker inserts at it with point
there, then moves the marker past the text and point back, past the
text if it was at or after the marker."
  (save-current-buffer
    (set-buffer (if (markerp stream) (marker-buffer stream) stream))
    (let ((old (point))
          (start (if (markerp stream) (marker-position stream))))
      (if start (goto-char start))
      (if (equal text "")
          (romacs--after-change (current-buffer) (point) (point) 0)
        (funcall insert text))
      (when start
        (set-marker stream (point))
        (goto-char (if (>= old start) (+ old (- (point) start)) old))))))

(defun romacs--after-change (buffer beg end length
                                    &optional behind in-front overlays)
  "Run signal_after_change for BUFFER: `after-change-functions' with BEG,
END and LENGTH, the length of the text the change replaced, then the
OVERLAYS' hooks and, after an insertion, the text's insert hooks BEHIND
and IN-FRONT with BEG and END, all with `inhibit-modification-hooks'
bound to t.  A non-local exit from `after-change-functions' resets it to nil.
Under `combine-after-change-calls' the change is recorded for
`combine-after-change-execute' instead, when GNU defers it."
  (save-current-buffer
    (set-buffer buffer)
    (cond
     (inhibit-modification-hooks nil)
     ((romacs--after-change-deferred-p) (romacs--defer-after-change beg end length))
     (t
      (if romacs--combined-changes (romacs--combine-after-change-execute))
      (let ((inhibit-modification-hooks t))
        (if after-change-functions
            (let ((done nil))
              (unwind-protect
                  (progn (run-hook-with-args 'after-change-functions beg end length)
                         (setq done t))
                (or done (setq after-change-functions nil)))))
        (romacs--overlay-hooks overlays t beg end length)
        (when (= length 0)
          (let ((functions behind))
            (while functions
              (funcall (car functions) beg end)
              (setq functions (cdr functions))))
          (unless (eq in-front behind)
            (let ((functions in-front))
              (while functions
                (funcall (car functions) beg end)
                (setq functions (cdr functions)))))))))))

(defvar romacs--combined-changes nil
  "insdel.c combine_after_change_list: the deferred changes, newest first,
each (UNCHANGED-AT-BEGINNING UNCHANGED-AT-END GROWTH) in characters.")

(defvar romacs--combined-changes-buffer nil
  "insdel.c combine_after_change_buffer: the buffer of the deferred changes.")

(defun romacs--after-change-deferred-p ()
  "Whether signal_after_change defers the current buffer's change:
`combine-after-change-calls' is on, `before-change-functions' is nil or
only a local `syntax-ppss-flush-cache', and the buffer has no overlays
\(a VM2 unibyte buffer holds none)."
  (and combine-after-change-calls
       (or (null before-change-functions)
           (and (eq (car-safe before-change-functions) t)
                (null (default-value 'before-change-functions))
                (equal (cdr before-change-functions) '(syntax-ppss-flush-cache))))
       (or (not enable-multibyte-characters)
           (null (save-restriction (widen) (overlays-in (point-min) (point-max)))))))

(defun romacs--defer-after-change (beg end length)
  "Record the current buffer's change of BEG..END, which replaced LENGTH
characters, as signal_after_change does, after running the changes
deferred in another buffer.  GNU's end count is the buffer's size past
END less LENGTH, so a deletion's combined end falls before its start."
  (if (and romacs--combined-changes
           (not (eq (current-buffer) romacs--combined-changes-buffer)))
      (romacs--combine-after-change-execute))
  (setq romacs--combined-changes
        (cons (list (1- beg) (- (1+ (buffer-size)) (- end length))
                    (- end beg length))
              romacs--combined-changes))
  (setq romacs--combined-changes-buffer (current-buffer)))

(defun romacs--combine-after-change-execute ()
  "Fcombine_after_change_execute: run the after-change hooks once over the
deferred changes' combined region, in their buffer, and forget them."
  (let ((changes romacs--combined-changes)
        (buffer romacs--combined-changes-buffer))
    (setq romacs--combined-changes nil)
    (when (and changes (buffer-live-p buffer))
      (save-current-buffer
        (set-buffer buffer)
        (let* ((z (1+ (buffer-size))) (beg (1- z)) (end beg) (growth 0))
          (while changes
            (let ((change (car changes)))
              (setq growth (+ growth (nth 2 change)))
              (setq beg (min beg (nth 0 change)))
              (setq end (min end (nth 1 change))))
            (setq changes (cdr changes)))
          (let ((combine-after-change-calls combine-after-change-calls)
                (start (1+ beg)) (stop (- z end)))
            (romacs--after-change buffer start stop (- stop start growth))))))
    nil))

(defun romacs--self-insert (n c)
  "The rest of cmds.c `self-insert-command' once N and C are checked:
`undo-auto-amalgamate' for a single character, then a character C is
translated by `translation-table-for-input' and inserted N times, and a
non-character rings the bell (`ding'; VM2 has none without it)."
  (if (and (< n 2) (fboundp 'undo-auto-amalgamate)) (undo-auto-amalgamate))
  (if (not (characterp c))
      (if (fboundp 'ding) (ding))
    ;; translate_char: a character the table gives replaces C.
    (let* ((translated (if (char-table-p translation-table-for-input)
                           (aref translation-table-for-input c)))
           (character (if (characterp translated) translated c)))
      (if (eq (romacs--internal-self-insert character n) 2)
          (setq undo-auto--this-command-amalgamating nil))))
  nil)

(defun romacs--internal-self-insert (c n)
  "cmds.c internal_self_insert: insert N copies of C, overwriting under
`overwrite-mode', after expanding an abbrev, then auto-fill and
`post-self-insert-hook'.  The value is 2 when this did things that
call for an undo boundary, 1 when it might not have been simple."
  (let ((hairy (if (or before-change-functions after-change-functions) 1 0)))
    (if (and overwrite-mode (< (point) (point-max)))
        (error "VM2 overwrite-mode self-insert is not implemented"))
    (catch 'romacs--no-self-insert
      (if (and abbrev-mode (/= (char-syntax c) ?w) (not buffer-read-only)
               (> (point) (point-min)) (= (char-syntax (preceding-char)) ?w))
          (let* ((tick (buffer-modified-tick))
                 (sym (expand-abbrev)))
            (if (and (symbolp sym) sym (symbol-function sym)
                     (symbolp (symbol-function sym))
                     (get (symbol-function sym) 'no-self-insert))
                (throw 'romacs--no-self-insert (setq hairy 1)))
            (if (/= tick (buffer-modified-tick)) (setq hairy 2))))
      (if (> n 0) (insert-and-inherit (make-string n c)))
      (when (and (if (char-table-p auto-fill-chars)
                     (aref auto-fill-chars c)
                   (memq c '(?\s ?\n)))
                 ;; GNU's repetition loop counts N down to 0: only a
                 ;; single insertion fills.
                 auto-fill-function (= n 1))
        (if (= c ?\n) (forward-char -1))
        (let ((result (if (fboundp 'internal-auto-fill) (internal-auto-fill)
                        (funcall auto-fill-function))))
          (if (and (= c ?\n) (< (point) (point-max)) (eq (char-after) ?\n))
              (forward-char 1))
          (if result (setq hairy 2))))
      (run-hooks 'post-self-insert-hook)
      hairy)))

(defun romacs--casify-noncontiguous (function)
  "casefiddle.c casify_pnc_region: FUNCTION, a region casing command, on
each (BEG . END) of the bounds `region-extract-function' gives."
  (let ((bounds (funcall region-extract-function 'bounds)))
    (while (consp bounds)
      (let ((bound (car bounds)))
        (or (consp bound) (signal 'wrong-type-argument (list 'consp bound)))
        (funcall function (car bound) (cdr bound)))
      (setq bounds (cdr bounds)))
    (if bounds (signal 'wrong-type-argument (list 'listp bounds))))
  nil)

(defun romacs--replace-source (source)
  "SOURCE of `replace-region-contents' as editfns.c reads it: a string as
it is, else [BUFFER BEG END] for the accessible text of a buffer or the
validated region of a vector [BUFFER BEG END]."
  (cond
   ((stringp source) source)
   ((bufferp source)
    (save-current-buffer
      (set-buffer source)
      (vector source (point-min) (point-max))))
   ((vectorp source)
    (let ((end (aref source 2))
          (beg (aref source 1))
          (buffer (aref source 0)))
      (or (bufferp buffer) (signal 'wrong-type-argument (list 'bufferp buffer)))
      (save-current-buffer
        (set-buffer buffer)
        (let ((region (romacs--validate-region beg end)))
          (vector buffer (car region) (cdr region))))))
   (t (signal 'wrong-type-argument (list '(or string buffer vector) source)))))

(defun romacs--replace-region-contents (beg end source max-secs max-costs inherit)
  "The rest of editfns.c `replace-region-contents' once BEG..END is valid.
A function SOURCE is called with the buffer narrowed to BEG..END.  When
either text is empty, or MAX-SECS or MAX-COSTS is 0, the region is
replaced whole.  Otherwise the change runs GNU's comparison finds replace
just what differs, last first, each a replace_range with modification
hooks bound off, inside one undo boundary and one before- and
after-change for the whole region; a comparison past MAX-SECS replaces
the region whole and gives nil.  The pinned GNU passes INHERIT as
replace_range's adjust-match-data flag, not its inherit flag: the
replacement inherits no properties, and INHERIT moves the match data."
  (if (functionp source)
      (setq source (save-excursion
                     (save-restriction
                       (narrow-to-region beg end)
                       (funcall source)))))
  (setq source (romacs--replace-source source))
  (let ((size-a (- end beg))
        (size-b (if (stringp source) (length source)
                  (- (aref source 2) (aref source 1)))))
    (cond
     ((and (= size-a 0) (= size-b 0)) t)
     ((or (= size-a 0) (= size-b 0) (eq max-secs 0) (eq max-costs 0))
      (romacs--replace-range beg end source inherit)
      t)
     (t
      (if (and (vectorp source) (eq (aref source 0) (current-buffer)))
          (error "Cannot replace a buffer with itself"))
      (let* ((too-expensive
              (cond ((null max-costs) 1000000)
                    ((integerp max-costs)
                     (max 0 (min max-costs most-positive-fixnum)))
                    (t (signal 'wrong-type-argument (list 'integerp max-costs)))))
             (runs (romacs--replace-runs beg end source too-expensive
                                         (and max-secs (float-time max-secs)))))
        (if (eq runs t)
            (progn (romacs--replace-range beg end source inherit) nil)
          (undo-boundary)
          (let ((before (if (not inhibit-modification-hooks)
                            (romacs--prepare-change beg end))))
            (let ((inhibit-modification-hooks t))
              (while runs
                (let ((run (car runs)))
                  (romacs--replace-range
                   (car run) (nth 1 run)
                   (if (stringp source)
                       (substring source (nth 2 run) (nth 3 run))
                     (vector (aref source 0) (nth 2 run) (nth 3 run)))
                   inherit))
                (setq runs (cdr runs))))
            (when before
              (romacs--after-change (current-buffer) beg (+ beg size-b) size-a
                                    nil nil (cdr before))
              (if (and (not (buffer-modified-p)) (stringp buffer-file-truename)
                       (fboundp 'unlock-file))
                  (unlock-file buffer-file-truename))))
          t))))))
)
