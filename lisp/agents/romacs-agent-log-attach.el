;;; romacs-agent-log-attach.el --- A chat's attachments, its composer's image links as chips -*- lexical-binding: t; -*-
;; SPDX-License-Identifier: GPL-3.0-or-later
;; Maintainer: Thanos Apollo <public@thanosapollo.org>

;;; Commentary:

;; An attachment is a Markdown link to its file in the composer,
;; [image](PATH): the message carries it, the chat shows it inline, and
;; nothing is copied.  The links are the data.  romacs-attach.el's tray
;; shows them as chips above the composer and edits them.
;;
;; Reading the draft never writes it: a change to the composer rescans
;; its links and binds each chip to one link (a link to a file that is
;; gone has none).  A chip edit writes back only what it changed, link by
;; link: a chip removed deletes its own link, a move trades two links'
;; text as written, a caption relabels its link alone.  DEL at the
;; composer's start enters the chips (n/p, d, M-f/M-b, c, RET, q).  C-y
;; with an image on the clipboard saves it once and links it where C-y
;; was typed.

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

(defvar-local romacs-agent-log-attach--slots nil
  "The composer's links as last read, in order: (KEY . ID) each, KEY
what the link says (`romacs-agent-log-attach--key'), ID its chip's item
id, nil for a link without a chip.")

(defvar-local romacs-agent-log-attach--unlinked nil
  "Item id -> the link text a chip removed had: undone, it comes back as written.")

(defvar-local romacs-agent-log-attach--dirty nil
  "The composer changed since its links were read; a sync is scheduled.")

(defvar-local romacs-agent-log-attach--paste nil
  "The clipboard paste under way: the marker where C-y was typed.")

(defvar romacs-agent-log-attach--reading nil
  "Non-nil while the tray takes the composer's links: nothing to write back.")

;;;; Links

(defun romacs-agent-log-attach--unescape (label)
  "LABEL's text, its backslash escapes undone."
  (replace-regexp-in-string "\\\\\\(.\\)" "\\1" label t))

(defun romacs-agent-log-attach--escape (caption)
  "CAPTION as a link's label: its brackets and backslashes escaped."
  (replace-regexp-in-string "[][\\]" "\\\\\\&" caption t))

(defun romacs-agent-log-attach--default-label (file)
  "The label a link to FILE has without a caption: `image' or `file'."
  (let ((link (romacs-agent-log-images-link file)))
    (substring link 1 (string-search "]" link))))

(defun romacs-agent-log-attach--caption (label)
  "The caption a link's LABEL says: none for the plain `image' or `file'."
  (and label (not (member label '("" "image" "file"))) label))

(defun romacs-agent-log-attach--links ()
  "The composer's image links, in order: (BEG END FILE CAPTION LABEL-END) each."
  (mapcar (pcase-lambda (`(,beg ,end ,file))
            (save-excursion
              (goto-char beg)
              (looking-at romacs-agent-log-images-label-regexp)
              (list beg end file
                    (romacs-agent-log-attach--caption
                     (romacs-agent-log-attach--unescape (match-string-no-properties 1)))
                    (match-end 0))))
          (romacs-agent-log-images-links nil (marker-position romacs-agent-log--input) (point-max))))

(defun romacs-agent-log-attach--key (file caption)
  "What a link and its chip agree on: its FILE and CAPTION."
  (cons file caption))

(defun romacs-agent-log-attach--link-key (link)
  (romacs-agent-log-attach--key (nth 2 link) (nth 3 link)))

(defun romacs-agent-log-attach--item-key (item)
  (romacs-agent-log-attach--key (plist-get item :path) (plist-get item :caption)))

(defun romacs-agent-log-attach--relabel (text file caption)
  "TEXT, a link to FILE as written, labelled CAPTION (its default
label without one), its destination as written."
  (concat "[" (romacs-agent-log-attach--escape
               (or (romacs-agent-log-attach--caption caption) (romacs-agent-log-attach--default-label file)))
          (substring text (progn (string-match romacs-agent-log-images-label-regexp text) (1- (match-end 0))))))

;;;; Reading: the draft's links as chips

(defun romacs-agent-log-attach--bind (links items make)
  "LINKS bound to ITEMS: (SLOTS . ITEMS), SLOTS one (KEY . ID) per link,
ITEMS those bound in link order.  A link takes the first item left with
its key, else one MAKE makes of it (nil for none)."
  (let ((free (copy-sequence items)) (slots nil) (bound nil))
    (dolist (link links)
      (let* ((key (romacs-agent-log-attach--link-key link))
             (item (or (seq-find (lambda (item) (equal (romacs-agent-log-attach--item-key item) key)) free)
                       (funcall make link))))
        (setq free (delq item free))
        (when item (push item bound))
        (push (cons key (plist-get item :id)) slots)))
    (cons (nreverse slots) (nreverse bound))))

(defun romacs-agent-log-attach--make (link)
  "A chip's item for LINK, or nil when its file cannot be one."
  (ignore-errors (romacs-attach-item (nth 2 link) :caption (nth 3 link) :source 'link)))

(defun romacs-agent-log-attach-sync ()
  "Show the composer's links as the tray's chips, when they changed.
Reading only: the draft is never written here."
  (setq romacs-agent-log-attach--dirty nil)
  (when (markerp romacs-agent-log--input)
    (let ((links (romacs-agent-log-attach--links)))
      (unless (equal (mapcar #'romacs-agent-log-attach--link-key links) (mapcar #'car romacs-agent-log-attach--slots))
        (pcase-let ((`(,slots . ,items) (romacs-agent-log-attach--bind links romacs-attach-items #'romacs-agent-log-attach--make)))
          (setq romacs-agent-log-attach--slots slots)
          (let ((romacs-agent-log-attach--reading t))
            (romacs-attach-set-items items)))))))

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

(defun romacs-agent-log-attach--edits (slots items)
  "What turns links bound as SLOTS into ITEMS: a list of (INDEX . WHAT)
by link, WHAT `delete' or (SOURCE . ITEM), the link at SOURCE's text
relabelled for ITEM; and the ITEMS no link has, to append."
  (let* ((ids (mapcar #'cdr slots))
         (wanted (mapcar (lambda (item) (plist-get item :id)) items))
         (staying (seq-filter (lambda (id) (and id (member id wanted))) ids))
         (order (seq-filter (lambda (item) (member (plist-get item :id) staying)) items))
         (k -1) edits)
    (cl-loop for id in ids for i from 0
             when id
             do (push (cons i (if (not (member id wanted)) 'delete
                                (let ((item (nth (setq k (1+ k)) order)))
                                  (cons (seq-position ids (plist-get item :id)) item))))
                      edits))
    (cons (nreverse edits)
          (seq-remove (lambda (item) (member (plist-get item :id) ids)) items))))

(defun romacs-agent-log-attach--delete-link (beg end)
  "Delete the link from BEG to END and one space beside it."
  (goto-char beg)
  (delete-region beg end)
  ;; Not the prompt's own space, before the composer.
  (cond ((and (> (point) romacs-agent-log--input) (eq (char-before) ?\s) (memq (char-after) '(?\s ?\n nil)))
         (delete-char -1))
        ((eq (char-after) ?\s) (delete-char 1))))

(defun romacs-agent-log-attach--write ()
  "Write the tray's last edit to the composer's links, each touched link
alone; when the draft no longer has the links last read, read it instead."
  (let ((links (romacs-agent-log-attach--links)))
    (if (not (equal (mapcar #'romacs-agent-log-attach--link-key links) (mapcar #'car romacs-agent-log-attach--slots)))
        (romacs-agent-log-attach-sync)
      (pcase-let* ((`(,edits . ,extra) (romacs-agent-log-attach--edits romacs-agent-log-attach--slots romacs-attach-items))
                   (texts (mapcar (lambda (link) (buffer-substring-no-properties (nth 0 link) (nth 1 link))) links)))
        (save-excursion
          (dolist (item extra)
            (goto-char (point-max))
            (unless (or (= (point) romacs-agent-log--input) (memq (char-before) '(?\s ?\n))) (insert " "))
            (insert (let ((text (cdr (assoc (plist-get item :id) romacs-agent-log-attach--unlinked))))
                      (if text (romacs-agent-log-attach--relabel text (plist-get item :path) (plist-get item :caption))
                        (romacs-agent-log-attach--relabel (romacs-agent-log-images-link (plist-get item :path))
                                                          (plist-get item :path) (plist-get item :caption))))))
          ;; From the last link, so the places of those before stay.
          (pcase-dolist (`(,i . ,what) (reverse edits))
            (pcase-let ((`(,beg ,end . ,_) (nth i links)))
              (if (eq what 'delete)
                  (progn (push (cons (cdr (nth i romacs-agent-log-attach--slots)) (nth i texts))
                               romacs-agent-log-attach--unlinked)
                         (romacs-agent-log-attach--delete-link beg end))
                (let ((text (romacs-agent-log-attach--relabel (nth (car what) texts) (plist-get (cdr what) :path)
                                                              (plist-get (cdr what) :caption))))
                  (unless (equal text (nth i texts))
                    (goto-char beg)
                    (delete-region beg end)
                    (insert text)))))))
        (setq romacs-agent-log-attach--slots
              (car (romacs-agent-log-attach--bind (romacs-agent-log-attach--links) romacs-attach-items #'ignore))
              romacs-agent-log-attach--dirty nil)))))

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

(defun romacs-agent-log-attach-setup ()
  "Make the current chat a tray host whose chips are its composer's links."
  (romacs-attach-tray-setup :consumer 'agent-chat :redisplay #'romacs-agent-log-attach--redisplay)
  ;; The chips show the composer's images: not again under the draft.
  (setq romacs-agent-log-images-under-draft nil)
  (add-hook 'after-change-functions #'romacs-agent-log-attach--changed nil t)
  (add-hook 'romacs-agent-log-draft-replaced-hook #'romacs-agent-log-attach--forget-paste nil t))

;;;; Keys

(defun romacs-agent-log-attach-delete-backward ()
  "DEL: at the composer's start with chips, the chips to edit; else a
character back, as ever."
  (interactive nil romacs-agent-log-mode)
  (if (and romacs-attach-items (= (point) romacs-agent-log--input))
      (romacs-attach-edit)
    (call-interactively #'delete-backward-char)))

(defun romacs-agent-log-attach--forget-paste ()
  "The draft was sent or replaced: a paste under way lands nowhere."
  (when romacs-agent-log-attach--paste
    (set-marker romacs-agent-log-attach--paste nil)
    (setq romacs-agent-log-attach--paste nil)))

(defun romacs-agent-log-attach--pasted (chat paste file)
  "The clipboard's image saved as FILE (nil when it could not be) for
PASTE, CHAT's paste: linked at its marker while it is CHAT's paste still."
  (cond ((not (buffer-live-p chat)))
        ((not file)
         (with-current-buffer chat (romacs-agent-log-attach--forget-paste))
         (message "Could not read the clipboard's image"))
        (t (with-current-buffer chat
             (when (and (eq paste romacs-agent-log-attach--paste) (marker-position paste)
                        (>= paste romacs-agent-log--input))
               ;; Still where C-y was typed: after the link, as a yank leaves it.
               (if (= (point) paste) (insert (romacs-agent-log-images-link file))
                 (save-excursion
                   (goto-char paste)
                   (insert (romacs-agent-log-images-link file)))))
             (romacs-agent-log-attach--forget-paste)))))

(defun romacs-agent-log-attach-yank (&optional arg)
  "C-y: with an image on the clipboard and point in the composer, the
image saved once and its link put in where C-y was typed; else a yank,
as ever (ARG).  One paste at a time."
  (interactive "*P" romacs-agent-log-mode)
  (if-let* (((not arg))
            ((>= (point) romacs-agent-log--input))
            (type (romacs-attach-clipboard-image-type)))
      (if romacs-agent-log-attach--paste
          (message "Still saving the last image pasted")
        (let ((chat (current-buffer))
              (paste (copy-marker (point))))
          (setq romacs-agent-log-attach--paste paste)
          (romacs-attach-clipboard-file
           type (lambda (file) (romacs-agent-log-attach--pasted chat paste file)))))
    (yank arg)))

(provide 'romacs-agent-log-attach)
;;; romacs-agent-log-attach.el ends here
