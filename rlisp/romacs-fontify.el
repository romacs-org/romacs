;;; romacs-fontify.el --- Font Lock of large buffers in a worker isolate  -*- lexical-binding: t -*-

;; SPDX-License-Identifier: GPL-3.0-or-later

;;; Commentary:

;; A large visited buffer is fontified ahead of redisplay in a worker
;; isolate (rlisp/vm/isolate.el) from a snapshot of its text: the worker
;; runs the buffer's major mode on its own copy, turns on Font Lock and
;; calls `jit-lock-function' over it chunk by chunk, as this session's
;; redisplay would, starting where the window's text ends.  It
;; streams the text properties that left, block by block, as frozen
;; messages.  When redisplay here asks Font Lock for a region the worker
;; has covered, `font-lock-fontify-region' puts the worker's properties
;; on it instead of fontifying it again, which costs a fraction of a
;; millisecond where fontifying costs a hundred or more.  Regions the
;; worker has not reached are fontified here as usual, so nothing ever
;; waits for the worker.  The worker is retired once its job is done:
;; fontifying a large buffer leaves it hundreds of megabytes of private
;; memory, and a new one starts in a fraction of a second.
;;
;; Results apply only while the snapshot is current
;; (`buffer-snapshot-current-p': the text, the mode, the generation) and
;; the worker's Font Lock setup is the session's (a fingerprint of the
;; keywords and region functions, so a user's mode hooks that add
;; keywords make the buffer fontify here); an edit drops them and the
;; buffer is snapshotted again once the session idles.  Property values
;; that are not plain data (keymaps, functions, nested lists) are not
;; sent: such a run stays a hole the session fontifies itself.
;; DIVERGENCES.md VM2-FONTIFY-ISOLATE.

;;; Code:

(defgroup romacs-fontify nil
  "Font Lock of large buffers in a worker isolate."
  :group 'font-lock)

(defcustom romacs-fontify-isolate t
  "Whether large buffers are fontified ahead of redisplay in a worker.
Only when this session restored from a world, so a worker adopts it."
  :type 'boolean)

(defcustom romacs-fontify-isolate-modes
  '((emacs-lisp-mode) (lisp-data-mode) (lisp-interaction-mode)
    (c-mode cc-mode) (c++-mode cc-mode))
  "Major modes fontified in a worker, each with the features its worker
loads before its first job (a worker whose job loads more is retired)."
  :type '(alist :key-type symbol :value-type (repeat symbol)))

(defcustom romacs-fontify-isolate-size 65536
  "Buffers with fewer characters than this are fontified in the session only."
  :type 'natnum)

(defcustom romacs-fontify-isolate-delay 2.0
  "Idle seconds after an edit before a buffer is snapshotted again."
  :type 'number)

(defcustom romacs-fontify-isolate-variables
  '(jit-lock-chunk-size font-lock-maximum-decoration
    c-default-style c-doc-comment-style c-font-lock-extra-types c++-font-lock-extra-types)
  "Variables whose session values the worker fontifies with."
  :type '(repeat symbol))

(defconst romacs-fontify--block 8192
  "Characters per block of results: what a message from the worker covers.")

(defvar romacs-fontify--tokens 0 "The last job token this session issued.")
(defvar romacs-fontify--jobs (make-hash-table :test 'eql) "Job tokens to their buffers.")
(defvar romacs-fontify-last-error nil "The last worker error, (BUFFER-NAME . ERROR).")

(defvar-local romacs-fontify--state nil
  "This buffer's worker job: [TOKEN SNAPSHOT FUTURE FINGERPRINT BLOCKS DONE
SETUP], FUTURE nil once resolved, BLOCKS a vector of nil or (PLISTS RUNS
HOLES NAMES) per block, SETUP `romacs-fontify--setup' when the
fingerprint was last found to match.")
(defvar-local romacs-fontify--refused nil
  "Non-nil once the worker's Font Lock setup differed from this buffer's.")
(defvar-local romacs-fontify--timer nil "The idle timer that snapshots this buffer again.")

;;;; The job, run in the worker

;; A lambda form, not a symbol: the worker's world does not hold this
;; module (DIVERGENCES.md VM2-ISOLATE-GLOBALS).

(defconst romacs-fontify--fingerprint
  '(lambda ()
     (let ((print-length nil) (print-level nil) (print-circle t)
           (keywords (if (eq (car-safe font-lock-keywords) t) (nth 1 font-lock-keywords) font-lock-keywords))
           (syntax (lambda (table)
                     (and table (let ((c 0) entries)
                                  (while (< c 256) (push (aref table c) entries) (setq c (1+ c)))
                                  entries)))))
       (secure-hash 'sha1 (prin1-to-string
                           (list keywords font-lock-keywords-case-fold-search font-lock-keywords-only
                                 font-lock-fontify-region-function font-lock-extend-region-functions
                                 font-lock-extra-managed-props syntax-propertize-function
                                 font-lock-syntactic-face-function jit-lock-chunk-size
                                 (funcall syntax (syntax-table)) (funcall syntax font-lock-syntax-table))))))
  "A digest of what decides a buffer's Font Lock results: its keywords, as
written, the functions that find and fontify regions and the syntax of
the first 256 characters.  A form, so the worker computes it as the
session does.")

(defun romacs-fontify--syntax (table)
  "TABLE's entries for the characters 0..255, a list; nil without TABLE."
  (and table (let ((c 255) entries)
               (while (>= c 0) (push (aref table c) entries) (setq c (1- c)))
               entries)))

(defun romacs-fontify--setup ()
  "The current buffer's Font Lock setup as the objects the fingerprint
digests, and the syntax entries it digests (a syntax table changes in
place): compared before results are used, so a change made since
(`font-lock-add-keywords', `modify-syntax-entry') is noticed."
  (list font-lock-keywords font-lock-keywords-case-fold-search font-lock-keywords-only
        font-lock-fontify-region-function font-lock-extend-region-functions
        font-lock-extra-managed-props syntax-propertize-function
        font-lock-syntactic-face-function jit-lock-chunk-size
        (romacs-fontify--syntax (syntax-table)) (romacs-fontify--syntax font-lock-syntax-table)))

(defconst romacs-fontify--job
  `(lambda (snapshot token block start)
     (with-snapshot-buffer snapshot
       ;; Font Lock through jit-lock, as the session's redisplay runs it;
       ;; a batch VM would refuse to turn it on.
       (let ((noninteractive nil)) (font-lock-mode 1))
       (unless jit-lock-mode (error "No jit-lock in %s" major-mode))
       (let* ((limit (point-max))
              (count (/ (+ (- limit 1) block -1) block))
              (fingerprint (funcall ,romacs-fontify--fingerprint))
              (plain (lambda (value)
                       (or (symbolp value) (numberp value)
                           (and (stringp value) (null (text-properties-at 0 value))
                                (null (next-property-change 0 value))))))
              (shippable (lambda (plist)
                           (let ((ok t))
                             (while (and ok plist)
                               (let ((value (nth 1 plist)) (n 0))
                                 (if (consp value)
                                     (while (and ok value)
                                       (if (and (consp value) (< n 32))
                                           (setq ok (funcall plain (car value)) value (cdr value) n (1+ n))
                                         (setq ok (funcall plain value) value nil)))
                                   (setq ok (funcall plain value))))
                               (setq plist (nthcdr 2 plist)))
                             ok)))
              ;; Redisplay's walk: fontify the first unfontified chunk, again.
              (scan (lambda (from to)
                      (let ((pos from))
                        (while (and pos (< pos to))
                          (setq pos (text-property-not-all pos to 'fontified t))
                          (when pos (jit-lock-function pos))))))
              (emit (lambda (i)
                      (let ((beg (1+ (* i block))) (table (make-hash-table :test 'equal))
                            end pos plists runs holes)
                        (setq end (min limit (+ beg block)) pos beg)
                        (while (< pos end)
                          (let ((next (next-property-change pos nil end))
                                (props (text-properties-at pos)) kept)
                            (while props
                              (unless (eq (car props) 'fontified)
                                (push (car props) kept) (push (nth 1 props) kept))
                              (setq props (nthcdr 2 props)))
                            (setq kept (nreverse kept))
                            (when kept
                              (if (funcall shippable kept)
                                  (let ((index (gethash kept table)))
                                    (unless index
                                      (setq index (hash-table-count table))
                                      (puthash kept index table)
                                      (push kept plists))
                                    (push pos runs) (push next runs) (push index runs))
                                (push (cons pos next) holes)))
                            (setq pos next)))
                        (isolate-send 'parent (list 'romacs-fontify token fingerprint beg end
                                                    (vconcat (nreverse plists)) (vconcat (nreverse runs))
                                                    (nreverse holes))))))
              ;; A block is final once the chunk after it is fontified: a
              ;; chunk's fontification reaches back to its line's start.
              (finish (lambda (i)
                        (funcall scan (1+ (* i block)) (min limit (+ 1 (* (1+ i) block) jit-lock-chunk-size)))
                        (funcall emit i)))
              (first (min (max 0 (/ (- start 1) block)) (max 0 (1- count))))
              (i first))
         (while (< i count) (funcall finish i) (setq i (1+ i)))
         (setq i 0)
         (while (< i first) (funcall finish i) (setq i (1+ i)))
         ;; The block where the walk began saw the wrap last.
         (when (> first 0) (funcall emit first))
         ;; What fontification found later unfontifies text it already
         ;; fontified (cc-mode's new types): redisplay would come back to it.
         (let ((round 0) again)
           (while (and (< round 3)
                       (progn
                         (setq again nil i 0)
                         (while (< i count)
                           (when (text-property-not-all (1+ (* i block)) (min limit (+ 1 (* (1+ i) block))) 'fontified t)
                             (push i again))
                           (setq i (1+ i)))
                         again))
             (dolist (j (nreverse again)) (funcall finish j))
             (setq round (1+ round))))
         (list token fingerprint count)))))

(defun romacs-fontify-fingerprint ()
  "A digest of what decides the current buffer's Font Lock results."
  (funcall (eval romacs-fontify--fingerprint t)))

;;;; Starting and dropping

(defun romacs-fontify--wanted-p ()
  "Whether the current buffer should be fontified in a worker now."
  (and romacs-fontify-isolate
       (fboundp 'isolate-spawn)
       (eq isolate-boot 'world) isolate-world
       (not romacs-fontify--state) (not romacs-fontify--refused)
       (not (buffer-base-buffer))
       font-lock-mode (bound-and-true-p jit-lock-mode)
       (assq major-mode romacs-fontify-isolate-modes)
       (>= (buffer-size) romacs-fontify-isolate-size)))

(defun romacs-fontify--bindings ()
  "`romacs-fontify-isolate-variables' as the session has them, the freezable ones."
  (let (bindings)
    (dolist (variable romacs-fontify-isolate-variables (nreverse bindings))
      (when (boundp variable)
        (let ((value (default-value variable)))
          (when (condition-case nil (progn (freeze value) t) (error nil))
            (push (cons variable value) bindings)))))))

(defun romacs-fontify--start (&optional buffer)
  "Fontify BUFFER in a worker, if it is one that wants it; the job's
state, or nil."
  (with-current-buffer (or buffer (current-buffer))
    (setq romacs-fontify--timer nil)
    (when (romacs-fontify--wanted-p)
      (let* ((require (cdr (assq major-mode romacs-fontify-isolate-modes)))
             (worker (isolate-pooled-worker require))
             (window (get-buffer-window (current-buffer) t))
             ;; Redisplay has fontified what the window shows: the walk
             ;; begins where it ends.
             (start (if window (window-end window) (point)))
             (token (setq romacs-fontify--tokens (1+ romacs-fontify--tokens)))
             (snapshot (and worker (save-restriction (widen) (buffer-snapshot))))
             (future (and worker
                          (condition-case nil
                              (isolate-spawn romacs-fontify--job snapshot token romacs-fontify--block start
                                             :isolate worker :require require
                                             :bindings (romacs-fontify--bindings))
                            (isolate-error nil)))))
        (when future
          (puthash token (current-buffer) romacs-fontify--jobs)
          (add-hook 'after-change-functions #'romacs-fontify--changed nil t)
          (add-hook 'kill-buffer-hook #'romacs-fontify--drop nil t)
          (let ((buffer (current-buffer)))
            (isolate-on-done future (lambda (future) (romacs-fontify--done buffer token future))))
          (setq romacs-fontify--state (romacs-fontify--new-state token snapshot future)))))))

(defun romacs-fontify--new-state (token snapshot future)
  "The current buffer's state for job TOKEN of SNAPSHOT, FUTURE: no blocks yet."
  (vector token snapshot future (romacs-fontify-fingerprint)
          (make-vector (/ (+ (buffer-size) romacs-fontify--block -1) romacs-fontify--block) nil)
          nil (romacs-fontify--setup)))

(defun romacs-fontify--drop (&optional again)
  "Forget the current buffer's worker results and stop its job; with
AGAIN, snapshot it again once the session has idled
`romacs-fontify-isolate-delay' seconds."
  (let ((state romacs-fontify--state))
    (when state
      (setq romacs-fontify--state nil)
      (remhash (aref state 0) romacs-fontify--jobs)
      ;; Not here: this runs inside isolate delivery and redisplay's
      ;; fontification, and a cancel pumps the isolates.
      (when (and (aref state 2) (not (isolate-future-done-p (aref state 2))))
        (run-at-time 0 nil #'romacs-fontify--cancel (aref state 2)))))
  (when (timerp romacs-fontify--timer) (cancel-timer romacs-fontify--timer))
  (setq romacs-fontify--timer
        (and again (run-with-idle-timer romacs-fontify-isolate-delay nil #'romacs-fontify--start (current-buffer)))))

(defun romacs-fontify--cancel (future)
  "Stop FUTURE's job, unless it has finished."
  (unless (isolate-future-done-p future)
    (condition-case nil (isolate-future-cancel future) (error nil))))

(defun romacs-fontify--refuse ()
  "Leave the current buffer to the session: its Font Lock setup is not the
worker's."
  (romacs-fontify--drop)
  (setq romacs-fontify--refused t))

(defun romacs-fontify--setup-current-p (state)
  "Whether the current buffer's Font Lock setup is still the one STATE's
fingerprint matched: the same objects and syntax, or ones with the same
digest."
  (or (let ((then (aref state 6)) (now (romacs-fontify--setup)) (same t))
        ;; The objects by identity, the two syntax lists by content.
        (while (and same (nthcdr 2 then)) (setq same (eq (pop then) (pop now))))
        (and same (equal then now)))
      (and (equal (romacs-fontify-fingerprint) (aref state 3))
           (progn (aset state 6 (romacs-fontify--setup)) t))))

(defun romacs-fontify--changed (&rest _)
  "After a change: drop results whose text is gone, and snapshot again later."
  (let ((state romacs-fontify--state))
    (when (and state (/= (buffer-chars-modified-tick) (snapshot-chars-tick (aref state 1))))
      (romacs-fontify--drop t))))

(defun romacs-fontify--done (buffer token future)
  "FUTURE, BUFFER's job TOKEN, has resolved: note it, or its error."
  (when (buffer-live-p buffer)
    (with-current-buffer buffer
      (let ((state romacs-fontify--state))
        (when (and state (eql (aref state 0) token))
          ;; The future holds the job, the snapshot's second copy.
          (aset state 2 nil)
          (condition-case err
              (progn (isolate-await future 0) (aset state 5 t))
            (error (setq romacs-fontify-last-error (cons (buffer-name) err))))))))
  ;; A worker that fontified a large buffer has collected over its world,
  ;; whose pages are then its own: hundreds of megabytes it would keep
  ;; while pooled.  Starting another costs a fraction of a second.
  (run-at-time 0 nil #'romacs-fontify--retire (isolate-future-isolate future)))

(defun romacs-fontify--retire (worker)
  "Retire WORKER unless it has been given other work meanwhile."
  (when (and (isolate-live-p worker) (zerop (isolate-pending worker)))
    (isolate-retire worker)))

;;;; Receiving and applying

(defun romacs-fontify--receive (_sender message)
  "Keep a block of results MESSAGE from a worker, if its buffer still wants it."
  (when (eq (car-safe message) 'romacs-fontify)
    (let ((buffer (gethash (nth 1 message) romacs-fontify--jobs)))
      (when (buffer-live-p buffer)
        (with-current-buffer buffer
          (let ((state romacs-fontify--state))
            (cond ((not (and state (eql (aref state 0) (nth 1 message)))))
                  ((not (buffer-snapshot-current-p (aref state 1) buffer))
                   (romacs-fontify--drop t))
                  ((not (and (equal (nth 2 message) (aref state 3)) (romacs-fontify--setup-current-p state)))
                   (romacs-fontify--refuse))
                  (t (romacs-fontify--keep state message)))))))))

(defun romacs-fontify--keep (state message)
  "Keep MESSAGE's block in STATE; a block sent again makes redisplay apply
it again where it already did."
  (let* ((offset (1- (snapshot-begin (aref state 1))))
         (beg (+ offset (nth 3 message))) (end (+ offset (nth 4 message)))
         (index (/ (- (nth 3 message) 1) romacs-fontify--block))
         (blocks (aref state 4))
         ;; Private copies: a kept region would hold one of the VM's
         ;; region ids for as long as the buffer keeps its results.
         (plists (thaw (nth 5 message)))
         (runs (thaw (nth 6 message)))
         (names (append '(face font-lock-multiline) font-lock-extra-managed-props)))
    (when (< index (length blocks))
      (dotimes (i (length plists))
        (let ((plist (aref plists i)))
          (while plist
            (unless (memq (car plist) names) (push (car plist) names))
            (setq plist (nthcdr 2 plist)))))
      (save-restriction
        (widen)
        (when (and (aref blocks index) (text-property-any beg end 'fontified t))
          (with-silent-modifications (put-text-property beg end 'fontified nil))))
      (aset blocks index (list plists runs (thaw (nth 7 message)) names)))))

(defun romacs-fontify--covering (beg end)
  "The blocks of results covering BEG..END in the current buffer, in order,
each (BLOCK-BEG PLISTS RUNS HOLES NAMES), or nil unless all of it is
covered and current."
  (let ((state romacs-fontify--state))
    (when (and state (< beg end) (buffer-snapshot-current-p (aref state 1) (current-buffer))
               (or (romacs-fontify--setup-current-p state) (progn (romacs-fontify--refuse) nil)))
      (let* ((offset (1- (snapshot-begin (aref state 1))))
             (from (- beg offset)) (to (- end offset))
             (blocks (aref state 4))
             (first (/ (- from 1) romacs-fontify--block))
             (last (/ (- to 2) romacs-fontify--block))
             (i first) (covered (>= from 1)) result)
        (while (and covered (<= i last))
          (let ((block (and (< i (length blocks)) (aref blocks i))))
            (if (null block)
                (setq covered nil)
              (dolist (hole (nth 2 block))
                (when (and (< (car hole) to) (> (cdr hole) from)) (setq covered nil)))
              (push (cons (+ offset 1 (* i romacs-fontify--block)) block) result)))
          (setq i (1+ i)))
        (and covered (nreverse result))))))

(defun romacs-fontify--apply (beg end blocks)
  "Put BLOCKS' properties on BEG..END: each block's properties replace the
ones it names there."
  (let ((offset (1- (snapshot-begin (aref romacs-fontify--state 1)))))
    (with-silent-modifications
      (dolist (block blocks)
        (let* ((block-beg (car block))
               (from (max beg block-beg))
               (to (min end (+ block-beg romacs-fontify--block)))
               (plists (nth 1 block)) (runs (nth 2 block))
               (low 0) (high (/ (length runs) 3)))
          (remove-list-of-text-properties from to (nth 4 block))
          ;; The first run ending after FROM.
          (while (< low high)
            (let ((mid (/ (+ low high) 2)))
              (if (<= (+ offset (aref runs (+ (* 3 mid) 1))) from) (setq low (1+ mid)) (setq high mid))))
          (let ((i (* 3 low)))
            (while (and (< i (length runs)) (< (+ offset (aref runs i)) to))
              (add-text-properties (max from (+ offset (aref runs i))) (min to (+ offset (aref runs (1+ i))))
                                   (aref plists (aref runs (+ i 2))))
              (setq i (+ i 3)))))))))

(defun romacs-fontify--region (original beg end &optional loudly)
  "Around `font-lock-fontify-region': the worker's results where they
cover BEG..END, else ORIGINAL with LOUDLY."
  (let ((blocks (and romacs-fontify--state (romacs-fontify--covering beg (min end (point-max))))))
    (if blocks
        (progn (romacs-fontify--apply beg (min end (point-max)) blocks) nil)
      (funcall original beg end loudly))))

(defun romacs-fontify--visited ()
  "On visiting a file: fontify it in a worker once the session idles."
  (when (romacs-fontify--wanted-p)
    (when (timerp romacs-fontify--timer) (cancel-timer romacs-fontify--timer))
    (setq romacs-fontify--timer (run-with-idle-timer 0.2 nil #'romacs-fontify--start (current-buffer)))))

(when (fboundp 'advice-add)
  (advice-add 'font-lock-fontify-region :around #'romacs-fontify--region)
  (add-hook 'isolate-message-functions #'romacs-fontify--receive)
  (add-hook 'find-file-hook #'romacs-fontify--visited))

(provide 'romacs-fontify)
;;; romacs-fontify.el ends here
