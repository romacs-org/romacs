;;; romacs-agent-switch-tests.el --- Switching between agent chats -*- lexical-binding: t; -*-
;; SPDX-License-Identifier: GPL-3.0-or-later
;; Maintainer: Thanos Apollo <public@thanosapollo.org>
(require 'ert)
(require 'cl-lib)
(require 'romacs-mcp)

;; Loading romacs-agent installs romacs-agent-tools: the endpoint's
;; registrations are left as they were.
(let ((romacs-mcp-call-function romacs-mcp-call-function)
      (romacs-mcp-tools (copy-alist romacs-mcp-tools)))
  (require 'romacs-agent-switch)
  (require 'romacs-agent-board))

(defmacro romacs-agent-switch-tests--with (&rest body)
  "BODY with `chat' making a chat buffer (NAME SESSION) and `session'
making a session (ID STATE &rest FIELDS) in a registry of its own; the
buffers are killed after."
  (declare (indent 0))
  `(let ((romacs-agent-glyphs nil) (made nil) (romacs-agent--sessions (make-hash-table :test #'equal)))
     (cl-flet ((chat (name session)
                 (let ((buffer (generate-new-buffer name)))
                   (push buffer made)
                   (with-current-buffer buffer
                     (romacs-agent-log-mode)
                     (setq romacs-agent-log--session session
                           romacs-agent-log--session-id (and session (romacs-agent-id session))))
                   buffer))
               (session (id state &rest fields)
                 (puthash id (apply #'romacs-agent--make
                                    (append fields (list :id id :harness 'claude :provider "claude" :state state
                                                         :cwd "/p/romacs/")))
                          romacs-agent--sessions)))
       (ignore #'chat #'session)
       (unwind-protect (progn ,@body)
         (mapc #'kill-buffer made)))))

(ert-deftest romacs-agent-switch-chats-in-order-current-last ()
  (romacs-agent-switch-tests--with
    (let* ((a (chat "*agent: a*" nil)) (b (chat "*agent: b*" nil))
           (other (generate-new-buffer "*not a chat*")))
      (unwind-protect
          (progn
            (should (equal (romacs-agent-switch--chats (list b other a) other) (list b a)))
            (should (equal (romacs-agent-switch--chats (list b other a) b) (list a b)))
            (should (null (romacs-agent-switch--chats (list other) other))))
        (kill-buffer other)))))

(ert-deftest romacs-agent-switch-annotation-says-state-instance-project ()
  (romacs-agent-switch-tests--with
    (let ((text (romacs-agent-switch--annotation
                 (romacs-agent-summary (romacs-agent-id (session "s1" 'running :model "claude-opus-5-5"))))))
      (should (equal (substring-no-properties text) "  Working claude · opus-5-5 romacs"))
      (should (eq (get-text-property 2 'face text) 'romacs-agent-running))
      (should (eq (get-text-property (string-search "claude" text) 'face text) 'romacs-agent-model))
      (should (eq (get-text-property (string-search "romacs" text) 'face text) 'romacs-agent-dim)))
    (should (equal (substring-no-properties
                    (romacs-agent-switch--annotation (romacs-agent-summary (romacs-agent-id (session "s2" 'idle)))))
                   "  Idle claude romacs"))
    (should (equal (romacs-agent-switch--annotation nil) ""))))

(ert-deftest romacs-agent-switch-properties-say-category-and-state ()
  (romacs-agent-switch-tests--with
    (let* ((chats (list (chat "*agent: z*" (session "z" 'running)) (chat "*agent: a*" (session "a" 'idle))))
           (properties (romacs-agent-switch--properties chats)))
      (should (eq (plist-get properties :category) 'romacs-agent-chat))
      (let ((annotate (plist-get properties :annotation-function)))
        (should (equal (substring-no-properties (funcall annotate "*agent: z*")) "  Working claude romacs"))
        (should (equal (substring-no-properties (funcall annotate "*agent: a*")) "  Idle claude romacs")))))
  (romacs-agent-switch-tests--with
    (let* ((properties (romacs-agent-switch--properties (list (chat "*agent: z*" (session "z" 'idle))
                                                              (chat "*agent: longer*" (session "l" 'idle)))))
           (annotate (plist-get properties :annotation-function)))
      (should (equal (+ (string-width "*agent: z*") (length (funcall annotate "*agent: z*")))
                     (+ (string-width "*agent: longer*") (length (funcall annotate "*agent: longer*"))))))))

(ert-deftest romacs-agent-switch-says-what-the-chats-header-says ()
  (romacs-agent-switch-tests--with
    (let ((session (session "w" 'waiting-approval)))
      (should (string-prefix-p (concat "  " (nth 2 (romacs-agent-ui--state-of (romacs-agent-ui--state-key session))))
                               (substring-no-properties
                                (romacs-agent-switch--annotation (romacs-agent-summary (romacs-agent-id session)))))))))

(ert-deftest romacs-agent-switch-switches-to-the-chosen-chat ()
  (romacs-agent-switch-tests--with
    (let ((a (chat "*agent: a*" (session "a" 'idle))) (shown nil))
      (cl-letf (((symbol-function 'completing-read) (lambda (&rest _) "*agent: a*"))
                ((symbol-function 'switch-to-buffer)
                 (lambda (buffer _norecord same-window) (setq shown (list buffer same-window)))))
        (romacs-agent-switch))
      (should (equal shown (list a t))))))

(ert-deftest romacs-agent-switch-names-its-own-buffer-before-case-folded ()
  (romacs-agent-switch-tests--with
    (let* ((upper (chat "*agent: Build*" nil)) (lower (chat "*agent: build*" nil))
           (named (list (cons (buffer-name upper) upper) (cons (buffer-name lower) lower))))
      (should (eq (romacs-agent-switch--named "*agent: build*" named) lower))
      (should (eq (romacs-agent-switch--named "*agent: Build*" named) upper))
      (should-not (romacs-agent-switch--named "*agent: BUILD*" named))
      (should (eq (romacs-agent-switch--named "*AGENT: BUILD*" (list (cons (buffer-name lower) lower))) lower)))))

(ert-deftest romacs-agent-switch-refuses-a-chat-gone-while-asked ()
  (romacs-agent-switch-tests--with
    (let ((a (chat "*agent: a*" (session "a" 'idle))) (before (length (buffer-list))))
      (cl-letf (((symbol-function 'completing-read) (lambda (&rest _) (kill-buffer a) "*agent: a*")))
        (should-error (romacs-agent-switch) :type 'user-error))
      (should-not (get-buffer "*agent: a*"))
      (should (= (length (buffer-list)) (1- before))))))

(ert-deftest romacs-agent-switch-aligns-annotations-at-most-so-far ()
  (romacs-agent-switch-tests--with
    (let* ((long (make-string 70 ?x))
           (annotate (plist-get (romacs-agent-switch--properties
                                 (list (chat "*agent: z*" (session "z" 'idle)) (chat long (session "l" 'idle))))
                                :annotation-function)))
      (should (= (+ (string-width "*agent: z*") (length (funcall annotate "*agent: z*")))
                 (+ romacs-agent-switch--column (length "  Idle claude romacs"))))
      (should (equal (substring-no-properties (funcall annotate long)) "  Idle claude romacs")))))

(ert-deftest romacs-agent-switch-no-chats-offers-the-board ()
  (let ((asked nil) (board nil))
    (cl-letf (((symbol-function 'romacs-agent-switch--chats) (lambda (&rest _) nil))
              ((symbol-function 'y-or-n-p) (lambda (prompt) (setq asked prompt) t))
              ((symbol-function 'romacs-agents) (lambda () (setq board t))))
      (romacs-agent-switch))
    (should (string-match-p "No agent chats are open" asked))
    (should board)))

(ert-deftest romacs-agent-switch-is-its-own-command-on-the-key ()
  (should (eq (symbol-function 'romacs-agent-switch) (indirect-function 'romacs-agent-switch)))
  (should-not (eq (indirect-function 'romacs-agent-switch) (indirect-function 'romacs-agents)))
  (should-not (fboundp 'romacs-agent-switch-chat))
  (should (eq (lookup-key romacs-agents-map "s") #'romacs-agent-switch))
  (should (eq (lookup-key romacs-agents-map "a") #'romacs-agents)))

;;; romacs-agent-switch-tests.el ends here
