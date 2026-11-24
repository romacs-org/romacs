;;; romacs-agent-log-attach-tests.el --- A chat's attachments are its composer's links -*- lexical-binding: t; -*-
;; SPDX-License-Identifier: GPL-3.0-or-later
;; Maintainer: Thanos Apollo <public@thanosapollo.org>
(require 'ert)
(require 'cl-lib)
(require 'romacs-mcp)

(let ((romacs-mcp-call-function romacs-mcp-call-function)
      (romacs-mcp-tools (copy-alist romacs-mcp-tools)))
  (require 'romacs-agent)
  (require 'romacs-agent-log)
  (require 'romacs-agent-menu))

(defconst romacs-agent-log-attach-tests--png
  (base64-decode-string "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mP8z8BQDwAEhQGAhKmMIQAAAABJRU5ErkJggg==")
  "A PNG of one pixel.")

(defmacro romacs-agent-log-attach-tests--with (&rest body)
  "BODY in the composer of a chat, `chat', the composer holding nothing,
with `a', `b' and `c', three PNG files in a fresh directory, and
`composer' its draft's text."
  (declare (indent 0))
  `(let* ((dir (file-name-as-directory (make-temp-file "romacs-attach-" t)))
          (a (expand-file-name "a.png" dir)) (b (expand-file-name "b.png" dir)) (c (expand-file-name "c.png" dir)))
     (unwind-protect
         (progn
           (dolist (file (list a b c))
             (let ((coding-system-for-write 'binary))
               (write-region romacs-agent-log-attach-tests--png nil file nil 'silent)))
           (with-temp-buffer
             (romacs-agent-log-mode)
             (setq romacs-agent-log--session (romacs-agent--make :id "A" :harness 'claude :state 'running
                                                                 :tools (make-hash-table :test #'equal)
                                                                 :streamed (make-hash-table :test #'equal))
                   romacs-agent-log--end (point-min-marker))
             (set-marker-insertion-type romacs-agent-log--end t)
             (romacs-agent-log--separator t)
             (let ((chat (current-buffer)))
               (ignore chat)
               (with-current-buffer (romacs-agent-log-composer-buffer chat)
                 (cl-flet ((composer () (buffer-substring-no-properties (point-min) (point-max)))
                           (link (file) (romacs-agent-log-images-link file))
                           (files () (mapcar (lambda (item) (plist-get item :path)) romacs-attach-items)))
                   (ignore #'composer #'link #'files)
                   ,@body)))))
       (delete-directory dir t))))

(defmacro romacs-agent-log-attach-tests--with-images (&rest body)
  "Run BODY with image header reads available in batch."
  (declare (indent 0))
  `(cl-letf (((symbol-function 'display-images-p) #'always)
             ((symbol-function 'romacs-image-header-size)
              (lambda (file) (and (file-exists-p file) '(1 . 1)))))
     ,@body))

(defun romacs-agent-log-attach-tests--shown (files)
  "Assert that FILES are exactly the chips and image overlays in the draft."
  (should (equal (mapcar (lambda (item) (plist-get item :path)) romacs-attach-items) files))
  (should-not (cdr romacs-agent-log-images--composer))
  (let ((overlays (car romacs-agent-log-images--composer)))
    (should (= (length overlays) (length files)))
    (save-excursion
      (goto-char (point-min))
      (cl-mapc (lambda (file overlay)
                 (let ((link (romacs-agent-log-images-link file)))
                   (should (search-forward link nil t))
                   (should (eq (overlay-buffer overlay) (current-buffer)))
                   (should (= (overlay-start overlay) (- (point) (length link))))
                   (should (= (overlay-end overlay) (point)))
                   (should (equal (overlay-get overlay 'romacs-agent-log-image) file))
                   (should (equal (overlay-get overlay 'display)
                                  (concat (romacs-agent-glyph 'image) " " (file-name-nondirectory file))))))
               files overlays))))

(defun romacs-agent-log-attach-tests--key (key)
  "Dispatch KEY through Romacs's command loop."
  (let ((unread-command-events (mapcar (lambda (event) (cons t event)) (listify-key-sequence (kbd key)))))
    (should-not (cdr (romacs--run-host-command)))))

(ert-deftest romacs-agent-log-attach-uses-file-completion-in-the-session-directory ()
  (romacs-agent-log-attach-tests--with
    (setf (romacs-agent-cwd (buffer-local-value 'romacs-agent-log--session chat)) (directory-file-name dir))
    (let ((read-file-name-function #'read-file-name-default)
          (insert-default-directory t)
          (use-dialog-box nil))
      (cl-letf (((symbol-function 'completing-read)
                 (lambda (_prompt table _predicate mustmatch initial &rest _)
                   (should (eq table 'read-file-name-internal))
                   (should (equal initial (abbreviate-file-name dir)))
                   (should romacs-complete-file-preview)
                   (should mustmatch)
                   (file-name-nondirectory a))))
        (romacs-agent-chat-attach t)))
    (romacs-agent-log-attach-sync)
    (should (equal (composer) (link a)))
    (should (equal (files) (list a)))))

(ert-deftest romacs-agent-log-attach-links-are-the-chips ()
  (romacs-agent-log-attach-tests--with
    (insert "see " (link a) " and [the second](" b ")")
    (romacs-agent-log-attach-sync)
    (should (equal (files) (list a b)))
    (should (equal (mapcar (lambda (item) (plist-get item :caption)) romacs-attach-items) '(nil "the second")))
    ;; The chips show at the draft's start, the images not again under it.
    (should (overlay-get romacs-agent-compose--start 'before-string))
    (should (= (overlay-start romacs-agent-compose--start) (point-min)))
    ;; The chat's separator written again leaves them there.
    (with-current-buffer chat (romacs-agent-log--separator t))
    (should (= (overlay-start romacs-agent-compose--start) (point-min)))
    (should-not romacs-agent-log-images-under-draft)
    ;; No links, no chips.
    (delete-region (point-min) (point-max))
    (romacs-agent-log-attach-sync)
    (should-not romacs-attach-items)))

(ert-deftest romacs-agent-log-attach-plain-typing-neither-schedules-nor-scans ()
  (romacs-agent-log-attach-tests--with
    (cl-letf (((symbol-function 'run-at-time)
               (lambda (&rest _) (ert-fail "Plain typing scheduled a timer")))
              ((symbol-function 'romacs-agent-log-images-links)
               (lambda (&rest _) (ert-fail "Plain typing scanned the draft"))))
      (dolist (last-command-event (string-to-list "plain (draft) text"))
        (self-insert-command 1))
      (delete-char -1)
      (should (equal (composer) "plain (draft) tex")))))

(ert-deftest romacs-agent-log-attach-composer-edits-update-chips-and-images ()
  (romacs-agent-log-attach-tests--with
    (let ((scan (symbol-function 'romacs-agent-log-images-links))
          (scans 0) (timer-list nil) (timer-idle-list nil))
      (romacs-agent-log-attach-tests--with-images
        (cl-letf (((symbol-function 'romacs-agent-log-images-links)
                   (lambda (&rest args) (cl-incf scans) (apply scan args))))
          (cl-flet ((finish (expected)
                      (romacs--timer-check)
                      (romacs-agent-log-attach-tests--shown expected)))
            (insert (substring (link a) 0 -1))
            (finish nil)
            (setq scans 0)
            (let ((last-command-event ?\))) (self-insert-command 1))
            (finish (list a))
            (should (= scans 1))
            (insert " " (link b))
            (insert " tail")
            (should (= (length timer-list) 1))
            (finish (list a b))
            ;; The composer keeps undo from its start: the edit undone
            ;; is the deletion alone.
            (undo-boundary)
            (backward-char (length " tail"))
            (delete-char -1)
            (finish (list a))
            (undo-boundary)
            (undo-only)
            (finish (list a b))))))))

(ert-deftest romacs-agent-log-attach-deletion-joins-link-syntax ()
  (romacs-agent-log-attach-tests--with
    (let ((pending nil))
      (cl-letf (((symbol-function 'run-at-time)
                 (lambda (_delay _repeat fn) (push fn pending))))
        (insert "[image] (" a ")")
        (should-not pending)
        (goto-char (+ (point-min) (length "[image]")))
        (delete-char 1)
        (should (= (length pending) 1))
        (funcall (pop pending))
        (should (equal (files) (list a)))
        (delete-region (point-min) (point-max))
        (funcall (pop pending))
        (should-not (files))
        (should-not romacs-agent-log-images--composer)
        (insert "plain again")
        (should-not pending)))))

(ert-deftest romacs-agent-log-attach-fences-hide-and-reveal-links ()
  (romacs-agent-log-attach-tests--with
    (romacs-agent-log-attach-tests--with-images
      (let ((timer-list nil) (timer-idle-list nil))
        (cl-flet ((finish (expected)
                    (romacs--timer-check)
                    (romacs-agent-log-attach-tests--shown expected)))
          (insert (link a) "\n" (link b))
          (finish (list a b))
          (goto-char (point-min))
          (insert "```\n")
          (finish nil)
          (forward-line 1)
          (insert "```\n")
          (finish (list b))
          (goto-char (point-min))
          (insert "x")
          (finish (list a)))))))

(ert-deftest romacs-agent-log-attach-a-yank-scrubs-inherited-properties ()
  (romacs-agent-log-attach-tests--with
    (let ((kill-ring nil) (interprogram-paste-function nil)
          (interprogram-cut-function nil)
          (yank-excluded-properties nil))
      (kill-new (concat "plain " (propertize "styled" 'face 'bold)))
      (yank)
      (should (equal (composer) "plain styled"))
      (should (equal-including-properties
               (buffer-substring (point-min) (point-max))
               "plain styled")))))

(ert-deftest romacs-agent-log-attach-a-timer-yank-settles-without-a-key ()
  (romacs-agent-log-attach-tests--with
    (romacs-agent-log-attach-tests--with-images
      (let ((timer-list nil) (timer-idle-list nil)
            (kill-ring nil) (interprogram-paste-function nil)
            (interprogram-cut-function nil) (this-command nil)
            (this-original-command nil) (chat (current-buffer)))
        (kill-new (link a))
        (run-at-time 0 nil (lambda () (with-current-buffer chat (yank))))
        (romacs--timer-check)
        (romacs--timer-check)
        (should (equal (composer) (link a)))
        (romacs-agent-log-attach-tests--shown (list a))
        (should-not romacs-agent-log-attach--dirty)))))

(ert-deftest romacs-agent-log-attach-a-post-command-edit-settles-without-another-key ()
  (save-window-excursion
    (romacs-agent-log-attach-tests--with
      (romacs-agent-log-attach-tests--with-images
        (switch-to-buffer (current-buffer))
        (let ((timer-list nil) (timer-idle-list nil) (post-command-hook nil))
          (add-hook 'post-command-hook (lambda () (insert (link a))) 95)
          (romacs-agent-log-attach-tests--key "x")
          (should (equal (composer) (concat "x" (link a))))
          (romacs--timer-check)
          (romacs-agent-log-attach-tests--shown (list a))
          (should-not romacs-agent-log-attach--dirty))))))

(ert-deftest romacs-agent-log-attach-a-deactivate-mark-edit-settles-without-another-key ()
  (save-window-excursion
    (romacs-agent-log-attach-tests--with
      (romacs-agent-log-attach-tests--with-images
        (switch-to-buffer (current-buffer))
        (let ((timer-list nil) (timer-idle-list nil)
              (transient-mark-mode t) (select-active-regions nil) (deactivate-mark-hook nil))
          (romacs-agent-log-attach-tests--key "x")
          (set-mark (point-min))
          (keymap-local-set "<f8>" (lambda () (interactive) (setq deactivate-mark t)))
          (add-hook 'deactivate-mark-hook (lambda () (insert (link a))))
          (romacs-agent-log-attach-tests--key "<f8>")
          (should (equal (composer) (concat "x" (link a))))
          (romacs--timer-check)
          (romacs-agent-log-attach-tests--shown (list a))
          (should-not romacs-agent-log-attach--dirty))))))

(ert-deftest romacs-agent-log-attach-key-edits-keep-chips-and-images-in-sync ()
  (save-window-excursion
    (romacs-agent-log-attach-tests--with
      (romacs-agent-log-attach-tests--with-images
        (switch-to-buffer (current-buffer))
        (let ((kill-ring nil) (interprogram-paste-function nil)
              (interprogram-cut-function nil) (timer-list nil) (timer-idle-list nil))
          (buffer-enable-undo)
          (keymap-local-set "<f8>" #'yank)
          (keymap-local-set "<f9>" #'kill-region)
          (keymap-local-set "<f10>" #'undo-only)
          (keymap-local-set "<f11>" #'undo-redo)
          (kill-new (concat "left " (link a)))
          (romacs-agent-log-attach-tests--key "<f8>")
          (romacs--timer-check)
          (let ((id (plist-get (car romacs-attach-items) :id))
                (overlay (caar romacs-agent-log-images--composer)))
            (romacs-agent-log-attach-tests--key "z")
            (romacs--timer-check)
            (should (equal (composer) (concat "left " (link a) "z")))
            (romacs-agent-log-attach-tests--shown (list a))
            (should (equal (plist-get (car romacs-attach-items) :id) id))
            (should (eq (caar romacs-agent-log-images--composer) overlay))
            (should-not (text-properties-at (1- (point)))))
          (kill-new (concat (link b) "right"))
          (romacs-agent-log-attach-tests--key "<f8>")
          (romacs--timer-check)
          (romacs-agent-log-attach-tests--shown (list a b))
          (goto-char (+ (point-min) (length "left ")))
          (set-mark (- (point-max) (length "right")))
          (setq mark-active t)
          (romacs-agent-log-attach-tests--key "<f9>")
          (romacs--timer-check)
          (should (equal (composer) "left right"))
          (romacs-agent-log-attach-tests--shown nil)
          (romacs-agent-log-attach-tests--key "<f10>")
          (romacs--timer-check)
          (should (equal (composer) (concat "left " (link a) "z" (link b) "right")))
          (romacs-agent-log-attach-tests--shown (list a b))
          (romacs-agent-log-attach-tests--key "<f11>")
          (romacs--timer-check)
          (should (equal (composer) "left right"))
          (romacs-agent-log-attach-tests--shown nil))))))

(ert-deftest romacs-agent-log-attach-editing-a-chip-edits-its-link ()
  (romacs-agent-log-attach-tests--with
    (insert "first " (link a) " then " (link b) " last " (link c))
    (romacs-agent-log-attach-sync)
    (let ((ids (mapcar (lambda (item) (plist-get item :id)) romacs-attach-items)))
      ;; Removed: its link goes, with one space beside it.
      (romacs-attach-set-items (romacs-attach-remove romacs-attach-items (nth 1 ids)))
      (should (equal (composer) (concat "first " (link a) " then last " (link c))))
      ;; Moved: the links trade places, the words stay.
      (romacs-attach-set-items (romacs-attach-move romacs-attach-items (nth 0 ids) 1))
      (should (equal (composer) (concat "first " (link c) " then last " (link a))))
      ;; Captioned: the caption is its link's label.
      (romacs-attach-set-items (romacs-attach-caption romacs-attach-items (nth 2 ids) "the plan"))
      (should (string-prefix-p "first [the plan](" (composer)))
      ;; One back (an undone removal): linked at the composer's end.
      (romacs-attach-set-items (append romacs-attach-items (list (romacs-attach-item b))))
      (should (string-suffix-p (concat " " (link b)) (composer)))
      (should (equal (files) (list c a b))))))

(ert-deftest romacs-agent-log-attach-yank-pastes-a-clipboard-image ()
  (save-window-excursion
    (romacs-agent-log-attach-tests--with
      (romacs-agent-log-attach-tests--with-images
        (switch-to-buffer (current-buffer))
        (let ((timer-list nil) (timer-idle-list nil)
              (kill-ring nil) (interprogram-paste-function nil)
              (interprogram-cut-function nil))
          (keymap-local-set "<f8>" #'romacs-agent-log-attach-yank)
          (cl-letf (((symbol-function 'romacs-attach-clipboard-image-type) (lambda () "image/png"))
                    ((symbol-function 'romacs-attach-clipboard-file)
                     (lambda (_type then) (run-at-time 0 nil then a))))
            (insert "look: ")
            (romacs-agent-log-attach-tests--key "<f8>")
            (should romacs-agent-log-attach--paste)
            (should (equal (composer) "look: "))
            (romacs-agent-log-attach-tests--shown nil)
            (romacs--timer-check)
            (romacs--timer-check)
            (should-not romacs-agent-log-attach--paste)
            (should-not romacs-agent-log-attach--dirty)
            (should (equal (composer) (concat "look: " (link a))))
            (romacs-agent-log-attach-tests--shown (list a)))
          (cl-letf (((symbol-function 'romacs-attach-clipboard-image-type) #'ignore))
            (kill-new " words")
            (romacs-agent-log-attach-tests--key "<f8>")
            (romacs--timer-check)
            (should (equal (composer) (concat "look: " (link a) " words")))
            (romacs-agent-log-attach-tests--shown (list a))))))))

(defun romacs-agent-log-attach-tests--del-at-start-is-quiet ()
  "Assert that DEL at the draft start leaves the chat and point intact."
  (should (= (point) (point-min)))
  (should-not romacs-attach-items)
  (should-not (use-region-p))
  (let ((before (buffer-string))
        (start (point)))
    (call-interactively #'romacs-agent-log-attach-delete-backward)
    (should (equal-including-properties (buffer-string) before))
    (should (= (point) start))))

(ert-deftest romacs-agent-log-attach-del-at-the-start-of-an-empty-draft ()
  (romacs-agent-log-attach-tests--with
    (should (equal (composer) ""))
    (romacs-agent-log-attach-tests--del-at-start-is-quiet)))

(ert-deftest romacs-agent-log-attach-del-at-the-start-of-a-nonempty-draft ()
  (romacs-agent-log-attach-tests--with
    (insert "draft")
    (goto-char (point-min))
    (romacs-agent-log-attach-tests--del-at-start-is-quiet)))

(ert-deftest romacs-agent-log-attach-del-at-the-start-after-streaming ()
  (romacs-agent-log-attach-tests--with
    (let ((session (buffer-local-value 'romacs-agent-log--session chat)))
      (with-current-buffer chat (romacs-agent-log-draw session '(:type delta :text "Reply")))
      (insert "draft")
      (goto-char (point-min))
      (with-current-buffer chat (romacs-agent-log-draw session '(:type delta :text " continues")))
      (should (equal (with-current-buffer chat (buffer-substring-no-properties (point-min) romacs-agent-log--end))
                     "Reply continues\n"))
      (should (equal (composer) "draft"))
      (romacs-agent-log-attach-tests--del-at-start-is-quiet))))

(ert-deftest romacs-agent-log-attach-del-at-the-start-edits-the-chips ()
  (romacs-agent-log-attach-tests--with
    (insert (link a))
    (romacs-agent-log-attach-sync)
    (let ((edited nil))
      (cl-letf (((symbol-function 'romacs-attach-edit) (lambda (&rest _) (setq edited t))))
        (goto-char (point-min))
        (romacs-agent-log-attach-delete-backward)
        (should edited)
        (setq edited nil)
        (goto-char (point-max))
        (romacs-agent-log-attach-delete-backward)
        (should-not edited)
        (should (= (length (composer)) (1- (length (link a)))))))))

(ert-deftest romacs-agent-log-attach-del-one-character-into-the-draft ()
  (romacs-agent-log-attach-tests--with
    (insert "xdraft")
    (goto-char (1+ (point-min)))
    (call-interactively #'romacs-agent-log-attach-delete-backward)
    (should (equal (composer) "draft"))
    (should (= (point) (point-min)))))

(ert-deftest romacs-agent-log-attach-del-at-the-start-deletes-the-active-region ()
  (romacs-agent-log-attach-tests--with
    (insert "cutdraft")
    (goto-char (point-min))
    (let ((transient-mark-mode t)
          (select-active-regions nil)
          (delete-active-region t))
      (set-mark (+ (point) 3))
      (should (use-region-p))
      (call-interactively #'romacs-agent-log-attach-delete-backward))
    (should (equal (composer) "draft"))
    (should (= (point) (point-min)))))

(ert-deftest romacs-agent-log-attach-reading-never-writes ()
  ;; A link to a file that is gone has no chip, and stays: neither a read
  ;; nor another chip's removal touches it.
  (romacs-agent-log-attach-tests--with
    (let ((gone "[image](/nonexistent/gone.png)"))
      (insert (link a) " " gone " " (link b))
      (romacs-agent-log-attach-sync)
      (should (equal (files) (list a b)))
      (romacs-agent-log-attach-sync)
      (should (equal (composer) (concat (link a) " " gone " " (link b))))
      (romacs-attach-set-items (romacs-attach-remove romacs-attach-items (plist-get (car romacs-attach-items) :id)))
      (should (equal (composer) (concat gone " " (link b))))
      ;; Linked files are the user's: no chip owns one.
      (should-not (seq-some (lambda (item) (plist-get item :owned)) romacs-attach-items)))))

(ert-deftest romacs-agent-log-attach-a-chip-is-its-own-link ()
  ;; Two links to one file: removing the first chip removes the first
  ;; link, the second keeps its words.
  (romacs-agent-log-attach-tests--with
    (insert "[before](" a ") versus [after](" a ")")
    (romacs-agent-log-attach-sync)
    (romacs-attach-set-items (romacs-attach-remove romacs-attach-items (plist-get (car romacs-attach-items) :id)))
    (should (equal (composer) (concat "versus [after](" a ")")))))

(ert-deftest romacs-agent-log-attach-an-edit-touches-its-link-alone ()
  ;; Captioning one chip leaves another link as written, relative path
  ;; and all; a caption with brackets reads back as written.
  (romacs-agent-log-attach-tests--with
    (let ((default-directory dir))
      (insert "[image](a.png) and " (link b))
      (romacs-agent-log-attach-sync)
      (should (equal (files) (list a b)))
      (romacs-attach-set-items (romacs-attach-caption romacs-attach-items (plist-get (nth 1 romacs-attach-items) :id) "step [2]"))
      (should (equal (composer) (concat "[image](a.png) and [step \\[2\\]](" b ")")))
      (let ((items romacs-attach-items))
        (romacs-agent-log-attach-sync)
        (should (eq items romacs-attach-items))
        (should (equal (plist-get (nth 1 items) :caption) "step [2]"))))))

(ert-deftest romacs-agent-log-attach-a-bracket-before-a-link-stays ()
  ;; An unescaped bracket is no label's: removing the chip leaves it.
  (romacs-agent-log-attach-tests--with
    (insert "array[0 " (link a))
    (romacs-agent-log-attach-sync)
    (romacs-attach-set-items nil)
    (should (equal (composer) "array[0"))))

(ert-deftest romacs-agent-log-attach-one-paste-at-a-time ()
  ;; A paste lands where C-y was typed, one at a time, and nowhere once
  ;; the draft was sent.
  (romacs-agent-log-attach-tests--with
    (let ((pending nil) (asked 0))
      (cl-letf (((symbol-function 'romacs-attach-clipboard-image-type) (lambda () "image/png"))
                ((symbol-function 'romacs-attach-clipboard-file)
                 (lambda (_type then) (setq asked (1+ asked) pending then))))
        (insert "see  now")
        (goto-char (- (point-max) 4))
        (romacs-agent-log-attach-yank)
        (romacs-agent-log-attach-yank)
        (should (= asked 1))
        (goto-char (point-max))
        (insert "!")
        (funcall pending a)
        (should (equal (composer) (concat "see " (link a) " now!")))
        ;; Sent before the image is saved: it lands nowhere.
        (romacs-agent-log-attach-yank)
        (delete-region (point-min) (point-max))
        (run-hooks 'romacs-agent-log-draft-replaced-hook)
        (funcall pending b)
        (should (equal (composer) ""))
        (should (= asked 2))))))

(ert-deftest romacs-agent-log-attach-a-tray-breaks-between-chips ()
  ;; Word wrap breaks at a space: the tray's only spaces are between its
  ;; chips, so a narrow window wraps whole chips, a name with a space too.
  (romacs-agent-log-attach-tests--with
    (let ((spaced (expand-file-name "shot 1.png" dir)))
      (copy-file a spaced)
      (insert (link a) " " (link b) " " (link spaced)))
    (romacs-agent-log-attach-sync)
    ;; The chips, before the prompt's mark the overlay shows after them.
    (let ((tray romacs-agent-log-attach--tray))
      (should (string-prefix-p tray (overlay-get romacs-agent-compose--start 'before-string)))
      (should (= (length (split-string tray "  " t "\n")) 3))
      (should (= (cl-count ?\s tray) 4))
      ;; A chip's size keeps its own face over the chip's.
      (should (seq-some (lambda (i) (equal (get-text-property i 'face tray) '(romacs-attach-badge romacs-attach-chip)))
                        (number-sequence 0 (1- (length tray))))))))

(ert-deftest romacs-agent-log-attach-undo-brings-back-the-link-alone ()
  ;; A removal undone puts its link back as written, where it was; a link
  ;; added meanwhile stays.
  (romacs-agent-log-attach-tests--with
    (insert "first " (link a) " then " (link b) " end")
    (romacs-agent-log-attach-sync)
    (setq romacs-attach--selected (plist-get (nth 1 romacs-attach-items) :id))
    (romacs-attach-edit-remove)
    (should (equal (composer) (concat "first " (link a) " then end")))
    (insert " " (link c))
    (romacs-agent-log-attach-sync)
    (romacs-attach-edit-undo)
    (should (equal (composer) (concat "first " (link a) " then " (link b) " end " (link c))))
    (should (equal (files) (list a b c)))))

(ert-deftest romacs-agent-log-attach-a-new-link-is-a-new-chip ()
  ;; The same file linked again, before the chips read it: removing the
  ;; first chip removes the first link, not the new one.
  (romacs-agent-log-attach-tests--with
    (let ((default-directory dir))
      (insert "[image](a.png)")
      (romacs-agent-log-attach-sync)
      (let ((id (plist-get (car romacs-attach-items) :id)))
        (goto-char (point-min))
        (insert (link a) " ")
        (romacs-attach-set-items (romacs-attach-remove romacs-attach-items id))
        (should (equal (composer) (link a)))))))

(ert-deftest romacs-agent-log-attach-redrawing-writes-nothing ()
  ;; A redraw (a selection) changes no text: a link keeps its spelling;
  ;; a caption of `image' is none; a move trades links as written.
  (romacs-agent-log-attach-tests--with
    (let ((written (concat "[file](" a ") [image](" b ")")))
      (insert written)
      (romacs-agent-log-attach-sync)
      (romacs-attach-set-items romacs-attach-items)
      (should (equal (composer) written))
      (romacs-attach-set-items (romacs-attach-caption romacs-attach-items (plist-get (car romacs-attach-items) :id) "image"))
      (romacs-attach-set-items romacs-attach-items)
      (should (equal (composer) written))
      (should (equal (files) (list a b)))
      (romacs-attach-set-items (romacs-attach-move romacs-attach-items (plist-get (car romacs-attach-items) :id) 1))
      (should (equal (composer) (concat "[image](" b ") [file](" a ")"))))))

(ert-deftest romacs-agent-log-attach-code-is-no-attachment ()
  ;; A link in code is text: no chip, and no chip edit touches it.
  (romacs-agent-log-attach-tests--with
    (insert "printf '`" (link a) "`' and ``x " (link b) " y`` and\n```\n" (link b) "\n```\n" (link c))
    (romacs-agent-log-attach-sync)
    (should (equal (files) (list c)))))

(ert-deftest romacs-agent-log-attach-a-plain-file-links-once ()
  ;; A file that is no image, attached through the tray, is linked once
  ;; and stays a plain link.
  (romacs-agent-log-attach-tests--with
    (let ((notes (expand-file-name "notes.txt" dir)))
      (write-region "x" nil notes nil 'silent)
      (romacs-attach-set-items (list (romacs-attach-item notes)))
      (romacs-attach-set-items romacs-attach-items)
      (should (equal (composer) (link notes)))
      (should-not romacs-attach-items))))

(ert-deftest romacs-agent-log-attach-a-stale-paste-leaves-the-next ()
  ;; A paste sent away, then another: the first's end leaves the second's.
  (romacs-agent-log-attach-tests--with
    (let ((thens nil))
      (cl-letf (((symbol-function 'romacs-attach-clipboard-image-type) (lambda () "image/png"))
                ((symbol-function 'romacs-attach-clipboard-file) (lambda (_type then) (push then thens))))
        (romacs-agent-log-attach-yank)
        (run-hooks 'romacs-agent-log-draft-replaced-hook)
        (romacs-agent-log-attach-yank)
        (funcall (cadr thens) a)
        (should (equal (composer) ""))
        (funcall (car thens) b)
        (should (equal (composer) (link b)))))))

(ert-deftest romacs-agent-log-attach-undo-keeps-the-order ()
  ;; Two neighbours removed, then brought back: each in its place.
  (romacs-agent-log-attach-tests--with
    (insert (link a) " " (link b) " " (link c))
    (romacs-agent-log-attach-sync)
    (let ((ids (mapcar (lambda (item) (plist-get item :id)) romacs-attach-items)))
      (setq romacs-attach--selected (nth 1 ids))
      (romacs-attach-edit-remove)
      (setq romacs-attach--selected (nth 0 ids))
      (romacs-attach-edit-remove)
      (should (equal (composer) (link c)))
      (romacs-attach-edit-undo)
      (romacs-attach-edit-undo)
      (should (equal (composer) (concat (link a) " " (link b) " " (link c)))))))

(ert-deftest romacs-agent-log-attach-a-redraw-keeps-typed-text ()
  ;; A link deleted by hand, the chips not yet read again: a redraw (a
  ;; selection) neither brings it back nor relabels a label typed anew.
  (romacs-agent-log-attach-tests--with
    (insert (link a) " " (link b))
    (romacs-agent-log-attach-sync)
    (delete-region (- (point-max) (length (link b)) 1) (point-max))
    (goto-char (1+ (point-min)))
    (delete-char 5)
    (insert "mine")
    (romacs-attach-set-items romacs-attach-items)
    (should (equal (composer) (concat "[mine" (substring (link a) 6))))))

(ert-deftest romacs-agent-log-attach-a-word-keeps-its-space ()
  ;; A link pasted against a word: removing it leaves the words' space.
  (romacs-agent-log-attach-tests--with
    (insert "foo" (link a) " bar")
    (romacs-agent-log-attach-sync)
    (romacs-attach-set-items nil)
    (should (equal (composer) "foo bar"))))

(ert-deftest romacs-agent-log-attach-a-failing-paste-lets-go ()
  ;; A paste whose link cannot go in lets the next one start; the menu's
  ;; paste, at the composer's end, is spaced from the text.
  (romacs-agent-log-attach-tests--with
    (let ((then nil))
      (cl-letf (((symbol-function 'romacs-attach-clipboard-file) (lambda (_type fn) (setq then fn))))
        (insert "see")
        (should (romacs-agent-log-attach-paste "image/png" (point-max) t))
        (let ((buffer-read-only t)) (should-error (funcall then a)))
        (should-not romacs-agent-log-attach--paste)
        (should (romacs-agent-log-attach-paste "image/png" (point-max) t))
        (funcall then a)
        (should (equal (composer) (concat "see " (link a))))))))

(ert-deftest romacs-agent-log-attach-a-chipless-link-is-no-neighbour ()
  ;; The last chip removed and brought back returns after its neighbour,
  ;; whatever chipless link comes before.
  (romacs-agent-log-attach-tests--with
    (let ((gone "[image](/nonexistent/gone.png)"))
      (insert gone " " (link a) " " (link b))
      (romacs-agent-log-attach-sync)
      (setq romacs-attach--selected (plist-get (nth 1 romacs-attach-items) :id))
      (romacs-attach-edit-remove)
      (romacs-attach-edit-undo)
      (should (equal (composer) (concat gone " " (link a) " " (link b)))))))

(ert-deftest romacs-agent-log-attach-markdown-says-what-is-code ()
  ;; Code as CommonMark reads it: a fence of four holding three, an
  ;; inline span of three; and an escaped bracket opens no link.  The
  ;; same for a sent message's text.
  (romacs-agent-log-attach-tests--with
    (let ((text (concat "````\n```\n" (link a) "\n````\nsee ```x``` here\n" (link b)
                        " and \\" (link c))))
      (insert text)
      (romacs-agent-log-attach-sync)
      (should (equal (files) (list b)))
      (should (equal (mapcar #'caddr (romacs-agent-log-images-links text)) (list b))))))

(ert-deftest romacs-agent-log-attach-a-caption-is-one-line ()
  (romacs-agent-log-attach-tests--with
    (insert (link a))
    (romacs-agent-log-attach-sync)
    (romacs-attach-set-items (romacs-attach-caption romacs-attach-items (plist-get (car romacs-attach-items) :id) "two\nlines"))
    (should (equal (composer) (concat "[two lines](" a ")")))
    (should (equal (files) (list a)))))

(ert-deftest romacs-agent-log-attach-a-link-half-in-code-is-none ()
  ;; A link whose end is in a code span is text: no chip to remove it.
  (romacs-agent-log-attach-tests--with
    (insert "[example `x](" a ")` and " (link b))
    (romacs-agent-log-attach-sync)
    (should (equal (files) (list b)))))

(ert-deftest romacs-agent-log-attach-a-remote-chat-yanks-as-ever ()
  ;; An agent running elsewhere cannot read a local path: C-y yanks.
  (romacs-agent-log-attach-tests--with
    (cl-letf (((symbol-function 'romacs-agent-log--commands-p) #'ignore)
              ((symbol-function 'romacs-attach-clipboard-image-type) (lambda () "image/png"))
              ((symbol-function 'romacs-attach-clipboard-file) (lambda (&rest _) (error "Not here"))))
      (kill-new "words")
      (romacs-agent-log-attach-yank)
      (should (equal (composer) "words")))))

(provide 'romacs-agent-log-attach-tests)
;;; romacs-agent-log-attach-tests.el ends here
