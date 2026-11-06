;;; romacs-attach.el --- Attachments: items, consumers, the tray and the viewer  -*- lexical-binding: t -*-
;; SPDX-License-Identifier: GPL-3.0-or-later
;; Maintainer: Thanos Apollo <public@thanosapollo.org>

;;; Commentary:
;; Attaching files to what a composer sends (design: attachments.md).
;;
;; An item is a plist naming a local file (`romacs-attach-item'); every
;; operation on a list of items returns a new list.  A consumer (an agent
;; harness, a chat, mail) declares what it takes with
;; `romacs-attach-define-consumer'; `romacs-attach-plan' maps items to
;; kinds or refusals, purely; `romacs-attach-prepare' makes outgoing
;; copies with the `romacs-attach-tool' helper, off the evaluator.
;;
;; A tray host (`romacs-attach-tray-setup') shows its items as chips above
;; its composer: a cached string, rebuilt only when the items or their
;; plan change, nil while there are none.  `romacs-attach-edit' selects,
;; moves, removes and captions chips through a transient map; the viewer
;; (`romacs-attach-view', a `viewer' popup) shows one full size.
;;
;; No attachment bytes pass through Lisp: types are sniffed from 256
;; bytes, pixel sizes come from `romacs-image-header-size', thumbnails are
;; `:file' images decoded off the evaluator, copies are the helper's.

;;; Code:

(eval-when-compile (require 'cl-lib))
(require 'seq)
(require 'subr-x)
(require 'romacs-popup)
(require 'romacs-icons)
(require 'romacs-key-hint)

(declare-function mailcap-extension-to-mime "mailcap" (extn))

(defgroup romacs-attach nil
  "Attaching files to what a composer sends."
  :group 'romacs-popup)

(defcustom romacs-attach-jobs 4
  "Most helper processes preparing outgoing copies at once."
  :type 'natnum)

(defface romacs-attach-chip '((t :inherit romacs-layer-glass))
  "An attachment chip in a tray.")

(defface romacs-attach-selected '((t :inherit romacs-layer-selection))
  "The chip edit mode selected.")

(defface romacs-attach-badge '((t :inherit romacs-popup-dim))
  "A chip's badge: what the plan does with it.")

(defface romacs-attach-refused '((t :inherit romacs-layer-toast-error))
  "A refused chip's badge.")

;;;; Items

(defun romacs-attach--id ()
  "A random item id, unique within the session in practice."
  (format "%06x" (random #x1000000)))

(defun romacs-attach--mime (file)
  "FILE's MIME type: its image signature (256 bytes), else its extension,
else `application/octet-stream'."
  (let ((type (ignore-errors (image-type-from-file-header file))))
    (cond
     ((eq type 'svg) "image/svg+xml")
     ((eq type 'xpm) "image/x-xpixmap")
     ((eq type 'xbm) "image/x-xbitmap")
     ((eq type 'pbm) "image/x-portable-anymap")
     (type (format "image/%s" type))
     (t (or (and (require 'mailcap nil t) (file-name-extension file)
                 (mailcap-extension-to-mime (file-name-extension file)))
            "application/octet-stream")))))

(defun romacs-attach--identity (file)
  "FILE's size and modification time, as an item records them."
  (let ((attributes (file-attributes file)))
    (cons (file-attribute-size attributes) (float-time (file-attribute-modification-time attributes)))))

(defun romacs-attach-item (file &rest properties)
  "An item for the local regular FILE, with PROPERTIES (`:owned',
`:source', `:origin', `:caption', `:name') over the sniffed ones."
  (let ((file (expand-file-name file)))
    (when (file-remote-p file) (user-error "Attach a local file"))
    (unless (file-regular-p file) (user-error "Not a regular file: %s" file))
    (let* ((mime (romacs-attach--mime file))
           (identity (romacs-attach--identity file)))
      (append properties
              (list :id (romacs-attach--id) :path file :name (file-name-nondirectory file)
                    :mime mime :bytes (car identity) :identity identity
                    :pixels (and (string-prefix-p "image/" mime) (fboundp 'romacs-image-header-size)
                                 (romacs-image-header-size file))
                    :source 'file)))))

(defun romacs-attach-changed-p (item)
  "Non-nil when ITEM's file is not what was attached (size or time)."
  (not (equal (ignore-errors (romacs-attach--identity (plist-get item :path))) (plist-get item :identity))))

;; Pure list operations: each returns a new list.

(defun romacs-attach-add (items new)
  "ITEMS followed by the items NEW."
  (append items new))

(defun romacs-attach-remove (items id)
  "ITEMS without the item ID."
  (seq-remove (lambda (item) (equal (plist-get item :id) id)) items))

(defun romacs-attach-move (items id delta)
  "ITEMS with the item ID moved DELTA places, within the list."
  (let* ((from (seq-position items id (lambda (item id) (equal (plist-get item :id) id))))
         (to (and from (max 0 (min (1- (length items)) (+ from delta))))))
    (if (or (null from) (= from to)) items
      (let ((item (nth from items)) (rest (romacs-attach-remove items id)))
        (append (seq-take rest to) (list item) (seq-drop rest to))))))

(defun romacs-attach-caption (items id caption)
  "ITEMS with the item ID captioned CAPTION (nil for none)."
  (mapcar (lambda (item)
            (if (equal (plist-get item :id) id) (plist-put (copy-sequence item) :caption caption) item))
          items))

(defun romacs-attach-delete-owned (items)
  "Delete the files of ITEMS that Romacs made (`:owned')."
  (dolist (item items)
    (when (plist-get item :owned) (ignore-errors (delete-file (plist-get item :path))))))

;;;; Owners' places

(defun romacs-attach--size (bytes)
  "BYTES as a person reads a file's size: 307 B, 1.4 MB."
  (file-size-human-readable bytes 'si " " "B"))


(defun romacs-attach-cache-directory ()
  "Where the attach cache lives: $XDG_CACHE_HOME/romacs/attach/."
  (expand-file-name "romacs/attach/" (or (getenv "XDG_CACHE_HOME") (expand-file-name "~/.cache"))))

(defun romacs-attach-staging-directory ()
  "This process's staging directory, for owned files of hosts without
saved drafts, made (mode 0700) when missing."
  (let ((dir (expand-file-name (format "staging/%d/" (emacs-pid)) (romacs-attach-cache-directory))))
    (unless (file-directory-p dir)
      (with-file-modes #o700 (make-directory dir t)))
    dir))

(defun romacs-attach-reconcile (directory items)
  "Delete the files in DIRECTORY (an owner's own, such as a chat's
drafts) that no item of ITEMS names: one listing, nothing by age."
  (let ((named (make-hash-table :test 'equal :size (length items))))
    (dolist (item items) (puthash (expand-file-name (plist-get item :path)) t named))
    (dolist (file (directory-files directory t directory-files-no-dot-files-regexp t))
      (unless (or (gethash file named) (file-directory-p file))
        (delete-file file)))))

;;;; Consumers

(defvar romacs-attach--consumers (make-hash-table :test 'eq)
  "NAME -> its declaration (a plist), from `romacs-attach-define-consumer'.")

;;;###autoload
(cl-defun romacs-attach-define-consumer (name &key display (style 'tray) rules max-count max-total
                                              paste-text-limit insert)
  "Declare the consumer NAME, shown as DISPLAY.  STYLE `tray' keeps items
for the host's send; `inline' inserts each at attach with INSERT, called
with the item and the context.  RULES are ((MIME-REGEXP KIND . LIMITS)
...), the first match winning, or a function of the context returning
them; KIND is one of `image', `document', `text', `path', `upload' and
`file'; LIMITS hold `:max-bytes', `:max-edge', `:formats', `:still' and
`:strip'.  MAX-COUNT and MAX-TOTAL bound a message's items and bytes."
  (puthash name (list :display (or display (symbol-name name)) :style style :rules rules
                      :max-count max-count :max-total max-total
                      :paste-text-limit paste-text-limit :insert insert)
           romacs-attach--consumers)
  name)

(defun romacs-attach-consumer (name)
  "NAME's declaration, or an error."
  (or (gethash name romacs-attach--consumers) (error "No attach consumer %S" name)))

(defun romacs-attach--rules (declaration context)
  "DECLARATION's rules for CONTEXT."
  (let ((rules (plist-get declaration :rules)))
    (if (functionp rules) (funcall rules context) rules)))

(defconst romacs-attach--kind-names
  '((image . "images") (document . "documents") (text . "text") (path . "file paths")
    (upload . "files") (file . "files"))
  "How a refusal names what a consumer takes.")

(defun romacs-attach--takes (display rules)
  "DISPLAY takes the kinds RULES name, as words."
  (let* ((names (delete-dups (mapcar (lambda (rule) (alist-get (cadr rule) romacs-attach--kind-names)) rules)))
         (names (delq nil names)))
    (format "%s takes %s; this is neither" display
            (pcase names
              ('() "nothing")
              (`(,one) one)
              (_ (concat (string-join (butlast names) ", ") " and " (car (last names))))))))

(defun romacs-attach--subtype (mime)
  "The image format MIME names, as the helper's `--formats' names it."
  (pcase mime
    ("image/jpeg" "jpeg") ("image/png" "png") ("image/gif" "gif") ("image/webp" "webp")
    ((pred (string-prefix-p "image/")) (substring mime 6))))

(defun romacs-attach--conversion (item limits)
  "The badge text when the plan's LIMITS change image ITEM, or nil."
  (let* ((formats (plist-get limits :formats))
         (edge (plist-get limits :max-edge))
         (pixels (plist-get item :pixels))
         (format (romacs-attach--subtype (plist-get item :mime)))
         (other (and formats (not (member format formats)) (upcase (car formats))))
         (smaller (and edge pixels (> (max (car pixels) (cdr pixels)) edge) edge)))
    (cond ((and other smaller) (format "→ %s %d px" other smaller))
          (other (format "→ %s" other))
          (smaller (format "→ %d px" smaller)))))

(defun romacs-attach-plan (consumer context items)
  "What CONSUMER, in CONTEXT, does with each of ITEMS, purely: a list of
\(ITEM :kind KIND :prep LIMITS) or (ITEM :refused REASON), in order."
  (let* ((declaration (romacs-attach-consumer consumer))
         (display (plist-get declaration :display))
         (rules (romacs-attach--rules declaration context))
         (max-count (plist-get declaration :max-count))
         (max-total (plist-get declaration :max-total))
         (count 0) (total 0))
    (mapcar
     (lambda (item)
       (let* ((mime (plist-get item :mime))
              (bytes (or (plist-get item :bytes) 0))
              (rule (seq-find (lambda (rule) (string-match-p (car rule) mime)) rules))
              (kind (cadr rule))
              (limits (cddr rule)))
         (cond
          ((null rule) (list item :refused (romacs-attach--takes display rules)))
          ((and (not (eq kind 'image)) (plist-get limits :max-bytes) (> bytes (plist-get limits :max-bytes)))
           (list item :refused (format "%s takes files up to %s" display
                                       (romacs-attach--size (plist-get limits :max-bytes)))))
          ((and max-count (>= count max-count))
           (list item :refused (format "%s takes at most %d attachments" display max-count)))
          ((and max-total (> (+ total bytes) max-total))
           (list item :refused (format "%s takes at most %s in one message" display
                                       (romacs-attach--size max-total))))
          (t (setq count (1+ count) total (+ total bytes))
             (append (list item :kind kind :prep limits)
                     (when-let* (((eq kind 'image)) (badge (romacs-attach--conversion item limits)))
                       (list :badge badge)))))))
     items)))

;;;; Preparing outgoing copies

(defcustom romacs-attach-tool-program nil
  "The `romacs-attach-tool' helper; nil for the one beside the running
binary, else on PATH."
  :type '(choice (const nil) file))

(defun romacs-attach--program ()
  "The helper `romacs-attach-tool-program' names, else the one beside the
running binary, else on PATH."
  (let ((beside (expand-file-name "romacs-attach-tool" invocation-directory)))
    (cond (romacs-attach-tool-program)
          ((file-executable-p beside) beside)
          (t (or (executable-find "romacs-attach-tool") "romacs-attach-tool")))))

(defun romacs-attach--max-image-size ()
  "GNU's `max-image-size' as the helper's W,H, or nil without a limit."
  (cond ((integerp max-image-size) (format "%d,%d" max-image-size max-image-size))
        ((floatp max-image-size)
         (format "%d,%d" (floor (* max-image-size (frame-pixel-width)))
                 (floor (* max-image-size (frame-pixel-height)))))))

(defun romacs-attach--arguments (item kind limits directory)
  "The helper's command line preparing ITEM as KIND under LIMITS into
DIRECTORY."
  (let ((size (and (eq kind 'image) (romacs-attach--max-image-size))))
    `("prepare" ,(plist-get item :path) "--into" ,directory
      ,@(when-let* ((edge (plist-get limits :max-edge))) (list "--max-edge" (number-to-string edge)))
      ,@(when-let* ((bytes (and (eq kind 'image) (plist-get limits :max-bytes))))
          (list "--max-bytes" (number-to-string bytes)))
      ,@(when-let* ((formats (plist-get limits :formats))) (list "--formats" (string-join formats ",")))
      ,@(and (plist-get limits :still) '("--still"))
      ,@(and (plist-get limits :strip) '("--strip"))
      ,@(and (eq kind 'text) '("--text"))
      ,@(and size (list "--max-image-size" size)))))

(defun romacs-attach--prepared-p (entry)
  "Whether plan ENTRY needs an outgoing copy: a planned item whose kind
sends bytes (not a path reference)."
  (and (plist-get (cdr entry) :kind) (not (memq (plist-get (cdr entry) :kind) '(path file)))))

(defun romacs-attach-prepare (plan directory callback)
  "Make the outgoing copies PLAN needs in DIRECTORY with the helper, at
most `romacs-attach-jobs' at once, then call CALLBACK with the plan, each
prepared item gaining `:sent-path', `:sha256' and `:sent-mime', or with
\(:error MESSAGE) on the first failure.  Returns a handle for
`romacs-attach-cancel'."
  (let* ((entries (vconcat plan))
         (pending (seq-filter (lambda (i) (romacs-attach--prepared-p (aref entries i)))
                              (number-sequence 0 (1- (length entries)))))
         (handle (list :processes nil :done nil))
         (running 0))
    (cl-labels
        ((finish (result)
           (unless (plist-get handle :done)
             (plist-put handle :done t)
             (romacs-attach-cancel handle)
             (funcall callback result)))
         (start ()
           (while (and pending (< running romacs-attach-jobs) (not (plist-get handle :done)))
             (let* ((index (pop pending))
                    (entry (aref entries index))
                    (output (generate-new-buffer " *romacs-attach-tool*" t)))
               (setq running (1+ running))
               (push (make-process
                      :name "romacs-attach-tool" :buffer output :noquery t :connection-type 'pipe
                      :command (cons (romacs-attach--program)
                                     (romacs-attach--arguments (car entry) (plist-get (cdr entry) :kind)
                                                               (plist-get (cdr entry) :prep) directory))
                      :stderr (get-buffer-create " *romacs-attach-tool-errors*")
                      :sentinel
                      (lambda (process _event)
                        (unless (process-live-p process)
                          (setq running (1- running))
                          (let ((reply (with-current-buffer output
                                         (ignore-errors
                                           (goto-char (point-min))
                                           (json-parse-buffer :object-type 'plist :null-object nil))))
                                (status (process-exit-status process)))
                            (kill-buffer output)
                            (cond
                             ((plist-get handle :done))
                             ((or (/= status 0) (null reply) (plist-get reply :error))
                              (finish (list :error (or (plist-get reply :error)
                                                       (format "romacs-attach-tool exited %d" status)))))
                             (t
                              (aset entries index
                                    (append (list (append (car entry)
                                                          (list :sent-path (plist-get reply :path)
                                                                :sha256 (plist-get reply :sha256)
                                                                :sent-mime (or (plist-get reply :mime)
                                                                               (plist-get (car entry) :mime)))))
                                            (cdr entry)))
                              (if (or pending (> running 0)) (start) (finish (append entries nil)))))))))
                     (plist-get handle :processes))))))
      ;; Nothing to copy: delivered through `finish' too, so a cancel stops it.
      (if pending (start) (run-at-time 0 nil #'finish plan))
      handle)))

(defun romacs-attach-cancel (handle)
  "Stop the preparation HANDLE: its helpers are killed, its callback not
called."
  (plist-put handle :done t)
  (dolist (process (plist-get handle :processes))
    (when (process-live-p process) (delete-process process))))
;;;; The tray

(defvar-local romacs-attach-items nil
  "The current buffer's tray: its items, in order.")

(defvar-local romacs-attach--tray nil
  "The current buffer's tray host: a plist of `:consumer' (a symbol, or a
function returning (SYMBOL . CONTEXT)), `:anchor', `:redisplay', and the
cached `:plan', `:string' and `:overlay'.")

(defvar-local romacs-attach--selected nil
  "The id of the chip edit mode selected, or nil.")

(defvar-local romacs-attach--removed nil
  "Removals edit mode can undo, newest first: (ITEMS-BEFORE . ITEM).
Their owned files are deleted when the mode ends.")

(defvar romacs-attach--swept nil
  "Whether this session ran the startup sweep of the attach cache.")

(defcustom romacs-attach-received-budget (* 512 1024 1024)
  "Bytes of received images the cache keeps; past it, a sweep cuts it to
80%, least recently used first."
  :type 'natnum)

(defcustom romacs-attach-received-max-files 4096
  "Received images the cache keeps; past it, a sweep cuts it to 80%."
  :type 'natnum)

(defun romacs-attach--sweep ()
  "Sweep the attach cache once a session, off the evaluator: dead
processes' staging directories, and received/ over its bounds."
  (unless romacs-attach--swept
    (setq romacs-attach--swept t)
    (ignore-errors
      (make-process :name "romacs-attach-sweep" :noquery t :buffer nil :connection-type 'pipe
                    :command (list (romacs-attach--program) "sweep" (romacs-attach-cache-directory)
                                   "--budget" (number-to-string romacs-attach-received-budget)
                                   "--max-files" (number-to-string romacs-attach-received-max-files))))))

;;;###autoload
(cl-defun romacs-attach-tray-setup (&key consumer anchor redisplay)
  "Make the current buffer a tray host for CONSUMER (a symbol, or a
function returning (SYMBOL . CONTEXT)).  With REDISPLAY, a function, the
host draws `romacs-attach-tray-string' itself and REDISPLAY runs when it
changes; else the tray shows before ANCHOR, a marker.  Nothing is
installed until the first item arrives."
  (setq romacs-attach--tray (list :consumer consumer :anchor anchor :redisplay redisplay))
  (add-hook 'kill-buffer-hook #'romacs-attach--killed nil t))

(defun romacs-attach--killed ()
  "The tray's owned files go with its buffer."
  (romacs-attach-delete-owned romacs-attach-items)
  (romacs-attach-delete-owned (mapcar #'cdr romacs-attach--removed)))

(defun romacs-attach--consumer ()
  "The tray's (CONSUMER . CONTEXT)."
  (let ((consumer (plist-get romacs-attach--tray :consumer)))
    ;; A declared consumer's name, though also a command's, is the name.
    (if (gethash consumer romacs-attach--consumers) (cons consumer nil) (funcall consumer))))

(defun romacs-attach-replan ()
  "Plan the tray's items again for its consumer's current context (a
harness switch), and redraw it."
  (romacs-attach-set-items romacs-attach-items))

(defun romacs-attach-set-items (items)
  "Install ITEMS as the current buffer's tray: plan them, drop the cached
string, and redraw it through the host."
  (unless romacs-attach--tray (error "Not a tray host: %s" (buffer-name)))
  (romacs-attach--sweep)
  (setq romacs-attach-items items)
  (let ((consumer (romacs-attach--consumer)))
    (plist-put romacs-attach--tray :plan (and items (romacs-attach-plan (car consumer) (cdr consumer) items))))
  (plist-put romacs-attach--tray :string nil)
  (romacs-attach--redraw))

(defun romacs-attach--redraw ()
  "Show the tray's string: through the host's function, or on its own
overlay at the anchor, made with the first item."
  (if-let* ((redisplay (plist-get romacs-attach--tray :redisplay)))
      (funcall redisplay)
    (let ((overlay (plist-get romacs-attach--tray :overlay))
          (string (romacs-attach-tray-string)))
      (when (and string (not (and overlay (overlay-buffer overlay))))
        (let ((anchor (plist-get romacs-attach--tray :anchor)))
          (setq overlay (make-overlay anchor anchor nil nil nil))
          (plist-put romacs-attach--tray :overlay overlay)))
      (when (and overlay (overlay-buffer overlay))
        (overlay-put overlay 'before-string string)))))

(defun romacs-attach-tray-string ()
  "The current buffer's tray as a string of chips ending in a newline,
cached until its items or plan change; nil without items."
  (when romacs-attach-items
    (or (plist-get romacs-attach--tray :string)
        (let ((string (romacs-attach--render-tray)))
          (plist-put romacs-attach--tray :string string)
          string))))

;;;; Chips

(defcustom romacs-attach-label-width 28
  "Columns a chip's file name takes at most; a longer one loses its middle."
  :type 'natnum)

(defun romacs-attach--elide (name width)
  "NAME in at most WIDTH columns, its middle elided and its extension kept."
  (if (<= (string-width name) width) name
    (let* ((extension (file-name-extension name t))
           (stem (file-name-sans-extension name))
           (room (max 2 (- width (string-width extension) 1)))
           (head (truncate-string-to-width stem (ceiling room 2)))
           (tail (romacs-attach--tail stem (/ room 2))))
      (concat head "…" tail extension))))

(defun romacs-attach--tail (string width)
  "The end of STRING in at most WIDTH columns."
  (let ((start (length string)))
    (while (and (> start 0) (<= (string-width string (1- start)) width))
      (setq start (1- start)))
    (substring string start)))

(defun romacs-attach--icon-name (mime)
  "The `romacs-icon' for a file of MIME type."
  (cond ((string-prefix-p "image/" mime) 'image)
        ((string-prefix-p "text/" mime) 'text)
        ((member mime '("application/json")) 'json)
        ((member mime '("application/pdf")) 'info)
        ((string-match-p "zip\\|tar\\|compressed\\|7z\\|rar" mime) 'data)
        (t 'default)))

(defun romacs-attach--thumbnail (item)
  "A thumbnail of ITEM: a `:file' image the header probe sized, decoded
off the evaluator at chip size; else its type's icon."
  (if (plist-get item :pixels)
      (propertize " " 'display (create-image (plist-get item :path) nil nil
                                             :max-height '(2 . em) :max-width '(4 . em)
                                             :ascent 'center :margin 2))
    (or (romacs-icon (romacs-attach--icon-name (plist-get item :mime)) 'romacs-attach-chip) "▫")))

(defun romacs-attach--chip (entry selected)
  "Plan ENTRY's chip, SELECTED or not."
  (let* ((item (car entry))
         (pixels (plist-get item :pixels))
         (size (concat (and pixels (format "%d×%d · " (car pixels) (cdr pixels)))
                       (romacs-attach--size (or (plist-get item :bytes) 0))))
         (refused (plist-get (cdr entry) :refused))
         (badge (cond (refused (propertize (concat " ⚠ " refused) 'face 'romacs-attach-refused))
                      ((romacs-attach-changed-p item) (propertize " changed" 'face 'romacs-attach-badge))
                      ((plist-get (cdr entry) :badge)
                       (propertize (concat " " (plist-get (cdr entry) :badge)) 'face 'romacs-attach-badge))))
         (caption (and (plist-get item :caption) (concat " “" (plist-get item :caption) "”"))))
    (propertize
     (concat (romacs-attach--thumbnail item) " "
             (romacs-attach--elide (or (plist-get item :name) "") romacs-attach-label-width)
             caption " " (propertize size 'face 'romacs-attach-badge) badge " ")
     'face (if selected 'romacs-attach-selected 'romacs-attach-chip)
     'romacs-attach-id (plist-get item :id)
     'help-echo (format "%s\n%s" (plist-get item :path) (plist-get item :mime)))))

(defcustom romacs-attach-tray-max-chips 24
  "Most chips a tray shows: the rest are counted in a last chip, and edit
mode's selection brings them into view.  Redisplay's cost per key follows
the chips shown, not the items."
  :type 'natnum)

(defun romacs-attach--shown (plan selected limit)
  "The slice of PLAN a tray of LIMIT chips shows: from the start, or
ending at the SELECTED entry when it is further; and how many are left
out before and after it."
  (let* ((index (or (seq-position plan selected (lambda (entry id) (equal (plist-get (car entry) :id) id))) 0))
         (start (max 0 (- index (1- limit))))
         (shown (seq-take (seq-drop plan start) limit)))
    (list shown start (- (length plan) start (length shown)))))

(defun romacs-attach--render-tray ()
  "The tray's chips, one string: at most `romacs-attach-tray-max-chips',
the others counted."
  (pcase-let ((`(,shown ,before ,after)
               (romacs-attach--shown (plist-get romacs-attach--tray :plan) romacs-attach--selected
                                     romacs-attach-tray-max-chips)))
    (concat (and (> before 0) (propertize (format "+%d  " before) 'face 'romacs-attach-badge))
            (mapconcat (lambda (entry)
                         (romacs-attach--chip entry (equal (plist-get (car entry) :id) romacs-attach--selected)))
                       shown "  ")
            (and (> after 0) (propertize (format "  +%d more" after) 'face 'romacs-attach-badge))
            "\n")))

;;;; Attaching

(defun romacs-attach-files (files &rest properties)
  "Attach FILES, with item PROPERTIES, to the current tray in one batch:
planned and redrawn once."
  (romacs-attach-set-items
   (romacs-attach-add romacs-attach-items (mapcar (lambda (file) (apply #'romacs-attach-item file properties)) files))))

(defun romacs-attach--here ()
  "The consumer the current buffer attaches to, (CONSUMER . CONTEXT): its
tray's, else mail's or org's by its mode, else nil."
  (cond (romacs-attach--tray (romacs-attach--consumer))
        ((derived-mode-p 'message-mode) (cons 'message nil))
        ((derived-mode-p 'org-mode) (cons 'org nil))))

;;;###autoload
(defun romacs-attach-file (file)
  "Attach FILE to the current buffer's tray, or insert it in an inline
consumer's buffer (mail, org) at point."
  (interactive (list (read-file-name "Attach file: " nil nil t)))
  (let* ((consumer (or (romacs-attach--here) (user-error "Nothing here takes attachments")))
         (declaration (romacs-attach-consumer (car consumer))))
    (if (eq (plist-get declaration :style) 'inline)
        (funcall (plist-get declaration :insert) (romacs-attach-item file) (cdr consumer))
      (romacs-attach-files (list file)))))

;;;; Sending

(defun romacs-attach--now (item)
  "ITEM with its file's size as it is now: a send takes the file as it is."
  (let ((identity (ignore-errors (romacs-attach--identity (plist-get item :path)))))
    (if (or (null identity) (equal identity (plist-get item :identity))) item
      (append (list :bytes (car identity) :identity identity) item))))

(defun romacs-attach-take ()
  "The tray's plan for a send, made now from the files as they are and the
consumer's current rules, and a function that clears the tray once the
consumer owns the message: (PLAN . COMMIT).  A send that fails never
calls COMMIT and leaves the tray as it was."
  (let* ((buffer (current-buffer))
         (consumer (romacs-attach--consumer))
         (plan (romacs-attach-plan (car consumer) (cdr consumer) (mapcar #'romacs-attach--now romacs-attach-items))))
    (cons plan
          (lambda ()
            (when (buffer-live-p buffer)
              (with-current-buffer buffer
                (romacs-attach-delete-owned (mapcar #'car plan))
                (romacs-attach-set-items nil)))))))

;;;; Edit mode

(defun romacs-attach--index ()
  "The selected chip's index in the tray."
  (seq-position romacs-attach-items romacs-attach--selected
                (lambda (item id) (equal (plist-get item :id) id))))

(defun romacs-attach--select (id)
  "Select chip ID and redraw."
  (setq romacs-attach--selected id)
  (plist-put romacs-attach--tray :string nil)
  (romacs-attach--redraw))

(defun romacs-attach-edit-next (n)
  "Select the chip N after the selected one."
  (interactive "p")
  (when romacs-attach-items
    (let ((index (max 0 (min (1- (length romacs-attach-items)) (+ (or (romacs-attach--index) 0) n)))))
      (romacs-attach--select (plist-get (nth index romacs-attach-items) :id)))))

(defun romacs-attach-edit-previous (n)
  "Select the chip N before the selected one."
  (interactive "p")
  (romacs-attach-edit-next (- n)))

(defun romacs-attach-edit-move-right (n)
  "Move the selected chip N places right."
  (interactive "p")
  (romacs-attach-set-items (romacs-attach-move romacs-attach-items romacs-attach--selected n)))

(defun romacs-attach-edit-move-left (n)
  "Move the selected chip N places left."
  (interactive "p")
  (romacs-attach-edit-move-right (- n)))

(defun romacs-attach-edit-remove ()
  "Remove the selected chip; `u' brings it back while edit mode lasts."
  (interactive)
  (when-let* ((index (romacs-attach--index)))
    (let ((item (nth index romacs-attach-items)))
      (push (cons romacs-attach-items item) romacs-attach--removed)
      (let ((items (romacs-attach-remove romacs-attach-items (plist-get item :id))))
        (setq romacs-attach--selected (plist-get (nth (min index (1- (length items))) items) :id))
        (romacs-attach-set-items items)))))

(defun romacs-attach-edit-undo ()
  "Bring back the last removed chip."
  (interactive)
  (when-let* ((removal (pop romacs-attach--removed)))
    (setq romacs-attach--selected (plist-get (cdr removal) :id))
    (romacs-attach-set-items (car removal))))

(defun romacs-attach-edit-caption (caption)
  "Caption the selected chip CAPTION (empty for none)."
  (interactive
   ;; The prompt reads with edit mode's keys suspended: its RET ends the read.
   (let ((overriding-terminal-local-map nil))
     (list (read-string "Caption: " (plist-get (nth (or (romacs-attach--index) 0) romacs-attach-items) :caption)))))
  (romacs-attach-set-items (romacs-attach-caption romacs-attach-items romacs-attach--selected
                                                  (and (not (string-empty-p caption)) caption))))

(defun romacs-attach-edit-view ()
  "View the selected chip, its siblings the tray's items."
  (interactive)
  (when-let* ((index (romacs-attach--index)))
    (romacs-attach-view (nth index romacs-attach-items) :siblings romacs-attach-items :context (current-buffer))))

(defun romacs-attach-edit-open ()
  "Visit the selected chip's file in another window."
  (interactive)
  (when-let* ((index (romacs-attach--index)))
    (find-file-other-window (plist-get (nth index romacs-attach-items) :path))))

(defun romacs-attach-edit-quit ()
  "Leave edit mode."
  (interactive))

(defvar-keymap romacs-attach-edit-map
  :doc "Edit mode's keys on a tray's chips."
  "f" #'romacs-attach-edit-next "n" #'romacs-attach-edit-next "C-f" #'romacs-attach-edit-next
  "<right>" #'romacs-attach-edit-next
  "b" #'romacs-attach-edit-previous "p" #'romacs-attach-edit-previous "C-b" #'romacs-attach-edit-previous
  "<left>" #'romacs-attach-edit-previous
  "M-f" #'romacs-attach-edit-move-right "M-b" #'romacs-attach-edit-move-left
  "d" #'romacs-attach-edit-remove "C-d" #'romacs-attach-edit-remove "DEL" #'romacs-attach-edit-remove
  "u" #'romacs-attach-edit-undo "C-/" #'romacs-attach-edit-undo
  "c" #'romacs-attach-edit-caption
  "RET" #'romacs-attach-edit-view "SPC" #'romacs-attach-edit-view "v" #'romacs-attach-edit-view
  "o" #'romacs-attach-edit-open
  "q" #'romacs-attach-edit-quit "C-g" #'romacs-attach-edit-quit)

(defun romacs-attach--in-maps (maps quit)
  "A transient map's keep predicate: the command just read is one of
MAPS' bindings for its keys, and not QUIT; or it runs in a minibuffer
one of them opened (a caption's prompt)."
  (lambda ()
    (or (minibufferp)
        (and (not (eq this-command quit))
             (seq-some (lambda (map) (eq this-command (lookup-key (symbol-value map) (this-command-keys-vector)))) maps)))))

;;;###autoload
(defun romacs-attach-edit (&optional id)
  "Edit the current tray's chips, from chip ID (the last by default).
Keys in `romacs-attach-edit-map' keep the mode; any other leaves it and
runs as usual.  Removed owned files are deleted when it ends."
  (interactive)
  (unless romacs-attach-items (user-error "No attachments"))
  (let ((buffer (current-buffer)))
    (romacs-attach--select (or id (plist-get (car (last romacs-attach-items)) :id)))
    ;; The viewer opened from a chip keeps the mode while it shows.
    (set-transient-map romacs-attach-edit-map
                       (romacs-attach--in-maps '(romacs-attach-edit-map romacs-attach-view-map)
                                               #'romacs-attach-edit-quit)
                       (lambda ()
                         (when (buffer-live-p buffer)
                           (with-current-buffer buffer
                             (romacs-attach-delete-owned (mapcar #'cdr romacs-attach--removed))
                             (setq romacs-attach--removed nil)
                             (romacs-attach--select nil)))))))

(defun romacs-attach-delete-backward (fallback)
  "At ANCHOR's position with items, enter edit mode on the last chip;
else call FALLBACK, the host's own DEL command."
  (if (and romacs-attach-items (= (point) (marker-position (plist-get romacs-attach--tray :anchor))))
      (romacs-attach-edit)
    (call-interactively fallback)))

;;;; The viewer

(defconst romacs-attach--view-buffer " *romacs-attach-view*"
  "The viewer's buffer.")

(defvar romacs-attach--view nil
  "The viewer's state: a plist of `:thing', `:siblings', `:context' and
`:zoom'.")

(defcustom romacs-attach-view-text-bytes 65536
  "Bytes of a text file the viewer shows."
  :type 'natnum)

(defun romacs-attach--thing (thing)
  "THING, an item or a file, as an item."
  (if (stringp thing) (romacs-attach-item thing) thing))

(defun romacs-attach--card (item)
  "The card for ITEM: icon, name, type, size and path."
  (insert (or (romacs-icon (romacs-attach--icon-name (plist-get item :mime))) "") " "
          (propertize (plist-get item :name) 'face 'romacs-popup-title) "\n"
          (propertize (format "%s · %s\n%s" (plist-get item :mime)
                              (romacs-attach--size (or (plist-get item :bytes) 0))
                              (plist-get item :path))
                      'face 'romacs-popup-dim)))

(defun romacs-attach--view-keys ()
  "The viewer's key row."
  (romacs-key-hint "\\<romacs-attach-view-map>\\[romacs-attach-view-next]/\\[romacs-attach-view-previous] next · \\[romacs-attach-view-zoom-in]/\\[romacs-attach-view-zoom-out] zoom · \\[romacs-attach-view-open] open · \\[romacs-attach-view-copy-path] copy path · \\[romacs-attach-view-quit] close"
                   'romacs-popup-dim))

(defun romacs-attach--fit (pixels width height zoom)
  "PIXELS (W . H) fitted inside WIDTH by HEIGHT, never enlarged, then
scaled by ZOOM."
  (let ((scale (* zoom (min 1.0 (/ (float width) (car pixels)) (/ (float height) (cdr pixels))))))
    (cons (max 1 (round (* scale (car pixels)))) (max 1 (round (* scale (cdr pixels)))))))

(defun romacs-attach-render (thing width height &optional zoom)
  "Fill the viewer's buffer with THING (an item or a local file) for a box
of WIDTH by HEIGHT pixels, at ZOOM (1 by default: fitted, never
enlarged), and return the buffer: an image the header probe sizes, the
first `romacs-attach-view-text-bytes' of a text file, or a card."
  (let ((item (romacs-attach--thing thing))
        (buffer (get-buffer-create romacs-attach--view-buffer)))
    (with-current-buffer buffer
      (let ((inhibit-read-only t))
        (erase-buffer)
        (let ((pixels (plist-get item :pixels))
              (mime (plist-get item :mime)))
          (insert (propertize (plist-get item :name) 'face 'romacs-popup-title) "  "
                  (propertize (concat (and pixels (format "%d×%d · " (car pixels) (cdr pixels)))
                                      (romacs-attach--size (or (plist-get item :bytes) 0)))
                              'face 'romacs-popup-dim)
                  "\n")
          (cond
           (pixels
            (insert-image (create-image (plist-get item :path) nil nil
                                        :width (car (romacs-attach--fit pixels width height (or zoom 1.0)))))
            (insert "\n"))
           ((string-prefix-p "text/" mime)
            (insert-file-contents (plist-get item :path) nil 0 romacs-attach-view-text-bytes)
            (goto-char (point-max)))
           (t (romacs-attach--card item) (insert "\n")))
          (insert (romacs-attach--view-keys)))
        (setq-local mode-line-format nil cursor-type nil)
        (setq buffer-read-only t)
        (goto-char (point-min))))
    buffer))

(defun romacs-attach--view-box ()
  "The viewer's image box: 90% of the frame's width, 85% of its height
less the title and key rows."
  (cons (max 16 (round (* 0.9 (frame-pixel-width))))
        (max 16 (- (round (* 0.85 (frame-pixel-height))) (* 2 (frame-char-height))))))

(defun romacs-attach--view-show ()
  "Render and show the viewer's current thing."
  (let* ((box (romacs-attach--view-box))
         (zoom (plist-get romacs-attach--view :zoom))
         (buffer (romacs-attach-render (plist-get romacs-attach--view :thing) (car box) (cdr box) zoom))
         (pixels (plist-get (romacs-attach--thing (plist-get romacs-attach--view :thing)) :pixels))
         (fitted (and pixels (romacs-attach--fit pixels (car box) (cdr box) zoom))))
    (romacs-popup-show 'viewer buffer
                       :columns (and fitted (max (1+ (string-width (romacs-attach--view-keys)))
                                                 (ceiling (car fitted) (frame-char-width))))
                       :lines (and fitted (+ 2 (ceiling (cdr fitted) (frame-char-height)))))))

(defun romacs-attach-view-next (n)
  "Show the sibling N after the viewed one."
  (interactive "p")
  (let* ((siblings (plist-get romacs-attach--view :siblings))
         (index (or (seq-position siblings (plist-get romacs-attach--view :thing)) 0)))
    (when siblings
      (plist-put romacs-attach--view :thing (nth (mod (+ index n) (length siblings)) siblings))
      (romacs-attach--view-show))))

(defun romacs-attach-view-previous (n)
  "Show the sibling N before the viewed one."
  (interactive "p")
  (romacs-attach-view-next (- n)))

(defun romacs-attach-view-zoom-in ()
  "Zoom the viewer in."
  (interactive)
  (plist-put romacs-attach--view :zoom (* 1.25 (plist-get romacs-attach--view :zoom)))
  (romacs-attach--view-show))

(defun romacs-attach-view-zoom-out ()
  "Zoom the viewer out."
  (interactive)
  (plist-put romacs-attach--view :zoom (/ (plist-get romacs-attach--view :zoom) 1.25))
  (romacs-attach--view-show))

(defun romacs-attach-view-zoom-reset ()
  "Fit the viewed image to the frame again."
  (interactive)
  (plist-put romacs-attach--view :zoom 1.0)
  (romacs-attach--view-show))

(defun romacs-attach-view-open ()
  "Visit the viewed file (image-mode for an image)."
  (interactive)
  (let ((path (plist-get (romacs-attach--thing (plist-get romacs-attach--view :thing)) :path)))
    (romacs-attach-view-quit)
    (find-file-other-window path)))

(defun romacs-attach-view-copy-path ()
  "Put the viewed file's path in the kill ring."
  (interactive)
  (let ((path (plist-get (romacs-attach--thing (plist-get romacs-attach--view :thing)) :path)))
    (kill-new path)
    (message "Copied %s" path)))

(defun romacs-attach-view-quit ()
  "Close the viewer, and its keys with it."
  (interactive)
  (let ((exit (plist-get romacs-attach--view :exit)))
    (setq romacs-attach--view nil)
    (romacs-popup-hide romacs-attach--view-buffer)
    (when exit (funcall exit))))

(defvar-keymap romacs-attach-view-map
  :doc "The viewer's keys, kept while it shows."
  "n" #'romacs-attach-view-next "f" #'romacs-attach-view-next
  "p" #'romacs-attach-view-previous "b" #'romacs-attach-view-previous
  "+" #'romacs-attach-view-zoom-in "=" #'romacs-attach-view-zoom-in
  "-" #'romacs-attach-view-zoom-out "0" #'romacs-attach-view-zoom-reset
  "o" #'romacs-attach-view-open "w" #'romacs-attach-view-copy-path
  "q" #'romacs-attach-view-quit "C-g" #'romacs-attach-view-quit "RET" #'romacs-attach-view-quit)

;; The keys its row names, of those that do the same.
(put 'romacs-attach-view-next :advertised-binding [?n])
(put 'romacs-attach-view-previous :advertised-binding [?p])
(put 'romacs-attach-view-zoom-in :advertised-binding [?+])
(put 'romacs-attach-view-quit :advertised-binding [?q])

;;;###autoload
(cl-defun romacs-attach-view (thing &key siblings context)
  "Show THING (an item or a local file) full size in the viewer, with
`n'/`p' over SIBLINGS.  CONTEXT is the tray host it came from, if any.
The viewer never takes focus; its keys are a transient map kept while
they are typed: any other key closes it and runs as usual."
  (setq romacs-attach--view (list :thing thing :siblings siblings :context context :zoom 1.0))
  (romacs-attach--view-show)
  (plist-put romacs-attach--view :exit
             (set-transient-map romacs-attach-view-map
                                (romacs-attach--in-maps '(romacs-attach-view-map) #'romacs-attach-view-quit)
                                (lambda () (when romacs-attach--view (romacs-attach-view-quit))))))

;;;; Mail and org: inline consumers

(defcustom romacs-attach-save-directory nil
  "Where an image Romacs made (a screenshot, a clipboard image) becomes
the user's file when attached in mail; nil for the XDG pictures
directory's Screenshots/, where Omarchy's own screenshots go."
  :type '(choice (const nil) directory))

(defcustom romacs-attach-snapshot-max (* 1024 1024)
  "Bytes of a region or buffer an attachment takes as text."
  :type 'natnum)

(declare-function xdg-user-dir "xdg" (name))
(declare-function mml-attach-file "mml" (file &optional type description disposition))
(declare-function mml-insert-tag "mml" (name &rest plist))
(declare-function mml-quote-region "mml" (beg end))
(declare-function org--dnd-local-file-handler "org" (url action &optional separator))

(defun romacs-attach--save-directory ()
  "The directory owned images go to when attached in mail."
  (file-name-as-directory
   (or romacs-attach-save-directory
       (expand-file-name "Screenshots" (or (and (require 'xdg nil t) (xdg-user-dir "PICTURES")) "~/Pictures")))))

(defun romacs-attach--free-name (directory name)
  "NAME in DIRECTORY, numbered (NAME-1.EXT ...) past the files there."
  (let ((file (expand-file-name name directory)) (n 0))
    (while (file-exists-p file)
      (setq n (1+ n)
            file (expand-file-name (format "%s-%d%s" (file-name-sans-extension name) n
                                           (or (file-name-extension name t) ""))
                                   directory)))
    file))

(defun romacs-attach--users (item)
  "ITEM's file as the user's: an owned one is moved into
`romacs-attach-save-directory' first, and a toast names it."
  (if (not (plist-get item :owned)) (plist-get item :path)
    (let* ((directory (romacs-attach--save-directory))
           (target (progn (make-directory directory t)
                          (romacs-attach--free-name directory (plist-get item :name)))))
      (romacs-attach--move (plist-get item :path) target)
      (message "Saved %s" (abbreviate-file-name target))
      target)))

(defun romacs-attach--move (from to)
  "Move the file FROM to TO: a rename, else (another filesystem, which
Romacs's `rename-file' refuses) a copy and then FROM's deletion; a real
failure is the copy's."
  (condition-case nil
      (rename-file from to)
    (error
     (copy-file from to)
     (delete-file from))))

(defun romacs-attach--mail-insert (item _context)
  "Insert ITEM in a mail draft at point, as GNU's commands do: a text
snapshot (region, buffer, pasted text) as an inline text part quoted as
`mml-insert-buffer' quotes it, any other file through `mml-attach-file'
(inline for an image), an owned one made the user's file first."
  (require 'mml)
  (let ((caption (plist-get item :caption)))
    (if (memq (plist-get item :source) '(region buffer paste-text))
        (progn
          (apply #'mml-insert-tag 'part 'type "text/plain" 'disposition "inline"
                 (and caption (list 'description caption)))
          (let ((start (point))
                (end (copy-marker (point) t)))
            (insert-file-contents (plist-get item :path) nil 0 romacs-attach-snapshot-max)
            (mml-quote-region start end)
            (goto-char end)
            (unless (bolp) (insert "\n")))
          (insert "<#/part>\n")
          (romacs-attach-delete-owned (list item)))
      (let ((mime (plist-get item :mime)))
        (mml-attach-file (romacs-attach--users item) mime caption
                         (if (string-prefix-p "image/" mime) "inline" "attachment"))))))

(defvar org-yank-image-save-method)

(defun romacs-attach--org-insert (item _context)
  "Insert ITEM in an org buffer at point as org's own local-file drop
handler does, by `org-yank-dnd-method' and `org-yank-image-save-method'
\(a function's directory resolved first, which org's handler does not
do), attaching by `org-attach-method'.  An owned file becomes the user's
\(`romacs-attach-save-directory') first, so a link to it stays good."
  (require 'org)
  (let ((org-yank-image-save-method
         (if (functionp org-yank-image-save-method)
             (let ((directory (funcall org-yank-image-save-method)))
               (if (stringp directory) directory
                 (user-error "`org-yank-image-save-method' did not return a string: %S" directory)))
           org-yank-image-save-method)))
    (org--dnd-local-file-handler (concat "file://" (string-replace "%" "%25" (romacs-attach--users item)))
                                 'private "")))

(romacs-attach-define-consumer 'message
  :display "Mail" :style 'inline :rules '(("" file)) :insert #'romacs-attach--mail-insert)

(romacs-attach-define-consumer 'org
  :display "Org" :style 'inline :rules '(("" file)) :insert #'romacs-attach--org-insert)

;;;; A test host

;; A composer-shaped buffer with a tray, for trying attachments and for
;; the GUI journeys: its text is the draft, the tray shows above it, `DEL'
;; at its start enters edit mode, `C-c C-a' attaches a file.

(romacs-attach-define-consumer 'romacs-attach-test-host
  :display "The test host"
  :rules '(("\\`image/" image :max-edge 1568) ("\\`text/" text) ("" path)))

(defun romacs-attach-test-host-delete-backward ()
  "`DEL' in the test host: edit mode at the draft's start with chips."
  (interactive nil romacs-attach-test-host-mode)
  (romacs-attach-delete-backward #'delete-backward-char))

(defvar-keymap romacs-attach-test-host-mode-map
  "DEL" #'romacs-attach-test-host-delete-backward
  "C-c C-a" #'romacs-attach-file)

(define-derived-mode romacs-attach-test-host-mode text-mode "Attach test"
  "A composer with a tray of attachments above its draft."
  (romacs-attach-tray-setup :consumer 'romacs-attach-test-host :anchor (copy-marker (point-min))))

;;;###autoload
(defun romacs-attach-test-host ()
  "Show a composer-shaped buffer that takes attachments."
  (interactive)
  (pop-to-buffer-same-window (get-buffer-create "*attach test*"))
  (unless (derived-mode-p 'romacs-attach-test-host-mode) (romacs-attach-test-host-mode)))

;; Edit mode's and the viewer's keys are a transient map, which M-x would
;; leave: they are only ever typed.
(dolist (command '( romacs-attach-edit-next romacs-attach-edit-previous romacs-attach-edit-move-right
                    romacs-attach-edit-move-left romacs-attach-edit-remove romacs-attach-edit-undo
                    romacs-attach-edit-caption romacs-attach-edit-view romacs-attach-edit-open
                    romacs-attach-edit-quit romacs-attach-view-next romacs-attach-view-previous
                    romacs-attach-view-zoom-in romacs-attach-view-zoom-out romacs-attach-view-zoom-reset
                    romacs-attach-view-open romacs-attach-view-copy-path romacs-attach-view-quit))
  (put command 'completion-predicate #'ignore))

;; With the autoloads, so M-x does not offer them before this file loads,
;; and does not load it to ask.  `romacs-attach--here' says the same.
;;;###autoload
(put 'romacs-attach-file 'completion-predicate
     (lambda (_symbol buffer)
       (with-current-buffer buffer
         (or (bound-and-true-p romacs-attach--tray) (derived-mode-p 'message-mode 'org-mode)))))

;;;###autoload
(put 'romacs-attach-edit 'completion-predicate
     (lambda (_symbol buffer)
       (with-current-buffer buffer (bound-and-true-p romacs-attach-items))))

(provide 'romacs-attach)
;;; romacs-attach.el ends here
