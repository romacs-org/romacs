;;; warm-restart.el --- Carry a session into a new Romacs process  -*- lexical-binding: t -*-

;; SPDX-License-Identifier: GPL-3.0-or-later
;; Maintainer: Thanos Apollo <public@thanosapollo.org>
;; Derived from warm-restart.el (a GNU Emacs package for the same hand-off
;; between two Emacs processes) by Thanos Apollo, version 0.4.1.

;;; Commentary:

;; A session image is the Lisp-visible state a user made: buffers with
;; their unsaved text, point, mark, narrowing, undo and modes; the window
;; layout of each frame; the kill ring, registers, minibuffer histories and
;; `warm-restart-globals'.  A new process, of this build or a newer one,
;; reads it after its own init and recreates that state; all code comes
;; from the new build.  Unsaved text always travels in the image, so a
;; restart never loses it: a restore that fails keeps the image and says
;; where it is.
;;
;; `romacs-restart' writes an image, ends this process and starts the next
;; one with `--restore IMAGE'.  `warm-restart-plan' says, as data, what an
;; image would carry and why each other buffer is not carried; processes,
;; terminals and network connections end with the process for now, and
;; are listed.
;;
;; Packages take part without loading this file:
;; - `warm-restart-blocker-functions' (global or buffer-local) name why a
;;   buffer cannot be handed over now; a restart refuses until none does.
;; - `warm-restart-regenerable' (buffer-local) marks a view its package
;;   rebuilds, which is listed and not carried.
;; - `warm-restart-left-out' (buffer-local) keeps a buffer out of every
;;   image, unsaved edits and all; the restored session names it.
;; - desktop.el's `desktop-save-buffer' and `desktop-buffer-mode-handlers'
;;   carry a buffer as its package's own data instead of its text.
;; - A handler that would attach live resources checks
;;   `warm-restart-passive' and defers to `warm-restart-activate-functions',
;;   which run once every buffer exists.
;;
;; The image is `romacs--session-image-write''s container: one printed
;; plist, read back with `read', and the raw text of each carried buffer
;; beside it.  Buffers are named in it by `warm-restart-buffer-id', never
;; by name.

;;; Code:

(require 'cl-lib)
(require 'subr-x)
(require 'desktop)

(declare-function romacs--session-image-write "session_image.rs" (file form buffers))
(declare-function romacs--session-image-read "session_image.rs" (file))
(declare-function romacs--lead-buffer-list "buffer_names.rs" (buffers))
(declare-function romacs-agent-load-session "romacs-agent" (id))
(declare-function romacs-agent-tools--deliver "romacs-agent-tools" (session text delivered &optional valid))
(declare-function romacs-agent-tools--restart-wake-file "romacs-agent-tools" (session))
(declare-function romacs-agent-mark "romacs-agent" (id kind on))
(declare-function romacs-agent-id "romacs-agent" (session))
(declare-function romacs-startup-progress "animate.rs" (stage &optional label state))
(defvar savehist-minibuffer-history-variables)
(defvar savehist-additional-variables)
(defvar uniquify-managed)

;;;; The protocol packages use

(defvar warm-restart-passive nil
  "Non-nil while a session image is being applied.
Handlers that would attach live resources then add a function to
`warm-restart-activate-functions' instead.")

(defvar warm-restart-activate-functions nil
  "Functions called with no arguments once a restored session's buffers exist.")

(defvar warm-restart-before-handover-functions nil
  "Functions called with no arguments as a restart begins, before its
image is written and its successor started: what must be on disk for the
successor (a queue of writes) is put there, however long it takes.")

(defvar warm-restart-blocker-functions nil
  "Abnormal hook naming why the current buffer cannot be handed over now.
Each function runs in a buffer and returns nil or a string.  May be set
buffer-locally.")

(defvar-local warm-restart-regenerable nil
  "Non-nil when this buffer is a view its package rebuilds.
A string is the reason listed.  A writable buffer with unsaved edits is
carried anyway.")

(defvar-local warm-restart-left-out nil
  "A reason string when this buffer never goes into a session image,
unsaved edits and all: a restart goes ahead without it, and the restored
session names it if its text was unsaved (romacs-ram-files.el).")

(defvar-local warm-restart-buffer-id nil
  "This buffer's identity in session images, made at its first image:
\(ID . BUFFER).  A clone copies the local with the other buffer in it,
so the clone makes an id of its own.")
(put 'warm-restart-buffer-id 'permanent-local t)

;;;; Options

(defgroup warm-restart nil
  "Carrying a session into a new Romacs process."
  :group 'environment)

(defcustom warm-restart-directory nil
  "Directory of session images; nil means romacs/warm-restart/ under the
XDG state directory."
  :type '(choice (const nil) directory))

(defcustom romacs-restart-command nil
  "The program and arguments `romacs-restart' runs, before `--restore IMAGE'.
nil means the environment's ROMACS_RESTART_COMMAND (a launcher that
supervises sessions names itself there), else this executable with this
session's launch flags.  A command given here or there chooses its own
runtime: the variables of `romacs-restart-unset-environment' are removed
from its environment."
  :type '(choice (const nil) (repeat string)))

(defcustom romacs-restart-unset-environment
  '("ROMACS_RUNTIME_ROOT" "ROMACS_LISP_DIR" "ROMACS_USAGE_LOG")
  "Variables removed from the environment of an explicit restart command."
  :type '(repeat string))

(defcustom warm-restart-globals '(recentf-list)
  "Global variables carried in images besides the kill ring, registers and
the minibuffer histories."
  :type '(repeat variable))

(defcustom warm-restart-history-variables
  '(minibuffer-history extended-command-history file-name-history
    buffer-name-history query-replace-history regexp-history
    read-expression-history shell-command-history search-ring
    regexp-search-ring command-history)
  "History variables carried besides savehist's own lists."
  :type '(repeat variable))

(defcustom warm-restart-buffer-properties '(hard)
  "Text properties carried with a buffer's text: those a mode cannot
rebuild, such as `hard' newlines the user typed."
  :type '(repeat symbol))

(defcustom warm-restart-regenerable-buffers
  '("\\`\\*Messages\\*\\'" "\\`\\*Completions\\*\\'" "\\`\\*Help\\*\\'"
    "\\`\\*Warnings\\*\\'" "\\`\\*Backtrace\\*\\'" "\\`\\*Compile-Log\\*\\'"
    "\\`\\*Apropos\\*\\'" "\\`\\*Buffer List\\*\\'" "\\`\\*Ibuffer\\*\\'"
    "\\`\\*Romacs\\*\\'" "\\`\\*Warm Restart\\*\\'")
  "Regexps naming generated buffers that are not carried."
  :type '(repeat regexp))

(defcustom warm-restart-text-modes '(text-mode prog-mode)
  "Mode families whose buffers without a file are carried as text, besides
`fundamental-mode' itself."
  :type '(repeat symbol))

(defconst warm-restart--schema 1 "The image form's schema.")

(defvar romacs-keeper-restarting)
(declare-function romacs-keeper-attached-p "romacs-keeper")
(declare-function romacs-keeper-close-orphans "romacs-keeper")

(defvar warm-restart--problems nil
  "What went wrong in the restore running now, as `lost'-style entries.")

(defvar warm-restart--defer nil
  "Non-nil while a record no window shows is restored: its buffer's modes
wait until it is shown (`warm-restart--wake').")

(defvar-local warm-restart--deferred nil
  "In a buffer restored unseen, (RECORD . FILE): RECORD's major mode,
minor modes and locals wait for it to be shown; FILE non-nil when it
visits a file whose own mode, local variables and `find-file-hook' wait
too.")

(defvar warm-restart--deferred-buffers nil
  "The buffers restored unseen whose modes wait, newest first.")

(defvar warm-restart--wake-timer nil
  "The idle timer that gives the next unseen buffer its modes, or nil.")

(defvar warm-restart-last-report nil
  "What the last restore did, as a plist: the image, the buffers restored,
what the image did not carry (:lost), problems, and timings.")

;;;; Portable data

(defun warm-restart--portable-p (object)
  "Non-nil when OBJECT prints as data that reads back equal.
Markers, buffers, windows, frames, processes, records, compiled code and
the like are not; nor is a hash table of a test other than the built-in
three.  Shared and circular structure is walked once."
  (let ((seen (make-hash-table :test #'eq)))
    (cl-labels
        ((ok (x)
           (cond ((or (numberp x) (symbolp x) (bool-vector-p x)) t)
                 ((gethash x seen) t)
                 (t (puthash x t seen)
                    (cond ((stringp x) (cl-every #'ok (object-intervals x)))
                          ((consp x) (chain x))
                          ((and (vectorp x) (not (recordp x))) (cl-every #'ok x))
                          ((hash-table-p x)
                           (and (memq (hash-table-test x) '(eq eql equal))
                                (catch 'bad
                                  (maphash (lambda (k v) (unless (and (ok k) (ok v)) (throw 'bad nil))) x)
                                  t)))))))
         (chain (x)
           (let ((tail x) (good t))
             (while (and good (consp tail))
               (setq good (ok (car tail)) tail (cdr tail))
               (when (consp tail)
                 (if (gethash tail seen) (setq tail nil) (puthash tail t seen))))
             (and good (ok tail)))))
      (ok object))))

(defun warm-restart--redacted (text)
  "TEXT, a string, without its properties, each character marked
`romacs-secret' replaced by `*', one for one, so every position stays
where it was."
  (let ((pos 0) (end (length text)) pieces)
    (while (< pos end)
      (let ((next (or (next-single-property-change pos 'romacs-secret text) end)))
        (push (if (get-text-property pos 'romacs-secret text)
                  (make-string (- next pos) ?*)
                (substring-no-properties text pos next))
              pieces)
        (setq pos next)))
    (apply #'concat (nreverse pieces))))

(defun warm-restart--holds-secret-p (object)
  "Whether OBJECT, data, holds a secret's text anywhere: a string with
`romacs-secret' text, in a cons, a vector, a record, a hash table's key
or value, or a string's property value.  Shared and circular structure is walked
once, and a list's spine by a loop, however long."
  (let ((seen (make-hash-table :test #'eq)))
    (cl-labels
        ((walk (x)
           (cond ((not (or (stringp x) (consp x) (vectorp x) (recordp x) (hash-table-p x))) nil)
                 ((gethash x seen) nil)
                 (t (puthash x t seen)
                    (cond ((stringp x)
                           (or (text-property-not-all 0 (length x) 'romacs-secret nil x)
                               (cl-some (lambda (interval) (walk (nth 2 interval))) (object-intervals x))))
                          ((consp x)
                           (let ((tail x) found)
                             (while (and (not found) (consp tail))
                               (setq found (walk (car tail)) tail (cdr tail))
                               (when (consp tail)
                                 (if (gethash tail seen) (setq tail nil) (puthash tail t seen))))
                             (or found (walk tail))))
                          ((or (vectorp x) (recordp x)) (cl-loop for i below (length x) thereis (walk (aref x i))))
                          (t (catch 'found
                               (maphash (lambda (k v) (when (or (walk k) (walk v)) (throw 'found t))) x)
                               nil)))))))
      (and (walk object) t))))

(defun warm-restart--without-secrets (object)
  "OBJECT, data, as an image may hold it: OBJECT itself when it holds no
secret (`warm-restart--holds-secret-p'); else a copy, its sharing and
cycles kept, strings' included, in which each string with secret text
is redacted (`warm-restart--redacted') and each other string whose
properties hold one loses its properties.  A hash table's starred keys
may then be equal, and one entry of each such key is kept."
  (if (not (warm-restart--holds-secret-p object))
      object
    (let ((copies (make-hash-table :test #'eq)))
      (cl-labels
          ((copy (x)
             (cond ((not (or (stringp x) (consp x) (vectorp x) (recordp x) (hash-table-p x))) x)
                   ((gethash x copies))
                   ((stringp x)
                    (puthash x (cond ((text-property-not-all 0 (length x) 'romacs-secret nil x) (warm-restart--redacted x))
                                     ((warm-restart--holds-secret-p (object-intervals x)) (substring-no-properties x))
                                     (t x))
                             copies))
                   ((consp x)
                    (let* ((head (cons nil nil)) (cell head) (tail (cdr x)))
                      (puthash x head copies)
                      (setcar head (copy (car x)))
                      (while (and (consp tail) (not (gethash tail copies)))
                        (let ((next (cons nil nil)))
                          (puthash tail next copies)
                          (setcdr cell next)
                          (setq cell next)
                          (setcar next (copy (car tail)))
                          (setq tail (cdr tail))))
                      (setcdr cell (if (consp tail) (gethash tail copies) (copy tail)))
                      head))
                   ((or (vectorp x) (recordp x))
                    (let ((v (copy-sequence x)))
                      (puthash x v copies)
                      (dotimes (i (length x)) (aset v i (copy (aref x i))))
                      v))
                   (t (let ((table (make-hash-table :test (hash-table-test x) :size (max 1 (hash-table-count x)))))
                        (puthash x table copies)
                        (maphash (lambda (k v) (puthash (copy k) (copy v) table)) x)
                        table)))))
        (copy object)))))

(defun warm-restart--strip (object)
  "OBJECT with the text properties of every string in it removed, each
secret's characters starred first (`warm-restart--redacted')."
  (cond ((stringp object) (warm-restart--redacted object))
        ((proper-list-p object) (mapcar #'warm-restart--strip object))
        ((consp object) (cons (warm-restart--strip (car object)) (warm-restart--strip (cdr object))))
        (t object)))

(defun warm-restart--print (form)
  "FORM printed under fixed bindings, so ambient printer settings never
reach an image."
  (let ((print-length nil) (print-level nil) (print-circle t) (print-gensym t)
        (print-escape-newlines nil) (print-escape-nonascii nil)
        (print-escape-multibyte nil) (print-escape-control-characters nil)
        (print-continuous-numbering nil) (print-number-table nil)
        (float-output-format nil) (print-integers-as-characters nil)
        (print-charset-text-property nil) (print-quoted t))
    (prin1-to-string form)))

;;;; Classifying buffers

(defun warm-restart--blocker ()
  "Why the current buffer cannot be handed over now, or nil."
  (run-hook-with-args-until-success 'warm-restart-blocker-functions))

(defun warm-restart-unsaved-p ()
  "Non-nil when the current buffer holds unsaved text someone typed: a
modified file buffer, or a modified writable buffer.  Not a read-only
view its package fills (a mail search, the agent board), nor a widget
buffer, whose text is generated and whose widgets, which make it work,
can't be carried.  A process's buffer only in a comint mode, whose
process mark ends the output: when text follows the mark (narrowing
aside), input typed and not yet sent, or when the mark is not placed."
  (and (buffer-modified-p)
       (or buffer-file-name
           (and (not buffer-read-only)
                (not (memq 'widget-before-change before-change-functions))
                (let ((process (get-buffer-process (current-buffer))))
                  (or (not process)
                      (and (derived-mode-p 'comint-mode)
                           (let ((mark (process-mark process)))
                             (or (not (eq (marker-buffer mark) (current-buffer)))
                                 (< mark (1+ (buffer-size))))))))))
       t))

(defun warm-restart--handler ()
  "The desktop handler restoring the current buffer, or nil."
  (and desktop-save-buffer (cdr (assq major-mode desktop-buffer-mode-handlers))))

(defun warm-restart--regenerable ()
  "Why the current buffer is a view rebuilt by its package, or nil."
  (cond ((warm-restart-unsaved-p) nil)
        ((cl-some (lambda (re) (string-match-p re (buffer-name))) warm-restart-regenerable-buffers)
         "generated buffer")
        ((stringp warm-restart-regenerable) warm-restart-regenerable)
        (warm-restart-regenerable "view; its package reopens it")))

(defun warm-restart--text-mode-p ()
  "Non-nil when the current buffer's mode is carried as text."
  (or (eq major-mode 'fundamental-mode) (apply #'derived-mode-p warm-restart-text-modes)))

(defun warm-restart--classify ()
  "The current buffer's (STRATEGY . REASON), or nil for an internal buffer.
STRATEGY is `blocked', `file', `text', `indirect', `handler' or `dropped'."
  (let ((blocker (warm-restart--blocker))
        (process (get-buffer-process (current-buffer)))
        (base (buffer-base-buffer))
        regenerable)
    (cond
     (blocker (cons 'blocked blocker))
     ((stringp warm-restart-left-out) (cons 'dropped warm-restart-left-out))
     ((string-prefix-p " " (buffer-name)) nil)
     (base (if (memq (car (with-current-buffer base (warm-restart--classify))) '(file text))
               (cons 'indirect (format "indirect buffer of %s" (buffer-name base)))
             (cons 'dropped (format "indirect buffer of %s, which is not carried" (buffer-name base)))))
     (buffer-file-name (cons 'file (if (buffer-modified-p) "unsaved text carried" "visits a file")))
     ((warm-restart--handler) (cons 'handler (format "%s restores it" major-mode)))
     ((setq regenerable (warm-restart--regenerable)) (cons 'dropped regenerable))
     (process (cons 'dropped (format "process %s ends: processes are not carried yet" (process-name process))))
     ((warm-restart--text-mode-p) (cons 'text (format "%s text" major-mode)))
     ((warm-restart-unsaved-p) (cons 'text (format "edited %s view carried as text" major-mode)))
     (t (cons 'dropped (format "%s view; reopen it" major-mode))))))

(defun warm-restart--processes ()
  "Each live process, which ends with this one: (:process NAME :type TYPE
:buffer NAME :query FLAG)."
  (cl-loop for process in (process-list)
           when (process-live-p process)
           collect (list :process (process-name process) :type (process-type process)
                         :buffer (and (process-buffer process) (buffer-name (process-buffer process)))
                         :query (and (process-query-on-exit-flag process) t))))

;;;###autoload
(defun warm-restart-plan ()
  "What a session image made now would carry, as a plist.
:buffers lists (:name NAME :strategy STRATEGY :reason REASON :modified
FLAG :unsaved FLAG) per buffer, internal ones aside, :unsaved when it
holds unsaved typed text (`warm-restart-unsaved-p'); :processes lists the
processes, which end with this process."
  (list :buffers (cl-loop for buffer in (buffer-list)
                          for entry = (with-current-buffer buffer (warm-restart--classify))
                          when entry
                          collect (list :name (buffer-name buffer) :strategy (car entry) :reason (cdr entry)
                                        :modified (and (buffer-modified-p buffer) t)
                                        :unsaved (with-current-buffer buffer (warm-restart-unsaved-p))
                                        :buffer buffer))
        :processes (warm-restart--processes)))

(defun warm-restart--entries (plan strategy)
  "PLAN's buffer entries of STRATEGY."
  (cl-remove-if-not (lambda (e) (eq (plist-get e :strategy) strategy)) (plist-get plan :buffers)))

;;;; Capturing a buffer

(defun warm-restart--buffer-id ()
  "The current buffer's id, made on first use."
  (if (eq (cdr-safe warm-restart-buffer-id) (current-buffer))
      (car warm-restart-buffer-id)
    (let ((id (format "%d-%s-%d" (emacs-pid) (format-time-string "%s%N") (random 1000000))))
      (setq warm-restart-buffer-id (cons id (current-buffer)))
      id)))

(defun warm-restart--carried-string (string)
  "STRING with only the `warm-restart-buffer-properties' that are data,
each secret's characters starred (`warm-restart--redacted')."
  (let ((out (warm-restart--redacted string)))
    (dolist (prop warm-restart-buffer-properties)
      (let ((pos 0))
        (while (< pos (length string))
          (let ((value (get-text-property pos prop string))
                (next (next-single-property-change pos prop string (length string))))
            (when (and value (warm-restart--portable-p value)) (put-text-property pos next prop value out))
            (setq pos next)))))
    out))

(defun warm-restart--portable-undo (list)
  "Undo LIST as data: (ENTRIES . CUT), CUT non-nil when an entry that is
not data ended it there.  Marker adjustments are dropped: no marker
survives anyway."
  (if (eq list t)
      (cons t nil)
    (let (out cut)
      (catch 'stop
        (dolist (entry list)
          (pcase entry
            (`(,(pred markerp) . ,_) nil)
            (`(,(and (pred stringp) text) . ,(and (pred integerp) pos))
             (push (cons (warm-restart--carried-string text) pos) out))
            (_ (if (warm-restart--portable-p entry) (push entry out) (setq cut t) (throw 'stop nil))))))
      (cons (nreverse out) cut))))

(defun warm-restart--property-runs ()
  "The runs of `warm-restart-buffer-properties' in the buffer: (PROP BEG END VALUE).
A unibyte buffer has none: its bytes are not typed text."
  (save-restriction
    (widen)
    (cl-loop for prop in (and enable-multibyte-characters warm-restart-buffer-properties)
             nconc (let ((pos (point-min)) runs)
                     (while (< pos (point-max))
                       (let ((value (get-text-property pos prop))
                             (next (next-single-property-change pos prop nil (point-max))))
                         (when (and value (warm-restart--portable-p value)) (push (list prop pos next value) runs))
                         (setq pos next)))
                     (nreverse runs)))))

(defun warm-restart--minor-modes ()
  "The buffer's enabled local minor modes, but those
`desktop-minor-mode-table' says not to restore."
  (cl-remove-if (lambda (mode)
                  (let ((special (assq mode desktop-minor-mode-table)))
                    (and special (not (cadr special)))))
                local-minor-modes))

(defun warm-restart--locals ()
  "The buffer's values of `desktop-locals-to-save' that are data."
  (cl-loop for var in desktop-locals-to-save
           when (and (local-variable-p var) (warm-restart--portable-p (symbol-value var)))
           collect (cons var (symbol-value var))))

(defvar warm-restart--libraries nil
  "During a write, a table from each major mode to the library defining it:
`symbol-file' scans `load-history', so each mode is looked up once.")

(defun warm-restart--library (mode)
  "The base name of the library that defined MODE, or nil."
  (let ((cached (if warm-restart--libraries (gethash mode warm-restart--libraries 'none) 'none)))
    (if (not (eq cached 'none))
        cached
      (let ((library (let ((file (symbol-file mode 'defun))) (and (stringp file) (file-name-base file)))))
        (when warm-restart--libraries (puthash mode library warm-restart--libraries))
        library))))

(defun warm-restart--common ()
  "What every carried buffer records."
  (list :id (warm-restart--buffer-id) :name (buffer-name) :mode major-mode
        :library (warm-restart--library major-mode)
        :directory default-directory :point (point) :mark (mark t) :mark-active (and mark-active t)
        :mark-ring (mapcar #'marker-position (cl-remove-if-not #'marker-position mark-ring))
        :read-only buffer-read-only :uniquify (and (bound-and-true-p uniquify-managed) t)
        :narrowing (and (buffer-narrowed-p) (cons (point-min) (point-max)))
        :minor-modes (warm-restart--minor-modes) :locals (warm-restart--locals)))

(defun warm-restart--text-record (strategy blobs)
  "The record of the current buffer, carried by STRATEGY `file' or `text'.
Its text goes in the image when it is not the file's: a snapshot of it
\(`warm-restart--snapshot'), taken with the rest of the record, is
pushed onto BLOBS, a cell (COUNT . BUFFERS), and its index named; the
record is `:starred' when the snapshot's secrets are."
  (let* ((undo (warm-restart--portable-undo buffer-undo-list))
         (carry (or (eq strategy 'text) (buffer-modified-p) (not (file-exists-p buffer-file-name))))
         (snapshot (and carry (warm-restart--snapshot (current-buffer)))))
    (append (warm-restart--common)
            (list :strategy strategy :hash (with-current-buffer (or (car snapshot) (current-buffer)) (buffer-hash))
                  :size (buffer-size)
                  :modified (and (buffer-modified-p) t) :undo (car undo) :undo-cut (cdr undo)
                  :unibyte (not enable-multibyte-characters) :properties (warm-restart--property-runs))
            (and (cdr snapshot) (list :starred t))
            (and buffer-file-name
                 (list :file buffer-file-name :modtime (visited-file-modtime) :coding buffer-file-coding-system))
            (and snapshot (progn (push (car snapshot) (cdr blobs))
                                 (list :blob (1- (cl-incf (car blobs)))))))))

(defvar warm-restart--staging nil
  "During a write, a cell whose car lists the buffers staged for its
blobs (`warm-restart--snapshot'), killed once it is written.")

(defun warm-restart--snapshot (buffer)
  "A copy of BUFFER's whole text as the image takes it, (STAGE . STARRED):
a staging buffer of its own, listed in `warm-restart--staging', which no
code run later (a handler, a hook after a collection) edits, its
secrets' characters starred one for one (`warm-restart--redacted') when
STARRED."
  (let ((stage (generate-new-buffer " *warm-restart staged*" t))
        (text (with-current-buffer buffer (save-restriction (widen) (buffer-string)))))
    (when warm-restart--staging (push stage (car warm-restart--staging)))
    (let ((starred (text-property-not-all 0 (length text) 'romacs-secret nil text)))
      (with-current-buffer stage
        (set-buffer-multibyte (buffer-local-value 'enable-multibyte-characters buffer))
        (let ((inhibit-modification-hooks t))
          (insert (if starred (warm-restart--redacted text) (substring-no-properties text)))))
      (cons stage (and starred t)))))

(defun warm-restart--set-apart (records)
  "RECORDS with each file buffer's whose secrets were starred set apart:
\(BUFFERS STARRED . RENAMES), STARRED those records as strategy `starred',
under \"NAME <starred>\" free of every record's name, with no file nor
file locals, their header saying what they are; RENAMES the windows'
\(OLD . NEW).  An image keeps STARRED out of `:buffers', so no reader
that predates them restores them, nor lands their stars in a buffer
visiting the file; this one restores each as a new buffer that visits
no file (`warm-restart--restore-starred')."
  (let ((taken (make-hash-table :test #'equal)) buffers starred renames)
    (dolist (record records) (when (plist-get record :name) (puthash (plist-get record :name) t taken)))
    (dolist (record records)
      (if (not (and (plist-get record :starred) (plist-get record :file)))
          (push record buffers)
        (let ((name (warm-restart--free-name (format "%s <starred>" (plist-get record :name)) (lambda (n) (gethash n taken))))
              (copy (copy-sequence record)))
          (puthash name t taken)
          (push (cons (plist-get record :name) name) renames)
          (dolist (pair `((:strategy . starred) (:name . ,name) (:copy-of . ,(plist-get record :file))
                          (:file . nil) (:modtime . nil) (:coding . nil) (:uniquify . nil)
                          (:locals . ,(cl-remove-if (lambda (l) (memq (car l) '(buffer-file-name buffer-file-truename
                                                                                  buffer-file-number header-line-format)))
                                                    (plist-get record :locals)))))
            (setq copy (plist-put copy (car pair) (cdr pair))))
          (push copy starred))))
    (cons (nreverse buffers) (cons (nreverse starred) renames))))

(defun warm-restart--restore-starred (record blobs)
  "Recreate RECORD, set apart by `warm-restart--set-apart', as a new buffer
that visits no file, under its name or the first free one after it: it
never lands in a buffer of this session.  Its header says whose copy it
is, and it offers to be saved at exit."
  (with-current-buffer (generate-new-buffer (warm-restart--free-name (plist-get record :name) #'get-buffer))
    (warm-restart--put-text (warm-restart--carried-text record blobs) (plist-get record :unibyte))
    (warm-restart--put-properties (plist-get record :properties))
    (set-buffer-modified-p t)
    (pcase-dolist (`(,var . ,value) (plist-get record :locals)) (set (make-local-variable var) value))
    (setq-local header-line-format (format "A copy of %s, its secrets starred: it visits no file" (plist-get record :copy-of))
                buffer-offer-save t)
    (warm-restart--apply-common record)
    (warm-restart--put-undo record)
    (current-buffer)))

(defun warm-restart--record (entry blobs)
  "The image record of plan ENTRY's buffer, or a `lost' entry when its data
is not portable."
  (with-current-buffer (plist-get entry :buffer)
    (pcase (plist-get entry :strategy)
      ((and (or 'file 'text) strategy) (warm-restart--as-transcript (warm-restart--text-record strategy blobs)))
      ('indirect (warm-restart--as-transcript
                  (append (warm-restart--common)
                          (list :strategy 'indirect
                                :base (with-current-buffer (buffer-base-buffer) (warm-restart--buffer-id))))))
      ('handler
       (let ((misc (and (functionp desktop-save-buffer)
                        (let ((desktop-dirname (warm-restart--directory))) (funcall desktop-save-buffer desktop-dirname)))))
         (if (warm-restart--portable-p misc)
             (append (warm-restart--common) (list :strategy 'handler :misc misc :file buffer-file-name))
           (list :lost (buffer-name) :reason (format "%s data is not portable" major-mode))))))))

;;;; Capturing the session

(defun warm-restart--frames ()
  "The window state of each frame that is not a child frame, selected first."
  (let ((frames (cons (selected-frame) (delq (selected-frame) (frame-list)))))
    (cl-loop for frame in frames
             when (and (frame-live-p frame) (not (frame-parameter frame 'parent-frame))
                       (not (eq (frame-parameter frame 'minibuffer) 'only)))
             collect (list :selected (eq frame (selected-frame))
                           :state (warm-restart--strip (window-state-get (frame-root-window frame) t))))))

(defun warm-restart--secret-p (value)
  "Whether VALUE is a string holding a secret, which no image holds: text
with the `romacs-secret' property anywhere, as the password manager
marks its secrets."
  (and (stringp value) (text-property-not-all 0 (length value) 'romacs-secret nil value) t))

(defun warm-restart--register (value)
  "Register VALUE as data: a position becomes (file-query FILE POS) in a
file's buffer or (warm-restart-position ID POS) in another carried one.
nil when it cannot be carried, or is a secret."
  (cond ((warm-restart--secret-p value) nil)
        ((markerp value)
         (let ((buffer (marker-buffer value)))
           (cond ((null buffer) nil)
                 ((buffer-file-name buffer) (list 'file-query (buffer-file-name buffer) (marker-position value)))
                 (t (list 'warm-restart-position (with-current-buffer buffer (warm-restart--buffer-id))
                          (marker-position value))))))
        (t (let ((value (warm-restart--strip value))) (and (warm-restart--portable-p value) value)))))

(defun warm-restart--variables (variables)
  "(VAR . VALUE) for each bound VARIABLES whose value is data, and the
names of those that are not."
  (let (carried lost)
    (dolist (var (delete-dups (copy-sequence variables)))
      (when (and (symbolp var) (boundp var))
        (let ((value (warm-restart--strip (symbol-value var))))
          (if (warm-restart--portable-p value) (push (cons var value) carried) (push var lost)))))
    (cons (nreverse carried) (nreverse lost))))

(defun warm-restart--history-variables ()
  "Every minibuffer history variable carried."
  (append (bound-and-true-p savehist-minibuffer-history-variables)
          (bound-and-true-p savehist-additional-variables)
          warm-restart-history-variables))

(defun warm-restart--globals ()
  "The session-wide part of an image, and what of it is lost.
The kill ring's secrets (`warm-restart--secret-p') are left out."
  (let ((registers (mapcar (lambda (cell) (cons (car cell) (warm-restart--register (cdr cell)))) register-alist))
        (histories (warm-restart--variables (warm-restart--history-variables)))
        (globals (warm-restart--variables warm-restart-globals))
        (kills (seq-remove #'warm-restart--secret-p kill-ring)))
    (list :kill-ring (mapcar #'substring-no-properties kills)
          :yank-index (- (length kills) (length (seq-remove #'warm-restart--secret-p kill-ring-yank-pointer)))
          :registers (cl-remove-if-not #'cdr registers)
          :histories (car histories) :globals (car globals)
          :lost (append (cl-loop for (char . value) in registers unless value
                                 collect (list :register char :reason "not data"))
                        (mapcar (lambda (var) (list :variable var :reason "value is not data"))
                                (append (cdr histories) (cdr globals)))))))

(defun warm-restart--directory ()
  "The image directory, made private when new."
  (let ((dir (file-name-as-directory
              (expand-file-name (or warm-restart-directory
                                    (expand-file-name "romacs/warm-restart/"
                                                      (or (getenv "XDG_STATE_HOME") "~/.local/state")))))))
    (unless (file-directory-p dir)
      (with-file-modes #o700 (make-directory dir t)))
    dir))

(defun warm-restart--image-name ()
  "A new image's file name."
  (expand-file-name (format "%s-%d.image" (format-time-string "%Y%m%dT%H%M%S") (emacs-pid))
                    (warm-restart--directory)))

(define-error 'warm-restart-blocked "Session cannot be handed over now")

;;;###autoload
(defun warm-restart-write (&optional plan file)
  "Write a session image of PLAN (default: `warm-restart-plan') to FILE.
Return FILE.  Signal `warm-restart-blocked' when a buffer is blocked."
  (let* ((plan (or plan (warm-restart-plan)))
         (blocked (warm-restart--entries plan 'blocked))
         (file (or file (warm-restart--image-name)))
         (blobs (cons 0 nil))
         (warm-restart--staging (list nil))
         (warm-restart--libraries (make-hash-table :test #'eq)))
    (when blocked
      (signal 'warm-restart-blocked
              (mapcar (lambda (e) (format "%s: %s" (plist-get e :name) (plist-get e :reason))) blocked)))
    (unwind-protect
        (let* ((carried (cl-remove-if (lambda (e) (memq (plist-get e :strategy) '(blocked dropped))) (plist-get plan :buffers)))
               (made (mapcar (lambda (e)
                               (condition-case err (warm-restart--record e blobs)
                                 (error (signal 'warm-restart-blocked
                                                (list (format "%s: cannot be saved: %s" (plist-get e :name)
                                                              (error-message-string err)))))))
                             carried))
               (apart (warm-restart--set-apart made))
               (named (warm-restart--name-transcripts (car apart)))
               (renames (let ((transcripts (cdr named)))
                          (append (mapcar (lambda (r) (cons (car r) (or (cdr (assoc (cdr r) transcripts)) (cdr r))))
                                          (cddr apart))
                                  transcripts)))
               (records (car named))
               (globals (warm-restart--globals))
               (form (list :schema warm-restart--schema :pid (emacs-pid) :created (format-time-string "%FT%T%z")
                           :transcripts t
                           :emacs-version emacs-version
                           :buffers (cl-remove-if (lambda (r) (plist-member r :lost)) records)
                           :starred (cadr apart)
                           :frames (warm-restart--renamed-in (warm-restart--frames) renames)
                           :lost (append (cl-loop for e in (plist-get plan :buffers)
                                                  when (eq (plist-get e :strategy) 'dropped)
                                                  collect (append (list :buffer (plist-get e :name) :reason (plist-get e :reason))
                                                                  (and (plist-get e :unsaved) (list :unsaved t))))
                                         (cl-loop for r in records when (plist-member r :lost)
                                                  collect (list :buffer (plist-get r :lost) :reason (plist-get r :reason)))
                                         (mapcar (lambda (p) (append p (list :reason "ends with this process")))
                                                 (plist-get plan :processes))
                                         (plist-get globals :lost)))))
          (setq form (append form (cl-loop for (key value) on globals by #'cddr unless (eq key :lost) append (list key value))))
          ;; No secret's text in any string the image holds.
          (romacs--session-image-write file (warm-restart--print (warm-restart--without-secrets form)) (nreverse (cdr blobs)))
          file)
      ;; Staged blobs, written or not.
      (mapc #'kill-buffer (car warm-restart--staging)))))

;;;; Restoring a buffer

(defun warm-restart--load-mode (mode library)
  "Make MODE defined and its library loaded (an autoload counts as not
loaded: the library may register MODE's desktop handler), from LIBRARY,
where it was defined, if need be."
  (when (and mode (fboundp mode)) (desktop-load-file mode))
  (when (and mode (not (fboundp mode)) library)
    (with-demoted-errors "warm-restart: %S" (load library t t)))
  (when (and mode (not (fboundp mode))) (desktop-load-file mode)))

(defun warm-restart--set-mode (record)
  "Put the current buffer in RECORD's major mode when it is defined.  A
shell's mode, known once its library is loaded, gives `fundamental-mode'
and marks the buffer a transcript (`warm-restart--transcript-of'): an
image written before shells were carried as transcripts names it."
  (let ((mode (plist-get record :mode)))
    (warm-restart--load-mode mode (plist-get record :library))
    (cond ((warm-restart--shell-mode-p mode)
           (fundamental-mode)
           (setq warm-restart--transcript-of mode)
           (unless (warm-restart--transcript-name-p (buffer-name))
             (rename-buffer (warm-restart--transcript-name (buffer-name) #'get-buffer))))
          ((not (eq major-mode mode))
           (funcall (if (and mode (fboundp mode)) mode #'fundamental-mode))))))

(defun warm-restart--put-text (text unibyte)
  "Replace the current buffer's text with TEXT, recording no change."
  (let ((inhibit-read-only t) (inhibit-modification-hooks t) (buffer-undo-list t))
    (widen)
    (erase-buffer)
    (set-buffer-multibyte (not unibyte))
    (insert text)))

(defun warm-restart--put-properties (runs)
  "Put the property RUNS back, as changes neither undo nor hooks see."
  (let ((inhibit-read-only t) (inhibit-modification-hooks t) (buffer-undo-list t)
        (modified (buffer-modified-p)))
    (pcase-dolist (`(,prop ,beg ,end ,value) runs)
      (when (<= end (1+ (buffer-size))) (put-text-property beg end prop value)))
    (set-buffer-modified-p modified)))

(defun warm-restart--apply-modes (record)
  "Enable RECORD's minor modes the mode did not, and set its locals.  A
restored transcript takes none of its shell's, only a header naming it."
  (if warm-restart--transcript-of
      (setq-local header-line-format (warm-restart--transcript-header warm-restart--transcript-of))
    (warm-restart--apply-minor-modes-and-locals record)))

(defun warm-restart--apply-minor-modes-and-locals (record)
  "Enable RECORD's minor modes the mode did not, and set its locals."
  (dolist (mode (plist-get record :minor-modes))
    (unless (memq mode local-minor-modes)
      (desktop-load-file mode)
      (let ((handler (cdr (assq mode desktop-minor-mode-handlers))))
        (with-demoted-errors "warm-restart: %S"
          (cond (handler (funcall handler (plist-get record :locals)))
                ((functionp mode) (funcall mode 1)))))))
  (pcase-dolist (`(,var . ,value) (plist-get record :locals))
    (set (make-local-variable var) value)))

(defun warm-restart--apply-common (record)
  "Apply RECORD's positions, directory and read-only flag."
  (let ((clamp (lambda (n) (and n (max (point-min) (min n (1+ (buffer-size))))))))
    (widen)
    (setq default-directory (plist-get record :directory))
    (when-let* ((mark (funcall clamp (plist-get record :mark)))) (set-marker (mark-marker) mark))
    (setq mark-ring (mapcar (lambda (n) (copy-marker (funcall clamp n))) (plist-get record :mark-ring)))
    (when (plist-get record :mark-active) (setq mark-active t))
    (goto-char (funcall clamp (plist-get record :point)))
    (when-let* ((narrowing (plist-get record :narrowing)))
      (narrow-to-region (funcall clamp (car narrowing)) (funcall clamp (cdr narrowing))))
    (setq buffer-read-only (plist-get record :read-only))))

(defun warm-restart--put-undo (record)
  "Install RECORD's undo when the text is the captured text; non-nil when
it was dropped instead."
  (if (equal (buffer-hash) (plist-get record :hash))
      (progn (setq buffer-undo-list (plist-get record :undo)) nil)
    (setq buffer-undo-list nil)
    t))

(defun warm-restart--carried-text (record blobs)
  "RECORD's carried text from BLOBS, or nil when the file holds it."
  (and (plist-get record :blob) (aref blobs (plist-get record :blob))))

(defun warm-restart--restore-file (record blobs)
  "Visit RECORD's file, put its carried text back, and return the buffer.
A buffer with carried text keeps the old visited-file modtime, so saving
over a file changed on disk still asks."
  (let* ((enable-local-variables :safe)
         (large-file-warning-threshold nil)
         (text (warm-restart--carried-text record blobs))
         (buffer (if (and (not warm-restart--defer) (not (plist-get record :transcript-of)))
                     (find-file-noselect (plist-get record :file) t (plist-get record :unibyte))
                   ;; Unseen: visited in no mode of its own, its hooks waiting.
                   (let ((auto-mode-alist nil) (magic-mode-alist nil) (magic-fallback-mode-alist nil)
                         (interpreter-mode-alist nil) (enable-local-variables nil) (find-file-hook nil))
                     (find-file-noselect (plist-get record :file) t (plist-get record :unibyte))))))
    (with-current-buffer buffer
      (unless warm-restart--defer (warm-restart--set-mode record))
      (when-let* ((coding (plist-get record :coding)))
        (when (coding-system-p coding) (set-buffer-file-coding-system coding t t)))
      (when text
        (warm-restart--put-text text (plist-get record :unibyte))
        (set-buffer-modified-p (plist-get record :modified))
        (ignore-errors (set-visited-file-modtime (plist-get record :modtime)))
        ;; Shown before the restore ends: its first edit locks its file.
        (when (plist-get record :modified) (setq-local warm-restart-lock-pending t)))
      (warm-restart--put-properties (plist-get record :properties))
      ;; A transcript's file wakes in its record's mode alone: no file mode,
      ;; no file hooks, which could make it a shell again.
      (if warm-restart--defer
          (warm-restart--defer-modes record (not (plist-get record :transcript-of)))
        (warm-restart--apply-modes record))
      (warm-restart--apply-common record)
      (when (and (warm-restart--put-undo record) (plist-get record :undo))
        (push (list :buffer (plist-get record :name) :reason "file changed on disk; reopened, undo dropped")
              warm-restart--problems))
      ;; uniquify names its buffers again once all are back; a name the
      ;; user gave is the buffer's own.
      (unless (or (equal (buffer-name) (plist-get record :name)) (plist-get record :uniquify))
        (rename-buffer (plist-get record :name) t)))
    buffer))

(defun warm-restart--restore-text (record blobs)
  "Recreate RECORD's buffer from its carried text and return it."
  (with-current-buffer (get-buffer-create (plist-get record :name))
    (setq default-directory (plist-get record :directory))
    (unless warm-restart--defer (let ((inhibit-read-only t)) (warm-restart--set-mode record)))
    (warm-restart--put-text (warm-restart--carried-text record blobs) (plist-get record :unibyte))
    (warm-restart--put-properties (plist-get record :properties))
    (set-buffer-modified-p (plist-get record :modified))
    (if warm-restart--defer (warm-restart--defer-modes record nil) (warm-restart--apply-modes record))
    (warm-restart--apply-common record)
    (warm-restart--put-undo record)
    (current-buffer)))

(defun warm-restart--restore-indirect (record buffers)
  "Recreate RECORD's indirect buffer of its restored base in BUFFERS."
  (let ((base (gethash (plist-get record :base) buffers)))
    (unless (buffer-live-p base) (error "Its base buffer was not restored"))
    (with-current-buffer (make-indirect-buffer base (generate-new-buffer-name (plist-get record :name)) nil t)
      (if warm-restart--defer
          (warm-restart--defer-modes record nil)
        (let ((inhibit-read-only t) (inhibit-modification-hooks t)) (warm-restart--set-mode record))
        (warm-restart--apply-modes record))
      (warm-restart--apply-common record)
      (current-buffer))))

(defvar desktop-buffer-ok-count)
(defvar desktop-buffer-fail-count)

(defun warm-restart--restore-handler (record)
  "Recreate RECORD's buffer through its mode's desktop handler."
  (warm-restart--load-mode (plist-get record :mode) (plist-get record :library))
  (let* ((mode (plist-get record :mode))
         (handler (or (cdr (assq mode desktop-buffer-mode-handlers))
                      (error "No desktop handler for %s" mode)))
         (desktop-buffer-file-name (plist-get record :file))
         (desktop-buffer-name (plist-get record :name))
         (desktop-buffer-major-mode mode)
         (desktop-buffer-minor-modes (plist-get record :minor-modes))
         (desktop-buffer-point (plist-get record :point))
         (desktop-buffer-mark (list (plist-get record :mark) (plist-get record :mark-active)))
         (desktop-buffer-read-only (plist-get record :read-only))
         (desktop-buffer-misc (plist-get record :misc))
         (desktop-buffer-locals (plist-get record :locals))
         (desktop-dirname (warm-restart--directory))
         (desktop-buffer-ok-count 0) (desktop-buffer-fail-count 0)
         (default-directory (plist-get record :directory))
         (buffer (save-current-buffer
                   (funcall handler desktop-buffer-file-name desktop-buffer-name desktop-buffer-misc))))
    (unless (buffer-live-p buffer) (error "%s's handler made no buffer" mode))
    (with-current-buffer buffer
      (unless (equal (buffer-name) desktop-buffer-name) (rename-buffer desktop-buffer-name t))
      (warm-restart--apply-modes record)
      (when-let* ((point (plist-get record :point))) (goto-char (min point (point-max))))
      (setq buffer-read-only (plist-get record :read-only)))
    buffer))

(defun warm-restart--rescued-text (record blobs condition)
  "What a rescue buffer of RECORD holds after CONDITION: its carried text,
or its handler's saved data printed, or nil."
  (or (warm-restart--carried-text record blobs)
      (and (plist-get record :misc)
           (format ";; %s could not restore this buffer: %s\n;; What it saved:\n%s\n"
                   (plist-get record :mode) (error-message-string condition)
                   (let ((print-length nil) (print-level nil)) (pp-to-string (plist-get record :misc)))))))

(defun warm-restart--rescue (record blobs condition)
  "After CONDITION, put RECORD's carried text, or its handler's saved data,
in a new fundamental-mode buffer so it is never lost; return that buffer,
or nil when there is neither."
  (when-let* ((text (warm-restart--rescued-text record blobs condition)))
    (with-current-buffer (generate-new-buffer (plist-get record :name))
      (warm-restart--put-text text (plist-get record :unibyte))
      (when (and (plist-get record :file) (plist-get record :blob)) (setq buffer-file-name (plist-get record :file)))
      (set-buffer-modified-p t)
      (setq default-directory (plist-get record :directory))
      (push (list :buffer (plist-get record :name) :text t
                  :reason (format "restored as plain text: %s" (error-message-string condition)))
            warm-restart--problems)
      (current-buffer))))

(defun warm-restart--made-buffer (record)
  "The buffer RECORD's restore makes or reuses, if it exists.  A starred
record's never exists: it makes a new buffer (`warm-restart--restore-starred')."
  (and (not (eq (plist-get record :strategy) 'starred))
       (or (and (plist-get record :file) (find-buffer-visiting (plist-get record :file)))
           (get-buffer (plist-get record :name)))))

(defun warm-restart--discard (buffer)
  "Kill BUFFER, half made by a failed restore, without asking."
  (with-current-buffer buffer (set-buffer-modified-p nil))
  (let ((kill-buffer-query-functions nil) (kill-buffer-hook nil)) (kill-buffer buffer)))

(defun warm-restart--beside (record blobs existing)
  "EXISTING, a buffer edited here before RECORD's turn in the restore came,
which keeps its edits; RECORD's carried text goes in a new buffer beside
it, and the problem names both."
  (when-let* ((text (warm-restart--carried-text record blobs)))
    (with-current-buffer (generate-new-buffer (format "%s (restored)" (plist-get record :name)))
      (warm-restart--put-text text (plist-get record :unibyte))
      (setq default-directory (plist-get record :directory))
      (push (list :buffer (plist-get record :name) :text t
                  :reason (format "edited in %s before its turn in the restore, which kept the edits; the carried text is in %s"
                                  (buffer-name existing) (buffer-name)))
            warm-restart--problems)))
  existing)

(defconst warm-restart--shell-modes
  '(eshell-mode comint-mode term-mode inferior-emacs-lisp-mode)
  "Modes of a shell, and the modes they derive.  A shell's buffer revived
in its mode would read its whole transcript as input at the next RET.")

(defvar-local warm-restart--transcript-of nil
  "The shell mode this buffer's restored transcript came from, or nil.")

(defun warm-restart--shell-mode-p (mode)
  "Whether MODE, defined, is a shell's: one of `warm-restart--shell-modes'
or derived from one."
  (and mode (symbolp mode) (fboundp mode)
       (apply #'provided-mode-derived-p mode warm-restart--shell-modes)
       t))

(defun warm-restart--transcript-header (mode)
  "The header line of a restored transcript of a MODE shell."
  (format "Transcript of %s; its shell ended with the restart" mode))

(defun warm-restart--free-name (name taken)
  "NAME, or NAME<2>, NAME<3>..., the first TAKEN, a function of a name,
says is free."
  (let ((candidate name) (n 1))
    (while (funcall taken candidate)
      (setq candidate (format "%s<%d>" name (cl-incf n))))
    candidate))

(defun warm-restart--transcript-name (name taken)
  "The name of NAME's transcript, \"NAME (transcript)\", one TAKEN, a
function of a name, says is free.  Not the shell's own name: a shell
opened later makes a buffer of its own, never one over the transcript."
  (let* ((base (format "%s (transcript)" name)) (candidate base) (n 1))
    (while (funcall taken candidate)
      (setq candidate (format "%s<%d>" base (cl-incf n))))
    candidate))

(defun warm-restart--transcript-name-p (name)
  "Whether NAME is a transcript's (`warm-restart--transcript-name')."
  (string-match-p " (transcript)\\(<[0-9]+>\\)?\\'" name))

(defun warm-restart--prepare (form)
  "FORM's records, ready to restore.  An image written before shells were
carried as transcripts (no `:transcripts') may name a shell's mode for a
file, text or indirect buffer: each such record carries `:legacy-names',
a table of the image's names shared by them all, for
`warm-restart--legacy-destination' to choose its buffer's name from at
its own turn, before its buffer is made."
  (let ((records (append (plist-get form :buffers) (plist-get form :starred))))
    (if (plist-get form :transcripts)
        records
      (let ((names (make-hash-table :test #'equal)))
        (dolist (record records) (puthash (plist-get record :name) t names))
        (mapcar (lambda (record)
                  (if (memq (plist-get record :strategy) '(file text indirect))
                      (plist-put (copy-sequence record) :legacy-names names)
                    record))
                records)))))

(defun warm-restart--legacy-destination (record)
  "RECORD, or, for a record of an older image (`:legacy-names') whose mode
is a shell's, the record under its transcript's name, which no record of
the image nor a live buffer has: its buffer is made under that name, so
it never lands in a live shell nor waits unseen under the shell's name.
A mode not loaded yet is loaded here with each parent not loaded yet, at
the record's own turn and within its passive restore, to know it; one
that cannot be resolved so is taken for a shell's.  Either way the record
is its transcript's (`warm-restart--inert-record'): nothing retries the
mode."
  (let ((names (plist-get record :legacy-names)) (mode (plist-get record :mode)))
    (if (not (and names mode (symbolp mode)))
        record
      (let ((inert (or (memq mode warm-restart--shell-modes)
                       (not (condition-case nil (warm-restart--mode-resolved-p mode (plist-get record :library))
                              (error nil)))
                       (warm-restart--shell-mode-p mode))))
        (if (not inert)
            record
          (let ((name (warm-restart--transcript-name (plist-get record :name)
                                                     (lambda (n) (or (gethash n names) (get-buffer n))))))
            (puthash name t names)
            (plist-put (warm-restart--inert-record record mode) :name name)))))))

(defun warm-restart--renamed-in (tree renames)
  "TREE, a window state or a list of them, with each buffer name in
RENAMES, an alist (OLD . NEW), renamed: a leaf's buffer and the entries
of its previous and next buffers."
  (cond ((and (consp tree) (eq (car tree) 'buffer) (consp (cdr tree)) (assoc (cadr tree) renames))
         (cons 'buffer (cons (cdr (assoc (cadr tree) renames)) (warm-restart--renamed-in (cddr tree) renames))))
        ((and (consp tree) (memq (car tree) '(prev-buffers next-buffers)))
         (cons (car tree) (mapcar (lambda (entry)
                                    (cond ((and (stringp entry) (assoc entry renames)) (cdr (assoc entry renames)))
                                          ((and (consp entry) (assoc (car entry) renames))
                                           (cons (cdr (assoc (car entry) renames)) (cdr entry)))
                                          (t entry)))
                                  (cdr tree))))
        ((consp tree) (cons (warm-restart--renamed-in (car tree) renames) (warm-restart--renamed-in (cdr tree) renames)))
        (t tree)))

(defun warm-restart--name-transcripts (records)
  "RECORDS with each shell transcript's (`warm-restart--as-transcript')
named apart from its shell, and the renames, as (RECORDS . RENAMES)."
  (let ((taken (make-hash-table :test #'equal)) renames)
    (dolist (record records) (puthash (plist-get record :name) t taken))
    (cons (mapcar (lambda (record)
                    (if (not (plist-get record :transcript-of))
                        record
                      (let ((name (warm-restart--transcript-name (plist-get record :name) (lambda (n) (gethash n taken)))))
                        (puthash name t taken)
                        (push (cons (plist-get record :name) name) renames)
                        (plist-put (copy-sequence record) :name name))))
                  records)
          renames)))

(defun warm-restart--as-transcript (record)
  "RECORD, a file, text or indirect buffer's, as its transcript's when its mode
is a shell's: `fundamental-mode', no minor modes, its locals only a
header naming the shell.  So any reader, an older one restoring a
rollback included, brings it back inert, never as a shell over its
transcript, whose input boundary it would no longer know."
  (let ((mode (plist-get record :mode)))
    (if (warm-restart--shell-mode-p mode) (warm-restart--inert-record record mode) record)))

(defun warm-restart--inert-record (record mode)
  "RECORD as the record of a transcript of a MODE buffer: `fundamental-mode',
no library, minor modes or uniquify, its locals only the header naming
MODE, `:transcript-of' MODE."
  (let ((record (copy-sequence record)))
    (dolist (pair `((:mode . fundamental-mode) (:library . nil) (:minor-modes . nil) (:transcript-of . ,mode) (:uniquify . nil)
                    (:locals . ((header-line-format . ,(warm-restart--transcript-header mode))))))
      (setq record (plist-put record (car pair) (cdr pair))))
    record))

(defun warm-restart--mode-resolved-p (mode library)
  "Load MODE, from LIBRARY if need be, and each of its parents not loaded
yet, through the libraries their autoloads name, an alias's target
included; non-nil when MODE and every parent are then defined.  May
signal, as a load may."
  (warm-restart--load-mode mode library)
  (let ((each mode) (resolved t) (seen nil))
    (while (and resolved each (not (memq each seen)))
      (push each seen)
      (when (and (fboundp each) (autoloadp (symbol-function each)))
        (autoload-do-load (symbol-function each) each))
      (let ((definition (and (fboundp each) (symbol-function each))))
        (cond ((and definition (symbolp definition)) (setq each definition)) ; an alias: its target
              ((and definition (not (autoloadp definition))) (setq each (get each 'derived-mode-parent)))
              (t (setq resolved nil)))))
    resolved))

(defun warm-restart--restore-buffer (record blobs buffers)
  "Restore RECORD; record its buffer under its id in BUFFERS.
When it fails, a buffer the attempt made is replaced by the rescue.  A
buffer it would reuse that was edited here first is left as it is
\(`warm-restart--beside')."
  (let* ((record (warm-restart--legacy-destination record))
         (existing (warm-restart--made-buffer record))
         ;; One made here meanwhile keeps the modes it has, and a modified
         ;; file, which a save may write, has its own at once: no deferral.
         (warm-restart--defer (and warm-restart--defer (not (buffer-live-p existing))
                                   (not (and (eq (plist-get record :strategy) 'file) (plist-get record :modified)))
                                   (not (eq (plist-get record :strategy) 'starred))))
         (buffer
         (if (and (buffer-live-p existing) (buffer-modified-p existing)
                  (memq (plist-get record :strategy) '(file text)))
             (warm-restart--beside record blobs existing)
         (condition-case condition
             (let ((buffer (pcase (plist-get record :strategy)
                             ('file (warm-restart--restore-file record blobs))
                             ('text (warm-restart--restore-text record blobs))
                             ('indirect (warm-restart--restore-indirect record buffers))
                             ('handler (warm-restart--restore-handler record))
                             ('starred (warm-restart--restore-starred record blobs)))))
               (with-current-buffer buffer (setq warm-restart-buffer-id (cons (plist-get record :id) buffer)))
               buffer)
           (error (let ((made (warm-restart--made-buffer record)))
                    (when (and made (not (eq made existing))) (warm-restart--discard made)))
                  (or (warm-restart--rescue record blobs condition)
                      (progn (push (list :buffer (plist-get record :name) :reason (error-message-string condition))
                                   warm-restart--problems)
                             nil)))))))
    (when buffer
      (puthash (plist-get record :id) buffer buffers)
      ;; Shown already (a window took its name meanwhile): its modes now.
      (when (get-buffer-window buffer t) (warm-restart--wake buffer)))))

(defun warm-restart--defer-modes (record file)
  "Let the current buffer's modes wait for it to be shown: RECORD's, and
FILE's own first when it visits one (`warm-restart--deferred')."
  (setq warm-restart--deferred (cons record file))
  (push (current-buffer) warm-restart--deferred-buffers)
  (add-hook 'window-buffer-change-functions #'warm-restart--wake-shown)
  (add-hook 'before-save-hook #'warm-restart--wake-current)
  ;; Code that reads a file's buffer (an agenda) finds it through this.
  (advice-add 'find-file-noselect :filter-return #'warm-restart--wake-found))

(defun warm-restart--wake (buffer)
  "Give BUFFER, restored unseen, the modes its record names: its file's
own mode, local variables and `find-file-hook' first when it visits one,
then the record's major mode, minor modes and locals."
  (when (buffer-live-p buffer)
    (with-current-buffer buffer
      (when-let* ((deferred warm-restart--deferred))
        (setq warm-restart--deferred nil)
        (let ((record (car deferred)) (modified (buffer-modified-p)) (inhibit-read-only t))
          ;; Its modes read the whole text, as at a visit; its narrowing stays.
          (save-excursion
            (save-restriction
              (widen)
              (with-demoted-errors "warm-restart: %S"
                (when (cdr deferred)
                  (let ((enable-local-variables :safe)) (normal-mode t))
                  (run-hooks 'find-file-hook))
                (warm-restart--set-mode record)
                (warm-restart--apply-modes record))))
          (set-buffer-modified-p modified)))))
  (setq warm-restart--deferred-buffers
        (cl-remove-if-not (lambda (b) (and (buffer-live-p b) (buffer-local-value 'warm-restart--deferred b)))
                          warm-restart--deferred-buffers))
  (unless warm-restart--deferred-buffers
    (remove-hook 'window-buffer-change-functions #'warm-restart--wake-shown)
    (remove-hook 'before-save-hook #'warm-restart--wake-current)
    (advice-remove 'find-file-noselect #'warm-restart--wake-found)))

(defun warm-restart--wake-found (buffer)
  "BUFFER, which `find-file-noselect' found, woken first."
  (warm-restart--wake buffer)
  buffer)

(defun warm-restart--wake-shown (frame)
  "Wake each buffer FRAME's windows show (`window-buffer-change-functions')."
  (dolist (window (window-list frame 'never))
    (warm-restart--wake (window-buffer window))))

(defun warm-restart--wake-current ()
  "Wake the buffer about to be saved (`before-save-hook')."
  (warm-restart--wake (current-buffer)))

(defun warm-restart--wake-soon ()
  "Wake the unseen buffers when the editor is idle, the oldest first, one
an idle moment, while no key waits: code that reads a buffer unshown (an
agenda reading its files) finds its mode soon."
  (when (and warm-restart--deferred-buffers (not warm-restart--wake-timer))
    (setq warm-restart--wake-timer
          (run-with-idle-timer (+ (if-let* ((idle (current-idle-time))) (float-time idle) 0) 0.5) nil
                               #'warm-restart--wake-idle))))

(defun warm-restart--wake-idle ()
  (setq warm-restart--wake-timer nil)
  (when-let* ((buffer (car (last warm-restart--deferred-buffers))))
    (unless (input-pending-p) (warm-restart--wake buffer))
    (warm-restart--wake-soon)))

(defun warm-restart--verify (records buffers)
  "Problems where a restored buffer's text or flag differs from RECORDS."
  (cl-loop for record in records
           for buffer = (gethash (plist-get record :id) buffers)
           when (and (buffer-live-p buffer) (plist-get record :blob))
           nconc (with-current-buffer buffer
                   (append (unless (equal (buffer-hash) (plist-get record :hash))
                             (list (list :buffer (buffer-name) :reason "text differs from the image" :text t)))
                           (unless (eq (and (buffer-modified-p) t) (plist-get record :modified))
                             (list (list :buffer (buffer-name) :reason "modified flag differs from the image")))))))

;;;; Restoring the session

(defun warm-restart--live-leaf (leaf names)
  "Window state LEAF with its buffers renamed by NAMES, a table from old
names to (BUFFER . OWNED).  A window on an OWNED buffer, whose handler
rebuilt its text, shows the buffer's own point from its beginning
\(redisplay scrolls to point): the carried start and point were the old
text's, and a state with no start would leave the window's start marker
pointing nowhere, where `pos-visible-in-window-p' and `posn-at-point'
signal.  A leaf whose buffer is gone shows its first live previous
buffer."
  (let* ((rename (lambda (name) (let ((buffer (car (gethash name names))))
                                  (cond ((buffer-live-p buffer) (buffer-name buffer))
                                        ((get-buffer name) name)))))
         (own-point (lambda (name props)
                      (let ((entry (gethash name names)))
                        (if (and (cdr entry) (buffer-live-p (car entry)))
                            (with-current-buffer (car entry)
                              (append (list (cons 'start (point-min)) (cons 'point (point)))
                                      (cl-remove-if (lambda (p) (memq (car-safe p) '(point start))) props)))
                          props))))
         (leaf (mapcar (lambda (item)
                         (pcase item
                           (`(buffer ,name . ,props)
                            `(buffer ,(or (funcall rename name) name) ,@(funcall own-point name props)))
                           (`(,(and (or 'prev-buffers 'next-buffers) key) . ,entries)
                            ;; An entry is (NAME START POINT), or a bare name.
                            (cons key (cl-loop for entry in entries
                                               for live = (funcall rename (if (consp entry) (car entry) entry))
                                               when live collect (if (consp entry) (cons live (cdr entry)) live))))
                           (_ item)))
                       leaf))
         (buffer (assq 'buffer leaf)))
    (if (or (null buffer) (get-buffer (cadr buffer)))
        leaf
      (if-let* ((prev (car (cdr (assq 'prev-buffers leaf)))))
          (mapcar (lambda (item)
                    (if (eq item buffer)
                        `(buffer ,(car prev) ,@(mapcar (lambda (prop)
                                                         (pcase (car-safe prop)
                                                           ('start (cons 'start (nth 1 prev)))
                                                           ('point (cons 'point (nth 2 prev)))
                                                           (_ prop)))
                                                       (cddr buffer)))
                      item))
                  leaf)
        leaf))))

(defun warm-restart--live-tree (tree names)
  "Window state TREE with `warm-restart--live-leaf' applied to its leaves."
  (pcase (car-safe tree)
    ('leaf (warm-restart--live-leaf tree names))
    ((or 'hc 'vc) (mapcar (lambda (node) (warm-restart--live-tree node names)) tree))
    (_ tree)))

(defvar window-restore-killed-buffer-windows)

(defun warm-restart--put-frame (frame state)
  "Lay window STATE out in FRAME."
  (let ((window-restore-killed-buffer-windows t))
    (window-state-put state (frame-root-window frame) 'safe)))

(defun warm-restart--put-again-on-resize (frame state saved)
  "Lay STATE out in FRAME again if its size changes before the first
command: a new window gets its size from the compositor only after
startup, and the layout's sizes were the old frame's.  SAVED is the
image's state, for the windows' histories (`warm-restart--put-noted')."
  (let ((size (list (frame-pixel-width frame) (frame-pixel-height frame)))
        again stop)
    (setq stop (lambda ()
                 (remove-hook 'window-size-change-functions again)
                 (remove-hook 'pre-command-hook stop))
          again (lambda (changed)
                  (when (and (eq changed frame) (frame-live-p frame)
                             (not (equal size (list (frame-pixel-width frame) (frame-pixel-height frame)))))
                    (funcall stop)
                    (warm-restart--put-noted frame state saved))))
    (add-hook 'window-size-change-functions again)
    (add-hook 'pre-command-hook stop)))

(defvar warm-restart--histories nil
  "Each window a restore laid out before all its buffers were back, with
the buffers it had shown before and after as its image saved them:
\(WINDOW PREV NEXT), put back once they are (`warm-restart--restore-histories').")

(defvar warm-restart--history-names nil
  "Once a restore laid out early has every buffer back, its names
\(`warm-restart--names'): a layout put again
\(`warm-restart--put-again-on-resize') gives its windows their histories
at once.")

(defun warm-restart--put-noted (frame state saved)
  "Lay STATE out in FRAME and note the histories SAVED, its image's state,
gave its windows; give them those now when every buffer is back."
  (warm-restart--put-frame frame state)
  (warm-restart--note-histories frame saved)
  (when warm-restart--history-names (warm-restart--restore-histories warm-restart--history-names)))

(defun warm-restart--leaves (tree)
  "The leaves of window state TREE, in window order."
  (cond ((eq (car-safe tree) 'leaf) (list tree))
        ((memq (car-safe tree) '(vc hc)) (cl-mapcan #'warm-restart--leaves (cl-remove-if-not #'consp (cdr tree))))))

(defun warm-restart--note-histories (frame state)
  "Note the buffer histories window STATE gave FRAME's windows, in order."
  (let ((windows (window-list frame 'never (frame-first-window frame)))
        (leaves (warm-restart--leaves (cdr state))))
    (when (= (length windows) (length leaves))
      (cl-loop for window in windows for leaf in leaves
               do (push (list window (alist-get 'prev-buffers (cdr leaf)) (alist-get 'next-buffers (cdr leaf)))
                        warm-restart--histories)))))

(defun warm-restart--restore-histories (names)
  "Give each window laid out early the buffers it had shown before and
after that are back now (NAMES, `warm-restart--names''s), after those it
showed since."
  (let ((buffer-of (lambda (name) (let ((buffer (car (gethash name names)))) (and (buffer-live-p buffer) buffer)))))
    (pcase-dolist (`(,window ,prev ,next) warm-restart--histories)
      (when (window-live-p window)
        (let ((had (window-prev-buffers window)))
          (set-window-prev-buffers
           window (append had (cl-loop for (name start point) in prev
                                       for buffer = (funcall buffer-of name)
                                       when (and buffer (not (assq buffer had)) (not (eq buffer (window-buffer window))))
                                       collect (list buffer (warm-restart--marker buffer (or start 1))
                                                     (warm-restart--marker buffer (or point 1)))))))
        (let ((had (window-next-buffers window)))
          (set-window-next-buffers
           window (append had (cl-loop for entry in next
                                       for buffer = (funcall buffer-of (if (consp entry) (car entry) entry))
                                       when (and buffer (not (memq buffer had))) collect buffer)))))))
  (setq warm-restart--histories nil))

(defun warm-restart--restore-frames (frames names)
  "Lay out FRAMES' window states; NAMES is `warm-restart--live-leaf''s.
The first frame's goes in the selected frame; another needs a new frame."
  (cl-loop for spec in frames
           for first = t then nil
           for frame = (if first (selected-frame) (ignore-errors (make-frame)))
           for state = (let ((state (plist-get spec :state)))
                         (cons (car state) (warm-restart--live-tree (cdr state) names)))
           if (frame-live-p frame)
           do (warm-restart--put-noted frame state (plist-get spec :state))
           and do (warm-restart--put-again-on-resize frame state (plist-get spec :state))
           else do (push (list :frame (1+ (length warm-restart--problems)) :reason "no frame for its windows")
                         warm-restart--problems)))

(defun warm-restart--marker (buffer pos)
  "A marker at POS in BUFFER, or nil when BUFFER is not live."
  (and (buffer-live-p buffer) (with-current-buffer buffer (copy-marker (min pos (1+ (buffer-size)))))))

(defun warm-restart--restore-globals (form buffers &optional part)
  "Restore FORM's kill ring, registers, histories and globals; PART
`early' all but the registers, which point into BUFFERS, `registers'
those alone."
  (unless (eq part 'registers)
    (setq kill-ring (plist-get form :kill-ring)
          kill-ring-yank-pointer (nthcdr (or (plist-get form :yank-index) 0) kill-ring))
    (pcase-dolist (`(,var . ,value) (append (plist-get form :histories) (plist-get form :globals)))
      (set-default var value)))
  (unless (eq part 'early)
    (warm-restart--restore-registers form buffers)))

(defun warm-restart--restore-registers (form buffers)
  "Restore FORM's registers, positions in BUFFERS."
  (pcase-dolist (`(,char . ,value) (plist-get form :registers))
    (set-register char (pcase value
                         (`(warm-restart-position ,id ,pos) (warm-restart--marker (gethash id buffers) pos))
                         (`(file-query ,file ,pos) (or (warm-restart--marker (find-buffer-visiting file) pos) value))
                         (_ value)))))

(defun warm-restart--order (records buffers)
  "Put the restored buffers first in the buffer list, in RECORDS' order,
which was the old buffer list's, as one change."
  (romacs--lead-buffer-list (delq nil (mapcar (lambda (r) (gethash (plist-get r :id) buffers)) records))))

(defun warm-restart--names (records buffers)
  "A table from each record's old name to (BUFFER . OWNED), its restored
buffer and whether a handler rebuilt that buffer's text."
  (let ((names (make-hash-table :test #'equal)))
    (dolist (record records)
      (when-let* ((buffer (gethash (plist-get record :id) buffers)))
        (puthash (plist-get record :name) (cons buffer (eq (plist-get record :strategy) 'handler)) names)))
    names))

(defvar warm-restart-lock-pending nil
  "Buffer-local, set: lock this buffer's file at its next edit though it is
modified.  GNU locks only at the first edit of an unmodified buffer; a
restored buffer is modified already, so a lock another editor held at the
restore would never be asked about.  lisp/vm/insdel.el locks while this
is local and removes it once `lock-file' returns; a refusal refuses the
edit and the next one asks again.  Permanent: a change of major mode
leaves the buffer modified, so it must keep the pending lock too.")
(put 'warm-restart-lock-pending 'permanent-local t)

(defun warm-restart--load-userlock ()
  "Load userlock.el's functions, so binding them does not autoload over the binding."
  (dolist (function '(ask-user-about-lock ask-user-about-supersession-threat
                      userlock--ask-user-about-supersession-threat))
    (when (autoloadp (symbol-function function))
      (autoload-do-load (symbol-function function) function))))

(defun warm-restart--lock (buffers)
  "Lock the files of the modified BUFFERS, as their next edit would.
Nothing asks here.  A lock another editor holds is left, named, and
asked about at the next edit of the buffer or of a view of its text
\(`warm-restart-lock-pending').
A file changed on disk since it was read is named: saving asks."
  (warm-restart--load-userlock)
  (let (problems)
    (cl-letf (((symbol-function 'ask-user-about-lock)
               (lambda (_file opponent)
                 (push (list :lock opponent :reason "file locked by another editor; asks at the next edit") problems)
                 (setq-local warm-restart-lock-pending t)
                 nil))
              ((symbol-function 'ask-user-about-supersession-threat)
               (lambda (file)
                 (push (list :lock file :reason "changed on disk since it was read; saving asks first") problems))))
      (dolist (buffer buffers)
        (when (buffer-live-p buffer)
          (with-current-buffer buffer
            (when (and buffer-file-name (buffer-modified-p) (not (buffer-base-buffer)))
              (condition-case err (lock-buffer)
                (error (push (list :lock buffer-file-name :reason (error-message-string err)) problems))))))))
    (when problems
      (setq warm-restart-last-report (plist-put warm-restart-last-report :locks problems)))))

(defun warm-restart--activate ()
  "Run and clear `warm-restart-activate-functions'; return their problems."
  (let ((functions warm-restart-activate-functions) problems)
    (setq warm-restart-activate-functions nil)
    (dolist (function functions)
      (condition-case err (funcall function)
        (error (push (list :activate function :reason (error-message-string err)) problems))))
    (nreverse problems)))

(defun warm-restart--invalid (form blobs)
  "Why FORM, with BLOBS, is not an image this build can apply, or nil."
  (let ((ids (make-hash-table :test #'equal))
        (records (and (proper-list-p (plist-get form :buffers)) (proper-list-p (plist-get form :starred))
                      (append (plist-get form :buffers) (plist-get form :starred)))))
    (cond
     ((not (eql (plist-get form :schema) warm-restart--schema))
      (format "image schema %S; this build reads %d" (plist-get form :schema) warm-restart--schema))
     ((not (and (proper-list-p (plist-get form :buffers)) (proper-list-p (plist-get form :starred))))
      "its buffers are not a list")
     (t (cl-loop for record in records
                 for id = (plist-get record :id)
                 for blob = (plist-get record :blob)
                 for strategy = (plist-get record :strategy)
                 unless (and (stringp id) (stringp (plist-get record :name)) (not (gethash id ids)))
                 return (format "record %S has no name or a repeated id" id)
                 do (puthash id t ids)
                 unless (memq strategy '(file text indirect handler starred))
                 return (format "record %S has strategy %S" id strategy)
                 unless (or (null blob) (and (natnump blob) (< blob (length blobs))))
                 return (format "record %S names blob %S of %d" id blob (length blobs))
                 when (and (memq strategy '(text starred)) (null blob)) return (format "text record %S has no blob" id)
                 when (and (eq strategy 'file) (not (stringp (plist-get record :file))))
                 return (format "file record %S has no file" id)
                 finally return
                 (cl-loop for record in records
                          when (and (eq (plist-get record :strategy) 'indirect)
                                    (not (gethash (plist-get record :base) ids)))
                          return (format "indirect record %S has no base" (plist-get record :id))))))))

(defun warm-restart--read (file)
  "The form and the blob vector of image FILE, checked whole before any of
it is applied."
  (pcase-let* ((`(,text . ,blobs) (romacs--session-image-read file))
               (`(,form . ,end) (read-from-string text)))
    (when-let* ((why (or (and (/= end (length text)) "text after the image's form")
                         (warm-restart--invalid form blobs))))
      (error "Not a usable session image: %s" why))
    (cons form blobs)))

(defun warm-restart--ordered (records)
  "RECORDS in the order they are restored: each indirect buffer after the
buffers it may be based on."
  (append (cl-remove 'indirect records :key (lambda (r) (plist-get r :strategy)))
          (cl-remove-if-not (lambda (r) (eq (plist-get r :strategy) 'indirect)) records)))

(defun warm-restart--shown (form)
  "The names of the buffers FORM's windows show."
  (let ((names nil))
    (cl-labels ((walk (x) (when (consp x)
                            (if (and (eq (car x) 'buffer) (stringp (car-safe (cdr x))))
                                (push (cadr x) names)
                              (walk (car x))
                              (walk (cdr x))))))
      (dolist (frame (plist-get form :frames)) (walk (plist-get frame :state))))
    names))

(defun warm-restart--split (form records)
  "RECORDS as (SHOWN . UNSEEN), each in restore order: those FORM's
windows show, with the buffers they are indirect to, and the rest."
  (let* ((names (warm-restart--shown form))
         (shown (cl-remove-if-not (lambda (record) (member (plist-get record :name) names)) records))
         (bases (cl-loop for record in shown
                         when (eq (plist-get record :strategy) 'indirect) collect (plist-get record :base)))
         (shown (cl-remove-if-not (lambda (record) (or (memq record shown) (member (plist-get record :id) bases)))
                                  records)))
    (cons (warm-restart--ordered shown)
          (warm-restart--ordered (cl-remove-if (lambda (record) (memq record shown)) records)))))

(defun warm-restart--restore-one (record blobs buffers)
  "Restore RECORD with BLOBS into BUFFERS: passively, locking nothing, and
asking nothing, since no one may be there to answer (a prompt signals,
and the buffer is rescued as text).  Its problems."
  (let ((warm-restart-passive t) (create-lockfiles nil) (inhibit-interaction t)
        (warm-restart--problems nil))
    (warm-restart--restore-buffer record blobs buffers)
    ;; Saved over, a file would get the stars: the user is told.
    (when (plist-get record :starred)
      (push (list :buffer (plist-get record :name) :reason "its secrets were carried as stars, not as their text")
            warm-restart--problems))
    warm-restart--problems))

(defun warm-restart--applied (form records buffers problems &optional laid-out)
  "The restore's state once FORM's RECORDS are back in BUFFERS with
PROBLEMS: its globals restored, and the text checked.  LAID-OUT: its
windows and all but its registers were already (`warm-restart--lay-out')."
  (let ((warm-restart-passive t)) (warm-restart--restore-globals form buffers (and laid-out 'registers)))
  (list :form form :records records :buffers buffers :laid-out laid-out
        :problems (append (warm-restart--verify records buffers) problems)))

(defun warm-restart--lay-out (form records buffers)
  "Show FORM's windows with the buffers of RECORDS restored so far in
BUFFERS, first in the buffer list, and its kill ring, histories and
globals; its registers wait for every buffer.  The problems."
  (let ((warm-restart--problems nil))
    (setq warm-restart--histories nil warm-restart--history-names nil)
    (let ((warm-restart-passive t)) (warm-restart--restore-globals form buffers 'early))
    (warm-restart--order records buffers)
    (condition-case err
        (warm-restart--restore-frames (plist-get form :frames) (warm-restart--names records buffers))
      (error (push (list :frame 1 :reason (error-message-string err)) warm-restart--problems)))
    warm-restart--problems))

(defun warm-restart--apply (form blobs)
  "Recreate FORM's buffers with BLOBS, and its globals.
Return the restore's state: FORM, the records, the buffers by id, and
the problems so far."
  (let ((records (warm-restart--prepare form)) (buffers (make-hash-table :test #'equal)) (problems nil))
    (dolist (record (warm-restart--ordered records))
      (setq problems (nconc (warm-restart--restore-one record blobs buffers) problems)))
    (warm-restart--applied form records buffers problems)))

(defun warm-restart--restore-later (file)
  "Restore image FILE from the command loop, a record a turn: a session's
socket and window come first, and no large buffer holds them back or
freezes the session.  The buffers its windows show come first, then its
windows are laid out, then the rest come back while no key waits, their
modes waiting for each to be shown (`warm-restart--defer').  Its progress
is the `restore' stage of `romacs-startup-progress'."
  (let ((start (float-time)))
    (condition-case err
        (pcase-let* ((`(,form . ,blobs) (progn (warm-restart--claim-pending file) (warm-restart--read file)))
                     (records (warm-restart--prepare form)))
          (romacs-startup-progress 'restore (format "Restoring buffers 0/%d" (length records))
                                   (list :status 'running :done 0 :of (length records) :seconds 0))
          (pcase-let ((`(,shown . ,unseen) (warm-restart--split form records)))
            (warm-restart--step file start form blobs records shown unseen 0 (make-hash-table :test #'equal) nil)))
      ((error quit) (warm-restart--report-later file start (list :error (error-message-string err)))))))

(defun warm-restart--step (file start form blobs records pending unseen done buffers problems)
  "Restore the first of PENDING, the records of FORM's RECORDS shown in
its windows after the DONE restored, then the rest from the next turns of
the command loop; with none left, lay out its windows and go on with the
UNSEEN (`warm-restart--step-unseen') for the restore of FILE begun at
START."
  (condition-case err
      (if (null pending)
          (warm-restart--step-unseen file start form blobs records unseen done buffers
                                     (nconc (warm-restart--lay-out form records buffers) problems))
        (warm-restart--restored-one start blobs records (car pending) done buffers problems nil
                                    (lambda (done problems)
                                      (run-at-time 0 nil #'warm-restart--step file start form blobs records
                                                   (cdr pending) unseen done buffers problems))))
    ((error quit) (warm-restart--stopped file start records done err))))

(defun warm-restart--step-unseen (file start form blobs records pending done buffers problems)
  "As `warm-restart--step', for PENDING, records no window shows: each once
no key waits (else when the editor is next idle), its modes waiting for
it to be shown; with none left, finish the restore."
  (condition-case err
      (cond ((null pending)
             (prog1 (warm-restart--report-later file start (warm-restart--applied form records buffers problems t))
               (warm-restart--wake-soon)))
            ((input-pending-p)
             (run-with-idle-timer 0.1 nil #'warm-restart--step-unseen file start form blobs records pending done buffers problems))
            (t (warm-restart--restored-one start blobs records (car pending) done buffers problems t
                                           (lambda (done problems)
                                             (run-at-time 0 nil #'warm-restart--step-unseen file start form blobs records
                                                          (cdr pending) done buffers problems)))))
    ((error quit) (warm-restart--stopped file start records done err))))

(defun warm-restart--restored-one (start blobs records record done buffers problems defer next)
  "Restore RECORD of RECORDS with BLOBS into BUFFERS, DEFER its modes when
non-nil, say how far the restore begun at START is, and call NEXT with
the count done and the problems."
  (let ((problems (nconc (let ((warm-restart--defer defer)) (warm-restart--restore-one record blobs buffers)) problems))
        (message-log-max nil))
    (romacs-startup-progress 'restore (format "Restoring buffers %d/%d" (1+ done) (length records))
                             (list :status 'running :done (1+ done) :of (length records)
                                   :name (plist-get record :name) :seconds (- (float-time) start)))
    (message "Restoring session: %d/%d" (1+ done) (length records))
    (funcall next (1+ done) problems)))

(defun warm-restart--stopped (file start records done err)
  "End the restore of FILE begun at START, stopped by ERR after DONE of RECORDS."
  (warm-restart--report-later file start (list :error (format "stopped after %d of %d records: %s" done (length records)
                                                             (error-message-string err)))))

(defun warm-restart--report-later (file start state)
  "Finish the restore of FILE begun at START with STATE, as the
`restore' step's end."
  (let ((report (warm-restart--report file start state)))
    (romacs-startup-progress 'restore (if (plist-get report :error) "Session restore failed" "Session restored")
                             (list :status (if (plist-get report :error) 'failed 'done)
                                   :done (plist-get report :buffers) :of (plist-get report :records)
                                   :error (plist-get report :error) :seconds (plist-get report :seconds)))
    report))

(defun warm-restart--finish (state)
  "Lay out STATE's windows, order its buffers and activate them; return
the report."
  (let* ((form (plist-get state :form))
         (records (plist-get state :records))
         (buffers (plist-get state :buffers))
         (warm-restart--problems nil))
    (if (plist-get state :laid-out)
        ;; Laid out before every buffer was back: their windows' histories
        ;; now, and at any layout put again.
        (progn (setq warm-restart--history-names (warm-restart--names records buffers))
               (warm-restart--restore-histories warm-restart--history-names))
      (setq warm-restart--history-names nil)
      (warm-restart--order records buffers)
      (condition-case err
          (warm-restart--restore-frames (plist-get form :frames) (warm-restart--names records buffers))
        (error (push (list :frame 1 :reason (error-message-string err)) warm-restart--problems))))
    (setq warm-restart--histories nil)
    (setq warm-restart--problems (append (warm-restart--activate) warm-restart--problems))
    ;; The adopters have claimed what they restore.  What the previous
    ;; session left in the keeper and nobody claimed ends here, as it
    ;; would have ended with that session; what is marked kept stays.
    (when (and (fboundp 'romacs-keeper-attached-p) (romacs-keeper-attached-p))
      (ignore-errors (romacs-keeper-close-orphans)))
    (let ((modified (cl-loop for buffer being the hash-values of buffers
                             when (buffer-modified-p buffer) collect buffer)))
      (run-at-time 0 nil #'warm-restart--lock modified))
    (list :buffers (hash-table-count buffers) :records (length records) :lost (plist-get form :lost)
          :problems (append (plist-get state :problems) (nreverse warm-restart--problems)))))

(defun warm-restart--keep-p (report)
  "Non-nil when REPORT says carried text may not have arrived."
  (or (plist-get report :error)
      (cl-some (lambda (p) (plist-get p :text)) (plist-get report :problems))
      (< (plist-get report :buffers) (plist-get report :records))))

(defun warm-restart--consume (file)
  "Keep FILE as the last restored image, replacing the previous one."
  (rename-file file (expand-file-name "restored.image" (file-name-directory file)) t))

(defun warm-restart--announce (report)
  "Say what REPORT's restore did; a failure is a warning, which shows.
Either way it names each buffer whose unsaved text the image left out."
  (let* ((lost (length (plist-get report :lost)))
         (unsaved (cl-loop for entry in (plist-get report :lost)
                           when (plist-get entry :unsaved) collect (plist-get entry :buffer)))
         (unsaved (if unsaved (format "; unsaved text not carried: %s" (string-join unsaved ", ")) ""))
         (problems (length (plist-get report :problems))))
    (if (warm-restart--keep-p report)
        (display-warning 'warm-restart
                         (format "Session restore incomplete: %s; the image stays at %s (M-x warm-restart-report)%s"
                                 (or (plist-get report :error) (format "%d problems" problems))
                                 (plist-get report :file) unsaved))
      (message "Session restored: %d buffers in %.2f s%s%s%s%s"
               (plist-get report :buffers) (plist-get report :seconds) unsaved
               (if (> lost 0) (format "; %d not carried" lost) "")
               (if (> problems 0) (format "; %d problems" problems) "")
               (if (> (+ lost problems) 0) " (M-x warm-restart-report)" "")))))

(defun warm-restart--report (file start state)
  "Finish STATE's restore of image FILE begun at START; return the report."
  (let ((report (append (list :file file)
                        (if (plist-get state :error) state
                          (condition-case err (warm-restart--finish state)
                            (error (list :error (error-message-string err))))))))
    (setq report (append (list :seconds (- (float-time) start)) report))
    (unless (warm-restart--keep-p report)
      (condition-case err (warm-restart--consume file)
        (error (setq report (plist-put report :problems
                                       (cons (list :image file :reason (error-message-string err))
                                             (plist-get report :problems)))))))
    (setq warm-restart-last-report report)
    (warm-restart--announce report)
    report))

(defun warm-restart-pending-file (server)
  "The file naming the image server SERVER's successor restores, while a
restart of it is under way."
  (expand-file-name (format "pending-%s" server) (warm-restart--directory)))

(defun warm-restart--publish-pending (image server)
  "Name IMAGE as what the next server SERVER restores.  Beside IMAGE,
its .server file names SERVER, so its restore removes just this record."
  (with-temp-file (concat (file-name-sans-extension image) ".server") (insert server))
  (with-temp-file (warm-restart-pending-file server) (insert (expand-file-name image))))

(defun warm-restart--withdraw-pending (image server)
  (dolist (file (list (warm-restart-pending-file server) (concat (file-name-sans-extension image) ".server")))
    (ignore-errors (delete-file file))))

(defun warm-restart--claim-pending (file)
  "Remove the pending record naming image FILE: this process restores it."
  (let ((sidecar (concat (file-name-sans-extension file) ".server")))
    (when (file-readable-p sidecar)
      (let ((record (warm-restart-pending-file (with-temp-buffer (insert-file-contents sidecar) (buffer-string)))))
        (when (equal (ignore-errors (with-temp-buffer (insert-file-contents record) (buffer-string)))
                     (expand-file-name file))
          (delete-file record)))
      (delete-file sidecar))))

;;;###autoload
(defun warm-restart-adopt-pending (server)
  "Restore the image a restart of server SERVER left, when this server was
started by someone else (a launcher) before the restart's successor."
  (let ((record (warm-restart-pending-file server)))
    (when (file-exists-p record)
      (let ((image (with-temp-buffer (insert-file-contents record) (buffer-string))))
        (delete-file record)
        (when (file-exists-p image) (warm-restart-restore-image image))))))

;;;###autoload
(defun warm-restart-restore-image (file &optional startup)
  "Restore the session image FILE over this session; return the report.
Under STARTUP, as `--restore' passes it, the restore begins once the
session serves, a record a turn of the command loop
\=(`warm-restart--restore-later'), and the value is nil.  The image is
kept as restored.image, or left where it is when any carried text may
not have arrived."
  (interactive (list (read-file-name "Session image: " (warm-restart--directory) nil t)))
  (if startup
      (progn (run-at-time 0 nil #'warm-restart--restore-later file) nil)
    (warm-restart--claim-pending file)
    (let* ((start (float-time))
           (state (condition-case err
                      (pcase-let ((`(,form . ,blobs) (warm-restart--read file)))
                        (warm-restart--apply form blobs))
                    (error (list :error (error-message-string err))))))
      (warm-restart--report file start state))))

;;;; Commands

(defun warm-restart--describe (entry)
  "One line describing a `lost' or problem ENTRY."
  (let ((what (cl-loop for key in '(:process :buffer :register :variable :lock :activate :frame :image)
                       when (plist-member entry key) return (format "%s %s" (substring (symbol-name key) 1)
                                                                    (plist-get entry key)))))
    (format "  %s: %s" (or what "?") (plist-get entry :reason))))

;;;###autoload
(defun warm-restart-report ()
  "Show what the last restore carried, did not carry, and what went wrong."
  (interactive)
  (let ((report warm-restart-last-report))
    (with-current-buffer (get-buffer-create "*Warm Restart*")
      (let ((inhibit-read-only t))
        (erase-buffer)
        (insert (format "Image: %s\nBuffers restored: %s\n\nNot carried:\n" (plist-get report :file)
                        (plist-get report :buffers))
                (mapconcat #'warm-restart--describe (plist-get report :lost) "\n")
                "\n\nProblems:\n"
                (mapconcat #'warm-restart--describe (append (plist-get report :problems) (plist-get report :locks)) "\n")
                "\n"))
      (special-mode)
      (display-buffer (current-buffer)))))

(defun warm-restart--launch-flags (argv)
  "The launch flags of ARGV, this process's arguments: the options that
choose how a session starts, without files, forms or an image."
  (let ((plain '("--loadup" "--gnu-lisp" "--no-gnu-lisp" "-Q" "-q" "--no-init" "-no-init-file"
                 "--no-init-file" "--live" "--trace-latency" "--trace-scene"))
        (valued '("--world-store" "--geometry" "--wayland" "--gc-floor"))
        flags)
    (while argv
      (let ((arg (pop argv)))
        (cond ((member arg plain) (push arg flags))
              ((and (member arg valued) argv) (push arg flags) (push (pop argv) flags))
              ((equal arg "--") (setq argv nil)))))
    (nreverse flags)))

(defun warm-restart--argv ()
  "This process's command line."
  (with-temp-buffer
    (set-buffer-multibyte nil)
    (insert-file-contents-literally "/proc/self/cmdline")
    (mapcar (lambda (arg) (decode-coding-string arg 'utf-8))
            (split-string (buffer-string) "\0" t))))

(defun romacs-restart--command ()
  "The successor's program and arguments, and whether it chooses its runtime."
  (let ((given (or romacs-restart-command
                   (and-let* ((env (getenv "ROMACS_RESTART_COMMAND"))) (split-string-and-unquote env)))))
    (if given
        (cons (cons (expand-file-name (car given)) (cdr given)) t)
      (cons (cons (expand-file-name invocation-name invocation-directory)
                  (warm-restart--launch-flags (cdr (warm-restart--argv))))
            nil))))

(defvar romacs-restart-environment nil
  "Entries (NAME=VALUE) given to the successor's environment, after
`romacs-restart-unset-environment' are removed: a restart request's target
build names its runtime here.")

(defvar romacs-server-name)

(defun romacs-restart--environment-arguments ()
  "systemd-run arguments giving a unit this environment, by name."
  (cl-loop for entry in process-environment
           when (and (string-match "\\`\\([A-Za-z_][A-Za-z0-9_]*\\)=" entry)
                     (not (member (match-string 1 entry) '("INVOCATION_ID" "JOURNAL_STREAM" "NOTIFY_SOCKET"
                                                            "MANAGERPID" "SYSTEMD_EXEC_PID"))))
           collect (concat "--setenv=" (match-string 1 entry))))

(defcustom romacs-restart-deadlines '(:exit 60 :start 180 :ready 300)
  "Seconds a server's restart gives each step: this session's end (then it
is ended), the new session's socket and window, and its init done and
answering within a second (its restore goes on after).  A step past its
deadline rolls the restart back."
  :type '(plist :value-type natnum))

(defvar romacs-restart-requester nil
  "The agent session the running restart was asked for by, told its verdict.")

(defun romacs-restart--directory ()
  "Where restarts' plans, records and kept images go."
  (expand-file-name "romacs/restarts/" (or (getenv "XDG_STATE_HOME") "~/.local/state")))

(defun romacs-restart--plan (image command own-runtime server)
  "Write the plan of a supervised restart of server SERVER into COMMAND
\=(program and launch flags; with OWN-RUNTIME, one that chooses its runtime),
restoring IMAGE; return its file.  The image is kept beside it for a
rollback, as the restore consumes it."
  (let* ((dir (progn (make-directory (romacs-restart--directory) t) (romacs-restart--directory)))
         (id (file-name-base image))
         (keep (expand-file-name (concat id ".image") dir))
         (file (expand-file-name (concat id ".plan.json") dir))
         (set (let ((table (make-hash-table :test #'equal)))
                (dolist (entry romacs-restart-environment table)
                  (when (string-match "\\`\\([^=]+\\)=\\(.*\\)\\'" entry)
                    (puthash (match-string 1 entry) (match-string 2 entry) table))))))
    (condition-case nil (add-name-to-file image keep t) (file-error (copy-file image keep t)))
    (with-temp-file file
      (insert (json-serialize
               (list :id id :server server :image (expand-file-name image) :keep keep
                     :record (expand-file-name (concat id ".json") dir)
                     :requester (or romacs-restart-requester :null)
                     :old (list :pid (emacs-pid) :exe (expand-file-name invocation-name invocation-directory)
                                :flags (vconcat (warm-restart--launch-flags (cdr (warm-restart--argv)))))
                     :target (list :exe (car command) :flags (vconcat (cdr command)) :set set
                                   :unset (if own-runtime (vconcat romacs-restart-unset-environment) []))
                     :deadlines romacs-restart-deadlines))))
    (set-file-modes file #o600)
    file))

(defun romacs-restart--spawn (image)
  "Start the successor restoring IMAGE, detached from this process.  A
server's successor is the same server, started and judged by a
supervisor (`romacs --restart-supervise PLAN') outside this process's
unit, whose end would end it too; it rolls back to this build when the
successor does not come up, so it keeps this session's environment and
gives the successor's changes to the successor alone.  Another session's
successor starts once this one is gone, its output in the image's .log
file."
  (pcase-let* ((`(,command . ,own-runtime) (romacs-restart--command))
               (default-directory (or (bound-and-true-p command-line-default-directory) (expand-file-name "~/")))
               (server (bound-and-true-p romacs-server-name)))
    (if server
        (romacs-restart--supervise (romacs-restart--plan image command own-runtime server) server)
      (let ((process-environment
             (append romacs-restart-environment
                     (if own-runtime
                         (cl-remove-if (lambda (entry)
                                         (cl-some (lambda (var) (or (equal entry var) (string-prefix-p (concat var "=") entry)))
                                                  romacs-restart-unset-environment))
                                       process-environment)
                       process-environment))))
        (romacs-restart--start command image (concat (file-name-sans-extension image) ".log"))))))

(defun romacs-restart--supervise (plan server)
  "Start PLAN's supervisor for server SERVER, this build's own binary,
outside this process's unit."
  (let ((program (expand-file-name invocation-name invocation-directory)))
    (if (getenv "INVOCATION_ID")
        (unless (zerop (apply #'call-process "systemd-run" nil nil nil
                              (append (list "--user" "--quiet" "--collect"
                                            (format "--unit=romacs-restart-%s-%d" server (emacs-pid)))
                                      (romacs-restart--environment-arguments)
                                      (list "--" program "--restart-supervise" plan))))
          (error "systemd-run could not start the restart's supervisor"))
      (call-process "setsid" nil 0 nil "-f" program "--restart-supervise" plan))))

(defun romacs-restart-record (&optional id)
  "The record of restart ID, the newest when nil, as a hash table; nil when
there is none."
  (let* ((dir (romacs-restart--directory))
         (file (if id (expand-file-name (concat id ".json") dir)
                 (and (file-directory-p dir)
                      (car (sort (directory-files dir t "\\`[^.].*[0-9]\\.json\\'" t) #'file-newer-than-file-p))))))
    (when (and file (file-readable-p file))
      (with-temp-buffer (insert-file-contents file) (json-parse-buffer)))))

(defun romacs-restart--summary (record)
  "One line saying how RECORD's restart ended."
  (let* ((plan (gethash "plan" record))
         (phases (gethash "phases" record))
         (seconds (and (> (length phases) 0) (gethash "at" (aref phases (1- (length phases)))))))
    (pcase (gethash "verdict" record)
      ("verified" (format "Romacs restart %s verified: %s, up in %.1f s" (gethash "id" plan)
                          (gethash "exe" (gethash "target" plan)) (or seconds 0)))
      ("rolled-back" (format "Romacs restart %s rolled back to %s: %s" (gethash "id" plan)
                             (gethash "exe" (gethash "old" plan)) (gethash "failed" record)))
      (verdict (format "Romacs restart %s: %s %s" (gethash "id" plan) (or verdict (gethash "state" record))
                       (or (gethash "failed" record "") ""))))))

(defun romacs-restart-verdict (file)
  "Say how the restart whose record is FILE ended: in the echo area, and
to the agent session that asked for it.  The supervisor calls this in the
session that serves once it has judged it."
  (let* ((record (with-temp-buffer (insert-file-contents file) (json-parse-buffer)))
         (text (romacs-restart--summary record))
         (requester (gethash "requester" (gethash "plan" record))))
    (message "%s" text)
    (when (and (stringp requester) (require 'romacs-agent nil t) (fboundp 'romacs-agent-tools--deliver))
      (when-let* ((session (romacs-agent-load-session requester)))
        (romacs-agent-tools--deliver session (format "%s\nRecord: %s" text file)
                                     (lambda () (ignore-errors (delete-file (romacs-agent-tools--restart-wake-file session)))
                                       (romacs-agent-mark (romacs-agent-id session) 'wake nil)))))
    text))

(defun romacs-restart--start (command image log)
  "Run COMMAND restoring IMAGE once this process is gone, detached, its
output to LOG: the successor of a session that is no server."
  (apply #'call-process "setsid" nil 0 nil "-f" "sh" "-c"
         (concat "while kill -0 \"$1\" 2>/dev/null; do sleep 0.1; done; shift; "
                 "exec \"$@\" >>\"$0\" 2>&1 </dev/null")
         log (number-to-string (emacs-pid)) (append command (list "--restore" image))))

(defun warm-restart--confirm (plan force asked)
  "Refuse PLAN while a buffer of it is blocked; unless FORCE, ask whether
the processes that would end and ask on exit, those not in ASKED, may end.
Return ASKED with the processes asked about, ASKED itself when none was:
processes, not names, as a name freed meanwhile may be taken again."
  (when-let* ((blocked (warm-restart--entries plan 'blocked)))
    (user-error "Cannot restart now: %s"
                (mapconcat (lambda (e) (format "%s (%s)" (plist-get e :name) (plist-get e :reason))) blocked "; ")))
  (let ((asking (and (not force)
                     (cl-loop for process in (process-list)
                              when (and (process-live-p process) (process-query-on-exit-flag process)
                                        (not (memq process asked)))
                              collect process))))
    (when (and asking
               (not (yes-or-no-p (format "Not carried, they end: %s.  Restart anyway? "
                                         (mapconcat #'process-name asking ", ")))))
      (user-error "Restart cancelled"))
    (append asking asked)))

;;;###autoload
(defun romacs-restart (&optional force)
  "Restart into a new process, carrying this session.
Write a session image, run `kill-emacs-hook' so packages save their
state, start the new process with `--restore IMAGE', and exit.  A blocked
buffer refuses the restart and names why.  Processes, terminals and
network connections are not carried yet: when one would ask before
`kill-emacs', so does this, unless FORCE.  See `romacs-restart-command'
for running a newer build."
  (interactive)
  (let ((asked (warm-restart--confirm (warm-restart-plan) force nil)) plan)
    ;; What the successor reads is on disk before it starts, and before
    ;; its supervisor's deadline runs.
    (run-hooks 'warm-restart-before-handover-functions)
    ;; Timers and process output run while those hooks wait, and while a
    ;; question waits for its answer: the plan is made again until one
    ;; needs no question, so what they made is carried, refused or asked
    ;; about.
    (while (let ((before asked))
             (setq plan (warm-restart-plan) asked (warm-restart--confirm plan force asked))
             (not (eq asked before))))
    (let ((image (warm-restart-write plan))
          (server (bound-and-true-p romacs-server-name)))
      ;; Before anything closes the endpoint: whoever starts this server
      ;; next restores the image, the successor or a server a launcher
      ;; started first (`warm-restart-adopt-pending').
      (when server (warm-restart--publish-pending image server))
      ;; The successor's start first: its supervisor waits for this
      ;; session's end, and ends it past a deadline, so from here nothing
      ;; a hook does can hold the restart.
      (condition-case err
          (romacs-restart--spawn image)
        (error (when server (warm-restart--withdraw-pending image server))
               (user-error "Restart failed, this session goes on: %s (image %s)" (error-message-string err) image)))
      ;; A restart, not a quit: what the keeper holds for this session
      ;; stays for the successor (romacs-keeper.el).  Each hook alone, and
      ;; asking nothing, since nobody may be there to answer: one that
      ;; fails is said, and the others run.
      (let ((romacs-keeper-restarting t) (inhibit-interaction t))
        (run-hook-wrapped 'kill-emacs-hook
                          (lambda (function)
                            (condition-case err (funcall function)
                              ((error quit) (message "Restart: %s failed: %s" function (error-message-string err))))
                            nil)))
      ;; The new process locks these files again; this one's locks would
      ;; name a pid that is about to end.
      (dolist (buffer (buffer-list))
        (with-current-buffer buffer (when (and buffer-file-name (buffer-modified-p)) (ignore-errors (unlock-buffer)))))
      (dolist (process (process-list)) (set-process-query-on-exit-flag process nil))
      (let ((kill-emacs-hook nil)) (kill-emacs 0)))))

;;;; Requests

;; An agent's restart: once the plan has nothing BLOCKED, READY holds and
;; the editor waits between commands, for `warm-restart-request-delay'
;; seconds, it restarts.  It never interrupts a command and never asks:
;; what `romacs-restart' would ask about counts as BLOCKED unless FORCE.

(defcustom warm-restart-request-delay 2
  "Seconds a `warm-restart-request' must find nothing in its way before it
restarts."
  :type 'number)

(defcustom warm-restart-request-interval 1
  "Seconds between checks of a pending `warm-restart-request'."
  :type 'number)

(defcustom warm-restart-request-expiry 900
  "Seconds a `warm-restart-request' waits for what is in its way to clear."
  :type 'number)

(defvar warm-restart-last-result nil
  "The last request's state: (:status scheduled|deferred|expired|cancelled|failed ...).")

(defvar warm-restart--request nil
  "The pending request, a plist, or nil.")

(defvar romacs-mcp-admission-paused)
(declare-function romacs-mcp-in-flight-p "romacs-mcp")
(declare-function romacs-mcp-resume-admission "romacs-mcp")

(defun warm-restart--target (target)
  "The restart command and environment for TARGET: a build directory (its
romacs-vm-gui with its own runtime) or a binary (its runtime its own).
A build finds its Lisp in its runtime root itself, so an older or newer
layout of that tree restarts as well.  TARGET is taken by its true name:
a link to a build (an installer's `current') names the build it points
to now, so the session stays on that build when the link moves, and its
runtime root is spelled as a launcher that resolves the link spells it,
whose worlds it then boots from."
  (let ((target (file-truename target)))
    (cond ((file-directory-p target)
           (let ((binary (expand-file-name "romacs-vm-gui" target)))
             (dolist (path (list binary (expand-file-name "etc" target)))
               (unless (file-exists-p path) (user-error "Not a build directory: %s has no %s" target
                                                       (file-name-nondirectory path))))
             (unless (file-executable-p binary) (user-error "%s is not executable" binary))
             (list (cons binary (warm-restart--launch-flags (cdr (warm-restart--argv))))
                   (concat "ROMACS_RUNTIME_ROOT=" target))))
          ((file-executable-p target)
           (list (cons target (warm-restart--launch-flags (cdr (warm-restart--argv))))))
          (t (user-error "No build at %s" target)))))

(defvar warm-restart-obstacle-functions nil
  "Functions, each saying what keeps a requested restart from starting now:
a list of reasons (strings), or nil when it has none.  A request waits
while any names one, as it waits for an idle session, never blocking the
editor: work a restart must not cut short (records still being written)
is done in the background meanwhile.")

(defun warm-restart--hook-obstacles ()
  "The reasons `warm-restart-obstacle-functions' give, in their order."
  (let ((reasons nil))
    (run-hook-wrapped 'warm-restart-obstacle-functions
                      (lambda (function) (setq reasons (append reasons (funcall function))) nil))
    reasons))

(defun warm-restart--obstacles (request)
  "Why REQUEST cannot restart now, as strings; nil when it can."
  (let ((ready (plist-get request :ready)))
    (cond ((not (current-idle-time)) (list "a command is running"))
          ((or (> (minibuffer-depth) 0) (> (recursion-depth) 0)) (list "the minibuffer or a recursive edit is open"))
          ((and ready (funcall ready)))
          ((and (fboundp 'romacs-mcp-in-flight-p) (romacs-mcp-in-flight-p)) (list "a Romacs tool call is in flight"))
          (t (let ((plan (warm-restart-plan)))
               (append (warm-restart--hook-obstacles)
                       (mapcar (lambda (e) (format "%s (%s)" (plist-get e :name) (plist-get e :reason)))
                               (warm-restart--entries plan 'blocked))
                       (unless (plist-get request :force)
                         (mapcar (lambda (p) (format "process %s would end" (plist-get p :process)))
                                 (cl-remove-if-not (lambda (p) (plist-get p :query)) (plist-get plan :processes))))))))))

(defun warm-restart--request-check (request)
  (when (eq request warm-restart--request)
    (let ((obstacles (warm-restart--obstacles request)) (now (float-time)))
      (cond ((> now (plist-get request :deadline))
             (warm-restart--request-end (list :status 'expired :obstacles obstacles))
             (when-let* ((failed (plist-get request :failed)))
               (ignore-errors (funcall failed (format "it waited %d s and these were still in the way: %s"
                                                      (round (- now (plist-get request :since-request)))
                                                      (string-join obstacles "; "))))))
            (obstacles
             (setf (plist-get request :since) nil)
             (setq warm-restart-last-result (list :status 'deferred :obstacles obstacles)))
            ((null (plist-get request :since)) (setf (plist-get request :since) now))
            ;; Quiet since the first clean check, and idle all that time: a
            ;; command between two checks starts the wait again.
            ((and (>= (- now (plist-get request :since)) (plist-get request :delay))
                  (>= (float-time (or (current-idle-time) 0)) (plist-get request :delay)))
             (warm-restart--request-run request))
            ((< (float-time (or (current-idle-time) 0)) (- now (plist-get request :since)))
             (setf (plist-get request :since) now))))))

(defun warm-restart--request-end (result)
  (when-let* ((timer (plist-get warm-restart--request :timer))) (cancel-timer timer))
  (setq warm-restart--request nil warm-restart-last-result result))

(defun warm-restart--resume-admission ()
  (if (fboundp 'romacs-mcp-resume-admission) (romacs-mcp-resume-admission)
    (setq romacs-mcp-admission-paused nil)))

(defun warm-restart--request-run (request)
  "Restart for REQUEST: new tool calls wait from here, and what is in the
way is asked once more first.  Any failure lets them run again and says
why."
  (setq romacs-mcp-admission-paused t)
  (condition-case err
      (let ((obstacles (warm-restart--obstacles request)))
        (if obstacles
            (progn (warm-restart--resume-admission)
                   (setf (plist-get request :since) nil)
                   (setq warm-restart-last-result (list :status 'deferred :obstacles obstacles)))
          (warm-restart--request-end (list :status 'restarting :target (plist-get request :target)))
          (pcase-let ((`(,command . ,environment) (and (plist-get request :target)
                                                       (warm-restart--target (plist-get request :target)))))
            (let ((romacs-restart-command (or command romacs-restart-command))
                  (romacs-restart-environment environment))
              (when (plist-get request :before) (funcall (plist-get request :before)))
              (romacs-restart t)))))
    ((error quit)
     (warm-restart--resume-admission)
     (when (eq warm-restart--request request) (warm-restart--request-end nil))
     (setq warm-restart-last-result (list :status 'failed :error (error-message-string err)))
     (when (plist-get request :failed)
       (ignore-errors (funcall (plist-get request :failed) (error-message-string err)))))))

(cl-defun warm-restart-request (&key ready target delay expiry force before failed)
  "Restart once nothing is in the way, as an agent asks.
Nothing BLOCKED, the editor between commands with no minibuffer open, no
Romacs tool call in flight and READY (a function, when given, returning
nil when ready or a list of reasons) for DELAY seconds (default
`warm-restart-request-delay'), checked every `warm-restart-request-interval';
up to EXPIRY seconds (default `warm-restart-request-expiry').  TARGET is a
build directory or binary to restart into (`warm-restart--target'), else
`romacs-restart-command''s.  A process that would end counts as in the way
unless FORCE.  BEFORE runs just before the restart; FAILED is called with
the error when the restart fails (the session goes on).  A new request
replaces a pending one.  Return `warm-restart-last-result'."
  (when target (warm-restart--target target))
  (warm-restart-cancel)
  ;; Every key the checks set is present: `setf' of a missing one would
  ;; cons a new head the timer's copy never sees.
  (let ((request (list :timer nil :since nil :since-request (float-time) :ready ready :target target :force force
                       :before before :failed failed
                       :delay (or delay warm-restart-request-delay)
                       :deadline (+ (float-time) (or expiry warm-restart-request-expiry)))))
    (setf (plist-get request :timer)
          (run-at-time warm-restart-request-interval warm-restart-request-interval
                       #'warm-restart--request-check request))
    (setq warm-restart--request request
          warm-restart-last-result (list :status 'scheduled :target target))))

(defun warm-restart-cancel ()
  "Cancel a pending `warm-restart-request'."
  (interactive)
  (when warm-restart--request (warm-restart--request-end (list :status 'cancelled))))

(provide 'warm-restart)
;;; warm-restart.el ends here
