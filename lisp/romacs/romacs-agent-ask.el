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
(defface romacs-agent-ask-row '((t :inherit romacs-layer-selection :extend t))
  "The option row the layer's keys act on: the completion panel's quiet
selection, never the prompt's accent.  The host's band takes its
background.")

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
(defvar-local romacs-agent-ask--columns nil "The layer's width, while it shows.")
(defvar-local romacs-agent-ask--row nil "The face of the row the keys act on.")

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

(defun romacs-agent-ask-row-at (text position columns)
  "The display row the line starting at POSITION is on, in TEXT (a
buffer's, from its start) wrapped in COLUMNS: the rows of the lines
before it."
  (if (<= position 1) 0
    ;; Its lines before the newline that ends the last of them.
    (apply #'+ (mapcar (lambda (line) (romacs-agent-ask--rows-of line columns))
                       (split-string (substring text 0 (- position 2)) "\n")))))

(defun romacs-agent-ask--follow ()
  "Put the layer's selection, as the completion panel's, on the row the
keys act on: its face, and the host's band over it."
  (unless (overlayp romacs-agent-ask--row)
    (setq romacs-agent-ask--row (make-overlay (point) (point)))
    (overlay-put romacs-agent-ask--row 'face 'romacs-agent-ask-row))
  (move-overlay romacs-agent-ask--row (line-beginning-position) (line-beginning-position 2))
  (when-let* ((columns romacs-agent-ask--columns)
              (window (get-buffer-window (current-buffer) t)))
    ;; In view, its row counted from the window's start, as the host's is.
    (unless (pos-visible-in-window-p (line-beginning-position) window)
      (with-selected-window window (recenter (if (< (point) (window-start window)) 0 -1))))
    (let ((row (- (romacs-agent-ask-row-at (buffer-string) (line-beginning-position) columns)
                  (romacs-agent-ask-row-at (buffer-string) (window-start window) columns)))
          (frame (window-frame window)))
      (unless (eq (frame-parameter frame 'romacs-layer-selection) row)
        (set-frame-parameter frame 'romacs-layer-selection row)))))

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

(defun romacs-agent-ask--rows-of (line columns)
  "How many rows LINE takes wrapped at words in COLUMNS, as the layer
wraps it: its indent counts, a row's last column is the wrap's, and a
word too long for a row starts its own and runs on."
  (let* ((room (max 1 (1- columns)))
         (indent (- (length line) (length (string-trim-left line " +"))))
         (rows 1) (used indent) (first t))
    ;; Split at each space: two in a row are an empty word, a column.
    (dolist (word (split-string (substring line indent) " "))
      (let ((width (string-width word)))
        (cond (first (setq used (+ used width) first nil))
              ((<= (+ used 1 width) room) (setq used (+ used 1 width)))
              (t (setq rows (1+ rows) used width)))
        ;; A word wider than a row takes rows of its own.
        (while (> used room)
          (setq rows (1+ rows) used (- used room)))))
    rows))

(defun romacs-agent-ask-size (text most)
  "TEXT's layer size, (COLUMNS . LINES): as wide as its longest line, at
most MOST columns, and as tall as its lines wrapped at words there."
  (let* ((lines (split-string (string-trim-right text "\n") "\n"))
         (columns (min most (+ 2 (apply #'max 1 (mapcar #'string-width lines))))))
    (cons columns (apply #'+ (mapcar (lambda (line) (romacs-agent-ask--rows-of line columns)) lines)))))

(defun romacs-agent-ask-place (row-top row-height height frame-height)
  "Where the layer goes, (TOP . ROOM) in pixels, clear of the question's
row at ROW-TOP, ROW-HEIGHT tall: below it when HEIGHT fits there in
FRAME-HEIGHT, else above it when it fits there, else on the side with
more room; ROOM is the most it may be tall there.  Nil when the row is
not in view."
  (when row-top
    (let* ((below-top (+ row-top (* 2 row-height)))
           (below (- frame-height below-top))
           (above (- row-top row-height)))
      (cond ((<= height below) (cons below-top below))
            ((<= height above) (cons (- above height) above))
            ((>= below above) (cons below-top below))
            (t (cons 0 above))))))

(defun romacs-agent-ask-fit (lines line placed)
  "LINES rows of LINE pixels in PLACED, a place (TOP . ROOM)
\(`romacs-agent-ask-place'), with its 24 pixels of padding: (TOP . LINES)
as many as the room holds; nil when it holds not one row, and the layer
goes where layers go."
  (when placed
    (let ((most (/ (- (cdr placed) 24) line)))
      (and (>= most 1) (cons (car placed) (min lines most))))))

(defun romacs-agent-ask--row-top (place)
  "The top of PLACE's row, (WINDOW . POSITION), in its frame's pixels,
or nil when it is not in view."
  (when-let* ((window (car place)) ((window-live-p window))
              (seen (pos-visible-in-window-p (cdr place) window t)))
    (+ (nth 1 (window-inside-pixel-edges window)) (nth 1 seen))))

(defun romacs-agent-ask--show (buffer &optional place)
  "Show BUFFER's layer, sized to its text and the frame, taking keys,
clear of PLACE, the question's row in its chat, (WINDOW . POSITION)."
  (let* ((root (romacs-layer--root-frame))
         (size (romacs-agent-ask-size (with-current-buffer buffer (buffer-string))
                                      (min 100 (max 30 (- (frame-width root) 8)))))
         (most (max 6 (- (frame-text-lines root) 6)))
         (lines (min most (cdr size)))
         (line (frame-char-height root))
         ;; No taller than its side has room for: it scrolls instead.
         (fit (romacs-agent-ask-fit lines line
                                    (romacs-agent-ask-place (and place (romacs-agent-ask--row-top place)) line
                                                            (+ (* lines line) 24) (frame-text-height root))))
         (top (car fit))
         (lines (if fit (cdr fit) lines)))
    (with-current-buffer buffer (setq romacs-agent-ask--columns (car size)))
    (romacs-popup-show 'palette buffer :columns (car size) :lines lines :max-lines most
                       :anchor (and top (cons (+ (frame-internal-border-width root)
                                                 (max 0 (/ (- (frame-text-width root)
                                                              (* (frame-char-width root) (car size)))
                                                           2)))
                                              top)))
    (with-current-buffer buffer (romacs-agent-ask--follow))
    (romacs-popup-take-keys buffer)))

(defun romacs-agent-ask-open (session id questions &optional place)
  "Show the layer asking SESSION's QUESTIONS, request ID, where they are
answered, clear of PLACE, their row in the chat, (WINDOW . POSITION),
and return its buffer."
  (let ((buffer (get-buffer-create (romacs-agent-ask--buffer-name session id))))
    (with-current-buffer buffer
      (unless (derived-mode-p 'romacs-agent-ask-mode)
        (romacs-agent-ask-mode)
        (setq romacs-agent-ask--session session romacs-agent-ask--id id
              romacs-agent-ask--questions questions))
      (romacs-agent-ask--draw))
    (when (display-graphic-p) (romacs-agent-ask--show buffer place))
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
