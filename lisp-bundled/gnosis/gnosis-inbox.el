;;; gnosis-inbox.el --- Approval inbox for agent-drafted themata -*- lexical-binding: t; -*-

;; Copyright (C) 2026 Free Software Foundation, Inc.
;; Author: Thanos Apollo <public@thanosapollo.org>
;; SPDX-License-Identifier: GPL-3.0-or-later

;;; Commentary:

;; Agents never write themata.  They append drafts to an Org inbox next
;; to the database; the learner approves each one with a single key.
;;
;; File: `gnosis-inbox-file', by default "inbox.org" in `gnosis-dir'.
;; Every level-1 heading is one entry.  A pending entry is a native thema
;; in the export codec (`gnosis-export--insert-thema' shape, GNOSIS_ID
;; NEW) whose property drawer also carries provenance:
;;
;;   :GNOSIS_PROPOSAL: lecture | contrast | rework-replace | rework-split
;;                     | rework-annotate
;;   :GNOSIS_SOURCE:   absolute source file, or thema:ID for rework
;;   :GNOSIS_PAGE:     physical PDF page or slide number (lecture)
;;   :GNOSIS_SPAN:     optional anchor inside the page (dedupe identity);
;;                     required to keep two questions with the same
;;                     answers on one page apart
;;   :GNOSIS_AGENT:    who drafted it, e.g. hermes/default
;;   :GNOSIS_REASON:   one line: why this card, e.g. "slide 13 step 1"
;;   :GNOSIS_CREATED:  ISO time
;;   :GNOSIS_KEY:      dedupe key (`gnosis-inbox-key'), filled by Gnosis
;;   :GNOSIS_TARGET:   rework only: thema ID being replaced, split or
;;                     annotated (an annotation only appends to its
;;                     explanation and keeps its schedule)
;;   :GNOSIS_TARGET_REVISION: rework only: content revision when drafted
;;
;; Approval, direct or after editing, saves through `gnosis-save-thema',
;; the same validation and writer as the native editor, refuses entries
;; with invalid provenance, and replaces the entry with a ledger
;; heading (GNOSIS_STATE applied, rejected or duplicate) that keeps only
;; its key and provenance, so re-running an agent cannot re-propose it.
;; Other non-pending headings, such as `gnosis-leech' rework requests
;; (GNOSIS_STATE requested), are never shown for approval.
;;
;; Producer entry points: `gnosis-inbox-add' (plists, from Lisp),
;; `gnosis-inbox-add-file' (an Org file of drafts) and the batch command
;; `gnosis-inbox-batch-add', which needs no running Emacs and never opens
;; the learner database: validation runs the native writer against a
;; disposable scratch database.  Reviewer entry point: `gnosis-inbox'.

;;; Code:

(require 'cl-lib)
(require 'gnosis)
(require 'gnosis-db)
(require 'gnosis-sqlite)
(require 'gnosis-cloze)
(require 'gnosis-export-import)
(require 'gnosis-lecture)
(require 'json)
(require 'org)
(require 'org-element)
(require 'url-util)

(declare-function gnosis-leech-state "gnosis-leech")
(declare-function gnosis-leech-recent-wrong "gnosis-leech")

(defgroup gnosis-inbox nil
  "Approval inbox for agent-drafted themata."
  :group 'gnosis)

(defcustom gnosis-inbox-file-name "inbox.org"
  "File name of the inbox, relative to `gnosis-dir'."
  :type 'string)

(defcustom gnosis-inbox-max-per-source 30
  "Maximum number of pending drafts from one source file."
  :type 'natnum)

(defcustom gnosis-inbox-max-pending 300
  "Maximum number of pending drafts in the inbox."
  :type 'natnum)

(defconst gnosis-inbox-proposal-kinds
  '("lecture" "contrast" "rework-replace" "rework-split" "rework-annotate")
  "Supported GNOSIS_PROPOSAL values.
Rework kinds carry GNOSIS_TARGET.  Accepting a replace or split suspends
the target; accepting an annotation appends to its explanation instead.")

(defconst gnosis-inbox-thema-types '("basic" "cloze" "mcq" "mc-cloze" "agent-eval")
  "Thema types agents may draft.  Media types need native attachment.")

(defconst gnosis-inbox--properties
  '((:kind . "GNOSIS_PROPOSAL") (:source . "GNOSIS_SOURCE")
    (:page . "GNOSIS_PAGE") (:span . "GNOSIS_SPAN")
    (:agent . "GNOSIS_AGENT") (:reason . "GNOSIS_REASON")
    (:created . "GNOSIS_CREATED") (:key . "GNOSIS_KEY")
    (:target . "GNOSIS_TARGET") (:target-revision . "GNOSIS_TARGET_REVISION")
    (:reject-reason . "GNOSIS_REJECT_REASON"))
  "Provenance plist keys and their Org property names, in drawer order.")

(defun gnosis-inbox-file ()
  "Return the inbox file for the current `gnosis-dir'."
  (expand-file-name gnosis-inbox-file-name gnosis-dir))

;;; Pure helpers

(defun gnosis-inbox--normalize (text)
  "Return TEXT downcased with whitespace collapsed."
  (string-trim (replace-regexp-in-string "[ \t\n\r]+" " " (downcase (or text "")))))

(defvar gnosis-inbox--source-hashes (make-hash-table :test 'equal)
  "Source SHA256 digests keyed by (TRUENAME DEVICE INODE SIZE MTIME CTIME).
Rewriting a file always moves its status-change time, even when the
modification time is restored afterwards.")

(defun gnosis-inbox--source-identity (source)
  "Return the dedupe identity of SOURCE.
A readable local file is identified by the SHA256 of its bytes, so a
moved or renamed lecture keeps identity and different lectures sharing a
file name stay apart.  Other sources, such as thema:ID, are their own
identity."
  (if (and (stringp source) (file-name-absolute-p source)
           (not (file-remote-p source))
           (file-regular-p source) (file-readable-p source))
      (let* ((attributes (file-attributes source))
             (stamp (list (file-truename source)
                          (file-attribute-device-number attributes)
                          (file-attribute-inode-number attributes)
                          (file-attribute-size attributes)
                          (file-attribute-modification-time attributes)
                          (file-attribute-status-change-time attributes))))
        (or (gethash stamp gnosis-inbox--source-hashes)
            (puthash stamp
                     (with-temp-buffer
                       (set-buffer-multibyte nil)
                       (insert-file-contents-literally source)
                       (concat "sha256:" (secure-hash 'sha256 (current-buffer))))
                     gnosis-inbox--source-hashes)))
    (format "%s" (or source ""))))

(defun gnosis-inbox-key (source page span &optional revision)
  "Return the dedupe key for SPAN on PAGE of SOURCE.
SOURCE is identified by `gnosis-inbox--source-identity', the SHA256 of
its bytes when it is a readable file.  REVISION is a rework target's
content revision: a changed target may be proposed again.
SPAN defaults, in `gnosis-inbox--draft-key', to the draft's answers: two
drafts testing the same answer on the same page are the same proposal
unless they carry distinct explicit spans."
  (secure-hash 'sha1 (format "gnosis-inbox-2\0%s\0%s\0%s\0%s"
                             (gnosis-inbox--source-identity source)
                             (or page "")
                             (gnosis-inbox--normalize span)
                             (or revision ""))))

(defun gnosis-inbox--draft-key (draft)
  "Return the dedupe key of DRAFT plist."
  (gnosis-inbox-key (plist-get draft :source)
                    (plist-get draft :page)
                    (or (plist-get draft :span)
                        (mapconcat #'identity (plist-get draft :answer) "\n"))
                    (plist-get draft :target-revision)))

(defun gnosis-inbox--list-text (values)
  "Return VALUES as the native codec list body."
  (concat (string-remove-prefix "\n" gnosis-export-separator)
          (mapconcat #'identity values gnosis-export-separator)))

(defun gnosis-inbox--one-line (value)
  "Return VALUE as a single-line property string, or nil."
  (when value
    (let ((text (string-trim (replace-regexp-in-string
                              "[\n\r]+" " " (format "%s" value)))))
      (unless (string-empty-p text) text))))

(defun gnosis-inbox--draft-text (draft)
  "Return native Org text for DRAFT plist, with provenance properties."
  (with-temp-buffer
    (delay-mode-hooks (org-mode))
    (let ((type (plist-get draft :type))
          (inhibit-read-only t))
      (gnosis-export--insert-thema
       "NEW" type (plist-get draft :keimenon)
       (gnosis-inbox--list-text (plist-get draft :hypothesis))
       (if (equal type "agent-eval")
           (car (plist-get draft :answer))
         (gnosis-inbox--list-text (plist-get draft :answer)))
       (or (plist-get draft :parathema) "")
       (plist-get draft :tags) nil
       (plist-get draft :aliases) (plist-get draft :rubric))
      (goto-char (point-min))
      (re-search-forward "^:END:$")
      (goto-char (match-beginning 0))
      (pcase-dolist (`(,key . ,name) gnosis-inbox--properties)
        (when-let* ((value (gnosis-inbox--one-line (plist-get draft key))))
          (insert ":" name ": " value "\n")))
      (string-trim (buffer-substring-no-properties (point-min) (point-max))))))

(defun gnosis-inbox--parse-thema (text)
  "Parse native thema TEXT; return its single thema or signal `user-error'."
  (with-temp-buffer
    (delay-mode-hooks (org-mode))
    (insert text)
    (let ((themata (gnosis-export-parse-themata)))
      (unless (= (length themata) 1)
        (user-error "Draft must contain exactly one thema"))
      (let ((thema (car themata)))
        (unless (equal (nth 0 thema) "NEW")
          (user-error "Draft GNOSIS_ID must be NEW"))
        (unless (member (downcase (nth 1 thema)) gnosis-inbox-thema-types)
          (user-error "Draft type must be one of: %s"
                      (mapconcat #'identity gnosis-inbox-thema-types ", ")))
        thema))))

(defun gnosis-inbox--thema-draft (thema props)
  "Return a draft plist from parsed native THEMA and provenance PROPS."
  (append (list :type (downcase (nth 1 thema)) :keimenon (nth 2 thema)
                :hypothesis (remove "" (nth 3 thema)) :answer (nth 4 thema)
                :parathema (nth 5 thema) :tags (nth 6 thema)
                :aliases (nth 8 thema) :rubric (nth 9 thema))
          props))

(defun gnosis-inbox--same-fields-p (draft thema)
  "Return non-nil if parsed THEMA preserves every field of DRAFT."
  (let ((parsed (gnosis-inbox--thema-draft thema nil)))
    (cl-every (lambda (key)
                (equal (or (plist-get draft key) (and (eq key :parathema) ""))
                       (or (plist-get parsed key) (and (eq key :parathema) ""))))
              '(:type :keimenon :hypothesis :answer :parathema :tags :aliases :rubric))))

(defun gnosis-inbox--check-provenance (draft)
  "Signal `user-error' unless DRAFT carries valid provenance."
  (let ((kind (plist-get draft :kind))
        (source (plist-get draft :source))
        (page (plist-get draft :page)))
    (unless (member kind gnosis-inbox-proposal-kinds)
      (user-error "GNOSIS_PROPOSAL must be one of: %s"
                  (mapconcat #'identity gnosis-inbox-proposal-kinds ", ")))
    (dolist (key '(:agent :reason))
      (unless (gnosis-inbox--one-line (plist-get draft key))
        (user-error "Draft requires %s" (cdr (assq key gnosis-inbox--properties)))))
    (pcase kind
      ("lecture"
       (unless (and (stringp source) (file-name-absolute-p source)
                    (not (file-remote-p source)))
         (user-error "Lecture drafts require an absolute local GNOSIS_SOURCE"))
       (unless (and (integerp page) (> page 0))
         (user-error "Lecture drafts require a positive GNOSIS_PAGE")))
      ((or "rework-replace" "rework-split" "rework-annotate")
       (unless (and (integerp (plist-get draft :target))
                    (plist-get draft :target-revision))
         (user-error "Rework drafts require GNOSIS_TARGET and GNOSIS_TARGET_REVISION"))))
    (unless (or (string-prefix-p "rework-" kind)
                (not (or (plist-get draft :target) (plist-get draft :target-revision))))
      (user-error "Only rework drafts may carry GNOSIS_TARGET"))))

;;; Scratch validation through the native writer

(defmacro gnosis-inbox--with-scratch-db (&rest body)
  "Run BODY with `gnosis-db' bound to a disposable fresh database.
The learner database is neither opened nor consulted."
  (declare (indent 0) (debug t))
  (let ((dir (gensym "dir")))
    `(let* ((,dir (make-temp-file "gnosis-inbox-" t))
            (gnosis-dir (file-name-as-directory ,dir))
            (gnosis-db nil)
            (gnosis--id-cache nil)
            (gnosis-save-hook nil))
       (unwind-protect
           (progn (setq gnosis-db (gnosis-db--open gnosis-dir))
                  ,@body)
         (when gnosis-db (gnosis-sqlite-close gnosis-db))
         (delete-directory ,dir t)))))

(defun gnosis-inbox--native-check (thema)
  "Return nil if the native writer accepts THEMA, else its error string.
Must run inside `gnosis-inbox--with-scratch-db'."
  (let ((result (gnosis-save-thema thema t)))
    (and (stringp result) result)))

(defun gnosis-inbox-validate (draft)
  "Validate DRAFT plist; return its native Org text or signal `user-error'.
Check provenance, the native codec round trip and the native writer.
Run the writer only against a scratch database."
  (gnosis-inbox--check-provenance draft)
  (when (and (equal (plist-get draft :kind) "lecture")
             (not (and (file-regular-p (plist-get draft :source))
                       (file-readable-p (plist-get draft :source)))))
    (user-error "Lecture source is not a readable file: %s" (plist-get draft :source)))
  (unless (member (plist-get draft :type) gnosis-inbox-thema-types)
    (user-error "Draft type must be one of: %s"
                (mapconcat #'identity gnosis-inbox-thema-types ", ")))
  (let* ((text (gnosis-inbox--draft-text draft))
         (thema (gnosis-inbox--parse-thema text)))
    (unless (gnosis-inbox--same-fields-p draft thema)
      (user-error "Draft does not survive the native Org codec unchanged"))
    (when-let* ((error (gnosis-inbox--with-scratch-db
                         (gnosis-inbox--native-check thema))))
      (user-error "%s" error))
    text))

;;; Reading and writing the file

(defun gnosis-inbox--read (file)
  "Return the contents of FILE, or the empty string if absent."
  (if (file-exists-p file)
      (with-temp-buffer
        (let ((coding-system-for-read 'utf-8))
          (insert-file-contents file))
        (buffer-string))
    ""))

(defun gnosis-inbox--read-input (file)
  "Return the contents of producer input FILE, or signal `user-error'.
Unlike the inbox, required input that is absent is an error."
  (unless (and (stringp file) (file-regular-p file) (file-readable-p file))
    (user-error "Drafts file is not a readable file: %s" file))
  (gnosis-inbox--read file))

(defun gnosis-inbox--hash (text)
  "Return the SHA256 of TEXT."
  (secure-hash 'sha256 (encode-coding-string text 'utf-8)))

(defun gnosis-inbox--number (value)
  "Return VALUE as a positive integer, or VALUE unchanged."
  (if (and (stringp value) (string-match-p "\\`[0-9]+\\'" value))
      (string-to-number value)
    value))

(defun gnosis-inbox-parse (text)
  "Parse inbox TEXT into a list of entry plists, in file order.
Each entry has :begin, :end (0-based offsets into TEXT), :text (its
exact text without trailing whitespace), :state (pending, applied,
rejected or duplicate), provenance keys, and for pending entries
either :thema (parsed native thema with valid provenance) or :error
\(why it cannot be accepted)."
  (with-temp-buffer
    (delay-mode-hooks (org-mode))
    (insert text)
    (let (entries)
      (org-element-map (org-element-parse-buffer 'headline) 'headline
        (lambda (headline)
          (when (= 1 (org-element-property :level headline))
            (let* ((begin (org-element-property :begin headline))
                   (end (org-element-property :end headline))
                   (state (org-element-property :GNOSIS_STATE headline))
                   (entry (list :begin (1- begin) :end (1- end)
                                :text (string-trim-right
                                       (buffer-substring-no-properties begin end))
                                :title (org-element-property :raw-value headline)
                                :state (if state (intern state) 'pending))))
              (pcase-dolist (`(,key . ,name) gnosis-inbox--properties)
                (when-let* ((value (org-element-property (intern (concat ":" name)) headline)))
                  (setq entry (plist-put entry key
                                         (if (memq key '(:page :target))
                                             (gnosis-inbox--number value)
                                           value)))))
              (when-let* ((ids (org-element-property :GNOSIS_THEMATA headline)))
                (setq entry (plist-put entry :themata
                                       (mapcar #'string-to-number (split-string ids)))))
              (when (eq (plist-get entry :state) 'pending)
                (condition-case err
                    (let ((thema (gnosis-inbox--parse-thema
                                  (buffer-substring-no-properties begin end))))
                      (gnosis-inbox--check-provenance
                       (gnosis-inbox--thema-draft thema entry))
                      (setq entry (plist-put entry :thema thema)))
                  (error (setq entry (plist-put entry :error (error-message-string err))))))
              (unless (plist-get entry :key)
                (setq entry (plist-put entry :key
                                       (if-let* ((thema (plist-get entry :thema)))
                                           (gnosis-inbox--draft-key
                                            (gnosis-inbox--thema-draft thema entry))
                                         (gnosis-inbox--hash
                                          (buffer-substring-no-properties begin end))))))
              (push entry entries))))
        nil nil 'headline)
      (nreverse entries))))

(defun gnosis-inbox--preamble ()
  "Return the header written to a new inbox file."
  (concat "#+title: Gnosis inbox\n"
          "# Agent drafts awaiting approval.  Review with M-x gnosis-inbox.\n"
          "# Append drafts with gnosis-inbox-add or gnosis-inbox-batch-add.\n"))

(defun gnosis-inbox--write (file expected text)
  "Atomically replace FILE with TEXT if FILE still hashes to EXPECTED.
EXPECTED is the hash of the contents the change was computed from."
  (let ((directory (file-name-directory file)))
    (make-directory directory t)
    (let ((temp (make-temp-file (expand-file-name ".inbox-" directory) nil ".tmp")))
      (unwind-protect
          (progn
            (let ((coding-system-for-write 'utf-8-unix))
              (write-region text nil temp nil 'silent))
            (unless (equal expected (gnosis-inbox--hash (gnosis-inbox--read file)))
              (user-error "Inbox changed on disk; reload and retry"))
            (rename-file temp file t))
        (when (file-exists-p temp) (delete-file temp))))))

(defun gnosis-inbox--ledger-text (entry state &optional ids extra)
  "Return ledger heading text closing ENTRY with STATE.
IDS are saved thema IDs.  EXTRA is a plist of additional properties."
  (concat
   "* " (capitalize (symbol-name state)) "\n:PROPERTIES:\n"
   ":GNOSIS_STATE: " (symbol-name state) "\n"
   (mapconcat (lambda (pair)
                (if-let* ((value (gnosis-inbox--one-line (plist-get entry (car pair)))))
                    (format ":%s: %s\n" (cdr pair) value)
                  ""))
              (seq-filter (lambda (pair)
                            (memq (car pair) '(:key :kind :source :page :span :agent :target)))
                          gnosis-inbox--properties)
              "")
   (if ids (format ":GNOSIS_THEMATA: %s\n" (mapconcat #'number-to-string ids " ")) "")
   (cl-loop for (key value) on extra by #'cddr
            when (gnosis-inbox--one-line value)
            concat (format ":%s: %s\n" key (gnosis-inbox--one-line value)))
   ":GNOSIS_CLOSED: " (format-time-string "%FT%T%z") "\n:END:\n"))

(defun gnosis-inbox--replace-entry (file expected entry replacement)
  "Replace ENTRY in FILE (hashing to EXPECTED) with REPLACEMENT text."
  (let* ((text (gnosis-inbox--read file))
         (_ (unless (equal expected (gnosis-inbox--hash text))
              (user-error "Inbox changed on disk; reload and retry")))
         (new (concat (substring text 0 (plist-get entry :begin))
                      replacement
                      (substring text (plist-get entry :end)))))
    (gnosis-inbox--write file expected new)
    new))

;;; Producer API

(defun gnosis-inbox-add (drafts)
  "Append DRAFTS to the inbox; return one result plist per draft.
DRAFTS is a list of plists with thema fields :type :keimenon :hypothesis
:answer :parathema :tags :aliases :rubric (lists of strings where the
native writer takes lists) and provenance :kind :source :page :span
:agent :reason, plus :target and :target-revision for rework.
Each result is (:key KEY :status STATUS :error MESSAGE) where STATUS is
`added', `duplicate' (same key already pending or closed), `invalid' or
`capped'.  Valid drafts are written in one atomic file replacement.
Never opens the learner database."
  (let* ((file (gnosis-inbox-file))
         (text (gnosis-inbox--read file))
         (hash (gnosis-inbox--hash text))
         (entries (gnosis-inbox-parse text))
         (known (mapcar (lambda (e) (plist-get e :key)) entries))
         (pending (seq-filter (lambda (e) (eq (plist-get e :state) 'pending)) entries))
         (count (length pending))
         (per-source (make-hash-table :test 'equal))
         added results)
    (dolist (entry pending)
      (cl-incf (gethash (plist-get entry :source) per-source 0)))
    (dolist (draft drafts)
      (let* ((draft (copy-sequence draft))
             (key (gnosis-inbox--draft-key draft)))
        (setq draft (plist-put draft :key key))
        (unless (plist-get draft :created)
          (setq draft (plist-put draft :created (format-time-string "%FT%T%z"))))
        (push
         (cond
          ((member key known) (list :key key :status 'duplicate))
          ((or (>= count gnosis-inbox-max-pending)
               (>= (gethash (plist-get draft :source) per-source 0)
                   gnosis-inbox-max-per-source))
           (list :key key :status 'capped))
          (t
           (condition-case err
               (let ((entry-text (gnosis-inbox-validate draft)))
                 (push entry-text added)
                 (push key known)
                 (cl-incf count)
                 (cl-incf (gethash (plist-get draft :source) per-source 0))
                 (list :key key :status 'added))
             (error (list :key key :status 'invalid
                          :error (error-message-string err))))))
         results)))
    (when added
      (gnosis-inbox--write
       file hash
       (concat (if (string-empty-p text) (gnosis-inbox--preamble)
                 (concat (string-trim-right text) "\n"))
               "\n" (mapconcat #'identity (nreverse added) "\n\n") "\n")))
    (nreverse results)))

(defun gnosis-inbox-add-file (file)
  "Append the native drafts in Org FILE to the inbox.
FILE holds level-1 native themata with GNOSIS_ID NEW and provenance
properties, as an agent writes them.  Return `gnosis-inbox-add' results;
an unparsable heading yields an `invalid' result.  Signal `user-error' if
FILE is unreadable or holds no heading."
  (let ((entries (gnosis-inbox-parse (gnosis-inbox--read-input file)))
        drafts invalid)
    (unless entries
      (user-error "No drafts in %s" file))
    (dolist (entry entries)
      (if-let* ((thema (plist-get entry :thema)))
          (push (gnosis-inbox--thema-draft
                 thema (cl-loop for (key . _) in gnosis-inbox--properties
                                unless (eq key :key)
                                append (list key (plist-get entry key))))
                drafts)
        (push (list :key (plist-get entry :key) :status 'invalid
                    :error (or (plist-get entry :error) "Not a pending draft"))
              invalid)))
    (append (nreverse invalid) (gnosis-inbox-add (nreverse drafts)))))

(defun gnosis-inbox--batch-print (&rest fields)
  "Print FIELDS, an alist, as one JSON object on standard output."
  (princ (json-encode fields))
  (terpri))

(defun gnosis-inbox-batch-add ()
  "Batch entry point: append drafts from the Org file named on the command line.
Run Emacs in batch mode with this package and keymap-popup on the load
path, load `gnosis-inbox', then call this function with DRAFTS.org as the
only remaining command-line argument.  The environment variable
GNOSIS_DIR must name the Gnosis directory: an unattended producer never
falls back to `gnosis-dir'.  The resolved inbox path is always reported.

Print one JSON object.  On success it has \"inbox\" and \"results\", and
the exit status is 0 when no draft is invalid, 1 otherwise.  On a usage,
read or write failure it has \"inbox\" and \"error\", the exit status is
2 and the inbox is unchanged.  Never opens the database."
  (unless noninteractive (user-error "Use gnosis-inbox-add-file interactively"))
  (let* ((args command-line-args-left)
         (dir (getenv "GNOSIS_DIR"))
         (gnosis-dir (and dir (not (string-empty-p dir)) dir))
         (inbox (and gnosis-dir (gnosis-inbox-file))))
    ;; Consume the arguments so Emacs does not visit them as files.
    (setq command-line-args-left nil)
    (condition-case err
        (progn
          (unless gnosis-dir
            (user-error "Set GNOSIS_DIR to the Gnosis directory to add drafts to"))
          (unless (= (length args) 1)
            (user-error "Usage: Emacs --batch ... -f gnosis-inbox-batch-add DRAFTS.org"))
          (let* ((results (gnosis-inbox-add-file (expand-file-name (car args))))
                 (invalid (cl-find 'invalid results :key (lambda (r) (plist-get r :status)))))
            (gnosis-inbox--batch-print
             (cons 'inbox inbox)
             (cons 'results
                   (vconcat
                    (mapcar (lambda (r)
                              (list (cons 'key (plist-get r :key))
                                    (cons 'status (symbol-name (plist-get r :status)))
                                    (cons 'error (plist-get r :error))))
                            results))))
            (kill-emacs (if invalid 1 0))))
      (error
       (gnosis-inbox--batch-print (cons 'inbox inbox)
                                  (cons 'error (error-message-string err)))
       (kill-emacs 2)))))

(defun gnosis-inbox--revision (id)
  "Return the content revision of thema ID, as `gnosis-agent-content' does."
  (secure-hash 'sha256 (gnosis-sqlite--serialize
                        (gnosis--draft-content (gnosis--ensure-db) id))))

(cl-defun gnosis-inbox-propose-rework (target drafts &key (kind "rework-replace")
                                              agent reason)
  "Queue DRAFTS that replace, split or annotate thema TARGET (leech rework).
KIND is \"rework-replace\", \"rework-split\" or \"rework-annotate\".
AGENT and REASON are provenance.  A draft's :target-revision is the
content revision it was written against; a draft without one records the
target's current revision.  Signal `user-error', writing nothing, if any
draft's revision differs from the current one: provenance is never
rewritten.  Accepting refuses if the target changed since, then in one
transaction saves the drafts and suspends TARGET, or, for an annotation,
appends the draft's explanation to TARGET and keeps its schedule.
Return `gnosis-inbox-add' results."
  (unless (gnosis-get 'id 'themata `(= id ,target))
    (user-error "No thema %s" target))
  (let ((revision (gnosis-inbox--revision target)))
    (dolist (draft drafts)
      (when-let* ((seen (plist-get draft :target-revision)))
        (unless (equal seen revision)
          (user-error "Rework target #%s changed since the draft was written; nothing queued"
                      target))))
    (gnosis-inbox-add
     (cl-loop for draft in drafts
              for n from 1
              collect (append (list :kind kind :source (format "thema:%d" target)
                                    :target target
                                    :target-revision (or (plist-get draft :target-revision)
                                                         revision)
                                    :agent agent :reason reason
                                    :span (or (plist-get draft :span)
                                              (format "%s %d" kind n)))
                              draft)))))

;;; Approval

(defun gnosis-inbox--citation (entry)
  "Return the lecture citation link for ENTRY, or nil."
  (let ((source (plist-get entry :source)))
    (when (and (equal (plist-get entry :kind) "lecture") (stringp source))
      (gnosis-lecture--citation source (plist-get entry :page)))))

(defun gnosis-inbox--effective-thema (entry)
  "Return ENTRY's thema as it will be saved, with its source cited.
Append a lecture citation to Parathema unless it already links SOURCE."
  (let* ((thema (copy-sequence (plist-get entry :thema)))
         (parathema (or (nth 5 thema) ""))
         (citation (gnosis-inbox--citation entry)))
    (when (and citation
               (not (string-match-p
                     (regexp-quote (url-hexify-string (plist-get entry :source)))
                     parathema)))
      (setf (nth 5 thema)
            (concat (string-trim-right parathema)
                    (if (string-empty-p (string-trim parathema)) "" "\n\n")
                    "- Source: " citation)))
    thema))

(defun gnosis-inbox--candidates (thema)
  "Return IDs of stored themata with THEMA's type, question and answer."
  (let* ((type (downcase (nth 1 thema)))
         (keimenon (if (member type '("cloze" "mc-cloze"))
                       (gnosis-cloze-remove-tags (nth 2 thema))
                     (nth 2 thema))))
    (cl-loop for (id answer) in (gnosis-select '[id answer] 'themata
                                               `(and (= type ,type) (= keimenon ,keimenon)))
             when (equal answer (nth 4 thema)) collect id)))

(defun gnosis-inbox--stores-p (id thema)
  "Return non-nil if every field of THEMA is stored in thema ID.
Compare choices or hints, answer, aliases, rubric, explanation and tags;
type and question are compared by `gnosis-inbox--candidates'."
  (pcase-let ((`(,hypothesis ,answer ,aliases ,rubric)
               (car (gnosis-select '[hypothesis answer accepted-aliases rubric]
                                   'themata `(= id ,id))))
              (sorted (lambda (tags) (sort (copy-sequence tags) #'string<))))
    (and (equal (remove "" hypothesis) (remove "" (nth 3 thema)))
         (equal answer (nth 4 thema))
         (equal aliases (nth 8 thema))
         (equal (or rubric "") (or (nth 9 thema) ""))
         (equal (string-trim (or (gnosis-get 'parathema 'extras `(= id ,id)) ""))
                (string-trim (or (nth 5 thema) "")))
         (equal (funcall sorted (gnosis-get-tags-for-ids (list id)))
                (funcall sorted (nth 6 thema))))))

(defun gnosis-inbox--existing-id (thema)
  "Return the ID of a stored thema with exactly THEMA's content.
This recognizes an approval whose database commit succeeded but whose
inbox update did not, so accepting again only closes the entry."
  (seq-find (lambda (id) (gnosis-inbox--stores-p id thema))
            (gnosis-inbox--candidates thema)))

(defun gnosis-inbox--similar-id (thema)
  "Return the ID of a stored thema sharing only THEMA's question and answer."
  (car (gnosis-inbox--candidates thema)))

(defun gnosis-inbox--entry-existing-id (entry thema)
  "Return the ID already storing ENTRY's effective THEMA, or nil.
An annotation edits its target only, so only that target, holding
exactly the annotated content, counts: another card with the same text
cannot satisfy it.  Other entries use `gnosis-inbox--existing-id'."
  (if (equal (plist-get entry :kind) "rework-annotate")
      (let ((target (plist-get entry :target)))
        (and (memql target (gnosis-inbox--candidates thema))
             (gnosis-inbox--stores-p target thema)
             target))
    (gnosis-inbox--existing-id thema)))

(defun gnosis-inbox--annotation (target thema)
  "Return THEMA as an edit of TARGET limited to its explanation.
Signal `user-error' if THEMA changes any other field of TARGET, or does
not extend TARGET's explanation."
  (pcase-let* ((`(,type ,keimenon ,hypothesis ,answer ,aliases ,rubric)
                (car (gnosis-select '[type keimenon hypothesis answer accepted-aliases rubric]
                                    'themata `(= id ,target))))
               (draft-type (downcase (nth 1 thema)))
               (old (string-trim (or (gnosis-get 'parathema 'extras `(= id ,target)) "")))
               (new (string-trim (or (nth 5 thema) "")))
               (sorted (lambda (tags) (sort (copy-sequence tags) #'string<))))
    (unless (and (equal draft-type (downcase type))
                 (equal (if (member draft-type '("cloze" "mc-cloze"))
                            (gnosis-cloze-remove-tags (nth 2 thema))
                          (nth 2 thema))
                        keimenon)
                 (equal (remove "" (nth 3 thema)) (remove "" hypothesis))
                 (equal (nth 4 thema) answer)
                 (equal (nth 8 thema) aliases)
                 (equal (or (nth 9 thema) "") (or rubric ""))
                 (equal (funcall sorted (nth 6 thema))
                        (funcall sorted (gnosis-get-tags-for-ids (list target)))))
      (user-error "An annotation may change only the explanation of #%s" target))
    (unless (and (string-prefix-p old new) (> (length new) (length old)))
      (user-error "An annotation must add to the explanation of #%s" target))
    (let ((edit (copy-sequence thema)))
      (setf (nth 0 edit) target)
      edit)))

(defun gnosis-inbox--commit (entry themata &optional validate)
  "Save THEMATA for pending ENTRY in one transaction; return their IDs.
Refuse an entry with a parse or provenance error.  Call VALIDATE, if
non-nil, inside the transaction before any write.  For rework entries,
refuse a changed or deleted target.  Suspend a replaced or split target
in the same transaction; save an annotation into its target instead
\(see `gnosis-inbox--annotation').  Do not run `gnosis-save-hook'."
  (when-let* ((error (plist-get entry :error)))
    (user-error "Invalid draft: %s" error))
  (gnosis-inbox--check-provenance
   (gnosis-inbox--thema-draft
    (or (plist-get entry :thema) (user-error "Not a pending draft"))
    entry))
  (unless themata (user-error "Draft holds no thema"))
  (let ((target (plist-get entry :target))
        (annotate (equal (plist-get entry :kind) "rework-annotate"))
        ids)
    (when (and annotate (cdr themata))
      (user-error "An annotation holds exactly one thema"))
    (gnosis-sqlite-with-transaction (gnosis--ensure-db)
      (when validate (funcall validate))
      (when target
        (unless (gnosis-get 'id 'themata `(= id ,target))
          (user-error "Rework target #%s no longer exists" target))
        (unless (equal (gnosis-inbox--revision target)
                       (plist-get entry :target-revision))
          (user-error "Rework target #%s changed since the proposal; reject it" target)))
      (dolist (thema themata)
        (let ((result (gnosis-save-thema
                       (if annotate (gnosis-inbox--annotation target thema) thema) t)))
          (when (stringp result) (user-error "%s" result))
          (setq ids (append ids result))))
      (when (and target (not annotate))
        (gnosis-toggle-suspend-themata (list target) 1 t)))
    ids))

(defun gnosis-inbox-apply (entry &optional thema)
  "Save pending ENTRY through the native writer; return the saved IDs.
THEMA overrides the entry's effective thema.  Refuse an entry carrying a
parse or provenance error.  For rework entries, refuse a changed target
and suspend it in the same transaction.  Signal `user-error' with the
native validation message on failure; the database is then unchanged."
  (let ((ids (gnosis-inbox--commit
              entry (list (or thema (and (plist-get entry :thema)
                                         (gnosis-inbox--effective-thema entry)))))))
    (dolist (id ids) (run-hook-with-args 'gnosis-save-hook id))
    ids))

;;; Review UI

(defface gnosis-inbox-dim '((t :inherit shadow))
  "Secondary inbox text: provenance, parathema, hints.")

(defface gnosis-inbox-kind '((t :inherit shadow :height 0.9))
  "Thema kind label above the question.")

(defface gnosis-inbox-answer '((t :inherit gnosis-face-correct :weight bold))
  "Answers inside the draft preview.")

(defface gnosis-inbox-key '((t :inherit help-key-binding))
  "Key hints in the inbox header line.")

(defface gnosis-inbox-warning '((t :inherit warning))
  "Duplicate and invalid-draft notes.")

(defface gnosis-inbox-added '((t :inherit default))
  "Explanation text an annotation adds to its target.
Plain text, distinct from the dimmed explanation it extends.")

(defvar-local gnosis-inbox--entries nil
  "Entries of the inbox file as last read.")

(defvar-local gnosis-inbox--file-hash nil
  "Hash of the inbox file contents `gnosis-inbox--entries' came from.")

(defvar-local gnosis-inbox--file nil
  "Inbox file shown in this buffer.")

(defvar-local gnosis-inbox--db nil
  "Database connection the inbox approves into.")

(defvar-local gnosis-inbox--current nil
  "Key of the entry being shown.")

(defvar-local gnosis-inbox--tally nil
  "Session counts as an alist of (STATE . N).")

(defvar-local gnosis-inbox--last nil
  "Short receipt of the last action, shown under the card.")

(defvar-local gnosis-inbox--occurrence nil
  "Fresh object identifying this opening of the inbox view.
Reopening the inbox replaces it, retiring commands captured before.
File or mode reassociation clears it for good.")

(defvar-local gnosis-inbox--retired nil
  "Non-nil once this buffer stopped being an inbox view.")

(defun gnosis-inbox--retire ()
  "Retire this inbox view; detaching or re-entering the mode never revives it."
  (setq gnosis-inbox--occurrence nil
        gnosis-inbox--db nil
        gnosis-inbox--retired t))

(defun gnosis-inbox--live-p (&optional occurrence)
  "Return non-nil if the current buffer is a live inbox view.
With OCCURRENCE, it must also be that opening of the view."
  (and (derived-mode-p 'gnosis-inbox-mode)
       (not gnosis-inbox--retired)
       (not buffer-file-name)
       gnosis-inbox--occurrence
       (or (null occurrence) (eq occurrence gnosis-inbox--occurrence))))

(defun gnosis-inbox--require-view ()
  "Signal `user-error' unless the current buffer is a live inbox view."
  (unless (gnosis-inbox--live-p)
    (user-error "This buffer is no longer an inbox view; open M-x gnosis-inbox")))

(defun gnosis-inbox--pending ()
  "Return pending entries in file order."
  (seq-filter (lambda (e) (eq (plist-get e :state) 'pending)) gnosis-inbox--entries))

(defun gnosis-inbox--entry ()
  "Return the current pending entry, or nil."
  (seq-find (lambda (e) (equal (plist-get e :key) gnosis-inbox--current))
            (gnosis-inbox--pending)))

(defun gnosis-inbox--load ()
  "Read the inbox file into this buffer's state."
  (let ((text (gnosis-inbox--read gnosis-inbox--file)))
    (setq gnosis-inbox--entries (gnosis-inbox-parse text)
          gnosis-inbox--file-hash (gnosis-inbox--hash text))
    (unless (gnosis-inbox--entry)
      (setq gnosis-inbox--current (plist-get (car (gnosis-inbox--pending)) :key)))))

(defun gnosis-inbox--check ()
  "Return the current entry if acting on it is still valid.
Reload instead and signal if the file changed since it was shown."
  (gnosis-inbox--require-view)
  (unless (eq gnosis-inbox--db gnosis-db)
    (user-error "Database changed since the inbox opened; reopen it"))
  (unless (equal gnosis-inbox--file-hash
                 (gnosis-inbox--hash (gnosis-inbox--read gnosis-inbox--file)))
    (gnosis-inbox--load)
    (gnosis-inbox--render)
    (user-error "Inbox changed on disk; reloaded, check the card and retry"))
  (or (gnosis-inbox--entry) (user-error "No pending draft")))

(defun gnosis-inbox--owner ()
  "Return the identity of this inbox view and its current entry.
The result is (BUFFER OCCURRENCE DATABASE FILE HASH KEY)."
  (list (current-buffer) gnosis-inbox--occurrence gnosis-inbox--db
        gnosis-inbox--file gnosis-inbox--file-hash gnosis-inbox--current))

(defun gnosis-inbox--owner-p (owner)
  "Return non-nil if OWNER, from `gnosis-inbox--owner', is still current.
The same opening of the same buffer must show the same entry of the same
file contents, approving into the current `gnosis-db'."
  (pcase-let ((`(,buffer ,occurrence ,db ,file ,hash ,key) owner))
    (and (buffer-live-p buffer)
         (with-current-buffer buffer
           (and (gnosis-inbox--live-p occurrence)
                (eq gnosis-inbox--db db) (eq gnosis-db db)
                (equal gnosis-inbox--file file)
                (equal gnosis-inbox--file-hash hash)
                (equal gnosis-inbox--current key)
                (equal hash (gnosis-inbox--hash (gnosis-inbox--read file))))))))

(defun gnosis-inbox--title (entry)
  "Return a short label for ENTRY's question, for receipts."
  (let ((thema (plist-get entry :thema)))
    (truncate-string-to-width
     (string-trim (replace-regexp-in-string
                   "[ \t\n]+" " "
                   (gnosis-inbox--strip-links
                    (if thema
                        (gnosis-cloze-remove-tags (nth 2 thema))
                      (or (plist-get entry :title) "")))))
     48 nil nil "…")))

(defun gnosis-inbox--close (entry state &optional ids extra)
  "Replace ENTRY with a STATE ledger heading for IDS; refresh the view.
EXTRA is a plist of additional ledger properties."
  (let ((next (gnosis-inbox--neighbour 1)))
    (gnosis-inbox--replace-entry gnosis-inbox--file gnosis-inbox--file-hash entry
                                 (gnosis-inbox--ledger-text entry state ids extra))
    (setf (alist-get state gnosis-inbox--tally) (1+ (alist-get state gnosis-inbox--tally 0)))
    (setq gnosis-inbox--current (and next (not (equal next (plist-get entry :key))) next))
    (gnosis-inbox--load)
    (gnosis-inbox--render)))

(defun gnosis-inbox--neighbour (step)
  "Return the key STEP pending entries away from the current one, wrapping."
  (let* ((keys (mapcar (lambda (e) (plist-get e :key)) (gnosis-inbox--pending)))
         (n (length keys))
         (i (or (cl-position gnosis-inbox--current keys :test #'equal) 0)))
    (when (> n 0) (nth (mod (+ i step) n) keys))))

(defun gnosis-inbox-accept ()
  "Save the current draft through the native writer and show the next one."
  (interactive nil gnosis-inbox-mode)
  (let* ((entry (gnosis-inbox--check))
         (_ (unless (plist-get entry :thema)
              (user-error "Invalid draft: %s; k to reject" (plist-get entry :error))))
         (thema (gnosis-inbox--effective-thema entry))
         (existing (gnosis-inbox--entry-existing-id entry thema)))
    (if existing
        (progn
          (setq gnosis-inbox--last (format "Already saved as #%s · nothing added: “%s”"
                                           existing (gnosis-inbox--title entry)))
          (gnosis-inbox--close entry 'duplicate (list existing)))
      (let ((ids (gnosis-inbox--commit entry (list thema))))
        (setq gnosis-inbox--last
              (format "Saved #%s: “%s”" (mapconcat #'number-to-string ids ", #")
                      (gnosis-inbox--title entry)))
        (unwind-protect (gnosis-inbox--close entry 'applied ids)
          (dolist (id ids) (run-hook-with-args 'gnosis-save-hook id)))))))

(defun gnosis-inbox-reject (&optional reason)
  "Reject the current draft, recording REASON if it is a string.
Interactively, a prefix argument asks for the reason.  The inbox view,
its entry and database are captured before asking and must be unchanged
afterwards, or nothing is rejected."
  (interactive "P" gnosis-inbox-mode)
  (let* ((entry (gnosis-inbox--check))
         (owner (gnosis-inbox--owner))
         (reason (if (and reason (not (stringp reason)))
                     (read-string "Reason: ")
                   reason)))
    (unless (gnosis-inbox--owner-p owner)
      (user-error "Inbox, draft or database changed while asking; nothing rejected"))
    (with-current-buffer (car owner)
      (setq gnosis-inbox--last (format "Rejected “%s”" (gnosis-inbox--title entry)))
      (gnosis-inbox--close entry 'rejected nil
                           (list "GNOSIS_REJECT_REASON" reason)))))

(defun gnosis-inbox-skip ()
  "Leave the current draft pending and show the next one."
  (interactive nil gnosis-inbox-mode)
  (gnosis-inbox--require-view)
  (setq gnosis-inbox--current (gnosis-inbox--neighbour 1)
        gnosis-inbox--last nil)
  (gnosis-inbox--render))

(defun gnosis-inbox-previous ()
  "Show the previous pending draft."
  (interactive nil gnosis-inbox-mode)
  (gnosis-inbox--require-view)
  (setq gnosis-inbox--current (gnosis-inbox--neighbour -1)
        gnosis-inbox--last nil)
  (gnosis-inbox--render))

(defun gnosis-inbox-reload ()
  "Re-read the inbox file."
  (interactive nil gnosis-inbox-mode)
  (gnosis-inbox--require-view)
  (gnosis-inbox--load)
  (gnosis-inbox--render))

(defun gnosis-inbox-view-source ()
  "Open the current draft's lecture source at its page in another window."
  (interactive nil gnosis-inbox-mode)
  (gnosis-inbox--require-view)
  (let* ((entry (or (gnosis-inbox--entry) (user-error "No pending draft")))
         (source (plist-get entry :source)))
    (cond
     ((equal (plist-get entry :kind) "lecture")
      (select-window (or (window-in-direction 'right) (split-window-right)))
      (gnosis-lecture-open (format "%d:%s" (or (plist-get entry :page) 0)
                                   (url-hexify-string source))))
     ((plist-get entry :target)
      (gnosis-edit-thema (plist-get entry :target)))
     (t (user-error "No viewable source")))))

(defvar-local gnosis-inbox--draft-entry nil
  "Inbox entry plist a native draft buffer was opened from.")

(defvar-local gnosis-inbox--draft-owner nil
  "Inbox view identity, from `gnosis-inbox--owner', when editing began.")

(defvar-keymap gnosis-inbox-draft-map
  :doc "Keys of a native draft opened from the inbox."
  :parent gnosis-edit-mode-map
  "C-c C-c" #'gnosis-inbox-draft-save)

(defun gnosis-inbox-edit ()
  "Open the current draft in the native thema editor.
\\<gnosis-inbox-draft-map>\\[gnosis-inbox-draft-save] approves the edited text and closes the entry;
\\[gnosis-edit-quit] leaves it pending.  Approval applies the same
checks as \\<gnosis-inbox-mode-map>\\[gnosis-inbox-accept], and refuses if the entry, the inbox
view or the database changed while editing."
  (interactive nil gnosis-inbox-mode)
  (let* ((entry (gnosis-inbox--check))
         (thema (gnosis-inbox--effective-thema
                 (if (plist-get entry :thema) entry
                   (user-error "Invalid draft: %s; k to reject"
                               (plist-get entry :error)))))
         (inbox (gnosis-inbox--owner))
         (owner (gnosis--ensure-db))
         (name "*Gnosis Inbox Draft*"))
    (when (get-buffer name)
      (user-error "Finish or cancel the open inbox draft first"))
    (window-configuration-to-register :gnosis-edit)
    (pop-to-buffer (get-buffer-create name))
    (gnosis-edit-mode)
    (use-local-map gnosis-inbox-draft-map)
    ;; Long questions and explanations wrap in this editor only.
    (setq-local truncate-lines nil)
    (visual-line-mode 1)
    (setq gnosis--draft-db owner
          gnosis-inbox--draft-entry entry
          gnosis-inbox--draft-owner inbox)
    (let* ((draft (gnosis-inbox--thema-draft thema nil))
           (type (plist-get draft :type)))
      (gnosis-export--insert-thema
       "NEW" type (plist-get draft :keimenon)
       (gnosis-inbox--list-text (plist-get draft :hypothesis))
       (if (equal type "agent-eval") (car (plist-get draft :answer))
         (gnosis-inbox--list-text (plist-get draft :answer)))
       (plist-get draft :parathema) (plist-get draft :tags) nil
       (plist-get draft :aliases) (plist-get draft :rubric)))
    (setq header-line-format
          (substitute-command-keys
           " Inbox draft · \\<gnosis-inbox-draft-map>\\[gnosis-inbox-draft-save] approve edited  \\[gnosis-edit-quit] back, keep pending"))
    (goto-char (point-min))
    (search-forward "** Keimenon")
    (forward-line)))

(defun gnosis-inbox--draft-entry-check (owner entry)
  "Signal `user-error' unless ENTRY is still pending unchanged for OWNER.
OWNER is the inbox view identity captured when editing began.  The view
must be the same opening on the same database and file, and the file
must still hold ENTRY's exact text under its key; drafts appended
elsewhere in the file are allowed.  Return the current entry."
  (pcase-let ((`(,buffer ,occurrence ,db ,file . ,_) owner))
    (unless (and (buffer-live-p buffer)
                 (with-current-buffer buffer
                   (and (gnosis-inbox--live-p occurrence)
                        (eq gnosis-inbox--db db)
                        (equal gnosis-inbox--file file)))
                 (eq gnosis-db db))
      (user-error "Inbox reopened, closed or reused; copy the draft text, cancel and reopen"))
    (or (seq-find (lambda (current)
                    (and (eq (plist-get current :state) 'pending)
                         (equal (plist-get current :key) (plist-get entry :key))
                         (equal (plist-get current :text) (plist-get entry :text))))
                  (gnosis-inbox-parse (gnosis-inbox--read file)))
        (user-error
         "Inbox entry changed or was closed; copy the draft text, cancel and reopen"))))

(defun gnosis-inbox--settle (owner entry state ids extra)
  "Close ENTRY with STATE for IDS in OWNER's file after a committed save.
EXTRA is a plist of ledger properties.  Locate ENTRY again by exact text,
so drafts appended meanwhile survive; leave the file unchanged if it is
gone.  The ledger records the committed save even if OWNER's view
retired meanwhile, but only a still-live opening is refreshed: a
reused buffer keeps its text.  Return non-nil if the entry was closed."
  (pcase-let ((`(,buffer ,occurrence ,_ ,file . ,_) owner))
    (let* ((text (gnosis-inbox--read file))
           (current (seq-find (lambda (e)
                                (and (eq (plist-get e :state) 'pending)
                                     (equal (plist-get e :text) (plist-get entry :text))))
                              (gnosis-inbox-parse text))))
      (prog1 (and current
                  (gnosis-inbox--replace-entry
                   file (gnosis-inbox--hash text) current
                   (gnosis-inbox--ledger-text current state ids extra))
                  t)
        (when (buffer-live-p buffer)
          (with-current-buffer buffer
            (when (gnosis-inbox--live-p occurrence)
              (setf (alist-get state gnosis-inbox--tally)
                    (1+ (alist-get state gnosis-inbox--tally 0)))
              (gnosis-inbox--load)
              (gnosis-inbox--render))))))))

(defun gnosis-inbox-draft-save ()
  "Approve this edited inbox draft and close its inbox entry.
Validate like `gnosis-save' and approve like `gnosis-inbox-accept': in
one transaction, refuse a changed draft owner, inbox entry or rework
target, save every thema and suspend a rework target.  On refusal the
draft text, the entry and the database are unchanged."
  (interactive nil gnosis-edit-mode)
  (gnosis--draft-check-owner)
  (let* ((entry gnosis-inbox--draft-entry)
         (owner gnosis-inbox--draft-owner)
         (_ (unless (and entry owner)
              (user-error "Not an inbox draft")))
         (themata (save-restriction (widen) (gnosis-export-parse-themata)))
         (validate (lambda ()
                     (gnosis--draft-validate themata)
                     (gnosis-inbox--draft-entry-check owner entry)))
         (ids nil))
    (funcall validate)
    ;; Settle a committed occurrence before quit delivery, as `gnosis-save'.
    (let ((inhibit-quit t))
      (setq ids (let ((inhibit-quit nil))
                  (gnosis-inbox--commit (gnosis-inbox--draft-entry-check owner entry)
                                        themata validate)))
      (setq gnosis--draft-saved-p t))
    (unwind-protect
        (let ((closed (condition-case err
                          (gnosis-inbox--settle
                           owner entry 'applied ids '("GNOSIS_EDITED" "t"))
                        (error (message "Saved #%s; inbox not updated: %s"
                                        (mapconcat #'number-to-string ids ", #")
                                        (error-message-string err))
                               nil))))
          (when (and (buffer-live-p (car owner))
                     (with-current-buffer (car owner)
                       (gnosis-inbox--live-p (cadr owner))))
            (with-current-buffer (car owner)
              (setq gnosis-inbox--last
                    (format "Saved edited #%s: “%s”%s"
                            (mapconcat #'number-to-string ids ", #")
                            (gnosis-inbox--title entry)
                            (if closed "" " · inbox entry left pending")))
              (gnosis-inbox--render)))
          (gnosis-edit-quit))
      (dolist (id (seq-uniq ids))
        (run-hook-with-args 'gnosis-save-hook id)))))

;;;; Rendering

(defun gnosis-inbox--strip-links (text)
  "Replace Org links in TEXT with their descriptions."
  (replace-regexp-in-string "\\[\\[\\([^]]+\\)\\]\\(?:\\[\\([^]]+\\)\\]\\)?\\]"
                            (lambda (m) (or (match-string 2 m) (match-string 1 m)))
                            (or text "") t t))

(defun gnosis-inbox--fill (text width &optional face)
  "Return TEXT filled to WIDTH, propertized with FACE."
  (with-temp-buffer
    (insert text)
    (let ((fill-column width))
      (fill-region (point-min) (point-max)))
    (let ((out (buffer-string)))
      (if face (propertize out 'face face) out))))

(defun gnosis-inbox--keimenon (thema width)
  "Return THEMA's question filled to WIDTH, cloze answers marked."
  (let* ((type (downcase (nth 1 thema)))
         (text (gnosis-inbox--strip-links
                (if (member type '("cloze" "mc-cloze"))
                    (gnosis-cloze-remove-tags (nth 2 thema))
                  (nth 2 thema))))
         (filled (gnosis-inbox--fill text width)))
    (when (member type '("cloze" "mc-cloze"))
      (dolist (span (gnosis-cloze--spans filled (nth 4 thema)))
        (when span
          (add-face-text-property (car span) (cdr span) 'gnosis-inbox-answer nil filled))))
    filled))

(defun gnosis-inbox--body (thema width)
  "Return the answer block of THEMA for WIDTH columns."
  (let* ((type (downcase (nth 1 thema)))
         (hints (remove "" (nth 3 thema)))
         (answers (nth 4 thema))
         (label (lambda (s) (propertize (format "%-9s" s) 'face 'gnosis-inbox-dim))))
    (concat
     (pcase type
       ((or "mcq" "mc-cloze")
        (mapconcat (lambda (choice)
                     (if (member choice answers)
                         (propertize (concat "●  " choice) 'face 'gnosis-inbox-answer)
                       (propertize (concat "○  " choice) 'face 'gnosis-inbox-dim)))
                   hints "\n"))
       ("cloze"
        (concat (funcall label "answer")
                (propertize (mapconcat #'identity answers "  ·  ") 'face 'gnosis-inbox-answer)
                (if hints (concat "\n" (funcall label "hints")
                                  (propertize (mapconcat #'identity hints "  ·  ")
                                              'face 'gnosis-inbox-dim))
                  "")))
       ("agent-eval"
        (concat (funcall label "reference") "\n"
                (gnosis-inbox--fill (car answers) width 'gnosis-inbox-answer)
                "\n\n" (funcall label "rubric") "\n"
                (gnosis-inbox--fill (nth 9 thema) width 'gnosis-inbox-dim)))
       (_
        (concat (funcall label "answer")
                (propertize (car answers) 'face 'gnosis-inbox-answer)
                (if (nth 8 thema)
                    (concat "\n" (funcall label "also")
                            (propertize (mapconcat #'identity (nth 8 thema) "  ·  ")
                                        'face 'gnosis-inbox-dim))
                  "")
                (if hints (concat "\n" (funcall label "hint")
                                  (propertize (car hints) 'face 'gnosis-inbox-dim))
                  "")))))))

(defun gnosis-inbox--leech-evidence (target width)
  "Return TARGET's lapse history and recent wrong answers, or nil.
Fill to WIDTH columns."
  (require 'gnosis-leech)
  (when-let* ((state (gnosis-leech-state target)))
    (let ((wrong (mapcar (lambda (w) (format "“%s”" (plist-get w :response)))
                         (gnosis-leech-recent-wrong target 3))))
      (let ((text (gnosis-inbox--fill
                   (concat (format "⚑ leech · %d lapses in %d reviews"
                                   (plist-get state :lapses) (plist-get state :reps))
                           (if wrong (concat " · answered " (mapconcat #'identity wrong " "))
                             ""))
                   width 'gnosis-inbox-dim)))
        (add-face-text-property 0 (length "⚑ leech") 'gnosis-inbox-warning nil text)
        text))))

(defun gnosis-inbox--provenance (entry)
  "Return ENTRY's compact provenance lines."
  (let* ((source (plist-get entry :source))
         (where (cond ((plist-get entry :target)
                       (format "rework #%s · %s" (plist-get entry :target)
                               (plist-get entry :kind)))
                      (source (format "%s · p.%s" (file-name-nondirectory source)
                                      (plist-get entry :page)))
                      (t "no source"))))
    (concat (propertize (concat "⌁ " where) 'face 'gnosis-inbox-dim)
            (propertize (format "  ·  %s" (or (plist-get entry :agent) "?"))
                        'face 'gnosis-inbox-dim)
            (if (plist-get entry :reason)
                (concat "\n" (propertize (format "  “%s”" (plist-get entry :reason))
                                         'face '(gnosis-inbox-dim italic)))
              ""))))

(defun gnosis-inbox--key-hint (key label)
  "Return header text for KEY and LABEL."
  (concat (propertize key 'face 'gnosis-inbox-key) " " label))

(defun gnosis-inbox--header ()
  "Return the header line."
  (let* ((pending (gnosis-inbox--pending))
         (i (cl-position gnosis-inbox--current pending
                         :key (lambda (e) (plist-get e :key)) :test #'equal)))
    (concat " "
            (propertize "Inbox" 'face 'bold)
            (if pending (format "  %d/%d" (1+ (or i 0)) (length pending)) "  empty")
            "    "
            (mapconcat (lambda (pair) (gnosis-inbox--key-hint (car pair) (cdr pair)))
                       (if pending
                           '(("a" . "accept") ("e" . "edit") ("k" . "reject")
                             ("s" . "skip") ("p" . "back") ("v" . "source") ("q" . "quit"))
                         '(("g" . "reload") ("q" . "quit")))
                       "   "))))

(defun gnosis-inbox--render ()
  "Render the current draft, centered in the selected window."
  (let* ((window (or (get-buffer-window (current-buffer)) (selected-window)))
         (total (window-body-width window))
         (width (max 30 (min 76 (- total 8))))
         (pad (make-string (max 0 (/ (- total width) 2)) ?\s))
         (entry (gnosis-inbox--entry))
         (inhibit-read-only t))
    (erase-buffer)
    (setq header-line-format (gnosis-inbox--header))
    (insert "\n\n\n")
    (if (null entry)
        (insert (propertize "Inbox empty" 'face 'bold) "\n\n"
                (propertize
                 (if gnosis-inbox--tally
                     (mapconcat (lambda (state)
                                  (format "%d %s" (alist-get state gnosis-inbox--tally) state))
                                (seq-filter (lambda (state) (alist-get state gnosis-inbox--tally))
                                            '(applied duplicate rejected))
                                "  ·  ")
                   "No drafts waiting.  Agents add drafts with gnosis-inbox-add.")
                 'face 'gnosis-inbox-dim)
                "\n")
      (let* ((thema (plist-get entry :thema))
             (annotate (equal (plist-get entry :kind) "rework-annotate"))
             (effective (and thema (gnosis-inbox--effective-thema entry)))
             (existing (and thema (gnosis-inbox--entry-existing-id entry effective)))
             (similar (and thema (not existing) (gnosis-inbox--similar-id effective))))
        ;; A rework naturally shares its target's question.
        (when (eql similar (plist-get entry :target)) (setq similar nil))
        (insert (propertize (upcase (if thema (downcase (nth 1 thema)) "invalid"))
                            'face 'gnosis-inbox-kind)
                (propertize (format "  ·  %s" (plist-get entry :kind)) 'face 'gnosis-inbox-kind)
                (if (and thema (nth 6 thema))
                    (propertize (format "  ·  %s" (mapconcat #'identity (nth 6 thema) " "))
                                'face 'gnosis-inbox-kind)
                  "")
                "\n\n")
        (if (not thema)
            (insert (propertize (format "Invalid draft: %s" (plist-get entry :error))
                                'face 'gnosis-inbox-warning)
                    "\n" (propertize "k rejects it; fix the agent output and re-add."
                                     'face 'gnosis-inbox-dim) "\n")
          (insert (gnosis-inbox--keimenon thema width) "\n\n"
                  (gnosis-inbox--body thema width) "\n")
          (when-let* ((target (plist-get entry :target))
                      (old (gnosis-get 'keimenon 'themata `(= id ,target))))
            (insert "\n" (propertize (if annotate
                                         (format "adds to #%s's explanation · schedule kept"
                                                 target)
                                       (format "replaces #%s (suspended on accept)" target))
                                     'face 'gnosis-inbox-dim)
                    "\n")
            (unless annotate
              (insert (gnosis-inbox--fill (gnosis-inbox--strip-links old) width
                                          '(gnosis-inbox-dim italic))
                      "\n"))
            (when-let* ((evidence (gnosis-inbox--leech-evidence target width)))
              (insert evidence "\n")))
          (let* ((parathema (string-trim (or (nth 5 thema) "")))
                 (before (and annotate
                              (string-trim (or (gnosis-get 'parathema 'extras
                                                           `(= id ,(plist-get entry :target)))
                                               ""))))
                 (added (and before (string-prefix-p before parathema)
                             (string-trim (substring parathema (length before))))))
            (unless (string-empty-p parathema)
              (insert "\n")
              (if (not added)
                  (insert (gnosis-inbox--fill (gnosis-inbox--strip-links parathema)
                                              width 'gnosis-inbox-dim)
                          "\n")
                (unless (string-empty-p before)
                  (insert (gnosis-inbox--fill (gnosis-inbox--strip-links before)
                                              width 'gnosis-inbox-dim)
                          "\n\n"))
                (insert (gnosis-inbox--fill (gnosis-inbox--strip-links added)
                                            width 'gnosis-inbox-added)
                        "\n")))))
        (insert "\n" (propertize (make-string (min width 24) ?─) 'face 'gnosis-inbox-dim) "\n"
                (gnosis-inbox--provenance entry) "\n")
        (when existing
          (insert (propertize (format "⚠ already in collection as #%s; a only closes the draft"
                                      existing)
                              'face 'gnosis-inbox-warning)
                  "\n"))
        (when similar
          (insert (propertize (format "⚠ #%s has the same question and answer; a adds this one too"
                                      similar)
                              'face 'gnosis-inbox-warning)
                  "\n"))))
    (when gnosis-inbox--last
      (insert "\n" (propertize (concat "Previous: " gnosis-inbox--last)
                               'face 'gnosis-inbox-dim)
              "\n"))
    (put-text-property (point-min) (point-max) 'line-prefix pad)
    (put-text-property (point-min) (point-max) 'wrap-prefix pad)
    (goto-char (point-min))))

(defun gnosis-inbox--resize (window)
  "Re-render the inbox shown in WINDOW after its size changed."
  (with-current-buffer (window-buffer window)
    (when (gnosis-inbox--live-p)
      (gnosis-inbox--render))))

(defvar-keymap gnosis-inbox-mode-map
  :doc "Keys of the Gnosis inbox."
  "a" #'gnosis-inbox-accept
  "e" #'gnosis-inbox-edit
  "k" #'gnosis-inbox-reject
  "s" #'gnosis-inbox-skip
  "n" #'gnosis-inbox-skip
  "SPC" #'gnosis-inbox-skip
  "p" #'gnosis-inbox-previous
  "v" #'gnosis-inbox-view-source
  "g" #'gnosis-inbox-reload)

(define-derived-mode gnosis-inbox-mode special-mode "Gnosis Inbox"
  "Review agent-drafted themata one at a time.
\\{gnosis-inbox-mode-map}"
  :interactive nil
  (setq-local cursor-type nil
              truncate-lines nil)
  (add-hook 'window-size-change-functions #'gnosis-inbox--resize nil t)
  ;; File association keeps the major mode but ends the view.
  (add-hook 'after-set-visited-file-name-hook #'gnosis-inbox--retire nil t)
  (add-hook 'change-major-mode-hook #'gnosis-inbox--retire nil t))

;;;###autoload
(defun gnosis-inbox ()
  "Review agent drafts waiting in the inbox."
  (interactive)
  (let ((db (gnosis--ensure-db))
        (file (gnosis-inbox-file))
        (buffer (get-buffer "*Gnosis Inbox*")))
    ;; Never repurpose a retired or unrelated buffer of that name.
    (unless (and buffer (with-current-buffer buffer (gnosis-inbox--live-p)))
      (setq buffer (generate-new-buffer "*Gnosis Inbox*")))
    (pop-to-buffer-same-window buffer)
    (gnosis-inbox-mode)
    (setq gnosis-inbox--db db
          gnosis-inbox--occurrence (list 'gnosis-inbox)
          gnosis-inbox--file file
          gnosis-inbox--tally nil
          gnosis-inbox--last nil)
    (gnosis-inbox--load)
    (gnosis-inbox--render)))

;;;###autoload
(defun gnosis-inbox-count ()
  "Return the number of pending inbox drafts, without opening the database."
  (cl-count 'pending (gnosis-inbox-parse (gnosis-inbox--read (gnosis-inbox-file)))
            :key (lambda (e) (plist-get e :state))))

(provide 'gnosis-inbox)
;;; gnosis-inbox.el ends here
