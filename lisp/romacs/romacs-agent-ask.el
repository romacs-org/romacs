;;; romacs-agent-ask.el --- Questions an agent asks, as cards -*- lexical-binding: t; -*-

;; SPDX-License-Identifier: GPL-3.0-or-later
;; Maintainer: Thanos Apollo <public@thanosapollo.org>

;;; Commentary:
;; A harness's questions (Claude's AskUserQuestion, Codex's user input)
;; arrive as an `approval' event with :questions.  A chat
;; (romacs-agent-ui.el) draws them as one card: each question a row, each
;; option a row under it, and a row for words of one's own.  RET on an
;; option picks it, n and p step through the options, and the card sends
;; once each question has its answer: at once for one question of one
;; choice, else by its Send row.  Answered, the card settles on the
;; answers sent (`romacs-agent-ask-note-sent'), by it or by `y': they
;; stay, words of one's own among them, the rest recedes, its keys go.
;; Answered where this chat did not send it (another client, a replay),
;; it says it was answered, and the call's row says how.
;;
;; `romacs-agent-ask-card' is pure: the chat inserts what it returns.  The
;; picks so far are the chat's, by request id.

;;; Code:

(require 'cl-lib)

(declare-function romacs-agent-respond "romacs-agent" (session request-id allow &optional message))
(defvar romacs-agent-ui--session)
(defvar romacs-agent-chat-transcript-map)

(defface romacs-agent-ask-option '((t :inherit default))
  "An option of a question a chat is asked, not yet picked.")
(defface romacs-agent-ask-picked '((t :inherit romacs-agent-done :weight bold))
  "The option picked for a question a chat was asked.")

(defvar-local romacs-agent-ask--sent nil
  "The answers this chat sent, by request id: ((QUESTION . ANSWER) ...).")

(defun romacs-agent-ask-note-sent (id answers)
  "Remember ANSWERS as what request ID was answered with, for its card."
  (setf (alist-get id romacs-agent-ask--sent nil nil #'equal) answers))

(defvar-local romacs-agent-ask--picks nil
  "Each question card's picks so far, by request id: an alist of
question index to the labels picked, in order.")

(defvar romacs-agent-ask-map
  (let ((map (make-sparse-keymap)))
    (define-key map (kbd "RET") #'romacs-agent-ask-pick)
    (define-key map [mouse-1] #'romacs-agent-ask-pick)
    (define-key map "n" #'romacs-agent-ask-next)
    (define-key map "p" #'romacs-agent-ask-previous)
    map)
  "Keys of a question card's rows while it waits for its answers.")

(defun romacs-agent-ask--glyph (multi picked)
  "An option's mark: a box for a question taking several, else a dot."
  (if multi (if picked "☑" "☐") (if picked "●" "○")))

(defun romacs-agent-ask--sends-at-once-p (questions)
  "Whether a pick answers QUESTIONS whole: one question, one choice."
  (and (= (length questions) 1) (not (plist-get (cdar questions) :multi))))

(defun romacs-agent-ask--row (id index what text)
  "A card row for request ID's question INDEX: WHAT it is (an option's
label, `other' or `send'), TEXT what it shows."
  (propertize text 'romacs-agent-ask (list id index what) 'romacs-agent-approval id
              'keymap romacs-agent-ask-map 'mouse-face 'highlight))

(defun romacs-agent-ask-card (id questions)
  "The card of request ID asking QUESTIONS (an `approval' event's
:questions): a row for each question, its options and one for other
words under it, then a Send row when one pick does not answer it all."
  (concat
   (cl-loop
    for (question . props) in questions
    for index from 0
    for multi = (plist-get props :multi)
    for notes = (plist-get props :notes)
    concat (concat
            (propertize (concat "◆ " (if-let* ((header (plist-get props :header)))
                                          (concat (propertize header 'face 'romacs-agent-dim) "  ")
                                        "")
                                question "\n")
                        'face 'romacs-agent-approval 'romacs-agent-approval id)
            (mapconcat
             (lambda (label)
               (romacs-agent-ask--row
                id index label
                (concat "    " (romacs-agent-ask--glyph multi nil) " "
                        (propertize label 'face 'romacs-agent-ask-option)
                        (if-let* ((note (cdr (assoc label notes))))
                            (propertize (concat "  " note) 'face 'romacs-agent-dim)
                          "")
                        "\n")))
             (plist-get props :options) "")
            (romacs-agent-ask--row id index 'other
                                   (propertize "    ✎ Other…\n" 'face 'romacs-agent-dim))))
   (if (romacs-agent-ask--sends-at-once-p questions) ""
     (romacs-agent-ask--row id nil 'send (propertize "    ↵ Send\n" 'face 'romacs-agent-link)))))

(defun romacs-agent-ask-answers (questions picks)
  "QUESTIONS' answers from PICKS (as `romacs-agent-ask--picks' keeps
them), ((QUESTION . ANSWER) ...), or nil while one has none."
  (cl-loop for (question . _) in questions
           for index from 0
           for labels = (cdr (assq index picks))
           unless labels return nil
           collect (cons question (string-join labels ", "))))

(defun romacs-agent-ask--repick (picks index label multi)
  "PICKS with LABEL picked for question INDEX: added or taken back for a
question of several choices, else the one."
  (let* ((had (cdr (assq index picks)))
         (labels (cond ((not multi) (list label))
                       ((member label had) (remove label had))
                       (t (append had (list label))))))
    (cons (cons index labels) (assq-delete-all index (copy-sequence picks)))))

;;; In the chat

(defun romacs-agent-ask--here ()
  "The card row at point: (ID INDEX WHAT), or nil."
  (get-text-property (line-beginning-position) 'romacs-agent-ask))

(defun romacs-agent-ask--asked (id)
  "Request ID's questions, as its approval event gave them."
  (plist-get (gethash id (romacs-agent-ui--asks)) :questions))

(declare-function romacs-agent-ui--asks "romacs-agent-ui" ())

(defun romacs-agent-ask--rows (id)
  "The starts of request ID's card rows that carry a choice, in order."
  (let ((at (point-min)) (rows nil))
    (while (setq at (text-property-not-all at (point-max) 'romacs-agent-ask nil))
      (let ((here (get-text-property at 'romacs-agent-ask))
            (end (next-single-property-change at 'romacs-agent-ask nil (point-max))))
        (when (equal (car here) id) (push at rows))
        (setq at end)))
    (nreverse rows)))

(defun romacs-agent-ask--show-picks (id)
  "Mark request ID's card's options as its picks have them."
  (let* ((questions (romacs-agent-ask--asked id))
         (picks (alist-get id romacs-agent-ask--picks nil nil #'equal))
         (inhibit-read-only t))
    (save-excursion
      (dolist (start (romacs-agent-ask--rows id))
        (pcase-let ((`(,_ ,index ,what) (get-text-property start 'romacs-agent-ask)))
          (when (stringp what)
            (let* ((multi (plist-get (cdr (nth index questions)) :multi))
                   (picked (member what (cdr (assq index picks))))
                   (glyph (+ start 4)))
              (goto-char glyph)
              (let ((properties (text-properties-at glyph)))
                (delete-char 1)
                (insert (apply #'propertize (romacs-agent-ask--glyph multi picked) properties)))
              (put-text-property (+ glyph 2) (+ glyph 2 (length what)) 'face
                                 (if picked 'romacs-agent-ask-picked 'romacs-agent-ask-option)))))))))

(defun romacs-agent-ask-chosen (question answer)
  "The labels ANSWER (a string, as sent) chose for QUESTION, an
`approval' event's: its options, and what it said in its own words."
  (let* ((props (cdr question))
         (said (if (plist-get props :multi) (split-string answer ", " t) (list answer)))
         (options (plist-get props :options)))
    (list (cl-intersection said options :test #'equal)
          (cl-set-difference said options :test #'equal))))

(declare-function romacs-agent-ui--insert-at "romacs-agent-ui" (position text))

(defun romacs-agent-ask-settle (id)
  "Settle request ID's card, answered: its keys go, the options its sent
answers chose stay, words of one's own take the Other row, and the rest
recedes.  With no answers known, its last row says it was answered."
  (let* ((questions (romacs-agent-ask--asked id))
         (answers (alist-get id romacs-agent-ask--sent nil nil #'equal))
         (rows (romacs-agent-ask--rows id))
         (last-other (car (last (cl-remove-if-not (lambda (at) (eq (nth 2 (get-text-property at 'romacs-agent-ask)) 'other))
                                                  rows))))
         (inhibit-read-only t))
    ;; Marked as its answers chose.
    (setf (alist-get id romacs-agent-ask--picks nil nil #'equal)
          (cl-loop for question in questions
                   for index from 0
                   for answer = (cdr (assoc (car question) answers))
                   when answer collect (cons index (car (romacs-agent-ask-chosen question answer)))))
    (romacs-agent-ask--show-picks id)
    (save-excursion
      ;; Last first: a row rewritten moves none still to come.
      (dolist (start (reverse rows))
        (let ((end (next-single-property-change start 'romacs-agent-ask nil (point-max))))
          (pcase-let* ((`(,_ ,index ,what) (get-text-property start 'romacs-agent-ask))
                       (question (and index (nth index questions)))
                       (answer (and question (cdr (assoc (car question) answers))))
                       (`(,options ,own) (if answer (romacs-agent-ask-chosen question answer) '(nil nil))))
            (cond
             ((stringp what)
              (put-text-property start end 'keymap romacs-agent-chat-transcript-map)
              (remove-text-properties start end '(mouse-face nil))
              (unless (member what options)
                (add-face-text-property start end 'romacs-agent-dim)))
             (t
              (delete-region start end)
              (let ((said (cond (own (propertize (concat "    ● " (string-join own ", ") "\n")
                                                 'face 'romacs-agent-ask-picked))
                                ((and (not answers) (eq start last-other))
                                 (propertize "    ✓ answered\n" 'face 'romacs-agent-dim)))))
                (when said (romacs-agent-ui--insert-at start said))))))))
      ;; Settled, its rows are a card's no more.
      (dolist (start (romacs-agent-ask--rows id))
        (let ((end (next-single-property-change start 'romacs-agent-ask nil (point-max))))
          (remove-text-properties start end '(romacs-agent-ask nil)))))
    (setq romacs-agent-ask--picks (cl-remove id romacs-agent-ask--picks :key #'car :test #'equal)
          romacs-agent-ask--sent (cl-remove id romacs-agent-ask--sent :key #'car :test #'equal))))

(defun romacs-agent-ask--send (id)
  "Send request ID's answers, when each question has one."
  (let* ((questions (romacs-agent-ask--asked id))
         (answers (romacs-agent-ask-answers questions (alist-get id romacs-agent-ask--picks nil nil #'equal))))
    (unless answers
      (user-error "Answer each question first"))
    (romacs-agent-ask-note-sent id answers)
    (romacs-agent-respond romacs-agent-ui--session id (list :answers answers))))

(defun romacs-agent-ask-pick ()
  "Pick the option at point, give words of your own for its question, or
send the card's answers."
  (interactive nil romacs-agent-chat-mode)
  (pcase-let* ((`(,id ,index ,what) (or (romacs-agent-ask--here) (user-error "No option here")))
               (questions (romacs-agent-ask--asked id)))
    (unless questions (user-error "This question is answered"))
    (pcase what
      ('send (romacs-agent-ask--send id))
      (_ (let* ((question (nth index questions))
                (label (if (eq what 'other) (read-string (format-prompt (car question) nil)) what)))
           (when (string-empty-p (string-trim label)) (user-error "No answer given"))
           (setf (alist-get id romacs-agent-ask--picks nil nil #'equal)
                 (romacs-agent-ask--repick (alist-get id romacs-agent-ask--picks nil nil #'equal)
                                           index label (plist-get (cdr question) :multi)))
           (romacs-agent-ask--show-picks id)
           (when (romacs-agent-ask--sends-at-once-p questions) (romacs-agent-ask--send id)))))))

(defun romacs-agent-ask--step (forward)
  (let* ((here (romacs-agent-ask--here))
         (rows (and here (romacs-agent-ask--rows (car here))))
         (now (line-beginning-position))
         (next (if forward (cl-find-if (lambda (at) (> at now)) rows)
                 (cl-find-if (lambda (at) (< at now)) (reverse rows)))))
    ;; On its words, past its indent and mark.
    (when next (goto-char next) (forward-char 6))))

(defun romacs-agent-ask-next ()
  "The card's next option."
  (interactive nil romacs-agent-chat-mode)
  (romacs-agent-ask--step t))

(defun romacs-agent-ask-previous ()
  "The card's previous option."
  (interactive nil romacs-agent-chat-mode)
  (romacs-agent-ask--step nil))

(provide 'romacs-agent-ask)
;;; romacs-agent-ask.el ends here
