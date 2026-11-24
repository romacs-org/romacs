;;; romacs-agent-compose-invariants-tests.el --- What a composer keeps, whatever is done -*- lexical-binding: t; -*-
;; SPDX-License-Identifier: GPL-3.0-or-later
;; Maintainer: Thanos Apollo <public@thanosapollo.org>

;;; Commentary:
;; A chat's composer, held to its invariants:
;; 1. its draft is never lost or changed silently while its chat lives;
;; 2. a draft being sent is not edited, and its acceptance takes away
;;    exactly the text sent;
;; 3. the focus comes back to it after the menu or a restore, unless the
;;    user moved it, and never while a popup is open;
;; 4. its height shows all it holds, the attachments' tray too;
;; 5. cleanup touches composer panes alone.

;;; Code:

(require 'ert)
(require 'cl-lib)
(require 'romacs-mcp)

(let ((romacs-mcp-call-function romacs-mcp-call-function)
      (romacs-mcp-tools (copy-alist romacs-mcp-tools)))
  (require 'romacs-agent)
  (require 'romacs-agent-claude)
  (require 'romacs-agent-log)
  (require 'romacs-agent-menu))
(require 'romacs-agent-test-attached)

(defmacro romacs-agent-compose-invariants-tests--with (&rest body)
  "Run BODY with a clean `session', its `chat' and its `composer'."
  (declare (indent 0))
  `(let* ((romacs-agent-log--chats (make-hash-table :test #'equal))
          (romacs-agent--sessions (make-hash-table :test #'equal))
          (session (romacs-agent--make :id "c0c0c0c0-0000-4000-8000-0000000000aa" :harness 'claude
                                       :state 'running :cwd default-directory))
          (chat (romacs-agent-log-chat session))
          (composer (romacs-agent-log-composer-buffer chat)))
     (unwind-protect
         (save-window-excursion ,@body)
       (when (buffer-live-p chat) (kill-buffer chat)))))

(defun romacs-agent-compose-invariants-tests--redisplay ()
  "Settle window change hooks, including in a batch frame."
  (when noninteractive (romacs--run-window-change-functions))
  (redisplay t))

(defun romacs-agent-compose-invariants-tests--key (key)
  "Dispatch KEY through Romacs's command loop."
  (let ((unread-command-events (mapcar (lambda (event) (cons t event))
                                      (listify-key-sequence (kbd key)))))
    (should-not (cdr (romacs--run-host-command)))))

(defun romacs-agent-compose-invariants-tests--alone (session)
  "Show SESSION's chat and composer as all the frame shows."
  (let ((ignore-window-parameters t)) (delete-other-windows))
  (romacs-agent-log-show session '((display-buffer-same-window)))
  (romacs-agent-compose-invariants-tests--redisplay))

(defmacro romacs-agent-compose-invariants-tests--held (&rest body)
  "Run BODY with `answer', a function giving the draft submitted the
answer of its STATUS, and `calls', the submits made."
  (declare (indent 0))
  `(let ((calls nil))
     (cl-letf (((symbol-function 'romacs-agent-submit)
                (lambda (_id intent callback) (push (cons intent callback) calls))))
       (cl-flet ((answer (status)
                   (pcase-let ((`(,intent . ,callback) (car calls)))
                     (funcall callback (append (list :draft-id (plist-get intent :draft-id) :status status)
                                               (and (eq status 'refused)
                                                    (list :reason 'owner-unavailable :message "Not now")))))))
         ,@body))))

;;; 1. A draft is never lost while its chat lives

(ert-deftest romacs-agent-compose-invariants-a-composer-outlives-a-kill-while-sending ()
  (romacs-agent-compose-invariants-tests--with
    (romacs-agent-compose-invariants-tests--held
      (with-current-buffer composer
        (insert "say this")
        (romacs-agent-log-send))
      (should-not (kill-buffer composer))
      (should (buffer-live-p composer))
      (answer 'refused)
      (should (equal (with-current-buffer composer (buffer-string)) "say this"))
      (should (eq (romacs-agent-log-composer-buffer chat) composer)))))

(ert-deftest romacs-agent-compose-invariants-a-queue-edit-never-joins-a-draft ()
  (romacs-agent-compose-invariants-tests--with
    (puthash (romacs-agent-id session) session romacs-agent--sessions)
    (romacs-agent-test-attach session t)
    (unwind-protect
        (let ((item (romacs-agent-item "queued message" :kind 'next :user t)))
          (romacs-agent--enqueue session item)
          (with-current-buffer chat (romacs-agent-log--separator t))
          (with-current-buffer composer (insert "unsent draft"))
          (should-not (kill-buffer composer))
          (with-current-buffer chat
            (goto-char (text-property-any romacs-agent-log--end (point-max) 'romacs-agent-log-queued item))
            (should-error (romacs-agent-log-queue-edit) :type 'user-error))
          (should (equal (with-current-buffer composer (buffer-string)) "unsent draft"))
          (should (equal (romacs-agent-item-text item) "queued message")))
      (delete-process (romacs-agent-connection session)))))

(ert-deftest romacs-agent-compose-invariants-c-x-k-offers-to-kill-the-chat ()
  (romacs-agent-compose-invariants-tests--with
    (with-current-buffer composer
      (should (eq (command-remapping 'kill-buffer) #'romacs-agent-compose-kill-chat))
      (cl-letf (((symbol-function 'yes-or-no-p) #'ignore))
        (call-interactively #'romacs-agent-compose-kill-chat)))
    (should (buffer-live-p chat))
    (should (buffer-live-p composer))
    (with-current-buffer composer
      (cl-letf (((symbol-function 'yes-or-no-p) #'always))
        (call-interactively #'romacs-agent-compose-kill-chat)))
    (should-not (buffer-live-p chat))
    (should-not (buffer-live-p composer))))

;;; 2. A draft being sent is not edited; acceptance takes the text sent

(ert-deftest romacs-agent-compose-invariants-read-only-mode-keeps-a-sending-draft ()
  (romacs-agent-compose-invariants-tests--with
    (romacs-agent-compose-invariants-tests--held
      (with-current-buffer composer
        (insert "send this")
        (romacs-agent-log-send)
        ;; Made writable, it still takes no edit, a second as the first.
        (call-interactively #'read-only-mode)
        (goto-char (point-max))
        (should-error (insert "x") :type 'buffer-read-only)
        (should-error (insert "x") :type 'buffer-read-only)
        (answer 'accepted)
        (should (equal (buffer-string) ""))))))

(ert-deftest romacs-agent-compose-invariants-view-mode-leaves-a-sending-draft-locked ()
  ;; In view-mode as it is sent, leaving it gives back the writable
  ;; buffer it found, without `read-only-mode-hook': still no edit.
  (romacs-agent-compose-invariants-tests--with
    (romacs-agent-compose-invariants-tests--held
      (with-current-buffer composer
        (insert "send this")
        (view-mode 1)
        (romacs-agent-log-send)
        (view-mode -1)
        (should-not buffer-read-only)
        (goto-char (point-max))
        (should-error (insert "x") :type 'buffer-read-only)
        (goto-char (point-min))
        (should-error (insert "x") :type 'buffer-read-only)
        (should-error (delete-char 1) :type 'buffer-read-only)
        (should (equal (buffer-string) "send this"))
        (answer 'accepted)
        (should (equal (buffer-string) ""))))))

(ert-deftest romacs-agent-compose-invariants-a-mode-change-keeps-the-guards ()
  ;; M-x text-mode while the draft is sent: it is still not killed, nor
  ;; edited, and the refusal leaves it as it was.
  (romacs-agent-compose-invariants-tests--with
    (romacs-agent-compose-invariants-tests--held
      (with-current-buffer composer
        (insert "say this")
        (romacs-agent-log-send)
        (text-mode)
        (goto-char (point-max))
        (should-error (insert "x") :type 'buffer-read-only))
      (should-not (kill-buffer composer))
      (should (buffer-live-p composer))
      (answer 'refused)
      (should (equal (with-current-buffer composer (buffer-string)) "say this"))
      (should (eq (romacs-agent-log-composer-buffer chat) composer)))))

(ert-deftest romacs-agent-compose-invariants-acceptance-takes-only-the-text-sent ()
  (romacs-agent-compose-invariants-tests--with
    (romacs-agent-compose-invariants-tests--held
      (with-current-buffer composer
        ;; Text after the one sent stays.
        (insert "send this")
        (romacs-agent-log-send)
        ;; Changed behind its guards' back, as no command can.
        (let ((inhibit-read-only t) (inhibit-modification-hooks t)) (goto-char (point-max)) (insert " and more"))
        (answer 'accepted)
        (should (equal (buffer-string) " and more"))
        ;; A draft that no longer starts with it stays as it is.
        (setq calls nil)
        (erase-buffer)
        (insert "send that")
        (romacs-agent-log-send)
        (let ((inhibit-read-only t) (inhibit-modification-hooks t)) (goto-char (point-min)) (insert "x"))
        (answer 'accepted)
        (should (equal (buffer-string) "xsend that"))))))

;;; 3. The focus comes back, and never under a popup

(ert-deftest romacs-agent-compose-invariants-a-second-popup-keeps-the-focus ()
  (romacs-agent-compose-invariants-tests--with
    (with-current-buffer chat (romacs-agent-log-draw session '(:type input :text "copied prompt")))
    (romacs-agent-compose-invariants-tests--alone session)
    (let ((keymap-popup-backend #'keymap-popup-backend-side-window) (kill-ring nil))
      (romacs-agent-compose-invariants-tests--key "C-c C-o")
      ;; The menu again, over the first: the focus stays in the chat.
      (romacs-agent-compose-invariants-tests--key "C-c C-o")
      (romacs-agent-compose-invariants-tests--redisplay)
      (should (eq (window-buffer) chat))
      (with-current-buffer chat (goto-char (point-min)))
      ;; Its row runs in the chat, then the composer has the focus.
      (romacs-agent-compose-invariants-tests--key "w")
      (should (string-search "copied prompt" (current-kill 0)))
      (romacs-agent-compose-invariants-tests--redisplay)
      (should (eq (window-buffer) composer)))))

(ert-deftest romacs-agent-compose-invariants-a-replaced-selected-slot-gives-focus-to-the-composer ()
  ;; As a restore can leave it: the composer's pane selected, showing
  ;; another buffer.
  (romacs-agent-compose-invariants-tests--with
    (romacs-agent-compose-invariants-tests--alone session)
    (let ((slot (get-buffer-window composer)))
      (select-window slot)
      (set-window-dedicated-p slot nil)
      (set-window-buffer slot (get-buffer-create "*scratch*")))
    (romacs-agent-compose-invariants-tests--redisplay)
    (should (eq (window-buffer) composer))
    (should (= (length (window-list)) 2))))

;;; 3. A restore gives the focus back to the composer that had it

(ert-deftest romacs-agent-compose-invariants-a-restore-selects-the-composer-it-saved ()
  (romacs-agent-compose-invariants-tests--with
    (romacs-agent-compose-invariants-tests--alone session)
    (let ((saved (with-current-buffer chat (funcall desktop-save-buffer default-directory))))
      ;; Saved with its composer selected: said so.
      (should (eq (plist-get saved :focus) t))
      (select-window (get-buffer-window chat))
      (should-not (plist-get (with-current-buffer chat (funcall desktop-save-buffer default-directory)) :focus))
      (kill-buffer chat)
      ;; Restored, the transcript shown and selected, as a restore leaves it.
      (cl-letf (((symbol-function 'romacs-agent-loaded) (lambda (_) session)))
        (setq chat (funcall (alist-get 'romacs-agent-log-mode desktop-buffer-mode-handlers) nil "chat" saved)))
      (let ((ignore-window-parameters t)) (delete-other-windows))
      (switch-to-buffer chat)
      (romacs-agent-compose-invariants-tests--redisplay)
      (should (eq (window-buffer) (romacs-agent-log-composer-buffer chat)))
      ;; Owed once: shown again later, the chat keeps the focus.
      (select-window (get-buffer-window chat))
      (romacs-agent-compose-invariants-tests--redisplay)
      (should (eq (window-buffer) chat)))))

;;; 4. The height shows the tray

(ert-deftest romacs-agent-compose-invariants-the-height-shows-the-tray ()
  (romacs-agent-compose-invariants-tests--with
    (romacs-agent-compose-invariants-tests--alone session)
    (let* ((coding-system-for-write 'binary)
           (png (base64-decode-string
                 "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mP8z8BQDwAEhQGAhKmMIQAAAABJRU5ErkJggg=="))
           (file (make-temp-file "compose-tray-" nil ".png" png))
           (window (get-buffer-window composer)))
      (unwind-protect
          (with-current-buffer composer
            (insert "one\ntwo\nthree " (romacs-agent-log-images-link file))
            (romacs-agent-log-attach-sync)
            (romacs-agent-compose-invariants-tests--redisplay)
            ;; The draft's rows measured as displayed, however its link's
            ;; path wraps: never a count fixed by a font or a directory.
            (let* ((tray (string-trim-right romacs-agent-log-attach--tray "\n"))
                   (tray-height (with-temp-buffer (insert tray) (cdr (buffer-text-pixel-size nil window))))
                   (text (cdr (window-text-pixel-size window (point-min) (point-max)))))
              (should (> tray-height 0))
              (should (>= text (* 3 (frame-char-height))))
              (should (= (window-body-height window t) (+ tray-height text)))))
        (delete-file file)))))

(ert-deftest romacs-agent-compose-invariants-word-wrapped-prose-gets-its-rows ()
  ;; Prose typed key by key wraps at word boundaries, before a multiple
  ;; of the width: after every key the composer shows all its rows.
  (romacs-agent-compose-invariants-tests--with
    (romacs-agent-compose-invariants-tests--alone session)
    (let* ((window (get-buffer-window composer))
           (words "the quick brown fox jumps over a lazy dog while agents write long replies ")
           (prose (substring (apply #'concat (make-list 20 words)) 0 1020))
           (short nil))
      (should (<= (window-body-width window) 80))
      (with-current-buffer composer
        (dolist (last-command-event (string-to-list prose))
          (self-insert-command 1)
          (unless (= (window-body-height window t) (romacs-agent-compose--height window))
            (push (current-column) short))))
      (should-not short))))

;;; 5. Cleanup touches composer panes alone

(ert-deftest romacs-agent-compose-invariants-an-unrelated-atom-pane-stays ()
  (romacs-agent-compose-invariants-tests--with
    (romacs-agent-compose-invariants-tests--alone session)
    (let* ((panel (get-buffer-create "*unrelated panel*"))
           (window (display-buffer-in-atom-window
                    panel `((window . ,(get-buffer-window chat)) (side . below)))))
      (unwind-protect
          (progn
            (should (window-live-p window))
            (romacs-agent-compose-invariants-tests--redisplay)
            (should (window-live-p window))
            (should (eq (window-buffer window) panel))
            (should (get-buffer-window composer)))
        (kill-buffer panel)))))

;;; Refit: only an edit that may change the rows measures

(ert-deftest romacs-agent-compose-invariants-a-plain-key-measures-nothing ()
  (romacs-agent-compose-invariants-tests--with
    (romacs-agent-compose-invariants-tests--alone session)
    (let ((measured 0) (measure (symbol-function 'window-text-pixel-size)))
      (cl-letf (((symbol-function 'window-text-pixel-size)
                 (lambda (&rest args) (cl-incf measured) (apply measure args))))
        (with-current-buffer composer
          (dolist (last-command-event (string-to-list "plain"))
            (self-insert-command 1))
          (delete-char -1)
          (should (= measured 0))
          (insert "\n")
          (should (> measured 0)))))))

(provide 'romacs-agent-compose-invariants-tests)
;;; romacs-agent-compose-invariants-tests.el ends here
