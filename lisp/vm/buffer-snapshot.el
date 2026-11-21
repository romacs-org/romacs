;;; buffer-snapshot.el --- Frozen snapshots of a buffer, for worker isolates  -*- lexical-binding: t -*-

;; SPDX-License-Identifier: GPL-3.0-or-later
;; Maintainer: Thanos Apollo <public@thanosapollo.org>

;;; Commentary:

;; The buffer snapshots worker isolates compute from (vm-concurrency
;; design §3.6, §4.4): frozen values (`freeze'), so they cross to a
;; worker as any job argument does (lisp/vm/isolate.el).  Not in GNU
;; (ledger/divergences VM2-ISOLATE).

;;; Code:

;; A snapshot (design §3.6, §4.4) is a frozen record of a buffer's
;; accessible text and what a worker needs to treat it as that buffer:
;; [buffer-snapshot NAME TEXT BEGIN TICK CHARS-TICK POINT FILE DIRECTORY
;; MODE LOCALS GENERATION].  The text is copied (raw bytes included: it
;; is the buffer's own string); a worker gets a buffer of its own over it
;; with `with-snapshot-buffer'.  Positions in a snapshot are 1-based from
;; its first character; the owner's position of that character is
;; `snapshot-begin'.

(defvar romacs--snapshot-generations 0 "The last snapshot generation this VM issued.")

(defvar-local buffer-snapshot-generation nil
  "The generation of a buffer's snapshots: results from a snapshot apply
only while it is the same.  Not permanent, so a major mode's
`kill-all-local-variables' ends it; `buffer-snapshot-invalidate' too.")

(defun buffer-snapshot-invalidate (&optional buffer)
  "Make results computed from BUFFER's earlier snapshots stale."
  (with-current-buffer (or buffer (current-buffer))
    (setq buffer-snapshot-generation nil)))

(defun romacs--snapshot-generation ()
  "The current buffer's snapshot generation, a new one if it has none."
  (or buffer-snapshot-generation
      (setq buffer-snapshot-generation (setq romacs--snapshot-generations (1+ romacs--snapshot-generations)))))

(defun buffer-snapshot (&optional buffer &rest keys)
  "A frozen snapshot of BUFFER's accessible text.  KEYS: :properties t
also copies the text properties; :locals, a list of variables, records
BUFFER's values of those that are buffer-local and freezable."
  (with-current-buffer (or buffer (current-buffer))
    (let (locals)
      (dolist (variable (plist-get keys :locals))
        (when (local-variable-p variable)
          (let ((value (symbol-value variable)))
            (when (condition-case nil (progn (freeze value) t) (error nil))
              (push (cons variable value) locals)))))
      (freeze (record 'buffer-snapshot (buffer-name)
                      (if (plist-get keys :properties)
                          (buffer-substring (point-min) (point-max))
                        (buffer-substring-no-properties (point-min) (point-max)))
                      (point-min) (buffer-modified-tick) (buffer-chars-modified-tick)
                      (- (point) (point-min) -1) buffer-file-name default-directory
                      major-mode (nreverse locals) (romacs--snapshot-generation))))))

(defun buffer-snapshot-p (object) "Non-nil for a buffer snapshot." (eq (type-of object) 'buffer-snapshot))
(defun snapshot-buffer-name (snapshot) "The name of SNAPSHOT's buffer." (aref snapshot 1))
(defun snapshot-string (snapshot) "SNAPSHOT's text, frozen." (aref snapshot 2))
(defun snapshot-begin (snapshot) "The owner's position of SNAPSHOT's first character." (aref snapshot 3))
(defun snapshot-tick (snapshot) "The buffer's `buffer-modified-tick' at SNAPSHOT." (aref snapshot 4))
(defun snapshot-chars-tick (snapshot) "The buffer's `buffer-chars-modified-tick' at SNAPSHOT." (aref snapshot 5))
(defun snapshot-point (snapshot) "Point at SNAPSHOT, in its own positions." (aref snapshot 6))
(defun snapshot-file-name (snapshot) "The buffer's `buffer-file-name' at SNAPSHOT." (aref snapshot 7))
(defun snapshot-mode (snapshot) "The buffer's `major-mode' at SNAPSHOT." (aref snapshot 9))
(defun snapshot-locals (snapshot) "The buffer-local values SNAPSHOT recorded, an alist." (aref snapshot 10))
(defun snapshot-generation (snapshot) "The buffer's `buffer-snapshot-generation' at SNAPSHOT." (aref snapshot 11))
(defun snapshot-local (snapshot variable)
  "SNAPSHOT's value of VARIABLE, or nil when it recorded none."
  (cdr (assq variable (snapshot-locals snapshot))))

(defun buffer-snapshot-current-p (snapshot &optional buffer)
  "Non-nil while results computed from SNAPSHOT still apply to BUFFER: its
text (`buffer-chars-modified-tick'), major mode, generation and the
recorded locals are as SNAPSHOT has them.  `buffer-modified-tick' is not
compared: it counts text property changes too, which the owner's own
redisplay fontification makes without moving any position."
  (let ((buffer (or buffer (get-buffer (snapshot-buffer-name snapshot)))))
    (and (buffer-live-p buffer)
         (with-current-buffer buffer
           (and (= (buffer-chars-modified-tick) (snapshot-chars-tick snapshot))
                (eq major-mode (snapshot-mode snapshot))
                (eql buffer-snapshot-generation (snapshot-generation snapshot))
                (let ((same t))
                  (dolist (local (snapshot-locals snapshot) same)
                    (unless (and (local-variable-p (car local))
                                 (equal (symbol-value (car local)) (cdr local)))
                      (setq same nil)))))))))

(defun romacs--with-snapshot-buffer (snapshot no-hooks body)
  "Call BODY in a buffer of this VM's own over SNAPSHOT's text; see
`with-snapshot-buffer'."
  (let ((buffer (generate-new-buffer (format "*snapshot %s*" (snapshot-buffer-name snapshot)) t)))
    (unwind-protect
        (with-current-buffer buffer
          (insert (snapshot-string snapshot))
          (set-buffer-modified-p nil)
          (goto-char (snapshot-point snapshot))
          (setq default-directory (aref snapshot 8))
          (if no-hooks
              (let ((delay-mode-hooks t)) (funcall (snapshot-mode snapshot)))
            (funcall (snapshot-mode snapshot)))
          ;; After the mode, as `normal-mode' applies file-local variables.
          (dolist (local (snapshot-locals snapshot))
            (set (make-local-variable (car local)) (thaw (cdr local))))
          (setq buffer-read-only t)
          (let ((tick (buffer-chars-modified-tick)))
            (prog1 (funcall body)
              (unless (= tick (buffer-chars-modified-tick))
                (signal 'error (list (format "Snapshot text changed in %s" (buffer-name))))))))
      (kill-buffer buffer))))

(defmacro with-snapshot-buffer (snapshot &rest body)
  "Run BODY in a temporary buffer of this VM's own whose text is SNAPSHOT's.
The buffer is in SNAPSHOT's major mode (its hooks run unless BODY starts
with `:no-hooks t'), has SNAPSHOT's recorded locals, point and directory,
and is read-only.  Its text properties (under `inhibit-read-only', as Font
Lock and `with-silent-modifications' bind it), overlays, markers, locals
and the mode's caches are this VM's to change; its text is not: a change
even under `inhibit-read-only' signals once BODY returns, since results
are positions in SNAPSHOT.  It visits no file."
  (declare (indent 1) (debug t))
  (let ((no-hooks (and (eq (car body) :no-hooks) (prog1 (nth 1 body) (setq body (nthcdr 2 body))))))
    `(romacs--with-snapshot-buffer ,snapshot ,no-hooks (lambda () ,@body))))

;;; buffer-snapshot.el ends here
