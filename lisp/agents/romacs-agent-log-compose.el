;;; romacs-agent-log-compose.el --- A chat's composer, a buffer below it  -*- lexical-binding: t -*-
;; SPDX-License-Identifier: GPL-3.0-or-later
;; Maintainer: Thanos Apollo <public@thanosapollo.org>

;;; Commentary:
;; A chat's composer is a buffer of its own, paired with the chat for its
;; life: its whole text is the draft, the attachments' tray and the
;; prompt's mark one overlay at its start.  The transcript's writes
;; never reach it, so its undo is its own.  It is made on first need (a
;; chat shown, a draft restored, a queued message to edit) and goes with
;; its chat, never alone: while its chat lives it refuses to be killed.
;;
;; Shown, it is a dedicated window below its chat's, the two one window
;; atom (`display-buffer-in-atom-window'), so `delete-other-windows' keeps
;; the pair.  Its height is its tray's and its draft's rows, wrapped
;; ones too, 1 to `romacs-agent-compose-max-lines': fitted again on an
;; edit that may change them (a line break, a line crossing a window's
;; width), a tray change or a resize, never on another plain key.  A
;; window comes to show a chat: its composer goes below it.  The pane the
;; composer was put in carries `romacs-agent-composer'; one that no
;; longer holds the composer of the chat above it goes, and a composer
;; shown outside one shows its chat there instead.
;;
;; RET submits the draft (`romacs-agent-submit') under one id until it is
;; accepted: read-only meanwhile, its prompt in the sending face; then
;; emptied once accepted, else kept, the refusal said.

;;; Code:

(require 'cl-lib)
(require 'subr-x)
(require 'romacs-agent-log)
(require 'romacs-agent-log-attach)

(declare-function romacs-agent-log-queue-put-back "romacs-agent-log-queue" ())
(declare-function romacs-agent-chat-attach "romacs-agent-menu" (&optional ask))

(defcustom romacs-agent-compose-max-lines 10
  "The most rows a chat's composer grows to; a longer draft scrolls."
  :type 'natnum :group 'romacs-agent)

;;; State, the composer's

(defvar-local romacs-agent-log--draft-id nil
  "The id the composer's draft is submitted under (`romacs-agent-submit'):
made on its first send, kept until it is accepted, so a retry is the same draft.")
(defvar-local romacs-agent-log--sending nil
  "The draft id of the draft being sent, until its answer comes; nil when none is.")
(defvar-local romacs-agent-log--recalled nil
  "The cells of the chat's `romacs-agent-log--prompts' M-p went through,
the one the composer shows first; nil when it shows the draft.")
(defvar-local romacs-agent-log--draft-kept nil
  "The composer's text as M-p first found it, given back past the newest.")
(defvar-local romacs-agent-log--sent-text nil
  "The composer's whole text when its draft was sent: what an acceptance
takes away, while the draft still starts with it.")
(defvar-local romacs-agent-compose--start nil
  "The overlay at the draft's start: the attachments' tray, then the
prompt's mark, one string, as the display measures only one overlay
string at a place.")
(defvar-local romacs-agent-compose--guard nil
  "The overlay over the whole draft whose modification hooks refuse any
edit while it is sent (`romacs-agent-compose--guard-edit').")
(defvar romacs-agent-compose--chat-going nil
  "Non-nil while a chat is killed: its composer goes with it.")
(defvar romacs-agent-compose--focus-owed nil
  "A chat restored with its composer selected: its composer is selected
again once the restore's windows show it (`romacs-agent-compose--windows').")

;; A mode change in a composer (M-x text-mode) keeps what guards its
;; draft: whose it is, whether and what it sends, and the hooks that
;; refuse its kill and its edits, and fit its rows.
(dolist (variable '(romacs-agent-log--chat romacs-agent-log--draft-id romacs-agent-log--sending
                    romacs-agent-log--sent-text romacs-agent-compose--start romacs-agent-compose--guard))
  (put variable 'permanent-local t))
(dolist (function '(romacs-agent-compose--may-kill romacs-agent-compose--killed
                    romacs-agent-compose--before-change romacs-agent-compose--after-change
                    romacs-agent-compose--resized))
  (put function 'permanent-local-hook t))

(defvar romacs-agent-log-draft-replaced-hook nil
  "Run in a composer emptied, sent or replaced by a recalled prompt: what
was pending on its draft lands nowhere.")

;;; The buffer

(defun romacs-agent-log-composer-buffer (chat)
  "CHAT's composer, made when it has none."
  (with-current-buffer chat
    (unless (buffer-live-p romacs-agent-log--composer)
      (let ((composer (generate-new-buffer " *agent compose*")))
        (with-current-buffer composer
          (romacs-agent-compose-mode)
          (setq romacs-agent-log--chat chat)
          ;; A buffer whose name starts with a space keeps no undo.
          (buffer-enable-undo))
        (setq romacs-agent-log--composer composer)
        (add-hook 'kill-buffer-hook #'romacs-agent-log--kill-composer nil t)))
    romacs-agent-log--composer))

(defun romacs-agent-log--kill-composer ()
  "The chat goes, and its composer with it."
  (when (buffer-live-p romacs-agent-log--composer)
    (let ((romacs-agent-compose--chat-going t))
      (kill-buffer romacs-agent-log--composer))))

(defun romacs-agent-compose--may-kill ()
  "Whether the composer may be killed: only with its chat, so its draft
is never lost while the chat lives (`kill-buffer-query-functions')."
  (or romacs-agent-compose--chat-going (not (buffer-live-p romacs-agent-log--chat))
      (progn (message "Kill the chat to close its composer") nil)))

(defun romacs-agent-compose-kill-chat ()
  "Kill the composer's chat, and the composer with it, once confirmed: the
composer alone is never killed."
  (interactive nil romacs-agent-compose-mode)
  (let ((chat romacs-agent-log--chat))
    (when (yes-or-no-p (format "Kill the chat %s, and its composer? " (buffer-name chat)))
      (kill-buffer chat))))

(defun romacs-agent-compose--guard-edit (_overlay after _beg _end &optional _length)
  "Refuse any edit of the draft while it is sent, whatever made the
composer writable (C-x C-q, `view-mode' leaving it, a mode change): the
modification hooks of the overlay over the whole draft.  An overlay's,
not `before-change-functions', which an error there empties."
  (when (and romacs-agent-log--sending (not after))
    (message "The draft is being sent: the composer stays read-only until its answer")
    (signal 'buffer-read-only (list (current-buffer)))))

(defun romacs-agent-compose--killed ()
  "The composer goes, with its chat: each window showing it goes alone."
  (dolist (window (get-buffer-window-list (current-buffer) nil t))
    (romacs-agent-compose--dissolve window)))

(defun romacs-agent-log--draft ()
  "The current chat's or composer's draft, trimmed, all of it however the
composer is narrowed; empty when it has none."
  (if-let* ((composer (romacs-agent-log--composer)))
      (with-current-buffer composer
        (save-restriction (widen) (string-trim (buffer-substring-no-properties (point-min) (point-max)))))
    ""))

(defun romacs-agent-log--sending-p ()
  "Whether the current chat's or composer's draft is being sent."
  (when-let* ((composer (romacs-agent-log--composer)))
    (buffer-local-value 'romacs-agent-log--sending composer)))

(defun romacs-agent-log--editing ()
  "The queued message the current chat or composer holds to edit, or nil."
  (when-let* ((chat (romacs-agent-log--chat)))
    (buffer-local-value 'romacs-agent-log-queue--editing chat)))

(defun romacs-agent-log--saved-draft ()
  "The chat's draft as it is saved with it (desktop.el, warm-restart.el).
A queued message held to edit is none: it waits where it was, as it was,
for the next session; gone from the queue meanwhile, the composer's text
is all there is of it, a draft."
  (if (and romacs-agent-log-queue--editing
           (memq romacs-agent-log-queue--editing
                 (romacs-agent-log-source-queue romacs-agent-log--source romacs-agent-log--session)))
      ""
    (if-let* ((composer (romacs-agent-log--composer)))
        (with-current-buffer composer
          (save-restriction (widen) (buffer-substring-no-properties (point-min) (point-max))))
      "")))

(defun romacs-agent-log--composer-selected-p ()
  "Whether the current chat's composer is in the selected window: what a
saved chat says, so its restore selects it again."
  (when-let* ((composer (romacs-agent-log--composer)))
    (eq (window-buffer (selected-window)) composer)))

(defun romacs-agent-log--owe-focus (chat)
  "CHAT, restored, had its composer selected: selected again once the
restore's windows show CHAT (`romacs-agent-compose--windows')."
  (setq romacs-agent-compose--focus-owed chat))

(defun romacs-agent-log--restore-draft (chat draft)
  "Make CHAT's composer, CHAT restored, before its windows are: DRAFT at its
end, unless it sends one already, the one it sends."
  (with-current-buffer (romacs-agent-log-composer-buffer chat)
    (unless romacs-agent-log--sending
      (save-excursion (goto-char (point-max)) (insert draft)))))

(defun romacs-agent-log--composer-handoff ()
  "The chat goes on with another session: its composer keeps the draft,
a recalled prompt given back for it, and forgets what M-p went through."
  (when-let* ((composer (romacs-agent-log--composer)))
    (with-current-buffer composer
      (when romacs-agent-log--recalled (romacs-agent-log--recall nil))
      (setq romacs-agent-log--recalled nil romacs-agent-log--draft-kept nil))))

;;; The prompt

(defun romacs-agent-compose--start-show ()
  "Show at the draft's start the tray's chips, then the prompt's mark,
once, before the first line, as comint and the minibuffer show theirs:
in the sending face while the draft is sent."
  (overlay-put romacs-agent-compose--start 'before-string
               (concat romacs-agent-log-attach--tray
                       (propertize (concat (romacs-agent-glyph 'composer) " ")
                                   'face (if romacs-agent-log--sending 'romacs-agent-compose-sending
                                           'romacs-agent-log-prompt)))))

(defun romacs-agent-compose--marks-changed ()
  "A mark of `romacs-agent-glyph-alist' changed: every composer's prompt
is marked again."
  (dolist (buffer (buffer-list))
    (when (eq (buffer-local-value 'major-mode buffer) 'romacs-agent-compose-mode)
      (with-current-buffer buffer (romacs-agent-compose--start-show)))))

(add-hook 'romacs-agent-glyph-alist-changed-hook #'romacs-agent-compose--marks-changed)

;;; Its windows

(defun romacs-agent-compose--above (window)
  "The window above WINDOW in their atom, WINDOW a composer's; else nil."
  (let ((above (window-prev-sibling window)))
    (and above (eq (window-parameter window 'window-atom) 'below)
         (eq (window-parameter above 'window-atom) 'main)
         above)))

(defun romacs-agent-compose--below (window composer)
  "COMPOSER's window below WINDOW in their atom, or nil."
  (let ((below (window-next-sibling window)))
    (and below (eq (window-parameter window 'window-atom) 'main)
         (eq (window-parameter below 'window-atom) 'below)
         (eq (window-buffer below) composer)
         below)))

(defun romacs-agent-compose--tray-height (window)
  "The pixels the attachments' tray takes above the draft in WINDOW: none
without one.  A measure of the draft leaves the overlay's string out."
  (let ((tray (string-trim-right (buffer-local-value 'romacs-agent-log-attach--tray (window-buffer window)) "\n")))
    (if (string-empty-p tray) 0
      (with-temp-buffer
        (insert tray)
        (cdr (buffer-text-pixel-size nil window))))))

(defun romacs-agent-compose--height (window)
  "The pixels WINDOW, a composer's, needs for its tray and its draft: the
tray's, and the draft's rows, one to `romacs-agent-compose-max-lines'.
A line break ending the draft starts a row of its own, where the cursor
goes."
  (let* ((line (frame-char-height (window-frame window)))
         (most (* line romacs-agent-compose-max-lines)))
    (with-current-buffer (window-buffer window)
      ;; Measured to before a last line break, whether a measure to the
      ;; end counts the empty row after it or not, and no further than
      ;; the most it may show.
      (let* ((end (if (eq (char-before (point-max)) ?\n) (1- (point-max)) (point-max)))
             (text (+ (if (= (point-min) end) line
                        (cdr (window-text-pixel-size window (point-min) end nil (1+ most))))
                      (if (< end (point-max)) line 0))))
        (+ (romacs-agent-compose--tray-height window) (max line (min text most)))))))

(defun romacs-agent-compose--fit (window)
  "Size WINDOW, a composer's, to its draft's rows and its divider, and
keep that height whatever its neighbours do."
  (let ((delta (- (+ (romacs-agent-compose--height window) (window-bottom-divider-width window))
                  (window-pixel-height window))))
    (window-preserve-size window nil nil)
    ;; By pixels, not rounded to lines; `safe': one row is all a composer
    ;; needs, whatever `window-min-height'.
    (let ((window-resize-pixelwise t))
      (when (and (/= delta 0) (window-resizable-p window delta nil 'safe t))
        (window-resize window delta nil 'safe t)))
    (window-preserve-size window nil t)))

(defun romacs-agent-compose--refit ()
  "Fit each window showing this composer to its draft."
  (dolist (window (get-buffer-window-list (current-buffer) nil t))
    (romacs-agent-compose--fit window)))

(defun romacs-agent-compose--resized (window)
  "WINDOW, a composer's, was resized: fitted to its draft again, which its
new width may wrap anew."
  (romacs-agent-compose--fit window))

(defun romacs-agent-compose--delete-other-windows (window)
  "Keep WINDOW's pair, a chat and its composer, alone in its frame: the
`delete-other-windows' parameter of both, so the pair filling its frame
already is nothing to do, not an error."
  (let ((root (window-atom-root window)))
    (cond ((null root)
           (set-window-parameter window 'delete-other-windows nil)
           (delete-other-windows window))
          ((not (eq root (frame-root-window window)))
           (delete-other-windows root)))))

(defun romacs-agent-compose--dissolve (window)
  "Undo the atom WINDOW, a composer's, makes with its chat's above it:
deleting WINDOW then deletes it alone."
  (let ((above (romacs-agent-compose--above window))
        (root (and (eq (window-parameter window 'window-atom) 'below) (window-parent window))))
    (set-window-parameter window 'romacs-agent-composer nil)
    (dolist (member (list window above root))
      (when (and member (window-valid-p member))
        (set-window-parameter member 'window-atom nil)
        (set-window-parameter member 'delete-other-windows nil)))))

(defun romacs-agent-log--pair (window)
  "The window of the composer below WINDOW, which shows a chat: made when
there is none, nil when there is no room for it."
  (let* ((composer (romacs-agent-log-composer-buffer (window-buffer window)))
         (kept (romacs-agent-compose--below window composer))
         (below (or kept
                    (and (not (window-parameter window 'window-side))
                         (display-buffer-in-atom-window
                          composer `((window . ,window) (side . below) (dedicated . t)))))))
    (when below
      ;; The composer's slot, whatever it comes to show (a bury, a
      ;; restore): no other pane below a chat is one.
      (set-window-parameter below 'romacs-agent-composer t)
      (dolist (member (list window below))
        (set-window-parameter member 'delete-other-windows #'romacs-agent-compose--delete-other-windows))
      (unless kept (romacs-agent-compose--fit below))
      below)))

(defun romacs-agent-compose--composer-p (buffer)
  "Whether BUFFER is a chat's composer."
  (eq (buffer-local-value 'major-mode buffer) 'romacs-agent-compose-mode))

(defun romacs-agent-compose--slot-p (window)
  "Whether WINDOW is a composer's slot, as `romacs-agent-log--pair' made
it, whatever it shows now: no other pane, below a chat or not."
  (window-parameter window 'romacs-agent-composer))

(defun romacs-agent-compose--orphan-p (window)
  "Whether WINDOW is a composer's slot that no longer holds the composer
of the chat shown above it."
  (and (romacs-agent-compose--slot-p window)
       (let ((buffer (window-buffer window)) (above (romacs-agent-compose--above window)))
         (not (and above (romacs-agent-compose--composer-p buffer)
                   (eq (window-buffer above) (buffer-local-value 'romacs-agent-log--chat buffer)))))))

(defun romacs-agent-compose--stray-p (window)
  "Whether WINDOW shows a composer outside any slot, as a split of one
leaves it."
  (and (not (romacs-agent-compose--slot-p window))
       (romacs-agent-compose--composer-p (window-buffer window))))

(defun romacs-agent-compose--show-chat (window)
  "Show in WINDOW, a stray composer's, its chat: a pair of its own then."
  (let ((chat (buffer-local-value 'romacs-agent-log--chat (window-buffer window))))
    (set-window-dedicated-p window nil)
    (set-window-buffer window chat)))

(defun romacs-agent-compose--let-go (window)
  "Delete WINDOW, a composer's slot that holds no longer the composer of
the chat above it, alone: the window above keeps what it shows.  Return
that window when WINDOW had the focus: it is owed its composer's."
  (let ((above (window-prev-sibling window)))
    (romacs-agent-compose--dissolve window)
    (when (window-deletable-p window)
      (prog1 (and (eq window (selected-window)) (window-live-p above)
                  (progn (select-window above) above))
        (delete-window window)))))

(defun romacs-agent-compose--windows (frame)
  "FRAME's windows changed what they show: a window showing a chat gets
its composer below it, and a composer's slot that no longer holds the
composer of the chat above it goes, the focus it had to the composer
that takes its place (a restore, a bury)."
  (let ((windows (window-list frame 'nomini)) (owed nil))
    (dolist (window windows)
      (when (window-live-p window)
        (cond ((romacs-agent-compose--orphan-p window)
               (when-let* ((above (romacs-agent-compose--let-go window))) (push above owed)))
              ((romacs-agent-compose--stray-p window) (romacs-agent-compose--show-chat window)))))
    (dolist (window windows)
      (when (and (window-live-p window) (romacs-agent-log--chat-p (window-buffer window)))
        (let ((composer (romacs-agent-log--pair window)))
          (when (and composer (memq window owed) (eq window (selected-window)))
            (select-window composer)))))
    (romacs-agent-compose--settle-focus windows)))

(defun romacs-agent-compose--settle-focus (windows)
  "A restore owed its chat's composer the focus: selected, below the
selected window when it shows that chat, else below any of WINDOWS that
does.  Owed once: the restore's first pass settles it."
  (when-let* ((chat (and (buffer-live-p romacs-agent-compose--focus-owed) romacs-agent-compose--focus-owed))
              (window (if (eq (window-buffer) chat) (selected-window)
                        (seq-find (lambda (window) (and (window-live-p window) (eq (window-buffer window) chat)))
                                  windows)))
              (composer (romacs-agent-log--pair window)))
    (select-window composer))
  (setq romacs-agent-compose--focus-owed nil))

(add-hook 'window-buffer-change-functions #'romacs-agent-compose--windows)
;; A window state, saved and put back (warm-restart.el), keeps the pair
;; and which pane is the composer's.
(add-to-list 'window-persistent-parameters '(window-atom . writable))
(add-to-list 'window-persistent-parameters '(romacs-agent-composer . writable))

(defun romacs-agent-log--leave-composer ()
  "From a composer's window, select its chat's above: a buffer shown in
the selected window goes there, never in the composer's."
  (when-let* ((above (romacs-agent-compose--above (selected-window))))
    (select-window above)))

(defun romacs-agent-log-composer ()
  "Go to the composer: its window below the chat's."
  (interactive nil romacs-agent-log-mode)
  (select-window (or (romacs-agent-log--pair (selected-window))
                     (user-error "No room for the composer below this chat"))))

(defun romacs-agent-compose-transcript ()
  "Go to the transcript: the chat's window above the composer."
  (interactive nil romacs-agent-compose-mode)
  (select-window (or (romacs-agent-compose--above (selected-window))
                     (user-error "The chat is not shown above this composer"))))

(defun romacs-agent-log-type (n)
  "Type the key's character N times in the composer, its window selected:
typed on the read-only chat, it goes there."
  (interactive "p" romacs-agent-log-mode)
  (romacs-agent-log-composer)
  (self-insert-command n))

(defun romacs-agent-log-delete-backward ()
  "Delete backward in the composer, its window selected: DEL typed on the
read-only chat goes there, as a typed character does."
  (interactive nil romacs-agent-log-mode)
  (romacs-agent-log-composer)
  (call-interactively #'romacs-agent-log-attach-delete-backward))

;;; Sending

(defun romacs-agent-log--sent ()
  "Take the text its session accepted out of the composer, however it is
narrowed, as an edit undo can take back, and forget its id and what M-p
went through: only while the composer still starts with that text, else
it is left as it is, never changed silently."
  (let ((sent (or romacs-agent-log--sent-text "")))
    (setq romacs-agent-log--draft-id nil romacs-agent-log--recalled nil romacs-agent-log--sent-text nil)
    (save-restriction
      (widen)
      (when (string-prefix-p sent (buffer-substring-no-properties (point-min) (point-max)))
        (delete-region (point-min) (+ (point-min) (length sent)))
        (run-hooks 'romacs-agent-log-draft-replaced-hook)
        ;; Sent: nothing typed is left.
        (when (zerop (buffer-size)) (set-buffer-modified-p nil))))))

(defun romacs-agent-log--not-sent (answer)
  "Say what ANSWER, not an acceptance, means for its draft: why a refusal
did not send it, in the error face; or that it is unconfirmed (a remote
send unanswered, `romacs-agent-remote--unconfirmed'), and how it goes."
  (pcase (plist-get answer :status)
    ('refused (message "%s" (propertize (plist-get answer :message) 'face 'error)))
    ('unconfirmed (message "%s" (propertize (plist-get answer :message) 'face 'warning)))))

(defun romacs-agent-log--sending-ended ()
  "The composer sends no more: its draft writable, its prompt as before."
  (setq romacs-agent-log--sending nil buffer-read-only nil)
  (romacs-agent-compose--start-show))

(defun romacs-agent-log--draft-answered (composer answer)
  "COMPOSER's draft has ANSWER (`romacs-agent-submit'), when it is still
the one COMPOSER sends: writable again, and emptied once accepted, else
kept, the refusal or its being unconfirmed said
\(`romacs-agent-log--not-sent').  It keeps its id, so a retry is the same
draft, but for a `conflict': its id's earlier text was sent, so this
text takes a new one."
  (when (buffer-live-p composer)
    (with-current-buffer composer
      (when (and romacs-agent-log--sending (equal romacs-agent-log--sending (plist-get answer :draft-id)))
        (romacs-agent-log--sending-ended)
        (when (eq (plist-get answer :reason) 'conflict) (setq romacs-agent-log--draft-id nil))
        (if (eq (plist-get answer :status) 'accepted) (romacs-agent-log--sent) (romacs-agent-log--not-sent answer))))))

(defun romacs-agent-log--submit (kind)
  "Submit the composer's draft as KIND (`romacs-agent-submit'), under its
draft id, read-only until its answer comes, or until its source signals
instead of answering.  Nothing while one is sent, or when it is empty; a
queued message the composer holds to edit goes back in its place instead
\(`romacs-agent-log-queue-put-back')."
  (let ((text (romacs-agent-log--draft)) (composer (current-buffer)) (asked nil))
    (cond ((romacs-agent-log--editing) (romacs-agent-log-queue-put-back))
          ((or romacs-agent-log--sending (string-empty-p text)) nil)
          (t (setq romacs-agent-log--draft-id (or romacs-agent-log--draft-id (romacs-agent--uuid))
                   romacs-agent-log--sending romacs-agent-log--draft-id
                   romacs-agent-log--sent-text (save-restriction (widen) (buffer-substring-no-properties (point-min) (point-max)))
                   ;; Read-only until the answer: no edit, no undo, of a
                   ;; draft its session may be taking.
                   buffer-read-only t)
             (save-restriction (widen) (move-overlay romacs-agent-compose--guard (point-min) (point-max)))
             (romacs-agent-compose--start-show)
             (let ((intent (list :draft-id romacs-agent-log--draft-id :kind kind :text text :attachments nil)))
               (unwind-protect
                   (progn (with-current-buffer romacs-agent-log--chat
                            (romacs-agent-log--act #'romacs-agent-log-source-submit intent
                                                   (lambda (answer) (romacs-agent-log--draft-answered composer answer))))
                          (setq asked t))
                 (unless asked (romacs-agent-log--sending-ended))))))))

(defun romacs-agent-log-send ()
  "Send the composer's draft now: into the turn under way, when one runs;
a queued message held to edit goes back in its place instead."
  (interactive nil romacs-agent-compose-mode romacs-agent-log-mode)
  (romacs-agent-log-in-composer (romacs-agent-log--submit 'send)))

(defun romacs-agent-log-queue ()
  "Send the composer's draft once the turn under way ends."
  (interactive nil romacs-agent-compose-mode romacs-agent-log-mode)
  (romacs-agent-log-in-composer (romacs-agent-log--submit 'queue)))

(defun romacs-agent-log-interrupt-and-send ()
  "Interrupt the turn under way; the composer's draft is sent once it
ends.  With no draft, only the interrupt goes (`romacs-agent-log-interrupt')."
  (interactive nil romacs-agent-compose-mode romacs-agent-log-mode)
  (romacs-agent-log-in-composer
    (if (string-empty-p (romacs-agent-log--draft))
        (romacs-agent-log-interrupt)
      (romacs-agent-log--submit 'interrupt))))

;;; Its edits

(defvar romacs-agent-log--recalling nil
  "Non-nil while M-p or M-n replaces the draft (`romacs-agent-log--recall'):
the composer empties only on the way to its next text.")

(defun romacs-agent-log--draft-emptied (_beg _end _length)
  "A composer its user emptied holds another draft next: it takes a new id.
Not as M-p or M-n replaces the draft; and undo, giving the text back,
gives its id back (`romacs-agent-log--draft-id-again')."
  (when (and romacs-agent-log--draft-id (not romacs-agent-log--sending) (not romacs-agent-log--recalling)
             ;; All of it, however the composer is narrowed.
             (zerop (buffer-size)))
    (when (listp buffer-undo-list)
      (push (list 'apply #'romacs-agent-log--draft-id-again romacs-agent-log--draft-id) buffer-undo-list))
    (setq romacs-agent-log--draft-id nil)))

(defun romacs-agent-log--draft-id-again (id)
  "Make ID the composer's draft id again: undo gave back the text it named."
  (setq romacs-agent-log--draft-id id))

(defun romacs-agent-log--composer-plain (beg end _length)
  "Text typed or yanked into the composer is plain: nothing of the
transcript's comes with it."
  (when (and (< beg end)
             (or (text-properties-at beg)
                 (and (< (1+ beg) end)
                      (< (next-property-change beg nil end) end))))
    (let ((inhibit-modification-hooks t) (inhibit-read-only t)) (set-text-properties beg end nil))))

(defvar-local romacs-agent-compose--before nil
  "What the change under way may change the rows of: (NEWLINE . COLUMNS),
whether it deletes a line break, and the columns of the line it starts on.")

(defun romacs-agent-compose--columns (pos)
  "The columns of the line POS is on."
  (save-excursion (goto-char pos) (end-of-line) (current-column)))

(defun romacs-agent-compose--before-change (beg end)
  "Note what the change from BEG to END may change the rows of, while a
window shows the composer: nothing measured for one none shows."
  (setq romacs-agent-compose--before
        (and (get-buffer-window nil t)
             (cons (and (< beg end) (save-excursion (goto-char beg) (search-forward "\n" end t)) t)
                   (romacs-agent-compose--columns beg)))))

(defun romacs-agent-compose--rows-changed-p (beg end)
  "Whether the change from BEG to END may have changed the draft's rows: a
line break typed or deleted, or its line, before or after, long enough
to wrap in a window, the prompt's mark taken from its width.  A word
wraps before the width's multiple, so a line that long is measured on
every edit; a shorter one, never."
  (pcase-let ((`(,newline . ,columns) romacs-agent-compose--before))
    (or newline
        (save-excursion (goto-char beg) (search-forward "\n" end t))
        (let ((longest (max columns (romacs-agent-compose--columns beg)))
              (mark (string-width (concat (romacs-agent-glyph 'composer) " "))))
          (seq-some (lambda (window) (>= longest (- (window-body-width window) mark)))
                    (get-buffer-window-list nil nil t))))))

(defun romacs-agent-compose--after-change (beg end _length)
  "The change from BEG to END may have changed the draft's rows
\(`romacs-agent-compose--rows-changed-p'): each window showing it fitted
again.  Any other edit measures nothing."
  (when (and romacs-agent-compose--before (romacs-agent-compose--rows-changed-p beg end))
    (romacs-agent-compose--refit))
  (setq romacs-agent-compose--before nil))

(defun romacs-agent-compose--tray-changed ()
  "The attachments' tray changed: shown at the draft's start, the
composer's windows fitted to it."
  (romacs-agent-compose--start-show)
  (romacs-agent-compose--refit))

;;; Its history

(defun romacs-agent-log--recall (cells)
  "Show the prompt of the first of CELLS in the composer, or the draft;
nothing while the draft is sent (`romacs-agent-log--sending')."
  (unless romacs-agent-log--sending
    (unless romacs-agent-log--recalled
      (setq romacs-agent-log--draft-kept (save-restriction (widen) (buffer-substring-no-properties (point-min) (point-max)))))
    (setq romacs-agent-log--recalled cells)
    (let ((romacs-agent-log--recalling t))
      (erase-buffer)
      (run-hooks 'romacs-agent-log-draft-replaced-hook)
      (insert (if cells (car (car cells)) (or romacs-agent-log--draft-kept ""))))))

(defun romacs-agent-log-history-previous ()
  "Put the chat's previous prompt in the composer."
  (interactive nil romacs-agent-compose-mode)
  (let ((cell (if romacs-agent-log--recalled (cdr (car romacs-agent-log--recalled))
                (buffer-local-value 'romacs-agent-log--prompts romacs-agent-log--chat))))
    (unless cell (user-error "No earlier prompt"))
    (romacs-agent-log--recall (cons cell romacs-agent-log--recalled))))

(defun romacs-agent-log-history-next ()
  "Put the chat's next prompt in the composer; past the newest, empty it."
  (interactive nil romacs-agent-compose-mode)
  (unless romacs-agent-log--recalled (user-error "No later prompt"))
  (romacs-agent-log--recall (cdr romacs-agent-log--recalled)))

;;; Keys and mode

(defvar-keymap romacs-agent-compose-mode-map
  :doc "Keys of a chat's composer: it edits as any buffer does, and RET
sends.  The session's keys are its chat's (`romacs-agent-log-session-map')."
  :parent romacs-agent-log-session-map
  "RET" #'romacs-agent-log-send
  "C-j" #'newline
  "S-<return>" #'newline
  "DEL" #'romacs-agent-log-attach-delete-backward
  "<remap> <yank>" #'romacs-agent-log-attach-yank
  "C-c C-j" #'romacs-agent-compose-transcript
  "M-p" #'romacs-agent-log-history-previous
  "M-n" #'romacs-agent-log-history-next
  ;; Killed only with its chat.
  "<remap> <kill-buffer>" #'romacs-agent-compose-kill-chat
  "<remap> <kill-current-buffer>" #'romacs-agent-compose-kill-chat
  "<remap> <kill-this-buffer>" #'romacs-agent-compose-kill-chat
  "<remap> <romacs-gui-kill-buffer>" #'romacs-agent-compose-kill-chat)

(define-derived-mode romacs-agent-compose-mode fundamental-mode "Agent compose"
  "A chat's composer: its draft, which RET sends to the chat's session.
\\{romacs-agent-compose-mode-map}"
  :interactive nil
  (setq-local truncate-lines nil word-wrap t mode-line-format nil header-line-format nil)
  ;; Its window is dedicated: a buffer switched to from it is shown
  ;; elsewhere, not refused.
  (setq-local switch-to-buffer-in-dedicated-window 'pop)
  ;; Empty, it stays at the draft's start whatever is typed there.
  (setq romacs-agent-compose--start (make-overlay (point-min) (point-min)))
  (romacs-agent-log-attach-setup)
  (add-hook 'romacs-agent-log-attach-tray-functions #'romacs-agent-compose--tray-changed nil t)
  (romacs-agent-compose--start-show)
  (add-hook 'after-change-functions #'romacs-agent-log--composer-plain nil t)
  (add-hook 'after-change-functions #'romacs-agent-log--draft-emptied nil t)
  (add-hook 'before-change-functions #'romacs-agent-compose--before-change nil t)
  (add-hook 'after-change-functions #'romacs-agent-compose--after-change nil t)
  (add-hook 'window-size-change-functions #'romacs-agent-compose--resized nil t)
  (add-hook 'kill-buffer-query-functions #'romacs-agent-compose--may-kill nil t)
  (add-hook 'kill-buffer-hook #'romacs-agent-compose--killed nil t)
  ;; Over the whole draft, whatever is typed at either end.
  (setq romacs-agent-compose--guard (make-overlay (point-min) (point-max) nil nil t))
  (dolist (hooks '(modification-hooks insert-in-front-hooks insert-behind-hooks))
    (overlay-put romacs-agent-compose--guard hooks (list #'romacs-agent-compose--guard-edit))))

(provide 'romacs-agent-log-compose)
;;; romacs-agent-log-compose.el ends here
