;;; hermes-tracker-tests.el --- Tests for hermes-tracker  -*- lexical-binding: t; -*-

;;; Code:

(require 'ert)
(require 'hermes-tracker)

(ert-deftest hermes-tracker-normalize-base-url-requires-secure-remote-origin ()
  "Tracker URLs require HTTPS except for loopback development."
  (should (equal (hermes-tracker--normalize-base-url " https://tracker.test/ ")
                 "https://tracker.test"))
  (should (equal (hermes-tracker--normalize-base-url "http://127.0.0.1:8000/")
                 "http://127.0.0.1:8000"))
  (should-error (hermes-tracker--normalize-base-url "http://tracker.test")
                :type 'user-error)
  (should-error (hermes-tracker--normalize-base-url "https://u:p@tracker.test")
                :type 'user-error)
  (should-error (hermes-tracker--normalize-base-url "file:///tmp/tracker")
                :type 'user-error))

(ert-deftest hermes-tracker-build-request-redacts-auth-and-adds-mutation-headers ()
  "Tracker requests carry bearer, actor, idempotency, and redaction metadata."
  (let ((hermes-tracker-url "https://tracker.test")
        (hermes-tracker-actor "emacs-user"))
    (cl-letf (((symbol-function 'hermes-tracker--token) (lambda (_) "top-secret")))
      (let* ((request (hermes-tracker--request
                       "POST" "/api/v1/repos/proj/todos"
                       '((title . "Do it")) "stable-invocation"))
             (headers (plist-get request :headers)))
        (should (equal (plist-get request :url)
                       "https://tracker.test/api/v1/repos/proj/todos"))
        (should (equal (cdr (assoc "Authorization" headers))
                       "Bearer top-secret"))
        (should (equal (cdr (assoc "X-Actor" headers)) "emacs-user"))
        (should (string-prefix-p "emacs-hermes-"
                                 (cdr (assoc "Idempotency-Key" headers))))
        (should (equal (plist-get request :secrets) '("top-secret")))))))

(ert-deftest hermes-tracker-unconfigured-signals-without-auth-lookup ()
  "An absent Tracker URL stops before credential lookup."
  (let ((hermes-tracker-url nil)
        looked-up)
    (cl-letf (((symbol-function 'auth-source-search)
               (lambda (&rest _) (setq looked-up t))))
      (should-error (hermes-tracker--base-url) :type 'user-error)
      (should-not looked-up))))

(ert-deftest hermes-tracker-repository-and-todo-rows-normalize-api-values ()
  "Repository and TODO payloads become stable tabulated-list entries."
  (let ((repo-row (car (hermes-tracker--repository-rows
                        [((slug . "proj") (name . "Project")
                          (description . "Work")
                          (summary
                           . ((status_counts
                               . ((open . 2) (closed . 3))))))])))
        (todo-row (car (hermes-tracker--todo-rows
                        '((todos . [((number . 3) (status . "open")
                                    (priority . 7) (assignee . "elisp-dev")
                                    (title . "Build it"))]))))))
    (should (equal (car repo-row) "proj"))
    (should (equal (cadr repo-row) ["Project" "2" "3" "Work"]))
    (should (equal (car todo-row) 3))
    (should (equal (cadr todo-row) ["3" "open" "7" "elisp-dev" "Build it"]))))

(ert-deftest hermes-tracker-repository-columns-match-api-ledger ()
  "Repository browser shows only API-backed lifecycle counts."
  (with-temp-buffer
    (hermes-tracker-repositories-mode)
    (should (equal (mapcar #'car (append tabulated-list-format nil))
                   '("Repository" "Open" "Closed" "Description")))))

(ert-deftest hermes-tracker-repository-counts-preserve-zero-and-unknown-values ()
  "Repository counts accept transport shapes without inventing missing values."
  (let* ((counts (make-hash-table :test #'equal))
         (summary (make-hash-table :test #'eq))
         (repository (make-hash-table :test #'equal)))
    (puthash "open" "0" counts)
    (puthash 'closed "not-a-count" counts)
    (puthash 'status_counts counts summary)
    (puthash "slug" "proj" repository)
    (puthash 'name "Project" repository)
    (puthash "summary" summary repository)
    (let ((entry (cadar (hermes-tracker--repository-rows (list repository)))))
      (should (equal (append (seq-subseq entry 1 3) nil)
                     '("0" "-"))))))

(ert-deftest hermes-tracker-todo-statuses-match-backend-lifecycle ()
  "Filters expose exactly the statuses accepted by the Tracker API."
  (should (equal hermes-tracker--todo-statuses
                 '("open" "closed")))
  (dolist (status hermes-tracker--todo-statuses)
    (should (equal (hermes-tracker--todo-list-path "repo name" status)
                   (format "/api/v1/repos/repo%%20name/todos?status_filter=%s"
                           status)))))

(ert-deftest hermes-tracker-filter-status-requires-backend-value-or-empty ()
  "Interactive filtering requires an exact status while empty clears it."
  (let (collection require-match refreshed)
    (cl-letf (((symbol-function 'completing-read)
               (lambda (_prompt choices _predicate required &rest _)
                 (setq collection choices
                       require-match required)
                 ""))
              ((symbol-function 'revert-buffer)
               (lambda (&rest _) (setq refreshed t))))
      (with-temp-buffer
        (setq-local hermes-tracker--status-filter "open")
        (call-interactively #'hermes-tracker-filter-status)
        (should (equal hermes-tracker--status-filter "all")))
      (should (equal collection '("" "open" "closed")))
      (should require-match)
      (should refreshed))))

(ert-deftest hermes-tracker-status-validation-covers-command-and-request-path ()
  "Status handling accepts exact API values and rejects other types before effects."
  (let ((cases '((nil "all" "/api/v1/repos/proj/todos")
                 ("open" "open" "/api/v1/repos/proj/todos?status_filter=open")
                 ("closed" "closed" "/api/v1/repos/proj/todos?status_filter=closed")))
        (refreshes 0)
        (requests 0))
    (cl-letf (((symbol-function 'revert-buffer)
               (lambda (&rest _) (cl-incf refreshes)))
              ((symbol-function 'hermes-tracker--request-async)
               (lambda (&rest _)
                 (cl-incf requests)
                 (hermes--promise-resolved nil))))
      (dolist (case cases)
        (pcase-let ((`(,input ,stored ,path) case))
          (with-temp-buffer
            (hermes-tracker-filter-status input)
            (should (equal hermes-tracker--status-filter stored)))
          (should (equal (hermes-tracker--todo-list-path "proj" input) path))))
      (let ((refresh-count refreshes))
        (with-temp-buffer
          (should-error (hermes-tracker-filter-status 'active)
                        :type 'user-error))
        (with-temp-buffer
          (hermes-tracker-todos-mode)
          (setq-local hermes-tracker--status-filter 'active)
          (should-error (hermes-tracker--render-todos "proj" "Project"
                                                      (current-buffer))
                        :type 'user-error))
        (should-error (hermes-tracker--todo-list-path "proj" 'active)
                      :type 'user-error)
        (should (= refreshes refresh-count))
        (should (= requests 0))))))

(ert-deftest hermes-tracker-structured-filter-stays-local-and-preserves-terms ()
  "Structured filtering passes one pure filter value to SQLite rendering."
  (let (filters refreshed)
    (cl-letf (((symbol-function 'revert-buffer)
               (lambda (&rest _)
                 (setq refreshed t
                       filters (hermes-tracker--active-filters)))))
      (with-temp-buffer
        (hermes-tracker-todos-mode)
        (hermes-tracker-filter
         "state:all label:bug label:urgent assignee:alice assignee:bob q:\"offline cache\"")
        (should refreshed)
        (should (equal filters
                       '(:state "all"
                         :labels ("bug" "urgent")
                         :assignees ("alice" "bob")
                         :query "offline cache")))
        (should (equal hermes-tracker--status-filter "all"))))))

(ert-deftest hermes-tracker-filter-status-rejects-unsupported-value ()
  "Non-exact status input stops before a refresh or request path."
  (let (refreshed)
    (cl-letf (((symbol-function 'revert-buffer)
               (lambda (&rest _) (setq refreshed t))))
      (dolist (status '("active" "blocked" "done" "claimed"
                        " open" "open " " "))
        (with-temp-buffer
          (should-error (hermes-tracker-filter-status status)
                        :type 'user-error))
        (should-error (hermes-tracker--todo-list-path "proj" status)
                      :type 'user-error))
      (should-not refreshed)
      (should (equal (hermes-tracker--todo-list-path "proj" "")
                     "/api/v1/repos/proj/todos")))))

(ert-deftest hermes-tracker-todo-popup-is-bounded-and-preserves-actions ()
  "TODO detail popup keeps every direct action in four bounded groups."
  (let* ((rows (keymap-popup--meta hermes-tracker-todo-mode-map
                                   'descriptions))
         (groups (apply #'append rows))
         (entries (apply #'append
                         (mapcar (lambda (group)
                                   (plist-get group :entries))
                                 groups)))
         (commands (mapcar (lambda (entry) (plist-get entry :command)) entries)))
    (should (= (length groups) 4))
    (should (seq-every-p (lambda (group)
                           (<= (length (plist-get group :entries)) 4))
                         groups))
    (dolist (binding
             '(("n" . hermes-tracker-next)
               ("p" . hermes-tracker-previous)
               ("b" . hermes-tracker-back-to-todos)
               ("RET" . push-button)
               ("TAB" . hermes-tracker-next-link)
               ("<backtab>" . hermes-tracker-previous-link)
               ("K" . hermes-tracker-open-kanban)
               ("u" . hermes-tracker-update-todo)
               ("l" . hermes-tracker-set-labels)
               ("c" . hermes-tracker-comment)
               ("a" . hermes-tracker-claim-todo)
               ("r" . hermes-tracker-release-todo)
               ("g" . revert-buffer)
               ("?" . hermes-tracker-todo-mode-map-popup)))
      (should (eq (keymap-lookup hermes-tracker-todo-mode-map (car binding))
                  (cdr binding)))
      (should (memq (cdr binding) commands)))))

(ert-deftest hermes-tracker-detail-faces-every-backend-status-semantically ()
  "Detail state text distinguishes open and closed."
  (dolist (case '(("open" . hermes-tracker-open-face)
                  ("closed" . hermes-tracker-closed-face)))
    (let ((text (hermes-tracker--format-state (car case))))
      (should (eq (get-text-property 0 'face text) (cdr case))))))

(ert-deftest hermes-tracker-rows-face-every-column ()
  "Tracker repository and TODO rows give every column its own face."
  (let* ((repository '((slug . "proj") (name . "Project")
                       (description . "Work")
                       (summary
                        . ((status_counts
                            . ((open . 2) (closed . 3)))))))
         (repo-row (car (hermes-tracker--repository-rows
                         (list repository))))
         (row (car (hermes-tracker--todo-rows
                    '((todos . (((number . 3) (status . "open")
                                 (priority . 7) (assignee . "elisp-dev")
                                 (title . "Build it"))))))))
         (entry (cadr row)))
    (should (eq (get-text-property 0 'face (aref (cadr repo-row) 0))
                'hermes-browser-name))
    (should (eq (get-text-property 0 'face (aref (cadr repo-row) 1))
                'hermes-tracker-open-count-face))
    (should (eq (get-text-property 0 'face (aref (cadr repo-row) 2))
                'hermes-tracker-closed-count-face))
    (should (eq (get-text-property 0 'face (aref (cadr repo-row) 3))
                'hermes-browser-description))
    (should (eq (get-text-property 0 'face (aref entry 0))
                'hermes-browser-identifier))
    (should (equal (get-text-property 0 'face (aref entry 1))
                   '(hermes-browser-active hermes-browser-status)))
    (should (eq (get-text-property 0 'face (aref entry 2))
                'hermes-browser-priority))
    (should (eq (get-text-property 0 'face (aref entry 3))
                'hermes-browser-assignee))
    (should (eq (get-text-property 0 'face (aref entry 4))
                'hermes-browser-title))))

(ert-deftest hermes-tracker-todo-nodes-build-chronological-timeline ()
  "TODO details merge comments, cards, and closure into one timeline."
  (let* ((todo '((repo_slug . "proj") (number . 3) (title . "Build it")
                 (comments . [((id . 9) (author . "elisp-dev")
                               (body . "Started")
                               (created_at . "2026-07-12T10:00:00Z"))
                              ((id . 10) (author . "reviewer")
                               (body . "Ready")
                               (created_at . "2026-07-12T12:00:00Z"))])
                 (closed_by . "reviewer")
                 (closed_at . "2026-07-12T13:00:00Z")
                 (closing_commit . "abc123")))
         (card '(:task-id "t_1234abcd" :board "default"
                 :title "Implement UI" :status "done"
                 :created-at "2026-07-12T11:00:00Z"))
         (nodes (hermes-tracker--todo-nodes todo (list card))))
    (should (equal (mapcar (lambda (node) (plist-get node :type)) nodes)
                   '(header comment event comment event)))
    (should (equal (mapcar (lambda (node) (plist-get node :id)) nodes)
                   '(header (comment 9) (event card "t_1234abcd")
                            (comment 10) (event closure))))
    (should (equal (plist-get (nth 2 nodes) :card) card))
    (should (equal (plist-get (nth 4 nodes) :event-type) "closed"))
    (should (equal (plist-get (nth 4 nodes) :detail) "abc123"))))

(ert-deftest hermes-tracker-todo-author-does-not-invent-creator ()
  "An assignee is not presented as the TODO creator."
  (should (equal (hermes-tracker--todo-author '((assignee . "worker")))
                 "tracker")))

(ert-deftest hermes-tracker-parse-tracker-reference-is-strict ()
  "Only one exact canonical tracker-ref block opts a Kanban card in."
  (should (equal (hermes-tracker-parse-reference
                  "before\n```tracker-ref\n{\"number\":3,\"repo_slug\":\"proj\"}\n```\nafter")
                 '(:repo-slug "proj" :number 3)))
  (dolist (body '(nil
                  "```tracker-ref\nnot-json\n```"
                  "```tracker-ref\n{\"repo_slug\":\"proj\",\"number\":0}\n```"
                  "```tracker-ref\n{\"repo_slug\":\"proj\",\"number\":3,\"extra\":true}\n```"
                  "```tracker-ref   \n{\"number\":3,\"repo_slug\":\"proj\"}\n```"
                  "````tracker-ref\n{\"number\":3,\"repo_slug\":\"proj\"}\n````"
                  "```tracker-ref\n{\"number\":3,\"repo_slug\":\"proj\"}\n```\n```tracker-ref"
                  "```tracker-ref\n{ \"number\": 3, \"repo_slug\": \"proj\" }\n```"
                  "```tracker-ref\n{\"repo_slug\":\"proj\",\"number\":3}\n```"
                  "```tracker-ref\n{\"number\":1,\"repo_slug\":\"a\"}\n```\n```tracker-ref\n{\"number\":2,\"repo_slug\":\"b\"}\n```"))
    (should-not (hermes-tracker-parse-reference body))))

(ert-deftest hermes-tracker-render-reference-is-canonical ()
  "Canonical tracker-ref rendering is compact."
  (should (equal (hermes-tracker-render-reference "proj" 3)
                 "```tracker-ref\n{\"number\":3,\"repo_slug\":\"proj\"}\n```")))

(ert-deftest hermes-tracker-kanban-references-and-drift-are-normalized ()
  "Canonical references resolve to linked-card records with drift state."
  (let* ((todo '((repo_slug . "proj") (number . 3)
                 (external_references
                  . [((system . "other") (external_id . "x"))
                     ((system . "hermes-kanban") (external_id . "t_1234abcd")
                      (metadata . ((board . "default") (profile . "elisp-dev"))))])
                 (superseded_kanban_executions
                  . [((task_id . "t_1234abcd")
                      (replacement_kanban_id . "t_99999999"))])))
         (refs (hermes-tracker--kanban-references todo))
         (linked (hermes-tracker--linked-card
                  todo (car refs)
                  '((task . ((id . "t_1234abcd") (title . "Card")
                             (status . "done") (assignee . "elisp-dev")
                             (body . "```tracker-ref\n{\"number\":3,\"repo_slug\":\"proj\"}\n```")))))))
    (should (= (length refs) 1))
    (should (equal (plist-get linked :drift) 'none))
    (should (equal (plist-get linked :execution-state) 'superseded))
    (should (equal (plist-get linked :replacement-id) "t_99999999")))
  (let ((missing (hermes-tracker--linked-card
                  '((repo_slug . "proj") (number . 3))
                  '((external_id . "t_deadbeef") (metadata . ((board . "default"))))
                  nil)))
    (should (equal (plist-get missing :drift) 'missing-card))))

(ert-deftest hermes-tracker-linked-card-detects-one-sided-and-mismatched-references ()
  "Loaded cards report missing or mismatched card-side backlinks."
  (let ((todo '((repo_slug . "proj") (number . 3)))
        (ref '((external_id . "t_1234abcd") (metadata . ((board . "default"))))))
    (should (equal
             (plist-get (hermes-tracker--linked-card
                         todo ref '((task . ((id . "t_1234abcd") (body . "plain")))))
                        :drift)
             'missing-card-backlink))
    (should (equal
             (plist-get (hermes-tracker--linked-card
                         todo ref
                         '((task . ((id . "t_1234abcd")
                                    (body . "```tracker-ref\n{\"number\":1,\"repo_slug\":\"other\"}\n```")))))
                        :drift)
             'mismatched-card-backlink))))

(ert-deftest hermes-tracker-integrations-register-without-configuration ()
  "Hermes exposes Tracker integrations without configuration or I/O."
  (let ((hermes-tracker-url nil))
    (dolist (function '(hermes-kanban-format-status
                        hermes-kanban-open-board-task
                        hermes-kanban-open-task))
      (should (fboundp function)))
    (should (eq (keymap-lookup hermes-dashboard-mode-map "T")
                'hermes-list-tracker-repositories))
    (should (eq (keymap-lookup hermes-kanban-task-mode-map "T")
                'hermes-kanban-open-tracker))
    (should
     (memq #'hermes-tracker--render-kanban-reference
           hermes-kanban-task-detail-functions))
    (should (featurep 'hermes-tracker))))

(ert-deftest hermes-tracker-todo-detail-uses-ewoc-with-card-links ()
  "Tracker TODO details render comments and linked cards through EWOC."
  (let ((card '(:task-id "t_1234abcd" :board "default"
                :title "Implement UI" :status "done"
                :created-at "2026-07-12T10:00:00Z"
                :profile "talos@rhodes"
                :payload ((task . ((id . "t_1234abcd"))))))
        opened-task opened-board)
    (cl-letf (((symbol-function 'pop-to-buffer) #'switch-to-buffer)
              ((symbol-function 'hermes-kanban-open-task)
               (lambda (task-id board)
                 (setq opened-task (list task-id board))))
              ((symbol-function 'hermes-kanban-open-board-task)
               (lambda (board task-id)
                 (setq opened-board (list board task-id)))))
      (unwind-protect
          (progn
            (hermes-tracker--display-todo
             (cons '((repo_slug . "proj") (number . 3) (title . "Build it")
                     (status . "open") (priority . 7) (assignee . "elisp-dev")
                     (author . "talos@rhodes")
                     (description . "**Description**")
                     (acceptance_criteria . ["Works"])
                     (verification_commands . ["make test"])
                     (verification_output . "36 tests pass")
                     (closing_commit . "abc123")
                     (closed_by . "reviewer")
                     (closed_at . "2026-07-12T12:00:00Z")
                     (links . [((link_type . "depends-on")
                                (target_repo_slug . "core")
                                (target_number . 2)
                                (target_title . "Foundation"))])
                     (comments . [((id . 9) (author . "reviewer")
                                   (body . "Ready")
                                   (created_at . "2026-07-12T11:00:00Z"))]))
                   (list card)))
            (with-current-buffer "*Hermes Tracker TODO*"
              (should (derived-mode-p 'hermes-tracker-todo-mode))
              (should (derived-mode-p 'special-mode))
              (should buffer-read-only)
              (should (ewoc-p hermes-tracker--todo-ewoc))
              (should (equal (mapcar (lambda (node) (plist-get node :type))
                                     (ewoc-collect hermes-tracker--todo-ewoc
                                                   #'identity))
                             '(header event comment event)))
              (dolist (text '("Description" "Works" "make test"
                              "36 tests pass" "abc123" "reviewer"
                              "depends-on core#2" "Ready"))
                (should (string-match-p (regexp-quote text) (buffer-string))))
              (should (string-match-p
                       "#3 Build it\nopen  talos@rhodes opened "
                       (buffer-string)))
              (should (string-match-p "Repository: proj" (buffer-string)))
              (should (string-match-p "Priority: 7" (buffer-string)))
              (should (string-match-p "Assignees: elisp-dev" (buffer-string)))
              (should-not (string-match-p "Comments (" (buffer-string)))
              (should-not (string-match-p "Linked Kanban cards ("
                                          (buffer-string)))
              (should (< (string-match "linked Kanban card" (buffer-string))
                         (string-match "reviewer commented" (buffer-string))
                         (string-match "reviewer closed" (buffer-string))))
              (should (string-match-p
                       "linked Kanban card [^\n]+\n  .*Implement UI\n  "
                       (buffer-string)))
              (goto-char (point-min))
              (search-forward "Implement UI")
              (let ((button (button-at (1- (point)))))
                (should button)
                (should (eq (button-get button 'face) 'link))
                (button-activate button))
              (should (equal opened-task '("t_1234abcd" "default")))
              (goto-char (point-min))
              (search-forward "default")
              (button-activate (button-at (1- (point))))
              (should (equal opened-board '("default" "t_1234abcd")))
              (goto-char (point-min))
              (search-forward "talos@rhodes")
              (should (eq (get-text-property (1- (point)) 'face)
                          'hermes-tracker-profile-face))
              (should-not (string-match-p "@talos@rhodes" (buffer-string)))
              (goto-char (point-min))
              (search-forward "reviewer commented")
              (let* ((comment-start (match-beginning 0))
                     (separator (text-property-any
                                 (point-min) comment-start 'face
                                 'hermes-tracker-separator-face)))
                (while-let ((next (and separator
                                       (text-property-any
                                        (1+ separator) comment-start 'face
                                        'hermes-tracker-separator-face))))
                  (setq separator next))
                (should separator)
                (should (string-blank-p
                         (buffer-substring-no-properties
                          (1+ separator) comment-start))))
              (goto-char (point-min))
              (should (text-property-search-forward
                       'face 'hermes-tracker-separator-face t))
              (setq-local revert-buffer-function (lambda (&rest _)))
              (revert-buffer t)
              (should (eq (keymap-lookup (current-local-map) "u")
                          'hermes-tracker-update-todo))
              (should (eq (keymap-lookup (current-local-map) "n")
                          'hermes-tracker-next))
              (should (eq (keymap-lookup (current-local-map) "p")
                          'hermes-tracker-previous))
              (should (eq (keymap-lookup (current-local-map) "RET")
                          'push-button))))
        (when (get-buffer "*Hermes Tracker TODO*")
          (kill-buffer "*Hermes Tracker TODO*"))))))

(ert-deftest hermes-tracker-missing-card-keeps-only-board-link ()
  "An unavailable Kanban card has no dead task link."
  (let ((card '(:task-id "t_1234abcd" :board "default"
                :profile "talos@rhodes" :drift missing-card))
        opened-board)
    (cl-letf (((symbol-function 'hermes-kanban-open-board-task)
               (lambda (board task-id)
                 (setq opened-board (list board task-id)))))
      (with-temp-buffer
        (hermes-tracker-todo-mode)
        (hermes-tracker--display-todo
         (cons '((repo_slug . "proj") (number . 3) (title . "Build it"))
               (list card))
         (current-buffer))
        (goto-char (point-min))
        (search-forward "unavailable")
        (should-not (button-at (1- (point))))
        (should (eq (get-text-property (1- (point)) 'face) 'warning))
        (goto-char (point-min))
        (search-forward "default")
        (button-activate (button-at (1- (point))))
        (should (equal opened-board '("default" "t_1234abcd")))
        (should (string-match-p "talos@rhodes" (buffer-string)))
        (should-not (string-match-p "@talos@rhodes" (buffer-string)))))))

(ert-deftest hermes-tracker-todo-detail-matches-forgejo-timeline-language ()
  "Tracker details use Forgejo header, body, comment, and separator order."
  (with-temp-buffer
    (hermes-tracker-todo-mode)
    (hermes-tracker--display-todo
     (cons
      '((repo_slug . "proj") (number . 3) (title . "Build it")
        (status . "open") (priority . 7) (assignee . "elisp-dev")
        (author . "talos@rhodes") (description . "Description")
        (created_at . "2025-01-01T12:00:00Z")
        (updated_at . "2025-01-01T12:00:00Z")
        (comments . [((id . 9) (author . "talos@rhodes")
                      (body . "Comment body")
                      (created_at . "2025-01-02T12:00:00Z"))]))
      nil)
     (current-buffer))
    (should
     (equal
      (buffer-substring-no-properties (point-min) (point-max))
      (concat
       "#3 Build it\n"
       "open  talos@rhodes opened 2025-01-01  [1 comments]\n"
       "Repository: proj\n"
       "Priority: 7\n"
       "Assignees: elisp-dev\n"
       "\n \n"
       "talos@rhodes\n\n"
       "Description\n"
       "\n \n"
       "talos@rhodes commented 2025-01-02\n\n"
       "Comment body\n"
       "\n \n")))))

(ert-deftest hermes-tracker-todo-detail-survives-stale-font-lock ()
  "TODO faces and separators survive stale Markdown fontification."
  (with-temp-buffer
    (hermes-tracker-todo-mode)
    (font-lock-mode -1)
    (setq-local
     font-lock-defaults
     '((("\\_<talosbot\\_>" . font-lock-warning-face))
       nil nil nil nil
       (font-lock-extra-managed-props face display keymap)))
    (font-lock-mode 1)
    (hermes-tracker--display-todo
     (cons
      '((repo_slug . "proj") (number . 3) (title . "Build it")
        (author . "talosbot@rhodes") (description . "Description")
        (comments . [((id . 9) (author . "talosbot@rhodes")
                      (body . "Comment body")
                      (created_at . "2025-01-02T12:00:00Z"))]))
      nil)
     (current-buffer))
    (font-lock-ensure)
    (should-not font-lock-mode)
    (goto-char (point-min))
    (search-forward "talosbot@rhodes commented")
    (let* ((start (match-beginning 0))
           (suffix (+ start (length "talosbot@rhodes")))
           (separator (text-property-any
                       (point-min) start 'face
                       'hermes-tracker-separator-face)))
      (should (eq (get-text-property start 'face)
                  'hermes-tracker-profile-face))
      (should (eq (get-text-property suffix 'face) 'shadow))
      (should separator)
      (should (equal (get-text-property separator 'display)
                     '(space :width text))))))

(ert-deftest hermes-tracker-todo-ewoc-refresh-preserves-current-comment ()
  "Refreshing a TODO keeps point on the current comment node."
  (with-temp-buffer
    (hermes-tracker-todo-mode)
    (let ((first '((repo_slug . "proj") (number . 3) (title . "Build it")
                   (comments . [((id . 9) (author . "reviewer")
                                 (body . "First")
                                 (created_at . "2026-07-12T11:00:00Z"))])))
          (updated '((repo_slug . "proj") (number . 3) (title . "Build it")
                     (comments . [((id . 9) (author . "reviewer")
                                   (body . "Updated")
                                   (created_at . "2026-07-12T11:00:00Z"))]))))
      (hermes-tracker--display-todo (cons first nil) (current-buffer))
      (ewoc-goto-node hermes-tracker--todo-ewoc
                      (gethash '(comment 9) hermes-tracker--todo-nodes))
      (hermes-tracker--display-todo (cons updated nil) (current-buffer))
      (should (equal (hermes-tracker--current-todo-node-id) '(comment 9)))
      (should (string-match-p "Updated" (buffer-string))))))

(ert-deftest hermes-tracker-repositories-render-cache-before-sync ()
  "Repository UI renders SQLite rows before its background sync settles."
  (let ((pending (hermes--promise-make)))
    (cl-letf (((symbol-function 'hermes-tracker-sync-cache-origin)
               (lambda () "origin"))
              ((symbol-function 'hermes-tracker-db-list-repository-summaries)
               (lambda (_)
                 '(((slug . "cached") (name . "Cached")
                    (description . "offline")
                    (summary . ((status_counts . ((open . 2) (closed . 1)))))))))
              ((symbol-function 'hermes-tracker-sync-all) (lambda () pending))
              ((symbol-function 'hermes-tracker--request-async)
               (lambda (&rest _) (error "UI bypassed SQLite")))
              ((symbol-function 'pop-to-buffer) #'ignore))
      (unwind-protect
          (progn
            (hermes-list-tracker-repositories)
            (with-current-buffer "*Hermes Tracker Repositories*"
              (should (equal (caar tabulated-list-entries) "cached"))))
        (when (get-buffer "*Hermes Tracker Repositories*")
          (kill-buffer "*Hermes Tracker Repositories*"))))))

(ert-deftest hermes-tracker-todos-render-cache-before-sync ()
  "TODO UI renders SQLite rows before its repository sync settles."
  (let ((pending (hermes--promise-make)))
    (cl-letf (((symbol-function 'hermes-tracker-sync-cache-origin)
               (lambda () "origin"))
              ((symbol-function 'hermes-tracker-db-query-todos)
               (lambda (&rest _)
                 '(((number . 4) (title . "Cached") (status . "open")
                    (priority . 1) (labels . nil)))))
              ((symbol-function 'hermes-tracker-sync-todos) (lambda (_) pending))
              ((symbol-function 'hermes-tracker--request-async)
               (lambda (&rest _) (error "UI bypassed SQLite")))
              ((symbol-function 'pop-to-buffer) #'ignore))
      (unwind-protect
          (progn
            (hermes-tracker--render-todos "proj" "Project" nil)
            (with-current-buffer "*Hermes Tracker TODOs*"
              (should (= (caar tabulated-list-entries) 4))))
        (when (get-buffer "*Hermes Tracker TODOs*")
          (kill-buffer "*Hermes Tracker TODOs*"))))))

(ert-deftest hermes-tracker-detail-open-uses-production-sync-entry-point ()
  "Opening detail calls the real sync entry point with TODO scope."
  (let (scope)
    (cl-letf (((symbol-function 'hermes-tracker-sync-cache-origin)
               (lambda () "origin"))
              ((symbol-function 'hermes-tracker-db-get-todo)
               (lambda (&rest _)
                 '((repo_slug . "proj") (number . 1) (title . "Cached")
                   (status . "open") (priority . 0) (labels . nil))))
              ((symbol-function 'hermes-tracker--linked-cards-for-todo)
               (lambda (_) (hermes--promise-resolved nil)))
              ((symbol-function 'hermes-tracker-sync--request)
               (lambda (resource _fetch _apply)
                 (setq scope resource)
                 (hermes--promise-resolved nil)))
              ((symbol-function 'pop-to-buffer) #'ignore))
      (unwind-protect
          (progn
            (hermes-tracker-open-todo "proj" 1)
            (should (equal scope '(todo "proj" 1))))
        (when (get-buffer "*Hermes Tracker TODO*")
          (kill-buffer "*Hermes Tracker TODO*"))))))

(ert-deftest hermes-tracker-detail-overlay-does-not-recreate-killed-buffer ()
  "A late Dashboard overlay cannot recreate the standard TODO buffer."
  (let ((cards (hermes--promise-make)))
    (cl-letf (((symbol-function 'hermes-tracker-sync-cache-origin)
               (lambda () "origin"))
              ((symbol-function 'hermes-tracker-db-get-todo)
               (lambda (&rest _)
                 '((repo_slug . "proj") (number . 1) (title . "Cached")
                   (status . "open") (priority . 0) (labels . nil))))
              ((symbol-function 'hermes-tracker-sync-todo)
               (lambda (&rest _) (hermes--promise-resolved nil)))
              ((symbol-function 'hermes-tracker--linked-cards-for-todo)
               (lambda (_) cards))
              ((symbol-function 'pop-to-buffer) #'ignore))
      (hermes-tracker-open-todo "proj" 1)
      (kill-buffer "*Hermes Tracker TODO*")
      (hermes--promise-resolve cards nil)
      (should-not (get-buffer "*Hermes Tracker TODO*")))))

(ert-deftest hermes-tracker-missing-cached-detail-clears-prior-state ()
  "A missing detail cannot retain another TODO's data or linked cards."
  (let ((buffer (generate-new-buffer " *missing Tracker TODO*")))
    (unwind-protect
        (with-current-buffer buffer
          (hermes-tracker-todo-mode)
          (setq-local hermes-tracker--todo '((number . 1))
                      hermes-tracker--linked-cards '((task-id . "old")))
          (hermes-tracker--display-missing-cached-todo "new" 2 buffer)
          (should-not hermes-tracker--todo)
          (should-not hermes-tracker--linked-cards)
          (should (equal hermes-tracker--repo-slug "new"))
          (should (= hermes-tracker--todo-number 2)))
      (when (buffer-live-p buffer) (kill-buffer buffer)))))

(ert-deftest hermes-tracker-cached-summary-keeps-requested-repository-context ()
  "A summary missing repo_slug cannot orphan its detail refresh."
  (with-temp-buffer
    (hermes-tracker-todo-mode)
    (setq-local hermes-tracker--todo-request-id 1)
    (cl-letf (((symbol-function 'hermes-tracker-sync-cache-origin)
               (lambda () "origin"))
              ((symbol-function 'hermes-tracker-db-get-todo)
               (lambda (&rest _)
                 '((number . 28) (title . "Cached") (status . "open")
                   (priority . 0) (labels . nil))))
              ((symbol-function 'hermes-tracker--linked-cards-for-todo)
               (lambda (_) (hermes--promise-resolved nil))))
      (hermes-tracker--render-cached-todo
       "emacs-hermes" 28 (current-buffer) 1)
      (should (equal hermes-tracker--repo-slug "emacs-hermes")))))

(ert-deftest hermes-tracker-todo-detail-renders-cache-before-sync ()
  "TODO detail renders SQLite before Tracker detail sync settles."
  (let ((pending (hermes--promise-make)))
    (cl-letf (((symbol-function 'hermes-tracker-sync-cache-origin)
               (lambda () "origin"))
              ((symbol-function 'hermes-tracker-db-get-todo)
               (lambda (&rest _)
                 '((repo_slug . "proj") (number . 1) (title . "Cached detail")
                   (status . "open") (priority . 0) (labels . nil))))
              ((symbol-function 'hermes-tracker-sync-todo) (lambda (&rest _) pending))
              ((symbol-function 'hermes-tracker--linked-cards-for-todo)
               (lambda (_) (hermes--promise-resolved nil)))
              ((symbol-function 'hermes-tracker--request-async)
               (lambda (&rest _) (error "View bypassed SQLite")))
              ((symbol-function 'pop-to-buffer) #'ignore))
      (unwind-protect
          (progn
            (hermes-tracker-open-todo "proj" 1)
            (with-current-buffer "*Hermes Tracker TODO*"
              (should (equal (hermes-transport--get hermes-tracker--todo 'title)
                             "Cached detail"))))
        (when (get-buffer "*Hermes Tracker TODO*")
          (kill-buffer "*Hermes Tracker TODO*"))))))

(ert-deftest hermes-tracker-sync-hook-does-not-repurpose-named-buffer ()
  "Background sync ignores same-named buffers outside Tracker modes."
  (let ((buffer (get-buffer-create "*Hermes Tracker Repositories*")))
    (unwind-protect
        (cl-letf (((symbol-function 'hermes-tracker-sync-cache-origin)
                   (lambda () "origin"))
                  ((symbol-function 'hermes-tracker-db-list-repository-summaries)
                   (lambda (_) (error "Non-view buffer was queried"))))
          (with-current-buffer buffer (fundamental-mode))
          (hermes-tracker--sync-changed "origin" '(repos))
          (with-current-buffer buffer
            (should (eq major-mode 'fundamental-mode))))
      (when (buffer-live-p buffer) (kill-buffer buffer)))))

(ert-deftest hermes-tracker-cleared-status-filter-queries-all-cached-todos ()
  "Clearing the compatibility status filter reaches SQLite as all."
  (let ((pending (hermes--promise-make)) captured)
    (cl-letf (((symbol-function 'hermes-tracker-sync-cache-origin)
               (lambda () "origin"))
              ((symbol-function 'hermes-tracker-db-query-todos)
               (lambda (_origin _slug filters) (setq captured filters) nil))
              ((symbol-function 'hermes-tracker-sync-todos) (lambda (_) pending)))
      (with-temp-buffer
        (hermes-tracker-todos-mode)
        (setq-local hermes-tracker--status-filter "all")
        (hermes-tracker--render-todos "proj" "Project" (current-buffer))
        (should (equal captured '(:state "all")))))))

(ert-deftest hermes-tracker-repositories-discard-late-response ()
  "A late repository sync cannot replace the latest cached rendering."
  (let ((old (hermes--promise-make)) (new (hermes--promise-make))
        (calls 0) rows)
    (cl-letf (((symbol-function 'hermes-tracker-sync-cache-origin)
               (lambda () "origin"))
              ((symbol-function 'hermes-tracker-db-list-repository-summaries)
               (lambda (_) rows))
              ((symbol-function 'hermes-tracker-sync-all)
               (lambda () (cl-incf calls) (if (= calls 1) old new)))
              ((symbol-function 'pop-to-buffer) #'ignore))
      (unwind-protect
          (progn
            (hermes-list-tracker-repositories)
            (hermes-list-tracker-repositories)
            (setq rows '(((slug . "new") (name . "New")
                          (summary . ((status_counts . nil))))))
            (hermes--promise-resolve new nil)
            (setq rows '(((slug . "old") (name . "Old")
                          (summary . ((status_counts . nil))))))
            (hermes--promise-resolve old nil)
            (with-current-buffer "*Hermes Tracker Repositories*"
              (should (equal (caar tabulated-list-entries) "new"))))
        (when (get-buffer "*Hermes Tracker Repositories*")
          (kill-buffer "*Hermes Tracker Repositories*"))))))

(ert-deftest hermes-tracker-repositories-preserve-cache-on-sync-error ()
  "A repository sync error leaves cached rows visible."
  (let ((failed (hermes--promise-make)))
    (cl-letf (((symbol-function 'hermes-tracker-sync-cache-origin)
               (lambda () "origin"))
              ((symbol-function 'hermes-tracker-db-list-repository-summaries)
               (lambda (_)
                 '(((slug . "cached") (name . "Cached")
                    (summary . ((status_counts . nil)))))))
              ((symbol-function 'hermes-tracker-sync-all) (lambda () failed))
              ((symbol-function 'pop-to-buffer) #'ignore)
              ((symbol-function 'message) #'ignore))
      (unwind-protect
          (progn
            (hermes-list-tracker-repositories)
            (hermes--promise-reject failed "backend unavailable")
            (with-current-buffer "*Hermes Tracker Repositories*"
              (should (equal (caar tabulated-list-entries) "cached"))
              (should (equal mode-line-process " [Offline cache]"))))
        (when (get-buffer "*Hermes Tracker Repositories*")
          (kill-buffer "*Hermes Tracker Repositories*"))))))

(ert-deftest hermes-tracker-repository-refresh-preserves-selected-row ()
  "Repository refresh keeps point on the same repository."
  (with-temp-buffer
    (hermes-tracker-repositories-mode)
    (hermes-tracker--display-repository-rows
     '(((slug . "a") (name . "Alpha")
        (summary . ((status_counts . ((open . 1))))))
       ((slug . "b") (name . "Beta")
        (summary . ((status_counts . ((active . 1))))))))
    (goto-char (point-min))
    (search-forward "Beta")
    (beginning-of-line)
    (move-to-column 3)
    (hermes-tracker--display-repository-rows
     '(((slug . "b") (name . "Beta changed")
        (summary . ((status_counts . ((active . 2))))))
       ((slug . "a") (name . "Alpha")
        (summary . ((status_counts . ((open . 1))))))))
    (should (equal (tabulated-list-get-id) "b"))
    (should (= (current-column) 3))))

(ert-deftest hermes-tracker-todo-list-discards-late-response ()
  "A late repository A sync cannot replace repository B cached TODOs."
  (let ((a (hermes--promise-make)) (b (hermes--promise-make)) rows)
    (cl-letf (((symbol-function 'hermes-tracker-sync-cache-origin)
               (lambda () "origin"))
              ((symbol-function 'hermes-tracker-db-query-todos)
               (lambda (&rest _) rows))
              ((symbol-function 'hermes-tracker-sync-todos)
               (lambda (slug) (if (equal slug "a") a b)))
              ((symbol-function 'pop-to-buffer) #'ignore))
      (unwind-protect
          (progn
            (hermes-tracker--render-todos "a" "Repo A")
            (hermes-tracker--render-todos "b" "Repo B")
            (setq rows '(((number . 2) (status . "open")
                          (title . "New") (priority . 0) (labels . nil))))
            (hermes--promise-resolve b nil)
            (setq rows '(((number . 1) (status . "open")
                          (title . "Old") (priority . 0) (labels . nil))))
            (hermes--promise-resolve a nil)
            (with-current-buffer "*Hermes Tracker TODOs*"
              (should (equal hermes-tracker--repo-slug "b"))
              (should (= (caar tabulated-list-entries) 2))))
        (when (get-buffer "*Hermes Tracker TODOs*")
          (kill-buffer "*Hermes Tracker TODOs*"))))))

(ert-deftest hermes-tracker-todo-detail-discards-late-response ()
  "A late Dashboard overlay cannot replace the newer cached TODO detail."
  (let ((a (hermes--promise-make)) (b (hermes--promise-make)))
    (cl-letf (((symbol-function 'hermes-tracker-sync-cache-origin)
               (lambda () "origin"))
              ((symbol-function 'hermes-tracker-db-get-todo)
               (lambda (_origin repo number)
                 `((repo_slug . ,repo) (number . ,number)
                   (title . ,(if (equal repo "a") "Old" "New"))
                   (status . "open") (priority . 0) (labels . nil))))
              ((symbol-function 'hermes-tracker-sync-todo)
               (lambda (&rest _) (hermes--promise-resolved nil)))
              ((symbol-function 'hermes-tracker--linked-cards-for-todo)
               (lambda (todo)
                 (if (equal (hermes-transport--get todo 'repo_slug) "a") a b)))
              ((symbol-function 'pop-to-buffer) #'ignore))
      (unwind-protect
          (progn
            (hermes-tracker-open-todo "a" 1)
            (hermes-tracker-open-todo "b" 2)
            (hermes--promise-resolve b nil)
            (hermes--promise-resolve a nil)
            (with-current-buffer "*Hermes Tracker TODO*"
              (should (equal hermes-tracker--repo-slug "b"))
              (should (= hermes-tracker--todo-number 2))
              (should (string-match-p "#2 New" (buffer-string)))
              (should-not (string-match-p "# Old" (buffer-string)))))
        (when (get-buffer "*Hermes Tracker TODO*")
          (kill-buffer "*Hermes Tracker TODO*"))))))

(ert-deftest hermes-tracker-show-todo-survives-unbound-live-mode-map ()
  "Showing a TODO does not depend on a hot-reloaded mode map variable."
  (let ((map-was-bound (boundp 'hermes-tracker-todo-mode-map))
        (saved-map (and (boundp 'hermes-tracker-todo-mode-map)
                        hermes-tracker-todo-mode-map)))
    (cl-letf (((symbol-function 'hermes-tracker-sync-cache-origin)
               (lambda () "origin"))
              ((symbol-function 'hermes-tracker-db-get-todo)
               (lambda (&rest _)
                 '((repo_slug . "proj") (number . 3) (title . "Build it")
                   (status . "open") (priority . 0) (labels . nil))))
              ((symbol-function 'hermes-tracker-sync-todo)
               (lambda (&rest _) (hermes--promise-resolved nil)))
              ((symbol-function 'hermes-tracker--linked-cards-for-todo)
               (lambda (_) (hermes--promise-resolved nil)))
              ((symbol-function 'pop-to-buffer) #'ignore))
      (unwind-protect
          (progn
            (with-current-buffer (get-buffer-create "*Hermes Tracker TODO*")
              (hermes-tracker-todo-mode))
            (makunbound 'hermes-tracker-todo-mode-map)
            (with-temp-buffer
              (hermes-tracker-todos-mode)
              (setq hermes-tracker--repo-slug "proj"
                    tabulated-list-entries
                    '((3 ["3" "open" "0" "" "Build it"])))
              (tabulated-list-print)
              (goto-char (point-min))
              (let ((promise (hermes-tracker-show-todo)))
                (should (eq (hermes--promise-state promise) 'resolved))
                (with-current-buffer "*Hermes Tracker TODO*"
                  (should (= hermes-tracker--todo-number 3))))))
        (if map-was-bound
            (set 'hermes-tracker-todo-mode-map saved-map)
          (makunbound 'hermes-tracker-todo-mode-map))
        (when (get-buffer "*Hermes Tracker TODO*")
          (kill-buffer "*Hermes Tracker TODO*"))))))

(ert-deftest hermes-tracker-repository-refresh-does-not-recreate-killed-owner ()
  "A late repository refresh cannot recreate its killed buffer."
  (let ((pending (hermes--promise-make))
        (buffer (get-buffer-create "*Hermes Tracker Repositories*")))
    (cl-letf (((symbol-function 'hermes-tracker-sync-cache-origin)
               (lambda () "origin"))
              ((symbol-function 'hermes-tracker-db-list-repository-summaries)
               (lambda (_) nil))
              ((symbol-function 'hermes-tracker-sync-all) (lambda () pending))
              ((symbol-function 'pop-to-buffer) #'ignore))
      (unwind-protect
          (progn
            (with-current-buffer buffer
              (hermes-tracker-repositories-mode)
              (hermes-tracker--repositories-revert))
            (kill-buffer buffer)
            (hermes--promise-resolve pending nil)
            (should-not (get-buffer "*Hermes Tracker Repositories*")))
        (when (get-buffer "*Hermes Tracker Repositories*")
          (kill-buffer "*Hermes Tracker Repositories*"))))))

(ert-deftest hermes-tracker-todo-list-refresh-does-not-recreate-killed-owner ()
  "A late TODO-list refresh cannot recreate its killed buffer."
  (let ((pending (hermes--promise-make))
        (buffer (get-buffer-create "*Hermes Tracker TODOs*")))
    (cl-letf (((symbol-function 'hermes-tracker-sync-cache-origin)
               (lambda () "origin"))
              ((symbol-function 'hermes-tracker-db-query-todos)
               (lambda (&rest _) nil))
              ((symbol-function 'hermes-tracker-sync-todos) (lambda (_) pending))
              ((symbol-function 'pop-to-buffer) #'ignore))
      (unwind-protect
          (progn
            (with-current-buffer buffer
              (hermes-tracker-todos-mode)
              (setq-local hermes-tracker--repo-slug "proj"
                          hermes-tracker--repo-name "Project")
              (hermes-tracker--todos-revert))
            (kill-buffer buffer)
            (hermes--promise-resolve pending '((todos . nil)))
            (should-not (get-buffer "*Hermes Tracker TODOs*")))
        (when (get-buffer "*Hermes Tracker TODOs*")
          (kill-buffer "*Hermes Tracker TODOs*"))))))

(ert-deftest hermes-tracker-todo-refresh-does-not-recreate-killed-owner ()
  "A late TODO-detail refresh cannot recreate its killed buffer."
  (let ((pending (hermes--promise-make))
        (buffer (get-buffer-create "*Hermes Tracker TODO*")))
    (cl-letf (((symbol-function 'hermes-tracker-sync-cache-origin)
               (lambda () "origin"))
              ((symbol-function 'hermes-tracker-db-get-todo) (lambda (&rest _) nil))
              ((symbol-function 'hermes-tracker-sync-todo) (lambda (&rest _) pending))
              ((symbol-function 'pop-to-buffer) #'ignore))
      (unwind-protect
          (progn
            (with-current-buffer buffer
              (hermes-tracker-todo-mode)
              (setq-local hermes-tracker--repo-slug "proj"
                          hermes-tracker--todo-number 3)
              (hermes-tracker--todo-revert))
            (kill-buffer buffer)
            (hermes--promise-resolve
             pending '(((repo_slug . "proj") (number . 3) (title . "Late"))))
            (should-not (get-buffer "*Hermes Tracker TODO*")))
        (when (get-buffer "*Hermes Tracker TODO*")
          (kill-buffer "*Hermes Tracker TODO*"))))))

(ert-deftest hermes-tracker-linked-card-finds-stale-board-metadata ()
  "A linked task moved to another board is reported as stale-board, not missing."
  (let* ((todo '((repo_slug . "proj") (number . 3)))
         (reference '((external_id . "t_1234abcd")
                      (metadata . ((board . "old")))))
         card calls)
    (cl-letf (((symbol-function
                'hermes-dashboard-transport-api-request-async)
               (lambda (_method path &rest args)
                 (let ((board (cdr (assq 'board (plist-get args :query)))))
                   (push (list path board) calls)
                   (cond
                    ((equal path "/api/plugins/kanban/boards")
                     (hermes--promise-resolved
                      '((boards . (((slug . "new")))))))
                    ((equal board "new")
                     (hermes--promise-resolved
                      '((task . ((id . "t_1234abcd") (title . "Moved")
                                  (status . "todo")
                                  (body . "```tracker-ref\n{\"number\":3,\"repo_slug\":\"proj\"}\n```"))))))
                    (t
                     (hermes--promise-rejected
                      "Hermes dashboard request failed (HTTP 404)")))))))
      (hermes--promise-then
       (hermes-tracker--fetch-linked-card todo reference)
       (lambda (value) (setq card value)))
      (should (equal (plist-get card :board) "new"))
      (should (eq (plist-get card :drift) 'stale-board))
      (should (member '("/api/plugins/kanban/boards" nil) calls)))))

(ert-deftest hermes-tracker-linked-card-propagates-primary-fetch-failure ()
  "A non-404 primary task fetch failure is not reported as a missing card."
  (let (rejected)
    (cl-letf (((symbol-function
                'hermes-dashboard-transport-api-request-async)
               (lambda (&rest _)
                 (hermes--promise-rejected
                  "Hermes dashboard request failed (HTTP 500)"))))
      (hermes--promise-catch
       (hermes-tracker--fetch-linked-card
        '((repo_slug . "proj") (number . 3))
        '((external_id . "t_1234abcd")
          (metadata . ((board . "default")))))
       (lambda (reason) (setq rejected reason)))
      (should (string-match-p "HTTP 500" rejected)))))

(ert-deftest hermes-tracker-linked-card-propagates-partial-board-failure ()
  "An incomplete fallback scan cannot turn an unknown card into a missing card."
  (let (rejected)
    (cl-letf (((symbol-function
                'hermes-dashboard-transport-api-request-async)
               (lambda (_method path &rest args)
                 (let ((board (cdr (assq 'board (plist-get args :query)))))
                   (cond
                    ((equal path "/api/plugins/kanban/boards")
                     (hermes--promise-resolved
                      '((boards . (((slug . "a")) ((slug . "b")))))))
                    ((member board '("default" "a"))
                     (hermes--promise-rejected
                      "Hermes dashboard request failed (HTTP 404)"))
                    (t
                     (hermes--promise-rejected
                      "Hermes dashboard request failed (HTTP 500)")))))))
      (hermes--promise-catch
       (hermes-tracker--fetch-linked-card
        '((repo_slug . "proj") (number . 3))
        '((external_id . "t_1234abcd")
          (metadata . ((board . "default")))))
       (lambda (reason) (setq rejected reason)))
      (should (string-match-p "HTTP 500" rejected)))))

(ert-deftest hermes-tracker-linked-card-prefers-match-over-partial-failure ()
  "A fallback match remains authoritative when a different board fetch fails."
  (let (card)
    (cl-letf (((symbol-function
                'hermes-dashboard-transport-api-request-async)
               (lambda (_method path &rest args)
                 (let ((board (cdr (assq 'board (plist-get args :query)))))
                   (cond
                    ((equal path "/api/plugins/kanban/boards")
                     (hermes--promise-resolved
                      '((boards . (((slug . "found")) ((slug . "broken")))))))
                    ((equal board "default")
                     (hermes--promise-rejected
                      "Hermes dashboard request failed (HTTP 404)"))
                    ((equal board "found")
                     (hermes--promise-resolved
                      '((task . ((id . "t_1234abcd") (title . "Moved")
                                  (body . "```tracker-ref\n{\"number\":3,\"repo_slug\":\"proj\"}\n```"))))))
                    (t
                     (hermes--promise-rejected
                      "Hermes dashboard request failed (HTTP 500)")))))))
      (hermes--promise-then
       (hermes-tracker--fetch-linked-card
        '((repo_slug . "proj") (number . 3))
        '((external_id . "t_1234abcd")
          (metadata . ((board . "default")))))
       (lambda (value) (setq card value)))
      (should (equal (plist-get card :board) "found"))
      (should (eq (plist-get card :drift) 'stale-board)))))

(ert-deftest hermes-tracker-mutation-invalidates-before-transport ()
  "Mutations fence reads before HTTP and after successful settlement."
  (let (events)
    (cl-letf (((symbol-function 'hermes-tracker-sync-note-mutation)
               (lambda (repo) (push (list 'invalidate repo) events)))
              ((symbol-function 'hermes-tracker--request-async)
               (lambda (method path &rest _)
                 (push (list 'request method path) events)
                 (hermes--promise-resolved nil))))
      (hermes-tracker--mutation-request-async
       "PATCH" "/api/v1/repos/proj/todos/3" "proj" '((priority . 1)))
      (should (equal (nreverse events)
                     '((invalidate "proj")
                       (request "PATCH" "/api/v1/repos/proj/todos/3")
                       (invalidate "proj")))))))

(ert-deftest hermes-tracker-create-todo-reads-multiline-description ()
  "TODO creation reads its description through the multiline editor."
  (let (request refreshed)
    (cl-letf (((symbol-function 'read-string) (lambda (&rest _) "Title"))
              ((symbol-function 'read-string-from-buffer)
               (lambda (prompt initial)
                 (should (equal prompt "Description (optional): "))
                 (should (equal initial ""))
                 "line one\nline two"))
              ((symbol-function 'read-number) (lambda (&rest _) 4))
              ((symbol-function 'hermes-tracker--request-async)
               (lambda (method path &optional body invocation)
                 (setq request (list method path body invocation))
                 (hermes--promise-resolved '((number . 1)))))
              ((symbol-function 'revert-buffer)
               (lambda (&rest _) (setq refreshed t))))
      (with-temp-buffer
        (hermes-tracker-todos-mode)
        (setq-local hermes-tracker--repo-slug "proj")
        (hermes-tracker-create-todo))
      (should refreshed)
      (should (equal (cdr (assq 'description (nth 2 request)))
                     "line one\nline two")))))

(ert-deftest hermes-tracker-create-todo-refreshes-origin-buffer ()
  "TODO creation refreshes its owner when resolved from another buffer."
  (let ((pending (hermes--promise-make))
        (origin (generate-new-buffer " *tracker-origin*"))
        (resolver (generate-new-buffer " *tracker-resolver*"))
        refreshed)
    (cl-letf (((symbol-function 'read-string) (lambda (&rest _) "Title"))
              ((symbol-function 'read-string-from-buffer)
               (lambda (&rest _) "Description"))
              ((symbol-function 'read-number) (lambda (&rest _) 1))
              ((symbol-function 'hermes-tracker--request-async)
               (lambda (&rest _) pending))
              ((symbol-function 'revert-buffer)
               (lambda (&rest _) (setq refreshed (current-buffer)))))
      (unwind-protect
          (progn
            (with-current-buffer origin
              (hermes-tracker-todos-mode)
              (setq-local hermes-tracker--repo-slug "proj")
              (hermes-tracker-create-todo))
            (with-current-buffer resolver
              (hermes--promise-resolve pending '((number . 1))))
            (should (eq refreshed origin)))
        (when (buffer-live-p origin) (kill-buffer origin))
        (when (buffer-live-p resolver) (kill-buffer resolver))))))

(ert-deftest hermes-tracker-create-todo-does-not-refresh-stale-owner ()
  "A superseded TODO creation cannot refresh an older list owner."
  (let ((pending (hermes--promise-make))
        refreshed)
    (cl-letf (((symbol-function 'read-string) (lambda (&rest _) "Title"))
              ((symbol-function 'read-string-from-buffer)
               (lambda (&rest _) "Description"))
              ((symbol-function 'read-number) (lambda (&rest _) 1))
              ((symbol-function 'hermes-tracker--request-async)
               (lambda (&rest _) pending))
              ((symbol-function 'revert-buffer)
               (lambda (&rest _) (setq refreshed t))))
      (with-temp-buffer
        (hermes-tracker-todos-mode)
        (setq-local hermes-tracker--repo-slug "proj")
        (hermes-tracker-create-todo)
        (hermes-tracker--begin-request 'hermes-tracker--todos-request-id)
        (hermes--promise-resolve pending '((number . 1)))
        (should-not refreshed)))))

(ert-deftest hermes-tracker-comment-reads-multiline-body ()
  "Tracker comments preserve multiline text from the editor."
  (let (request refreshed)
    (cl-letf (((symbol-function 'read-string-from-buffer)
               (lambda (prompt initial)
                 (should (equal prompt "Comment: "))
                 (should (equal initial ""))
                 "first\nsecond"))
              ((symbol-function 'hermes-tracker--request-async)
               (lambda (method path &optional body invocation)
                 (setq request (list method path body invocation))
                 (hermes--promise-resolved '((ok . t)))))
              ((symbol-function 'revert-buffer)
               (lambda (&rest _) (setq refreshed t))))
      (with-temp-buffer
        (setq-local major-mode 'hermes-tracker-todo-mode
                    hermes-tracker--repo-slug "proj"
                    hermes-tracker--todo-number 3)
        (hermes-tracker-comment))
      (should refreshed)
      (should (equal (cdr (assq 'body (nth 2 request))) "first\nsecond")))))

(ert-deftest hermes-tracker-close-is-explicitly-evidence-gated ()
  "Interactive closure stops before any unsupported HTTP route."
  (let (called)
    (cl-letf (((symbol-function 'hermes-tracker--request-async)
               (lambda (&rest _) (setq called t))))
      (should-error (hermes-tracker-close-todo) :type 'user-error)
      (should-not called))))

(ert-deftest hermes-tracker-set-labels-completes-from-cache-and-patches ()
  "Label mutation completes locally and sends the selected canonical names."
  (let ((hermes-tracker-url "https://tracker.test")
        choices initial require-match request refreshed)
    (cl-letf (((symbol-function 'hermes-tracker-db-list-labels)
               (lambda (_origin)
                 '(((name . "urgent") (color . "orange")))))
              ((symbol-function 'hermes-tracker-db-get-todo)
               (lambda (&rest _) '((labels . ("bug")))))
              ((symbol-function 'completing-read-multiple)
               (lambda (_prompt collection _predicate required
                                &optional initial-input &rest _)
                 (setq choices collection
                       initial initial-input
                       require-match required)
                 '("bug" "urgent")))
              ((symbol-function 'hermes-tracker--request-async)
               (lambda (method path &optional body invocation)
                 (setq request (list method path body invocation))
                 (hermes--promise-resolved '((labels . ["bug" "urgent"])))))
              ((symbol-function 'revert-buffer)
               (lambda (&rest _) (setq refreshed t))))
      (with-temp-buffer
        (setq-local major-mode 'hermes-tracker-todo-mode
                    hermes-tracker--repo-slug "proj"
                    hermes-tracker--todo-number 3)
        (hermes-tracker-set-labels))
      (should refreshed)
      (should require-match)
      (should (equal choices '("bug" "urgent")))
      (should (equal initial "bug,"))
      (should (equal request
                     '("PATCH" "/api/v1/repos/proj/todos/3"
                       ((labels . ["bug" "urgent"])) nil))))))

(ert-deftest hermes-tracker-clear-labels-encodes-empty-array ()
  "Clearing labels sends an empty array rather than JSON null."
  (let ((hermes-tracker-url "https://tracker.test") captured)
    (cl-letf (((symbol-function 'hermes-tracker-db-list-labels)
               (lambda (_origin) nil))
              ((symbol-function 'hermes-tracker-db-get-todo)
               (lambda (&rest _) '((labels . nil))))
              ((symbol-function 'completing-read-multiple)
               (lambda (&rest _) nil))
              ((symbol-function 'hermes-tracker--token) (lambda (_) "token"))
              ((symbol-function 'revert-buffer) #'ignore))
      (let ((hermes-dashboard-transport-http-request-async-function
             (lambda (_url &rest args)
               (setq captured args)
               (hermes--promise-resolved '(:status 200 :body nil)))))
        (with-temp-buffer
          (setq-local major-mode 'hermes-tracker-todo-mode
                      hermes-tracker--repo-slug "proj"
                      hermes-tracker--todo-number 3)
          (hermes-tracker-set-labels))
        (should (equal (plist-get captured :data) "{\"labels\":[]}"))))))

(ert-deftest hermes-tracker-claim-patches-assignee ()
  "Claim uses the pinned API's ordinary update route."
  (let (request refreshed)
    (cl-letf (((symbol-function 'read-string) (lambda (&rest _) "dev"))
              ((symbol-function 'hermes-tracker--request-async)
               (lambda (method path &optional body invocation)
                 (setq request (list method path body invocation))
                 (hermes--promise-resolved '((assignee . "dev")))))
              ((symbol-function 'revert-buffer)
               (lambda (&rest _) (setq refreshed t))))
      (with-temp-buffer
        (setq-local major-mode 'hermes-tracker-todo-mode
                    hermes-tracker--repo-slug "proj"
                    hermes-tracker--todo-number 3)
        (hermes-tracker-claim-todo))
      (should refreshed)
      (should (equal request
                     '("PATCH" "/api/v1/repos/proj/todos/3"
                       ((assignee . "dev")) nil))))))

(ert-deftest hermes-tracker-release-patches-assignee-and-refreshes-on-success ()
  "Release clears assignee through the pinned API's update route."
  (let (request refreshed)
    (cl-letf (((symbol-function 'hermes-tracker--request-async)
               (lambda (method path &optional body invocation)
                 (setq request (list method path body invocation))
                 (hermes--promise-resolved '((status . "open")))))
              ((symbol-function 'revert-buffer)
               (lambda (&rest _) (setq refreshed t))))
      (with-temp-buffer
        (setq-local major-mode 'hermes-tracker-todo-mode
                    hermes-tracker--repo-slug "proj"
                    hermes-tracker--todo-number 3)
        (hermes-tracker-release-todo))
      (should refreshed)
      (should (equal (seq-take request 2)
                     '("PATCH" "/api/v1/repos/proj/todos/3")))
      (should (equal (nth 2 request) '((assignee . :null)))))))

(ert-deftest hermes-tracker-release-encodes-null-assignee ()
  "Release sends explicit null assignee required by the update API."
  (let ((hermes-tracker-url "https://tracker.test")
        captured)
    (cl-letf (((symbol-function 'hermes-tracker--token) (lambda (_) "token"))
              ((symbol-function 'revert-buffer) #'ignore))
      (let ((hermes-dashboard-transport-http-request-async-function
             (lambda (_url &rest args)
               (setq captured args)
               (hermes--promise-resolved '(:status 200 :body nil)))))
        (with-temp-buffer
          (setq-local major-mode 'hermes-tracker-todo-mode
                      hermes-tracker--repo-slug "proj"
                      hermes-tracker--todo-number 3)
          (hermes-tracker-release-todo)))
      (should (equal (plist-get captured :data) "{\"assignee\":null}"))
      (should (equal (cdr (assoc "Content-Type" (plist-get captured :headers)))
                     "application/json")))))

(ert-deftest hermes-tracker-release-rejection-does-not-refresh ()
  "A rejected release reports the backend reason without refreshing."
  (let (messages refreshed)
    (cl-letf (((symbol-function 'hermes-tracker--request-async)
               (lambda (&rest _)
                 (hermes--promise-rejected "only active TODOs can be released")))
              ((symbol-function 'revert-buffer)
               (lambda (&rest _) (setq refreshed t)))
              ((symbol-function 'message)
               (lambda (format-string &rest args)
                 (push (apply #'format format-string args) messages))))
      (with-temp-buffer
        (setq-local major-mode 'hermes-tracker-todo-mode
                    hermes-tracker--repo-slug "proj"
                    hermes-tracker--todo-number 3)
        (hermes-tracker-release-todo))
      (should-not refreshed)
      (should (seq-some (lambda (text)
                          (string-match-p "only active TODOs" text))
                        messages)))))

(ert-deftest hermes-tracker-link-kanban-is-explicitly-unsupported ()
  "The client never calls absent Tracker external-reference routes."
  (let (called)
    (cl-letf (((symbol-function 'hermes-tracker--request-async)
               (lambda (&rest _) (setq called t))))
      (should-error (hermes-tracker-link-kanban) :type 'user-error)
      (should-not called))))

(ert-deftest hermes-tracker-kanban-detail-renders-reference-button ()
  "A canonical Kanban reference renders a working Tracker button."
  (let (opened)
    (cl-letf (((symbol-function 'pop-to-buffer) #'switch-to-buffer)
              ((symbol-function 'hermes-tracker-open-todo)
               (lambda (repo number) (setq opened (list repo number)))))
      (unwind-protect
          (progn
            (hermes-kanban--display-task
             '((task . ((id . "t_1234abcd") (title . "Tracked")
                        (status . "todo")
                        (body . "Body\n\n```tracker-ref\n{\"number\":3,\"repo_slug\":\"proj\"}\n```"))))
             "default")
            (with-current-buffer "*Hermes Kanban Task*"
              (should (string-match-p "Tracker: proj#3" (buffer-string)))
              (should (equal hermes-tracker--kanban-reference
                             '(:repo-slug "proj" :number 3)))
              (hermes-kanban-open-tracker)
              (goto-char (point-min))
              (search-forward "proj#3")
              (button-activate (button-at (1- (point)))))
            (should (equal opened '("proj" 3))))
        (when (get-buffer "*Hermes Kanban Task*")
          (kill-buffer "*Hermes Kanban Task*"))))))

(ert-deftest hermes-tracker-ignores-kanban-task-without-reference ()
  "A Kanban task without a canonical reference adds no Tracker text."
  (with-temp-buffer
    (hermes-tracker--render-kanban-reference
     '((task . ((body . "Ordinary task body")))) "default")
    (should-not hermes-tracker--kanban-reference)
    (should (string-empty-p (buffer-string)))
    (should-error (hermes-kanban-open-tracker) :type 'user-error)))

(provide 'hermes-tracker-tests)
;;; hermes-tracker-tests.el ends here
