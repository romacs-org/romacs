;;; romacs-mcp-editor.el --- Bounded native MCP reads -*- lexical-binding: t; -*-

;; SPDX-License-Identifier: GPL-3.0-or-later

;;; Commentary:
;; Optional observation tools, not mutation handles or an ownership framework.
;; Explicit registration only; loading never starts a listener or opens a file.
;; Names and ticks describe the currently resolved native buffer, not an
;; enduring identity.  Catalogue pages are independent, unstable observations.

;;; Code:
(require 'romacs-mcp)
(require 'rx)

(defconst romacs-mcp-editor-output-limit 32768
  "Maximum encoded tool-result bytes, including the escaped text envelope.
The JSON-RPC request ID and framing are separately capped by the MCP core.
Bound extraction before serialization; this is not a peak-allocation bound.")
(defconst romacs-mcp-editor-scan-limit 128
  "Maximum native buffer-list slots examined per discovery call.
Native `buffer-list' still enumerates all references and offset traversal costs
O(offset); this does not promise O(page-size) total cost or atomic pagination.")
(defconst romacs-mcp--editor-name-limit 256)

(defun romacs-mcp--editor-credential-name-p (name)
  "Return non-nil if NAME lexically resembles a credential file or store."
  (and (stringp name)
       (let ((case-fold-search t))
         (string-match-p
          (rx (or string-start "/")
              (or (seq (optional (any "._")) (or "authinfo" "netrc")
                       (or string-end "." "~" "<"))
                  (seq (optional ".") "password-store" (or string-end "/"))))
          name))))

(defun romacs-mcp--editor-denied-p (buffer)
  "Return non-nil if BUFFER's native metadata denies structured disclosure.
Exclude minibuffers and their indirect aliases, credential-like names/paths,
and eval-approval modes.  Never inspect contents or resolve file paths.
This lexical policy is not a sandbox and cannot identify renamed secret copies."
  (with-current-buffer buffer
    (or (minibufferp buffer)
        (derived-mode-p 'hermes-exec-approval-mode)
        (cl-some #'romacs-mcp--editor-credential-name-p
                 (list (buffer-name) buffer-file-name buffer-file-truename))
        (when-let* ((base (buffer-base-buffer)))
          (romacs-mcp--editor-denied-p base)))))

(defun romacs-mcp--editor-integer (arguments key minimum maximum)
  "Return ARGUMENTS' integer KEY within MINIMUM and MAXIMUM, or refuse."
  (let ((value (gethash key arguments)))
    (unless (and (integerp value) (<= minimum value) (<= value maximum))
      (user-error "Invalid bounded integer: %s" key))
    value))

(defun romacs-mcp--editor-result-bytes (value)
  "Return the encoded modern text tool-result byte size for VALUE.
Match the core's nested JSON serialization, not just character or UTF-8 size."
  (string-bytes
   (json-serialize
    (romacs-mcp--object
     "resultType" "complete" "isError" :false
     "content" (vector
                (romacs-mcp--object
                 "type" "text" "text"
                 (decode-coding-string
                  (json-serialize value :false-object :false :null-object :null) 'utf-8))))
    :false-object :false :null-object :null)))

(defun romacs-mcp--editor-entry (buffer)
  "Return bounded native metadata for BUFFER without line scans or hooks."
  (with-current-buffer buffer
    (let ((mode (symbol-name major-mode)))
      (romacs-mcp--object
       "name" (buffer-name) "tick" (buffer-modified-tick)
       "sizeChars" (buffer-size) "point" (point)
       "mode" (substring mode 0 (min 128 (length mode)))
       "modeTruncated" (if (> (length mode) 128) t :false)
       "modified" (if (buffer-modified-p) t :false)
       "readOnly" (if buffer-read-only t :false)))))

(defun romacs-mcp--buffer-list (arguments)
  "Observe a bounded page of native buffer metadata for ARGUMENTS.
OFFSET addresses native enumeration slots, including excluded entries.
Names exceeding 256 characters, internal buffers and denied buffers are skipped.
A page can be empty and truncated; refresh after native catalogue changes."
  (romacs-mcp--instance arguments)
  (let* ((offset (romacs-mcp--editor-integer arguments "offset" 0 most-positive-fixnum))
         (limit (romacs-mcp--editor-integer arguments "limit" 1 32))
         (remaining (nthcdr offset (buffer-list)))
         (scanned 0) (entries nil) (full nil)
         (result (romacs-mcp--object
                  "instance" (gethash "instance" arguments) "offset" offset
                  "buffers" [] "scanned" 0 "truncated" :false "nextOffset" :null
                  "encodedOutputLimit" romacs-mcp-editor-output-limit)))
    (while (and remaining (not full) (< (length entries) limit)
                (< scanned romacs-mcp-editor-scan-limit))
      (let* ((buffer (car remaining)) (name (buffer-name buffer))
             (entry (and (buffer-live-p buffer) name
                         (> (length name) 0) (not (eq (aref name 0) ?\s))
                         (<= (length name) romacs-mcp--editor-name-limit)
                         (not (romacs-mcp--editor-denied-p buffer))
                         (romacs-mcp--editor-entry buffer))))
        (when entry
          (puthash "buffers" (vconcat (reverse (cons entry entries))) result)
          ;; Reserve final pagination metadata before accepting this slot.
          (puthash "scanned" (1+ scanned) result)
          (puthash "nextOffset" (+ offset scanned 1) result)
          (puthash "truncated" t result)
          (if (> (romacs-mcp--editor-result-bytes result) romacs-mcp-editor-output-limit)
              (setq full t)
            (push entry entries)))
        (unless full
          (setq remaining (cdr remaining))
          (cl-incf scanned))))
    (puthash "buffers" (vconcat (reverse entries)) result)
    (puthash "scanned" scanned result)
    (puthash "truncated" (if remaining t :false) result)
    (puthash "nextOffset" (if remaining (+ offset scanned) :null) result)
    (when (> (romacs-mcp--editor-result-bytes result) romacs-mcp-editor-output-limit)
      (user-error "Editor metadata exceeds encoded output budget"))
    (when (and full (= scanned 0))
      (user-error "Buffer metadata cannot fit in encoded output budget"))
    result))

(defun romacs-mcp--buffer-read (arguments)
  "Read a bounded literal character range from the named buffer in ARGUMENTS.
Resolve the current native namesake, check optional EXPECTEDTICK and preserve
current buffer, point and restriction.  Coordinates are widened, 1-based and
end-exclusive.  A name plus tick is not an enduring buffer or mutation identity."
  (romacs-mcp--instance arguments)
  (let* ((name (gethash "name" arguments))
         (count (romacs-mcp--editor-integer arguments "maxChars" 1 4096))
         (expected (gethash "expectedTick" arguments :absent))
         (buffer (and (stringp name) (> (length name) 0)
                      (<= (length name) romacs-mcp--editor-name-limit) (get-buffer name))))
    (unless (and (buffer-live-p buffer) (not (romacs-mcp--editor-denied-p buffer)))
      (user-error "Named buffer is unavailable for structured read"))
    (unless (or (eq expected :absent) (and (integerp expected) (>= expected 0)))
      (user-error "Invalid expected modification tick"))
    (with-current-buffer buffer
      (save-excursion
        (save-restriction
          (widen)
          (let* ((tick (buffer-modified-tick))
                 (start (romacs-mcp--editor-integer arguments "start" 1 (point-max)))
                 (end (min (point-max) (+ start count)))
                 (result (romacs-mcp--object
                          "instance" (gethash "instance" arguments) "name" (buffer-name)
                          "tick" tick "start" start "end" end "text" ""
                          "truncated" :false "nextStart" :null
                          "encodedOutputLimit" romacs-mcp-editor-output-limit)))
            (unless (or (eq expected :absent) (= expected tick))
              (user-error "Named buffer modification tick is stale"))
            ;; Every candidate materializes at most COUNT property-free chars.
            ;; Halving is bounded, honest truncation, not maximum-fit pagination.
            (while
                (progn
                  (puthash "text" (buffer-substring-no-properties start end) result)
                  (puthash "end" end result)
                  (puthash "truncated" (if (< end (point-max)) t :false) result)
                  (puthash "nextStart" (if (< end (point-max)) end :null) result)
                  (> (romacs-mcp--editor-result-bytes result) romacs-mcp-editor-output-limit))
              (when (= end start)
                (user-error "Read metadata exceeds encoded output budget"))
              (setq end (+ start (/ (- end start) 2))))
            (when (and (= end start) (< start (point-max)))
              (user-error "No character fits in encoded output budget"))
            result))))))

;;;###autoload
(defun romacs-mcp-enable-editor-tools ()
  "Register two optional native observation tools without editor side effects.
Preserve trusted owner eval and the separate companion claim/receipt authority."
  (interactive)
  (let* ((instance '("instance" . "string"))
         (list-schema (romacs-mcp--schema
                       (list instance '("offset" . "integer") '("limit" . "integer"))
                       '("instance" "offset" "limit")))
         (read-schema (romacs-mcp--schema
                       (list instance '("name" . "string") '("start" . "integer")
                             '("maxChars" . "integer") '("expectedTick" . "integer"))
                       '("instance" "name" "start" "maxChars"))))
    (dolist (spec (list (list list-schema "offset" 0 most-positive-fixnum)
                       (list list-schema "limit" 1 32)
                       (list read-schema "start" 1 most-positive-fixnum)
                       (list read-schema "maxChars" 1 4096)
                       (list read-schema "expectedTick" 0 most-positive-fixnum)))
      (let ((property (gethash (nth 1 spec) (gethash "properties" (car spec)))))
        (puthash "minimum" (nth 2 spec) property)
        (puthash "maximum" (nth 3 spec) property)))
    (puthash "maxLength" romacs-mcp--editor-name-limit
             (gethash "name" (gethash "properties" read-schema)))
    (romacs-mcp-register-tool
     "romacs_buffer_list" "Observe bounded native metadata. Unstable slot pagination; names are not handles."
     list-schema #'romacs-mcp--buffer-list (romacs-mcp--object "readOnlyHint" t))
    (romacs-mcp-register-tool
     "romacs_buffer_read" "Read current named buffer in widened character coordinates; optional expectedTick; no mutation authority."
     read-schema #'romacs-mcp--buffer-read (romacs-mcp--object "readOnlyHint" t))))

(provide 'romacs-mcp-editor)
;;; romacs-mcp-editor.el ends here
