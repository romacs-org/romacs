;;; romacs-companion.el --- Trusted-local companion -*- lexical-binding: t; -*-

;;; Commentary:
;; Support cooperative owner-controlled companion editing, not a sandbox.
;; All edits are headless.  No buffer/window selection, display, waits or I/O.
;; Modification callbacks are suppressed for this dedicated scratch edit;
;; native read-only checks and ordinary undo remain enabled.

;;; Code:

(defvar romacs-companion--incarnation nil)
(defvar romacs-companion--claims (make-hash-table :test #'equal))
(defvar romacs-companion--serial 0)
(defvar-local romacs-companion--claim nil)

(defun romacs-companion-identity ()
  "Return this editor's PID and process-local incarnation.
The incarnation is an accidental-target fence, not an authentication secret."
  (unless romacs-companion--incarnation
    (setq romacs-companion--incarnation
          (format "%s-%s-%s" (emacs-pid) (float-time) (random))))
  (list (emacs-pid) (copy-sequence romacs-companion--incarnation)))

(defun romacs-companion--check-instance (instance)
  "Refuse unless INSTANCE matches this editor's current incarnation."
  (unless (equal instance (romacs-companion-identity))
    (user-error "Companion editor instance mismatch")))

(defun romacs-companion--retire ()
  "Irreversibly retire the current buffer's constructor claim."
  (when romacs-companion--claim
    (remhash romacs-companion--claim romacs-companion--claims)
    (setq romacs-companion--claim nil)))

(defun romacs-companion--buffer (instance handle)
  "Return the owned buffer for INSTANCE and HANDLE, or refuse."
  (romacs-companion--check-instance instance)
  (let ((buffer (gethash handle romacs-companion--claims)))
    (unless (and (buffer-live-p buffer)
                 (with-current-buffer buffer
                   (and (equal handle romacs-companion--claim)
                        (not buffer-file-name) (not (buffer-base-buffer)))))
      (user-error "Companion claim is absent or retired"))
    buffer))

(defun romacs-companion-claim (instance)
  "Create a unique non-file companion for INSTANCE without displaying it.
Return metadata with :handle, :buffer and :tick.  Never adopt a namesake."
  (romacs-companion--check-instance instance)
  (setq romacs-companion--serial (1+ romacs-companion--serial))
  (let* ((buffer (generate-new-buffer "*Romacs companion*"))
         (handle (format "%s:%s" romacs-companion--incarnation
                         romacs-companion--serial)))
    (with-current-buffer buffer
      (buffer-enable-undo)
      (setq romacs-companion--claim handle)
      (add-hook 'change-major-mode-hook #'romacs-companion--retire nil t)
      (add-hook 'after-set-visited-file-name-hook
                #'romacs-companion--retire nil t)
      (add-hook 'kill-buffer-hook #'romacs-companion--retire nil t))
    (puthash handle buffer romacs-companion--claims)
    (romacs-companion-state instance handle)))

(defun romacs-companion-state (instance handle)
  "Return allowlisted metadata for INSTANCE and HANDLE, without buffer text."
  (with-current-buffer (romacs-companion--buffer instance handle)
    (list :handle (copy-sequence handle) :buffer (buffer-name)
          :tick (buffer-modified-tick))))

(defun romacs-companion-edit (instance handle tick start end text)
  "Replace a region in the exact companion and return updated metadata.
INSTANCE and HANDLE must still own the non-file buffer.  TICK must equal
`buffer-modified-tick'.  START and END are widened character positions.
Insert TEXT literally, without properties.  Refusal never retries; an error
or quit rolls back the text transaction.  Preserve point and narrowing and
ordinary undo.  Suppress modification callbacks only inside this edit."
  (with-current-buffer (romacs-companion--buffer instance handle)
    (unless (and (integerp tick) (= tick (buffer-modified-tick)))
      (user-error "Companion modification tick is stale"))
    (when (or buffer-read-only (eq buffer-undo-list t))
      (user-error "Companion must be writable with undo enabled"))
    (save-excursion
      (save-restriction
        (widen)
        (unless (and (integerp start) (integerp end) (stringp text)
                     (<= (point-min) start end (point-max)))
          (user-error "Invalid companion edit region"))
        (let ((literal (substring-no-properties text))
              (inhibit-modification-hooks t))
          (undo-boundary)
          (atomic-change-group
            (delete-region start end)
            (goto-char start)
            (insert literal))
          (undo-boundary))))
    (romacs-companion-state instance handle)))

(defun romacs-companion-retire (instance handle)
  "Retire INSTANCE's HANDLE without killing or erasing its buffer."
  (with-current-buffer (romacs-companion--buffer instance handle)
    (romacs-companion--retire))
  t)

(provide 'romacs-companion)
;;; romacs-companion.el ends here
