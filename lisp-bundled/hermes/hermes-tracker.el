;;; hermes-tracker.el --- Tracker addon for Hermes  -*- lexical-binding: t; -*-

;; Copyright (C) 2026  Thanos Apollo

;; Author: Thanos Apollo <public@thanosapollo.org>
;; Version: 0.1.0
;; URL: https://git.thanosapollo.org/emacs-hermes-tracker
;; Keywords: tools, convenience
;; Package-Requires: ((emacs "29.1") (hermes "0.2.0") (keymap-popup "0.4.0"))

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

;; Emacs-native client for the separate Hermes Tracker service.
;; Tracker credentials come from auth-source and never share dashboard auth.
;; Repository/TODO browsing and safe mutations are asynchronous.  This module
;; also adds Tracker navigation to the Hermes dashboard and Kanban task details.

;;; Code:

(require 'button)
(require 'cl-lib)
(require 'ewoc)
(require 'json)
(require 'subr-x)
(require 'tabulated-list)
(require 'url-util)
(require 'keymap-popup)
(require 'hermes)
(require 'hermes-kanban)
(require 'hermes-tracker-api)
(require 'hermes-tracker-db)
(require 'hermes-tracker-filter)
(require 'hermes-tracker-sync)
(require 'hermes-tracker-vc)

(declare-function read-string-from-buffer "string-edit")
(declare-function hermes-tracker-sync-todo "hermes-tracker-sync" (slug number))


(defface hermes-tracker-number-face
  '((t :inherit shadow))
  "Face for Tracker TODO numbers."
  :group 'hermes-tracker)

(defface hermes-tracker-open-face
  '((t :inherit hermes-browser-pending))
  "Face for open Tracker TODOs."
  :group 'hermes-tracker)

(defface hermes-tracker-closed-face
  '((t :inherit shadow))
  "Face for closed Tracker TODOs."
  :group 'hermes-tracker)

(defface hermes-tracker-profile-face
  '((t :inherit font-lock-constant-face :weight bold))
  "Face for Tracker profile identities."
  :group 'hermes-tracker)

(defface hermes-tracker-event-face
  '((t :inherit shadow :slant italic))
  "Face for Tracker timeline events."
  :group 'hermes-tracker)

(defface hermes-tracker-separator-face
  '((default :inherit org-hide)
    (((background light)) :strike-through "gray70")
    (t :strike-through "gray30"))
  "Face for separators in Tracker TODO details."
  :group 'hermes-tracker)

(defface hermes-tracker-open-count-face
  '((t :inherit hermes-tracker-open-face))
  "Face for open TODO counts."
  :group 'hermes-tracker)

(defface hermes-tracker-closed-count-face
  '((t :inherit shadow))
  "Face for closed TODO counts."
  :group 'hermes-tracker)


(defcustom hermes-tracker-markdown-mode
  (cond
   ((fboundp 'gfm-mode) 'gfm-mode)
   ((fboundp 'markdown-ts-mode) 'markdown-ts-mode)
   (t 'text-mode))
  "Major mode used to fontify Tracker TODO and comment bodies."
  :type '(choice (function-item gfm-mode)
                 (function-item markdown-ts-mode)
                 (function-item text-mode)
                 (function :tag "Other"))
  :group 'hermes-tracker)

(defconst hermes-tracker--reference-marker "```tracker-ref"
  "Raw marker that reserves a Tracker reference fence.")

(defconst hermes-tracker--todo-statuses '("open" "closed")
  "TODO statuses supported by the Tracker API.")

(defconst hermes-tracker--todo-status-faces
  '(("open" . hermes-tracker-open-face)
    ("closed" . hermes-tracker-closed-face))
  "Semantic faces for Tracker TODO statuses.")

(defconst hermes-tracker--reference-regexp
  "\\(?:\\`\\|\n\\)\\(```tracker-ref\n\\([^\n]+\\)\n```\\)\\(?:\\'\\|\n\\)"
  "Regexp matching one exact canonical Tracker reference fence.")

(defvar-local hermes-tracker--repo-slug nil)
(defvar-local hermes-tracker--repo-name nil)
(defvar-local hermes-tracker--status-filter "open")
(defvar-local hermes-tracker--filters nil)
(defvar-local hermes-tracker--filter-query "")
(defvar-local hermes-tracker--todo-number nil)
(defvar-local hermes-tracker--todo nil)
(defvar-local hermes-tracker--linked-cards nil)
(defvar-local hermes-tracker--todo-ewoc nil)
(defvar-local hermes-tracker--todo-nodes nil)

(defvar hermes-tracker--repositories-request-id 0)
(defvar hermes-tracker--todos-request-id 0)
(defvar hermes-tracker--todo-request-id 0)

(defun hermes-tracker--begin-request (slot)
  "Advance request identity SLOT and return its new value."
  (let ((request-id (1+ (symbol-value slot))))
    (set slot request-id)
    request-id))

(defun hermes-tracker--request-current-p (slot request-id)
  "Return non-nil when REQUEST-ID is the latest value of SLOT."
  (= request-id (symbol-value slot)))

(defun hermes-tracker--request-applicable-p
    (owner mode slot request-id)
  "Return non-nil when SLOT REQUEST-ID may update OWNER in MODE."
  (and (hermes-tracker--request-current-p slot request-id)
       (or (null owner)
           (and (buffer-live-p owner)
                (with-current-buffer owner
                  (derived-mode-p mode))))))


(defun hermes-tracker--display (object field &optional fallback)
  "Return OBJECT FIELD as a display string, or FALLBACK."
  (or (hermes-transport--non-empty-string
       (hermes-transport--display-field object field))
      fallback ""))

(defun hermes-tracker--count-value (counts status)
  "Return non-negative integer COUNTS value for STATUS, or nil."
  (let ((value (hermes-transport--get counts status)))
    (cond
     ((and (integerp value) (>= value 0)) value)
     ((and (stringp value) (string-match-p "\\`[0-9]+\\'" value))
      (string-to-number value)))))

(defun hermes-tracker--repository-count-cell (counts status face)
  "Return COUNTS value for STATUS styled with FACE."
  (let ((count (hermes-tracker--count-value counts status)))
    (hermes-browser--face-cell
     (if (integerp count) count "-")
     (if (and (integerp count) (> count 0)) face 'shadow))))

(defun hermes-tracker--repository-record (repository)
  "Return plain normalized record for REPOSITORY."
  (if (and (listp repository)
           (plist-get repository :tracker-repository))
      repository
    (let ((slug (hermes-tracker--display repository 'slug))
          (summary (hermes-transport--get repository 'summary)))
      (when (string-empty-p slug)
        (error "Tracker repository is missing slug"))
      (list :tracker-repository t
            :slug slug
            :name (hermes-tracker--display repository 'name slug)
            :description (hermes-tracker--display repository 'description)
            :summary-present-p
            (hermes-transport--field-present-p repository 'summary)
            :counts (and summary
                         (hermes-transport--get summary 'status_counts))))))

(defun hermes-tracker--todo-status-counts (payload)
  "Return status counts computed from TODO-list PAYLOAD."
  (unless (hermes-transport--field-present-p payload 'todos)
    (error "Tracker TODO-list response is missing todos"))
  (let* ((todos (hermes-tracker--items
                 (hermes-transport--get payload 'todos)))
         (statuses (mapcar (lambda (todo)
                            (hermes-tracker--display todo 'status))
                          todos)))
    (unless (seq-every-p
             (lambda (status) (member status hermes-tracker--todo-statuses))
             statuses)
      (error "Tracker TODO list contains an invalid status"))
    (mapcar (lambda (status)
              (cons status (cl-count status statuses :test #'string=)))
            hermes-tracker--todo-statuses)))

(defun hermes-tracker--fetch-repository-counts (repository)
  "Return promise of normalized REPOSITORY with status counts."
  (let ((record (hermes-tracker--repository-record repository)))
    (if (plist-get record :summary-present-p)
        (hermes--promise-resolved record)
      (hermes--promise-map
       (hermes-tracker--request-async
        "GET" (concat (hermes-tracker--api-path
                       "repos" (plist-get record :slug) "todos")
                      "?state=all"))
       (lambda (payload)
         (plist-put (copy-sequence record) :counts
                    (hermes-tracker--todo-status-counts payload)))))))

(defun hermes-tracker--fetch-repositories ()
  "Return promise of repository records with real status counts."
  (hermes--promise-map
   (hermes-tracker--request-async "GET" (hermes-tracker--api-path "repos"))
   (lambda (repositories)
     (hermes--promise-all
      (mapcar #'hermes-tracker--fetch-repository-counts
              (hermes-tracker--items repositories))))))

(defun hermes-tracker--repository-rows (repositories)
  "Return tabulated rows for REPOSITORIES."
  (mapcar (lambda (repo)
            (let* ((record (hermes-tracker--repository-record repo))
                   (slug (plist-get record :slug))
                   (counts (plist-get record :counts)))
              (list slug (vector (hermes-browser--face-cell
                                  (plist-get record :name)
                                  'hermes-browser-name)
                                 (hermes-tracker--repository-count-cell
                                  counts 'open 'hermes-tracker-open-count-face)
                                 (hermes-tracker--repository-count-cell
                                  counts 'closed 'hermes-tracker-closed-count-face)
                                 (hermes-browser--face-cell
                                  (plist-get record :description)
                                  'hermes-browser-description)))))
          (hermes-tracker--items repositories)))

(defun hermes-tracker--todo-rows (payload)
  "Return tabulated TODO rows from PAYLOAD."
  (mapcar (lambda (todo)
            (let ((number (hermes-transport--get todo 'number)))
              (list number
                    (vector (hermes-browser--face-cell
                             number 'hermes-browser-identifier)
                            (hermes-browser--status-cell
                             (hermes-tracker--display todo 'status)
                             'hermes-browser-status)
                            (hermes-browser--face-cell
                             (hermes-tracker--display todo 'priority "0")
                             'hermes-browser-priority)
                            (hermes-browser--face-cell
                             (hermes-tracker--display todo 'assignee "-")
                             'hermes-browser-assignee)
                            (hermes-browser--face-cell
                             (hermes-tracker--display todo 'title)
                             'hermes-browser-title)))))
          (hermes-tracker--items (hermes-transport--get payload 'todos))))

(defun hermes-tracker--format-link (link)
  "Return one TODO LINK row."
  (format "%s %s#%s - %s"
          (hermes-tracker--display link 'link_type)
          (hermes-tracker--display link 'target_repo_slug)
          (hermes-tracker--display link 'target_number)
          (hermes-tracker--display link 'target_title)))

(defun hermes-tracker--list-body (values)
  "Return Markdown list for VALUES."
  (string-join
   (mapcar (lambda (item) (format "- %s" item))
           (hermes-tracker--items values))
   "\n"))

(defun hermes-tracker--relative-time (value)
  "Return a compact relative description of timestamp VALUE."
  (condition-case nil
      (let ((seconds (float-time (time-subtract nil (date-to-time value)))))
        (cond
         ((< seconds 60) "just now")
         ((< seconds 3600) (format "%dm ago" (floor (/ seconds 60))))
         ((< seconds 86400) (format "%dh ago" (floor (/ seconds 3600))))
         ((< seconds 604800) (format "%dd ago" (floor (/ seconds 86400))))
         ((< seconds 2592000) (format "%dw ago" (floor (/ seconds 604800))))
         (t (format-time-string "%Y-%m-%d" (date-to-time value)))))
    (error (or value ""))))

(defun hermes-tracker--fontify-markdown (text)
  "Return TEXT with Markdown fontification when available."
  (if (stringp text)
      (with-temp-buffer
        (insert text)
        (delay-mode-hooks (funcall hermes-tracker-markdown-mode))
        (font-lock-ensure)
        (buffer-string))
    ""))

(defun hermes-tracker--todo-author (todo)
  "Return the best available author identity for TODO."
  (or (hermes-transport--non-empty-string
       (hermes-tracker--display todo 'author))
      "tracker"))

(defun hermes-tracker--todo-body (todo)
  "Return TODO durable content as one Markdown body."
  (let ((criteria (hermes-tracker--items
                   (hermes-transport--get todo 'acceptance_criteria)))
        (commands (hermes-tracker--items
                   (hermes-transport--get todo 'verification_commands)))
        (evidence (hermes-transport--non-empty-string
                   (hermes-tracker--display todo 'verification_output)))
        (links (hermes-tracker--items
                (hermes-transport--get todo 'links))))
    (string-join
     (delq nil
           (list
            (or (hermes-transport--non-empty-string
                 (hermes-tracker--display todo 'description))
                "No description.")
            (and criteria
                 (format "Acceptance criteria:\n%s"
                         (hermes-tracker--list-body criteria)))
            (and commands
                 (format "Verification commands:\n%s"
                         (hermes-tracker--list-body commands)))
            (and evidence (format "Verification evidence:\n%s" evidence))
            (and links
                 (format "Links:\n%s"
                         (hermes-tracker--list-body
                          (mapcar #'hermes-tracker--format-link links))))))
     "\n\n")))

(defun hermes-tracker--comment-node (comment index)
  "Return timeline node for COMMENT at fallback INDEX."
  (list :id (list 'comment
                  (or (hermes-transport--get comment 'id) index))
        :type 'comment
        :author (hermes-tracker--display comment 'author "anon")
        :body (hermes-tracker--display comment 'body)
        :created-at (hermes-tracker--display comment 'created_at nil)))

(defun hermes-tracker--card-node (card actor)
  "Return timeline node for linked CARD attributed to ACTOR."
  (list :id (list 'event 'card (plist-get card :task-id))
        :type 'event
        :event-type "linked Kanban card"
        :actor actor
        :created-at (plist-get card :created-at)
        :card card))

(defun hermes-tracker--closure-node (todo)
  "Return closure timeline node for TODO, or nil."
  (let ((actor (hermes-transport--non-empty-string
                (hermes-tracker--display todo 'closed_by)))
        (created (hermes-transport--non-empty-string
                  (hermes-tracker--display todo 'closed_at)))
        (commit (hermes-transport--non-empty-string
                 (hermes-tracker--display todo 'closing_commit))))
    (when (or actor created commit)
      (list :id '(event closure)
            :type 'event
            :event-type "closed"
            :actor (or actor (hermes-tracker--todo-author todo))
            :created-at created
            :detail commit))))

(defun hermes-tracker--timeline-node-time (node)
  "Return sortable timestamp for timeline NODE, or nil."
  (when-let* ((value (plist-get node :created-at)))
    (condition-case nil
        (float-time (date-to-time value))
      (error nil))))

(defun hermes-tracker--timeline-entry-less-p (left right)
  "Return non-nil when timeline entry LEFT precedes RIGHT."
  (let ((left-time (hermes-tracker--timeline-node-time (cdr left)))
        (right-time (hermes-tracker--timeline-node-time (cdr right))))
    (cond
     ((and left-time right-time)
      (if (= left-time right-time)
          (< (car left) (car right))
        (< left-time right-time)))
     (left-time t)
     (right-time nil)
     (t (< (car left) (car right))))))

(defun hermes-tracker--sort-timeline (nodes)
  "Return timeline NODES in chronological order."
  (mapcar
   #'cdr
   (sort (cl-loop for node in nodes
                  for index from 0
                  collect (cons index node))
         #'hermes-tracker--timeline-entry-less-p)))

(defun hermes-tracker--todo-nodes (todo cards)
  "Return header and chronological timeline nodes for TODO and CARDS."
  (let* ((comments (hermes-tracker--items
                    (hermes-transport--get todo 'comments)))
         (author (hermes-tracker--todo-author todo))
         (timeline
          (append
           (cl-loop for comment in comments
                    for index from 0
                    collect (hermes-tracker--comment-node comment index))
           (mapcar (lambda (card)
                     (hermes-tracker--card-node card author))
                   cards)
           (delq nil (list (hermes-tracker--closure-node todo))))))
    (cons (list :id 'header :type 'header :todo todo)
          (hermes-tracker--sort-timeline timeline))))

(defun hermes-tracker-parse-reference (body)
  "Return canonical Tracker reference plist parsed from Kanban BODY."
  (when (stringp body)
    (let ((start 0) (markers 0))
      (while (string-match (regexp-quote hermes-tracker--reference-marker)
                           body start)
        (cl-incf markers)
        (setq start (match-end 0)))
      (when (and (= markers 1)
                 (string-match hermes-tracker--reference-regexp body))
        (condition-case nil
            (let* ((block (match-string 1 body))
                   (object (json-parse-string (match-string 2 body)
                                              :object-type 'alist))
                   (keys (mapcar #'car object))
                   (slug (cdr (assq 'repo_slug object)))
                   (number (cdr (assq 'number object))))
              (and (equal (sort keys (lambda (a b) (string-lessp (symbol-name a)
                                                                    (symbol-name b))))
                          '(number repo_slug))
                   (stringp slug) (not (string-empty-p (string-trim slug)))
                   (integerp number) (> number 0)
                   (equal block (hermes-tracker-render-reference slug number))
                   (list :repo-slug (string-trim slug) :number number)))
          (error nil))))))

(defun hermes-tracker-render-reference (repo-slug number)
  "Return canonical fenced reference for REPO-SLUG TODO NUMBER."
  (unless (and (stringp repo-slug) (not (string-empty-p (string-trim repo-slug)))
               (integerp number) (> number 0))
    (user-error "Tracker reference requires repository slug and positive TODO number"))
  (format "```tracker-ref\n%s\n```"
          (json-serialize `((number . ,number) (repo_slug . ,(string-trim repo-slug))))))

(defun hermes-tracker--kanban-references (todo)
  "Return TODO's canonical Hermes Kanban external references."
  (seq-filter
   (lambda (reference)
     (and (equal (hermes-tracker--display reference 'system) "hermes-kanban")
          (string-match-p "\\`t_[0-9a-f]\\{8\\}\\'"
                          (hermes-tracker--display reference 'external_id))))
   (hermes-tracker--items (hermes-transport--get todo 'external_references))))

(defun hermes-tracker--supersession (todo task-id)
  "Return TASK-ID supersession record from TODO."
  (seq-find (lambda (item)
              (equal (hermes-tracker--display item 'task_id) task-id))
            (hermes-tracker--items
             (hermes-transport--get todo 'superseded_kanban_executions))))

(defun hermes-tracker--linked-card (todo reference payload)
  "Return normalized linked card from TODO REFERENCE and Kanban PAYLOAD."
  (let* ((task-id (hermes-tracker--display reference 'external_id))
         (metadata (hermes-transport--get reference 'metadata))
         (task (and payload (hermes-transport--get payload 'task)))
         (card-ref (and task (hermes-tracker-parse-reference
                              (hermes-tracker--display task 'body))))
         (expected (list :repo-slug (hermes-tracker--display todo 'repo_slug)
                         :number (hermes-transport--get todo 'number)))
         (supersession (hermes-tracker--supersession todo task-id))
         (drift (cond ((null task) 'missing-card)
                      ((null card-ref) 'missing-card-backlink)
                      ((not (equal card-ref expected)) 'mismatched-card-backlink)
                      (t 'none))))
    (list :task-id task-id
          :board (hermes-tracker--display metadata 'board "default")
          :profile (hermes-tracker--display metadata 'profile "-")
          :created-at (hermes-tracker--display reference 'created_at nil)
          :title (and task (hermes-tracker--display task 'title task-id))
          :status (and task (hermes-tracker--display task 'status))
          :assignee (and task (hermes-tracker--display task 'assignee "-"))
          :payload payload :drift drift
          :execution-state (and supersession 'superseded)
          :replacement-id (and supersession
                               (hermes-tracker--display supersession
                                                        'replacement_kanban_id nil)))))

(defun hermes-tracker--linked-card-line (card)
  "Return display line for linked CARD."
  (let ((drift (plist-get card :drift)))
    (format "%s  [%s] %s  %s  %s%s%s"
            (plist-get card :task-id) (plist-get card :board)
            (or (plist-get card :title) "unavailable")
            (or (plist-get card :status) "missing")
            (or (plist-get card :assignee) (plist-get card :profile) "-")
            (if (and drift (not (eq drift 'none)))
                (format "  drift: %s" drift)
              "")
            (if (plist-get card :execution-state)
                (format "  superseded%s"
                        (if-let* ((replacement (plist-get card :replacement-id)))
                            (format " by %s" replacement) ""))
              ""))))

(defun hermes-tracker--not-found-reason-p (reason)
  "Return non-nil when REASON confirms an HTTP 404 response."
  (and (stringp reason)
       (string-match-p "(HTTP 404)" reason)))

(defun hermes-tracker--fetch-dashboard-task (id board)
  "Return a promise resolving task ID on BOARD, or nil when absent."
  (hermes--promise-catch
   (hermes-dashboard-transport-api-request-async
    "GET" (concat "/api/plugins/kanban/tasks/" (url-hexify-string id))
    :query `((board . ,board)))
   (lambda (reason)
     (if (hermes-tracker--not-found-reason-p reason)
         nil
       (hermes--promise-rejected reason)))))

(defun hermes-tracker--board-task-result (id board)
  "Return a promise of one BOARD lookup result for task ID."
  (hermes--promise-catch
   (hermes--promise-map
    (hermes-tracker--fetch-dashboard-task id board)
    (lambda (payload) (list :match (and payload (cons board payload)))))
   (lambda (reason) (list :error reason))))

(defun hermes-tracker--select-board-task-result (results)
  "Return the match in RESULTS, reject its first error, or return nil."
  (let ((match (seq-some (lambda (result) (plist-get result :match)) results))
        (reason (seq-some (lambda (result) (plist-get result :error)) results)))
    (cond (match match)
          (reason (hermes--promise-rejected reason)))))

(defun hermes-tracker--find-dashboard-task (id stale-board)
  "Return a promise resolving (BOARD . PAYLOAD) for ID outside STALE-BOARD."
  (hermes--promise-then
   (hermes-dashboard-transport-api-request-async
    "GET" "/api/plugins/kanban/boards")
   (lambda (payload)
     (hermes--promise-map
     (hermes--promise-all
       (mapcar
        (lambda (board)
          (hermes-tracker--board-task-result id board))
        (seq-remove
         (lambda (board) (equal board stale-board))
         (mapcar (lambda (board)
                   (hermes-tracker--display board 'slug))
                 (hermes-tracker--items
                  (hermes-transport--get payload 'boards))))))
      #'hermes-tracker--select-board-task-result))))

(defun hermes-tracker--linked-card-at-board (todo reference payload board)
  "Return TODO REFERENCE card data from PAYLOAD found on BOARD."
  (let ((card (hermes-tracker--linked-card todo reference payload)))
    (if (and payload (not (equal board (plist-get card :board))))
        (plist-put
         (plist-put card :board board) :drift
         (if (eq (plist-get card :drift) 'none)
             'stale-board
           (plist-get card :drift)))
      card)))

(defun hermes-tracker--fetch-linked-card (todo reference)
  "Return promise resolving TODO REFERENCE to a linked-card record."
  (let* ((id (hermes-tracker--display reference 'external_id))
         (metadata (hermes-transport--get reference 'metadata))
         (board (hermes-tracker--display metadata 'board "default")))
    (hermes--promise-then
     (hermes-tracker--fetch-dashboard-task id board)
     (lambda (payload)
       (if payload
           (hermes-tracker--linked-card todo reference payload)
         (hermes--promise-map
          (hermes-tracker--find-dashboard-task id board)
          (lambda (match)
            (hermes-tracker--linked-card-at-board
             todo reference (cdr match) (or (car match) board)))))))))

(defun hermes-tracker--linked-cards-for-todo (todo)
  "Return promise of Dashboard cards linked from TODO."
  (hermes--promise-all
   (mapcar (lambda (reference)
             (hermes-tracker--fetch-linked-card todo reference))
           (hermes-tracker--kanban-references todo))))

(defun hermes-tracker--fetch-todo-with-cards (repo number)
  "Return promise of (TODO . LINKED-CARDS) for REPO NUMBER."
  (hermes--promise-then
   (hermes-tracker--request-async "GET"
                                  (hermes-tracker--api-path "repos" repo "todos" number))
   (lambda (todo)
     (hermes--promise-map
      (hermes-tracker--linked-cards-for-todo todo)
      (lambda (cards) (cons todo cards))))))

(defun hermes-tracker--report-error (reason)
  "Report Tracker rejection REASON safely."
  (message "Hermes Tracker: %s" reason))

(defvar hermes-tracker-repositories-mode-map)
(keymap-popup-define hermes-tracker-repositories-mode-map
  "Keymap for Tracker repositories."
  :parent tabulated-list-mode-map :description "Hermes Tracker Repositories"
  :group "Navigate" "RET" ("Open repository" hermes-tracker-open-repository)
  :group "View" "g" ("Refresh" revert-buffer)
  "?" ("Help" hermes-tracker-repositories-mode-map-popup))

(define-derived-mode hermes-tracker-repositories-mode tabulated-list-mode
  "Tracker Repositories" "Major mode for Tracker repositories."
  :interactive nil
  (setq tabulated-list-format [("Repository" 28 t)
                               ("Open" 7 t)
                               ("Closed" 7 t)
                               ("Description" 50 t)]
        revert-buffer-function #'hermes-tracker--repositories-revert)
  (tabulated-list-init-header))

(defun hermes-tracker--display-list-state (text face)
  "Display tabulated-list state TEXT with FACE in the current buffer."
  (setq tabulated-list-entries nil)
  (tabulated-list-print t)
  (let ((inhibit-read-only t))
    (goto-char (point-min))
    (insert (propertize text 'face face) "\n")))

(defun hermes-tracker--display-repository-rows (repositories)
  "Display REPOSITORIES in the current repository buffer."
  (setq mode-line-process nil
        tabulated-list-entries (hermes-tracker--repository-rows repositories))
  (if tabulated-list-entries
      (tabulated-list-print t)
    (hermes-tracker--display-list-state "No Tracker repositories." 'shadow)))

(defun hermes-tracker--repository-buffer (&optional owner)
  "Return prepared repository OWNER or the standard repository buffer."
  (let ((buffer (or owner
                    (get-buffer-create "*Hermes Tracker Repositories*"))))
    (when (buffer-live-p buffer)
      (with-current-buffer buffer
        (unless (derived-mode-p 'hermes-tracker-repositories-mode)
          (hermes-tracker-repositories-mode))))
    buffer))

(defun hermes-tracker--render-repositories (repositories &optional owner)
  "Render REPOSITORIES in OWNER, or create and display their buffer."
  (let ((buffer (hermes-tracker--repository-buffer owner)))
    (when (buffer-live-p buffer)
      (with-current-buffer buffer
        (hermes-tracker--display-repository-rows repositories)
        (unless owner (pop-to-buffer (current-buffer)))))))

(defun hermes-tracker--render-cached-repositories (owner)
  "Render OWNER repository rows exclusively from SQLite."
  (hermes-tracker--render-repositories
   (hermes-tracker-db-list-repository-summaries
    (hermes-tracker-sync-cache-origin))
   owner))

(defun hermes-tracker--repositories-revert (&rest _)
  "Render cached repositories, then synchronize them in place."
  (let ((owner (current-buffer))
        (request-id (hermes-tracker--begin-request
                     'hermes-tracker--repositories-request-id)))
    (hermes-tracker--render-cached-repositories owner)
    (setq mode-line-process " [Syncing repositories]")
    (hermes--promise-then
     (hermes-tracker-sync-all)
     (lambda (_)
       (when (hermes-tracker--request-applicable-p
              owner 'hermes-tracker-repositories-mode
              'hermes-tracker--repositories-request-id request-id)
         (hermes-tracker--render-cached-repositories owner)))
     (lambda (reason)
       (when (hermes-tracker--request-applicable-p
              owner 'hermes-tracker-repositories-mode
              'hermes-tracker--repositories-request-id request-id)
         (with-current-buffer owner
           (setq mode-line-process " [Offline cache]"))
         (hermes-tracker--report-error reason))))))

;;;###autoload
(defun hermes-list-tracker-repositories ()
  "Browse configured Hermes Tracker repositories."
  (interactive)
  (let ((owner (hermes-tracker--repository-buffer)))
    (with-current-buffer owner
      (hermes-tracker--repositories-revert))
    (pop-to-buffer owner)
    owner))

(defun hermes-tracker-open-repository ()
  "Open selected Tracker repository's TODO list."
  (interactive)
  (let ((slug (or (tabulated-list-get-id) (user-error "No repository here")))
        (entry (tabulated-list-get-entry)))
    (hermes-tracker--render-todos slug (aref entry 0))))

(defvar hermes-tracker-todos-mode-map)
(keymap-popup-define hermes-tracker-todos-mode-map
  "Keymap for Tracker TODOs."
  :parent tabulated-list-mode-map :description "Hermes Tracker TODOs"
  :group "Navigate" "RET" ("Show TODO" hermes-tracker-show-todo)
  "b" ("Repositories" hermes-list-tracker-repositories)
  :group "TODO" "+" ("Create TODO" hermes-tracker-create-todo)
  "u" ("Update TODO" hermes-tracker-update-todo)
  "l" ("Set labels" hermes-tracker-set-labels)
  "c" ("Comment" hermes-tracker-comment)
  :group "Lifecycle" "a" ("Claim TODO" hermes-tracker-claim-todo)
  "r" ("Release TODO" hermes-tracker-release-todo)
  :group "View" "/" ("Filter" hermes-tracker-filter)
  "f" ("Filter status" hermes-tracker-filter-status)
  "g" ("Refresh" revert-buffer) "?" ("Help" hermes-tracker-todos-mode-map-popup))

(define-derived-mode hermes-tracker-todos-mode tabulated-list-mode
  "Tracker TODOs" "Major mode for one Tracker repository's TODOs."
  :interactive nil
  (setq tabulated-list-format [("#" 5 t) ("Status" 10 t) ("Pri" 5 t)
                               ("Assignee" 18 t) ("Title" 60 t)]
        revert-buffer-function #'hermes-tracker--todos-revert)
  (tabulated-list-init-header))

(defun hermes-tracker--validate-status-filter (status)
  "Return exact STATUS, or nil when it is absent or empty.
Signal `user-error' for any other value."
  (cond
   ((null status) nil)
   ((not (stringp status))
    (user-error "Unsupported Tracker TODO status: %s" status))
   ((string-empty-p status) nil)
   ((member status hermes-tracker--todo-statuses) status)
   (t (user-error "Unsupported Tracker TODO status: %s" status))))

(defun hermes-tracker--local-status-state (status)
  "Return SQLite state for optional local STATUS."
  (let ((state (or status "open")))
    (if (member state '("open" "closed" "all"))
        state
      (user-error "Unsupported Tracker TODO status: %s" state))))

(defun hermes-tracker--active-filters ()
  "Return current SQLite TODO filters."
  (or hermes-tracker--filters
      (list :state
            (hermes-tracker--local-status-state
             hermes-tracker--status-filter))))

(defun hermes-tracker--todo-list-path (slug status)
  "Return TODO-list path for SLUG and optional STATUS."
  (let ((status (hermes-tracker--validate-status-filter status)))
    (concat (hermes-tracker--api-path "repos" slug "todos")
            (if status
                (concat "?status_filter=" (url-hexify-string status))
              ""))))

(defun hermes-tracker--display-todos (payload slug name &optional owner)
  "Display TODO PAYLOAD for repository SLUG NAME in optional OWNER."
  (let ((buffer (or owner (get-buffer-create "*Hermes Tracker TODOs*"))))
    (when (buffer-live-p buffer)
      (with-current-buffer buffer
        (unless (derived-mode-p 'hermes-tracker-todos-mode)
          (hermes-tracker-todos-mode))
        (setq hermes-tracker--repo-slug slug
              hermes-tracker--repo-name name
              mode-line-process
              (format " [%s%s]" slug
                      (if (string-empty-p hermes-tracker--filter-query)
                          (format ":%s"
                                  (plist-get (hermes-tracker--active-filters)
                                             :state))
                        (format " | %s" hermes-tracker--filter-query)))
              tabulated-list-entries (hermes-tracker--todo-rows payload))
        (tabulated-list-print t)
        (unless owner (pop-to-buffer (current-buffer)))))))

(defun hermes-tracker--render-todos (slug name &optional owner)
  "Render cached SLUG NAME TODOs, then synchronize optional OWNER."
  (let* ((buffer (or owner (get-buffer-create "*Hermes Tracker TODOs*")))
         (filters (if (buffer-live-p buffer)
                      (with-current-buffer buffer
                        (if (derived-mode-p 'hermes-tracker-todos-mode)
                            (copy-tree (hermes-tracker--active-filters))
                          '(:state "open")))
                    '(:state "open")))
         (request-id (with-current-buffer buffer
                       (hermes-tracker--begin-request
                        'hermes-tracker--todos-request-id))))
    (hermes-tracker--display-todos
     `((todos . ,(hermes-tracker-db-query-todos
                  (hermes-tracker-sync-cache-origin) slug filters)))
     slug name buffer)
    (unless owner (pop-to-buffer buffer))
    (hermes--promise-then
     (hermes-tracker-sync-todos slug)
     (lambda (_)
       (when (hermes-tracker--request-applicable-p
              buffer 'hermes-tracker-todos-mode
              'hermes-tracker--todos-request-id request-id)
         (hermes-tracker--display-todos
          `((todos . ,(hermes-tracker-db-query-todos
                       (hermes-tracker-sync-cache-origin) slug filters)))
          slug name buffer)))
     (lambda (reason)
       (when (hermes-tracker--request-applicable-p
              buffer 'hermes-tracker-todos-mode
              'hermes-tracker--todos-request-id request-id)
         (hermes-tracker--report-error reason))))))

(defun hermes-tracker--todos-revert (&rest _)
  "Refresh current Tracker TODO list."
  (hermes-tracker--render-todos
   hermes-tracker--repo-slug hermes-tracker--repo-name (current-buffer)))

(defun hermes-tracker--render-cached-todo-buffer (buffer)
  "Render BUFFER TODO rows exclusively from SQLite."
  (when (buffer-live-p buffer)
    (with-current-buffer buffer
      (when (derived-mode-p 'hermes-tracker-todos-mode)
        (hermes-tracker--display-todos
         `((todos . ,(hermes-tracker-db-query-todos
                      (hermes-tracker-sync-cache-origin)
                      hermes-tracker--repo-slug
                      (hermes-tracker--active-filters))))
         hermes-tracker--repo-slug hermes-tracker--repo-name buffer)))))

(defun hermes-tracker--sync-changed (origin scope)
  "Refresh live SQLite-backed views after an ORIGIN SCOPE commit."
  (when (condition-case nil
            (equal origin (hermes-tracker-sync-cache-origin))
          (error nil))
    (let* ((repositories (get-buffer "*Hermes Tracker Repositories*"))
           (repository-view
            (and repositories
                 (with-current-buffer repositories
                   (derived-mode-p 'hermes-tracker-repositories-mode))))
           (todos (get-buffer "*Hermes Tracker TODOs*"))
           (detail (get-buffer "*Hermes Tracker TODO*")))
      (pcase scope
        (`(repos)
         (when repository-view
           (hermes-tracker--render-cached-repositories repositories)))
        (`(todos ,slug)
         (when repository-view
           (hermes-tracker--render-cached-repositories repositories))
         (when (and todos
                    (with-current-buffer todos
                      (equal hermes-tracker--repo-slug slug)))
           (hermes-tracker--render-cached-todo-buffer todos)))
        (`(todo ,slug ,number)
         (when (and detail
                    (with-current-buffer detail
                      (and (derived-mode-p 'hermes-tracker-todo-mode)
                           (equal hermes-tracker--repo-slug slug)
                           (equal hermes-tracker--todo-number number))))
           (hermes-tracker--render-cached-todo
            slug number detail
            (hermes-tracker--begin-request
             'hermes-tracker--todo-request-id))))))))

(add-hook 'hermes-tracker-sync-changed-hook #'hermes-tracker--sync-changed)

(defun hermes-tracker-filter-status (status)
  "Filter current TODO list by STATUS; empty input clears it."
  (interactive (list (completing-read "Status (empty for all): "
                                      (cons "" hermes-tracker--todo-statuses)
                                      nil t)))
  (setq-local hermes-tracker--status-filter
              (or (hermes-tracker--validate-status-filter status) "all")
              hermes-tracker--filters nil
              hermes-tracker--filter-query "")
  (revert-buffer nil t))

(defun hermes-tracker-filter (query)
  "Filter current cached TODO list using structured QUERY."
  (interactive (list (read-string "Tracker filter: "
                                  hermes-tracker--filter-query)))
  (let ((filters (hermes-tracker-filter-parse query)))
    (setq-local hermes-tracker--filters filters
                hermes-tracker--filter-query query
                hermes-tracker--status-filter (plist-get filters :state))
    (revert-buffer nil t)))

(defun hermes-tracker--number-for-command ()
  "Return selected/current TODO number."
  (or (and (derived-mode-p 'hermes-tracker-todo-mode) hermes-tracker--todo-number)
      (tabulated-list-get-id) (user-error "No Tracker TODO here")))

(defun hermes-tracker--context ()
  "Return current (REPO NUMBER REFRESH) command context."
  (let ((buffer (current-buffer)))
    (list (or hermes-tracker--repo-slug (user-error "No Tracker repository here"))
          (hermes-tracker--number-for-command)
          (lambda () (when (buffer-live-p buffer)
                       (with-current-buffer buffer (revert-buffer nil t)))))))

(defun hermes-tracker-show-todo ()
  "Show selected Tracker TODO with linked Kanban state."
  (interactive)
  (hermes-tracker-open-todo hermes-tracker--repo-slug
                            (hermes-tracker--number-for-command)))

(defun hermes-tracker--render-cached-todo (repo number owner request-id)
  "Render cached REPO TODO NUMBER in OWNER for REQUEST-ID."
  (when-let* ((cached (hermes-tracker-db-get-todo
                       (hermes-tracker-sync-cache-origin) repo number)))
    (let ((todo (copy-tree cached)))
      (setf (alist-get 'repo_slug todo) repo)
      (hermes-tracker--display-todo (cons todo nil) owner)
      (hermes--promise-then
       (hermes-tracker--linked-cards-for-todo todo)
       (lambda (cards)
         (when (hermes-tracker--request-applicable-p
                owner 'hermes-tracker-todo-mode
                'hermes-tracker--todo-request-id request-id)
           (hermes-tracker--display-todo (cons todo cards) owner)))
       (lambda (reason)
         (when (hermes-tracker--request-applicable-p
                owner 'hermes-tracker-todo-mode
                'hermes-tracker--todo-request-id request-id)
           (hermes-tracker--report-error reason))))
      t)))

(defun hermes-tracker--display-missing-cached-todo (repo number owner)
  "Display missing-cache state for REPO TODO NUMBER in OWNER."
  (let ((buffer (or owner (get-buffer-create "*Hermes Tracker TODO*"))))
    (when (buffer-live-p buffer)
      (with-current-buffer buffer
        (unless (derived-mode-p 'hermes-tracker-todo-mode)
          (hermes-tracker-todo-mode))
        (setq hermes-tracker--repo-slug repo
              hermes-tracker--todo-number number
              hermes-tracker--todo nil
              hermes-tracker--linked-cards nil
              hermes-tracker--todo-ewoc nil
              hermes-tracker--todo-nodes (make-hash-table :test #'equal)
              mode-line-process (format " [%s#%s]" repo number)
              header-line-format (format " %s#%s" repo number))
        (let ((inhibit-read-only t))
          (erase-buffer)
          (insert (format "No cached Tracker TODO %s#%s. Syncing…\n" repo number)))
        (unless owner (pop-to-buffer buffer))))))

(defun hermes-tracker-open-todo (repo number &optional owner)
  "Open cached Tracker TODO NUMBER in REPO, then synchronize OWNER."
  (let* ((target (or owner (get-buffer-create "*Hermes Tracker TODO*")))
         (request-id (hermes-tracker--begin-request
                      'hermes-tracker--todo-request-id)))
    (unless (hermes-tracker--render-cached-todo repo number target request-id)
      (hermes-tracker--display-missing-cached-todo repo number target))
    (unless owner (pop-to-buffer target))
    (hermes--promise-then
     (hermes-tracker-sync-todo repo number)
     #'ignore
     (lambda (reason)
       (when (hermes-tracker--request-applicable-p
              target 'hermes-tracker-todo-mode
              'hermes-tracker--todo-request-id request-id)
         (hermes-tracker--report-error reason))))))

(defun hermes-tracker-next ()
  "Move to the next Tracker TODO EWOC node."
  (interactive)
  (ewoc-goto-next hermes-tracker--todo-ewoc 1))

(defun hermes-tracker-previous ()
  "Move to the previous Tracker TODO EWOC node."
  (interactive)
  (ewoc-goto-prev hermes-tracker--todo-ewoc 1))

(defun hermes-tracker-next-link ()
  "Move to the next link in the Tracker TODO buffer."
  (interactive)
  (if-let* ((match (text-property-search-forward 'keymap nil nil t)))
      (goto-char (prop-match-beginning match))
    (message "No next link")))

(defun hermes-tracker-previous-link ()
  "Move to the previous link in the Tracker TODO buffer."
  (interactive)
  (if-let* ((match (text-property-search-backward 'keymap nil nil t)))
      (goto-char (prop-match-beginning match))
    (message "No previous link")))

(defvar hermes-tracker-todo-mode-map)
(keymap-popup-define hermes-tracker-todo-mode-map
  "Keymap for Tracker TODO detail."
  :parent special-mode-map :description "Hermes Tracker TODO"
  :group "Navigate"
  "n" ("Next" hermes-tracker-next)
  "p" ("Previous" hermes-tracker-previous)
  "b" ("TODO list" hermes-tracker-back-to-todos)
  "?" ("Help" hermes-tracker-todo-mode-map-popup)
  :group "Links" "RET" ("Follow link" push-button)
  "TAB" ("Next link" hermes-tracker-next-link)
  "<backtab>" ("Previous link" hermes-tracker-previous-link)
  "K" ("Open linked Kanban card" hermes-tracker-open-kanban)
  :group "TODO" "u" ("Update" hermes-tracker-update-todo)
  "l" ("Set labels" hermes-tracker-set-labels)
  "c" ("Comment" hermes-tracker-comment)
  "g" ("Refresh" revert-buffer)
  :group "Lifecycle" "a" ("Claim" hermes-tracker-claim-todo)
  "r" ("Release" hermes-tracker-release-todo))

(defun hermes-tracker--insert-separator ()
  "Insert a full-width separator."
  (insert "\n"
          (propertize " " 'display '(space :width text)
                      'face 'hermes-tracker-separator-face)
          "\n"))

(defun hermes-tracker--insert-markdown (text)
  "Insert Markdown TEXT with a trailing newline."
  (let ((text (string-trim-right (or text ""))))
    (unless (string-empty-p text)
      (insert (hermes-tracker--fontify-markdown text) "\n"))))

(defun hermes-tracker--format-state (state)
  "Propertize Tracker STATE like Forgejo issue state."
  (let ((state (format "%s" (or state ""))))
    (propertize state 'face
                (or (cdr (assoc state hermes-tracker--todo-status-faces))
                    'hermes-browser-status))))

(defun hermes-tracker--insert-edited-indicator ()
  "Insert a styled edited indicator."
  (insert " " (propertize "(edited)" 'face 'shadow)))

(defun hermes-tracker--todo-edited-p (todo)
  "Return non-nil when TODO has different creation and update times."
  (let ((created (hermes-tracker--display todo 'created_at))
        (updated (hermes-tracker--display todo 'updated_at)))
    (and (not (string-empty-p created))
         (not (string-empty-p updated))
         (not (string= created updated)))))

(defun hermes-tracker--insert-header-metadata (todo)
  "Insert Forgejo-style metadata for TODO."
  (let* ((state (hermes-tracker--display todo 'status "open"))
         (author (hermes-tracker--todo-author todo))
         (created (hermes-tracker--display todo 'created_at))
         (comments (hermes-tracker--items
                    (hermes-transport--get todo 'comments))))
    (insert (hermes-tracker--format-state state)
            "  "
            (propertize author 'face 'hermes-tracker-profile-face)
            " opened " (hermes-tracker--relative-time created))
    (when (hermes-tracker--todo-edited-p todo)
      (hermes-tracker--insert-edited-indicator))
    (insert (format "  [%d comments]\n" (length comments))
            "Repository: " (hermes-tracker--display todo 'repo_slug) "\n"
            "Priority: " (hermes-tracker--display todo 'priority "0") "\n")
    (when-let* ((assignee (hermes-transport--non-empty-string
                           (hermes-tracker--display todo 'assignee))))
      (insert "Assignees: " assignee "\n"))))

(defun hermes-tracker--insert-todo-header (todo)
  "Insert Forgejo-style header and body for TODO."
  (let ((author (hermes-tracker--todo-author todo))
        (body (hermes-tracker--todo-body todo)))
    (insert (propertize
             (format "#%s " (hermes-tracker--display todo 'number))
             'face 'bold)
            (propertize (hermes-tracker--display todo 'title) 'face 'bold)
            "\n")
    (hermes-tracker--insert-header-metadata todo)
    (hermes-tracker--insert-separator)
    (insert (propertize author 'face 'hermes-tracker-profile-face)
            "\n\n")
    (hermes-tracker--insert-markdown body)
    (hermes-tracker--insert-separator)))

(defun hermes-tracker--card-profile (card)
  "Return the visible profile identity for linked CARD."
  (or (hermes-transport--non-empty-string (plist-get card :assignee))
      (hermes-transport--non-empty-string (plist-get card :profile))
      "-"))

(defun hermes-tracker--card-note (card)
  "Return optional state note for linked CARD."
  (let ((drift (plist-get card :drift))
        (replacement (plist-get card :replacement-id)))
    (string-join
     (delq nil
           (list
            (and drift (not (eq drift 'none))
                 (replace-regexp-in-string "-" " " (symbol-name drift)))
            (and (plist-get card :execution-state)
                 (if replacement
                     (format "superseded by %s" replacement)
                   "superseded"))))
     " · ")))

(defun hermes-tracker--insert-card-title (card)
  "Insert the primary linked-card line for CARD."
  (insert "  ")
  (if (plist-get card :payload)
      (progn
        (insert (hermes-kanban-format-status
                 (or (plist-get card :status) "unknown"))
                "  ")
        (insert-text-button
         (or (plist-get card :title) (plist-get card :task-id))
         'face 'link
         'follow-link t
         'help-echo "Open Hermes Kanban task"
         'action (lambda (_) (hermes-tracker--open-card card))))
    (insert (propertize "? unavailable" 'face 'warning)))
  (insert "\n"))

(defun hermes-tracker--insert-card-metadata (card)
  "Insert board, task, and profile metadata for linked CARD."
  (let ((board (plist-get card :board))
        (task-id (plist-get card :task-id))
        (note (hermes-tracker--card-note card)))
    (insert "  ")
    (insert-text-button
     board
     'face 'link
     'follow-link t
     'help-echo "Open Hermes Kanban board"
     'action (lambda (_) (hermes-tracker--open-card-board card)))
    (insert (propertize (format " · %s · " task-id) 'face 'shadow)
            (propertize (hermes-tracker--card-profile card)
                        'face 'hermes-tracker-profile-face))
    (unless (string-empty-p note)
      (insert (propertize (concat " · " note) 'face 'shadow)))
    (insert "\n")))

(defun hermes-tracker--insert-card-detail (card)
  "Insert compact linked CARD detail."
  (hermes-tracker--insert-card-title card)
  (hermes-tracker--insert-card-metadata card))

(defun hermes-tracker--insert-event (node)
  "Insert Forgejo-style timeline event from NODE."
  (let ((event-type (plist-get node :event-type))
        (created (plist-get node :created-at)))
    (insert (propertize (plist-get node :actor)
                        'face 'hermes-tracker-profile-face)
            " "
            (propertize event-type
                        'face (if (string= event-type "closed")
                                  'hermes-tracker-closed-face
                                'hermes-tracker-event-face)))
    (if-let* ((card (plist-get node :card)))
        (progn
          (when created
            (insert (propertize
                     (format " %s" (hermes-tracker--relative-time created))
                     'face 'shadow)))
          (insert "\n")
          (hermes-tracker--insert-card-detail card))
      (progn
        (when-let* ((detail (plist-get node :detail)))
          (insert " " (propertize detail 'face 'hermes-tracker-event-face)))
        (when created
          (insert (propertize
                   (format " %s" (hermes-tracker--relative-time created))
                   'face 'shadow)))
        (insert "\n")))
    (hermes-tracker--insert-separator)))

(defun hermes-tracker--insert-comment (node)
  "Insert Forgejo-style comment from NODE."
  (insert (propertize (plist-get node :author)
                      'face 'hermes-tracker-profile-face)
          (propertize
           (concat " commented "
                   (hermes-tracker--relative-time
                    (plist-get node :created-at)))
           'face 'shadow)
          "\n\n")
  (hermes-tracker--insert-markdown (plist-get node :body))
  (hermes-tracker--insert-separator))

(defun hermes-tracker--print-todo-node (node)
  "Pretty-print Tracker TODO EWOC NODE."
  (pcase (plist-get node :type)
    ('header (hermes-tracker--insert-todo-header (plist-get node :todo)))
    ('event (hermes-tracker--insert-event node))
    ('comment (hermes-tracker--insert-comment node))))

(defun hermes-tracker--todo-header-line (todo)
  "Return Forgejo-style header line for TODO."
  (let ((state (hermes-tracker--display todo 'status))
        (assignee (hermes-transport--non-empty-string
                   (hermes-tracker--display todo 'assignee))))
    (concat
     (propertize
      (format " #%s" (hermes-tracker--display todo 'number))
      'face 'hermes-tracker-number-face)
     " " (hermes-tracker--format-state state)
     "  " (propertize (hermes-tracker--display todo 'title) 'face 'bold)
     (and assignee (concat "  " (propertize assignee 'face 'shadow))))))

(defun hermes-tracker--current-todo-node-id ()
  "Return the EWOC node id at point, or nil."
  (when-let* ((node (and hermes-tracker--todo-ewoc
                         (ewoc-locate hermes-tracker--todo-ewoc))))
    (plist-get (ewoc-data node) :id)))

(defun hermes-tracker--render-todo-ewoc (todo cards)
  "Render TODO and linked CARDS into the current buffer."
  (let ((selected-id (hermes-tracker--current-todo-node-id))
        (nodes (hermes-tracker--todo-nodes todo cards))
        (table (make-hash-table :test #'equal))
        (inhibit-read-only t))
    (erase-buffer)
    (setq hermes-tracker--todo-ewoc
          (ewoc-create #'hermes-tracker--print-todo-node nil nil t))
    (dolist (node-data nodes)
      (puthash (plist-get node-data :id)
               (ewoc-enter-last hermes-tracker--todo-ewoc node-data)
               table))
    (setq hermes-tracker--todo-nodes table)
    (if-let* ((node (and selected-id (gethash selected-id table))))
        (ewoc-goto-node hermes-tracker--todo-ewoc node)
      (goto-char (point-min)))
    (set-buffer-modified-p nil)))

(defun hermes-tracker--prepare-todo-buffer ()
  "Prepare the current buffer for property-driven TODO rendering."
  (when font-lock-mode
    (font-lock-mode -1))
  (setq-local font-lock-defaults nil)
  (setq-local font-lock-keywords nil)
  (setq-local font-lock-extra-managed-props nil))

(define-derived-mode hermes-tracker-todo-mode special-mode "Tracker TODO"
  "Major mode for Tracker TODO details."
  :interactive nil
  (hermes-tracker--prepare-todo-buffer)
  (setq-local revert-buffer-function #'hermes-tracker--todo-revert)
  (setq-local truncate-lines nil))

(defun hermes-tracker--display-todo (pair &optional owner)
  "Render TODO and linked cards from PAIR in optional OWNER."
  (let ((todo (car pair)) (cards (cdr pair)))
    (let ((buffer (or owner (get-buffer-create "*Hermes Tracker TODO*"))))
      (when (buffer-live-p buffer)
        (with-current-buffer buffer
          (unless (derived-mode-p 'hermes-tracker-todo-mode)
            (hermes-tracker-todo-mode))
          (hermes-tracker--prepare-todo-buffer)
          (when (boundp 'hermes-tracker-todo-mode-map)
            (use-local-map hermes-tracker-todo-mode-map))
          (setq hermes-tracker--repo-slug
                (hermes-tracker--display todo 'repo_slug)
                hermes-tracker--todo-number (hermes-transport--get todo 'number)
                hermes-tracker--todo todo
                hermes-tracker--linked-cards cards
                mode-line-process
                (format " [%s#%s]" hermes-tracker--repo-slug
                        hermes-tracker--todo-number)
                header-line-format
                (hermes-tracker--todo-header-line todo))
          (hermes-tracker--render-todo-ewoc todo cards)
          (unless owner (pop-to-buffer (current-buffer))))))))

(defun hermes-tracker--todo-revert (&rest _)
  "Refresh current Tracker TODO."
  (hermes-tracker-open-todo
   hermes-tracker--repo-slug hermes-tracker--todo-number (current-buffer)))

(defun hermes-tracker-back-to-todos ()
  "Return to current repository's TODO list."
  (interactive)
  (hermes-tracker--render-todos hermes-tracker--repo-slug hermes-tracker--repo-slug))

(defun hermes-tracker--open-card (card)
  "Open linked Kanban CARD through the public Hermes Kanban entry point."
  (unless (plist-get card :payload) (user-error "Linked Kanban card is unavailable"))
  (hermes-kanban-open-task
   (plist-get card :task-id) (plist-get card :board)))

(defun hermes-tracker--open-card-board (card)
  "Open linked CARD's Kanban board and select the task."
  (hermes-kanban-open-board-task
   (plist-get card :board) (plist-get card :task-id)))

(defun hermes-tracker-open-kanban ()
  "Choose and open a linked Kanban card."
  (interactive)
  (unless hermes-tracker--linked-cards (user-error "TODO has no linked Kanban cards"))
  (let* ((choices (mapcar (lambda (card) (cons (hermes-tracker--linked-card-line card) card))
                          hermes-tracker--linked-cards))
         (card (cdr (assoc (completing-read "Kanban card: " choices nil t) choices))))
    (hermes-tracker--open-card card)))

(defun hermes-tracker--invocation (operation repo number)
  "Return unique invocation identity for OPERATION REPO NUMBER."
  (format "%s:%s:%s:%s" operation repo number (float-time)))

(defun hermes-tracker--mutation-request-async
    (method path repo &optional body invocation)
  "Invalidate REPO requests, then return mutation METHOD PATH promise.
Pass optional BODY and INVOCATION to the Tracker API boundary."
  (hermes-tracker-sync-note-mutation repo)
  (hermes--promise-then
   (hermes-tracker--request-async method path body invocation)
   (lambda (value)
     ;; Fence reads started while the mutation was in flight before callers
     ;; begin canonical post-mutation synchronization.
     (hermes-tracker-sync-note-mutation repo)
     value)))

(defun hermes-tracker-create-todo ()
  "Create a TODO in the current Tracker repository."
  (interactive)
  (let* ((owner (current-buffer))
         (repo hermes-tracker--repo-slug)
         (title (string-trim (read-string "Title: ")))
         (description (read-string-from-buffer "Description (optional): " ""))
         (priority (read-number "Priority: " 0)))
    (when (string-empty-p title) (user-error "Title is required"))
    (let ((request-id (hermes-tracker--begin-request
                       'hermes-tracker--todos-request-id)))
      (hermes--promise-then
       (hermes-tracker--mutation-request-async
        "POST" (hermes-tracker--api-path "repos" repo "todos") repo
        `((title . ,title) (description . ,description) (priority . ,priority))
        (hermes-tracker--invocation "create" repo 0))
       (lambda (_)
         (when (hermes-tracker--request-applicable-p
                owner 'hermes-tracker-todos-mode
                'hermes-tracker--todos-request-id request-id)
           (with-current-buffer owner (revert-buffer nil t))))
       #'hermes-tracker--report-error))))

(defun hermes-tracker-update-todo ()
  "Update title, priority, and assignee of current Tracker TODO."
  (interactive)
  (pcase-let ((`(,repo ,number ,refresh) (hermes-tracker--context)))
    (let* ((todo hermes-tracker--todo)
           (title (read-string "Title: " (hermes-tracker--display todo 'title)))
           (priority (read-number "Priority: "
                                  (or (hermes-transport--get todo 'priority) 0)))
           (assignee (read-string "Assignee (empty to clear): "
                                  (hermes-tracker--display todo 'assignee))))
      (hermes--promise-then
       (hermes-tracker--mutation-request-async
        "PATCH" (hermes-tracker--api-path "repos" repo "todos" number) repo
        `((title . ,title) (priority . ,priority)
          (assignee . ,(if (string-empty-p assignee) :null assignee))))
       (lambda (_) (funcall refresh)) #'hermes-tracker--report-error))))

(defun hermes-tracker--cached-label-names ()
  "Return label names from the local Tracker catalog."
  (mapcar (lambda (label) (hermes-transport--get label 'name))
          (hermes-tracker-db-list-labels
           (hermes-tracker-sync-cache-origin))))

(defun hermes-tracker-set-labels ()
  "Set current Tracker TODO labels using cached completion."
  (interactive)
  (pcase-let ((`(,repo ,number ,refresh) (hermes-tracker--context)))
    (let* ((todo (hermes-tracker-db-get-todo
                  (hermes-tracker-sync-cache-origin) repo number))
           (current (hermes-transport--get todo 'labels))
           (initial (and current
                         (concat (string-join current ",") ",")))
           (labels (completing-read-multiple
                    "Labels (empty clears): "
                    (delete-dups
                     (append current (hermes-tracker--cached-label-names)))
                    nil t initial)))
      (hermes--promise-then
       (hermes-tracker--mutation-request-async
        "PATCH" (hermes-tracker--api-path "repos" repo "todos" number) repo
        `((labels . ,(vconcat labels))))
       (lambda (_) (funcall refresh)) #'hermes-tracker--report-error))))

(defun hermes-tracker-claim-todo ()
  "Claim current Tracker TODO for an assignee."
  (interactive)
  (pcase-let ((`(,repo ,number ,refresh) (hermes-tracker--context)))
    (let ((assignee (string-trim (read-string "Assignee: " hermes-tracker-actor))))
      (when (string-empty-p assignee) (user-error "Assignee is required"))
      (hermes--promise-then
       (hermes-tracker--mutation-request-async
        "PATCH" (hermes-tracker--api-path "repos" repo "todos" number) repo
        `((assignee . ,assignee)))
       (lambda (_) (funcall refresh)) #'hermes-tracker--report-error))))

(defun hermes-tracker-release-todo ()
  "Release current Tracker TODO to the unassigned open backlog."
  (interactive)
  (pcase-let ((`(,repo ,number ,refresh) (hermes-tracker--context)))
    (hermes--promise-then
     (hermes-tracker--mutation-request-async
      "PATCH" (hermes-tracker--api-path "repos" repo "todos" number) repo
      '((assignee . :null)))
     (lambda (_) (funcall refresh)) #'hermes-tracker--report-error)))

(defun hermes-tracker-comment ()
  "Append a public Tracker comment to current TODO."
  (interactive)
  (pcase-let ((`(,repo ,number ,refresh) (hermes-tracker--context)))
    (let ((body (string-trim (read-string-from-buffer "Comment: " ""))))
      (when (string-empty-p body) (user-error "Comment is required"))
      (hermes--promise-then
       (hermes-tracker--mutation-request-async
        "POST" (hermes-tracker--api-path "repos" repo "todos" number "comments") repo
        `((author . ,hermes-tracker-actor) (body . ,body))
        (hermes-tracker--invocation "comment" repo number))
       (lambda (_) (funcall refresh)) #'hermes-tracker--report-error))))

(defun hermes-tracker-close-todo ()
  "Explain the pinned API's evidence-gated closure boundary."
  (interactive)
  (user-error
   "Tracker closure is created by verified Git closure ingestion, not an interactive route"))

(defun hermes-tracker-link-kanban ()
  "Explain why Tracker-to-Kanban link mutation is unavailable."
  (interactive)
  (user-error "Pinned Tracker API does not provide external-reference mutation routes"))

;;; Hermes integration

(defvar-local hermes-tracker--kanban-reference nil
  "Canonical Tracker reference in the current Kanban task.")

(defun hermes-kanban-open-tracker ()
  "Open the Tracker TODO referenced by the current Kanban task."
  (interactive)
  (unless hermes-tracker--kanban-reference
    (user-error "This Kanban task has no canonical Tracker reference"))
  (hermes-tracker-open-todo
   (plist-get hermes-tracker--kanban-reference :repo-slug)
   (plist-get hermes-tracker--kanban-reference :number)))

(defun hermes-tracker--render-kanban-reference (payload _board-slug)
  "Render the Tracker reference from Kanban task PAYLOAD."
  (let* ((task (hermes-transport--get payload 'task))
         (reference
          (hermes-tracker-parse-reference
           (hermes-transport--display-field task 'body))))
    (setq-local hermes-tracker--kanban-reference reference)
    (when reference
      (insert "\nTracker: ")
      (insert-text-button
       (format "%s#%s"
               (plist-get reference :repo-slug)
               (plist-get reference :number))
       'follow-link t
       'help-echo "Open Tracker TODO"
       'action (lambda (_) (hermes-kanban-open-tracker)))
      (insert "\n"))))

(keymap-popup-add-entry
 hermes-dashboard-mode-map "T" "Tracker"
 #'hermes-list-tracker-repositories "View")

(keymap-popup-add-entry
 hermes-kanban-task-mode-map "T" "Open Tracker TODO"
 #'hermes-kanban-open-tracker "Task")

(add-hook 'hermes-kanban-task-detail-functions
          #'hermes-tracker--render-kanban-reference)

(provide 'hermes-tracker)
;;; hermes-tracker.el ends here
