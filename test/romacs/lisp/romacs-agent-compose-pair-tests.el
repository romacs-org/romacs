;;; romacs-agent-compose-pair-tests.el --- A chat and its composer, as a pair -*- lexical-binding: t; -*-
;; SPDX-License-Identifier: GPL-3.0-or-later
;; Maintainer: Thanos Apollo <public@thanosapollo.org>

;;; Commentary:
;; What a chat and its composer keep through what is done to them: a
;; killed composer, a slot whose buffer was replaced, a split, the
;; prompt's mark, the chat's keys, the menu's focus, a wrapped draft.

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

(defmacro romacs-agent-compose-pair-tests--with (&rest body)
  "Run BODY with a clean `session', its `chat' and its `composer'."
  (declare (indent 0))
  `(let* ((romacs-agent-log--chats (make-hash-table :test #'equal))
          (session (romacs-agent--make :id "pair-test" :harness 'claude :state 'running))
          (chat (romacs-agent-log-chat session))
          (composer (romacs-agent-log-composer-buffer chat)))
     (unwind-protect
         (save-window-excursion ,@body)
       (dolist (buffer (list chat composer))
         (when (buffer-live-p buffer) (kill-buffer buffer))))))

(defun romacs-agent-compose-pair-tests--redisplay ()
  "Settle window change hooks, including in a batch frame."
  (when noninteractive (romacs--run-window-change-functions))
  (redisplay t))

(defun romacs-agent-compose-pair-tests--key (key)
  "Dispatch KEY through Romacs's command loop."
  (let ((unread-command-events (mapcar (lambda (event) (cons t event))
                                      (listify-key-sequence (kbd key)))))
    (should-not (cdr (romacs--run-host-command)))))

(defun romacs-agent-compose-pair-tests--alone (session)
  "Show SESSION's chat and composer as all the frame shows."
  (let ((ignore-window-parameters t)) (delete-other-windows))
  (romacs-agent-log-show session '((display-buffer-same-window)))
  (romacs-agent-compose-pair-tests--redisplay))

(defun romacs-agent-compose-pair-tests--paired (chat composer)
  "Assert the frame shows CHAT above COMPOSER, one pair, and no more."
  (let ((top (get-buffer-window chat)) (bottom (get-buffer-window composer)))
    (should (window-live-p top))
    (should (window-live-p bottom))
    (should (eq (window-parameter bottom 'window-atom) 'below))
    (should (eq (window-prev-sibling bottom) top))
    (should (= (length (window-list)) 2))))

;;; B1: a killed composer leaves its draft to the next

(ert-deftest romacs-agent-compose-pair-a-killed-composer-leaves-its-draft ()
  (romacs-agent-compose-pair-tests--with
    (with-current-buffer composer (insert "keep my draft"))
    (kill-buffer composer)
    (should (buffer-live-p chat))
    (setq composer (romacs-agent-log-composer-buffer chat))
    (should (equal (with-current-buffer composer (buffer-string)) "keep my draft"))
    ;; Given once: the next one after it starts empty.
    (with-current-buffer composer (erase-buffer))
    (kill-buffer composer)
    (setq composer (romacs-agent-log-composer-buffer chat))
    (should (equal (with-current-buffer composer (buffer-string)) ""))))

;;; B2: a composer's slot is the window's, whatever it shows

(ert-deftest romacs-agent-compose-pair-burying-the-composer-keeps-one-pair ()
  (romacs-agent-compose-pair-tests--with
    (romacs-agent-compose-pair-tests--alone session)
    (romacs-agent-compose-pair-tests--paired chat composer)
    (select-window (get-buffer-window composer))
    (bury-buffer)
    (romacs-agent-compose-pair-tests--redisplay)
    (romacs-agent-compose-pair-tests--paired chat composer)))

(ert-deftest romacs-agent-compose-pair-a-slot-holding-another-chat-goes ()
  ;; As a warm restore leaves it: another chat shown above, the first
  ;; chat's transcript in the composer's slot.
  (romacs-agent-compose-pair-tests--with
    (let* ((other-session (romacs-agent--make :id "pair-other" :harness 'claude :state 'running))
           (other (romacs-agent-log-chat other-session)))
      (unwind-protect
          (progn
            (romacs-agent-compose-pair-tests--alone session)
            (let ((top (get-buffer-window chat)) (bottom (get-buffer-window composer)))
              (set-window-buffer top other)
              (set-window-dedicated-p bottom nil)
              (set-window-buffer bottom chat))
            (romacs-agent-compose-pair-tests--redisplay)
            (romacs-agent-compose-pair-tests--paired other (romacs-agent-log-composer-buffer other)))
        (kill-buffer other)))))

;;; B3: a split of the pair keeps both panes

(ert-deftest romacs-agent-compose-pair-a-split-from-the-composer-keeps-its-pane ()
  (romacs-agent-compose-pair-tests--with
    (romacs-agent-compose-pair-tests--alone session)
    (select-window (get-buffer-window composer))
    (romacs-agent-compose-pair-tests--key "C-x 3")
    (romacs-agent-compose-pair-tests--redisplay)
    ;; The new pane shows the chat, a pair of its own; the first keeps its row.
    (should (= (length (window-list)) 4))
    (should (= (length (get-buffer-window-list chat)) 2))
    (dolist (window (get-buffer-window-list composer))
      (should (eq (window-parameter window 'window-atom) 'below))
      (should (= (window-body-height window t) (frame-char-height))))))

;;; B4: the prompt's mark, once, before the first line

(ert-deftest romacs-agent-compose-pair-the-prompt-is-one-mark-before-the-draft ()
  (romacs-agent-compose-pair-tests--with
    (with-current-buffer composer
      (cl-flet ((mark ()
                  (let ((at (seq-filter (lambda (o) (overlay-get o 'before-string))
                                        (overlays-in (point-min) (point-min)))))
                    (should (= (length at) 1))
                    (overlay-get (car at) 'before-string))))
        ;; Empty, the mark still shows.
        (should (string-suffix-p (concat (romacs-agent-glyph 'composer) " ") (mark)))
        (insert "one\ntwo")
        (goto-char (point-min))
        (insert "zero ")
        (let ((shown (mark)))
          (should (string-suffix-p (concat (romacs-agent-glyph 'composer) " ") shown))
          (should (eq (get-text-property (1- (length shown)) 'face shown) 'romacs-agent-log-prompt)))
        ;; No mark on every line.
        (should-not line-prefix)
        (should-not (get-char-property (point-min) 'line-prefix))
        (should-not (get-char-property (point-max) 'line-prefix))
        ;; Sent: the mark in the sending face, the draft read-only.
        (cl-letf (((symbol-function 'romacs-agent-submit) #'ignore))
          (romacs-agent-log-send))
        (let ((shown (mark)))
          (should (eq (get-text-property (1- (length shown)) 'face shown) 'romacs-agent-compose-sending)))
        (should buffer-read-only)))))

;;; B5: g in the chat selects the composer

(ert-deftest romacs-agent-compose-pair-g-at-the-chat-s-end-types-nothing ()
  (romacs-agent-compose-pair-tests--with
    (romacs-agent-compose-pair-tests--alone session)
    (with-current-buffer composer (insert "draft"))
    (romacs-agent-compose-pair-tests--key "C-c C-j")
    (should (eq (window-buffer) chat))
    (goto-char (point-max))
    (romacs-agent-compose-pair-tests--key "g")
    (should (eq (window-buffer) composer))
    (should (equal (with-current-buffer composer (buffer-string)) "draft"))))

;;; B7: the menu opened from the composer gives the focus back

(defun romacs-agent-compose-pair-tests--menu (session chat composer keys)
  "Open the chat's menu from COMPOSER with C-c C-o, then type KEYS, each a
command: the selected window's buffer after."
  (romacs-agent-compose-pair-tests--alone session)
  (should (eq (window-buffer) composer))
  ;; A batch frame has no layer: the popup in a side window.
  (let ((keymap-popup-backend #'keymap-popup-backend-side-window))
    (romacs-agent-compose-pair-tests--key "C-c C-o")
    (should (eq (window-buffer) chat))
    (dolist (key keys) (romacs-agent-compose-pair-tests--key key))
    (romacs-agent-compose-pair-tests--redisplay)
    (window-buffer)))

(ert-deftest romacs-agent-compose-pair-the-menu-gives-the-focus-back ()
  (romacs-agent-compose-pair-tests--with
    (cl-letf (((symbol-function 'read-string) (lambda (&rest _) "named"))
              ((symbol-function 'romacs-agent--update-manifest) #'ignore)
              ((symbol-function 'romacs-agent-ui--batch) #'ignore)
              ((symbol-function 'romacs-agent-log--loaded) #'identity)
              ((symbol-function 'romacs-agent-respond) #'ignore))
      ;; C-g: closed, nothing run.
      (should (eq (romacs-agent-compose-pair-tests--menu session chat composer '("C-g")) composer))
      ;; R, its title asked: renamed, then back.
      (should (eq (romacs-agent-compose-pair-tests--menu session chat composer '("R")) composer))
      (should (equal (romacs-agent-title session) "named"))
      ;; d on what waits, its reason asked: denied, then back.
      (puthash "q1" t (romacs-agent-approvals session))
      (with-current-buffer chat
        (romacs-agent-log-draw session (list :type 'approval :request-id "q1" :tool "Bash")))
      (should (eq (romacs-agent-compose-pair-tests--menu session chat composer '("d")) composer)))))

(ert-deftest romacs-agent-compose-pair-the-menu-leaves-focus-it-moved ()
  (romacs-agent-compose-pair-tests--with
    (let* ((board (get-buffer-create "*pair-test board*"))
           (other-session (romacs-agent--make :id "pair-new" :harness 'claude :state 'idle))
           (other nil))
      ;; A new session starts in this one's directory.
      (setf (romacs-agent-cwd session) default-directory)
      (unwind-protect
          (cl-letf (((symbol-function 'romacs-agents)
                     (lambda () (interactive) (switch-to-buffer board)))
                    ((symbol-function 'romacs-agent--start-arguments) #'ignore)
                    ((symbol-function 'romacs-agent-start)
                     (lambda (&rest _) (interactive)
                       (setq other (romacs-agent-log-show other-session)))))
            ;; b: the board took the chat's window.
            (should (eq (romacs-agent-compose-pair-tests--menu session chat composer '("b")) board))
            ;; n: a new session's composer took the focus.
            (should (eq (romacs-agent-compose-pair-tests--menu session chat composer '("n"))
                        (romacs-agent-log-composer-buffer other))))
        (kill-buffer board)
        (when (buffer-live-p other) (kill-buffer other))))))

;;; S1: a wrapped draft gets its rows

(ert-deftest romacs-agent-compose-pair-a-wrapped-draft-gets-its-rows ()
  (romacs-agent-compose-pair-tests--with
    (romacs-agent-compose-pair-tests--alone session)
    (let ((window (get-buffer-window composer)))
      (with-current-buffer composer
        (insert (mapconcat #'identity (make-list 60 "word") " ")))
      (romacs-agent-compose-pair-tests--redisplay)
      (should (> (window-body-height window t) (frame-char-height)))
      (should (= (window-body-height window t)
                 (romacs-agent-compose--height window)))
      (with-current-buffer composer (erase-buffer))
      (romacs-agent-compose-pair-tests--redisplay)
      (should (= (window-body-height window t) (frame-char-height))))))

;;; DEL at the read-only chat's end goes to the composer, as typing does

(ert-deftest romacs-agent-compose-pair-del-at-the-chat-s-end-deletes-in-the-composer ()
  (romacs-agent-compose-pair-tests--with
    (romacs-agent-compose-pair-tests--alone session)
    (with-current-buffer composer (insert "draft"))
    (romacs-agent-compose-pair-tests--key "C-c C-j")
    (should (eq (window-buffer) chat))
    (goto-char (point-max))
    (romacs-agent-compose-pair-tests--key "DEL")
    (should (eq (window-buffer) composer))
    (should (equal (with-current-buffer composer (buffer-string)) "draf"))))

(provide 'romacs-agent-compose-pair-tests)
;;; romacs-agent-compose-pair-tests.el ends here
