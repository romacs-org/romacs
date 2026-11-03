;;; romacs-agent-gnosis.el --- Gnosis's agent API as Romacs MCP tools -*- lexical-binding: t; -*-

;; SPDX-License-Identifier: GPL-3.0-or-later
;; Maintainer: Thanos Apollo <public@thanosapollo.org>

;;; Commentary:
;; Gnosis's agent API v1 (rlisp/gnosis/gnosis-agent*.el) as MCP tools, one
;; per public function, so any agent reviews and authors with the API's own
;; invariants: practice is native human answering, nothing here grades,
;; and the durable core owns acceptance.  Each tool is an explicit wrapper:
;; JSON arguments become the function's own argument types (lists for
;; practice selections, vectors for content and review records, t and nil
;; for policy flags), the open database connection is supplied here, and
;; text is data, never read or evaluated.  Results are API v1 plists
;; serialised as the API says (`json-serialize' with :false-object :false,
;; :null-object nil).
;;
;; Cursors and preview plans must come back exactly as given; JSON would
;; not keep them so (key order, lists against vectors).  They stay here,
;; and the agent is given a token for each (the newest 256 are kept).
;; Gnosis loads at a tool's first call, on the connected database.

;;; Code:
(require 'cl-lib)
(require 'romacs-mcp)

(declare-function gnosis--ensure-db "gnosis")

;;; Tokens

(defconst romacs-agent-gnosis--kept 256 "Tokens kept.")
(defvar romacs-agent-gnosis--tokens (make-hash-table :test #'equal) "Token to its exact Lisp value.")
(defvar romacs-agent-gnosis--ring (make-vector romacs-agent-gnosis--kept nil)
  "The kept tokens: a new one takes the slot of the oldest.")
(defvar romacs-agent-gnosis--count 0)

(defun romacs-agent-gnosis--token (value)
  "A token standing for VALUE, or nil for a nil VALUE."
  (when value
    (let* ((slot (% romacs-agent-gnosis--count romacs-agent-gnosis--kept))
           (token (format "gnosis-%d-%d" (emacs-pid) (cl-incf romacs-agent-gnosis--count))))
      (when (aref romacs-agent-gnosis--ring slot) (remhash (aref romacs-agent-gnosis--ring slot) romacs-agent-gnosis--tokens))
      (aset romacs-agent-gnosis--ring slot token)
      (puthash token value romacs-agent-gnosis--tokens)
      token)))

(defun romacs-agent-gnosis--value-of (token)
  "The value TOKEN stands for; nil for none given."
  (cond ((memq token '(nil :null)) nil)
        ((gethash token romacs-agent-gnosis--tokens))
        (t (user-error "Unknown or expired token %S: start again" token))))

;;; JSON arguments to the API's types

(defun romacs-agent-gnosis--arg (arguments name)
  "ARGUMENTS' NAME, JSON null as nil."
  (let ((value (gethash name arguments)))
    (unless (eq value :null) value)))

(defun romacs-agent-gnosis--list (value predicate what)
  "VALUE, a JSON array, as a list whose elements satisfy PREDICATE."
  (cond ((memq value '(nil :null)) nil)
        ((and (vectorp value) (cl-every predicate value)) (append value nil))
        (t (user-error "%s must be an array" what))))

(defun romacs-agent-gnosis--data (value)
  "VALUE, JSON data, as API v1 data: objects as keyword plists in their
own key order, arrays as vectors, null as nil, false as :false."
  (cond ((hash-table-p value)
         (let (plist)
           (maphash (lambda (key item) (setq plist (cl-list* (intern (concat ":" key)) (romacs-agent-gnosis--data item) plist)))
                    value)
           (romacs-agent-gnosis--reverse-plist plist)))
        ((vectorp value) (vconcat (mapcar #'romacs-agent-gnosis--data value)))
        ((eq value :null) nil)
        (t value)))

(defun romacs-agent-gnosis--reverse-plist (plist)
  (let (out)
    (while plist (setq out (cl-list* (car plist) (cadr plist) out) plist (cddr plist)))
    out))

(defun romacs-agent-gnosis--record (object keys)
  "OBJECT, a JSON object, as a keyword plist in the order of KEYS, holding
only the keys OBJECT has; other keys are refused."
  (unless (hash-table-p object) (user-error "Expected an object"))
  (maphash (lambda (key _) (unless (memq (intern (concat ":" key)) keys) (user-error "Unknown field %s" key))) object)
  (cl-loop for key in keys
           for name = (substring (symbol-name key) 1)
           unless (eq (gethash name object :absent) :absent)
           append (list key (romacs-agent-gnosis--data (gethash name object)))))

(defun romacs-agent-gnosis--policy (object)
  "A practice POLICY object as the API's keyword plist, flags as t or nil."
  (when (hash-table-p object)
    (let ((plist (romacs-agent-gnosis--record object '(:successes-required :successes-after-failure
                                                       :consecutive :max-attempts-per-thema))))
      (when (plist-member plist :consecutive)
        (setq plist (plist-put plist :consecutive (eq (plist-get plist :consecutive) t))))
      plist)))

(defun romacs-agent-gnosis--groups (value)
  (mapcar (lambda (group) (romacs-agent-gnosis--list group #'integerp "Each thema group"))
          (romacs-agent-gnosis--list value #'vectorp "thema-groups")))

(defun romacs-agent-gnosis--records (value keys what)
  (unless (vectorp value) (user-error "%s must be an array" what))
  (vconcat (mapcar (lambda (object) (romacs-agent-gnosis--record object keys)) value)))

(defconst romacs-agent-gnosis--change-keys
  '(:id :revision :add-tags :remove-tags :add-sources :remove-sources))

(defconst romacs-agent-gnosis--question-keys
  '(:id :type :question :answer :choices :reference-answer :rubric :hints :parathema :tags :source))

;;; Results

(defun romacs-agent-gnosis--json (result)
  (decode-coding-string (json-serialize result :false-object :false :null-object nil) 'utf-8))

(defun romacs-agent-gnosis--paged (result key)
  "RESULT with its continuation at KEY replaced by a token."
  (romacs-agent-gnosis--json (plist-put (copy-sequence result) key (or (romacs-agent-gnosis--token (plist-get result key))))))

;;; Tools

(defun romacs-agent-gnosis--load ()
  (require 'gnosis-agent)
  (require 'gnosis-agent-content)
  (require 'gnosis-agent-review))

(defun romacs-agent-gnosis--session (arguments)
  (let ((id (romacs-agent-gnosis--arg arguments "session-id")))
    (unless (stringp id) (user-error "session-id must be a string"))
    id))

(defmacro romacs-agent-gnosis--tool (name description properties required &rest body)
  "Register gnosis tool NAME; BODY sees ARGUMENTS and returns API v1 data
or a JSON string."
  (declare (indent 4))
  `(romacs-mcp-register-tool
    ,name ,description (romacs-mcp--schema ,properties ,required)
    (lambda (arguments)
      (romacs-mcp--instance arguments)
      (romacs-agent-gnosis--load)
      (let ((value (progn ,@body)))
        (if (stringp value) value (romacs-agent-gnosis--json value))))))

(defconst romacs-agent-gnosis--instance '("instance" . "string"))
(defconst romacs-agent-gnosis--session-property '("session-id" . "string"))

(romacs-agent-gnosis--tool "gnosis_status"
    "Gnosis API v1 status and progress of an exact practice or review session."
    (list romacs-agent-gnosis--instance romacs-agent-gnosis--session-property) '("session-id")
  (gnosis-agent-status (romacs-agent-gnosis--session arguments)))

(romacs-agent-gnosis--tool "gnosis_results"
    "Gnosis API v1 per-item outcomes and corrections of an exact practice session (practice only)."
    (list romacs-agent-gnosis--instance romacs-agent-gnosis--session-property) '("session-id")
  (gnosis-agent-results (romacs-agent-gnosis--session arguments)))

(romacs-agent-gnosis--tool "gnosis_resume"
    "Schedule the current unfinished session for native answering again."
    (list romacs-agent-gnosis--instance romacs-agent-gnosis--session-property) '("session-id")
  (gnosis-agent-resume (romacs-agent-gnosis--session arguments)))

(romacs-agent-gnosis--tool "gnosis_cancel"
    "Cancel a pending or interrupted session, keeping all accepted evidence."
    (list romacs-agent-gnosis--instance romacs-agent-gnosis--session-property) '("session-id")
  (gnosis-agent-cancel (romacs-agent-gnosis--session arguments)))

(romacs-agent-gnosis--tool "gnosis_current_practice"
    "Status of the current practice reservation, or null; starts nothing."
    (list romacs-agent-gnosis--instance) nil
  (gnosis-agent-current-practice))

(romacs-agent-gnosis--tool "gnosis_sessions"
    "Retained sessions, newest insertion first, on the connected database: limit 1..50 payloads examined; pass \
the returned next-cursor token back as cursor until complete; kind agent-review, practice or due."
    (list romacs-agent-gnosis--instance '("limit" . "integer") '("cursor" . "string") '("kind" . "string")) nil
  (romacs-agent-gnosis--paged
   (gnosis-agent-sessions :connection (gnosis--ensure-db)
                          :limit (or (romacs-agent-gnosis--arg arguments "limit") 10)
                          :cursor (romacs-agent-gnosis--value-of (gethash "cursor" arguments))
                          :kind (romacs-agent-gnosis--arg arguments "kind"))
   :next-cursor))

(defconst romacs-agent-gnosis--selection
  (list romacs-agent-gnosis--instance '("limit" . "integer") '("policy" . "object")))

(romacs-agent-gnosis--tool "gnosis_start_practice"
    "Reserve native practice of thema-ids (integers) or topic-ids (strings), with a positive limit and an \
optional policy {successes-required, successes-after-failure, consecutive, max-attempts-per-thema}. The human \
answers later; this records no grade."
    (append romacs-agent-gnosis--selection '(("thema-ids" . "array") ("topic-ids" . "array"))) '("limit")
  (gnosis-agent-start-practice
   :thema-ids (romacs-agent-gnosis--list (gethash "thema-ids" arguments) #'integerp "thema-ids")
   :topic-ids (romacs-agent-gnosis--list (gethash "topic-ids" arguments) #'stringp "topic-ids")
   :limit (gethash "limit" arguments)
   :policy (romacs-agent-gnosis--policy (gethash "policy" arguments))))

(romacs-agent-gnosis--tool "gnosis_start_practice_groups"
    "Reserve ordered native practice from tags (strings, one group each) or thema-groups (arrays of integer \
ids), with a limit and an optional policy. Records no grade."
    (append romacs-agent-gnosis--selection '(("tags" . "array") ("thema-groups" . "array"))) '("limit")
  (gnosis-agent-start-practice-groups
   :tags (romacs-agent-gnosis--list (gethash "tags" arguments) #'stringp "tags")
   :thema-groups (romacs-agent-gnosis--groups (gethash "thema-groups" arguments))
   :limit (gethash "limit" arguments)
   :policy (romacs-agent-gnosis--policy (gethash "policy" arguments))))

(romacs-agent-gnosis--tool "gnosis_start_review_groups"
    "Reserve ordered scheduled review (due items only) from tags or thema-groups, with a limit; native \
answering and ordinary scheduling own the outcome."
    (list romacs-agent-gnosis--instance '("limit" . "integer") '("tags" . "array") '("thema-groups" . "array"))
    '("limit")
  (gnosis-agent-start-review-groups
   :tags (romacs-agent-gnosis--list (gethash "tags" arguments) #'stringp "tags")
   :thema-groups (romacs-agent-gnosis--groups (gethash "thema-groups" arguments))
   :limit (gethash "limit" arguments)))

(romacs-agent-gnosis--tool "gnosis_content_fetch"
    "Content records of decimal-string thema ids (at most 500), in request order."
    (list romacs-agent-gnosis--instance '("ids" . "array")) '("ids")
  (gnosis-agent-content-fetch (vconcat (romacs-agent-gnosis--list (gethash "ids" arguments) #'stringp "ids"))))

(romacs-agent-gnosis--tool "gnosis_content_search"
    "Content in id order matching literal text (empty matches all), optionally a tag and a source-id; pass the \
returned next token back as cursor until complete."
    (list romacs-agent-gnosis--instance '("text" . "string") '("tag" . "string") '("source-id" . "string")
          '("limit" . "integer") '("cursor" . "string"))
    nil
  (romacs-agent-gnosis--paged
   (gnosis-agent-content-search :text (or (romacs-agent-gnosis--arg arguments "text") "")
                                :tag (romacs-agent-gnosis--arg arguments "tag")
                                :source-id (romacs-agent-gnosis--arg arguments "source-id")
                                :limit (or (romacs-agent-gnosis--arg arguments "limit") 100)
                                :cursor (romacs-agent-gnosis--value-of (gethash "cursor" arguments)))
   :next))

(romacs-agent-gnosis--tool "gnosis_content_vocabulary"
    "Existing tags or sources (kind \"tags\" or \"sources\"); pass the next token back as cursor until complete."
    (list romacs-agent-gnosis--instance '("kind" . "string") '("limit" . "integer") '("cursor" . "string")) '("kind")
  (romacs-agent-gnosis--paged
   (gnosis-agent-content-vocabulary (gethash "kind" arguments)
                                    :limit (or (romacs-agent-gnosis--arg arguments "limit") 100)
                                    :cursor (romacs-agent-gnosis--value-of (gethash "cursor" arguments)))
   :next))

(romacs-agent-gnosis--tool "gnosis_content_source"
    "An indexed source's saved Org subtree, without visiting or syncing it."
    (list romacs-agent-gnosis--instance '("id" . "string")) '("id")
  (gnosis-agent-content-source (gethash "id" arguments)))

(romacs-agent-gnosis--tool "gnosis_content_encounter"
    "Read-only metadata of a running native encounter (busy means organization must wait)."
    (list romacs-agent-gnosis--instance) nil
  (gnosis-agent-content-encounter))

(romacs-agent-gnosis--tool "gnosis_content_preview"
    "Preview organizing themata on the connection owner (from fetch): changes, at most 500, each {id, revision, \
add-tags?, remove-tags?, add-sources? [{id, label}], remove-sources?}. Writes nothing; returns the plan and a \
plan-token for gnosis_content_apply."
    (list romacs-agent-gnosis--instance '("owner" . "string") '("changes" . "array")) '("owner" "changes")
  (let* ((changes (romacs-agent-gnosis--records (gethash "changes" arguments) romacs-agent-gnosis--change-keys "changes"))
         (changes (vconcat (mapcar (lambda (change)
                                     (let ((sources (plist-get change :add-sources)))
                                       (if (vectorp sources)
                                           (plist-put change :add-sources
                                                      (vconcat (mapcar (lambda (source)
                                                                         (if (listp source) source
                                                                           (user-error "add-sources holds objects")))
                                                                       sources)))
                                         change)))
                                   changes)))
         (plan (gnosis-agent-content-preview (gethash "owner" arguments) changes)))
    (romacs-agent-gnosis--json (append plan (list :plan-token (romacs-agent-gnosis--token plan))))))

(romacs-agent-gnosis--tool "gnosis_content_apply"
    "Apply exactly the plan gnosis_content_preview returned (its plan-token), atomically; refused when anything \
changed since. Return fresh records."
    (list romacs-agent-gnosis--instance '("plan-token" . "string")) '("plan-token")
  (gnosis-agent-content-apply (romacs-agent-gnosis--value-of (gethash "plan-token" arguments))))

(romacs-agent-gnosis--tool "gnosis_review_start_session"
    "Reserve a local tutoring session: questions (at most 128 records {id, type, question, answer [], choices [], \
rubric, hints [], parathema, tags [], source}), a teaching goal and a fixed source. The tutor is the session's \
gnosis-agent-review-function; native answering owns acceptance; no themata or grades are created."
    (list romacs-agent-gnosis--instance '("questions" . "array") '("goal" . "string") '("source" . "string"))
    '("questions" "goal" "source")
  (gnosis-agent-start-review-session
   :questions (romacs-agent-gnosis--records (gethash "questions" arguments) romacs-agent-gnosis--question-keys "questions")
   :goal (gethash "goal" arguments) :source (gethash "source" arguments)))

(romacs-agent-gnosis--tool "gnosis_review_status"
    "Status of an exact tutoring session."
    (list romacs-agent-gnosis--instance romacs-agent-gnosis--session-property) '("session-id")
  (gnosis-agent-review-status (romacs-agent-gnosis--session arguments)))

(romacs-agent-gnosis--tool "gnosis_review_results"
    "Results and transcript of an exact tutoring session."
    (list romacs-agent-gnosis--instance romacs-agent-gnosis--session-property) '("session-id")
  (gnosis-agent-review-results (romacs-agent-gnosis--session arguments)))

(romacs-agent-gnosis--tool "gnosis_review_save_question"
    "Save one question of the paused current tutoring session as an ordinary thema, on this explicit request."
    (list romacs-agent-gnosis--instance romacs-agent-gnosis--session-property '("question-id" . "string"))
    '("session-id" "question-id")
  (gnosis-agent-review-save-question (romacs-agent-gnosis--session arguments) (gethash "question-id" arguments)))

(provide 'romacs-agent-gnosis)
;;; romacs-agent-gnosis.el ends here
