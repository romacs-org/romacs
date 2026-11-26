;;; romacs-agent-log-width-tests.el --- Width-independent chat text -*- lexical-binding: t; -*-
;; SPDX-License-Identifier: GPL-3.0-or-later
;; Maintainer: Thanos Apollo <public@thanosapollo.org>

(require 'ert)
(require 'cl-lib)

;; Import the chat fixture without selecting the other file's tests.
(let ((load-path (cons (file-name-directory load-file-name) load-path)))
  (cl-letf (((symbol-function 'ert-set-test) #'ignore))
    (require 'romacs-agent-log-tests)))

(defmacro romacs-agent-log-width-tests--shown (&rest body)
  "Run BODY in the visible chat of the registered test session."
  (declare (indent 0))
  `(romacs-agent-log-tests--with
     (ignore #'draw)
     (let ((romacs-agent--sessions (make-hash-table :test #'equal))
           (romacs-agent-log--chats (make-hash-table :test #'equal))
           (romacs-agent-log--ticker nil)
           (chat (current-buffer))
           (frame (romacs--create-frame
                   `((parent-frame . ,(selected-frame)) (width . 80)
                     (height . 15) (left-fringe . 0) (right-fringe . 0)))))
       (puthash (romacs-agent-id session) session romacs-agent--sessions)
       (puthash (romacs-agent-id session) (current-buffer) romacs-agent-log--chats)
       (setq romacs-agent-log--filled t)
       (unwind-protect
           (with-selected-frame frame
             (switch-to-buffer chat)
             ,@body)
         (delete-frame frame)
         (when (timerp romacs-agent-log--ticker)
           (cancel-timer romacs-agent-log--ticker))))))

(defun romacs-agent-log-width-tests--snapshot (widths)
  "Render the chat in windows with body WIDTHS and copy all text."
  (let* ((left (selected-window))
         (split (cdr widths))
         (cols (if split (+ 1 (apply #'+ widths)) (car widths)))
         (chat (current-buffer)))
    (set-frame-width (selected-frame) cols)
    (let ((right (when split (split-window-right (1+ (car widths))))))
      (unwind-protect
          (progn
            (when right (set-window-buffer right chat))
            (should (equal (mapcar #'window-body-width (if right (list left right) (list left)))
                           widths))
            (romacs-agent-log--separator t)
            (romacs-agent-log--activity-refresh)
            (buffer-string))
        (when right (delete-window right))))))

(ert-deftest romacs-agent-log-width-text-and-properties-match-across-windows ()
  (romacs-agent-log-width-tests--shown
    (let* ((title (concat "Long worker title " (make-string 160 ?w)))
           (task (romacs-agent--make :id "width-worker" :title title :state 'running))
           (rows (list (list :session task :title title :key 'working)))
           (queued (romacs-agent-item (concat "Queued prompt " (make-string 230 ?q)) :kind 'next)))
      (setf (romacs-agent-queue session) (list queued))
      (cl-letf (((symbol-function 'romacs-agent-children-rows) (lambda (_) rows)))
        (draw :type 'input :text "Prompt in the transcript")
        (let* ((layouts '((40) (80) (200) (200 40)))
               (snapshots (mapcar #'romacs-agent-log-width-tests--snapshot layouts)))
          (dolist (text snapshots)
            (should (string-search "Prompt in the transcript" text))
            (should (text-property-any 0 (length text) 'romacs-agent-log-delegated-thread
                                       (romacs-agent-id task) text))
            (should (text-property-any 0 (length text) 'romacs-agent-log-queued queued text)))
          (cl-mapc (lambda (layout text)
                     (ert-info ((format "Window widths %S versus (40)" layout))
                       (should (equal-including-properties (car snapshots) text))))
                   (cdr layouts) (cdr snapshots)))))))

(ert-deftest romacs-agent-log-width-live-deltas-do-not-read-window-geometry ()
  (romacs-agent-log-width-tests--shown
    (let* ((width-reads 0) (window-searches 0)
          (count-width (lambda (&rest _) (cl-incf width-reads)))
          (count-windows (lambda (&rest _) (cl-incf window-searches))))
      (unwind-protect
          (progn
            (advice-add 'window-body-width :before count-width)
            (advice-add 'get-buffer-window-list :before count-windows)
            (dotimes (i 200)
              (romacs-agent-log--on-event
               session (list :type 'delta :text "width sample " :seq (1+ i) :last t))))
        (advice-remove 'window-body-width count-width)
        (advice-remove 'get-buffer-window-list count-windows))
      (should (= romacs-agent-log--seq 200))
      (should (= (how-many "width sample" (point-min) romacs-agent-log--end) 200))
      (should (equal (list width-reads window-searches) '(0 0))))))

(defun romacs-agent-log-width-tests--geometry-reads (events)
  "Draw EVENTS in the chat; return the window geometry they read."
  (let* ((reads nil)
         (advices (mapcar (lambda (name) (cons name (lambda (&rest _) (push name reads))))
                          '(window-body-width get-buffer-window-list get-buffer-window))))
    (unwind-protect
        (progn
          (pcase-dolist (`(,name . ,count) advices) (advice-add name :before count))
          (dolist (event events) (romacs-agent-log--on-event romacs-agent-log--session event)))
      (pcase-dolist (`(,name . ,count) advices) (advice-remove name count)))
    reads))

(ert-deftest romacs-agent-log-width-a-prompt-without-an-image-reads-no-window-geometry ()
  (romacs-agent-log-width-tests--shown
    (should (equal (romacs-agent-log-width-tests--geometry-reads
                    '((:type input :text "a plain prompt" :seq 1 :last t)))
                   nil))))

(ert-deftest romacs-agent-log-width-a-reply-without-a-table-reads-no-window-geometry ()
  (romacs-agent-log-width-tests--shown
    (should (equal (romacs-agent-log-width-tests--geometry-reads
                    '((:type delta :text "pick a | b" :message "m" :seq 1 :last t)
                      (:type message-end :message "m" :seq 2 :last t)))
                   nil))))

(ert-deftest romacs-agent-log-width-resize-does-not-rewrite-chat ()
  (romacs-agent-log-width-tests--shown
    (draw :type 'input :text "A transcript without tables or images")
    (let* ((left (selected-window))
           (right (split-window-right))
           (other (generate-new-buffer " *agent width other*"))
           (sizes nil)
           (record-size (lambda (window) (when (eq window left) (push (window-body-width window) sizes)))))
      (unwind-protect
          (progn
            (set-window-buffer right other)
            (romacs--run-window-change-functions)
            (add-hook 'window-size-change-functions record-size nil t)
            (let ((width (window-body-width left))
                  (tick (buffer-modified-tick)))
              (window-resize left -10 t)
              (romacs--run-window-change-functions)
              (should (< (window-body-width left) width))
              (should (equal sizes (list (window-body-width left))))
              (should (= (buffer-modified-tick) tick))))
        (remove-hook 'window-size-change-functions record-size t)
        (kill-buffer other)))))

(defun romacs-agent-log-width-tests--divider ()
  "Return the divider line with its properties."
  (save-excursion
    (goto-char romacs-agent-log--divider)
    (buffer-substring (point) (pos-eol))))

(ert-deftest romacs-agent-log-width-divider-rule-is-its-newline-s-extended-face ()
  (romacs-agent-log-width-tests--shown
    (let* ((divider (romacs-agent-log-width-tests--divider))
           (newline (save-excursion (goto-char romacs-agent-log--divider) (pos-eol)))
           (faces (ensure-list (get-text-property newline 'face))))
      (should (eq (char-after newline) ?\n))
      (should-not (string-match-p "──" divider))
      (should-not (text-property-not-all romacs-agent-log--divider (1+ newline) 'display nil))
      ;; What a window extends is drawn in the extending faces alone:
      ;; one of them draws the line, in the rule's colour.
      (should (cl-some (lambda (face)
                         (and (facep face) (face-attribute face :extend nil t)
                              (cl-some (lambda (line) (face-attribute face line nil t))
                                       '(:strike-through :underline :overline))
                              (equal (face-attribute face :foreground nil t)
                                     (face-attribute 'romacs-agent-log-rule :foreground nil t))))
                       faces)))))

(defun romacs-agent-log-width-tests--check-state (work label face)
  "Check the divider's WORK glyph and its optional LABEL in FACE."
  (let* ((divider (romacs-agent-log-width-tests--divider))
         (glyph (romacs-working-still romacs-agent-log--thread-cols work)))
    (should (string-prefix-p glyph divider))
    (should (eq (get-text-property 0 'face divider) (get-text-property 0 'face glyph)))
    (if label
        (let ((start (string-search label divider)))
          (should start)
          (should (memq face (ensure-list (get-text-property start 'face divider)))))
      (should-not (string-match-p "[[:alpha:]]" divider)))))

(ert-deftest romacs-agent-log-width-divider-keeps-labels-faces-and-thread ()
  (romacs-agent-log-width-tests--shown
    (romacs-agent-log-width-tests--check-state 'running nil nil)
    (setf (romacs-agent-state session) 'idle)
    (romacs-agent-log--separator)
    (romacs-agent-log-width-tests--check-state 'idle nil nil)
    (setf (romacs-agent-failed session) t)
    (romacs-agent-log--separator)
    (romacs-agent-log-width-tests--check-state 'failed "failed" 'romacs-agent-log-denied)
    (puthash "width-ask" t (romacs-agent-approvals session))
    (draw :type 'approval :request-id "width-ask" :tool "Bash"
          :input (romacs-agent-log-tests--input "command" "true"))
    (romacs-agent-log-width-tests--check-state 'waiting "waits on you" 'romacs-agent-log-ask)))

(ert-deftest romacs-agent-log-width-divider-keeps-point-on-its-queued-row ()
  (romacs-agent-log-width-tests--shown
    (let ((first (romacs-agent-item "First prompt" :kind 'next))
          (second (romacs-agent-item "Second prompt" :kind 'next)))
      (setf (romacs-agent-queue session) (list first second))
      (romacs-agent-log--separator)
      (goto-char romacs-agent-log--end)
      (romacs-agent-log-queue-next)
      (should (eq (get-text-property (point) 'romacs-agent-log-queued) first))
      (romacs-agent-log-queue-next)
      (should (eq (get-text-property (point) 'romacs-agent-log-queued) second))
      (setf (romacs-agent-state session) 'idle (romacs-agent-failed session) t)
      (romacs-agent-log--separator)
      (should (eq (get-text-property (point) 'romacs-agent-log-queued) second))
      (should (looking-at-p ".*Second prompt")))))

(provide 'romacs-agent-log-width-tests)
;;; romacs-agent-log-width-tests.el ends here
