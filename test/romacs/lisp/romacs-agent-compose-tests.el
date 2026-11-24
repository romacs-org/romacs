;;; romacs-agent-compose-tests.el --- Separate agent composer -*- lexical-binding: t; -*-
;; SPDX-License-Identifier: GPL-3.0-or-later
;; Maintainer: Thanos Apollo <public@thanosapollo.org>

(require 'ert)
(require 'cl-lib)
(require 'romacs-mcp)

(let ((romacs-mcp-call-function romacs-mcp-call-function)
      (romacs-mcp-tools (copy-alist romacs-mcp-tools)))
  (require 'romacs-agent)
  (require 'romacs-agent-claude)
  (require 'romacs-agent-log))

(defmacro romacs-agent-compose-tests--with (&rest body)
  "Run BODY with a clean SESSION, CHAT and COMPOSER."
  (declare (indent 0))
  `(progn
     (should (fboundp 'romacs-agent-log-composer-buffer))
     (let* ((romacs-agent-log--chats (make-hash-table :test #'equal))
            (session (romacs-agent--make :id "compose-test" :harness 'claude :state 'running))
            (chat (romacs-agent-log-chat session))
            (composer nil))
       (unwind-protect
           (save-window-excursion
             (setq composer (romacs-agent-log-composer-buffer chat))
             (should (buffer-live-p composer))
             (should-not (eq chat composer))
             ,@body)
         (dolist (buffer (list chat composer))
           (when (buffer-live-p buffer) (kill-buffer buffer)))))))

(defun romacs-agent-compose-tests--redisplay ()
  "Settle window change hooks, including in a batch frame."
  (when noninteractive
    (romacs--run-window-change-functions))
  (redisplay t))

(defun romacs-agent-compose-tests--show (session)
  "Show SESSION in the selected window, with an unrelated window above."
  (delete-other-windows)
  (prog1 (split-window (selected-window) nil 'above)
    (romacs-agent-log-show session '((display-buffer-same-window)))
    (romacs-agent-compose-tests--redisplay)))

(defun romacs-agent-compose-tests--pair (chat composer)
  "Assert that CHAT and COMPOSER occupy a dedicated vertical window atom."
  (let ((top (get-buffer-window chat)) (bottom (get-buffer-window composer)))
    (should (window-live-p top))
    (should (window-live-p bottom))
    (should (window-dedicated-p bottom))
    (should (eq (window-parameter top 'window-atom) 'main))
    (should (eq (window-parameter bottom 'window-atom) 'below))
    (should (eq (window-atom-root top) (window-atom-root bottom)))
    (should (= (nth 3 (window-pixel-edges top)) (nth 1 (window-pixel-edges bottom))))))

(defun romacs-agent-compose-tests--key (key)
  "Dispatch KEY through Romacs's command loop."
  (let ((unread-command-events (mapcar (lambda (event) (cons t event))
                                      (listify-key-sequence (kbd key)))))
    (should-not (cdr (romacs--run-host-command)))))

(defun romacs-agent-compose-tests--rows (window rows)
  "Assert WINDOW has ROWS of body plus its divider."
  (let ((body (* rows (frame-char-height (window-frame window)))))
    (should (= (window-body-height window t) body))
    (should (= (window-pixel-height window) (+ body (window-bottom-divider-width window))))))

(ert-deftest romacs-agent-compose-atom-survives-delete-other-windows ()
  (romacs-agent-compose-tests--with
    (dolist (from (list chat composer))
      (save-window-excursion
        (let ((other (romacs-agent-compose-tests--show session)))
          (romacs-agent-compose-tests--pair chat composer)
          (should (eq (window-buffer) composer))
          (with-current-buffer composer
            (erase-buffer)
            (insert "one\ntwo\nthree\nfour"))
          (romacs-agent-compose-tests--redisplay)
          (romacs-agent-compose-tests--rows (get-buffer-window composer) 4)
          (select-window (get-buffer-window from))
          (romacs-agent-compose-tests--key "C-x 1")
          (romacs-agent-compose-tests--redisplay)
          (should-not (window-live-p other))
          (romacs-agent-compose-tests--pair chat composer)
          (romacs-agent-compose-tests--rows (get-buffer-window composer) 4))))))

(ert-deftest romacs-agent-compose-window-state-retains-the-atom ()
  (romacs-agent-compose-tests--with
    (let ((other (romacs-agent-compose-tests--show session)))
      (romacs-agent-compose-tests--pair chat composer)
      (should (eq (alist-get 'window-atom window-persistent-parameters) 'writable))
      (let ((state (window-state-get nil t)))
        (select-window other)
        (delete-other-windows)
        (window-state-put state)
        ;; Check the restored atom before redisplay could pair the buffers again.
        (romacs-agent-compose-tests--pair chat composer)
        (select-window (get-buffer-window composer))
        (romacs-agent-compose-tests--key "C-x 1")
        (should (= (length (window-list)) 2))))))

(ert-deftest romacs-agent-compose-fits-one-through-ten-body-rows ()
  (romacs-agent-compose-tests--with
    (should (boundp 'romacs-agent-compose-max-lines))
    (should (= romacs-agent-compose-max-lines 10))
    (delete-other-windows)
    (romacs-agent-log-show session '((display-buffer-same-window)))
    (let ((window (get-buffer-window composer)))
      (should (window-live-p window))
      (romacs-agent-compose-tests--redisplay)
      (romacs-agent-compose-tests--rows window 1)
      (with-current-buffer composer (insert "draft"))
      (dolist (rows (number-sequence 2 11))
        (with-current-buffer composer (insert "\ndraft"))
        (romacs-agent-compose-tests--redisplay)
        (romacs-agent-compose-tests--rows window (min rows 10)))
      (with-current-buffer composer
        (delete-region (+ (point-min) (length "draft")) (point-max)))
      (romacs-agent-compose-tests--redisplay)
      (romacs-agent-compose-tests--rows window 1))))

(ert-deftest romacs-agent-compose-killing-chat-kills-composer ()
  (romacs-agent-compose-tests--with
    (kill-buffer chat)
    (should-not (buffer-live-p composer))))

(ert-deftest romacs-agent-compose-orphan-cleanup-keeps-replacement-window ()
  (romacs-agent-compose-tests--with
    (let* ((other (romacs-agent-compose-tests--show session))
           (top (get-buffer-window chat))
           (bottom (get-buffer-window composer))
           (replacement (generate-new-buffer "*compose test replacement*")))
      (unwind-protect
          (progn
            (romacs-agent-compose-tests--pair chat composer)
            (select-window top)
            (switch-to-buffer replacement)
            (romacs-agent-compose-tests--redisplay)
            (should (window-live-p top))
            (should (eq (window-buffer top) replacement))
            (should (window-live-p other))
            (should-not (window-live-p bottom))
            (should-not (window-atom-root top))
            (should (buffer-live-p composer)))
        (kill-buffer replacement)))))

(ert-deftest romacs-agent-compose-typing-in-read-only-chat-inserts-in-composer ()
  (romacs-agent-compose-tests--with
    (romacs-agent-compose-tests--show session)
    (romacs-agent-log--on-event session '(:type delta :text "Reply"))
    (select-window (get-buffer-window chat))
    (goto-char (point-min))
    (should buffer-read-only)
    (let ((transcript (buffer-string)))
      (romacs-agent-compose-tests--key "x")
      (should (eq (window-buffer) composer))
      (should (equal (with-current-buffer composer (buffer-string)) "x"))
      (should (equal (with-current-buffer chat (buffer-string)) transcript)))))

(ert-deftest romacs-agent-compose-jump-toggles-the-selected-window ()
  (romacs-agent-compose-tests--with
    (romacs-agent-compose-tests--show session)
    (should (eq (window-buffer) composer))
    (romacs-agent-compose-tests--key "C-c C-j")
    (should (eq (window-buffer) chat))
    (romacs-agent-compose-tests--key "C-c C-j")
    (should (eq (window-buffer) composer))))

(ert-deftest romacs-agent-compose-return-submits-the-draft ()
  (romacs-agent-compose-tests--with
    (should (fboundp 'romacs-agent-submit))
    (romacs-agent-compose-tests--show session)
    (should (eq (window-buffer) composer))
    (insert "send this")
    (let (submitted)
      (cl-letf (((symbol-function 'romacs-agent-submit)
                 (lambda (id intent callback)
                   (push (list id intent callback) submitted))))
        (romacs-agent-compose-tests--key "RET"))
      (should (= (length submitted) 1))
      (pcase-let ((`((,id ,intent ,callback)) submitted))
        (should (equal id (romacs-agent-id session)))
        (should (memq (plist-get intent :kind) '(send steer)))
        (should (equal (plist-get intent :text) "send this"))
        (should (plist-get intent :draft-id))
        (should (equal (buffer-string) "send this"))
        (funcall callback (list :status 'accepted :draft-id (plist-get intent :draft-id)))
        (should (equal (buffer-string) ""))))))

(ert-deftest romacs-agent-compose-draft-is-the-whole-buffer ()
  (romacs-agent-compose-tests--with
    (should (fboundp 'romacs-agent-compose-mode))
    (should (fboundp 'romacs-agent-log--chat))
    (should (fboundp 'romacs-agent-log--composer))
    (should (string-prefix-p " *agent compose*" (buffer-name composer)))
    (dolist (buffer (list chat composer))
      (with-current-buffer buffer
        (should (eq (romacs-agent-log--chat) chat))
        (should (eq (romacs-agent-log--composer) composer))))
    (with-current-buffer composer
      (should (eq major-mode 'romacs-agent-compose-mode))
      (should (= (point) (point-min) (point-max)))
      (insert "draft\nsecond line")
      (should (equal (buffer-string) "draft\nsecond line"))
      (goto-char (point-min))
      (should (get-char-property (point) 'line-prefix)))
    (with-current-buffer chat
      (should (equal (romacs-agent-log--draft) "draft\nsecond line"))
      (should-not (string-search "second line" (buffer-string))))))

(ert-deftest romacs-agent-compose-desktop-round-trips-the-draft-through-chat ()
  (romacs-agent-compose-tests--with
    (with-current-buffer composer
      (should-not desktop-save-buffer)
      (insert "unsent\nβ"))
    (let ((saved (with-current-buffer chat (funcall desktop-save-buffer default-directory))))
      (should (equal saved '(:session "compose-test" :draft "unsent\nβ")))
      (kill-buffer chat)
      (cl-letf (((symbol-function 'romacs-agent-loaded)
                 (lambda (id) (and (equal id "compose-test") session))))
        (setq chat (funcall (alist-get 'romacs-agent-log-mode desktop-buffer-mode-handlers)
                            nil "old chat" saved)
              composer (romacs-agent-log-composer-buffer chat)))
      (should (buffer-live-p chat))
      (should (buffer-live-p composer))
      (should-not (get-buffer-window composer))
      (should (equal (with-current-buffer composer (buffer-string)) "unsent\nβ")))))

(ert-deftest romacs-agent-compose-attachment-tray-starts-at-point-min ()
  (romacs-agent-compose-tests--with
    (let* ((coding-system-for-write 'binary)
           (png (base64-decode-string
                 "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mP8z8BQDwAEhQGAhKmMIQAAAABJRU5ErkJggg=="))
           (file (make-temp-file "compose-attachment-" nil ".png" png)))
      (unwind-protect
          (with-current-buffer composer
            (insert "See " (romacs-agent-log-images-link file))
            (romacs-agent-log-attach-sync)
            (should (equal (mapcar (lambda (item) (plist-get item :path)) romacs-attach-items)
                           (list file)))
            (should (eq (overlay-buffer romacs-agent-log-attach--overlay) composer))
            (should (= (overlay-start romacs-agent-log-attach--overlay) (point-min)))
            (should (overlay-get romacs-agent-log-attach--overlay 'before-string)))
        (delete-file file)))))

(ert-deftest romacs-agent-compose-attach-plain-typing-neither-schedules-nor-scans ()
  (romacs-agent-compose-tests--with
    (with-current-buffer composer
      (should (memq 'romacs-agent-log-attach--changed after-change-functions))
      (cl-letf (((symbol-function 'run-at-time)
                 (lambda (&rest _) (ert-fail "Plain typing scheduled a timer")))
                ((symbol-function 'romacs-agent-log-images-links)
                 (lambda (&rest _) (ert-fail "Plain typing scanned the draft"))))
        (dolist (last-command-event (string-to-list "plain (draft) text"))
          (self-insert-command 1))
        (delete-char -1)
        (should (equal (buffer-string) "plain (draft) tex"))))))

(ert-deftest romacs-agent-compose-undo-preserves-streamed-transcript ()
  (romacs-agent-compose-tests--with
    (romacs-agent-log--on-event session '(:type delta :text "Reply"))
    (with-current-buffer composer
      (should-not (eq buffer-undo-list t))
      (insert "draft")
      (undo-boundary)
      (insert " tail")
      (undo-boundary)
      (let ((undo-list buffer-undo-list)
            (undo-entries (copy-tree buffer-undo-list)))
        (romacs-agent-log--on-event session '(:type delta :text " continues streaming"))
        (should (eq buffer-undo-list undo-list))
        (should (equal buffer-undo-list undo-entries))
        (should (equal (buffer-string) "draft tail")))
      (let ((last-command 'self-insert-command) (this-command 'undo))
        (call-interactively #'undo))
      (should (equal (buffer-string) "draft")))
    (with-current-buffer chat
      (should (equal (buffer-substring-no-properties (point-min) romacs-agent-log--end)
                     "Reply continues streaming\n")))))

(ert-deftest romacs-agent-compose-removes-the-interim-undo-shift ()
  (should-not (fboundp 'romacs-agent-log--apply-undo-shifts))
  (should-not (boundp 'romacs-agent-log--undo-shifts)))
