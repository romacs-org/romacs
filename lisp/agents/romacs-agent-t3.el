;;; romacs-agent-t3.el --- T3 Code's provider instances, as a source  -*- lexical-binding: t -*-
;; SPDX-License-Identifier: GPL-3.0-or-later
;; Maintainer: Thanos Apollo <public@thanosapollo.org>

;;; Commentary:
;; The symbol `t3' in `romacs-agent-providers' stands for T3 Code's own
;; provider instances, read from its settings file
;; (`romacs-agent-t3-settings').  Only an instance's driver, display name,
;; enabled flags, and config's homePath, binaryPath, launchArgs, setupMode
;; and whether shadowHomePath is set are read; never its environment, nor
;; anything else in the file.  T3's own rules are followed: its two
;; default instances (claudeAgent and codex) are there unless the file
;; configures them, `enabled' is resolved as T3 resolves it, and
;; launchArgs is split as T3 splits it.  A Codex instance with an account
;; overlay (shadowHomePath, or setupMode "managed") is disabled: run
;; directly in its shared home it could use the wrong account.

;;; Code:

(require 'cl-lib)
(require 'subr-x)

(defcustom romacs-agent-t3-settings "~/.t3/userdata/settings.json"
  "T3 Code's settings file, read by the `t3' entry of `romacs-agent-providers'."
  :type 'file :group 'romacs-agent)

(defconst romacs-agent-t3--default-instances
  '(("claudeAgent" . "Claude") ("codex" . "Codex"))
  "T3's default instances, enabled while its settings do not configure
them: their ids (also their drivers) and display names.")

(defconst romacs-agent-t3--disabled-drivers
  '("cursor" "grok" "fx" "muse" "pi" "opencode" "antigravity")
  "T3's drivers whose instances are disabled unless enabled explicitly.")

(defun romacs-agent-t3-split (args)
  "ARGS, a launch arguments string, as T3 Code splits it into words:
whitespace separates them; single and double quotes group (a backslash in
double quotes escapes \" \\ $ and `); a backslash outside quotes escapes
whitespace or a quote."
  (let ((tokens nil) (current nil) (quote nil) (quoted nil) (i 0) (n (length args)))
    (cl-flet ((space-p (c) (and c (string-match-p "[[:space:]]" (string c))))
              (next () (and (< (1+ i) n) (aref args (1+ i)))))
      (while (< i n)
        (let ((c (aref args i)))
          (cond (quote
                 (cond ((eq c quote) (setq quote nil quoted t))
                       ((and (eq c ?\\) (eq quote ?\") (memq (next) '(?\" ?\\ ?$ ?`)))
                        (push (next) current) (cl-incf i))
                       (t (push c current))))
                ((memq c '(?' ?\")) (setq quote c quoted t))
                ((space-p c)
                 (when (or current quoted)
                   (push (apply #'string (nreverse current)) tokens)
                   (setq current nil quoted nil)))
                ((and (eq c ?\\) (or (space-p (next)) (memq (next) '(?' ?\"))))
                 (push (next) current) (cl-incf i))
                (t (push c current))))
        (cl-incf i)))
    (when (or current quoted) (push (apply #'string (nreverse current)) tokens))
    (nreverse tokens)))

(defun romacs-agent-t3--enabled (instance config)
  "T3's resolveProviderInstanceEnabled: either flag false disables, else
the instance's flag, the config's, then its driver's default."
  (let ((own (gethash "enabled" instance :absent))
        (configured (if (hash-table-p config) (gethash "enabled" config :absent) :absent)))
    (cond ((or (eq own :false) (eq configured :false)) nil)
          ((eq own t) t)
          ((eq configured t) t)
          (t (not (member (gethash "driver" instance) romacs-agent-t3--disabled-drivers))))))

(defun romacs-agent-t3--name (display)
  "An instance name from T3's DISPLAY name: \"Claude2\" is claude2."
  (replace-regexp-in-string "[[:space:]]+" "-" (downcase (string-trim display))))

(defun romacs-agent-t3--check (id object key types)
  "OBJECT's KEY, signalling an error unless it is absent, null or one of
TYPES (`string', `boolean', `object'): a mistyped field is never taken
for a default (another account's directory, say)."
  (let ((value (gethash key object)))
    (unless (or (null value)
                (and (memq 'string types) (stringp value))
                (and (memq 'boolean types) (memq value '(t :false)))
                (and (memq 'object types) (hash-table-p value)))
      (error "T3 instance %s: %s is not %s" (or id "settings") key
             (mapconcat #'symbol-name types " or ")))
    value))

(defun romacs-agent-t3--path (value)
  "A path field's VALUE trimmed, as T3 reads it; `~' expanded where it is
a path (a bare command name is looked up as it is); nil when empty."
  (let ((value (and value (string-trim value))))
    (cond ((or (null value) (string-empty-p value)) nil)
          ((string-prefix-p "~" value) (expand-file-name value))
          (t value))))

(defun romacs-agent-t3-instances (settings harness-of)
  "T3 Code's instances in SETTINGS (its parsed settings, a hash table),
as (INSTANCES . CONSTRAINTS): each instance a plist of
`romacs-agent-providers' keys plus :t3-id and :source t3, its harness
the one HARNESS-OF (a function of T3's driver name) gives; a constraint
\(:source t3 :id ID :reason TEXT) for each instance Romacs cannot run as
T3 does.  T3's defaults come first, then its configured instances in
order.  An instance is named after its display name unless that is
taken, or is another instance's T3 id; then by its own T3 id.  A field
of the wrong type signals an error: the file is not taken at all."
  (let* ((configured (or (romacs-agent-t3--check nil settings "providerInstances" '(object))
                         (make-hash-table :test #'equal)))
         (rows (append (cl-loop for (id . display) in romacs-agent-t3--default-instances
                                unless (gethash id configured)
                                collect (let ((row (make-hash-table :test #'equal)))
                                          (puthash "driver" id row) (puthash "displayName" display row)
                                          (puthash "enabled" t row)
                                          (cons id row)))
                       (let ((pairs nil)) (maphash (lambda (id row) (push (cons id row) pairs)) configured)
                            (nreverse pairs))))
         ;; Every T3 id is kept for its own instance, should it need it.
         (ids (let ((table (make-hash-table :test #'equal)))
                (pcase-dolist (`(,id . ,_) rows) (puthash id t table))
                table))
         (taken (make-hash-table :test #'equal))
         (instances nil) (constraints nil))
    (pcase-dolist (`(,id . ,row) rows)
      (unless (hash-table-p row) (error "T3 instance %s is not an object" id))
      (let* ((driver (romacs-agent-t3--check id row "driver" '(string)))
             (shown (romacs-agent-t3--check id row "displayName" '(string)))
             (config (romacs-agent-t3--check id row "config" '(object)))
             (field (lambda (key types) (and config (romacs-agent-t3--check id config key types)))))
        (romacs-agent-t3--check id row "enabled" '(boolean))
        (dolist (key '("homePath" "binaryPath" "launchArgs" "setupMode" "shadowHomePath"))
          (funcall field key '(string)))
        (funcall field "enabled" '(boolean))
        (unless driver (error "T3 instance %s has no driver" id))
        (let* ((harness (funcall harness-of driver))
               ;; A default instance configured with no name keeps its own.
               (display (if (and shown (not (string-empty-p (string-trim shown)))) shown
                          (or (cdr (assoc id romacs-agent-t3--default-instances)) id)))
               (name (let ((candidate (romacs-agent-t3--name display)))
                       (if (or (gethash candidate taken)
                               (and (gethash candidate ids) (not (equal candidate id))))
                           id
                         candidate)))
               (overlay (and (equal driver "codex")
                             (or (romacs-agent-t3--path (funcall field "shadowHomePath" '(string)))
                                 (equal (funcall field "setupMode" '(string)) "managed")))))
          (cond
           ((not harness)
            (push (list :source 't3 :id id :reason (format "no Romacs harness for T3 driver %s" driver)) constraints))
           (t
            (puthash name t taken)
            (when overlay
              (push (list :source 't3 :id id
                          :reason "T3 auth overlay (shadowHomePath / managed setup) not supported")
                    constraints))
            (push (append (list name :harness harness :display display :source 't3 :t3-id id
                                :enabled (and (not overlay) (romacs-agent-t3--enabled row config)))
                          (when-let* ((home (romacs-agent-t3--path (funcall field "homePath" '(string)))))
                            (list :config-dir home))
                          (when-let* ((binary (romacs-agent-t3--path (funcall field "binaryPath" '(string)))))
                            (list :program binary))
                          (when-let* ((args (funcall field "launchArgs" '(string))))
                            (list :arguments (romacs-agent-t3-split args))))
                  instances))))))
    (cons (nreverse instances) (nreverse constraints))))

(defvar romacs-agent-t3--read nil
  "The last read of a settings file: (FILE STAMP GENERATION . RESULT), its
size and modification time, the driver registry's generation it was read
for, and `romacs-agent-t3-instances''s result.")

(defun romacs-agent-t3-read (file harness-of generation)
  "T3's instances in settings FILE, as `romacs-agent-t3-instances' gives
them, read again only when FILE (its size and time) or the driver
registry (its GENERATION) changed; a missing or malformed file gives no
instance and says why, once."
  (let* ((file (expand-file-name file))
         (attributes (file-attributes file))
         (stamp (and attributes (cons (file-attribute-size attributes)
                                      (file-attribute-modification-time attributes)))))
    (if (and romacs-agent-t3--read (equal (nth 0 romacs-agent-t3--read) file)
             (equal (nth 1 romacs-agent-t3--read) stamp) (eql (nth 2 romacs-agent-t3--read) generation))
        (nthcdr 3 romacs-agent-t3--read)
      (let ((result
             (if (not stamp)
                 (cons nil (list (list :source 't3 :id nil :reason (format "T3 settings not found: %s" file))))
               (condition-case err
                   (let ((settings (with-temp-buffer
                                     (let ((coding-system-for-read 'utf-8)) (insert-file-contents file))
                                     (json-parse-buffer :object-type 'hash-table :null-object nil :false-object :false))))
                     (unless (hash-table-p settings) (error "not a JSON object"))
                     (romacs-agent-t3-instances settings harness-of))
                 (error
                  (message "T3 settings %s: %s" file (error-message-string err))
                  (cons nil (list (list :source 't3 :id nil
                                        :reason (format "T3 settings unreadable: %s" (error-message-string err))))))))))
        (setq romacs-agent-t3--read (cons file (cons stamp (cons generation result))))
        result))))

(defun romacs-agent-t3-store (settings)
  "T3's thread store beside its SETTINGS file."
  (expand-file-name "statev2.sqlite" (file-name-directory (expand-file-name settings))))

(defvar romacs-agent-t3--titles (make-hash-table :test #'equal)
  "Each thread store read, to (STAMP . TITLES): TITLES from a thread's key
\(`romacs-agent-t3-title-key') to its title, as of the store's STAMP.")

(defun romacs-agent-t3-title-key (instance native)
  "The key of T3 INSTANCE's thread on harness conversation NATIVE: two
instances' homes can hold the same id."
  (concat instance "\n" native))

(defconst romacs-agent-t3--titles-query
  (concat "select p.provider_instance_id, json_extract(p.payload_json, '$.nativeThreadRef.nativeId'), t.title"
          " from orchestration_v2_projection_provider_threads p"
          " join orchestration_v2_projection_threads t on t.thread_id = p.thread_id"
          " where t.title is not null and t.title != '' order by p.updated_at")
  "Each thread's title, by its T3 instance and harness conversation: titles
only, never a message; the newest row of a key last, so it wins.")

(defun romacs-agent-t3-thread-titles (store)
  "T3's own titles of its threads in thread STORE (`romacs-agent-t3-store'),
by `romacs-agent-t3-title-key': a thread T3 delegated starts with T3's
preamble, its title says what it is about.  Read once while STORE and
its write-ahead log are unchanged, opened read only (T3 writes it as it
runs) with a short busy timeout, never waited on; nil when it cannot be
read (gone, locked, another schema), quietly: a title then comes from
the prompt, and the next refresh reads again."
  (let ((stamp (mapcar (lambda (f) (let ((a (file-attributes f)))
                                     (and a (cons (file-attribute-size a) (file-attribute-modification-time a)))))
                       (list store (concat store "-wal"))))
        (cached (gethash store romacs-agent-t3--titles)))
    (cond ((or (null (car stamp)) (not (fboundp 'sqlite-open))) nil)
          ((equal (car cached) stamp) (cdr cached))
          (t
           (condition-case nil
               (let ((db (sqlite-open store t)) (titles (make-hash-table :test #'equal)))
                 (unwind-protect
                     (progn
                       (sqlite-pragma db "busy_timeout = 50")
                       ;; Row by row: a row that cannot be read signals
                       ;; (a whole-list select stops there quietly).
                       (let ((rows (sqlite-select db romacs-agent-t3--titles-query nil 'set)))
                         (unwind-protect
                             (let (row)
                               (while (setq row (sqlite-next rows))
                                 (when (and (stringp (nth 0 row)) (stringp (nth 1 row)) (stringp (nth 2 row)))
                                   (puthash (romacs-agent-t3-title-key (nth 0 row) (nth 1 row)) (nth 2 row) titles))))
                           (sqlite-finalize rows))))
                   (sqlite-close db))
                 ;; Kept only when read: a failed read is tried again.
                 (puthash store (cons stamp titles) romacs-agent-t3--titles)
                 titles)
             (error nil))))))

(provide 'romacs-agent-t3)
;;; romacs-agent-t3.el ends here
