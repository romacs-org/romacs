;;; romacs-agent-ask.el --- Questions an agent asks, in a floating layer -*- lexical-binding: t; -*-

;; SPDX-License-Identifier: GPL-3.0-or-later
;; Maintainer: Thanos Apollo <public@thanosapollo.org>

;;; Commentary:
;; A harness's questions (Claude's AskUserQuestion, Codex's user input)
;; arrive as an `approval' event with :questions.  They are answered in a
;; layer over the frame (romacs-popup.el's `palette', as the agent board
;; is): the session's title, each question with its options as rows, a
;; row for words of one's own, and a Send row when one pick does not
;; answer it all.  n and p move between options, RET picks, a digit picks
;; the question's Nth option, q puts the layer away with the question
;; still asked.  The answers go straight back to the agent.
;;
;; The chat (romacs-agent-ui.el) keeps one row for it: the question and
;; that it waits, then what it was answered.  RET on that row, or y,
;; brings the layer back.  The layer shows by itself only when the
;; question reaches a chat in view, never in a replay.
;;
;; Answers are kept as the labels chosen, ((QUESTION . LABELS) ...), and
;; joined only for the harness (`romacs-agent-ask-answers'): a label may
;; hold a comma.

;;; Code:

(require 'cl-lib)
(require 'romacs-popup)

(declare-function romacs-agent-respond "romacs-agent" (session request-id allow &optional message))
(declare-function romacs-agent-title "romacs-agent" (session))
(declare-function romacs-agent-id "romacs-agent" (session))

(defface romacs-agent-ask-option '((t :inherit romacs-popup-description))
  "An option of a question an agent asks.")
(defface romacs-agent-ask-picked '((t :inherit (romacs-popup-key bold)))
  "An option picked.")
(defface romacs-agent-ask-row '((t :inherit highlight :extend t))
  "The option row the layer's keys act on.")

;;; Pure parts

(defun romacs-agent-ask--glyph (multi picked)
  "An option's mark: a box for a question taking several, else a dot."
  (if multi (if picked "☑" "☐") (if picked "●" "○")))

(defun romacs-agent-ask--at-once-p (questions)
  "Whether a pick answers QUESTIONS whole: one question, one choice."
  (and (= (length questions) 1) (not (plist-get (cdar questions) :multi))))

(defun romacs-agent-ask-repick (picks index label multi)
  "PICKS, an alist of question index to labels, with LABEL picked for
question INDEX: added or taken back for a question of several choices,
else the one."
  (let* ((had (cdr (assq index picks)))
         (labels (cond ((not multi) (list label))
                       ((member label had) (remove label had))
                       (t (append had (list label))))))
    (cons (cons index labels) (assq-delete-all index (copy-sequence picks)))))

(defun romacs-agent-ask-chosen (questions picks)
  "QUESTIONS' chosen labels from PICKS, ((QUESTION . LABELS) ...), or nil
while one has none."
  (cl-loop for (question . _) in questions
           for index from 0
           for labels = (cdr (assq index picks))
           unless labels return nil
           collect (cons question labels)))

(defun romacs-agent-ask-answers (chosen)
  "CHOSEN, ((QUESTION . LABELS) ...), as a harness takes answers:
((QUESTION . ANSWER) ...), several labels comma-separated."
  (mapcar (lambda (pair) (cons (car pair) (string-join (cdr pair) ", "))) chosen))

(defun romacs-agent-ask-text (title questions picks)
  "The layer's text for TITLE's QUESTIONS with PICKS so far: a row per
question, its options and an Other row under it, then a Send row when
one pick does not answer it all.  Each choosing row carries
`romacs-agent-ask' (INDEX . WHAT), WHAT a label, `other' or `send'."
  (concat
   (propertize (concat title "\n") 'face 'romacs-popup-title)
   (cl-loop
    for (question . props) in questions
    for index from 0
    for multi = (plist-get props :multi)
    for chosen = (cdr (assq index picks))
    concat (concat
            "\n"
            (if-let* ((header (plist-get props :header)))
                (propertize (concat header "  ") 'face 'romacs-popup-dim)
              "")
            (propertize question 'face 'bold) "\n"
            (cl-loop for label in (plist-get props :options)
                     for n from 1
                     for picked = (member label chosen)
                     concat (propertize
                             (concat (propertize (format " %d " n) 'face 'romacs-popup-key)
                                     (romacs-agent-ask--glyph multi picked) " "
                                     (propertize label 'face (if picked 'romacs-agent-ask-picked 'romacs-agent-ask-option))
                                     (if-let* ((note (cdr (assoc label (plist-get props :notes)))))
                                         (propertize (concat "  " note) 'face 'romacs-popup-dim)
                                       "")
                                     "\n")
                             'romacs-agent-ask (cons index label)))
            (let ((own (cl-set-difference chosen (plist-get props :options) :test #'equal)))
              (propertize (concat "   ✎ " (if own (propertize (string-join own ", ") 'face 'romacs-agent-ask-picked)
                                            (propertize "Other…" 'face 'romacs-popup-dim))
                                  "\n")
                          'romacs-agent-ask (cons index 'other)))))
   (if (romacs-agent-ask--at-once-p questions) ""
     (concat "\n" (propertize (concat (propertize "   ↵ Send" 'face 'romacs-popup-key) "\n")
                              'romacs-agent-ask (cons nil 'send))))))

(defun romacs-agent-ask-record (questions chosen)
  "The chat's one row for QUESTIONS: the first and how many more, then
CHOSEN's answers when it is answered, t when how is not known, else
that it waits."
  (concat "? " (car (split-string (caar questions) "\n"))
          (if (cdr questions) (format " (+%d)" (length (cdr questions))) "")
          (propertize (cond ((consp chosen)
                             (concat "  → " (mapconcat (lambda (pair) (string-join (cdr pair) ", ")) chosen " · ")))
                            (chosen "  · answered")
                            (t "  · waiting: RET answers"))
                      'face 'romacs-agent-dim)))

;;; The layer

(defvar-local romacs-agent-ask--session nil "The session the layer's questions are from.")
(defvar-local romacs-agent-ask--id nil "Their request id.")
(defvar-local romacs-agent-ask--questions nil "The questions, as the approval event gave them.")
(defvar-local romacs-agent-ask--picks nil "The picks so far: question index to labels.")
(defvar-local romacs-agent-ask--row nil "The highlight of the row the keys act on.")

(defvar romacs-agent-ask-mode-map
  (let ((map (make-sparse-keymap)))
    (define-key map "n" #'romacs-agent-ask-next)
    (define-key map "p" #'romacs-agent-ask-previous)
    (define-key map (kbd "C-n") #'romacs-agent-ask-next)
    (define-key map (kbd "C-p") #'romacs-agent-ask-previous)
    (define-key map (kbd "RET") #'romacs-agent-ask-pick)
    (define-key map "q" #'romacs-agent-ask-hide)
    (define-key map (kbd "C-g") #'romacs-agent-ask-hide)
    (dotimes (n 9)
      (define-key map (number-to-string (1+ n)) #'romacs-agent-ask-pick-number))
    map)
  "Keys of the layer answering an agent's questions.")

(define-derived-mode romacs-agent-ask-mode special-mode "Question"
  "Answer an agent's questions: \\<romacs-agent-ask-mode-map>\\[romacs-agent-ask-next] and \\[romacs-agent-ask-previous] move, \\[romacs-agent-ask-pick] picks, a digit picks that option, \\[romacs-agent-ask-hide] puts it away.
\\{romacs-agent-ask-mode-map}"
  :interactive nil
  (setq-local cursor-type nil mode-line-format nil)
  ;; A question or an option longer than the layer is wide wraps: none
  ;; is cut.
  (romacs-layer-wrap-words))

(defun romacs-agent-ask--buffer-name (session id)
  "The layer's buffer for SESSION's request ID: ids are a session's own."
  (format " *agent question %s %s*" (romacs-agent-id session) id))

(defun romacs-agent-ask--here ()
  "The choosing row at point: (INDEX . WHAT), or nil."
  (get-text-property (line-beginning-position) 'romacs-agent-ask))

(defun romacs-agent-ask--follow ()
  "Highlight the row the keys act on."
  (unless (overlayp romacs-agent-ask--row)
    (setq romacs-agent-ask--row (make-overlay (point) (point)))
    (overlay-put romacs-agent-ask--row 'face 'romacs-agent-ask-row))
  (move-overlay romacs-agent-ask--row (line-beginning-position) (line-beginning-position 2)))

(defun romacs-agent-ask--draw ()
  "Draw the layer's text, point on the row it was on, or on the first."
  (let ((here (romacs-agent-ask--here)) (inhibit-read-only t))
    (erase-buffer)
    (insert (romacs-agent-ask-text (romacs-agent-title romacs-agent-ask--session)
                                   romacs-agent-ask--questions romacs-agent-ask--picks))
    (goto-char (point-min))
    (let ((at (and here (text-property-search-forward 'romacs-agent-ask here #'equal))))
      (if at (goto-char (prop-match-beginning at))
        (romacs-agent-ask--step t)))
    (romacs-agent-ask--follow)))

(defun romacs-agent-ask-size (text most)
  "TEXT's layer size, (COLUMNS . LINES): as wide as its longest line, at
most MOST columns, and as tall as its lines wrapped at that width."
  (let* ((widths (mapcar #'string-width (split-string (string-trim-right text "\n") "\n")))
         (columns (min most (+ 2 (apply #'max 1 widths)))))
    (cons columns (apply #'+ (mapcar (lambda (width) (max 1 (ceiling width (float columns)))) widths)))))

(defun romacs-agent-ask--show (buffer)
  "Show BUFFER's layer, sized to its text and the frame, taking keys."
  (let* ((root (romacs-layer--root-frame))
         (size (romacs-agent-ask-size (with-current-buffer buffer (buffer-string))
                                      (min 100 (max 30 (- (frame-width root) 8)))))
         (most (max 6 (- (frame-text-lines root) 6))))
    (romacs-popup-show 'palette buffer :columns (car size) :lines (min most (cdr size)) :max-lines most)
    (romacs-popup-take-keys buffer)))

(defun romacs-agent-ask-open (session id questions)
  "Show the layer asking SESSION's QUESTIONS, request ID, where they are
answered, and return its buffer."
  (let ((buffer (get-buffer-create (romacs-agent-ask--buffer-name session id))))
    (with-current-buffer buffer
      (unless (derived-mode-p 'romacs-agent-ask-mode)
        (romacs-agent-ask-mode)
        (setq romacs-agent-ask--session session romacs-agent-ask--id id
              romacs-agent-ask--questions questions))
      (romacs-agent-ask--draw))
    (when (display-graphic-p) (romacs-agent-ask--show buffer))
    buffer))

(defun romacs-agent-ask-close (session id)
  "Put away and forget the layer of SESSION's request ID, answered."
  (when-let* ((buffer (get-buffer (romacs-agent-ask--buffer-name session id))))
    (romacs-popup-hide buffer)
    (kill-buffer buffer)))

(defun romacs-agent-ask-close-all (session)
  "Put away and forget every layer of SESSION's: its turn is over, or
its chat gone, and nothing waits for their answers."
  (dolist (buffer (buffer-list))
    (when (eq (buffer-local-value 'romacs-agent-ask--session buffer) session)
      (romacs-popup-hide buffer)
      (kill-buffer buffer))))

(defun romacs-agent-ask--step (forward)
  (let ((start (point)) (step (if forward 1 -1)))
    (forward-line step)
    (while (and (not (romacs-agent-ask--here)) (not (if forward (eobp) (bobp))))
      (forward-line step))
    (unless (romacs-agent-ask--here) (goto-char start))))

(defun romacs-agent-ask-next ()
  "The next option."
  (interactive nil romacs-agent-ask-mode)
  (romacs-agent-ask--step t)
  (romacs-agent-ask--follow))

(defun romacs-agent-ask-previous ()
  "The previous option."
  (interactive nil romacs-agent-ask-mode)
  (romacs-agent-ask--step nil)
  (romacs-agent-ask--follow))

(defun romacs-agent-ask-hide ()
  "Put the layer away: the question stays asked, its chat row answers it."
  (interactive nil romacs-agent-ask-mode)
  (romacs-popup-hide (current-buffer)))

(defvar romacs-agent-ask-sent-functions nil
  "Called with a session, a request id and its chosen labels, ((QUESTION
. LABELS) ...), as the layer sends its answers: its chat's row says
them.")

(defun romacs-agent-ask--send ()
  "Send the answers, when each question has one, and put the layer away."
  (let ((chosen (romacs-agent-ask-chosen romacs-agent-ask--questions romacs-agent-ask--picks))
        (session romacs-agent-ask--session) (id romacs-agent-ask--id))
    (unless chosen (user-error "Answer each question first"))
    (run-hook-with-args 'romacs-agent-ask-sent-functions session id chosen)
    (romacs-agent-respond session id (list :answers (romacs-agent-ask-answers chosen)))
    (romacs-agent-ask-close session id)))

(defun romacs-agent-ask--choose (index label)
  "Pick LABEL for question INDEX, and send when that answers it all."
  (let ((question (nth index romacs-agent-ask--questions)))
    (setq romacs-agent-ask--picks
          (romacs-agent-ask-repick romacs-agent-ask--picks index label (plist-get (cdr question) :multi)))
    (if (romacs-agent-ask--at-once-p romacs-agent-ask--questions)
        (romacs-agent-ask--send)
      (romacs-agent-ask--draw))))

(defun romacs-agent-ask-pick ()
  "Pick the option at point, give words of your own for its question, or
send the answers."
  (interactive nil romacs-agent-ask-mode)
  (pcase-let ((`(,index . ,what) (or (romacs-agent-ask--here) (user-error "No option here"))))
    (pcase what
      ('send (romacs-agent-ask--send))
      ('other
       (let ((words (string-trim (read-string (format-prompt (car (nth index romacs-agent-ask--questions)) nil)))))
         (when (string-empty-p words) (user-error "No answer given"))
         (romacs-agent-ask--choose index words)))
      (_ (romacs-agent-ask--choose index what)))))

(defun romacs-agent-ask-pick-number ()
  "Pick the Nth option, N the digit typed, of the question at point."
  (interactive nil romacs-agent-ask-mode)
  (let* ((index (or (car (romacs-agent-ask--here)) 0))
         (n (- (aref (this-command-keys-vector) 0) ?0))
         (label (nth (1- n) (plist-get (cdr (nth index romacs-agent-ask--questions)) :options))))
    (unless label (user-error "No option %d" n))
    (romacs-agent-ask--choose index label)))

(provide 'romacs-agent-ask)
;;; romacs-agent-ask.el ends here
