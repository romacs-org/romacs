;;; romacs-agent-history.el --- The harnesses' own sessions, read and resumed -*- lexical-binding: t; -*-

;; SPDX-License-Identifier: GPL-3.0-or-later
;; Maintainer: Thanos Apollo <public@thanosapollo.org>

;;; Commentary:

;; Each harness driver supplies how to find, describe and read its own
;; transcripts.  This library lists them by provider instance, shows one
;; read-only, and resumes it as a Romacs session.
;;
;; Harness files are only ever read.  Listing is directory metadata; the
;; newest `romacs-agent-history-limit' transcripts of a directory are
;; described from a bounded head of each, cached by size and time, so a
;; redraw reads nothing new.  A view renders the end of a transcript, at
;; most `romacs-agent-history-view-bytes' of it.

;;; Code:

(require 'cl-lib)
(require 'romacs-agent)
(require 'romacs-agent-transcript)
(require 'romacs-agent-ui)

(defcustom romacs-agent-history-limit 50
  "How many of a directory's newest transcripts the home page lists."
  :type 'integer :group 'romacs-agent)

(defcustom romacs-agent-history-per-project 3
  "How many history rows a project shows until `h' shows them all."
  :type 'integer :group 'romacs-agent)

(defcustom romacs-agent-history-view-bytes (* 2 1024 1024)
  "How much of a transcript's end its view renders, in bytes."
  :type 'integer :group 'romacs-agent)

(defconst romacs-agent-history--heads '(131072 524288 1048576)
  "The heads read, in turn, until one names its transcript: Codex writes
its instructions before the first prompt.")

(defvar romacs-agent-history--descriptions (make-hash-table :test #'equal)
  "A transcript's path to (SIZE MTIME . DESCRIPTION), DESCRIPTION nil for
one with nothing to resume.")

(defvar romacs-agent-history--listing nil
  "The transcripts the last listing described, newest first per directory.")

;;; Listing

(defun romacs-agent-history--get (harness key)
  "How HARNESS's history is read: its driver's :history KEY."
  (plist-get (plist-get (romacs-agent--driver-of harness) :history) key))

(defun romacs-agent-history--places ()
  "The directories holding history, once per harness: (DIR HARNESS PROVIDER
CONFIG-DIR), every instance's own and its harness's default (under the
first instance of that harness, run with the default directory)."
  (let ((places nil))
    (dolist (provider (romacs-agent-instances))
      (let* ((harness (plist-get provider :harness))
             (default (romacs-agent--home harness nil))
             (config-dir (romacs-agent--config-dir provider)))
        (when (and (plist-get provider :enabled) (plist-get (romacs-agent--driver-of harness) :history))
          (push (list (or config-dir default) harness (plist-get provider :name) config-dir) places)
          (push (list default harness (plist-get provider :name) nil) places))))
    (let ((seen (make-hash-table :test #'equal)))
      (cl-loop for place in (nreverse places)
               for key = (list (car place) (cadr place))
               unless (gethash key seen)
               collect (progn (puthash key t seen) place)))))

(defun romacs-agent-history--describe (path size mtime harness)
  "PATH's description, read from its head once per SIZE and MTIME: from
a longer head while it names no title."
  (let ((cached (gethash path romacs-agent-history--descriptions)))
    (if (and cached (eql (car cached) size) (equal (cadr cached) mtime))
        (cddr cached)
      (let ((description nil) (heads romacs-agent-history--heads))
        (while heads
          (setq description (funcall (romacs-agent-history--get harness :describe)
                                     (romacs-agent-transcript-read path 0 (min size (car heads))
                                                                 (romacs-agent-history--get harness :keep))
                                     path))
          (setq heads (and (not (plist-get description :title)) (> size (car heads)) (cdr heads))))
        (puthash path (cons size (cons mtime description)) romacs-agent-history--descriptions)
        description))))

(declare-function romacs-agent-t3-thread-titles "romacs-agent-t3" (store))
(declare-function romacs-agent-t3-title-key "romacs-agent-t3" (instance native))

(defun romacs-agent-history--scan ()
  "The harnesses' transcripts: the newest of each directory, described
\(from their heads, once per size and time).  A transcript of an
instance T3 Code configures takes the title T3 gave its thread, when T3
ran it: its first prompt is then T3's preamble."
  (cl-loop with from-t3 = (let ((table (make-hash-table :test #'equal)))
                          ;; Each T3 instance's id and thread store, by name.
                          (dolist (i (romacs-agent-instances) table)
                            (when (plist-get i :t3-store)
                              (puthash (plist-get i :name) (cons (plist-get i :t3-id) (plist-get i :t3-store)) table))))
         with stores = (make-hash-table :test #'equal)
         for (dir harness provider config-dir) in (romacs-agent-history--places)
         for t3 = (gethash provider from-t3)
         ;; One read of each store a refresh, failed or not.
         for titles = (and t3 (let ((kept (gethash (cdr t3) stores 'unread)))
                                (if (not (eq kept 'unread)) kept
                                  (require 'romacs-agent-t3)
                                  (puthash (cdr t3) (romacs-agent-t3-thread-titles (cdr t3)) stores))))
         append
         (let ((files (sort (funcall (romacs-agent-history--get harness :files) dir romacs-agent-history-limit)
                            (lambda (a b) (> (nth 2 a) (nth 2 b))))))
           (cl-loop for (path size mtime) in (seq-take files romacs-agent-history-limit)
                    for description = (romacs-agent-history--describe path size mtime harness)
                    when (plist-get description :title)
                    collect (append (let ((title (and titles
                                                      (gethash (romacs-agent-t3-title-key (car t3) (plist-get description :id))
                                                               titles))))
                                      ;; T3's title as T3 shows it, cut to fit.
                                      (and title (not (string-empty-p (string-trim title)))
                                           (list :title (truncate-string-to-width (string-trim title) 60 nil nil t))))
                                    (list :path path :size size :mtime mtime
                                          :provider provider :config-dir config-dir)
                                    description)))))

;;; The listing, as rows

;; The scan reads a head of every new or changed transcript: seconds for
;; a few hundred.  So the listing is rows of the agents' sessions.sqlite,
;; `transcripts', written by a batch process of this build
;; (`romacs-agent-history-index-soon'); a listing reads the rows, and
;; starts the next scan aside.

(defconst romacs-agent-history--schema
  '("create table if not exists transcripts (path text primary key, harness text, provider text,
       config_dir text, id text, cwd text, title text, size integer, mtime real) without rowid")
  "The transcripts' table, made with the database.")

(defvar romacs-agent-history--made nil "The database the transcripts' table was made in.")

(defun romacs-agent-history--db ()
  "The database, its transcripts' table made; nil without SQLite."
  (when-let* ((db (and (fboundp 'romacs-agent-store--open) (romacs-agent-store--open))))
    (unless (or (eq romacs-agent-history--made db) (romacs-agent-store-readonly-p))
      (dolist (statement romacs-agent-history--schema) (sqlite-execute db statement))
      (setq romacs-agent-history--made db))
    db))

(add-hook 'romacs-agent-store-made-functions #'romacs-agent-history--db)

(defun romacs-agent-history-index ()
  "Scan the transcripts (`romacs-agent-history--scan') and make the rows
theirs: one a transcript listed, the others' removed, in one
transaction.  How many rows it holds."
  (let ((items (romacs-agent-history--scan)) (db (romacs-agent-history--db)))
    (when db
      (sqlite-execute db "begin immediate")
      (condition-case err
          (progn
            (sqlite-execute db "delete from transcripts")
            (dolist (item items)
              (sqlite-execute db "insert or replace into transcripts values (?, ?, ?, ?, ?, ?, ?, ?, ?)"
                              (list (plist-get item :path) (symbol-name (plist-get item :harness))
                                    (plist-get item :provider) (plist-get item :config-dir) (plist-get item :id)
                                    (plist-get item :cwd) (plist-get item :title)
                                    (plist-get item :size) (plist-get item :mtime))))
            (sqlite-execute db "commit"))
        (error (ignore-errors (sqlite-execute db "rollback")) (signal (car err) (cdr err))))
      (length items))))

(defvar romacs-agent-store-patience)
(defvar romacs-agent-store-patient)

(defun romacs-agent-history-index-batch ()
  "`romacs-agent-history-index' in a batch process, its count printed.
Off the editor, it waits long for the store's lock, and is done again
until the lock lets it: a listing left unwritten lists nothing."
  (require 'romacs-agent-store)
  (let ((romacs-agent-store-patience romacs-agent-store-patient) (count nil))
    (while (not count)
      (condition-case nil (setq count (or (romacs-agent-history-index) 0))
        (sqlite-locked-error nil)))
    (princ (format "romacs-agent-history: %d transcripts\n" count))))

(defvar romacs-agent-history--indexing nil "The batch process scanning the transcripts, while it runs.")

(defun romacs-agent-history-indexing-p ()
  "Whether a batch process is writing the transcripts' rows anew."
  (process-live-p romacs-agent-history--indexing))

(defvar romacs-agent-history-indexed-hook nil
  "Run once a batch process has written the transcripts' rows anew.")

(defun romacs-agent-history--settings ()
  "The form giving a batch process this editor's settings the scan reads:
its providers, T3's settings file and the listing's size."
  (format "(setq romacs-agent-providers '%S romacs-agent-history-limit %S%s)"
          romacs-agent-providers romacs-agent-history-limit
          ;; T3's, when set here: unset, the process keeps its own default.
          (if (boundp 'romacs-agent-t3-settings) (format " romacs-agent-t3-settings %S" romacs-agent-t3-settings) "")))

(defun romacs-agent-history-index-soon ()
  "Write the transcripts' rows anew in a batch process of this build, off
the editor, when none runs; `romacs-agent-history-indexed-hook' runs once
it ends."
  (when (and (not noninteractive) (not (process-live-p romacs-agent-history--indexing)) (romacs-agent-history--db))
    (setq romacs-agent-history--indexing
          (make-process
           :name "romacs-agent-history-index" :noquery t :connection-type 'pipe
           :buffer (generate-new-buffer " *romacs-agent-history-index*" t)
           :command (list (expand-file-name invocation-name invocation-directory) "--batch"
                          "-l" "romacs-agent-history"
                          "--eval" (romacs-agent--driver-batch-form)
                          "--eval" (romacs-agent-history--settings)
                          "-f" "romacs-agent-history-index-batch")
           :sentinel (lambda (process _event)
                       (unless (process-live-p process)
                         (let ((buffer (process-buffer process)))
                           ;; Failed: said, its last line; the rows as they were.
                           (unless (eql (process-exit-status process) 0)
                             (message "romacs-agent-history: listing the transcripts failed: %s"
                                      (if (buffer-live-p buffer)
                                          (with-current-buffer buffer
                                            (car (last (split-string (buffer-string) "\n" t))))
                                        (process-exit-status process))))
                           (when (buffer-live-p buffer) (kill-buffer buffer)))
                         (romacs-agent-history--from-rows)
                         (romacs-agent--run-hook 'romacs-agent-history-indexed-hook)))))))

(defun romacs-agent-history--from-rows ()
  "The listing, from the transcripts' rows: nil without them."
  (when-let* ((db (romacs-agent-history--db)))
    (setq romacs-agent-history--listing
          (mapcar (lambda (row)
                    (pcase-let ((`(,path ,harness ,provider ,config-dir ,id ,cwd ,title ,size ,mtime) row))
                      (list :title title :path path :size size :mtime mtime :provider provider
                            :config-dir config-dir :harness (intern harness) :id id :cwd cwd)))
                  (sqlite-select db (concat "select path, harness, provider, config_dir, id, cwd, title, size, mtime"
                                            " from transcripts where title is not null order by mtime desc"))))))

(defun romacs-agent-history-list (&optional wait)
  "List the harnesses' transcripts: from their rows, the next scan of
them started aside (`romacs-agent-history-index-soon'), which lists them
anew as it ends.  Without the rows' database, or in a batch process,
scanned here (`romacs-agent-history--scan').  WAIT, for a program that
reads the listing at once: list what is on disk now, waiting for a scan
started after this call, up to WAIT seconds (t: 30)."
  (if (and (not noninteractive) (romacs-agent-history--db))
      ;; One running already may have started before what is to be listed
      ;; was written: waited for, then one more.
      (let ((earlier (romacs-agent-history-indexing-p))
            (end (+ (float-time) (if (numberp wait) wait 30))))
        (romacs-agent-history--from-rows)
        (romacs-agent-history-index-soon)
        (when wait
          (dotimes (_ (if earlier 2 1))
            (romacs-agent-history-index-soon)
            (while (and (romacs-agent-history-indexing-p) (< (float-time) end))
              (accept-process-output romacs-agent-history--indexing 0.05)))
          ;; Ended, its sentinel may not have run yet: what it wrote is read
          ;; here, not left to it.
          (romacs-agent-history--from-rows)))
    (setq romacs-agent-history--listing (romacs-agent-history--scan))))

(defun romacs-agent-history-find (harness config-dir id)
  "HARNESS's conversation ID kept in CONFIG-DIR (nil: its default), as
an item of the listing (`romacs-agent-history-list'), the description
nil when its head names no prompt; nil when no transcript has it."
  (let ((path (and (funcall (or (plist-get (romacs-agent--driver-of harness) :native-id-p) #'stringp) id)
                   (funcall (romacs-agent-history--get harness :find) (romacs-agent--home harness config-dir) id))))
    (when path
      (let* ((attributes (file-attributes path))
             (size (file-attribute-size attributes))
             (mtime (float-time (file-attribute-modification-time attributes))))
        (append (list :path path :size size :mtime mtime :harness harness :id id :config-dir config-dir)
                (romacs-agent-history--describe path size mtime harness))))))

(defun romacs-agent-history-items ()
  "The listed transcripts no Romacs session continues, loaded or a row,
newest first."
  (let ((known (make-hash-table :test #'equal)))
    ;; A conversation by its identity: two configuration directories can
    ;; hold the same id.
    (maphash (lambda (_id session)
               (puthash (romacs-agent--native-key (romacs-agent-harness session) (romacs-agent-config-dir session)
                                                  (romacs-agent--native-id session))
                        t known)
               (when-let* ((imported (romacs-agent-imported session))) (puthash (plist-get imported :path) t known)))
             romacs-agent--sessions)
    ;; And a session kept as its row alone (quiet, or ended), by the
    ;; transcript its row names.
    (when-let* ((db (romacs-agent-history--db)))
      (dolist (row (ignore-errors (sqlite-select db "select transcript from sessions where transcript is not null")))
        (puthash (car row) t known)))
    (sort (cl-remove-if (lambda (item)
                          (or (gethash (romacs-agent--native-key (plist-get item :harness) (plist-get item :config-dir)
                                                                 (plist-get item :id))
                                       known)
                              (gethash (plist-get item :path) known)))
                        romacs-agent-history--listing)
          (lambda (a b) (> (plist-get a :mtime) (plist-get b :mtime))))))

;;; The view

(defconst romacs-agent-history--facts
  '((:model . romacs-agent-model) (:permission-mode . romacs-agent-permission-mode)
    (:context-used . romacs-agent-context-used) (:context-window . romacs-agent-context-window))
  "The header's facts a transcript tells, and the session's fields for them.")

(defun romacs-agent-history--keep-facts (session facts)
  "Give SESSION the transcript's last FACTS where it has none of its own:
an imported session's own turns know better."
  (pcase-dolist (`(,key . ,field) romacs-agent-history--facts)
    (when (and (plist-get facts key) (null (funcall field session)))
      (pcase key
        (:model (setf (romacs-agent-model session) (plist-get facts key)))
        (:permission-mode (setf (romacs-agent-permission-mode session) (plist-get facts key)))
        (:context-used (setf (romacs-agent-context-used session) (plist-get facts key)))
        (:context-window (setf (romacs-agent-context-window session) (plist-get facts key))))))
  (romacs-agent--changed session))

(defun romacs-agent-history-first-request (harness imported)
  "The first request of the transcript IMPORTED names, read from its head
by HARNESS's history, or nil."
  (let ((path (plist-get imported :path)))
    (when (file-readable-p path)
      (let ((end (min (or (plist-get imported :size) (file-attribute-size (file-attributes path)) 0) (* 512 1024))))
        (cl-find 'input (funcall (romacs-agent-history--get harness :events)
                                 (romacs-agent-transcript-read path 0 end (romacs-agent-history--get harness :keep)))
                 :key (lambda (event) (plist-get event :type)))))))

(defun romacs-agent-history-events (harness imported)
  "The events of the transcript IMPORTED names (:path, and :size, how much
of it was the harness's before Romacs went on with it), read by HARNESS's
history: its last `romacs-agent-history-view-bytes', from a prompt on.
Return (EVENTS . CUT), CUT whether earlier turns were left out; nil when
the file is gone."
  (let* ((path (plist-get imported :path))
         (end (or (plist-get imported :size) (file-attribute-size (file-attributes path)) 0))
         (beg (max 0 (- end romacs-agent-history-view-bytes))))
    (when (file-readable-p path)
      (let ((events (funcall (romacs-agent-history--get harness :events)
                             (romacs-agent-transcript-read path beg end (romacs-agent-history--get harness :keep)))))
        ;; Cut: from the first prompt on.
        (when (> beg 0)
          (while (and events (not (memq (plist-get (car events) :type) '(input arrival)))) (pop events)))
        ;; A result whose call the cut left out shows nothing.
        (let ((started (make-hash-table :test #'equal)))
          (setq events (seq-filter (lambda (event)
                                     (or (not (eq (plist-get event :type) 'tool))
                                         (if (eq (plist-get event :state) 'started) (puthash (plist-get event :id) t started)
                                           (gethash (plist-get event :id) started))))
                                   events)))
        (cons events (> beg 0))))))

(defun romacs-agent-history-draw (session imported)
  "Draw the transcript IMPORTED names (:path, and :size, how much of it
was the harness's before Romacs went on with it) into the current view,
for SESSION: its end, from a prompt on."
  (let ((facts nil))
    (dolist (event (romacs-agent-log-draw-imported session imported))
      (pcase-dolist (`(,key . ,_) romacs-agent-history--facts)
        (when (plist-get event key) (setq facts (plist-put facts key (plist-get event key))))))
    (romacs-agent-history--keep-facts session facts)))

(defvar-local romacs-agent-history--item nil "The transcript this view shows.")

(defvar-keymap romacs-agent-history-text-map
  :doc "Keys of a harness transcript's text: a chat's, but read-only."
  :parent romacs-agent-log-transcript-map
  "<remap> <self-insert-command>" #'undefined
  "?" #'describe-mode
  "g" #'undefined
  "e" #'undefined
  "u" #'undefined
  "D" #'undefined
  "q" #'quit-window
  "r" #'romacs-agent-history-resume-this)

(defvar-keymap romacs-agent-history-mode-map
  :doc "Keys of a harness transcript's view: its text's, wherever point is."
  :parent romacs-agent-history-text-map)

(define-derived-mode romacs-agent-history-mode special-mode "Agent history"
  "A harness's own transcript, read-only, drawn as a chat's transcript
\(romacs-agent-log.el) with no composer, so none of a chat's composing
commands applies: \\[romacs-agent-history-resume-this] goes on with it.
\\{romacs-agent-history-text-map}"
  :interactive nil
  (setq-local truncate-lines nil word-wrap t)
  (setq-local header-line-format "")
  ;; As a chat's: its bars never in what one copies.
  (add-function :filter-return (local 'filter-buffer-substring-function) #'romacs-agent-log--undecorated)
  (setq romacs-agent-log--text-map romacs-agent-history-text-map))

;;;###autoload
(defun romacs-agent-history-open (item &optional action)
  "Show the transcript ITEM (one of `romacs-agent-history-items') read-only,
as `romacs-agent-display' does with ACTION."
  (let ((buffer (cl-find-if (lambda (buffer) (equal (plist-get (buffer-local-value 'romacs-agent-history--item buffer) :path)
                                                    (plist-get item :path)))
                            (buffer-list))))
    (unless buffer
      (setq buffer (generate-new-buffer (format "*history: %s*" (plist-get item :title))))
      (with-current-buffer buffer
        (romacs-agent-history-mode)
        (setq romacs-agent-history--item item
              romacs-agent-log--session (romacs-agent--make
                                         :id (plist-get item :id) :harness (plist-get item :harness)
                                         :provider (plist-get item :provider) :config-dir (plist-get item :config-dir)
                                         :cwd (plist-get item :cwd) :title (plist-get item :title) :state 'history
                                         :tools (make-hash-table :test #'equal) :streamed (make-hash-table :test #'equal))
              romacs-agent-ui--session romacs-agent-log--session
              ;; No composer: the transcript is the whole buffer.
              romacs-agent-log--end (point-max-marker))
        (set-marker-insertion-type romacs-agent-log--end t))
      ;; Shown first: its rules fit its window.
      (romacs-agent-display buffer action)
      (with-current-buffer buffer
        (romacs-agent-history-draw romacs-agent-log--session (list :path (plist-get item :path)))
        (romacs-agent-log--header-show)))
    (romacs-agent-display buffer action)
    ;; On its last entry, its end in view: nothing here is typed at.
    (goto-char (point-max))
    (romacs-agent-log-previous 1)
    (recenter -1)
    buffer))

(defun romacs-agent-history-resume (item &optional action)
  "Go on with the transcript ITEM as a Romacs session, through its
harness's own resume, and show its chat, as `pop-to-buffer' does with
ACTION: the session already going on with it, when there is one,
attached or resumed."
  (let* ((id (plist-get item :id))
         (known (romacs-agent--native-session (plist-get item :harness) (plist-get item :config-dir) id))
         (session (if known (romacs-agent--continue known)
                    (romacs-agent-import (plist-get item :harness) id (plist-get item :path)
                                         (plist-get item :cwd) (plist-get item :title)
                                         (plist-get item :provider) (plist-get item :config-dir)))))
    (romacs-agent-ui--batch)
    (romacs-agent-show session action)))

(defun romacs-agent-history-resume-this ()
  "Go on with this transcript as a Romacs session."
  (interactive nil romacs-agent-history-mode)
  (let ((item romacs-agent-history--item) (view (current-buffer)))
    (romacs-agent-history-resume item)
    (kill-buffer view)))

(provide 'romacs-agent-history)
;;; romacs-agent-history.el ends here
