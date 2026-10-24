;;; romacs-notmuch.el --- Mail search, reading and composition -*- lexical-binding: t -*-

;; SPDX-License-Identifier: GPL-3.0-or-later

;;; Commentary:

;; Notmuch owns the index.  Lisp owns each view and draft; bounded background
;; commands own external I/O.  Reading never changes tags or fetches remote content.

;;; Code:

(defvar romacs-notmuch-command '("notmuch"))
(defvar romacs-notmuch-send-command nil
  "Argv of a mail transport accepting a complete UTF-8 message on standard input.
Exit zero must mean accepted and stored in Sent.  Failed/uncertain attempts are
never automatically retried.  Configure this explicitly for your account.")
(defvar romacs-notmuch-from nil)
(defvar romacs-notmuch-search-history nil)
(defvar romacs-notmuch-query nil)
(defvar romacs-notmuch-rows nil)
(defvar romacs-notmuch-messages nil)
(defvar romacs-notmuch-owner nil)
(defvar romacs-notmuch-job nil)
(defvar romacs-notmuch-send-state nil)
(defvar romacs-notmuch-return-buffer nil)
(defvar romacs-notmuch-reply-id nil)

(defface notmuch-search-date '((t (:foreground "#00b8ad"))) "Mail dates.")
(defface notmuch-search-subject '((t (:foreground "#d4c5a0"))) "Mail subjects.")
(defface notmuch-tag '((t (:foreground "#72b6e4"))) "Mail tags.")
(defface notmuch-message-summary '((t (:foreground "#b09bd2"))) "Mail headers.")

(defun romacs-notmuch-get (key object) (cdr (assoc key object)))

(defun romacs-notmuch-user-error (format-string &rest arguments)
  (signal 'user-error (list (apply 'format format-string arguments))))

(defun romacs-notmuch-prefix-p (prefix text)
  (and (>= (length text) (length prefix))
       (equal prefix (substring text 0 (length prefix)))))

(defun romacs-notmuch-clean (text &optional line)
  "Return inert display text, preserving newlines only outside LINE fields."
  (mapconcat (lambda (character)
               (if (or (and (>= character 127) (<= character 159))
                       (and (< character 32) (or line (/= character 10))))
                   " " (char-to-string character)))
             (or text "") ""))

(defun romacs-notmuch-pad (text width)
  (let ((text (romacs-notmuch-clean text t)))
    (concat (substring text 0 (min width (length text)))
            (make-string (max 0 (- width (length text))) 32))))

(defun romacs-notmuch-search-line (row)
  "Render ROW without deriving its identity from the displayed text."
  (concat (propertize (romacs-notmuch-pad (romacs-notmuch-get "date_relative" row) 13)
                      'face 'notmuch-search-date)
          " [" (number-to-string (or (romacs-notmuch-get "matched" row) 0)) "/"
          (number-to-string (or (romacs-notmuch-get "total" row) 0)) "] "
          (romacs-notmuch-pad (romacs-notmuch-get "authors" row) 24) " "
          (propertize (romacs-notmuch-clean (romacs-notmuch-get "subject" row) t)
                      'face 'notmuch-search-subject)
          " (" (propertize (mapconcat (lambda (tag) (romacs-notmuch-clean tag t))
                                     (romacs-notmuch-get "tags" row) " ") 'face 'notmuch-tag) ")\n"))

(defun romacs-notmuch-cancel ()
  (when (and romacs-notmuch-job (not (eq romacs-notmuch-send-state 'sending)))
    (ignore-errors (romacs-process-forget romacs-notmuch-job))
    (setq romacs-notmuch-job nil)))

(defun romacs-notmuch-query-kill ()
  (cond ((eq romacs-notmuch-send-state 'sending)
         (message "Mail submission is pending; keep this draft until it settles") nil)
        ((and (eq major-mode 'romacs-notmuch-compose-mode) (buffer-modified-p))
         (equal (read-string "Discard this mail draft? Type yes to discard: ") "yes"))
        (t t)))

(defun romacs-notmuch-quit-allowed-p ()
  "Protect pending submissions and unsaved mail on editor exit."
  (let ((pending nil) (dirty nil))
    (dolist (buffer (buffer-list))
      (with-current-buffer buffer
        (when (and (eq major-mode 'romacs-notmuch-compose-mode) (not buffer-file-name))
          (when (eq romacs-notmuch-send-state 'sending) (setq pending t))
          (when (buffer-modified-p) (setq dirty t)))))
    (cond (pending (message "Mail submission is pending; keep the editor open until it settles") nil)
          (dirty (equal (read-string "Discard unsaved mail drafts and continue quitting? Type yes: ") "yes"))
          (t t))))

(defun romacs-notmuch-initialize (mode label map)
  (dolist (name '(romacs-notmuch-query romacs-notmuch-rows romacs-notmuch-messages
                  romacs-notmuch-owner romacs-notmuch-job romacs-notmuch-send-state
                  romacs-notmuch-return-buffer romacs-notmuch-reply-id))
    (set (make-local-variable name) nil))
  (setq major-mode mode mode-name label buffer-file-name nil)
  (use-local-map map)
  (make-local-variable 'mode-line-format)
  (setq mode-line-format (list "%b" "  |  " label))
  (setq romacs-notmuch-owner (list mode))
  (make-local-variable 'mode-line-format)
  (set (make-local-variable 'kill-buffer-hook) '(romacs-notmuch-cancel))
  (set (make-local-variable 'kill-buffer-query-functions) '(romacs-notmuch-query-kill)))

(defun romacs-notmuch-owned-p (buffer owner mode)
  (and (buffer-live-p buffer)
       (with-current-buffer buffer
         (and (eq romacs-notmuch-owner owner) (eq major-mode mode) (not buffer-file-name)))))

(defun romacs-notmuch-text (text)
  (let ((inhibit-read-only t)) (widen) (erase-buffer) (insert text) (goto-char (point-min)))
  (set-buffer-modified-p nil))

(defun romacs-notmuch-result (output)
  (unless (and (eq (car output) 0) (not (nth 3 output)))
    (error "%s" (romacs-notmuch-clean (or (nth 3 output)
                                         (and (not (equal (nth 2 output) "")) (nth 2 output))
                                         "Mail command failed") t)))
  (romacs-json-read (nth 1 output)))

(defvar romacs-notmuch-search-map
  (let ((map (make-sparse-keymap)))
    (define-key map [110] 'romacs-notmuch-next)
    (define-key map [112] 'romacs-notmuch-previous)
    (define-key map [13] 'romacs-notmuch-open)
    (define-key map [10] 'romacs-notmuch-open)
    (define-key map [103] 'romacs-notmuch-refresh)
    (define-key map [115] 'notmuch-search)
    (define-key map [109] 'notmuch-mua-new-mail)
    (define-key map [113] 'romacs-notmuch-quit) map))

(defvar romacs-notmuch-show-map
  (let ((map (make-sparse-keymap)))
    (define-key map [110] 'romacs-notmuch-next)
    (define-key map [112] 'romacs-notmuch-previous)
    (define-key map [114] 'romacs-notmuch-reply)
    (define-key map [109] 'notmuch-mua-new-mail)
    (define-key map [115] 'notmuch-search)
    (define-key map [113] 'romacs-notmuch-quit) map))

(defun notmuch () "Search your mail without changing the index." (interactive)
  (notmuch-search "tag:inbox"))

(defun notmuch-search (query)
  "Search QUERY; show up to 50 newest threads.  Tags remain unchanged."
  (interactive (list (read-string "Notmuch search: " nil 'romacs-notmuch-search-history)))
  (let ((return (current-buffer)))
    (switch-to-buffer (get-buffer-create (generate-new-buffer-name "*notmuch-search*")))
    (romacs-notmuch-initialize 'romacs-notmuch-search-mode "Notmuch Search" romacs-notmuch-search-map)
    (setq romacs-notmuch-query query romacs-notmuch-return-buffer return buffer-read-only t)
    (romacs-notmuch-refresh)))

(defun romacs-notmuch-refresh ()
  "Refresh this search asynchronously, retaining buffer ownership."
  (interactive)
  (unless (eq major-mode 'romacs-notmuch-search-mode) (romacs-notmuch-user-error "Not a mail search"))
  (romacs-notmuch-cancel)
  (setq romacs-notmuch-owner (list romacs-notmuch-query))
  (let ((buffer (current-buffer)) (owner romacs-notmuch-owner))
    (setq romacs-notmuch-job
          (romacs-process-start
           (append romacs-notmuch-command
                   (list "search" "--format=json" "--limit=50" "--sort=newest-first" "--" romacs-notmuch-query))
           "" default-directory 30
           (lambda (output)
             (when (romacs-notmuch-owned-p buffer owner 'romacs-notmuch-search-mode)
               (with-current-buffer buffer
                 (setq romacs-notmuch-job nil)
                 (condition-case err
                     (let ((rows (romacs-notmuch-result output)) (positions nil))
                       (let ((inhibit-read-only t))
                         (widen) (erase-buffer)
                         (dolist (row rows)
                           (setq positions (cons (cons (point) (romacs-notmuch-get "thread" row)) positions))
                           (insert (romacs-notmuch-search-line row)))
                         (insert (if (= (length rows) 50) "Showing 50 threads; narrow the search for more.\n" "End of search results.\n")))
                       (setq romacs-notmuch-rows (nreverse positions))
                       (goto-char (point-min)) (set-buffer-modified-p nil))
                   (error (romacs-notmuch-text (format "Search failed: %s\n" (romacs-notmuch-clean (format "%s" err) t)))))
                 (not (null (get-buffer-window buffer))))))))
    (unless romacs-notmuch-rows (romacs-notmuch-text "Searching…\n"))))

(defun romacs-notmuch-next () (interactive) (forward-line 1))
(defun romacs-notmuch-previous () (interactive) (forward-line -1))
(defun romacs-notmuch-quit () (interactive)
  (if (buffer-live-p romacs-notmuch-return-buffer)
      (switch-to-buffer romacs-notmuch-return-buffer)
    (switch-to-buffer "*scratch*")))

(defun romacs-notmuch-collect (node &optional depth)
  "Flatten Notmuch's thread tuples, not message body parts."
  (when (> (or depth 0) 64) (error "Mail thread nesting is too deep"))
  (cond ((and (listp node) (assoc "id" node)) (list node))
        ((listp node)
         (let ((messages nil))
           (dolist (child node)
             (setq messages (append messages (romacs-notmuch-collect child (1+ (or depth 0))))))
           messages))))

(defun romacs-notmuch-body (parts &optional depth)
  "Render inert plain MIME parts.  HTML, attachments and encrypted parts stay inert."
  (when (> (or depth 0) 32) (error "MIME nesting is too deep"))
  (mapconcat
   (lambda (part)
     (let ((type (romacs-notmuch-get "content-type" part)) (content (romacs-notmuch-get "content" part)))
       (cond ((equal (romacs-notmuch-get "content-disposition" part) "attachment")
              (concat "[Attachment: " (romacs-notmuch-clean (romacs-notmuch-get "filename" part) t) "]\n"))
             ((equal type "text/plain") (romacs-notmuch-clean content))
             ((and (stringp type) (romacs-notmuch-prefix-p "multipart/" type) (listp content))
              (romacs-notmuch-body content (1+ (or depth 0))))
             (t (concat "[" (romacs-notmuch-clean type t) " not displayed]\n"))))) parts "\n"))

(defun romacs-notmuch-message-text (mail)
  (let ((headers (romacs-notmuch-get "headers" mail)))
    (concat (propertize (romacs-notmuch-clean (romacs-notmuch-get "From" headers) t) 'face 'notmuch-message-summary)
            " (" (propertize (mapconcat (lambda (tag) (romacs-notmuch-clean tag t)) (romacs-notmuch-get "tags" mail) " ") 'face 'notmuch-tag) ")\n"
            "Subject: " (romacs-notmuch-clean (romacs-notmuch-get "Subject" headers) t) "\n"
            "To: " (romacs-notmuch-clean (romacs-notmuch-get "To" headers) t) "\n"
            "Date: " (romacs-notmuch-clean (romacs-notmuch-get "Date" headers) t) "\n\n"
            (romacs-notmuch-body (romacs-notmuch-get "body" mail)) "\n\n")))

(defun romacs-notmuch-open ()
  "Open the exact thread stored at this row, without marking messages read."
  (interactive)
  (let ((thread (cdr (assoc (line-beginning-position) romacs-notmuch-rows)))
        (return (current-buffer)))
    (unless thread (romacs-notmuch-user-error "No thread on this row"))
    (switch-to-buffer (get-buffer-create (generate-new-buffer-name "*notmuch-show*")))
    (romacs-notmuch-initialize 'romacs-notmuch-show-mode "Notmuch Show" romacs-notmuch-show-map)
    (setq buffer-read-only t romacs-notmuch-return-buffer return)
    (romacs-notmuch-text "Loading message…\n")
    (let ((buffer (current-buffer)) (owner romacs-notmuch-owner))
      (setq romacs-notmuch-job
            (romacs-process-start
             (append romacs-notmuch-command (list "show" "--format=json" "--body=true" "--decrypt=false" "--" (concat "thread:" thread)))
             "" default-directory 30
             (lambda (output)
               (when (romacs-notmuch-owned-p buffer owner 'romacs-notmuch-show-mode)
                 (with-current-buffer buffer
                   (setq romacs-notmuch-job nil)
                   (condition-case err
                       (progn (setq romacs-notmuch-messages (romacs-notmuch-collect (romacs-notmuch-result output)))
                              (romacs-notmuch-text (mapconcat 'romacs-notmuch-message-text romacs-notmuch-messages "")))
                     (error (romacs-notmuch-text (format "Message failed: %s\n" (romacs-notmuch-clean (format "%s" err) t)))))
                   (not (null (get-buffer-window buffer)))))))))))

(defvar romacs-notmuch-compose-map
  (let ((map (make-sparse-keymap)))
    (define-key map [3 3] 'romacs-notmuch-send)
    (define-key map [3 19] 'romacs-notmuch-send)
    (define-key map [3 11] 'romacs-notmuch-abort) map))

(defun notmuch-mua-new-mail (&optional to subject)
  "Compose editable plain-text mail.  C-c C-c submits; C-c C-k aborts."
  (interactive)
  (switch-to-buffer (get-buffer-create (generate-new-buffer-name "*unsent mail*")))
  (romacs-notmuch-initialize 'romacs-notmuch-compose-mode "Notmuch Compose" romacs-notmuch-compose-map)
  (setq buffer-read-only nil)
  (insert "From: " (or romacs-notmuch-from (and (boundp 'user-mail-address) user-mail-address) "")
          "\nTo: " (romacs-notmuch-clean to t) "\nSubject: " (romacs-notmuch-clean subject t)
          "\n--text follows this line--\n")
  (set-buffer-modified-p t))

(defun romacs-notmuch-reply ()
  "Compose a plain reply to the first displayed message."
  (interactive)
  (let* ((mail (car romacs-notmuch-messages)) (headers (romacs-notmuch-get "headers" mail))
         (subject (or (romacs-notmuch-get "Subject" headers) ""))
         (body (romacs-notmuch-body (romacs-notmuch-get "body" mail)))
         (id (romacs-notmuch-get "id" mail)))
    (unless mail (romacs-notmuch-user-error "No message to reply to"))
    (notmuch-mua-new-mail (or (romacs-notmuch-get "Reply-To" headers) (romacs-notmuch-get "From" headers))
                          (if (romacs-notmuch-prefix-p "Re:" subject) subject (concat "Re: " subject)))
    (setq romacs-notmuch-reply-id id)
    (insert "\n" (concat "> " (mapconcat (lambda (c) (if (= c 10) "\n> " (char-to-string c))) body "")) "\n")))

(defun romacs-notmuch-message ()
  "Validate this draft and return its frozen RFC822 submission text."
  (let ((text (buffer-string)))
    (unless (string-match "\n--text follows this line--\n" text) (romacs-notmuch-user-error "Mail separator is missing"))
    (let ((headers (substring text 0 (match-beginning 0))) (body (substring text (match-end 0))))
      (dolist (name '("From" "To" "Subject"))
        (unless (string-match (concat "^" name ": [^\n\r]+$") headers)
          (romacs-notmuch-user-error "Missing or invalid %s header" name)))
      (when (string-match "\r" headers) (romacs-notmuch-user-error "Invalid carriage return in headers"))
      (when (and romacs-notmuch-reply-id
                 (string-match "[<>\000-\040\177]" romacs-notmuch-reply-id))
        (romacs-notmuch-user-error "Invalid reply message identity"))
      (concat headers
              "\nDate: " (format-time-string "%a, %d %b %Y %T +0000" nil t)
              "\nMessage-ID: <" (make-temp-name "romacs-") "@localhost>"
              (if romacs-notmuch-reply-id (concat "\nIn-Reply-To: <" romacs-notmuch-reply-id ">") "")
              "\nUser-Agent: Romacs notmuch 0.1\nMIME-Version: 1.0\nContent-Type: text/plain; charset=utf-8\nContent-Transfer-Encoding: 8bit\n\n" body))))

(defun romacs-notmuch-settle-send (buffer owner output)
  "Settle submission ownership independently from presentation ownership."
  (when (and (buffer-live-p buffer)
             (with-current-buffer buffer (eq romacs-notmuch-owner owner)))
    (with-current-buffer buffer
      (setq romacs-notmuch-job nil)
      (if (not (romacs-notmuch-owned-p buffer owner 'romacs-notmuch-compose-mode))
          (progn
            (setq romacs-notmuch-send-state 'uncertain)
            (message "Mail submission settled after its draft was retired; check transport receipts")
            nil)
        (if (and (eq (car output) 0) (not (nth 3 output)))
            (progn (setq romacs-notmuch-send-state 'sent mode-line-format '("%b" " | Mail sent"))
                   (set-buffer-modified-p nil) (message "Mail transport confirmed submission and Sent storage"))
          (setq romacs-notmuch-send-state 'uncertain buffer-read-only nil
                mode-line-format '("%b" " | Mail submission uncertain — draft retained"))
          (message "Mail submission failed or uncertain; draft retained. Check transport receipts before retrying"))
        (not (null (get-buffer-window buffer)))))))

(defun romacs-notmuch-send ()
  "Submit once through the configured transport, preserving uncertain drafts."
  (interactive)
  (unless (eq major-mode 'romacs-notmuch-compose-mode) (romacs-notmuch-user-error "Not a mail draft"))
  (unless romacs-notmuch-send-command (romacs-notmuch-user-error "Set romacs-notmuch-send-command before sending"))
  (when romacs-notmuch-send-state (romacs-notmuch-user-error "This draft already has a submission; reconcile it before sending again"))
  (let ((text (romacs-notmuch-message)) (buffer (current-buffer)) (owner romacs-notmuch-owner))
    (when (equal (read-string "Submit this message? Type yes to send: ") "yes")
      (setq romacs-notmuch-job
            (romacs-process-start
             romacs-notmuch-send-command text default-directory 120
             (lambda (output) (romacs-notmuch-settle-send buffer owner output))))
      (setq romacs-notmuch-send-state 'sending buffer-read-only t
            mode-line-format '("%b" " | Sending mail…")))))

(defun romacs-notmuch-abort ()
  "Retain a pending submission; otherwise use the ordinary dirty-buffer consent."
  (interactive)
  (if (eq romacs-notmuch-send-state 'sending)
      (romacs-notmuch-user-error "Mail is being submitted; wait for the transport result")
    (romacs-gui-kill-buffer)))

(provide 'romacs-notmuch)
;;; romacs-notmuch.el ends here
