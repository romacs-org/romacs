;;; romacs-mail.el --- Buffer-owned mail views -*- lexical-binding: t -*-

;; SPDX-License-Identifier: GPL-3.0-or-later

;;; Commentary:

;; Mail views consume account-qualified plain data.  Account operations own
;; backend queries and decoding; the runner owns bounded external effects.

;;; Code:

(defvar romacs-mail-runner 'romacs-process-start)
(defvar romacs-mail-account nil)
(defvar romacs-mail-owner nil)
(defvar romacs-mail-job nil)
(defvar romacs-mail-query nil)
(defvar romacs-mail-rows nil)
(defvar romacs-mail-thread nil)
(defvar romacs-mail-message nil)
(defvar romacs-mail-parent nil)
(defvar romacs-mail-children nil)
(defvar romacs-mail-active-key nil)
(defvar romacs-mail-pair-window nil)
(defvar romacs-mail-pair-buffer nil)
(defvar romacs-mail-tree-window nil)
(defvar romacs-mail-envelope nil)
(defvar romacs-mail-send-state nil)

(defface notmuch-search-date '((t (:foreground "#00b8ad"))) "Mail dates.")
(defface notmuch-search-subject '((t (:foreground "#d4c5a0"))) "Mail subjects.")
(defface notmuch-tag '((t (:foreground "#72b6e4"))) "Mail tags.")
(defface notmuch-message-summary '((t (:foreground "#b09bd2"))) "Mail headers.")
(defface notmuch-tree-no-match-face '((t (:foreground "#888888"))) "Mail context rows.")

(defun romacs-mail-header (name headers) (cdr (assoc name headers)))
(defun romacs-mail-error (text) (signal 'user-error (list text)))

(defun romacs-mail-clean (text &optional line)
  "Return inert TEXT, retaining newlines unless LINE is non-nil."
  (mapconcat (lambda (c)
               (if (or (and (< c 32) (or line (/= c 10)))
                       (and (>= c 127) (<= c 159))
                       (and (>= c 8234) (<= c 8238))
                       (and (>= c 8294) (<= c 8297)))
                   " " (char-to-string c))) (or text "") ""))

(defun romacs-mail-pad (text width)
  "Return inert TEXT clipped and padded to WIDTH characters."
  (let ((clean (romacs-mail-clean text t)))
    (concat (substring clean 0 (min width (length clean)))
            (make-string (max 0 (- width (length clean))) 32))))

(defun romacs-mail-tags (tags)
  (propertize (mapconcat (lambda (tag) (romacs-mail-clean tag t)) tags " ") 'face 'notmuch-tag))

(defun romacs-mail-owned-p (buffer owner mode)
  "Return non-nil when BUFFER still owns OWNER in MODE."
  (and (buffer-live-p buffer)
       (with-current-buffer buffer
         (and (eq romacs-mail-owner owner) (eq major-mode mode) (not buffer-file-name)))))

(defun romacs-mail-cancel ()
  "Cancel only this buffer's pending read request."
  (when (and romacs-mail-job (not (eq romacs-mail-send-state 'sending)))
    (condition-case nil (romacs-process-forget romacs-mail-job) (error nil))
    (setq romacs-mail-job nil)))

(defun romacs-mail-retire ()
  "Retire this view's request and close only its owned message pane."
  (romacs-mail-cancel)
  (when (eq major-mode 'romacs-mail-tree-mode)
    (romacs-mail-close-pair (current-buffer))))

(defun romacs-mail-initialize (account mode label map parent)
  "Initialize a new owned mail buffer with ACCOUNT, MODE, LABEL, MAP and PARENT."
  (dolist (name '(romacs-mail-account romacs-mail-owner romacs-mail-job romacs-mail-query
                  romacs-mail-rows romacs-mail-thread romacs-mail-message romacs-mail-parent
                  romacs-mail-children romacs-mail-active-key romacs-mail-pair-window
                  romacs-mail-pair-buffer romacs-mail-tree-window romacs-mail-envelope
                  romacs-mail-send-state))
    (set (make-local-variable name) nil))
  (setq romacs-mail-account account romacs-mail-owner (list mode) romacs-mail-parent parent
        major-mode mode mode-name label buffer-file-name nil)
  (use-local-map map)
  (set (make-local-variable 'mode-line-format) (list "%b" " | " label))
  (set (make-local-variable 'kill-buffer-hook) '(romacs-mail-retire t)))

(defun romacs-mail-text (text)
  "Publish TEXT in the owned buffer without adding save modifications."
  (setq romacs-mail-rows nil)
  (let ((inhibit-read-only t)) (widen) (erase-buffer) (insert text) (goto-char (point-min)))
  (set-buffer-modified-p nil))

(defun romacs-mail-request (account operation arguments buffer owner mode on-data)
  "Request OPERATION with ARGUMENTS and apply ON-DATA only to its owned BUFFER."
  (unless (and (memq operation (plist-get account :capabilities)) (plist-get account operation))
    (romacs-mail-error (format "This mail account cannot %s" operation)))
  (let* ((request (apply (plist-get account operation) arguments))
         (argv (car request)) (decode (cdr request)))
    (funcall romacs-mail-runner argv "" default-directory 30
             (lambda (output)
               (when (romacs-mail-owned-p buffer owner mode)
                 (with-current-buffer buffer
                   (setq romacs-mail-job nil)
                   (condition-case err
                       (progn
                         (unless (and (eq (car output) 0) (not (nth 3 output)))
                           (error "%s" (romacs-mail-clean
                                        (or (nth 3 output)
                                            (and (not (equal (nth 2 output) "")) (nth 2 output))
                                            "Mail command failed") t)))
                         (funcall on-data (funcall decode (romacs-json-read (nth 1 output)))))
                     (error (romacs-mail-text
                             (format "%s failed: %s\n" mode-name (romacs-mail-clean (format "%s" err) t)))))
                   (not (null (get-buffer-window buffer)))))))))

(defun romacs-mail-row () (cdr (assoc (line-beginning-position) romacs-mail-rows)))
(defun romacs-mail-row-key (row) (or (plist-get row :key) (plist-get row :thread)))

(defun romacs-mail-search-line (row)
  (concat (propertize (romacs-mail-pad (plist-get row :date-label) 13) 'face 'notmuch-search-date)
          " [" (number-to-string (or (plist-get row :matched) 0)) "/"
          (number-to-string (or (plist-get row :total) 0)) "] "
          (romacs-mail-pad (plist-get row :authors-matched) 24) " "
          (propertize (romacs-mail-clean (plist-get row :subject) t) 'face 'notmuch-search-subject)
          " (" (romacs-mail-tags (plist-get row :tags)) ")\n"))

(defun romacs-mail-tree-prefix (row)
  "Return connectors derived only from ROW's structural ancestry."
  (let* ((lasts (cdr (plist-get row :lasts))) (depth (plist-get row :depth))
         (reversed (reverse lasts)))
    (if (= depth 0) ""
      (concat (mapconcat (lambda (last) (if last "  " "│ ")) (reverse (cdr reversed)) "")
              (if (car reversed) "└─" "├─")))))

(defun romacs-mail-tree-line (row)
  (let* ((subject (concat (romacs-mail-tree-prefix row) (romacs-mail-clean (plist-get row :subject) t)))
         (matching (and (plist-get row :match) (not (plist-get row :excluded)))))
    (concat (if (equal (plist-get row :key) romacs-mail-active-key) "▶ " "  ")
            (propertize (romacs-mail-pad (plist-get row :date-label) 12) 'face 'notmuch-search-date) " "
            (romacs-mail-pad (plist-get row :from) 20) " "
            (propertize subject 'face (if matching 'notmuch-search-subject 'notmuch-tree-no-match-face))
            (if (plist-get row :excluded) " [excluded]" "")
            " (" (romacs-mail-tags (plist-get row :tags)) ")\n")))

(defun romacs-mail-publish-rows (rows formatter key footer)
  "Publish ROWS with FORMATTER, restoring KEY when it survives."
  (let ((positions nil) (target nil))
    (let ((inhibit-read-only t))
      (widen) (erase-buffer)
      (while rows
        (let ((row (car rows)))
          (when (equal key (romacs-mail-row-key row)) (setq target (point)))
          (setq positions (cons (cons (point) row) positions))
          (insert (funcall formatter row)))
        (setq rows (cdr rows)))
      (insert footer))
    (setq romacs-mail-rows (nreverse positions))
    (goto-char (or target (point-min))) (set-buffer-modified-p nil)))

(defun romacs-mail-search (account query)
  "Open a read-only search for opaque QUERY using ACCOUNT."
  (let ((parent (current-buffer)))
    (switch-to-buffer (get-buffer-create (generate-new-buffer-name "*mail-search*")))
    (romacs-mail-initialize account 'romacs-mail-search-mode "Mail Search" romacs-mail-search-map parent)
    (setq romacs-mail-query query buffer-read-only t)
    (romacs-mail-refresh)))

(defun romacs-mail-refresh ()
  "Refresh the current view while retaining its semantic row identity."
  (interactive)
  (unless (memq major-mode '(romacs-mail-search-mode romacs-mail-tree-mode))
    (romacs-mail-error "Not a mail collection"))
  (let ((key (romacs-mail-row-key (romacs-mail-row))))
    (romacs-mail-cancel)
    (setq romacs-mail-owner (list romacs-mail-query))
    (if (eq major-mode 'romacs-mail-tree-mode)
        (romacs-mail-load-tree key nil)
      (romacs-mail-text "Searching…\n")
      (setq romacs-mail-job
            (romacs-mail-request romacs-mail-account :search
                                 (list romacs-mail-query 0 50 'newest)
                                 (current-buffer) romacs-mail-owner major-mode
                                 (lambda (rows)
                                   (romacs-mail-publish-rows rows 'romacs-mail-search-line key
                                    (if (= (length rows) 50) "Showing 50 threads; narrow the search for more.\n"
                                      "End of search results.\n"))))))))

(defun romacs-mail-load-tree (key fallback)
  "Load this thread, retaining KEY and falling back from an empty search context."
  (romacs-mail-text "Loading thread…\n")
  (setq romacs-mail-job
        (romacs-mail-request romacs-mail-account :tree
                             (list (cdr romacs-mail-thread) (unless fallback romacs-mail-query))
                             (current-buffer) romacs-mail-owner major-mode
                             (lambda (rows)
                               (if (and (null rows) romacs-mail-query (not fallback))
                                   (romacs-mail-load-tree key t)
                                 (let ((chosen (or key
                                                  (romacs-mail-row-key
                                                   (catch 'found
                                                     (dolist (row rows)
                                                       (when (plist-get row :match) (throw 'found row))))))))
                                   (romacs-mail-publish-rows rows 'romacs-mail-tree-line chosen "End of thread.\n")))))))

(defun romacs-mail-open-tree ()
  "Open the exact thread on this search row, retaining its query context."
  (interactive)
  (let* ((row (romacs-mail-row)) (thread (plist-get row :thread))
         (parent (current-buffer)) (account romacs-mail-account) (query romacs-mail-query)
         (existing (cdr (assoc thread romacs-mail-children))))
    (unless (and (eq major-mode 'romacs-mail-search-mode) thread)
      (romacs-mail-error "No thread on this row"))
    (if (and (buffer-live-p existing)
             (with-current-buffer existing
               (and (eq major-mode 'romacs-mail-tree-mode) (not buffer-file-name)
                    (eq romacs-mail-parent parent))))
        (switch-to-buffer existing)
      (let ((buffer (get-buffer-create (generate-new-buffer-name "*mail-tree*"))))
        (setq romacs-mail-children (cons (cons thread buffer) romacs-mail-children))
        (switch-to-buffer buffer)
        (romacs-mail-initialize account 'romacs-mail-tree-mode "Mail Tree" romacs-mail-tree-map parent)
        (setq romacs-mail-thread thread romacs-mail-query query buffer-read-only t)
        (romacs-mail-load-tree nil nil)))))

(defun romacs-mail-plain-parts (parts)
  "Render normalized PARTS as inert text, not executable MIME."
  (mapconcat
   (lambda (part)
     (let ((type (plist-get part :type)) (children (plist-get part :children)))
       (cond ((equal (plist-get part :disposition) "attachment")
              (format "[Attachment %s: %s (%s)]\n" (or (plist-get part :id) "")
                      (romacs-mail-clean (plist-get part :filename) t) (romacs-mail-clean type t)))
             ((equal type "text/plain") (romacs-mail-clean (plist-get part :content)))
             ((equal type "multipart/alternative")
              (let ((plain nil))
                (dolist (child children)
                  (when (equal (plist-get child :type) "text/plain") (setq plain child)))
                (if plain (romacs-mail-plain-parts (list plain)) "[HTML message not displayed]\n")))
             (children (romacs-mail-plain-parts children))
             (t (concat "[" (romacs-mail-clean type t) " not displayed]\n"))))) parts "\n"))

(defun romacs-mail-message-text (message)
  "Return readable headers and inert MIME text for one normalized MESSAGE."
  (let ((headers (plist-get message :headers)))
    (concat (propertize (romacs-mail-clean (romacs-mail-header "From" headers) t) 'face 'notmuch-message-summary)
            " (" (romacs-mail-tags (plist-get message :tags)) ")\n"
            "Subject: " (romacs-mail-clean (romacs-mail-header "Subject" headers) t) "\n"
            "To: " (romacs-mail-clean (romacs-mail-header "To" headers) t) "\n"
            (if (romacs-mail-header "Cc" headers) (concat "Cc: " (romacs-mail-clean (romacs-mail-header "Cc" headers) t) "\n") "")
            "Date: " (romacs-mail-clean (romacs-mail-header "Date" headers) t) "\n\n"
            (romacs-mail-plain-parts (plist-get message :parts)) "\n")))

(defun romacs-mail-pair-owned-p (tree)
  "Return non-nil when TREE still owns its exact message window and buffer."
  (with-current-buffer tree
    (and (window-live-p romacs-mail-pair-window)
         (buffer-live-p romacs-mail-pair-buffer)
         (eq (window-buffer romacs-mail-pair-window) romacs-mail-pair-buffer)
         (with-current-buffer romacs-mail-pair-buffer
           (and (eq major-mode 'romacs-mail-message-mode) (not buffer-file-name)
                (eq romacs-mail-parent tree))))))

(defun romacs-mail-row-data ()
  "Return the immutable row values from the current positional projection."
  (let ((rows nil))
    (dolist (entry romacs-mail-rows) (setq rows (cons (cdr entry) rows)))
    (nreverse rows)))

(defun romacs-mail-show-message ()
  "Pop the exact selected message into this tree's reusable message pane."
  (interactive)
  (unless (eq major-mode 'romacs-mail-tree-mode) (romacs-mail-error "Not a mail tree"))
  (let* ((row (romacs-mail-row)) (key (plist-get row :key)) (tree (current-buffer))
         (account romacs-mail-account) (anchor (get-buffer-window tree))
         (paired (and (romacs-mail-pair-owned-p tree) romacs-mail-pair-window)))
    (unless key (romacs-mail-error "No message on this row"))
    (unless (window-live-p anchor) (romacs-mail-error "The mail tree is not displayed"))
    (let ((buffer (if paired romacs-mail-pair-buffer
                    (get-buffer-create (generate-new-buffer-name "*mail-message*")))))
    (setq romacs-mail-active-key key)
    (romacs-mail-publish-rows (romacs-mail-row-data) 'romacs-mail-tree-line key "End of thread.\n")
    (with-current-buffer buffer
      (romacs-mail-cancel)
      (romacs-mail-initialize account 'romacs-mail-message-mode "Mail Message" romacs-mail-message-map tree)
      (setq buffer-read-only t romacs-mail-active-key key)
      (romacs-mail-text "Loading message…\n"))
    (let ((window (romacs-display-buffer-paired buffer anchor paired 0.25)))
      (setq romacs-mail-pair-window window romacs-mail-pair-buffer buffer romacs-mail-tree-window anchor)
      (select-window window) (set-buffer buffer))
    (setq romacs-mail-job
          (romacs-mail-request account :message (list (cdr key)) buffer romacs-mail-owner major-mode
                               (lambda (message)
                                 (unless (equal key (plist-get message :key)) (error "Message identity changed"))
                                 (setq romacs-mail-message message)
                                 (romacs-mail-text (romacs-mail-message-text message))))))))

(defun romacs-mail-move (direction all)
  "Move DIRECTION among matching rows, or every row when ALL is non-nil."
  (let* ((rows (if (> direction 0) romacs-mail-rows (reverse romacs-mail-rows)))
         (position (line-beginning-position)) (target nil))
    (dolist (entry rows)
      (when (and (not target)
                 (if (> direction 0) (> (car entry) position) (< (car entry) position))
                 (or all (eq major-mode 'romacs-mail-search-mode) (plist-get (cdr entry) :match)))
        (setq target (car entry))))
    (when target (goto-char target))))
(defun romacs-mail-next () (interactive) (romacs-mail-move 1 nil))
(defun romacs-mail-previous () (interactive) (romacs-mail-move -1 nil))
(defun romacs-mail-next-all () (interactive) (romacs-mail-move 1 t))
(defun romacs-mail-previous-all () (interactive) (romacs-mail-move -1 t))

(defun romacs-mail-close-pair (tree)
  "Close only the message pane currently owned by TREE."
  (when (and (buffer-live-p tree) (romacs-mail-pair-owned-p tree))
    (with-current-buffer tree
      (let ((window romacs-mail-pair-window) (buffer romacs-mail-pair-buffer)
            (anchor romacs-mail-tree-window))
        (setq romacs-mail-pair-window nil romacs-mail-pair-buffer nil romacs-mail-tree-window nil)
        (if (eq window anchor)
            (when (window-live-p window) (set-window-buffer window tree))
          (romacs-delete-paired-window window))
        (when (buffer-live-p buffer) (kill-buffer buffer))))))

(defun romacs-mail-quit ()
  "Return to the live parent without destroying unrelated buffers or windows."
  (interactive)
  (let ((buffer (current-buffer)) (parent romacs-mail-parent) (mode major-mode))
    (cond ((eq mode 'romacs-mail-message-mode)
           (when (buffer-live-p parent)
             (romacs-mail-close-pair parent)
             (let ((window (get-buffer-window parent)))
               (when window (select-window window)))
             (switch-to-buffer parent)))
          ((eq mode 'romacs-mail-tree-mode)
           (romacs-mail-close-pair buffer)
           (when (buffer-live-p parent) (switch-to-buffer parent))
           (kill-buffer buffer))
          ((buffer-live-p parent) (switch-to-buffer parent))
          (t (switch-to-buffer "*scratch*")))))

(defun romacs-mail-tree-current-query ()
  "Return from a message to its current tree and selected message identity."
  (interactive)
  (unless (and (eq major-mode 'romacs-mail-message-mode) (buffer-live-p romacs-mail-parent))
    (romacs-mail-error "No mail tree for this message"))
  (let ((parent romacs-mail-parent))
    (romacs-mail-close-pair parent)
    (let ((window (get-buffer-window parent))) (when window (select-window window)))
    (switch-to-buffer parent)))

(defun romacs-mail-compose-text (account headers envelope quote)
  "Populate the owned draft with HEADERS, ENVELOPE and QUOTE using ACCOUNT."
  (setq romacs-mail-account account romacs-mail-envelope envelope buffer-read-only nil)
  (let ((inhibit-read-only t))
    (erase-buffer)
    (dolist (name '("From" "To" "Cc" "Subject"))
      (when (or (member name '("From" "To" "Subject"))
                (and (romacs-mail-header name headers) (not (equal (romacs-mail-header name headers) ""))))
        (insert name ": " (romacs-mail-clean (romacs-mail-header name headers) t) "\n")))
    (insert "--text follows this line--\n" (or quote "")))
  (goto-char (point-max)) (set-buffer-modified-p t))

(defun romacs-mail-compose (account &optional to subject)
  "Compose plain text for ACCOUNT with optional TO and SUBJECT."
  (let ((parent (current-buffer)))
    (switch-to-buffer (get-buffer-create (generate-new-buffer-name "*unsent mail*")))
    (romacs-mail-initialize account 'romacs-mail-compose-mode "Mail Compose" romacs-mail-compose-map parent)
    (romacs-mail-compose-text account
                             (list (cons "From" (or (plist-get account :from)
                                                    (and (boundp 'user-mail-address) user-mail-address) ""))
                                   (cons "To" (or to "")) (cons "Subject" (or subject ""))) nil nil)))

(defun romacs-mail-reply (all)
  "Compose a reply to the exact current message, including all recipients when ALL."
  (let* ((key (if (eq major-mode 'romacs-mail-tree-mode)
                  (plist-get (romacs-mail-row) :key) romacs-mail-active-key))
         (account romacs-mail-account) (parent (current-buffer)))
    (unless (and key (memq major-mode '(romacs-mail-tree-mode romacs-mail-message-mode)))
      (romacs-mail-error "No message to reply to"))
    (romacs-mail-compose account)
    (setq buffer-read-only t)
    (romacs-mail-text "Preparing reply…\n")
    (setq romacs-mail-job
          (romacs-mail-request account :reply (list (cdr key) all)
                               (current-buffer) romacs-mail-owner major-mode
                               (lambda (envelope)
                                 (unless (equal key (plist-get envelope :source)) (error "Reply identity changed"))
                                 (let ((headers (plist-get envelope :headers)))
                                   (romacs-mail-compose-text
                                    account headers envelope
                                    (concat (romacs-mail-clean (plist-get envelope :quote-attribution)) "\n"
                                            "> " (mapconcat (lambda (c) (if (= c 10) "\n> " (char-to-string c)))
                                                              (romacs-mail-clean (plist-get envelope :quote-text)) "") "\n\n"))))))))

(defun romacs-mail-freeze (text envelope date message-id)
  "Return RFC text from TEXT and ENVELOPE using explicit DATE and MESSAGE-ID."
  (unless (string-match "\n--text follows this line--\n" text) (romacs-mail-error "Mail separator is missing"))
  (let ((headers (substring text 0 (match-beginning 0))) (body (substring text (match-end 0))))
    (dolist (name '("From" "To" "Subject"))
      (unless (string-match (concat "^" name ": [^\n\r]+$") headers)
        (romacs-mail-error (format "Missing or invalid %s header" name))))
    (when (string-match "\r" headers) (romacs-mail-error "Invalid carriage return in headers"))
    (dolist (name '("Date" "Message-ID" "In-Reply-To" "References" "User-Agent" "MIME-Version" "Content-Type" "Content-Transfer-Encoding"))
      (when (string-match (concat "^" name ":") headers) (romacs-mail-error (format "Reserved %s header" name))))
    (dolist (value (list (plist-get envelope :in-reply-to) (plist-get envelope :references)))
      (when (and value (string-match "[\000-\037\177]" value)) (romacs-mail-error "Invalid reply threading header")))
    (concat headers "\nDate: " date "\nMessage-ID: <" message-id ">"
            (if (plist-get envelope :in-reply-to) (concat "\nIn-Reply-To: " (plist-get envelope :in-reply-to)) "")
            (if (plist-get envelope :references) (concat "\nReferences: " (plist-get envelope :references)) "")
            "\nUser-Agent: Romacs notmuch 0.1\nMIME-Version: 1.0\nContent-Type: text/plain; charset=utf-8\nContent-Transfer-Encoding: 8bit\n\n" body)))

(defun romacs-mail-message ()
  "Freeze the current draft for one explicit submission."
  (romacs-mail-freeze (buffer-string) romacs-mail-envelope
                     (format-time-string "%a, %d %b %Y %T +0000" nil t)
                     (concat (make-temp-name "romacs-") "@localhost")))

(defun romacs-mail-settle-send (buffer owner output)
  "Settle submission ownership without rewriting a retired document."
  (when (and (buffer-live-p buffer) (with-current-buffer buffer (eq romacs-mail-owner owner)))
    (with-current-buffer buffer
      (setq romacs-mail-job nil)
      (if (not (romacs-mail-owned-p buffer owner 'romacs-mail-compose-mode))
          (progn (setq romacs-mail-send-state 'uncertain)
                 (message "Mail submission settled after its draft was retired; check transport receipts") nil)
        (if (and (eq (car output) 0) (not (nth 3 output)))
            (progn (setq romacs-mail-send-state 'sent mode-line-format '("%b" " | Mail sent"))
                   (set-buffer-modified-p nil) (message "Mail transport confirmed submission and Sent storage"))
          (setq romacs-mail-send-state 'uncertain buffer-read-only nil
                mode-line-format '("%b" " | Mail submission uncertain — draft retained"))
          (message "Mail submission failed or uncertain; draft retained. Check receipts before retrying"))
        (not (null (get-buffer-window buffer)))))))

(defun romacs-mail-send ()
  "Submit once after confirmation, retaining uncertain drafts without retry."
  (interactive)
  (unless (eq major-mode 'romacs-mail-compose-mode) (romacs-mail-error "Not a mail draft"))
  (unless (plist-get romacs-mail-account :send) (romacs-mail-error "Configure the account mail transport before sending"))
  (when romacs-mail-send-state (romacs-mail-error "This draft already has a submission; reconcile it before sending again"))
  (let ((text (romacs-mail-message)) (draft (buffer-string))
        (buffer (current-buffer)) (owner romacs-mail-owner) (transport (plist-get romacs-mail-account :send)))
    (when (equal (read-string "Submit this message? Type yes to send: ") "yes")
      (unless (and (romacs-mail-owned-p buffer owner 'romacs-mail-compose-mode) (equal draft (buffer-string)))
        (romacs-mail-error "The draft changed during confirmation"))
      (setq romacs-mail-job
            (romacs-process-start transport text default-directory 120
                                  (lambda (output) (romacs-mail-settle-send buffer owner output))))
      (setq romacs-mail-send-state 'sending buffer-read-only t mode-line-format '("%b" " | Sending mail…")))))

(defun romacs-mail-abort ()
  "Kill this draft when requested, without imposing a mail-specific veto."
  (interactive)
  (romacs-gui-kill-buffer))

(defun romacs-mail-new-search ()
  "Search again using the current mail account."
  (interactive)
  (romacs-mail-search romacs-mail-account (read-string "Mail search: ")))
(defun romacs-mail-new-compose ()
  "Compose a new message using the current mail account."
  (interactive)
  (romacs-mail-compose romacs-mail-account))
(defun romacs-mail-reply-sender () (interactive) (romacs-mail-reply nil))
(defun romacs-mail-reply-all () (interactive) (romacs-mail-reply t))

(defvar romacs-mail-search-map
  (let ((map (make-sparse-keymap)))
    (define-key map [110] 'romacs-mail-next) (define-key map [112] 'romacs-mail-previous)
    (define-key map [10] 'romacs-mail-open-tree) (define-key map [13] 'romacs-mail-open-tree)
    (define-key map [103] 'romacs-mail-refresh) (define-key map [113] 'romacs-mail-quit)
    (define-key map [115] 'romacs-mail-new-search) (define-key map [109] 'romacs-mail-new-compose) map))
(defvar romacs-mail-tree-map
  (let ((map (make-sparse-keymap)))
    (set-keymap-parent map romacs-mail-search-map)
    (define-key map [10] 'romacs-mail-show-message) (define-key map [13] 'romacs-mail-show-message)
    (define-key map [78] 'romacs-mail-next-all) (define-key map [80] 'romacs-mail-previous-all)
    (define-key map [114] 'romacs-mail-reply-sender) (define-key map [82] 'romacs-mail-reply-all) map))
(defvar romacs-mail-message-map
  (let ((map (make-sparse-keymap)))
    (define-key map [113] 'romacs-mail-quit) (define-key map [90] 'romacs-mail-tree-current-query)
    (define-key map [114] 'romacs-mail-reply-sender) (define-key map [82] 'romacs-mail-reply-all)
    (define-key map [115] 'romacs-mail-new-search) (define-key map [109] 'romacs-mail-new-compose) map))
(defvar romacs-mail-compose-map
  (let ((map (make-sparse-keymap)))
    (define-key map [3 3] 'romacs-mail-send) (define-key map [3 19] 'romacs-mail-send)
    (define-key map [3 11] 'romacs-mail-abort) map))

(provide 'romacs-mail)
;;; romacs-mail.el ends here
