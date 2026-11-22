;;; romacs-agent-log-attach.el --- A chat's attachments, its composer's image links as chips -*- lexical-binding: t; -*-
;; SPDX-License-Identifier: GPL-3.0-or-later
;; Maintainer: Thanos Apollo <public@thanosapollo.org>

;;; Commentary:

;; An attachment is a Markdown link to its file in the composer,
;; [image](PATH): the message carries it, the chat shows it inline, and
;; nothing is copied.  The links are the data.  romacs-attach.el's tray
;; shows them as chips above the composer and edits them.
;;
;; A link's text carries its chip's id as a text property, so the two
;; stay one while the text around them changes; a link new to the draft
;; (typed, yanked, undone back: the composer makes what comes in plain)
;; has none and gets a chip of its own.  Reading the
;; draft never writes it: a change to the composer reads its links once
;; the command or timer is done.  A chip edit writes back only what it
;; changed: a chip removed deletes its own link (undone, the link comes
;; back as written, where it was), a caption relabels its link's label
;; alone, a move trades links as written.  Then the draft is read again,
;; so the chips are always what its links say.  Links in code (`...`,
;; fenced blocks) are text, not attachments.
;;
;; DEL at the composer's start enters the chips (n/p, d, M-f/M-b, c,
;; RET, q).  C-y with an image on the clipboard saves it once and links
;; it where C-y was typed.

;;; Code:

(require 'cl-lib)
(require 'seq)
(require 'romacs-attach)
(require 'romacs-agent-log-images)

(defvar romacs-agent-log--input)
(defvar romacs-agent-log-draft-replaced-hook)

(romacs-attach-define-consumer 'agent-chat
  :display "The chat" :rules '(("\\`image/" image) ("" path)))

(defvar-local romacs-agent-log-attach--overlay nil
  "The overlay that shows the tray at the composer's start.")

(defvar-local romacs-agent-log-attach--shown nil
  "The tray's items as last read from the draft, in order: (ID . CAPTION)
each, what a chip edit is measured against.")

(defvar-local romacs-agent-log-attach--unlinked nil
  "Links chip removals took, newest first: (ID MARKER TEXT SPACE), MARKER
where the link was, TEXT the link as written, SPACE the side, `before' or
`after', of the space that went with it.")

(defvar-local romacs-agent-log-attach--dirty nil
  "The composer changed since its links were read; a read is scheduled.")

(defvar-local romacs-agent-log-attach--paste nil
  "The clipboard paste under way: the marker where C-y was typed.")

(defvar romacs-agent-log-attach--reading nil
  "Non-nil while the tray takes the draft's links: nothing to write back.")

(defconst romacs-agent-log-attach--id 'romacs-agent-log-attach
  "The text property a link carries: its chip's item id.")

;;;; Links

(defun romacs-agent-log-attach--unescape (label)
  "LABEL's text, its backslash escapes undone."
  (replace-regexp-in-string "\\\\\\(.\\)" "\\1" label t))

(defun romacs-agent-log-attach--escape (caption)
  "CAPTION as a link's label: on one line, its brackets and backslashes
escaped."
  (replace-regexp-in-string "[][\\]" "\\\\\\&" (replace-regexp-in-string "[\n\r]+" " " caption) t))

(defun romacs-agent-log-attach--caption (label)
  "The caption a link's LABEL says: none for the plain `image' or `file'."
  (and label (not (member label '("" "image" "file"))) label))

(defun romacs-agent-log-attach--label (file caption)
  "The label of a link to FILE captioned CAPTION: its default without one."
  (if-let* ((caption (romacs-agent-log-attach--caption caption)))
      (romacs-agent-log-attach--escape caption)
    (let ((link (romacs-agent-log-images-link file)))
      (substring link 1 (string-search "]" link)))))

(defun romacs-agent-log-attach--links ()
  "The composer's image links (code holds none), in order:
\(BEG END FILE CAPTION LABEL-BEG LABEL-END ID) each, ID the chip id its
text carries."
  (mapcar (pcase-lambda (`(,beg ,end ,file))
            (save-excursion
              (goto-char beg)
              (looking-at romacs-agent-log-images-label-regexp)
              (list beg end file
                    (romacs-agent-log-attach--caption
                     (romacs-agent-log-attach--unescape (match-string-no-properties 1)))
                    (match-beginning 1) (match-end 1)
                    (get-text-property beg romacs-agent-log-attach--id))))
          (romacs-agent-log-images-links nil (marker-position romacs-agent-log--input) (point-max))))

;;;; Reading: the draft's links as chips

(defun romacs-agent-log-attach-sync ()
  "Show the composer's links as the tray's chips.  The draft's text is
never changed here; a link without a chip id gets its chip's."
  (setq romacs-agent-log-attach--dirty nil)
  (when (markerp romacs-agent-log--input)
    (let ((old (mapcar (lambda (item) (cons (plist-get item :id) item)) romacs-attach-items))
          (seen nil) (items nil))
      (pcase-dolist (`(,beg ,end ,file ,caption ,_ ,_ ,id) (romacs-agent-log-attach--links))
        (let* ((kept (and id (not (member id seen)) (cdr (assoc id old))))
               (item (if (and kept (equal (plist-get kept :path) file))
                         (if (equal (plist-get kept :caption) caption) kept
                           (plist-put (copy-sequence kept) :caption caption))
                       (ignore-errors (romacs-attach-item file :caption caption :source 'link)))))
          (when item
            (push (plist-get item :id) seen)
            (push item items)
            (unless (equal id (plist-get item :id))
              (romacs-agent-log-attach--mark beg end (plist-get item :id))))))
      (setq items (nreverse items)
            romacs-agent-log-attach--shown (mapcar (lambda (item) (cons (plist-get item :id) (plist-get item :caption))) items))
      (unless (equal items romacs-attach-items)
        (let ((romacs-agent-log-attach--reading t))
          (romacs-attach-set-items items))))))

(defun romacs-agent-log-attach--changed (beg _end _length)
  "After a change from BEG: in the composer, its links read again once
the current command or timer is done."
  (when (and (markerp romacs-agent-log--input) (>= beg romacs-agent-log--input)
             (not romacs-agent-log-attach--dirty))
    (setq romacs-agent-log-attach--dirty t)
    (let ((chat (current-buffer)))
      (run-at-time 0 nil (lambda ()
                           (when (buffer-live-p chat)
                             (with-current-buffer chat
                               (when romacs-agent-log-attach--dirty (romacs-agent-log-attach-sync)))))))))

;;;; Writing: a chip edit to its links

(defun romacs-agent-log-attach--mark (beg end id)
  "Give the link from BEG to END the chip ID: the composer makes what is
inserted into it plain (`romacs-agent-log--composer-plain')."
  (with-silent-modifications (put-text-property beg end romacs-agent-log-attach--id id)))

(defun romacs-agent-log-attach--replace (beg end text &optional id)
  "Make BEG..END TEXT, the link of chip ID when given: inserted before,
then the old deleted, so a marker at END stays at the end."
  (goto-char beg)
  (insert text)
  (delete-region (point) (+ (point) (- end beg)))
  (when id (romacs-agent-log-attach--mark beg (point) id)))

(defun romacs-agent-log-attach--delete-link (beg end)
  "Delete the link from BEG to END, and one space beside it when the link
stood apart, spaces or the draft's edge on both sides; which side's,
`before' or `after', or nil.  A word touching the link keeps its space."
  (goto-char beg)
  (delete-region beg end)
  ;; The prompt's own space, before the composer, is the draft's edge.
  (let ((start (= (point) romacs-agent-log--input)))
    (cond ((and (not start) (eq (char-before) ?\s) (memq (char-after) '(?\s ?\n nil)))
           (delete-char -1) 'before)
          ((and (or start (eq (char-before) ?\n)) (eq (char-after) ?\s))
           (delete-char 1) 'after))))

(defun romacs-agent-log-attach--neighbours (id)
  "Where the links of chip ID's neighbours in the tray are: (PREV-END
. NEXT-BEG), either nil without one."
  (let* ((ids (mapcar (lambda (item) (plist-get item :id)) romacs-attach-items))
         (i (seq-position ids id))
         (links (romacs-agent-log-attach--links))
         ;; A link without a chip (nil id) is no neighbour.
         (at (lambda (id) (and id (seq-find (lambda (link) (equal (nth 6 link) id)) links))))
         (prev (and i (> i 0) (funcall at (nth (1- i) ids))))
         (next (and i (funcall at (nth (1+ i) ids)))))
    (cons (nth 1 prev) (nth 0 next))))

(defun romacs-agent-log-attach--relink (item)
  "Put ITEM's link back: as written, where it was when a removal took it,
kept between its neighbours' links; a chip new to the tray, linked anew
at the composer's end."
  (let* ((id (plist-get item :id))
         (gone (assoc id romacs-agent-log-attach--unlinked)))
    (setq romacs-agent-log-attach--unlinked (delq gone romacs-agent-log-attach--unlinked))
    (if (not gone)
        (let ((link (romacs-agent-log-images-link (plist-get item :path))))
          (goto-char (point-max))
          (unless (or (= (point) romacs-agent-log--input) (memq (char-before) '(?\s ?\n))) (insert " "))
          (let ((beg (point)))
            (insert "[" (romacs-agent-log-attach--label (plist-get item :path) (plist-get item :caption))
                    (substring link (string-search "]" link)))
            (romacs-agent-log-attach--mark beg (point) id)))
      (pcase-let* ((`(,_ ,marker ,text ,space) gone)
                   (`(,prev . ,next) (romacs-agent-log-attach--neighbours id))
                   (at (or (and (marker-position marker) (>= marker romacs-agent-log--input) (marker-position marker))
                           (point-max))))
        (set-marker marker nil)
        ;; Two removals at one place leave their markers together: the
        ;; neighbours put each back in its order.
        (cond ((and prev (< at prev)) (setq at prev space 'before))
              ((and next (> at next)) (setq at next space 'after)))
        (goto-char at)
        (when (eq space 'before) (insert " "))
        (let ((beg (point)))
          (insert text)
          (romacs-agent-log-attach--mark beg (point) id))
        (when (eq space 'after) (insert " "))))))

(defun romacs-agent-log-attach--write ()
  "Write the tray's edit, measured against the draft as last read, to the
links it touches, each alone; then read the draft again.  A redraw that
changes no chip writes nothing, and text typed since the last read is
never undone by it."
  (let* ((shown romacs-agent-log-attach--shown)
         (ids (mapcar (lambda (item) (plist-get item :id)) romacs-attach-items))
         (seen nil)
         (linked (seq-filter (lambda (link)
                               (let ((id (nth 6 link)))
                                 (and (assoc id shown) (not (member id seen)) (push id seen))))
                             (romacs-agent-log-attach--links)))
         (staying (seq-filter (lambda (link) (member (nth 6 link) ids)) linked))
         (order (seq-filter (lambda (id) (seq-find (lambda (link) (equal (nth 6 link) id)) staying)) ids))
         (texts (mapcar (lambda (link) (cons (nth 6 link) (buffer-substring (nth 0 link) (nth 1 link)))) staying)))
    (save-excursion
      ;; From the last link, so the places of those before stay.
      (dolist (link (reverse linked))
        (pcase-let ((`(,beg ,end ,file ,_ ,label-beg ,label-end ,id) link))
          (if (not (member id ids))
              (let ((text (buffer-substring beg end)) (space (romacs-agent-log-attach--delete-link beg end)))
                (push (list id (copy-marker (point)) text space) romacs-agent-log-attach--unlinked))
            (let* ((want (nth (seq-position staying link) order))
                   (item (seq-find (lambda (item) (equal (plist-get item :id) want)) romacs-attach-items))
                   (caption (romacs-agent-log-attach--caption (plist-get item :caption))))
              (cond ((not (equal want id))
                     (romacs-agent-log-attach--replace beg end (cdr (assoc want texts)) want))
                    ((not (equal caption (romacs-agent-log-attach--caption (cdr (assoc id shown)))))
                     (romacs-agent-log-attach--replace label-beg label-end
                                                       (romacs-agent-log-attach--label file caption))))))))
      (dolist (item romacs-attach-items)
        (unless (assoc (plist-get item :id) shown)
          (romacs-agent-log-attach--relink item))))
    (romacs-agent-log-attach-sync)))

;;;; The tray

(defun romacs-agent-log-attach--redisplay ()
  "The tray changed: a chip edit written to its links, the chips shown
at the composer's start."
  (unless romacs-agent-log-attach--reading (romacs-agent-log-attach--write))
  (unless (and romacs-agent-log-attach--overlay (overlay-buffer romacs-agent-log-attach--overlay))
    (setq romacs-agent-log-attach--overlay (make-overlay 1 1 nil nil nil)))
  (overlay-put romacs-agent-log-attach--overlay 'before-string (romacs-attach-tray-string))
  (romacs-agent-log-attach-place))

(defun romacs-agent-log-attach-place ()
  "Keep the chips above the composer's line, its prompt mark included:
the separator, written again, takes that line's start with it."
  (when (and romacs-agent-log-attach--overlay (overlay-buffer romacs-agent-log-attach--overlay))
    ;; Not `line-beginning-position': the separator is a field of its own.
    (let ((line (save-excursion (goto-char romacs-agent-log--input) (forward-line 0) (point))))
      (move-overlay romacs-agent-log-attach--overlay line line))))

(defun romacs-agent-log-attach--draft-replaced ()
  "The draft was sent or replaced: what was pending on it lands nowhere."
  (romacs-agent-log-attach--forget-paste romacs-agent-log-attach--paste)
  (dolist (gone romacs-agent-log-attach--unlinked) (set-marker (nth 1 gone) nil))
  (setq romacs-agent-log-attach--unlinked nil))

(defun romacs-agent-log-attach-setup ()
  "Make the current chat a tray host whose chips are its composer's links."
  (romacs-attach-tray-setup :consumer 'agent-chat :redisplay #'romacs-agent-log-attach--redisplay)
  ;; The chips show the composer's images: not again under the draft.
  (setq romacs-agent-log-images-under-draft nil)
  (add-hook 'after-change-functions #'romacs-agent-log-attach--changed nil t)
  (add-hook 'romacs-agent-log-draft-replaced-hook #'romacs-agent-log-attach--draft-replaced nil t))

;;;; Keys

(defun romacs-agent-log-attach-delete-backward ()
  "DEL: at the composer's start with chips, the chips to edit; else a
character back, as ever."
  (interactive nil romacs-agent-log-mode)
  (if (and romacs-attach-items (= (point) romacs-agent-log--input))
      (romacs-attach-edit)
    (call-interactively #'delete-backward-char)))

(defun romacs-agent-log-attach--forget-paste (paste)
  "Let PASTE go when it is the paste under way."
  (when (and paste (eq paste romacs-agent-log-attach--paste))
    (set-marker paste nil)
    (setq romacs-agent-log-attach--paste nil)))

(defun romacs-agent-log-attach--pasted (chat paste file spaced)
  "The clipboard's image saved as FILE (nil when it could not be) for
PASTE, CHAT's paste: linked at its marker while it is CHAT's paste
still, after a space when SPACED and the text before it needs one."
  (when (buffer-live-p chat)
    (with-current-buffer chat
      (when (eq paste romacs-agent-log-attach--paste)
        (unwind-protect
            (if (not file) (message "Could not read the clipboard's image")
              (when (>= paste romacs-agent-log--input)
                (let ((link (concat (and spaced (> paste romacs-agent-log--input)
                                         (not (memq (char-before paste) '(?\s ?\n)))
                                         " ")
                                    (romacs-agent-log-images-link file))))
                  ;; Still where the paste began: after the link, as a yank leaves it.
                  (if (= (point) paste) (insert link)
                    (save-excursion (goto-char paste) (insert link))))))
          (romacs-agent-log-attach--forget-paste paste))))))

(defun romacs-agent-log-attach-paste (type at &optional spaced)
  "Save the clipboard's image of TYPE once and link it at AT in the
composer (after a space when SPACED), unless the draft is sent or
replaced first; one paste at a time.  Nil when one is under way."
  (if romacs-agent-log-attach--paste
      (progn (message "Still saving the last image pasted") nil)
    (let ((chat (current-buffer))
          (paste (copy-marker at)))
      (setq romacs-agent-log-attach--paste paste)
      (condition-case err
          (romacs-attach-clipboard-file
           type (lambda (file) (romacs-agent-log-attach--pasted chat paste file spaced)))
        (error (romacs-agent-log-attach--forget-paste paste)
               (signal (car err) (cdr err))))
      t)))

(defun romacs-agent-log-attach-yank (&optional arg)
  "C-y: with an image on the clipboard and point in the composer, the
image saved once and its link put in where C-y was typed; else a yank,
as ever (ARG).  One paste at a time."
  (interactive "*P" romacs-agent-log-mode)
  (if-let* (((not arg))
            ((>= (point) romacs-agent-log--input))
            (type (romacs-attach-clipboard-image-type)))
      (romacs-agent-log-attach-paste type (point))
    (yank arg)))

(provide 'romacs-agent-log-attach)
;;; romacs-agent-log-attach.el ends here
