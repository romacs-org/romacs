;;; romacs-agent-log-attach.el --- A chat's attachments, its composer's image links as chips -*- lexical-binding: t; -*-
;; SPDX-License-Identifier: GPL-3.0-or-later
;; Maintainer: Thanos Apollo <public@thanosapollo.org>

;;; Commentary:

;; An attachment is a Markdown link to its file in the composer,
;; [image](PATH): the message carries it, the chat shows it inline, and
;; nothing is copied.  The links are the data.  romacs-attach.el's tray
;; shows them as chips above the composer and edits them: a chip removed
;; removes its link, a chip moved trades places with another link, a
;; caption becomes its link's label.  DEL at the composer's start enters
;; the chips (n/p, d, M-f/M-b, c, RET, q).  C-y with an image on the
;; clipboard saves it once and puts its link in.

;;; Code:

(require 'cl-lib)
(require 'seq)
(require 'romacs-attach)
(require 'romacs-agent-log-images)

(defvar romacs-agent-log--input)

(romacs-attach-define-consumer 'agent-chat
  :display "The chat" :rules '(("\\`image/" image) ("" path)))

(defvar-local romacs-agent-log-attach--overlay nil
  "The overlay that shows the tray at the composer's start.")

(defun romacs-agent-log-attach--links ()
  "The composer's image links, in order: (BEG END FILE LABEL) each."
  (mapcar (pcase-lambda (`(,beg ,end ,file))
            (list beg end file (save-excursion (goto-char beg)
                                               (and (looking-at "\\[\\([^]\n]*\\)\\]") (match-string-no-properties 1)))))
          (romacs-agent-log-images-links nil (marker-position romacs-agent-log--input) (point-max))))

(defun romacs-agent-log-attach--caption (label)
  "The caption a link's LABEL says: none for the plain `image' or `file'."
  (and label (not (member label '("" "image" "file"))) label))

(defun romacs-agent-log-attach--key (file caption)
  "What a link and its chip agree on: its FILE and CAPTION."
  (cons file caption))

(defun romacs-agent-log-attach--link-text (item)
  "ITEM's link: [image](PATH), its caption the label when it has one."
  (let ((link (romacs-agent-log-images-link (plist-get item :path)))
        (caption (plist-get item :caption)))
    (if caption (concat "[" caption (substring link (string-search "]" link))) link)))

(defun romacs-agent-log-attach--items (links)
  "The tray's items for LINKS, an item kept where its file and caption
still are at its place; a link to a file gone has no chip."
  (let ((old romacs-attach-items))
    (delq nil
          (cl-loop for (_beg _end file label) in links
                   for i from 0
                   for caption = (romacs-agent-log-attach--caption label)
                   for kept = (nth i old)
                   collect (if (and kept (equal (plist-get kept :path) file) (equal (plist-get kept :caption) caption))
                               kept
                             (ignore-errors (romacs-attach-item file :caption caption :source 'link)))))))

(defun romacs-agent-log-attach-sync ()
  "Show the composer's links as the tray's chips, when they changed."
  (when (and romacs-agent-log--input (markerp romacs-agent-log--input))
    (let ((links (romacs-agent-log-attach--links)))
      (unless (equal (mapcar (pcase-lambda (`(,_ ,_ ,file ,label))
                               (romacs-agent-log-attach--key file (romacs-agent-log-attach--caption label)))
                             links)
                     (mapcar (lambda (item) (romacs-agent-log-attach--key (plist-get item :path) (plist-get item :caption)))
                             romacs-attach-items))
        (romacs-attach-set-items (romacs-agent-log-attach--items links))))))

(defun romacs-agent-log-attach--matched (links items)
  "LINKS paired with ITEMS by file, each item once: (KEPT . EXTRA), KEPT
the links that have an item, in place, EXTRA the items no link has."
  (let ((free (copy-sequence items)) (kept nil))
    (dolist (link links)
      (when-let* ((item (seq-find (lambda (item) (equal (plist-get item :path) (nth 2 link))) free)))
        (setq free (delq item free))
        (push link kept)))
    (cons (nreverse kept) free)))

(defun romacs-agent-log-attach--write ()
  "Make the composer's links the tray's items: a link whose chip went goes
(with a space beside it), the links that stay take the chips in the
tray's order in their places (a move, a caption), and a chip no link
has is linked at the composer's end."
  (let* ((links (romacs-agent-log-attach--links))
         (items romacs-attach-items))
    (unless (equal (mapcar (pcase-lambda (`(,_ ,_ ,file ,label))
                             (romacs-agent-log-attach--key file (romacs-agent-log-attach--caption label)))
                           links)
                   (mapcar (lambda (item) (romacs-agent-log-attach--key (plist-get item :path) (plist-get item :caption)))
                           items))
      (pcase-let* ((`(,kept . ,extra) (romacs-agent-log-attach--matched links items))
                   (placed (seq-remove (lambda (item) (memq item extra)) items)))
        (save-excursion
          (dolist (item extra)
            (goto-char (point-max))
            (unless (or (= (point) romacs-agent-log--input) (memq (char-before) '(?\s ?\n))) (insert " "))
            (insert (romacs-agent-log-attach--link-text item)))
          ;; From the last, so the places of those before stay.
          (let ((n (length kept)))
            (dolist (link (reverse links))
              (pcase-let ((`(,beg ,end . ,_) link))
                (goto-char beg)
                (delete-region beg end)
                (if (memq link kept)
                    (insert (romacs-agent-log-attach--link-text (nth (setq n (1- n)) placed)))
                  (cond ((and (eq (char-before) ?\s) (memq (char-after) '(?\s ?\n nil))) (delete-char -1))
                        ((eq (char-after) ?\s) (delete-char 1))))))))))))

(defun romacs-agent-log-attach--redisplay ()
  "The tray changed: its links written to the composer, its chips shown
at the composer's start."
  (romacs-agent-log-attach--write)
  (let ((string (romacs-attach-tray-string)))
    (unless (and romacs-agent-log-attach--overlay (overlay-buffer romacs-agent-log-attach--overlay))
      (setq romacs-agent-log-attach--overlay (make-overlay romacs-agent-log--input romacs-agent-log--input nil nil nil)))
    (move-overlay romacs-agent-log-attach--overlay romacs-agent-log--input romacs-agent-log--input)
    (overlay-put romacs-agent-log-attach--overlay 'before-string string)))

(defun romacs-agent-log-attach-setup ()
  "Make the current chat a tray host whose chips are its composer's links."
  (romacs-attach-tray-setup :consumer 'agent-chat :redisplay #'romacs-agent-log-attach--redisplay)
  ;; The chips show the composer's images: not again under the draft.
  (setq romacs-agent-log-images-under-draft nil)
  (add-hook 'post-command-hook #'romacs-agent-log-attach-sync nil t))

(defun romacs-agent-log-attach-delete-backward ()
  "DEL: at the composer's start with chips, the chips to edit; else a
character back, as ever."
  (interactive nil romacs-agent-log-mode)
  (if (and romacs-attach-items (= (point) romacs-agent-log--input))
      (romacs-attach-edit)
    (call-interactively #'delete-backward-char)))

(defun romacs-agent-log-attach-yank (&optional arg)
  "C-y: with an image on the clipboard and point in the composer, the
image saved once and its link put in; else a yank, as ever (ARG)."
  (interactive "*P" romacs-agent-log-mode)
  (if-let* (((not arg))
            ((>= (point) romacs-agent-log--input))
            (type (romacs-attach-clipboard-image-type)))
      (let ((chat (current-buffer)))
        (romacs-attach-clipboard-file
         type (lambda (file)
                (cond ((not file) (message "Could not read the clipboard's image"))
                      ((buffer-live-p chat)
                       (with-current-buffer chat
                         (insert (romacs-agent-log-images-link file))
                         (romacs-agent-log-attach-sync)))))))
    (yank arg)))

(provide 'romacs-agent-log-attach)
;;; romacs-agent-log-attach.el ends here
