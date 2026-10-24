;;; romacs-mcp-companion.el --- Cooperative MCP tools -*- lexical-binding: t; -*-

;; SPDX-License-Identifier: GPL-3.0-or-later

;;; Commentary:
;; Optional tool registration atop the existing companion claim and receipt
;; APIs.  No second ownership system, file editing, display or learner access.

;;; Code:
(require 'romacs-mcp)
(require 'romacs-companion-receipts)

(defun romacs-mcp--companion-instance (arguments)
  "Fence ARGUMENTS, then return the companion's existing native identity."
  (romacs-mcp--instance arguments)
  (romacs-companion-identity))

(defun romacs-mcp--plain-object (plist)
  "Convert the helper's plain PLIST to a JSON object without private payloads."
  (apply #'romacs-mcp--object
         (cl-loop for (key value) on plist by #'cddr
                  append (list (substring (symbol-name key) 1)
                               (cond ((keywordp (car-safe value)) (romacs-mcp--plain-object value))
                                     ((symbolp value) (symbol-name value))
                                     (t value))))))

(defun romacs-mcp--companion-claim (arguments)
  "Claim an undisplayed scratch companion for fenced ARGUMENTS."
  (romacs-mcp--plain-object
   (romacs-companion-claim (romacs-mcp--companion-instance arguments))))

(defun romacs-mcp--companion-read (arguments)
  "Read the live claim named by ARGUMENTS without moving the study window."
  (let* ((instance (romacs-mcp--companion-instance arguments))
         (handle (gethash "handle" arguments))
         (state (romacs-mcp--plain-object (romacs-companion-state instance handle))))
    (with-current-buffer (romacs-companion--buffer instance handle)
      (save-restriction
        (widen)
        (when (> (buffer-size) romacs-companion-receipts-text-limit)
          (user-error "Companion text exceeds read limit"))
        (puthash "text" (buffer-substring-no-properties (point-min) (point-max)) state)))
    state))

(defun romacs-mcp--companion-edit (arguments)
  "Apply the helper's receipt-qualified edit described by ARGUMENTS."
  (romacs-mcp--plain-object
   (romacs-companion-edit-once
    (romacs-mcp--companion-instance arguments) (gethash "operationId" arguments)
    (gethash "handle" arguments) (gethash "tick" arguments)
    (gethash "start" arguments) (gethash "end" arguments) (gethash "text" arguments))))

(defun romacs-mcp--companion-receipt (arguments)
  "Read the historical operation receipt named by fenced ARGUMENTS."
  (romacs-mcp--plain-object
   (romacs-companion-operation-state
    (romacs-mcp--companion-instance arguments) (gethash "operationId" arguments))))

(defun romacs-mcp--companion-undo (arguments)
  "Undo one ordinary group in the exact live claim named by ARGUMENTS.
Require the expected tick; do not auto-retry or rewrite an edit receipt."
  (let* ((instance (romacs-mcp--companion-instance arguments))
         (handle (gethash "handle" arguments)))
    (with-current-buffer (romacs-companion--buffer instance handle)
      (unless (= (gethash "tick" arguments) (buffer-modified-tick))
        (user-error "Companion modification tick is stale"))
      (when (or buffer-read-only (eq buffer-undo-list t))
        (user-error "Companion must be writable with undo enabled"))
      (save-excursion
        (save-restriction
          (widen)
          (let ((inhibit-modification-hooks t)) (undo-only 1))))
      (romacs-mcp--plain-object (romacs-companion-state instance handle)))))

(defun romacs-mcp--companion-retire (arguments)
  "Retire the exact claim named by ARGUMENTS without killing its buffer."
  (romacs-companion-retire (romacs-mcp--companion-instance arguments)
                            (gethash "handle" arguments))
  (romacs-mcp--object "retired" t))

;;;###autoload
(defun romacs-mcp-enable-companion-tools ()
  "Register optional cooperative companion tools without creating any buffers.
Receipt capacity is finite and process-local.  Never imply restart durability."
  (interactive)
  (let ((instance '("instance" . "string"))
        (handle '("handle" . "string"))
        (tick '("tick" . "integer"))
        (operation '("operationId" . "string")))
    (dolist (entry
             (list
              (list "romacs_companion_claim" "Claim an undisplayed non-file buffer." (list instance) #'romacs-mcp--companion-claim)
              (list "romacs_companion_read" "Read a live claimed buffer and modification tick." (list instance handle) #'romacs-mcp--companion-read)
              (list "romacs_companion_edit" "Edit once by operation ID and expected tick; 256 non-evicting process-local receipts." (list instance handle operation tick '("start" . "integer") '("end" . "integer") '("text" . "string")) #'romacs-mcp--companion-edit)
              (list "romacs_companion_receipt" "Read historical execution evidence before retrying an uncertain edit." (list instance operation) #'romacs-mcp--companion-receipt)
              (list "romacs_companion_undo" "Undo one ordinary group at expected tick. No automatic replay or receipt." (list instance handle tick) #'romacs-mcp--companion-undo)
              (list "romacs_companion_retire" "Retire a claim without killing or erasing its buffer." (list instance handle) #'romacs-mcp--companion-retire)))
      (romacs-mcp-register-tool
       (nth 0 entry) (nth 1 entry)
       (romacs-mcp--schema (nth 2 entry) (mapcar #'car (nth 2 entry)))
       (nth 3 entry)))))

(provide 'romacs-mcp-companion)
;;; romacs-mcp-companion.el ends here
