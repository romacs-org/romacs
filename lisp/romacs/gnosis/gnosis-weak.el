;;; gnosis-weak.el --- Weak-spot signals in note buffers  -*- lexical-binding: t; -*-
;; Maintainer: Thanos Apollo <public@thanosapollo.org>

;; Copyright (C) 2026  Free Software Foundation, Inc.

;; Author: Thanos Apollo <public@thanosapollo.org>
;; Keywords: extensions

;; This program is free software; you can redistribute it and/or modify
;; it under the terms of the GNU General Public License as published by
;; the Free Software Foundation, either version 3 of the License, or
;; (at your option) any later version.

;; This program is distributed in the hope that it will be useful,
;; but WITHOUT ANY WARRANTY; without even the implied warranty of
;; MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE.  See the
;; GNU General Public License for more details.

;; You should have received a copy of the GNU General Public License
;; along with this program.  If not, see <https://www.gnu.org/licenses/>.

;;; Commentary:

;; Mark note headings whose linked themata are weak, and study them with
;; one key.  A thema's weakness combines its current forgetting
;; probability (pinned FSRS retrievability), difficulty and lapses.
;; Unreviewed and suspended themata carry no evidence and are ignored.
;; Healthy nodes show nothing.
;;
;; `gnosis-weak-nodes' returns the same scores as plain data for agents.
;; The minor mode only reads the database and adds overlays; it never
;; changes note text.  `gnosis-weak-practice' practises the weak themata
;; without rescheduling; `gnosis-weak-review' reviews only those already
;; due, with normal FSRS acceptance.

;;; Code:

(require 'cl-lib)
(require 'seq)
(require 'subr-x)
(require 'org)
(require 'gnosis)
(require 'gnosis-fsrs)
(require 'gnosis-scheduler)
(require 'gnosis-org)
(require 'gnosis-nodes)

(declare-function gnosis-review-loop "gnosis-review" (collector &optional mode target validate))
(declare-function gnosis-review--session-target "gnosis-review" ())
(defvar gnosis-review--running)
(defvar gnosis-weak-mode)

(defgroup gnosis-weak nil
  "Weak-spot signals for Gnosis notes."
  :group 'gnosis)

(defcustom gnosis-weak-indicators t
  "When non-nil, mark weak nodes in Org notes under `gnosis-nodes-dir'."
  :type 'boolean)

(defcustom gnosis-weak-threshold 0.35
  "Minimum thema weakness, between 0 and 1, that marks it weak."
  :type 'number)

(defcustom gnosis-weak-severe-threshold 0.6
  "Weakness at or above which a node uses `gnosis-weak-severe'."
  :type 'number)

(defface gnosis-weak '((t :inherit warning :weight normal))
  "Indicator for a node with weak themata.")

(defface gnosis-weak-severe '((t :inherit error :weight normal))
  "Indicator for a node whose weakest thema is severely weak.")

;;; Scoring (pure)

(defconst gnosis-weak--weights '(0.5 0.3 0.2)
  "Weights of forgetting probability, difficulty and lapses.
They sum to one so a weakness stays within [0, 1].")

(defconst gnosis-weak--lapse-cap 4
  "Lapses at which the lapse component saturates.")

(defun gnosis-weak--elapsed-days (last-review-day today)
  "Return nonnegative logical days from LAST-REVIEW-DAY to TODAY."
  (condition-case nil
      (gnosis-scheduler--days-between last-review-day today)
    (error 0)))

(defun gnosis-weak-thema-score (stability difficulty elapsed-days lapses)
  "Return a weakness plist for one thema, or nil without memory state.
STABILITY and DIFFICULTY are FSRS memory state, ELAPSED-DAYS the days since
its last scheduled review and LAPSES its failure count.  The plist has
`:score' in [0, 1], `:retrievability', `:difficulty' and `:lapses'.
Retrievability uses the pinned FSRS model; nothing here is fitted."
  (when (and (numberp stability) (> stability 0) (numberp difficulty))
    (let* ((retrievability (gnosis-fsrs--retrievability
                            (max 0 elapsed-days) stability))
           (difficulty (min 10.0 (max 1.0 (float difficulty))))
           (lapses (max 0 (or lapses 0)))
           (parts (list (- 1.0 retrievability)
                        (/ (- difficulty 1.0) 9.0)
                        (/ (float (min lapses gnosis-weak--lapse-cap))
                           gnosis-weak--lapse-cap))))
      (list :score (min 1.0 (max 0.0 (apply #'+ (cl-mapcar #'* gnosis-weak--weights parts))))
            :retrievability retrievability
            :difficulty difficulty
            :lapses lapses))))

(defun gnosis-weak--snippet (text)
  "Return a short one-line display form of thema TEXT."
  (let ((line (string-trim (replace-regexp-in-string "[ \t\n]+" " " (format "%s" (or text ""))))))
    (truncate-string-to-width line 60 nil nil t)))

(defun gnosis-weak-aggregate (rows today &optional threshold)
  "Aggregate thema ROWS into per-node weakness plists for TODAY.
Each row is (NODE THEMA STABILITY DIFFICULTY LAST-REVIEW-DAY LAPSES DUE-DAY
KEIMENON TITLE).  THRESHOLD defaults to `gnosis-weak-threshold'.  Return
one plist per node, in first-seen order: `:node', `:title', `:themata'
\(scored count), `:unscored', `:weak' (count), `:score' (the weakest
score, or 0) and `:items', the weak themata sorted weakest first.  Each item
has `:thema', `:keimenon', `:due-day' and the `gnosis-weak-thema-score'
keys.  ROWS are not modified."
  (let ((threshold (or threshold gnosis-weak-threshold))
        (nodes (make-hash-table :test #'equal))
        ;; Many themata share a last review day; date arithmetic dominates.
        (elapsed (make-hash-table :test #'eql))
        order)
    (pcase-dolist (`(,node ,thema ,stability ,difficulty ,last-day ,lapses
                           ,due-day ,keimenon ,title)
                   rows)
      (let ((entry (or (gethash node nodes)
                       (progn (push node order)
                              (puthash node (list :node node :title title
                                                  :themata 0 :unscored 0 :items nil)
                                       nodes)))))
        (if-let* ((score (and last-day
                              (gnosis-weak-thema-score
                               stability difficulty
                               (with-memoization (gethash last-day elapsed)
                                 (gnosis-weak--elapsed-days last-day today))
                               lapses))))
            (progn
              (setq entry (plist-put entry :themata (1+ (plist-get entry :themata))))
              (when (>= (plist-get score :score) threshold)
                (setq entry (plist-put entry :items
                                       (cons (append (list :thema thema
                                                           :keimenon (gnosis-weak--snippet keimenon)
                                                           :due-day due-day)
                                                     score)
                                             (plist-get entry :items))))))
          (setq entry (plist-put entry :unscored (1+ (plist-get entry :unscored)))))
        (puthash node entry nodes)))
    (mapcar (lambda (node)
              (let* ((entry (gethash node nodes))
                     (items (sort (copy-sequence (plist-get entry :items))
                                  (lambda (a b) (> (plist-get a :score)
                                                   (plist-get b :score))))))
                (list :node node :title (plist-get entry :title)
                      :themata (plist-get entry :themata)
                      :unscored (plist-get entry :unscored)
                      :weak (length items)
                      :score (if items (plist-get (car items) :score) 0.0)
                      :items items)))
            (nreverse order))))

;;; Query

(defconst gnosis-weak--select
  "SELECT l.dest, s.thema_id, s.stability, s.difficulty, s.last_review_day,
          s.lapses, s.due_day, t.keimenon, n.title
     FROM thema_links l
     CROSS JOIN scheduler_state s ON s.thema_id = l.source
     JOIN themata t ON t.id = l.source
     LEFT JOIN nodes n ON n.id = l.dest
    WHERE s.suspended = 0"
  "Linked active thema state for weakness scoring.
CROSS JOIN keeps `thema_links' as the outer loop.  Otherwise SQLite may
walk every active scheduler row through the suspended index and probe
each one against every requested node.")

(defun gnosis-weak--restricted-sql ()
  "Return the node-restricted read, with %s for the ID placeholders."
  (concat gnosis-weak--select " AND l.dest IN (%s) ORDER BY l.dest, s.thema_id"))

(defun gnosis-weak--rows (&optional node-ids)
  "Return linked thema rows for NODE-IDS, or for every linked node.
Run one read-only query (batched only beyond SQLite's variable limit)."
  (let ((db (gnosis--ensure-db)))
    (if node-ids
        (gnosis-sqlite-select-batch db (gnosis-weak--restricted-sql) node-ids)
      (gnosis-sqlite-select db (concat gnosis-weak--select " ORDER BY l.dest, s.thema_id")))))

;;;###autoload
(defun gnosis-weak-nodes (&optional node-ids threshold today)
  "Return weak nodes, weakest first, as plain plists.
NODE-IDS restricts the result to those Org IDs; nil means every node with
linked themata.  THRESHOLD defaults to `gnosis-weak-threshold' and TODAY
to the current logical day (YYYYMMDD).  Only nodes with at least one weak
thema are returned; see `gnosis-weak-aggregate' for the plist shape.
This reads the current database and writes nothing."
  (let ((nodes (gnosis-weak-aggregate (gnosis-weak--rows (and node-ids (delete-dups (copy-sequence node-ids))))
                                      (or today (gnosis--today-int))
                                      threshold)))
    (sort (seq-filter (lambda (node) (> (plist-get node :weak) 0)) nodes)
          (lambda (a b) (> (plist-get a :score) (plist-get b :score))))))

;;; Buffer indicators

(defcustom gnosis-weak-idle-delay 0.3
  "Idle seconds before indicators are recomputed or re-placed.
Opening, saving, showing and editing a note only schedule this work, so
none of them waits for the database."
  :type 'number)

(defvar-local gnosis-weak--refreshed-at nil
  "Time of the last successful indicator refresh in this buffer.")

(defvar-local gnosis-weak--nodes nil
  "Hash table of weak node plists by ID from the last database read.")

(defvar-local gnosis-weak--timer nil
  "Pending idle timer for this buffer's indicators, or nil.")

(defvar gnosis-weak--views nil
  "Live buffers with `gnosis-weak-mode' enabled.
While non-nil, `gnosis-weak--after-change' observes edits in every buffer,
so editing through a view without indicators still re-anchors the others.")

(defvar-local gnosis-weak--pending nil
  "Indicator work awaiting idle time: nil, `place' or `refresh'.
`place' re-anchors the cached scores without reading the database.")

(defconst gnosis-weak--structure-re
  "^\\(?:\\*+[ \t]\\|[ \t]*:ID:\\|#\\+title:\\)"
  "Lines whose edits can add, move or remove a node anchor.")

(defun gnosis-weak--root-anchor ()
  "Return the root node's anchor line start in the widened buffer.
That is the first #+title line, or the first line without one."
  (save-excursion
    (goto-char (point-min))
    (let ((case-fold-search t)
          (limit (save-excursion (outline-next-heading) (point))))
      (if (re-search-forward "^#\\+title:" limit t)
          (line-beginning-position)
        (point-min)))))

(defun gnosis-weak--line-node (pos)
  "Return the ID of the node anchored on the line at POS, or nil.
Heading lines anchor their own entry; the root title line anchors the
file-level entry.  Assume a widened buffer."
  (save-excursion
    (goto-char pos)
    (beginning-of-line)
    (cond ((org-at-heading-p) (org-entry-get nil "ID"))
          ((and (org-before-first-heading-p)
                (= (point) (gnosis-weak--root-anchor)))
           (org-entry-get nil "ID")))))

(defun gnosis-weak--buffer-nodes ()
  "Return (ID . ANCHOR) for each node in the current Org buffer.
ANCHOR is the start of the owning heading line, or of the root title
line.  Ignore narrowing; do not move point or create IDs."
  (org-with-wide-buffer
   (goto-char (point-min))
   (let (nodes)
     (while (re-search-forward "^[ \t]*:ID:[ \t]+\\(\\S-+\\)[ \t]*$" nil t)
       (let ((id (match-string-no-properties 1)))
         (save-excursion
           (when (equal id (org-entry-get nil "ID"))
             (push (cons id (if (org-before-first-heading-p)
                                (gnosis-weak--root-anchor)
                              (org-back-to-heading t)
                              (point)))
                   nodes)))))
     (nreverse nodes))))

(defun gnosis-weak--keys-help ()
  "Return the key hint shown at the end of indicator details."
  (substitute-command-keys
   "\n\\<gnosis-weak-mode-map>\\[gnosis-weak-practice] practise (no rescheduling)  \
\\[gnosis-weak-review] review due  (C-u: with subtree)"))

(defun gnosis-weak--help (node &optional keys)
  "Return tooltip text for weak NODE.
KEYS is the key hint, computed when nil."
  (concat
   (format "%d of %d reviewed themata weak (weakest %.2f)"
           (plist-get node :weak) (plist-get node :themata) (plist-get node :score))
   (mapconcat (lambda (item)
                (format "\n  %.2f  R %.0f%%  D %.1f  lapses %d  %s"
                        (plist-get item :score)
                        (* 100 (plist-get item :retrievability))
                        (plist-get item :difficulty)
                        (plist-get item :lapses)
                        (plist-get item :keimenon)))
              (seq-take (plist-get node :items) 3) "")
   (or keys (gnosis-weak--keys-help))))

(defun gnosis-weak--indicator (node &optional help)
  "Return the right-aligned indicator string for weak NODE.
HELP is its details text, computed when nil.  A filled dot marks severe
weakness, a hollow one ordinary weakness, so the distinction survives
themes whose warning and error colors are similar."
  (let* ((severe (>= (plist-get node :score) gnosis-weak-severe-threshold))
         (label (format "%s %d" (if severe "●" "○") (plist-get node :weak))))
    (concat (propertize " " 'display `(space :align-to (- right ,(1+ (string-width label)))))
            (propertize label 'face (if severe 'gnosis-weak-severe 'gnosis-weak)
                        'help-echo (or help (gnosis-weak--help node))
                        'gnosis-weak-node (plist-get node :node)))))

(defun gnosis-weak--overlays (&optional beg end)
  "Return weak-spot overlays between BEG and END, default the whole buffer.
Ignore narrowing."
  (save-restriction
    (widen)
    (seq-filter (lambda (ov) (overlay-get ov 'gnosis-weak))
                (overlays-in (or beg (point-min)) (or end (point-max))))))

(defun gnosis-weak--clear ()
  "Remove every weak-spot overlay from the current buffer, even if narrowed."
  (save-restriction
    (widen)
    (remove-overlays (point-min) (point-max) 'gnosis-weak t)))

(defun gnosis-weak--place (&optional anchors)
  "Show indicators for the cached weak nodes at their current anchors.
ANCHORS are the buffer's (ID . ANCHOR) pairs when the caller has just
read them (`gnosis-weak--buffer-nodes').  Replace all existing
indicators; never read the database."
  (gnosis-weak--clear)
  ;; With no weak node there is nothing to place, and no reason to walk.
  (when (and gnosis-weak--nodes (> (hash-table-count gnosis-weak--nodes) 0))
    (let ((keys (gnosis-weak--keys-help)))
      (save-restriction
        (widen)
        (pcase-dolist (`(,id . ,anchor) (or anchors (gnosis-weak--buffer-nodes)))
          (when-let* ((node (gethash id gnosis-weak--nodes)))
            ;; Span exactly the anchor line so `display-local-help' shows the
            ;; details from the keyboard.  Neither end advances: text typed
            ;; at either edge, such as a newline ending the heading, stays
            ;; outside; `gnosis-weak--after-change' re-spans the line.
            (let ((ov (make-overlay anchor (save-excursion (goto-char anchor)
                                                           (line-end-position))))
                  (help (gnosis-weak--help node keys)))
              (overlay-put ov 'gnosis-weak t)
              (overlay-put ov 'gnosis-weak-node id)
              (overlay-put ov 'help-echo help)
              (overlay-put ov 'after-string (gnosis-weak--indicator node help)))))))))

(defun gnosis-weak--cache (nodes)
  "Return a hash table of the weak members of NODES by ID."
  (let ((weak (make-hash-table :test #'equal)))
    (dolist (node nodes weak)
      (when (> (plist-get node :weak) 0)
        (puthash (plist-get node :node) node weak)))))

(defun gnosis-weak-refresh (&optional buffer)
  "Recompute weak-spot indicators in BUFFER, defaulting to the current one.
Read the database once for every node in the buffer, synchronously.
Mode hooks call this from an idle timer instead; see
`gnosis-weak-idle-delay'."
  (interactive)
  (with-current-buffer (or buffer (current-buffer))
    (condition-case err
        (let* ((anchors (gnosis-weak--buffer-nodes))
               (ids (delete-dups (mapcar #'car anchors)))
               (nodes (and ids (gnosis-weak-aggregate
                                (gnosis-weak--rows ids)
                                (gnosis--today-int)))))
          (setq gnosis-weak--nodes (gnosis-weak--cache nodes))
          (gnosis-weak--place anchors)
          (setq gnosis-weak--refreshed-at (float-time))
          nodes)
      (error (setq gnosis-weak--nodes nil)
             (gnosis-weak--clear)
             (message "Gnosis weak spots unavailable: %s" (error-message-string err))
             nil))))

(defun gnosis-weak--run-pending (buffer)
  "Run BUFFER's pending indicator work while its weak-spot mode is on."
  (when (buffer-live-p buffer)
    (with-current-buffer buffer
      (when (timerp gnosis-weak--timer) (cancel-timer gnosis-weak--timer))
      (let ((work gnosis-weak--pending))
        (setq gnosis-weak--timer nil gnosis-weak--pending nil)
        (when gnosis-weak-mode
          (pcase work
            ('refresh (gnosis-weak-refresh))
            ('place (gnosis-weak--place))))))))

(defun gnosis-weak--schedule (work)
  "Schedule WORK, `place' or `refresh', for the current buffer's next idle.
A pending refresh subsumes re-placing."
  (setq gnosis-weak--pending (if (memq 'refresh (list work gnosis-weak--pending))
                                 'refresh
                               'place))
  (unless (timerp gnosis-weak--timer)
    (setq gnosis-weak--timer
          (run-with-idle-timer gnosis-weak-idle-delay nil
                               #'gnosis-weak--run-pending (current-buffer)))))

(defun gnosis-weak--cancel ()
  "Cancel the current buffer's pending indicator work."
  (when (timerp gnosis-weak--timer) (cancel-timer gnosis-weak--timer))
  (setq gnosis-weak--timer nil gnosis-weak--pending nil))

(defun gnosis-weak--reanchor (beg end)
  "Keep the current buffer's indicators on their headings after BEG..END.
Re-span each indicator near the change to its whole anchor line, delete
those whose line no longer anchors their node, and schedule re-placing
from cached scores when node structure may have changed.  Never read the
database."
  (when gnosis-weak--nodes
    (save-match-data
      (save-excursion
        (save-restriction
          (widen)
          (condition-case err
              (let* ((from (progn (goto-char beg)
                                  (if (org-before-first-heading-p)
                                      (point-min)
                                    (org-back-to-heading t)
                                    (point))))
                     (to (progn (goto-char end) (line-end-position)))
                     (structural (progn (goto-char beg)
                                        (beginning-of-line)
                                        (re-search-forward gnosis-weak--structure-re to t))))
                (dolist (ov (gnosis-weak--overlays from (min (point-max) (1+ to))))
                  (let ((start (overlay-start ov)))
                    (if (equal (overlay-get ov 'gnosis-weak-node)
                               (gnosis-weak--line-node start))
                        (goto-char start)
                      (setq structural t)
                      (delete-overlay ov))
                    (when (overlay-buffer ov)
                      (move-overlay ov (line-beginning-position) (line-end-position)))))
                (when structural (gnosis-weak--schedule 'place)))
            (error (message "Gnosis weak spots: %s" (error-message-string err)))))))))

(defun gnosis-weak--after-change (beg end _length)
  "Re-anchor indicators after the change from BEG to END in every view.
Base and indirect buffers share text but not overlays, and the edited
view may have no indicators, so re-anchor each enabled view sharing the
edited text."
  (let ((base (or (buffer-base-buffer) (current-buffer))))
    (dolist (buffer gnosis-weak--views)
      (when (and (buffer-live-p buffer)
                 (eq base (or (buffer-base-buffer buffer) buffer))
                 (buffer-local-value 'gnosis-weak-mode buffer))
        (with-current-buffer buffer (gnosis-weak--reanchor beg end))))))

(defun gnosis-weak--register (on)
  "Add the current buffer to the enabled views when ON, else remove it.
Observe edits globally only while some view is enabled."
  (setq gnosis-weak--views
        (seq-filter #'buffer-live-p (delq (current-buffer) gnosis-weak--views)))
  (when on (push (current-buffer) gnosis-weak--views))
  (if gnosis-weak--views
      ;; After Org's own change handlers, so its element cache is current.
      (add-hook 'after-change-functions #'gnosis-weak--after-change 90)
    (remove-hook 'after-change-functions #'gnosis-weak--after-change)))

(defun gnosis-weak--kill ()
  "Cancel pending work and retire the killed buffer's view."
  (gnosis-weak--cancel)
  (gnosis-weak--register nil))

(defun gnosis-weak--forget-clone ()
  "Give a cloned indirect buffer its own timer for its pending work.
Cloning copies the base's overlays and timer variables, but that timer
runs only the base buffer's work."
  (let ((work gnosis-weak--pending))
    (setq gnosis-weak--timer nil gnosis-weak--pending nil)
    (when gnosis-weak-mode (gnosis-weak--register t))
    (when work (gnosis-weak--schedule work))))

(defun gnosis-weak--refresh-all (&rest _)
  "Schedule a refresh in every buffer showing weak-spot indicators."
  (dolist (buffer (buffer-list))
    (when (buffer-local-value 'gnosis-weak-mode buffer)
      (with-current-buffer buffer (gnosis-weak--schedule 'refresh)))))

(defun gnosis-weak--window-refresh (window)
  "Schedule a refresh of WINDOW's buffer when shown again after a second."
  (with-current-buffer (window-buffer window)
    (when (and gnosis-weak-mode
               (or (null gnosis-weak--refreshed-at)
                   (> (- (float-time) gnosis-weak--refreshed-at) 1.0)))
      (gnosis-weak--schedule 'refresh))))

(defun gnosis-weak--after-save ()
  "Schedule a refresh once the node index has been updated."
  (gnosis-weak--schedule 'refresh))

;;; Batches

(defun gnosis-weak--nodes-at-point (subtree)
  "Return node IDs at point, including descendants when SUBTREE."
  (unless (derived-mode-p 'org-mode) (user-error "Visit a Gnosis note first"))
  (let ((owner (gnosis-org-get-id)))
    (unless owner (user-error "No Org ID owns point"))
    (if (not subtree) (list owner)
      (delete-dups
       (delq nil (cons owner
                       (org-with-wide-buffer
                        (unless (org-before-first-heading-p) (org-narrow-to-subtree))
                        (org-map-entries (lambda () (org-entry-get nil "ID"))))))))))

(defun gnosis-weak--batch (nodes mode)
  "Return (IDS . ITEMS) weak themata of NODES for MODE, weakest first.
In `due' MODE keep only themata due today."
  (let* ((today (gnosis--today-int))
         (items (sort (apply #'append
                             (mapcar (lambda (node) (plist-get node :items))
                                     (gnosis-weak-nodes nodes nil today)))
                      (lambda (a b) (> (plist-get a :score) (plist-get b :score)))))
         (seen (make-hash-table :test #'eql))
         unique)
    (dolist (item items)
      (unless (or (gethash (plist-get item :thema) seen)
                  (and (eq mode 'due) (> (plist-get item :due-day) today)))
        (puthash (plist-get item :thema) t seen)
        (push item unique)))
    (setq unique (nreverse unique))
    (cons (mapcar (lambda (item) (plist-get item :thema)) unique) unique)))

(defun gnosis-weak--start (nodes mode)
  "Confirm and start a MODE batch of the weak themata of NODES."
  (require 'gnosis-review)
  (when gnosis-review--running (user-error "Finish the active review first"))
  (let* ((target (gnosis-review--session-target))
         (buffer (current-buffer))
         (batch (gnosis-weak--batch nodes mode))
         (ids (car batch))
         (title (or (gnosis-get 'title 'nodes `(= id ,(car nodes))) "this note"))
         (scope (if (cdr nodes) (format "%s and %d subnodes" title (1- (length nodes))) title)))
    (if (null ids)
        (message "No %sweak themata in %s" (if (eq mode 'due) "due " "") scope)
      (when (y-or-n-p
             (format "%s %d weak %s of %s, weakest first (“%s”)? "
                     (if (eq mode 'practice) "Practise without rescheduling"
                       "Review due (reschedules)")
                     (length ids) (if (cdr ids) "themata" "thema") scope
                     (plist-get (cadr batch) :keimenon)))
        (unwind-protect (gnosis-review-loop ids mode target)
          (when (buffer-live-p buffer)
            (with-current-buffer buffer
              (when gnosis-weak-mode (gnosis-weak-refresh)))))))))

;;;###autoload
(defun gnosis-weak-practice (&optional subtree)
  "Practise the weak themata of the node at point without rescheduling.
With prefix SUBTREE, include descendant nodes.  Weakest themata come first."
  (interactive "P")
  (gnosis-weak--start (gnosis-weak--nodes-at-point subtree) 'practice))

;;;###autoload
(defun gnosis-weak-review (&optional subtree)
  "Review the due weak themata of the node at point with normal scheduling.
With prefix SUBTREE, include descendant nodes.  Not-due themata are skipped."
  (interactive "P")
  (gnosis-weak--start (gnosis-weak--nodes-at-point subtree) 'due))

;;; Minor mode

(defvar-keymap gnosis-weak-mode-map
  :doc "Keymap for `gnosis-weak-mode'."
  "C-c C-=" #'gnosis-weak-review)

;;;###autoload
(define-minor-mode gnosis-weak-mode
  "Mark nodes whose linked themata are weak, without changing the note.
A right-aligned dot with the weak count follows each weak node's heading:
filled when severe, hollow otherwise; healthy nodes show nothing.  Hover,
or use \\[display-local-help] on the heading, for details.  Scores are read
at idle time after opening, saving or showing the note; editing only
re-anchors the indicators.
\\{gnosis-weak-mode-map}"
  :lighter nil
  :keymap gnosis-weak-mode-map
  (if gnosis-weak-mode
      (progn
        (add-hook 'after-save-hook #'gnosis-weak--after-save 90 t)
        (add-hook 'window-buffer-change-functions #'gnosis-weak--window-refresh nil t)
        (add-hook 'kill-buffer-hook #'gnosis-weak--kill nil t)
        (add-hook 'change-major-mode-hook #'gnosis-weak--kill nil t)
        (add-hook 'clone-indirect-buffer-hook #'gnosis-weak--forget-clone nil t)
        ;; Never block opening the note on the database.
        (gnosis-weak--register t)
        (gnosis-weak--schedule 'refresh))
    (remove-hook 'after-save-hook #'gnosis-weak--after-save t)
    (remove-hook 'window-buffer-change-functions #'gnosis-weak--window-refresh t)
    (remove-hook 'kill-buffer-hook #'gnosis-weak--kill t)
    (remove-hook 'change-major-mode-hook #'gnosis-weak--kill t)
    (remove-hook 'clone-indirect-buffer-hook #'gnosis-weak--forget-clone t)
    (gnosis-weak--kill)
    (setq gnosis-weak--refreshed-at nil gnosis-weak--nodes nil)
    (gnosis-weak--clear)))

;;;###autoload
(defun gnosis-weak-maybe-enable ()
  "Enable `gnosis-weak-mode' in Org notes under `gnosis-nodes-dir'.
Journal files are excluded.  `gnosis' adds this to `gnosis-nodes-mode-hook';
customize `gnosis-weak-indicators' to turn the indicators off."
  (when (and gnosis-weak-indicators gnosis-nodes-mode buffer-file-name
             (derived-mode-p 'org-mode)
             (file-in-directory-p buffer-file-name gnosis-nodes-dir)
             (not (gnosis-nodes--journal-buffer-p)))
    (gnosis-weak-mode 1)))

(add-hook 'gnosis-practice-completed-hook #'gnosis-weak--refresh-all)

(provide 'gnosis-weak)
;;; gnosis-weak.el ends here
