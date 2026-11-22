;;; romacs-agent-log-attach-tests.el --- A chat's attachments are its composer's links -*- lexical-binding: t; -*-
;; SPDX-License-Identifier: GPL-3.0-or-later
;; Maintainer: Thanos Apollo <public@thanosapollo.org>
(require 'ert)
(require 'cl-lib)
(require 'romacs-mcp)

(let ((romacs-mcp-call-function romacs-mcp-call-function)
      (romacs-mcp-tools (copy-alist romacs-mcp-tools)))
  (require 'romacs-agent)
  (require 'romacs-agent-log))

(defconst romacs-agent-log-attach-tests--png
  (base64-decode-string "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mP8z8BQDwAEhQGAhKmMIQAAAABJRU5ErkJggg==")
  "A PNG of one pixel.")

(defmacro romacs-agent-log-attach-tests--with (&rest body)
  "BODY in a chat whose composer holds nothing, with `a', `b' and `c', three
PNG files in a fresh directory, and `composer' its draft's text."
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
                   romacs-agent-log--end (point-min-marker)
                   romacs-agent-log--input (point-min-marker))
             (set-marker-insertion-type romacs-agent-log--end t)
             (romacs-agent-log--separator t)
             (goto-char (point-max))
             (cl-flet ((composer () (buffer-substring-no-properties romacs-agent-log--input (point-max)))
                       (link (file) (romacs-agent-log-images-link file))
                       (files () (mapcar (lambda (item) (plist-get item :path)) romacs-attach-items)))
               (ignore #'composer #'link #'files)
               ,@body)))
       (delete-directory dir t))))

(ert-deftest romacs-agent-log-attach-links-are-the-chips ()
  (romacs-agent-log-attach-tests--with
    (insert "see " (link a) " and [the second](" b ")")
    (romacs-agent-log-attach-sync)
    (should (equal (files) (list a b)))
    (should (equal (mapcar (lambda (item) (plist-get item :caption)) romacs-attach-items) '(nil "the second")))
    ;; The chips show above the draft's line, its prompt mark included,
    ;; the images not again under it.
    (should (overlay-get romacs-agent-log-attach--overlay 'before-string))
    (should (= (overlay-start romacs-agent-log-attach--overlay)
               (save-excursion (goto-char romacs-agent-log--input) (forward-line 0) (point))))
    (should (< (overlay-start romacs-agent-log-attach--overlay) romacs-agent-log--input))
    ;; The separator written again: the chips stay above its prompt line.
    (romacs-agent-log--separator t)
    (should (< (overlay-start romacs-agent-log-attach--overlay) romacs-agent-log--input))
    (should-not romacs-agent-log-images-under-draft)
    ;; No links, no chips.
    (delete-region romacs-agent-log--input (point-max))
    (romacs-agent-log-attach-sync)
    (should-not romacs-attach-items)))

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
  (romacs-agent-log-attach-tests--with
    (cl-letf (((symbol-function 'romacs-attach-clipboard-image-type) (lambda () "image/png"))
              ((symbol-function 'romacs-attach-clipboard-file) (lambda (_type then) (funcall then a))))
      (insert "look: ")
      (romacs-agent-log-attach-yank)
      (should (equal (composer) (concat "look: " (link a))))
      ;; The change read once the command is done.
      (should romacs-agent-log-attach--dirty)
      (romacs-agent-log-attach-sync)
      (should (equal (files) (list a))))
    ;; No image on the clipboard: a yank, as ever.
    (cl-letf (((symbol-function 'romacs-attach-clipboard-image-type) (lambda () nil)))
      (kill-new " words")
      (romacs-agent-log-attach-yank)
      (should (string-suffix-p " words" (composer))))))

(ert-deftest romacs-agent-log-attach-del-at-the-start-edits-the-chips ()
  (romacs-agent-log-attach-tests--with
    (insert (link a))
    (romacs-agent-log-attach-sync)
    (let ((edited nil))
      (cl-letf (((symbol-function 'romacs-attach-edit) (lambda (&rest _) (setq edited t))))
        (goto-char romacs-agent-log--input)
        (romacs-agent-log-attach-delete-backward)
        (should edited)
        (setq edited nil)
        (goto-char (point-max))
        (romacs-agent-log-attach-delete-backward)
        (should-not edited)
        (should (= (length (composer)) (1- (length (link a)))))))))

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
        (delete-region romacs-agent-log--input (point-max))
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
    (let ((tray (overlay-get romacs-agent-log-attach--overlay 'before-string)))
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
        (goto-char romacs-agent-log--input)
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
    (goto-char (1+ romacs-agent-log--input))
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

(provide 'romacs-agent-log-attach-tests)
;;; romacs-agent-log-attach-tests.el ends here
