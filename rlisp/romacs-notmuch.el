;;; romacs-notmuch.el --- Pure Notmuch mail provider -*- lexical-binding: t -*-

;; SPDX-License-Identifier: GPL-3.0-or-later

;;; Commentary:

;; Adapt Notmuch JSON v5 to account-qualified mail data without changing tags.

;;; Code:

(defvar romacs-notmuch-command '("notmuch")
  "Notmuch command argv, including any account configuration selector.")
(defvar romacs-notmuch-send-command nil
  "Transport argv accepting complete UTF-8 mail on standard input.
Exit zero must mean accepted and stored in Sent.  Uncertain submissions must
never be automatically retried.")
(defvar romacs-notmuch-from nil
  "Optional From override for new mail; replies use Notmuch's inferred From.")
(defvar romacs-notmuch-search-history nil)

(when (fboundp 'declare-function)
  (declare-function romacs-mail-search "romacs-mail" (account query))
  (declare-function romacs-mail-compose "romacs-mail" (account &optional to subject))
  (declare-function romacs-mail-reply "romacs-mail" (all-p))
  (declare-function romacs-mail-tree-current-query "romacs-mail" ()))

(defun romacs-notmuch--map (function values)
  "Return FUNCTION applied to each element of proper list VALUES."
  (let ((result nil))
    (dolist (value values) (setq result (cons (funcall function value) result)))
    (nreverse result)))

(defun romacs-notmuch--get (key object)
  "Return string KEY's value from JSON OBJECT."
  (cdr (assoc key object)))

(defun romacs-notmuch--array (value)
  "Validate and return JSON array VALUE."
  (unless (proper-list-p value) (error "Invalid Notmuch array"))
  value)

(defun romacs-notmuch--object (value)
  "Validate and return JSON object VALUE."
  (unless (and (consp value) (proper-list-p value)
               (catch 'invalid
                 (mapc (lambda (entry)
                         (unless (and (consp entry) (stringp (car entry)))
                           (throw 'invalid nil))) value)
                 t))
    (error "Invalid Notmuch object"))
  value)

(defun romacs-notmuch--string (value &optional nonempty)
  "Validate string VALUE, requiring content when NONEMPTY is non-nil."
  (unless (and (stringp value) (or (not nonempty) (> (length value) 0)))
    (error "Invalid Notmuch string"))
  value)

(defun romacs-notmuch--number (value)
  "Validate nonnegative integer VALUE."
  (unless (and (integerp value) (>= value 0))
    (error "Invalid Notmuch number"))
  value)

(defun romacs-notmuch--boolean (value)
  "Normalize JSON boolean VALUE without treating JSON false as true."
  (cond ((eq value t) t)
        ((memq value '(nil json-false json-null)) nil)
        (t (error "Invalid Notmuch boolean"))))

(defun romacs-notmuch--tags (value)
  "Validate string array VALUE."
  (romacs-notmuch--map #'romacs-notmuch--string (romacs-notmuch--array value)))

(defun romacs-notmuch--header (name headers &optional required)
  "Return NAME from HEADERS, requiring a string when REQUIRED."
  (let ((entry (assoc name headers)))
    (if entry (romacs-notmuch--string (cdr entry))
      (if required (error "Missing Notmuch header: %s" name) ""))))

(defun romacs-notmuch--headers (value)
  "Validate and copy string-key header alist VALUE."
  (romacs-notmuch--map (lambda (entry)
            (cons (car entry) (romacs-notmuch--string (cdr entry))))
          (romacs-notmuch--object value)))

(defun romacs-notmuch--identity (value)
  "Validate opaque message identity VALUE."
  (romacs-notmuch--string value t)
  (when (string-match "[\000-\037\177]" value)
    (error "Invalid Notmuch message identity"))
  value)

(defun romacs-notmuch-id-term (id)
  "Return an exact Notmuch query term for opaque message ID."
  (concat "id:\""
          (mapconcat (lambda (c) (if (= c 34) "\"\"" (char-to-string c)))
                     (romacs-notmuch--identity id) "") "\""))

(defun romacs-notmuch--thread-id (id)
  "Validate hexadecimal thread ID."
  (romacs-notmuch--string id t)
  (unless (string-match "\\`[0-9a-f]+\\'" id)
    (error "Invalid Notmuch thread identity"))
  id)

(defun romacs-notmuch--argv (argv)
  "Validate and copy command ARGV, including each string."
  (unless (consp (romacs-notmuch--array argv))
    (error "Missing Notmuch command"))
  (romacs-notmuch--map (lambda (arg)
            (romacs-notmuch--string arg)
            (when (string-match "\000" arg) (error "Invalid command argument"))
            (copy-sequence arg)) argv))

(defun romacs-notmuch--summary (key row)
  "Normalize search ROW with account KEY."
  (romacs-notmuch--object row)
  (let* ((authors (romacs-notmuch--string (romacs-notmuch--get "authors" row)))
         (split (string-match "|" authors)))
    (list :thread (cons key (romacs-notmuch--thread-id (romacs-notmuch--get "thread" row)))
          :date-label (romacs-notmuch--string (romacs-notmuch--get "date_relative" row))
          :timestamp (romacs-notmuch--number (romacs-notmuch--get "timestamp" row))
          :matched (romacs-notmuch--number (romacs-notmuch--get "matched" row))
          :total (romacs-notmuch--number (romacs-notmuch--get "total" row))
          :authors-matched (if split (substring authors 0 split) authors)
          :authors-other (if split
                             (let ((other (substring authors (1+ split))))
                               (if (string-match "\\` +" other)
                                   (substring other (match-end 0)) other))
                           "")
          :subject (romacs-notmuch--string (romacs-notmuch--get "subject" row))
          :tags (romacs-notmuch--tags (romacs-notmuch--get "tags" row)))))

(defun romacs-notmuch--tree-row (key mail parent depth lasts)
  "Normalize tree MAIL with KEY, PARENT, DEPTH and LASTS.
LASTS includes the current row: its length is DEPTH plus one, root first."
  (romacs-notmuch--object mail)
  (let ((headers (romacs-notmuch--headers (romacs-notmuch--get "headers" mail))))
    (list :key (cons key (romacs-notmuch--identity (romacs-notmuch--get "id" mail)))
          :parent-id parent :depth depth :lasts lasts
          :match (romacs-notmuch--boolean (romacs-notmuch--get "match" mail))
          :excluded (romacs-notmuch--boolean (romacs-notmuch--get "excluded" mail))
          :from (romacs-notmuch--header "From" headers t)
          :date-label (romacs-notmuch--string (romacs-notmuch--get "date_relative" mail))
          :timestamp (romacs-notmuch--number (romacs-notmuch--get "timestamp" mail))
          :subject (romacs-notmuch--header "Subject" headers t)
          :tags (romacs-notmuch--tags (romacs-notmuch--get "tags" mail)))))

(defun romacs-notmuch--trees (key nodes parent depth lasts)
  "Flatten sibling NODES for KEY retaining PARENT, DEPTH and LASTS."
  (romacs-notmuch--array nodes)
  (when (and nodes (> depth 64)) (error "Notmuch tree nesting exceeds 64"))
  (let ((rows nil))
    (while nodes
      (let* ((tuple (romacs-notmuch--array (car nodes)))
             (mail (car tuple))
             (path (append lasts (list (null (cdr nodes)))))
             (placeholder (memq mail '(nil json-null)))
             (row (unless placeholder (romacs-notmuch--tree-row key mail parent depth path))))
        (unless (= (length tuple) 2) (error "Invalid Notmuch tree tuple"))
        (when row (setq rows (cons row rows)))
        (dolist (child (romacs-notmuch--trees key (nth 1 tuple)
                                            (if row (cdr (plist-get row :key)) parent)
                                            (1+ depth) path))
          (setq rows (cons child rows))))
      (setq nodes (cdr nodes)))
    (nreverse rows)))

(defun romacs-notmuch--forest (key forest)
  "Normalize FOREST for KEY without losing thread boundaries."
  (apply #'append
         (romacs-notmuch--map (lambda (thread)
                   (let ((rows (romacs-notmuch--trees key thread nil 0 nil)))
                     (if rows
                         (cons (append (car rows) '(:thread-start t)) (cdr rows))
                       nil)))
                 (romacs-notmuch--array forest))))

(defun romacs-notmuch--parts (parts depth)
  "Normalize MIME PARTS at DEPTH; HTML stays inert."
  (when (> depth 32) (error "Notmuch MIME nesting exceeds 32"))
  (romacs-notmuch--map
   (lambda (part)
     (romacs-notmuch--object part)
     (let* ((type (romacs-notmuch--string (romacs-notmuch--get "content-type" part) t))
            (content (romacs-notmuch--get "content" part))
            (multipart (and (>= (length type) 10) (equal (substring type 0 10) "multipart/")))
            (disposition (romacs-notmuch--get "content-disposition" part))
            (filename (romacs-notmuch--get "filename" part))
            (size (romacs-notmuch--get "content-length" part)))
       (when (and (equal type "text/plain") content
                  (not (eq content 'json-null)) (not (stringp content)))
         (error "Invalid Notmuch plain text content"))
       (list :id (romacs-notmuch--number (romacs-notmuch--get "id" part))
             :type type
             :disposition (when disposition (romacs-notmuch--string disposition))
             :filename (when filename (romacs-notmuch--string filename))
             :size (when size (romacs-notmuch--number size))
             :content (when (and (equal type "text/plain") (stringp content)) content)
             :children (when multipart (romacs-notmuch--parts content (1+ depth))))))
   (romacs-notmuch--array parts)))

(defun romacs-notmuch--message (key id mail)
  "Normalize exact MAIL object for account KEY and requested ID."
  (romacs-notmuch--object mail)
  (unless (and (assoc "body" mail) (assoc "tags" mail))
    (error "Notmuch message is missing body or tags"))
  (unless (equal id (romacs-notmuch--identity (romacs-notmuch--get "id" mail)))
    (error "Notmuch returned a different message identity"))
  (list :key (cons key id)
        :headers (romacs-notmuch--headers (romacs-notmuch--get "headers" mail))
        :tags (romacs-notmuch--tags (romacs-notmuch--get "tags" mail))
        :parts (romacs-notmuch--parts (romacs-notmuch--get "body" mail) 0)))

(defun romacs-notmuch--quote (parts)
  "Return plain quote text from normalized PARTS, preferring plain alternatives."
  (mapconcat
   (lambda (part)
     (let ((type (plist-get part :type)) (children (plist-get part :children)))
       (cond ((equal (plist-get part :disposition) "attachment") "")
             ((equal type "text/plain") (or (plist-get part :content) ""))
             ((equal type "multipart/alternative")
              (let ((plain (catch 'plain
                             (mapc (lambda (child)
                                     (when (equal (plist-get child :type) "text/plain")
                                       (throw 'plain child))) children)
                             nil)))
                (if plain (romacs-notmuch--quote (list plain))
                  (romacs-notmuch--quote children))))
             (children (romacs-notmuch--quote children))
             ((equal type "text/html") "[HTML message not quoted]")
             (t "")))) parts "\n"))

(defun romacs-notmuch--reply (key id json)
  "Normalize authoritative reply JSON for KEY and source ID."
  (romacs-notmuch--object json)
  (let* ((original (romacs-notmuch--message key id (romacs-notmuch--get "original" json)))
         (headers (romacs-notmuch--headers (romacs-notmuch--get "reply-headers" json)))
         (source-headers (plist-get original :headers))
         (in-reply-to (romacs-notmuch--header "In-reply-to" headers t))
         (references (romacs-notmuch--header "References" headers t)))
    (unless (equal in-reply-to (concat "<" id ">"))
      (error "Notmuch reply has a different source identity"))
    (list :source (cons key id)
          :headers (romacs-notmuch--map (lambda (name) (cons name (romacs-notmuch--header name headers (not (equal name "Cc")))))
                           '("From" "To" "Cc" "Subject"))
          :in-reply-to in-reply-to :references references
          :quote-attribution (format "On %s, %s wrote:"
                                     (romacs-notmuch--header "Date" source-headers)
                                     (romacs-notmuch--header "From" source-headers t))
          :quote-text (romacs-notmuch--quote (plist-get original :parts)))))

(defun romacs-notmuch-account ()
  "Capture a Notmuch account and return its pure request builders.
Each builder returns (ARGV . NORMALIZER).  Commands and account identity are
copied now, never read from ambient configuration by retained callbacks."
  (let* ((command (romacs-notmuch--argv romacs-notmuch-command))
         (key (list 'notmuch (romacs-notmuch--argv command))))
    (list :name "notmuch" :key key :capabilities '(:search :tree :message :reply)
          :from (when romacs-notmuch-from (copy-sequence (romacs-notmuch--string romacs-notmuch-from)))
          :send (when romacs-notmuch-send-command (romacs-notmuch--argv romacs-notmuch-send-command))
          :search
          (lambda (query offset limit order)
            (romacs-notmuch--string query)
            (romacs-notmuch--number offset)
            (romacs-notmuch--number limit)
            (unless (and (> limit 0) (memq order '(newest oldest)))
              (error "Invalid Notmuch search bounds or order"))
            (cons (append (romacs-notmuch--argv command)
                          (list "search" "--format=json" "--format-version=5" "--output=summary"
                                (if (eq order 'oldest) "--sort=oldest-first" "--sort=newest-first")
                                (format "--offset=%d" offset) (format "--limit=%d" limit) "--" query))
                  (lambda (json)
                    (romacs-notmuch--map (lambda (row) (romacs-notmuch--summary key row))
                            (romacs-notmuch--array json)))))
          :tree
          (lambda (thread query)
            (romacs-notmuch--thread-id thread)
            (when query (romacs-notmuch--string query))
            (cons (append (romacs-notmuch--argv command)
                          (list "show" "--format=json" "--format-version=5" "--body=false"
                                "--entire-thread=true" "--exclude=true" "--decrypt=false" "--"
                                (concat "thread:" thread (if query (concat " and (" query ")") ""))))
                  (lambda (json) (romacs-notmuch--forest key json))))
          :message
          (lambda (id)
            (let ((id (copy-sequence (romacs-notmuch--identity id))))
              (cons (append (romacs-notmuch--argv command)
                            (list "show" "--format=json" "--format-version=5" "--part=0"
                                  "--exclude=false" "--body=true" "--decrypt=false" "--"
                                  (romacs-notmuch-id-term id)))
                    (lambda (json) (romacs-notmuch--message key id json)))))
          :reply
          (lambda (id all-p)
            (let ((id (copy-sequence (romacs-notmuch--identity id))))
              (cons (append (romacs-notmuch--argv command)
                            (list "reply" "--format=json" "--format-version=5"
                                  (if all-p "--reply-to=all" "--reply-to=sender")
                                  "--decrypt=false" "--" (romacs-notmuch-id-term id)))
                    (lambda (json) (romacs-notmuch--reply key id json))))))))

(defun notmuch ()
  "Search the inbox without changing the index."
  (interactive)
  (notmuch-search "tag:inbox"))

(defun notmuch-search (query)
  "Search QUERY using a freshly captured Notmuch account."
  (interactive (list (read-string "Notmuch search: " nil 'romacs-notmuch-search-history)))
  (romacs-mail-search (romacs-notmuch-account) query))

(defun notmuch-tree ()
  "Open a tree retaining the current mail query and message identity."
  (interactive)
  (romacs-mail-tree-current-query))

(defun notmuch-tree-from-show-current-query ()
  "Open a tree retaining the current message and query context."
  (interactive)
  (romacs-mail-tree-current-query))

(defun notmuch-mua-new-mail (&optional to subject)
  "Compose editable plain-text mail addressed to TO with SUBJECT."
  (interactive)
  (romacs-mail-compose (romacs-notmuch-account) to subject))

(defun notmuch-show-reply ()
  "Reply to all recipients of the exact current message."
  (interactive)
  (romacs-mail-reply t))

(defun notmuch-show-reply-sender ()
  "Reply to the sender of the exact current message."
  (interactive)
  (romacs-mail-reply nil))

(defun romacs-notmuch-reply ()
  "Reply to the sender of the exact current message."
  (interactive)
  (notmuch-show-reply-sender))

(provide 'romacs-notmuch)
;;; romacs-notmuch.el ends here
