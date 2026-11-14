;;; gnosis-agent-content.el --- Structured content access -*- lexical-binding: t; -*-
;; Maintainer: Thanos Apollo <public@thanosapollo.org>

;; Copyright (C) 2026 Free Software Foundation, Inc.
;; Author: Thanos Apollo <public@thanosapollo.org>
;; Keywords: extensions
;; URL: https://thanosapollo.org/projects/gnosis

;; This program is free software; you can redistribute it and/or modify
;; it under the terms of the GNU General Public License as published by
;; the Free Software Foundation, either version 3 of the License, or
;; (at your option) any later version.
;; This program is distributed in the hope that it will be useful,
;; but WITHOUT ANY WARRANTY; without even the implied warranty of
;; MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE.  See the
;; GNU General Public License for more details.
;; You should have received a copy of the GNU General Public License
;; along with this program.  If not, see <https://www.gnu.org/licenses/>.

;;; Commentary:

;; Optional content companion to `gnosis-agent'.  No model or transport policy.
;; Results are keyword plists, vectors, strings, numbers, t, :false and nil.
;; Use `json-serialize' with :false-object :false and :null-object nil, and
;; `json-parse-string' with :object-type 'plist and the same false/null values.
;; Thema IDs are canonical decimal strings, including in cursors and edits.
;; Reads may open the configured database normally; they never sync Org files.
;; Organization plans are plain values, not retained jobs or content versions.

;;; Code:

(require 'gnosis-agent)
(require 'gnosis-nodes)
(require 'json)
(require 'org-element)

(defvar gnosis-agent-content--owners (make-hash-table :test 'eq :weakness 'key)
  "Process-local identities of exact SQLite connections, not filenames.")

(defun gnosis-agent-content--owner ()
  "Return the connected database's opaque lifetime identity."
  (let ((db (gnosis--ensure-db)))
    (or (gethash db gnosis-agent-content--owners)
        (puthash db (gnosis-scheduler-event-id) gnosis-agent-content--owners))))

(defun gnosis-agent-content--id (id)
  "Validate canonical decimal string ID and return its SQLite integer."
  (unless (and (stringp id) (string-match-p "\\`\\(?:0\\|-?[1-9][0-9]*\\)\\'" id)
               (<= -9223372036854775808 (string-to-number id) 9223372036854775807))
    (user-error "Thema ID must be a canonical decimal string within SQLite range"))
  (string-to-number id))

(defun gnosis-agent-content--strings (values)
  "Validate VALUES as a vector of nonempty strings; return a fresh list."
  (unless (and (vectorp values)
               (seq-every-p (lambda (s) (and (stringp s) (not (string-empty-p s)))) values))
    (user-error "Expected a vector of nonempty strings"))
  (delete-dups (append values nil)))

(defun gnosis-agent-content--revision (id)
  "Return a deterministic revision of ID's native editable content."
  (secure-hash 'sha256
               (gnosis-sqlite--serialize (gnosis--draft-content (gnosis--ensure-db) id))))

(defun gnosis-agent-content--record (id)
  "Read structured content for integer ID, or return a missing receipt."
  (if-let* ((row (car (gnosis-select
                      '[type keimenon hypothesis answer accepted-aliases rubric]
                      'themata `(= id ,id)))))
      (pcase-let* ((`(,type ,question ,hypothesis ,answer ,aliases ,rubric) row)
                   (cloze (member type '("cloze" "mc-cloze")))
                   (rendered (if cloze
                                 (car (gnosis-cloze--render
                                       question answer
                                       (number-sequence 0 (1- (length answer)))
                                       (and (equal type "cloze") hypothesis)))
                               question)))
        (list :id (number-to-string id) :status "found" :type type
              :revision (gnosis-agent-content--revision id)
              :question question :recall-question (substring-no-properties rendered)
              :answers (vconcat answer) :hypothesis (vconcat hypothesis)
              :hints (if (member type '("basic" "cloze" "agent-eval"))
                         (vconcat hypothesis) [])
              :accepted-aliases (vconcat aliases) :rubric rubric
              :parathema (gnosis-get 'parathema 'extras `(= id ,id))
              :review-image (gnosis-get 'review-image 'extras `(= id ,id))
              :tags (vconcat (sort (gnosis-select 'tag 'thema-tag `(= thema-id ,id) t) #'string<))
              :source-ids (vconcat (sort (gnosis-select 'dest 'thema-links `(= source ,id) t) #'string<))
              :authored-source-ids
              (vconcat (gnosis--thema-expected-links
                        question (or (gnosis-get 'parathema 'extras `(= id ,id)) "")))))
    (list :id (number-to-string id) :status "missing")))

(defun gnosis-agent-content-fetch (ids)
  "Return content records for decimal-string IDS, a vector of at most 500.
Preserve request order and duplicates; missing IDs have status \"missing\".
Empty IDS returns an empty :items vector.  Found records contain :id,
:revision, :type, authored :question, initial :recall-question, :answers,
:hypothesis, :hints, :accepted-aliases, :rubric, :parathema, :review-image,
:tags, indexed :source-ids and :authored-source-ids (including missing nodes).
Cloze recall text uses the native blank/hint renderer before Org fontification;
media fields describe stored references, not rendered pixels.  MCQ hypothesis
contains choices; model/image hypothesis retains its native field strings.
The :owner identifies the exact open connection, not just its filename."
  (unless (and (vectorp ids) (<= (length ids) 500))
    (user-error "Provide a vector of at most 500 IDs"))
  (let ((numbers (mapcar #'gnosis-agent-content--id ids)))
    (gnosis-sqlite-with-transaction (gnosis--ensure-db)
      (list :api-version 1 :owner (gnosis-agent-content--owner)
            :items (vconcat (mapcar #'gnosis-agent-content--record numbers))))))

(defun gnosis-agent-content--epoch ()
  "Return the revision following local or external database writes."
  (list :local (number-to-string
                (caar (sqlite-select (gnosis--ensure-db) "SELECT total_changes()")))
        :external (number-to-string
                   (caar (sqlite-select (gnosis--ensure-db) "PRAGMA data_version")))))

(defun gnosis-agent-content--page-start (cursor scope limit)
  "Validate CURSOR for SCOPE and LIMIT; return its last key or nil."
  (unless (and (integerp limit) (<= 1 limit 500))
    (user-error "Page limit must be between 1 and 500"))
  (when cursor
    (unless (and (equal (plist-get cursor :owner) (gnosis-agent-content--owner))
                 (equal (plist-get cursor :epoch) (gnosis-agent-content--epoch))
                 (equal (plist-get cursor :scope) scope))
      (user-error "Cursor is stale or belongs to a different query or database"))
    (plist-get cursor :after)))

(defun gnosis-agent-content--page (items more after scope)
  "Build a page of ITEMS, with MORE continuation AFTER for SCOPE."
  (list :api-version 1 :owner (gnosis-agent-content--owner)
        :items (vconcat items) :complete (if more :false t)
        :next (when more
                (list :owner (gnosis-agent-content--owner)
                      :epoch (gnosis-agent-content--epoch) :scope scope :after after))))

(cl-defun gnosis-agent-content-search (&key (text "") tag source-id (limit 100) cursor)
  "Enumerate content in numeric ID order with literal case-sensitive TEXT.
Match TEXT in authored question, answers or parathema; empty TEXT matches all.
Optional TAG and SOURCE-ID restrict exact indexed associations (AND).
Return fetch-shaped :items, :complete and :next.  Pass :next back as CURSOR
with the same filters.  LIMIT is 1..500; suspended themata remain discoverable.
Any database write, even unrelated study evidence, invalidates a continuation;
restart on that error.  Each call is a consistent read transaction.  Text
search examines at most 128 candidates per call, batching extras with them.
An empty :items page can be incomplete: continue until :complete is t.
LIMIT bounds returned records, not the number of calls required."
  (unless (and (stringp text) (or (null tag) (stringp tag))
               (or (null source-id) (stringp source-id)))
    (user-error "Search filters must be strings"))
  (gnosis-sqlite-with-transaction (gnosis--ensure-db)
    (let* ((scope (list :kind "themata" :text text :tag tag :source-id source-id))
           (start (gnosis-agent-content--page-start cursor scope limit))
           (after (and start (gnosis-agent-content--id start)))
           (case-fold-search nil)
           items done)
      ;; Bound decoded work even for no matches; one spare row proves more.
      (let* ((rows (gnosis-sqlite-select
                    (gnosis--ensure-db)
                    (concat "SELECT t.id, t.keimenon, t.answer, e.parathema FROM themata t LEFT JOIN extras e ON e.id = t.id WHERE "
                            (if after "t.id > ?" "1")
                            (when tag " AND t.id IN (SELECT thema_id FROM thema_tag WHERE tag = ?)")
                            (when source-id " AND t.id IN (SELECT source FROM thema_links WHERE dest = ?)")
                            " ORDER BY t.id LIMIT 129")
                    (append (when after (list after)) (when tag (list tag)) (when source-id (list source-id)))))
             (remaining rows)
             (scanned 0))
        (while (and remaining (< scanned 128) (< (length items) limit))
          (let ((row (pop remaining)))
            (setq after (car row) scanned (1+ scanned))
            (when (or (string-empty-p text)
                      (seq-some (lambda (s) (and (stringp s) (string-search text s)))
                                (append (list (nth 1 row) (nth 3 row)) (nth 2 row))))
              (push (car row) items))))
        (setq done (null remaining)))
      (gnosis-agent-content--page
       (mapcar #'gnosis-agent-content--record (nreverse items)) (not done)
       (and after (number-to-string after)) scope))))

(cl-defun gnosis-agent-content-vocabulary (kind &key (limit 100) cursor)
  "Enumerate existing KIND, either \"tags\" or \"sources\", in storage order.
Return :items, :complete and :next as in `gnosis-agent-content-search'.
Tags are strings.  Sources are indexed node/journal records with :id, :title,
:file and :kind; duplicate node/journal IDs have kind \"ambiguous\".
This is index metadata, not a claim of file availability.
LIMIT and CURSOR follow the same conservative continuation contract."
  (unless (member kind '("tags" "sources")) (user-error "Unknown vocabulary kind"))
  (gnosis-sqlite-with-transaction (gnosis--ensure-db)
    (let* ((scope (list :kind kind))
           (after (gnosis-agent-content--page-start cursor scope limit))
           (rows (gnosis-sqlite-select
                  (gnosis--ensure-db)
                  (if (equal kind "tags")
                      "SELECT DISTINCT tag FROM thema_tag WHERE (? IS NULL OR tag > ?) ORDER BY tag LIMIT ?"
                    "SELECT id, MIN(title), MIN(file),
                            CASE WHEN COUNT(*) > 1 THEN 'ambiguous' ELSE MIN(kind) END
                     FROM (SELECT id, title, file, 'nodes' AS kind FROM nodes
                           UNION ALL SELECT id, title, file, 'journal' AS kind FROM journal)
                     WHERE (? IS NULL OR id > ?) GROUP BY id ORDER BY id LIMIT ?")
                  (list after after (1+ limit))))
           (page (seq-take rows limit)))
      (gnosis-agent-content--page
       (mapcar (lambda (row)
                 (if (equal kind "tags") (car row)
                   (list :id (car row) :title (nth 1 row) :file (nth 2 row)
                         :kind (format "%s" (nth 3 row))))) page)
       (> (length rows) limit) (caar (last page)) scope))))

(defun gnosis-agent-content-source (id)
  "Read indexed source ID's saved Org subtree without visiting or syncing it.
Return :status \"found\" and :text, or \"missing\", \"ambiguous\",
\"unavailable\", \"dirty\" or \"stale\" with no text.  A changed whole-file hash
or absent ID is stale.  Root IDs return the full file.  Dirty visiting buffers
are never disclosed or saved; clean visiting buffers are ignored in favor of
disk.  This reads local indexed sources only; remote/encrypted paths refuse."
  (unless (and (stringp id) (not (string-empty-p id))) (user-error "Source ID must be a string"))
  (let* ((rows (append
                (mapcar (lambda (r) (cons 'nodes r))
                        (gnosis-select '[file hash] 'nodes `(= id ,id)))
                (mapcar (lambda (r) (cons 'journal r))
                        (gnosis-select '[file hash] 'journal `(= id ,id)))))
         (row (car rows))
         (file (and row (expand-file-name
                        (nth 1 row) (if (eq (car row) 'nodes) gnosis-nodes-dir
                                      (gnosis-nodes--journal-dir))))))
    (let ((status
           (cond ((null row) "missing") ((cdr rows) "ambiguous")
                 ((or (file-remote-p file) (string-suffix-p ".gpg" file)
                      (not (file-readable-p file))) "unavailable")
                 ((seq-some
                   (lambda (buffer)
                     (with-current-buffer buffer
                       (and (buffer-modified-p)
                            (if buffer-file-name
                                (and (not (file-remote-p buffer-file-name))
                                     (file-equal-p file buffer-file-name))
                              (equal (seq-take gnosis-nodes--deleted-file 2)
                                     (list file gnosis-db))))))
                   (buffer-list)) "dirty"))))
      (if status (list :id id :status status :text nil)
        (condition-case nil
            (with-temp-buffer
              (insert-file-contents file)
              (if (not (equal (nth 2 row) (secure-hash 'sha1 (current-buffer))))
                  (list :id id :status "stale" :text nil)
                (delay-mode-hooks (org-mode))
                (goto-char (point-min))
                (if-let* ((position (org-find-property "ID" id)))
                    (progn
                      (goto-char position)
                      (unless (org-before-first-heading-p) (org-narrow-to-subtree))
                      (list :id id :status "found"
                            :text (buffer-substring-no-properties (point-min) (point-max))))
                  (list :id id :status "stale" :text nil))))
          (file-error (list :id id :status "unavailable" :text nil)))))))

(defun gnosis-agent-content-encounter ()
  "Return read-only native encounter metadata without learner answers.
:busy means organization must wait.  :items lists known :id, :session-id,
:mode and :phase (\"answering\" or \"feedback\").  Refuse organization
throughout any running native session, including source visits and
transitional callbacks.
No buffer names, input text or grading results are exposed."
  (let ((items
         (cl-loop for buffer in (buffer-list)
                  append
                  (with-current-buffer buffer
                    (when (and gnosis-review--state
                               (equal gnosis-review--running
                                      (gnosis-review-state-session-id gnosis-review--state)))
                      (when-let* ((id (car (gnosis-review-state-remaining gnosis-review--state))))
                        (list (list :id (number-to-string id)
                                    :session-id gnosis-review--running
                                    :mode (symbol-name (gnosis-review-state-mode gnosis-review--state))
                                    :phase (if gnosis-review--feedback "feedback" "answering")))))))))
    (list :api-version 1 :busy (if gnosis-review--running t :false) :items (vconcat items))))

(defun gnosis-agent-content--check-edit (id revision)
  "Refuse editing ID with stale REVISION, active input or a native draft."
  (when gnosis-review--running (user-error "Native encounter active; defer organization"))
  (unless (and (gnosis-get 'id 'themata `(= id ,id))
               (equal revision (gnosis-agent-content--revision id)))
    (user-error "Thema %s is missing or changed; fetch it again" id))
  (when (seq-some
         (lambda (buffer)
           (with-current-buffer buffer
             (and (eq gnosis--draft-db gnosis-db) (not gnosis--draft-saved-p)
                  (equal id (car gnosis--draft-original)))))
         (buffer-list))
    (user-error "Thema %s has an open native draft" id)))

(defun gnosis-agent-content--source-proof (id)
  "Qualify saved source ID and return metadata and a content digest."
  (let ((source (gnosis-agent-content-source id)))
    (unless (equal (plist-get source :status) "found")
      (user-error "Source %s is not a current saved source" id))
    (list :id id :digest (secure-hash 'sha256 (plist-get source :text))
          :nodes (vconcat (mapcar #'vconcat (gnosis-select '[file title hash] 'nodes `(= id ,id))))
          :journal (vconcat (mapcar #'vconcat (gnosis-select '[file title hash] 'journal `(= id ,id)))))))

(defun gnosis-agent-content--unlink (text ids)
  "Remove bracketed links to IDS in TEXT, retaining their descriptions.
Refuse ambiguous markup rather than rewriting unrelated authored prose."
  (with-temp-buffer
    (insert text)
    (delay-mode-hooks (org-mode))
    ;; Work backwards so the native syntax positions remain valid.  Literal
    ;; examples are not links; the residual index check below refuses them.
    (dolist (link (reverse (org-element-map (org-element-parse-buffer) 'link
                            (lambda (link)
                              (when (and (equal (org-element-property :type link) "id")
                                         (member (org-element-property :path link) ids))
                                link)))))
      (let* ((begin (org-element-property :begin link))
             (end (- (org-element-property :end link)
                     (org-element-property :post-blank link)))
             (raw (buffer-substring-no-properties begin end)))
        (unless (string-match "\\`\\[\\[id:\\([^]\n]+\\)\\]\\(?:\\[\\([^][\n]*\\)\\]\\)?\\]\\'" raw)
          (user-error "Cannot safely remove literal or complex source link markup"))
        (let ((description (or (match-string 2 raw) "")))
          (delete-region begin end)
          (goto-char begin)
          (insert description))))
    (when (seq-intersection ids (gnosis-extract-id-links (buffer-string)))
      (user-error "Cannot safely remove literal or complex source link markup"))
    (buffer-string)))

(defun gnosis-agent-content--change (change)
  "Validate CHANGE and return its exact proposed content receipt."
  (let* ((allowed '(:id :revision :add-tags :remove-tags :add-sources :remove-sources))
         (keys (and (proper-list-p change) (cl-evenp (length change))
                    (cl-loop for (key _) on change by #'cddr collect key))))
    (unless (and keys (= (length keys) (length (delete-dups (copy-sequence keys))))
                 (seq-every-p (lambda (key) (memq key allowed)) keys))
      (user-error "Malformed organization change")))
  (let* ((id (gnosis-agent-content--id (plist-get change :id)))
         (add (gnosis-agent-content--strings (or (plist-get change :add-tags) [])))
         (remove (gnosis-agent-content--strings (or (plist-get change :remove-tags) [])))
         (sources (or (plist-get change :add-sources) []))
         (unlink (gnosis-agent-content--strings (or (plist-get change :remove-sources) []))))
    (gnosis-agent-content--check-edit id (plist-get change :revision))
    (gnosis-tags--check-org add)
    (when (seq-intersection add remove) (user-error "A tag cannot be both added and removed"))
    (unless (and (vectorp sources) (<= (+ (length sources) (length unlink)) 100))
      (user-error "Provide at most 100 source operations per thema"))
    (let* ((before (gnosis-agent-content--record id))
           (question (plist-get before :question))
           (old (or (plist-get before :parathema) ""))
           (proofs
            (mapcar
             (lambda (source)
               (unless (and (proper-list-p source) (= (length source) 4)
                            (plist-member source :id) (plist-member source :label)
                            (seq-every-p
                             (lambda (s) (and (stringp s) (not (string-empty-p s))
                                              (not (string-match-p "[][\n\r]" s))))
                             (list (plist-get source :id) (plist-get source :label))))
                 (user-error "Each source needs a plain nonempty :id and :label"))
               (gnosis-agent-content--source-proof (plist-get source :id))) sources))
           (add-ids (mapcar (lambda (s) (plist-get s :id)) sources))
           (tags (sort (seq-union (seq-difference (append (plist-get before :tags) nil) remove) add) #'string<)))
      (unless (= (length add-ids) (length (delete-dups (copy-sequence add-ids))))
        (user-error "Duplicate added source"))
      (when (seq-intersection unlink (gnosis-extract-id-links question))
        (user-error "Source occurs in recall question; use the native content editor"))
      (let* ((stripped (gnosis-agent-content--unlink old unlink))
             (parathema (concat stripped
                                (mapconcat (lambda (s)
                                             (format "\n[[id:%s][%s]]" (plist-get s :id) (plist-get s :label)))
                                           sources "")))
             (links (sort (seq-union
                           (seq-difference (append (plist-get before :source-ids) nil) unlink)
                           add-ids) #'string<)))
        (list :id (number-to-string id) :status "ready" :before before
              :tags (vconcat tags) :question question
              :parathema (if (or (plist-member change :add-sources)
                                 (plist-member change :remove-sources))
                             parathema (plist-get before :parathema))
              :sources (vconcat proofs) :source-ids (vconcat links))))))

(defun gnosis-agent-content-preview (owner changes)
  "Preview explicit organization on exact connection OWNER.
CHANGES is a vector of at most 500 unique-ID plists.  Each requires :id and
:revision from fetch; optional :add-tags/:remove-tags are string vectors.
Optional :add-sources is a vector of (:id SOURCE-ID :label DESCRIPTION).
Append descriptive links to post-answer parathema, never to the question.
Optional :remove-sources is a string vector: remove those parathema links,
retaining descriptions, and their indexed associations.  Refuse removal if
that source occurs in the question, literal Org text or complex markup.
Explicit empty source fields still write content, normalizing nil parathema
to an empty string.  Omitting both fields preserves its exact value.
Combine removal and addition to correct a source or label.  Other prose and
associations remain; no arbitrary graph-only edges are added.  Each thema
permits at most 100 source operations.  Added sources must have current saved
node or journal content; revalidate metadata and content during apply.
Stale content, active review and open drafts signal `user-error'.
Return a JSON-ready plan with :changes and per-item :items showing exact
:before content, proposed :tags, :question, :parathema, :source-ids and
:sources proofs.  Empty changes are a no-op; no writes occur."
  (unless (equal owner (gnosis-agent-content--owner)) (user-error "Database owner changed"))
  (unless (and (vectorp changes) (<= (length changes) 500))
    (user-error "Provide at most 500 explicit changes"))
  (gnosis-sqlite-with-transaction (gnosis--ensure-db)
    (let* ((items (mapcar #'gnosis-agent-content--change changes))
           (ids (mapcar (lambda (item) (plist-get item :id)) items)))
      (unless (= (length ids) (length (delete-dups (copy-sequence ids))))
        (user-error "Duplicate change IDs"))
      (list :api-version 1 :owner owner :changes (copy-tree changes t) :items (vconcat items)))))

(defun gnosis-agent-content-apply (plan)
  "Atomically apply the exact organization PLAN returned by preview.
Recompute and compare the whole plan in the write transaction before any
writes.  Signal on stale content, owner, native draft, active encounter or
modified plan.  All items roll back on error/quit; there is no partial success.
Return fresh fetch-shaped :items with status \"found\" and exact new revisions.
No schedules, history, session membership, Org files, UI or Git are changed.
A retry after a changing success is stale: fetch/read back before proposing
again.  This local API is an explicit write boundary, not an authorization
service; callers decide whether the shown plan is approved."
  (let ((db (gnosis--ensure-db)))
    (gnosis-sqlite-with-transaction db
      (unless (equal plan (gnosis-agent-content-preview
                           (plist-get plan :owner) (plist-get plan :changes)))
        (user-error "Organization preview changed; preview again"))
      (cl-mapc
       (lambda (change item)
         (let ((id (gnosis-agent-content--id (plist-get item :id))))
           (gnosis-agent-content--check-edit id (plist-get change :revision))
           (gnosis-modify-thema-tags
            (list id) (append (plist-get change :add-tags) nil)
            (append (plist-get change :remove-tags) nil))
           (when (or (plist-member change :add-sources) (plist-member change :remove-sources))
             (let ((before (plist-get item :before)))
               (gnosis-update-thema
                id (plist-get before :question) (append (plist-get before :hypothesis) nil)
                (append (plist-get before :answers) nil) (plist-get item :parathema)
                (append (plist-get item :tags) nil) (append (plist-get item :source-ids) nil))))))
       (plist-get plan :changes) (plist-get plan :items))
      (gnosis-agent-content-fetch
       (vconcat (mapcar (lambda (item) (plist-get item :id)) (plist-get plan :items)))))))

;;; Creating themata

(defconst gnosis-agent-content--card-keys
  '(:key :type :question :answer :choices :hints :parathema :rubric :aliases :tags)
  "Fields of a card record for `gnosis-agent-content-create'.")

(defconst gnosis-agent-content--card-types
  '("basic" "double" "mcq" "cloze" "mc-cloze" "agent-eval")
  "Types `gnosis-agent-content-create' makes.
Model cards have their own agent tools, which check the model first.")

(defconst gnosis-agent-content--choice-types '("mcq" "mc-cloze")
  "Types whose alternatives are choices rather than hints.")

(defconst gnosis-agent-content--create-limit 500
  "Most cards one `gnosis-agent-content-create' call takes.")

(defun gnosis-agent-content--texts (value field &optional nonempty)
  "Return VALUE, a vector of strings for FIELD, as a list.
Nil means none.  With NONEMPTY, require at least one nonempty string."
  (unless (and (or (null value) (vectorp value))
               (seq-every-p (lambda (s) (and (stringp s) (not (string-empty-p (string-trim s)))))
                            value))
    (user-error "%s must be an array of nonempty strings" field))
  (when (and nonempty (zerop (length value)))
    (user-error "%s must hold at least one string" field))
  ;; Outer whitespace is not content: the editor trims it on reading.
  (mapcar #'string-trim value))

(defun gnosis-agent-content--card (card)
  "Return CARD as `gnosis-save-thema' takes it, or signal what is wrong.
CARD is a plist of `gnosis-agent-content--card-keys'."
  (unless card (user-error "Card is empty"))
  (unless (and (consp card) (plistp card))
    (user-error "Each card must be an object"))
  (let ((unknown (cl-loop for (key _) on card by #'cddr
                          unless (memq key gnosis-agent-content--card-keys) collect key)))
    (when unknown (user-error "Unknown fields: %s" (mapconcat #'symbol-name unknown " "))))
  (let ((type (plist-get card :type)))
    (unless (and (stringp type) (member (downcase type) gnosis-agent-content--card-types))
      (user-error "type must be one of: %s (model cards: gnosis_model_card_preview)"
                  (mapconcat #'identity gnosis-agent-content--card-types ", ")))
    (let* ((type (downcase type))
           (choice (member type gnosis-agent-content--choice-types))
           (choices (gnosis-agent-content--texts (plist-get card :choices) "choices"))
           (hints (gnosis-agent-content--texts (plist-get card :hints) "hints"))
           (tags (delete-dups (gnosis-agent-content--texts (plist-get card :tags) "tags")))
           (aliases (gnosis-agent-content--texts (plist-get card :aliases) "aliases"))
           ;; A cloze may mark its blanks inline instead, with no answer.
           (answer (gnosis-agent-content--texts (plist-get card :answer) "answer"
                                                (not (equal type "cloze")))))
      (unless (and (stringp (plist-get card :question))
                   (not (string-empty-p (string-trim (plist-get card :question)))))
        (user-error "question must be a nonempty string"))
      (dolist (key '(:parathema :rubric))
        (unless (or (null (plist-get card key)) (stringp (plist-get card key)))
          (user-error "%s must be a string" (substring (symbol-name key) 1))))
      (unless (or (null (plist-get card :key))
                  (and (stringp (plist-get card :key))
                       (not (string-empty-p (string-trim (plist-get card :key))))))
        (user-error "key must be a nonempty string"))
      (if choice
          (when hints (user-error "%s takes choices, not hints" type))
        (when choices (user-error "%s takes hints, not choices" type)))
      ;; Org tags, so the editor can open the card again.
      (gnosis-tags--check-org tags)
      (list "NEW" type (string-trim (plist-get card :question)) (if choice choices hints)
            answer (string-trim (or (plist-get card :parathema) "")) tags nil aliases
            (and (plist-get card :rubric) (string-trim (plist-get card :rubric)))))))

(defun gnosis-agent-content--card-guid (card thema)
  "Return the stored source identity of CARD, saved as THEMA.
That is its key, or a hash of what the learner is shown and graded on:
its type, question, hints or choices, answer, parathema, aliases and
rubric.  Tags organise a card rather than define it, so a retry that
only retags it finds it."
  (if-let* ((key (plist-get card :key)))
      (concat "agent:key:" key)
    (concat "agent:hash:"
            (secure-hash 'sha256 (gnosis-sqlite--serialize
                                  (mapcar (lambda (i) (nth i thema)) '(1 2 3 4 5 8 9)))))))

(defun gnosis-agent-content--existing (db guids)
  "Return a table of each of GUIDS that DB's themata hold to their IDs."
  (let ((found (make-hash-table :test #'equal)))
    (when guids
      ;; Raw text, as Anki import stores its identities in this column.
      (pcase-dolist (`(,id ,guid)
                     (sqlite-select
                      db (format "SELECT id, source_guid FROM themata WHERE source_guid IN (%s)
                                  ORDER BY id"
                                 (mapconcat (lambda (_) "?") guids ","))
                      guids))
        (push (number-to-string id) (gethash guid found))))
    (maphash (lambda (guid ids) (puthash guid (nreverse ids) found)) found)
    found))

(defun gnosis-agent-content--prepare (cards)
  "Return (INDEX GUID THEMA ERROR) for each of CARDS, checked.
A card repeating an earlier card's identity is an error."
  (let ((seen (make-hash-table :test #'equal)))
    (cl-loop
     for card across cards for index from 0
     collect
     (condition-case err
         (let* ((thema (gnosis-agent-content--card card))
                (guid (gnosis-agent-content--card-guid card thema))
                (earlier (gethash guid seen)))
           (if earlier
               (list index guid nil (format "%s repeats card %d of this call"
                                            (if (plist-get card :key) "key" "content")
                                            earlier))
             (puthash guid index seen)
             (list index guid thema nil)))
       (error (list index (and (consp card) (plistp card) (stringp (plist-get card :key))
                               (concat "agent:key:" (plist-get card :key)))
                    nil (error-message-string err)))))))

(defun gnosis-agent-content--draft-themata (ids)
  "Return themata IDS as the editor reads its draft of them back."
  (with-temp-buffer
    (delay-mode-hooks (org-mode))
    (gnosis-export--insert-themata ids)
    (gnosis-export-parse-themata)))

(defun gnosis-agent-content--kept (id)
  "Return what the editor shows and saves of thema ID.
An absent hint reads as an empty one: Gnosis stores either for none."
  (pcase-let ((`(,type ,question ,hints ,answer ,aliases ,rubric)
               (car (gnosis-select '[type keimenon hypothesis answer accepted-aliases rubric]
                                   'themata `(= id ,id)))))
    (list type question (mapcar (lambda (hint) (or hint "")) hints) answer aliases rubric
          (gnosis-get 'parathema 'extras `(= id ,id))
          (sort (gnosis-select 'tag 'thema-tag `(= thema-id ,id) t) #'string<)
          (sort (gnosis-select 'dest 'thema-links `(= source ,id) t) #'string<))))

(defconst gnosis-agent-content--kept-fields
  '("type" "question" "hints or choices" "answer" "aliases" "rubric" "parathema" "tags" "links")
  "What each element of `gnosis-agent-content--kept' is, as the agent names it.")

(defun gnosis-agent-content--resave (id thema before)
  "Save THEMA, the editor's reading of ID, and return why it lost BEFORE, or nil.
The editor's version is what then stays: it differs from BEFORE at most
by what the editor treats as the same (an absent hint read as empty)."
  (let ((saved (gnosis-save-thema thema)))
    (if saved
        (format "the editor could not save it again: %s"
                (replace-regexp-in-string "\\`Line [^ ]* (id:[^)]*): " "" saved))
      (let ((after (gnosis-agent-content--kept id)))
        (unless (equal before after)
          (cl-loop for field in gnosis-agent-content--kept-fields
                   for old in before for new in after
                   unless (equal old new)
                   return (format "the editor reads its %s as Org structure and would keep %S"
                                  field new)))))))

(defun gnosis-agent-content--editor-losses (ids)
  "Return (ID . WHY) for each of IDS that would not survive the editor.
Write their draft as the editor opens it, read it back and save each, as
reopening and saving them would, and require what it keeps unchanged.
One draft holds them all; only when it does not read back whole is each
thema drafted alone, to say which."
  (let* ((before (mapcar (lambda (id) (cons id (gnosis-agent-content--kept id))) ids))
         (themata (and ids (condition-case nil (gnosis-agent-content--draft-themata ids)
                             (error nil)))))
    (delq nil
          (if (or (null ids) (and themata (= (length themata) (length ids))))
              (mapcar (lambda (thema)
                        (let ((id (string-to-number (car thema))))
                          (when-let* ((why (gnosis-agent-content--resave
                                            id thema (alist-get id before))))
                            (cons id why))))
                      themata)
            (mapcar (lambda (id)
                      (condition-case err
                          (let ((alone (gnosis-agent-content--draft-themata (list id))))
                            (if (/= (length alone) 1)
                                (cons id "the editor would read it as several themata")
                              (when-let* ((why (gnosis-agent-content--resave
                                                id (car alone) (alist-get id before))))
                                (cons id why))))
                        (error (cons id (format "the editor could not read it back: %s"
                                                (error-message-string err))))))
                    ids)))))

(defun gnosis-agent-content--result (db entry existing dry-run)
  "Return ENTRY's result in DB, saving its thema unless EXISTING holds it.
ENTRY is (INDEX GUID THEMA ERROR); DRY-RUN names what a save is."
  (pcase-let* ((`(,index ,guid ,thema ,error) entry)
               (key (and guid (string-prefix-p "agent:key:" guid)
                         (string-remove-prefix "agent:key:" guid)))
               (card (list :index index :key key))
               (ids (and thema (gethash guid existing))))
    (cond
     (error (append card (list :status "invalid" :error error)))
     (ids (append card (list :status "exists" :ids (vconcat ids))))
     (t
      (let ((saved (gnosis-save-thema thema t)))
        (cond
         ((stringp saved)
          (append card (list :status "invalid"
                             :error (replace-regexp-in-string
                                     "\\`Line [^ ]* (id:[^)]*): " "" saved))))
         (t
          (dolist (id saved)
            (sqlite-execute db "UPDATE themata SET source_guid = ? WHERE id = ?" (list guid id)))
          (append card (list :status (if dry-run "would-create" "created")
                             :ids (vconcat (mapcar #'number-to-string (sort saved #'<))))))))))))

(cl-defun gnosis-agent-content-create (cards &key dry-run)
  "Create themata from CARDS, a vector of card records, all or none.
Each record is a plist of `gnosis-agent-content--card-keys': `:type' (one
of `gnosis-agent-content--card-types'), `:question', `:answer' (a vector),
`:choices' (mcq and mc-cloze) or `:hints' (the others), `:parathema',
`:rubric' (agent-eval), `:aliases' and `:tags' (vectors of Org tags), and
`:key'.  A card is saved as Gnosis saves any thema (`gnosis-save-thema'),
so each type's own rules apply; a double, or a cloze marking its blanks
inline, makes several themata.

The key, or a hash of the card's content (all but its tags) when none
is given, makes creation idempotent: a card whose key is already stored
is reported `exists' with its IDs, not made again.  A key names one card
across every call.  A card whose text the editor could not open and save
again unchanged is invalid.
Every card is checked in one transaction, rolled back when any card is
invalid or for DRY-RUN, so nothing is then written.  It is refused
inside another transaction, which could not roll back alone.

Return (:dry-run BOOL :written BOOL :cards [RESULT...]), each RESULT
\(:index N :key KEY :status STATUS ...): `created' or `exists' with
`:ids' (decimal strings, ascending), `invalid' with `:error', or
`would-create' with `:themata', the number it makes, when nothing was
written.  KEY is the card's own key, or nil."
  (unless (and (vectorp cards) (< 0 (length cards) (1+ gnosis-agent-content--create-limit)))
    (user-error "cards must be an array of 1 to %d records" gnosis-agent-content--create-limit))
  (let ((db (gnosis--ensure-db))
        (prepared (gnosis-agent-content--prepare cards))
        results written)
    (when (memq db gnosis-sqlite--transaction-dbs)
      (error "Creating cards needs a transaction of its own"))
    (catch 'gnosis-agent-content--undo
      (gnosis-sqlite-with-transaction db
        ;; Read stored identities under the write lock, so a retry racing
        ;; this one cannot make the cards twice.
        (let ((existing (gnosis-agent-content--existing
                         db (delq nil (mapcar (lambda (entry) (and (nth 2 entry) (nth 1 entry)))
                                              prepared)))))
          (setq results (mapcar (lambda (entry)
                                  (gnosis-agent-content--result db entry existing dry-run))
                                prepared))
          ;; Every new thema must open and save again in the editor.
          (let ((losses (gnosis-agent-content--editor-losses
                         (mapcan (lambda (r)
                                      (and (member (plist-get r :status) '("created" "would-create"))
                                           (mapcar #'string-to-number (plist-get r :ids))))
                                    results))))
            (when losses
              (setq results
                    (mapcar (lambda (r)
                              (if-let* ((why (seq-some (lambda (id)
                                                         (alist-get (string-to-number id) losses))
                                                       (plist-get r :ids))))
                                  (list :index (plist-get r :index) :key (plist-get r :key)
                                        :status "invalid" :error (concat "Its text cannot be kept: " why))
                                r))
                            results)))))
        (when (or dry-run
                  (seq-some (lambda (r) (equal (plist-get r :status) "invalid")) results))
          (throw 'gnosis-agent-content--undo nil))
        (setq written (seq-some (lambda (r) (equal (plist-get r :status) "created")) results))))
    (when written
      (dolist (result results)
        (when (equal (plist-get result :status) "created")
          (mapc (lambda (id) (run-hook-with-args 'gnosis-save-hook (string-to-number id)))
                (plist-get result :ids)))))
    (list :dry-run (if dry-run t :false)
          :written (if written t :false)
          :cards (vconcat
                  (mapcar (lambda (result)
                            ;; Nothing was kept: no IDs to point at, only how
                            ;; many themata the card makes.
                            (if (and (not written) (member (plist-get result :status)
                                                           '("created" "would-create")))
                                (list :index (plist-get result :index) :key (plist-get result :key)
                                      :status "would-create"
                                      :themata (length (plist-get result :ids)))
                              result))
                          results)))))

(provide 'gnosis-agent-content)
;;; gnosis-agent-content.el ends here
