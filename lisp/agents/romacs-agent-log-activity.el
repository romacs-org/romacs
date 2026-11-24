;;; romacs-agent-log-activity.el --- Active workers beside the composer -*- lexical-binding: t; -*-
;; SPDX-License-Identifier: GPL-3.0-or-later
;; Maintainer: Thanos Apollo <public@thanosapollo.org>

;;; Commentary:
;; The live part of the chat is one EWOC, just above its divider.
;; Worker state belongs to the session.  This view only holds its rendered
;; lines and their bounds, so they never become part of a replayed transcript.

;;; Code:

(require 'cl-lib)
(require 'subr-x)
(require 'seq)
(require 'ewoc)
(require 'button)
(require 'tooltip)
(require 'romacs-agent)
(require 'romacs-agent-glyphs)
(require 'romacs-icons)
(require 'romacs-working)

(defface romacs-agent-log-subagent '((t :inherit font-lock-keyword-face :weight bold))
  "The name and animation of a running harness subagent in chat.")

(defface romacs-agent-log-delegated-work '((t :inherit font-lock-type-face))
  "The animation of a thread delegated by Romacs.")

(defface romacs-agent-log-delegated-title
  '((t :inherit default :weight bold))
  "The title that opens a thread delegated by Romacs.")

(defface romacs-agent-log-activity-idle '((t :inherit shadow))
  "The secondary words of a delegated thread's state."
  :group 'romacs-agent)

(defface romacs-agent-log-activity-overflow '((t :inherit (default button) :underline t))
  "The inline link to the remaining delegated threads."
  :group 'romacs-agent)

(defface romacs-agent-log-activity-overflow-focus '((t :inherit default :inverse-video t))
  "The focused link to the remaining delegated threads."
  :group 'romacs-agent)

(defcustom romacs-agent-log-activity-idle-cap 3
  "Maximum number of inactive delegated threads shown above the composer.
The default leaves room for the transcript and work still in progress."
  :type 'natnum :group 'romacs-agent)

(defvar-local romacs-agent-log-activity--view nil
  "The live chat's (EWOC START . END), or nil.")

(defvar-local romacs-agent-log-activity--threads nil
  "The working threads in the live activity, from id to their marker.")

(defvar-local romacs-agent-log-activity--focus-overlay nil
  "The highlight over the overflow button at point.")

(defconst romacs-agent-log-activity--cols 4
  "How many cells a native subagent's animation fills.")

(defconst romacs-agent-log-activity--delegated-states
  '((working chip-working "Working") (approval chip-you "Needs approval") (question chip-you "Asks you")
    (watching watching "Job") (waiting waiting "Workers")
    (idle task-idle "Idle") (completed completed nil) (failed failure "Failed"))
  "The glyph roles and labels for delegated threads.")

(defvar romacs-agent-ui--live-keys)
(defvar romacs-agent-log-activity--ordered (make-hash-table :test #'eq :weakness 'key)
  "Delegated thread groups by the worker row snapshot that produced them.")
(declare-function romacs-agent-log--key "romacs-agent-log" (command &optional map))
(declare-function romacs-agent-log--open-delegated-list "romacs-agent-log" (_button))

(defun romacs-agent-log-activity--thread (row)
  "The id of the delegated session worker ROW is, or nil for a harness's
own subagent: a row as the session's record has it, or as its summary does."
  (or (plist-get row :id) (and-let* ((child (plist-get row :session))) (romacs-agent-id child))))

(defun romacs-agent-log-activity-delegated-rows (rows)
  "Return the live and inactive delegated groups for worker snapshot ROWS."
  (when rows
    (or (gethash rows romacs-agent-log-activity--ordered)
        (puthash rows (romacs-agent-log-activity--order rows) romacs-agent-log-activity--ordered))))

(defun romacs-agent-log-activity--order (rows)
  "Group unsettled delegated ROWS as (LIVE . INACTIVE), newest inactive first."
  (let (live inactive)
    (dolist (row rows)
      (when (and (romacs-agent-log-activity--thread row) (not (eq (plist-get row :key) 'settled)))
        (if (memq (plist-get row :key) romacs-agent-ui--live-keys)
            (push row live)
          (push (plist-put (copy-sequence row) :key (plist-get row :inactive)) inactive))))
    (cons (nreverse live)
          (sort (nreverse inactive) (lambda (a b) (> (or (plist-get a :time) 0) (or (plist-get b :time) 0)))))))

(defun romacs-agent-log-activity--delegated-label (row)
  "Return the short activity label for delegated thread ROW."
  (let ((unit (plist-get row :unit)))
    (if unit (concat "Job · " (string-remove-suffix ".service" unit))
      (nth 2 (assq (plist-get row :key) romacs-agent-log-activity--delegated-states)))))

(defun romacs-agent-log-activity--running-p (row)
  "Whether session worker ROW belongs in the live subagent view."
  (and (plist-get row :subagent) (equal (plist-get row :state) "running")))

(defun romacs-agent-log-activity--before-p (a b)
  "Whether A started before B, using their tool ids to break a tie."
  (let ((x (or (plist-get a :started) 0))
        (y (or (plist-get b :started) 0)))
    (if (= x y) (string< (plist-get a :tool) (plist-get b :tool)) (< x y))))

(defun romacs-agent-log-activity--animation (id width face)
  "The animation in FACE for worker ID when WIDTH has room for it."
  ;; The display overlays working cells at the start of their line.
  (when (> width (+ romacs-agent-log-activity--cols 8))
    (concat (romacs-agent-log-activity--spinner id romacs-agent-log-activity--cols face) "⠀⠀")))

(defun romacs-agent-log-activity--spinner (id cols face)
  "Draw the shared resting frame of worker ID in COLS cells and FACE."
  (propertize (romacs-working-still cols 'running face)
              'romacs-agent-worker id 'romacs-agent-worker-cols cols))

(defun romacs-agent-log-activity--line (title activity prefix width face &optional activity-face)
  "Draw TITLE in FACE and ACTIVITY after PREFIX, within WIDTH, wrapping
under TITLE in a narrower window.
With ACTIVITY-FACE, make room for the inactive state word before the title."
  (let* ((room (max 1 (- width (string-width (or prefix "")) 1)))
         (title-width (if (> room 14) (min 40 (/ room 2)) room))
         (name (romacs-agent-log-activity--clip
                title (if (and activity-face activity)
                          (max 1 (min title-width (- room 5 (string-width activity)))) title-width)))
         (action (- room (string-width name) 5)))
    (concat (propertize (concat prefix (propertize name 'face face)
                                 (when (and activity (> action 2))
                                   (concat (propertize "  ·  " 'face 'romacs-agent-log-note)
                                           (propertize (romacs-agent-log-activity--clip activity action)
                                                       'face (or activity-face 'romacs-agent-log-note)))))
                        'help-echo (if activity (concat title " · " activity) title)
                        'wrap-prefix (make-string (string-width (or prefix "")) ?\s)))))

(defun romacs-agent-log-activity--subagent-line (row width)
  "Draw the native subagent ROW within WIDTH columns."
  (concat (romacs-agent-log-activity--line (or (plist-get row :title) "Subagent")
                                  (or (plist-get row :activity) "Working")
                                  (romacs-agent-log-activity--animation (plist-get row :tool) width
                                                                        'romacs-agent-log-subagent)
                                  width 'romacs-agent-log-subagent)
          "\n"))

(defun romacs-agent-log-activity--slot (key id)
  "Draw KEY's state for thread ID, with a fixed gap before the icon."
  (concat (if (eq key 'working)
              (romacs-agent-log-activity--spinner (cons 'session id) 1 'romacs-agent-log-delegated-work)
            (let ((glyph (romacs-agent-glyph (nth 1 (assq key romacs-agent-log-activity--delegated-states))))
                  (face (pcase key
                          ('failed 'romacs-agent-error)
                          ((or 'watching 'completed) 'default)
                          (_ 'shadow))))
              (propertize glyph 'face face)))
          (propertize " " 'display '(space :align-to 2) 'romacs-grid-align 2)
          " "))

(defun romacs-agent-log-activity-delegated-line (row width)
  "Draw a delegated thread ROW in WIDTH columns, without a newline."
  (let* ((id (romacs-agent-log-activity--thread row))
         (key (plist-get row :key))
         (unit (plist-get row :unit))
         (name (or (plist-get row :title) "Delegated thread"))
         (icon (propertize (or (romacs-icon 'romacs) (romacs-agent-glyph 'romacs)) 'face 'romacs-agent-log-note))
         (prefix (concat (romacs-agent-log-activity--slot key id) icon " "))
         (face (if (memq key romacs-agent-ui--live-keys) 'romacs-agent-log-delegated-title 'default))
         (state-face (if (eq key 'failed) 'romacs-agent-error 'romacs-agent-log-activity-idle))
         (line (romacs-agent-log-activity--line name (romacs-agent-log-activity--delegated-label row)
                                               prefix width face state-face)))
    (when unit
      (add-text-properties
       0 (length line)
       (list 'romacs-agent-log-job-unit unit 'help-echo
             (format "%s · %s (%s copies unit)" name unit (romacs-agent-log--key 'romacs-agent-log-copy-job-unit)))
       line))
    (propertize line 'romacs-agent-log-delegated-thread id)))

(defun romacs-agent-log-activity--clip (text width)
  "A single line of TEXT that fits WIDTH."
  (truncate-string-to-width (replace-regexp-in-string "[[:space:]]+" " " text)
                            width nil nil (truncate-string-ellipsis)))

(defun romacs-agent-log-activity--heading (title count width)
  "Draw TITLE and COUNT within WIDTH when COUNT is positive."
  (when (> count 0)
    (concat (propertize (truncate-string-to-width (format "%s (%d)" title count) (max 1 (1- width)))
                        'face 'romacs-agent-log-note)
            "\n")))

(defun romacs-agent-log-activity--overflow-help (&rest _)
  "Show the overflow action in the echo area."
  (tooltip-show-help-non-mode
   (format "%s opens these threads on the agent board" (romacs-agent-log--key 'push-button 'button-map)))
  nil)

(defun romacs-agent-log-activity-focus ()
  "Highlight the overflow button at point and echo its action."
  (let ((button (button-at (point)))
        (focus romacs-agent-log-activity--focus-overlay))
    (cond ((and button (eq (button-get button 'action) #'romacs-agent-log--open-delegated-list))
           (unless (and focus (eql (overlay-start focus) (button-start button))
                        (eql (overlay-end focus) (button-end button)))
             (when focus (delete-overlay focus))
             (setq romacs-agent-log-activity--focus-overlay
                   (make-overlay (button-start button) (button-end button)))
             (overlay-put romacs-agent-log-activity--focus-overlay 'face 'romacs-agent-log-activity-overflow-focus)
             (overlay-put romacs-agent-log-activity--focus-overlay 'after-string
                          (concat " " (propertize (romacs-agent-log--key 'push-button 'button-map)
                                                  'face '(romacs-agent-log-activity-idle help-key-binding))))
             (romacs-agent-log-activity--overflow-help)))
          (focus
           (delete-overlay focus)
           (setq romacs-agent-log-activity--focus-overlay nil)
           (tooltip-show-help-non-mode nil)))))

(defun romacs-agent-log-activity--delegated-heading (live hidden width)
  "Draw a thread heading with LIVE workers and HIDDEN in overflow."
  (let ((label (if (> live 0) (format "Threads · %d live" live) "Threads"))
        (button (when (> hidden 0)
                  (buttonize (propertize (format "+%d more" hidden) 'face 'romacs-agent-log-activity-overflow)
                             #'romacs-agent-log--open-delegated-list))))
    (concat (truncate-string-to-width
             (concat (propertize label 'face 'romacs-agent-log-note)
                     (when button (concat " " button)))
             (max 1 (1- width)))
            "\n")))

(defun romacs-agent-log-activity--delegated-lines (groups width)
  "Draw delegated thread GROUPS within WIDTH, with overflow in the heading."
  (pcase-let* ((`(,live . ,inactive) groups)
               (shown (seq-take inactive romacs-agent-log-activity-idle-cap))
               (hidden (- (length inactive) (length shown))))
    (when (or live inactive)
      (concat (romacs-agent-log-activity--delegated-heading (length live) hidden width)
              (mapconcat (lambda (row) (concat (romacs-agent-log-activity-delegated-line row width) "\n"))
                         (append live shown) "")))))

(defun romacs-agent-log-activity-lines (rows width)
  "Render unsettled delegated threads and active subagents in ROWS within WIDTH."
  (let ((native (seq-filter #'romacs-agent-log-activity--running-p rows)))
    (concat (romacs-agent-log-activity--delegated-lines (romacs-agent-log-activity-delegated-rows rows) width)
            (romacs-agent-log-activity--heading "Subagents" (length native) width)
            (mapconcat (lambda (row) (romacs-agent-log-activity--subagent-line row width))
                       (sort native #'romacs-agent-log-activity--before-p) ""))))

(defun romacs-agent-log-activity-bounds ()
  "The active view's (START . END) markers, or nil."
  (cdr romacs-agent-log-activity--view))

(defun romacs-agent-log-activity--animate (&optional start end)
  "Attach the working animations to the rows between START and END."
  (let ((old romacs-agent-log-activity--threads) (next nil) (at start))
    (while (and at (setq at (text-property-not-all at end 'romacs-agent-worker nil)))
      (let* ((id (list (current-buffer) (get-text-property at 'romacs-agent-worker)))
             (entry (assoc id old))
             (marker (or (cdr entry) (copy-marker at))))
        (if entry (set-marker marker at)
          (romacs-working-show id (current-buffer) marker (get-text-property at 'romacs-agent-worker-cols)
                               'running (get-text-property at 'face)))
        (push (cons id marker) next)
        (setq at (next-single-property-change at 'romacs-agent-worker nil end))))
    (dolist (entry old)
      (unless (assoc (car entry) next)
        (romacs-working-stop (car entry))
        (set-marker (cdr entry) nil)))
    (setq romacs-agent-log-activity--threads next)))

(defun romacs-agent-log-activity-set (text &optional at)
  "Show sealed TEXT in the live view.  Create it at AT, or clear it if empty."
  (save-restriction
    (widen)
    (with-silent-modifications
      (save-excursion
        (pcase romacs-agent-log-activity--view
          (`(,ewoc ,start . ,end)
           (let ((node (ewoc-nth ewoc 0)))
             (cond ((string-empty-p text)
                    (romacs-agent-log-activity--animate)
                    (ewoc-delete ewoc node)
                    (delete-region (1- start) start)
                    (set-marker start nil)
                    (set-marker end nil)
                    (setq romacs-agent-log-activity--view nil))
                   ((not (equal-including-properties text (ewoc-data node)))
                    (ewoc-set-data node text)
                    (ewoc-invalidate ewoc node)
                    (romacs-agent-log-activity--animate start end)))))
          (_ (unless (string-empty-p text)
               (goto-char at)
               ;; The transcript grows before this newline.  Its insertions
               ;; cannot share the EWOC's first marker.
               (insert (substring text -1))
               (let ((start (point-marker))
                     (end (copy-marker (point) t))
                     (ewoc (ewoc-create #'insert nil nil t)))
                 (ewoc-enter-last ewoc text)
                 (setq romacs-agent-log-activity--view (cons ewoc (cons start end)))
                 (romacs-agent-log-activity--animate start end)))))))))

(provide 'romacs-agent-log-activity)
;;; romacs-agent-log-activity.el ends here
