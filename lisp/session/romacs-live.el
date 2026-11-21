;;; romacs-live.el --- Load changed Lisp into the running session  -*- lexical-binding: t -*-
;; SPDX-License-Identifier: GPL-3.0-or-later
;; Maintainer: Thanos Apollo <public@thanosapollo.org>

;;; Commentary:
;; `romacs-live-update' makes the running session's Lisp equal to a
;; source's: landed beta, a branch or commit, a worktree, or a set of
;; files.  The session is its build (its runtime root's build.json) plus
;; the files of `romacs-live-ledger', each at a git blob.  What it has
;; loaded of a file is the ledger's blob, else the build's own copy, which
;; an install never changes; so only the files whose content differs from
;; what the session has are loaded, all of them, in the order their
;; `require's give.  A session without a build.json takes named files only.
;;
;; An update is a survey, a plan, then an apply.  The survey (git, the
;; files' two texts) is data and runs in a worker VM in an interactive
;; session.  The plan reads both texts as definitions
;; (romacs-live-forms.el) and decides, before anything loads, what each
;; changed definition needs; it refuses the whole update for a struct
;; whose layout changed, a runtime epoch or a `lisp/vm/' file.  The apply
;; loads each file by its library name from the content store
;; (`romacs-live-directory', one directory a blob), so the compiled-Lisp
;; store keys it the same in every process and compiles it off the main
;; VM, and handles what a plain `load' leaves wrong:
;; - a keymap the file defines keeps its object, which takes the new
;;   map's contents, so every buffer, derived map and text property that
;;   holds it has the new keys; the keys the user bound stay;
;; - an option the user left alone takes its new default;
;; - a `defvar' keeps its value, but for a literal nobody changed;
;; - a face nobody customized takes its new spec;
;; - advice and the user's hook functions stay, as `load' keeps them.
;; A file the session has not loaded is staged: its directory goes first
;; on `load-path', so a later `require' gets it.  The report says what
;; changed, file by file and definition by definition.

;;; Code:

(require 'cl-lib)
(require 'subr-x)
(require 'romacs-live-forms)

(declare-function isolate-spawn "isolate" (function &rest args))
(declare-function isolate-on-done "isolate" (future callback))
(declare-function isolate-await "isolate" (future &optional timeout))
(declare-function isolate-main-p "isolate" ())
(declare-function romacs-mcp-defer "romacs-mcp" (poll &optional deadline))
(declare-function romacs-mcp--instance "romacs-mcp" (arguments))
(declare-function cl-struct-slot-info "cl-macs" (struct-type))

(defgroup romacs-live nil
  "Loading changed Lisp into the running session."
  :group 'lisp
  :prefix "romacs-live-")

(defcustom romacs-live-repository nil
  "The git repository `romacs-live-update' takes a branch or commit from.
nil means the repository of the current buffer's directory, when it is a
Romacs source tree."
  :type '(choice (const nil) directory))

(defcustom romacs-live-directory nil
  "The content store of live-loaded files; nil means romacs/live-lisp/
under the XDG state directory."
  :type '(choice (const nil) directory))

(defface romacs-live-patched '((t :inherit (bold font-lock-constant-face)))
  "The mark of a session running live-loaded files: an accent, not a warning.")
(defface romacs-live-heading '((t :inherit bold))
  "A heading in the live update's buffer.")
(defface romacs-live-path '((t :inherit default))
  "A file in the live update's buffer: plain, since a theme's string or
constant colour may be its error colour.")
(defface romacs-live-partial '((t :inherit (bold warning)))
  "A file loaded in part, and a refusal: what wants a look, marked ◐ or ⊘
beside it so it reads apart from an error where a theme's warning colour
is its error colour.")
(defface romacs-live-build '((t :inherit default))
  "The build's commit in the live update's buffer.")
(defface romacs-live-dim '((t :inherit shadow))
  "Detail in the live update's buffer: blobs, sources, times, notes.")
(defface romacs-live-ok '((t :inherit success))
  "What went as it should.")
(defface romacs-live-warning '((t :inherit warning))
  "What the update left for you to look at.")
(defface romacs-live-error '((t :inherit (bold error)))
  "What failed or was refused.")

(defvar romacs-live-ledger nil
  "What this session runs beyond its build: a plist, `:build' (the build's
`:commit', `:tree' and `:root') and `:entries', one plist a file: `:path'
\(lisp/... in a Romacs tree, else absolute), `:file' (its copy in the
content store), `:base' and `:new' (git blobs: what the session had, what
it has), `:source', `:time' and `:state' (`loaded', `staged' or
`partial').  Nil when the session is its build.")

(defvar romacs-live-last-report nil
  "The last update's report, a plist (`romacs-live-update').")

(defvar romacs-live-mode-line nil
  "The mode line's mark of the ledger, or nil when it is empty.")
(put 'romacs-live-mode-line 'risky-local-variable t)

(defvar romacs-live-keymap-defaults-function #'romacs-live--worker-keymap-defaults
  "Function given (FILES LIBRARY VARIABLES CALLBACK): call CALLBACK with an
alist of each keymap variable in VARIABLES to its bindings as LIBRARY alone
makes them, flattened (`romacs-live-forms-keymap-flat'), after loading
FILES; or with nil when that cannot be known.  nil: never known.")

(defvar romacs-live--running nil "The update in progress, or nil.")

(defvar romacs-live-last-source nil
  "The source of the last update from one (`romacs-live-survey''s
`:source'): where `romacs-live-test' finds the suites.")



;;;; The build and what the session loaded

(defun romacs-live--runtime-root ()
  "This session's runtime root: the directory holding its build.json, else
`ROMACS_RUNTIME_ROOT', else nil."
  (let ((candidates (delq nil (list (getenv "ROMACS_RUNTIME_ROOT") invocation-directory
                                    (and invocation-directory (expand-file-name ".." invocation-directory))))))
    (or (cl-find-if (lambda (dir) (file-exists-p (expand-file-name "build.json" dir))) candidates)
        (getenv "ROMACS_RUNTIME_ROOT"))))

(defun romacs-live-build ()
  "This session's build: a plist of `:commit', `:tree' and `:root', the
commit nil when the runtime root has no build.json."
  (let* ((root (romacs-live--runtime-root))
         (file (and root (expand-file-name "build.json" root)))
         (record (and file (file-readable-p file)
                      (ignore-errors (with-temp-buffer (insert-file-contents file)
                                                       (json-parse-buffer :object-type 'plist :null-object nil))))))
    (list :commit (plist-get record :commit) :tree (plist-get record :tree) :root root)))

(defun romacs-live--lisp-roots (root)
  "The directory names under which ROOT's Lisp tree is loaded from."
  (delete-dups
   (cl-loop for dir in (delq nil (list (and root (expand-file-name "lisp" root))
                                       (and root (expand-file-name "share/romacs/lisp" root))
                                       (and (bound-and-true-p lisp-directory) lisp-directory)))
            when (file-directory-p dir)
            append (list (file-name-as-directory (expand-file-name dir))
                         (file-name-as-directory (file-truename dir))))))

(defun romacs-live--relative (file roots)
  "FILE as lisp/... when it is under one of ROOTS, else nil."
  (cl-loop for root in roots
           when (string-prefix-p root file) return (concat "lisp/" (substring file (length root)))))

(defun romacs-live--entry (path)
  "The ledger's entry for PATH, or nil."
  (cl-find path (plist-get romacs-live-ledger :entries) :key (lambda (e) (plist-get e :path)) :test #'equal))

(defun romacs-live--loaded (roots)
  "A table of the files the session loaded, each path (lisp/... or
absolute) to the name load-history has it by: the build's copy, a store
copy of the ledger's, or a file by its own name."
  (let ((table (make-hash-table :test #'equal))
        (stored (make-hash-table :test #'equal)))
    (dolist (entry (plist-get romacs-live-ledger :entries))
      (puthash (file-truename (plist-get entry :file)) (plist-get entry :path) stored))
    (dolist (entry (reverse load-history))
      (when-let* ((file (and (stringp (car entry)) (car entry))))
        (puthash (or (gethash file stored) (gethash (ignore-errors (file-truename file)) stored)
                     (romacs-live--relative file roots) file)
                 file table)))
    table))

;;;; The content store

(defun romacs-live--store ()
  (file-name-as-directory
   (expand-file-name (or romacs-live-directory
                         (expand-file-name "romacs/live-lisp/" (or (getenv "XDG_STATE_HOME") "~/.local/state"))))))

(defun romacs-live--store-put (blob name bytes)
  "Keep BYTES, content BLOB, in the store once, as NAME (its file name
relative to its `load-path' directory); return its file."
  (let ((file (expand-file-name name (expand-file-name blob (romacs-live--store)))))
    (unless (file-exists-p file)
      (with-file-modes #o700 (make-directory (file-name-directory file) t))
      (let ((temporary (make-temp-file (expand-file-name ".put-" (file-name-directory file))))
            (coding-system-for-write 'no-conversion))
        (write-region bytes nil temporary nil 'silent)
        (rename-file temporary file t)))
    file))

;;;; The survey: data, run in a worker

(defun romacs-live--git (directory &rest args)
  "Git ARGS in DIRECTORY: its output as bytes; signal when it fails."
  (with-temp-buffer
    (set-buffer-multibyte nil)
    (let* ((coding-system-for-read 'no-conversion)
           (default-directory (file-name-as-directory directory))
           (status (apply #'call-process "git" nil (list t nil) nil args)))
      (unless (eql status 0)
        (error "git %s failed in %s" (string-join args " ") directory))
      (buffer-string))))

(defun romacs-live--z (bytes)
  "BYTES, a NUL-separated git listing, as decoded strings."
  (mapcar (lambda (s) (decode-coding-string s 'utf-8)) (split-string bytes "\0" t)))

(defun romacs-live--blob (bytes)
  "The git blob id of BYTES, as `git hash-object' names them."
  (secure-hash 'sha1 (concat (string-to-unibyte (format "blob %d\0" (length bytes))) bytes)))

(defun romacs-live--read-file (file)
  "FILE's (BLOB . BYTES), read once, or nil when it is no regular file."
  (and (file-regular-p file)
       (let ((bytes (romacs-live--bytes file))) (cons (romacs-live--blob bytes) bytes))))

(defun romacs-live--bytes (file)
  (with-temp-buffer
    (set-buffer-multibyte nil)
    (let ((coding-system-for-read 'no-conversion)) (insert-file-contents-literally file))
    (buffer-string)))

(defun romacs-live--tree-of (file)
  "The Romacs tree FILE is in: the directory holding
etc/romacs-runtime-epoch whose lisp/ has FILE, or nil."
  (when-let* ((dir (locate-dominating-file file "etc/romacs-runtime-epoch")))
    (and (string-prefix-p (expand-file-name "lisp/" dir) (expand-file-name file)) (expand-file-name dir))))

(defun romacs-live--short (rev) (substring rev 0 (min 9 (length rev))))

(defun romacs-live--candidates (source commit)
  "What SOURCE has that may differ from the build at COMMIT: (LABEL NAMES
. READ), NAMES the paths that differ and READ a function of a path giving
\(BLOB . BYTES), nil for a file the source does not have."
  (pcase (plist-get source :kind)
    ('rev
     (let* ((repo (plist-get source :repository))
            (sha (string-trim (romacs-live--git repo "rev-parse" "--verify" (concat (plist-get source :rev) "^{commit}")))))
       (list (format "%s@%s" (plist-get source :rev) (romacs-live--short sha))
             (romacs-live--z (romacs-live--git repo "diff" "--name-only" "-z" commit sha))
             (lambda (path)
               (let ((line (romacs-live--git repo "ls-tree" sha "--" path)))
                 (when (string-match "\\`[0-9]+ blob \\([0-9a-f]+\\)\t" line)
                   (let ((blob (match-string 1 line)))
                     (cons blob (romacs-live--git repo "cat-file" "blob" blob)))))))))
    ('worktree
     (let* ((tree (file-name-as-directory (expand-file-name (plist-get source :directory))))
            (head (string-trim (romacs-live--git tree "rev-parse" "HEAD")))
            (dirty (not (string-empty-p (romacs-live--git tree "status" "--porcelain" "--" "lisp")))))
       (list (format "%s@%s%s" (file-name-nondirectory (directory-file-name tree)) (romacs-live--short head)
                     (if dirty "+dirty" ""))
             (delete-dups (append (romacs-live--z (romacs-live--git tree "diff" "--name-only" "-z" commit))
                                  (romacs-live--z (romacs-live--git tree "ls-files" "-z" "--others" "--exclude-standard" "--" "lisp"))))
             (lambda (path) (romacs-live--read-file (expand-file-name path tree))))))))

(defun romacs-live--files-candidates (files)
  "Explicit FILES as candidates: (LABEL NAMES . READ), each path lisp/...
when the file is in a Romacs tree."
  (let ((paths (mapcar (lambda (file)
                         (let* ((file (expand-file-name file)) (tree (romacs-live--tree-of file)))
                           (cons (if tree (file-relative-name file tree) file) file)))
                       files)))
    (list (if (cdr files) (format "%d files" (length files)) (file-name-nondirectory (car files)))
          (mapcar #'car paths)
          (lambda (path) (romacs-live--read-file (cdr (assoc path paths)))))))

(defun romacs-live--text (bytes)
  (and bytes (decode-coding-string bytes 'utf-8)))

(defun romacs-live-survey (request)
  "What REQUEST's source has that this session lacks, as data.
REQUEST is a plist: `:source' (`:kind' `rev' with `:repository' and
`:rev', `worktree' with `:directory', `files' with `:files'), `:build'
\(`romacs-live-build'), `:ledger' (an alist of path to (BLOB FILE PARTIAL)) and
`:loaded' (the paths the session loaded).  The value is a plist:
`:label', `:files' (one plist a path that differs: `:path', `:base',
`:new' and `:build-blob' blobs, `:bytes', `:old' text, `:text', `:loaded'),
`:refusals' and `:warnings' (strings)."
  (let* ((source (plist-get request :source))
         (build (plist-get request :build))
         (commit (plist-get build :commit))
         (roots (romacs-live--lisp-roots (plist-get build :root)))
         (ledger (plist-get request :ledger))
         (refusals nil) (warnings nil))
    (when (and (not commit) (not (eq (plist-get source :kind) 'files)))
      (push (format "This session's build is unknown: no build.json in %s; name the files instead"
                    (or (plist-get build :root) "its runtime root"))
            refusals))
    (if refusals
        (list :label nil :files nil :refusals refusals :warnings nil)
      (pcase-let* ((`(,label ,names ,read) (if (eq (plist-get source :kind) 'files)
                                                 (romacs-live--files-candidates (plist-get source :files))
                                               (romacs-live--candidates source commit)))
                   (lisp nil) (rust 0))
        (dolist (name names)
          (cond ((member name '("etc/romacs-runtime-epoch"))
                 (push "The runtime epoch differs from the build's: it needs an install" refusals))
                ((or (string-prefix-p "lisp/vm/" name) (equal name "lisp/bundled.json"))
                 (push (format "%s is read by the VM itself: it needs an install" name) refusals))
                ((and (string-suffix-p ".el" name)
                      (or (string-prefix-p "lisp/" name) (eq (plist-get source :kind) 'files)))
                 (push name lisp))
                ((string-prefix-p "crates/" name) (cl-incf rust))))
        (when (> rust 0)
          (push (format "%d Rust files differ from build %s: Lisp that needs them waits for an install"
                        rust (romacs-live--short commit))
                warnings))
        ;; A file the ledger holds is one too: the source may have it back
        ;; as the build had it.
        (unless (eq (plist-get source :kind) 'files)
          (dolist (entry ledger)
            (when (and (string-prefix-p "lisp/" (car entry)) (not (member (car entry) lisp)))
              (push (car entry) lisp))))
        (let ((files
               (cl-loop
                for path in (sort lisp #'string<)
                ;; The build's copy is what the session loaded only when the
                ;; build is known: a session running a worktree's own tree
                ;; may have loaded what is since edited in place.
                for installed = (and commit (string-prefix-p "lisp/" path) roots
                                     (romacs-live--read-file (expand-file-name (substring path 5) (car roots))))
                for build-blob = (car installed)
                for stored = (cdr (assoc path ledger))
                for base = (or (car stored) build-blob)
                for got = (funcall read path)
                ;; A partial entry loads again, whatever it is compared with.
                unless (and base (not (nth 2 stored)) (equal (car got) base))
                collect (list :path path :base base :new (car got) :build-blob build-blob
                              :bytes (cdr got) :text (romacs-live--text (cdr got))
                              :old (let ((file (nth 1 stored)))
                                     (romacs-live--text (if (and file (file-regular-p file)) (romacs-live--bytes file)
                                                          (cdr installed))))
                              :loaded (and (member path (plist-get request :loaded)) t)))))
          (list :label label :files files :refusals (nreverse refusals) :warnings warnings))))))

;;;; The plan: decisions, before anything loads

(defun romacs-live--layout-now (name)
  "The slot names struct NAME has in this session, or nil."
  (and (get name 'cl--class) (fboundp 'cl-struct-slot-info)
       (ignore-errors (mapcar #'car (cdr (cl-struct-slot-info name))))))

(defun romacs-live--layout-refusals (path old new)
  "Why loading NEW over OLD (forms of PATH) would break records already
made: a struct or class whose layout changed."
  (let ((old-defs (car (romacs-live-forms-definitions old))))
    (cl-loop for (key . form) in (car (romacs-live-forms-definitions new))
             when (memq (car key) '(struct class))
             append (let* ((before (cdr (assoc key old-defs)))
                           (now (romacs-live-forms-layout form))
                           ;; With no old text, the slots the session's struct has.
                           (was (if before (romacs-live-forms-layout before)
                                  (and (eq (car key) 'struct) (romacs-live--layout-now (cadr key))
                                       (list (car now) (romacs-live--layout-now (cadr key)) (nth 2 now)))))
                           (names (lambda (slots) (mapcar (lambda (s) (if (consp s) (car s) s)) slots))))
                      (when (and was (not (equal was now)))
                        (list (format "%s: %s %s changes its layout (%s); live updates of a layout wait for live-structs, so restart onto a build"
                                      path (car key) (cadr key)
                                      (string-join
                                       (or (append (mapcar (lambda (s) (format "+%s" s))
                                                           (cl-set-difference (funcall names (cadr now)) (funcall names (cadr was))))
                                                   (mapcar (lambda (s) (format "-%s" s))
                                                           (cl-set-difference (funcall names (cadr was)) (funcall names (cadr now))))
                                                   (and (not (equal (car was) (car now))) (list "parent"))
                                                   (and (not (equal (nth 2 was) (nth 2 now))) (list "representation")))
                                           (list "slot order or allocation"))
                                       " "))))))))

(defun romacs-live--order (records)
  "RECORDS in the order their files' `require's give, ties by path."
  (let* ((provider (make-hash-table :test #'eq))
         (done nil) (visiting nil) (out nil))
    (dolist (r records) (dolist (f (plist-get r :provides)) (puthash f r provider)))
    (cl-labels ((visit (r)
                  (unless (or (memq r done) (memq r visiting))
                    (push r visiting)
                    (dolist (f (plist-get r :requires))
                      (when-let* ((dep (gethash f provider))) (visit dep)))
                    (push r done)
                    (push r out))))
      (mapc #'visit records))
    (nreverse out)))

(defun romacs-live--history-entry (file)
  "FILE's load-history entry, by the name it was loaded by."
  (and file (assoc file load-history)))

(defun romacs-live--dependents (records loaded)
  "Records for the loaded files that require a feature RECORDS change a
macro or struct of, not among RECORDS: they load again after them."
  (let* ((features (cl-loop for r in records when (romacs-live-forms-macro-change-p (plist-get r :diff))
                            append (plist-get r :provides)))
         (paths (mapcar (lambda (r) (plist-get r :path)) records))
         (out nil))
    (when features
      (maphash (lambda (path file)
                 (unless (or (member path paths) (cl-find path out :key (lambda (r) (plist-get r :path)) :test #'equal))
                   (when (cl-some (lambda (item) (and (eq (car-safe item) 'require) (memq (cdr item) features)))
                                  (cdr (romacs-live--history-entry file)))
                     (when-let* ((bytes (and (file-regular-p file) (romacs-live--bytes file)))
                                 (forms (ignore-errors (romacs-live-forms-read (romacs-live--text bytes) path))))
                       (push (list :path path :action 'dependent :bytes bytes :file file :exact file :new (plist-get (romacs-live--entry path) :new)
                                   :forms forms :old-forms forms :diff (romacs-live-forms-diff forms forms)
                                   :requires (romacs-live-forms-requires forms) :provides (romacs-live-forms-provides forms))
                             out)))))
               loaded))
    out))

(defun romacs-live-plan (survey loaded)
  "The plan of SURVEY (`romacs-live-survey') for a session that LOADED
\(`romacs-live--loaded'): a plist `:label', `:records' in load order,
`:refusals' and `:warnings'.  A record is the survey's file with
`:action' (`load', `stage', `revert', `dependent' or `deleted'), its
`:forms', `:old-forms', `:diff', `:requires' and `:provides'."
  (let* ((refusals (copy-sequence (plist-get survey :refusals)))
         (warnings (copy-sequence (plist-get survey :warnings)))
         (records nil)
         (parsed (cl-loop for file in (plist-get survey :files)
                          collect (condition-case err
                                      (list file (and (plist-get file :text)
                                                      (romacs-live-forms-read (plist-get file :text) (plist-get file :path)))
                                            (and (plist-get file :old)
                                                 (ignore-errors (romacs-live-forms-read (plist-get file :old) (plist-get file :path)))))
                                    (romacs-live-unreadable
                                     (push (format "%s does not read at %s: %s" (nth 1 err) (nth 2 err) (nth 3 err)) refusals)
                                     nil))))
         ;; What the source declares holds as it is read, loaded or not.
         (romacs-live-forms--declared (cl-loop for (_ new) in parsed append (romacs-live-forms-declarations new))))
    (pcase-dolist (`(,file ,new ,old) (delq nil parsed))
          (let* ((path (plist-get file :path))
                 (action (cond ((null (plist-get file :new))
                                ;; Gone from the source: a staged copy goes too.
                                (if (and (romacs-live--entry path) (not (plist-get file :loaded))) 'revert 'deleted))
                               ((and (equal (plist-get file :new) (plist-get file :build-blob)) (romacs-live--entry path)) 'revert)
                               ((plist-get file :loaded) 'load)
                               (t 'stage))))
            (when (memq action '(load revert))
              (setq refusals (append refusals (romacs-live--layout-refusals path old new))))
            (when (and (memq action '(load stage)) (romacs-live-forms-beside-itself-p new))
              (push (format "%s reads files beside it in the installation: restart onto a build to load it" path)
                    refusals))
            (push (append (list :action action :forms new :old-forms old
                                :diff (romacs-live-forms-diff old new)
                                :requires (romacs-live-forms-requires new) :provides (romacs-live-forms-provides new))
                          file)
                  records)))
    (setq records (nreverse records))
    ;; The files that use a changed macro join the changed ones in one
    ;; order, so a changed file that requires them loads after them.
    (let ((dependents (romacs-live--dependents (cl-remove-if-not (lambda (r) (memq (plist-get r :action) '(load revert))) records)
                                               loaded)))
      (list :label (plist-get survey :label) :declared romacs-live-forms--declared :dropped (plist-get survey :dropped)
            :records (romacs-live--order (append records dependents))
            :refusals refusals
            :warnings (append warnings (romacs-live--relocation-warnings records))))))

(defun romacs-live--relocation-warnings (records)
  "Warnings for RECORDS' files that find files beside themselves: they load
from their store copy, where those files are not."
  (cl-loop for r in records
           when (and (memq (plist-get r :action) '(load stage)) (plist-get r :text)
                     (string-match-p "\\_<load-\\(?:true-\\)?file-name\\_>" (plist-get r :text)))
           collect (format "%s reads load-file-name: it runs from its store copy, so files it finds beside itself are looked up there"
                           (plist-get r :path))))

;;;; Keymaps, in place

(defun romacs-live--keymap-variables (entry)
  "The variables a load-history ENTRY defines whose value is a keymap list."
  (delete-dups
   (cl-loop for item in (cdr entry)
            when (and (symbolp item) (boundp item) (keymapp (default-value item)) (consp (default-value item)))
            collect item)))

(defun romacs-live--substitute-in (map pairs seen)
  "Make every NEW of PAIRS ((NEW . OLD) ...) held in MAP's cells OLD."
  (unless (memq map seen)
    (push map seen)
    (let ((tail map))
      (while (consp tail)
        (let ((item (car tail)))
          (cond ((char-table-p item)
                 (map-char-table (lambda (key value)
                                   (if-let* ((old (cdr (assq value pairs)))) (set-char-table-range item key old)
                                     (when (and (consp value) (keymapp value)) (setq seen (romacs-live--substitute-in value pairs seen)))))
                                 item))
                ((consp item)
                 (let ((binding (cdr item)))
                   (cond ((assq binding pairs) (setcdr item (cdr (assq binding pairs))))
                         ((and (eq (car-safe binding) 'menu-item) (assq (nth 2 binding) pairs))
                          (setcar (nthcdr 2 binding) (cdr (assq (nth 2 binding) pairs))))
                         ((and (consp binding) (keymapp binding))
                          (setq seen (romacs-live--substitute-in binding pairs seen))))))))
        (when (assq (cdr-safe tail) pairs) (setcdr tail (cdr (assq (cdr tail) pairs))))
        (setq tail (cdr-safe tail)))))
  seen)

(defun romacs-live--substitute (pairs)
  "Put back each OLD keymap of PAIRS ((NEW . OLD) ...) wherever the load
left its NEW: inside the swapped maps (their parents and prefix maps), the
minor modes' map lists and the function cells of the file's prefix commands."
  (let ((seen nil))
    (dolist (pair pairs) (setq seen (romacs-live--substitute-in (cdr pair) pairs seen))))
  (dolist (alist (append (list minor-mode-map-alist minor-mode-overriding-map-alist)
                         (mapcar (lambda (e) (if (symbolp e) (and (boundp e) (symbol-value e)) e))
                                 emulation-mode-map-alists)))
    (dolist (cell alist)
      (when-let* ((old (and (consp cell) (cdr (assq (cdr cell) pairs))))) (setcdr cell old)))))

(defun romacs-live--before-keymaps (entry defaults)
  "Unbind the keymap variables ENTRY's file defines, so its load makes them
anew; their old objects, with the user's bindings (from DEFAULTS, an alist
of variable to its library's bindings, or :unknown)."
  (let ((saved (cl-loop for var in (romacs-live--keymap-variables entry)
                        collect (let* ((old (default-value var))
                                       (default (and (listp defaults) (assq var defaults))))
                                  (list var old (and default (romacs-live-forms-user-bindings
                                                              (romacs-live-forms-keymap-flat old t) (cdr default)))
                                        (and default t))))))
    (dolist (item saved) (makunbound (car item)))
    saved))

(defun romacs-live--after-keymaps (saved entry)
  "Give each old keymap in SAVED the contents its variable now has, set the
variable back to it and bind the user's keys again; the report rows."
  (let ((pairs nil) (rows nil))
    (pcase-dolist (`(,var ,old ,user ,known) saved)
      (let ((new (and (boundp var) (default-value var))))
        (cond ((not (and (consp new) (keymapp new)))
               (set-default var old)
               (push (list :variable var :kept "the file no longer makes it") rows))
              (t
               ;; A map the file gives no parent keeps the one it had: a
               ;; derived mode's map gets its parent when the mode runs.
               (let ((parent (keymap-parent old)))
                 (setcdr old (cdr new))
                 (when (and parent (not (keymap-parent old))) (set-keymap-parent old parent)))
               (set-default var old)
               (push (cons new old) pairs)
               ;; The user's keys, bound again; one whose prefix the map
               ;; now binds to a command is said instead.
               (let ((bound nil) (conflicts nil))
                 (pcase-dolist (`(,key . ,binding) user)
                   (if (condition-case nil (progn (define-key old key binding) t) (error nil))
                       (push key bound)
                     (push key conflicts)))
                 (push (list :variable var :replaced t :unchecked (not known)
                             :user (nreverse bound) :conflicts (nreverse conflicts))
                       rows))))))
    (romacs-live--substitute pairs)
    (dolist (item (cdr entry))
      (when-let* ((function (and (eq (car-safe item) 'defun) (cdr item)))
                  (old (and (fboundp function) (cdr (assq (symbol-function function) pairs)))))
        (fset function old)))
    (nreverse rows)))

(defun romacs-live--worker-keymap-defaults (files library variables callback)
  "`romacs-live-keymap-defaults-function' in a worker VM: the world alone,
FILES loaded, then LIBRARY."
  (if (not (and (not noninteractive) (fboundp 'isolate-spawn) (isolate-main-p)))
      (funcall callback nil)
    (condition-case nil
        (isolate-on-done
         (isolate-spawn '(lambda (files library variables)
                           (dolist (file files) (load file nil t))
                           (load library nil t)
                           (mapcar (lambda (var) (cons var (and (boundp var) (keymapp (symbol-value var))
                                                                (romacs-live-forms-keymap-flat (symbol-value var)))))
                                   variables))
                        files library variables :require '(romacs-live-forms) :fresh t)
         (lambda (future) (funcall callback (condition-case nil (isolate-await future 0) (error nil)))))
      (error (funcall callback nil)))))

;;;; Options, variables, faces

(defun romacs-live--untouched-option-p (var)
  "Whether option VAR has the value its standard gives and nobody set it."
  (and (not (or (get var 'saved-value) (get var 'customized-value) (get var 'theme-value)))
       (default-boundp var)
       (condition-case nil (equal (default-value var) (eval (car (get var 'standard-value)) t)) (error nil))))

(defun romacs-live--before-values (entry record)
  "What ENTRY's file holds now that its load could leave wrong: options
with their standard, faces with their spec, and literal `defvar's whose
literal RECORD's diff changes."
  (let* ((old-defs (car (romacs-live-forms-definitions (plist-get record :old-forms))))
         (new-defs (car (romacs-live-forms-definitions (plist-get record :forms)))))
    (list :options (cl-loop for item in (cdr entry)
                            when (and (symbolp item) (get item 'standard-value))
                            collect (list item (get item 'standard-value) (romacs-live--untouched-option-p item)))
          ;; `defface' names a face in load-history only when it first
          ;; defines it: the old text names it on every later update.
          :faces (cl-loop for face in (delete-dups
                                       (append (cl-loop for item in (cdr entry)
                                                        when (eq (car-safe item) 'defface) collect (cdr item))
                                               (cl-loop for (key) in old-defs
                                                        when (and (eq (car key) 'face) (facep (cadr key))) collect (cadr key))))
                          collect (progn
                                    (list face (get face 'face-defface-spec)
                                          (not (or (get face 'face-modified) (get face 'saved-face)
                                                   (get face 'customized-face) (get face 'theme-face))))))
          :literals (cl-loop for key in (plist-get (plist-get record :diff) :changed)
                             for var = (cadr key)
                             for old = (romacs-live-forms-initial (cdr (assoc key old-defs)))
                             for new = (romacs-live-forms-initial (cdr (assoc key new-defs)))
                             when (and (eq (car key) 'variable) old new
                                       (romacs-live-forms-literal-p (car old)) (romacs-live-forms-literal-p (car new))
                                       (not (get var 'romacs-live-keep)) (default-boundp var)
                                       (not (and (consp (default-value var)) (keymapp (default-value var)))))
                             collect (list var (equal (default-value var) (romacs-live-forms-literal-value (car old)))
                                           (romacs-live-forms-literal-value (car new))))
          :specs (cl-loop for (key . form) in new-defs when (eq (car key) 'face)
                          collect (cons (cadr key) (romacs-live-forms-face-spec form))))))

(defun romacs-live--after-values (saved)
  "Give what SAVED holds its new defaults where nobody changed it; the
report rows."
  (append
   (cl-loop for (var old untouched) in (plist-get saved :options)
            for new = (get var 'standard-value)
            unless (equal old new)
            collect (if untouched
                        (condition-case err
                            (progn (funcall (or (get var 'custom-set) #'set-default) var (eval (car new) t))
                                   (list :option var :defaulted t))
                          (error (list :option var :error (error-message-string err))))
                      (list :option var :kept "you set it")))
   (cl-loop for (face old untouched) in (plist-get saved :faces)
            for form = (assq face (plist-get saved :specs))
            ;; A nil spec is a spec: only a missing or failing one is none.
            for new = (and form (condition-case nil (list (eval (cdr form) t)) (error nil)))
            when (and new (not (equal old (car new))))
            collect (if untouched
                        (progn (face-spec-set face (car new) 'face-defface-spec) (list :face face :respecified t))
                      (list :face face :kept "you set it")))
   (cl-loop for (var untouched new) in (plist-get saved :literals)
            collect (if untouched
                        (progn (set-default var new) (list :variable var :defaulted t))
                      (list :variable var :kept "its value changed since it was defined")))))

;;;; Apply

(defun romacs-live--load-name (path roots)
  "PATH's file name relative to the deepest `load-path' directory holding
its build copy: the name `load' and `require' find it by, a qualified one
\(semantic/wisent/grammar.el) for a file below a directory on the path.
Its base name when no directory holds it."
  (let ((places (if (string-prefix-p "lisp/" path)
                    (mapcar (lambda (root) (expand-file-name (substring path 5) root)) roots)
                  (list (expand-file-name path))))
        (best nil))
    (dolist (dir load-path)
      (when (stringp dir)
        (let ((dir (file-name-as-directory (expand-file-name dir))))
          (dolist (place places)
            (when (and (string-prefix-p dir place) (or (null best) (< (- (length place) (length dir)) (length best))))
              (setq best (substring place (length dir))))))))
    (or best (file-name-nondirectory path))))

(defun romacs-live--forget-history (keep path roots previous)
  "Drop the load-history entries of PATH's other copies than KEEP, so one
entry says what the session has of it: PREVIOUS, the name it was loaded
by before, and any other copy of the build's or the ledger's."
  (setq load-history
        (cl-remove-if (lambda (entry)
                        (let ((file (car-safe entry)))
                          (and (stringp file) (not (equal file keep))
                               (equal (or (and (equal file previous) path)
                                          (romacs-live--relative file roots)
                                          (let ((e (cl-find-if (lambda (e) (equal (plist-get e :file) file))
                                                               (plist-get romacs-live-ledger :entries))))
                                            (plist-get e :path))
                                          file)
                                      path))))
                      load-history)))

(defun romacs-live--set-entry (entry)
  "Put ENTRY in the ledger in place of PATH's, or drop PATH's when ENTRY
has only `:path', with its directory first on `load-path' in place of the
old one's."
  (let* ((path (plist-get entry :path))
         (old (romacs-live--entry path))
         (others (cl-remove path (plist-get romacs-live-ledger :entries) :key (lambda (e) (plist-get e :path)) :test #'equal)))
    (when-let* ((dir (plist-get old :dir))
                ((not (equal dir (plist-get entry :dir))))
                ((not (cl-find dir others :key (lambda (e) (plist-get e :dir)) :test #'equal))))
      (setq load-path (delete dir load-path)))
    (when-let* ((dir (plist-get entry :dir)))
      (setq load-path (cons dir (delete dir load-path))))
    (setq romacs-live-ledger
          (list :build (or (plist-get romacs-live-ledger :build) (romacs-live-build))
                :entries (if (plist-get entry :file) (append others (list entry)) others)))
    (unless (plist-get romacs-live-ledger :entries) (setq romacs-live-ledger nil))))

(defun romacs-live--foreign-setq-warnings (record)
  "Warnings for RECORD's top-level `setq's of variables other files define."
  (let ((defined (cl-loop for (key) in (car (romacs-live-forms-definitions (plist-get record :forms)))
                          when (memq (car key) '(variable constant option keymap)) collect (cadr key))))
    (mapcar (lambda (var) (format "sets %s, a variable another file defines" var))
            (romacs-live-forms-foreign-setqs (plist-get record :forms) defined))))

(defun romacs-live--entry-fields (record store label)
  "The ledger entry RECORD's file makes, its copy STORE, from LABEL's source
unless RECORD names its own; its `:state' still to add."
  (let ((path (plist-get record :path)))
    (list :path path :file store :dir (directory-file-name (expand-file-name (plist-get record :new) (romacs-live--store)))
          :base (plist-get record :base) :new (plist-get record :new)
          ;; The build's own copy, whatever patches came between.
          :origin (or (plist-get (romacs-live--entry path) :origin) (plist-get record :build-blob))
          :source (or (plist-get record :source) label) :time (float-time))))

(defun romacs-live--load (record loaded label defaults roots)
  "Load RECORD's file over the session, handling what a `load' leaves
wrong; its report row.  A file loads by its name on `load-path': a store
copy from its directory, put first; a file back to its build's content
from the build's tree, once its copy's directory is gone."
  (let* ((path (plist-get record :path))
         (loaded-p (gethash path loaded))
         ;; Loaded since the survey: it loads now.
         (action (pcase (plist-get record :action)
                   ('stage (if loaded-p 'load 'stage))
                   ('revert (if loaded-p 'revert 'unstage))
                   (action action)))
         (entry (romacs-live--history-entry loaded-p))
         (name (romacs-live--load-name path roots))
         (store (and (memq action '(load stage)) (romacs-live--store-put (plist-get record :new) name (plist-get record :bytes))))
         (fields (and store (romacs-live--entry-fields record store label)))
         (row (list :path path :action action :diff (plist-get record :diff))))
    (pcase action
      ('stage (romacs-live--set-entry (append fields (list :state 'staged))) row)
      ('unstage (romacs-live--set-entry (list :path path)) row)
      (_
       (let ((previous (romacs-live--entry path)))
         ;; The copy's directory first, or for a revert none of it.
         (romacs-live--set-entry (or fields (list :path path)))
         (let* ((values (romacs-live--before-values entry record))
                (exact (plist-get record :exact))
                (target (or exact (locate-file (file-name-sans-extension name) load-path (get-load-suffixes))))
                (keymaps nil) (start (float-time)) (keymap-rows nil)
                (failure (unwind-protect
                             (condition-case err
                                 (progn (setq keymaps (romacs-live--before-keymaps
                                                       entry (if defaults (cdr (assoc path defaults)) :unknown)))
                                        (load (or exact (file-name-sans-extension name)) nil t)
                                        nil)
                               (error (error-message-string err))
                               (quit "stopped by C-g"))
                           ;; Whatever happened, every map is bound again.
                           (setq keymap-rows (romacs-live--after-keymaps keymaps entry))))
                (ms (round (* 1000 (- (float-time) start))))
                ;; The load's own entry, wherever an after-load hook's
                ;; loads put theirs.
                (loaded-as (if (assoc target load-history) target (car-safe (car load-history))))
                (value-rows (and (not failure) (romacs-live--after-values values))))
           (when (and (not failure) (stringp loaded-as)) (romacs-live--forget-history loaded-as path roots loaded-p))
           (cond ((not failure))
                 ;; A failed load is partial: the next update loads it again.
                 (fields (romacs-live--set-entry (append fields (list :state 'partial))))
                 (previous (romacs-live--set-entry (plist-put (copy-sequence previous) :state 'partial))))
           (unless failure
             (when fields (romacs-live--set-entry (append fields (list :state 'loaded)))))
           (append row (list :ms ms :keymaps keymap-rows :values value-rows
                             :warnings (romacs-live--foreign-setq-warnings record) :error failure))))))))

(defun romacs-live--gone (rows)
  "The functions the report ROWS say their files no longer define and no
loaded file does: those another file defines (moved, or defined twice)
are not gone."
  (cl-loop for row in rows
           append (cl-loop for key in (plist-get (plist-get row :diff) :removed)
                           when (and (memq (car key) '(function macro)) (fboundp (cadr key))
                                     (not (symbol-file (cadr key) 'defun)))
                           collect (cadr key))))

(defun romacs-live-apply (plan &optional unbind-gone)
  "Load PLAN's files; the report, a plist: `:label', `:rows' (one a file),
`:gone' (functions no file defines any more), `:unbound', `:warnings',
`:failed' (the file that stopped the update, or nil) and `:ms'.  An
update stops at a file that signals; the files after it are not loaded.
UNBIND-GONE: `fmakunbound' the gone functions (a function, asked with
their list, or t)."
  (let* ((roots (romacs-live--lisp-roots (plist-get (romacs-live-build) :root)))
         (loaded (romacs-live--loaded roots))
         (defaults (plist-get plan :keymap-defaults))
         (romacs-live-forms--declared (plist-get plan :declared))
         ;; Loaded since the survey, so staged by the plan: checked now,
         ;; before any file loads.
         (late (cl-loop for r in (plist-get plan :records)
                        when (and (eq (plist-get r :action) 'stage) (gethash (plist-get r :path) loaded))
                        append (romacs-live--layout-refusals (plist-get r :path) (plist-get r :old-forms) (plist-get r :forms))))
         (start (float-time)))
    (if late
        (list :label (plist-get plan :label) :refusals late)
      (romacs-live--apply-records plan loaded defaults roots start unbind-gone))))

(defun romacs-live--apply-records (plan loaded defaults roots start unbind-gone)
  "Load PLAN's records with LOADED, DEFAULTS and ROOTS, begun at START,
and UNBIND-GONE; the report (`romacs-live-apply')."
  (let ((rows nil) (failed nil))
    (cl-loop for record in (plist-get plan :records)
             until failed
             do (if (eq (plist-get record :action) 'deleted)
                    (push (list :path (plist-get record :path) :action 'deleted :diff (plist-get record :diff)) rows)
                  (let ((row (romacs-live--load record loaded (plist-get plan :label) defaults roots)))
                    (push row rows)
                    (when (plist-get row :error) (setq failed (plist-get row :path))))))
    (setq rows (nreverse rows))
    (let* ((gone (romacs-live--gone rows))
           (unbound (and gone unbind-gone (or (eq unbind-gone t) (funcall unbind-gone gone))
                         (progn (mapc #'fmakunbound gone) gone))))
      (romacs-live--refresh-mode-line)
      (list :label (plist-get plan :label) :rows rows :gone gone :unbound unbound
            :warnings (plist-get plan :warnings) :failed failed
            :ms (round (* 1000 (- (float-time) start)))))))

;;;; The update

(defun romacs-live--worker-p ()
  (and (not noninteractive) (fboundp 'isolate-spawn) (fboundp 'isolate-main-p) (isolate-main-p)))

(defun romacs-live--request (source)
  (let* ((build (or (plist-get romacs-live-ledger :build) (romacs-live-build)))
         (roots (romacs-live--lisp-roots (plist-get build :root))))
    (list :source source :build build
          ;; A partial entry's file loads again: it has no blob to compare.
          :ledger (mapcar (lambda (e) (list (plist-get e :path) (plist-get e :new) (plist-get e :file)
                                            (eq (plist-get e :state) 'partial)))
                          (plist-get romacs-live-ledger :entries))
          :loaded (hash-table-keys (romacs-live--loaded roots)))))

(defun romacs-live--survey-then (request callback)
  "Survey REQUEST in a worker VM, or here in batch; CALLBACK gets the
survey, or signals' text as (:error TEXT)."
  (let ((answer (lambda (thunk) (funcall callback (condition-case err (funcall thunk)
                                                     (error (list :error (error-message-string err))))))))
    (cond
     ((eq (plist-get (plist-get request :source) :kind) 'ledger)
      ;; A replay reads the store and the build's own files: no git.
      (funcall answer (lambda () (romacs-live--replay-survey (plist-get (plist-get request :source) :ledger)))))
     ((romacs-live--worker-p)
        (isolate-on-done (isolate-spawn 'romacs-live-survey request :require '(romacs-live))
                         (lambda (future) (funcall answer (lambda () (isolate-await future 0))))))
     (t (funcall answer (lambda () (romacs-live-survey request)))))))

(defun romacs-live--defaults-then (plan callback)
  "PLAN with `:keymap-defaults' (path to its keymaps' library defaults),
given to CALLBACK."
  (let* ((roots (romacs-live--lisp-roots (plist-get (romacs-live-build) :root)))
         (loaded (romacs-live--loaded roots))
         (wanted (cl-loop for r in (plist-get plan :records)
                          for vars = (and (memq (plist-get r :action) '(load revert dependent))
                                          (romacs-live--keymap-variables (romacs-live--history-entry (gethash (plist-get r :path) loaded))))
                          when vars collect (list (plist-get r :path) (gethash (plist-get r :path) loaded) vars)))
         (ledger (mapcar (lambda (e) (plist-get e :file)) (plist-get romacs-live-ledger :entries)))
         (results nil))
    (if (or (null wanted) (null romacs-live-keymap-defaults-function))
        (funcall callback plan)
      (let ((pending (length wanted)))
        (pcase-dolist (`(,path ,file ,vars) wanted)
          (funcall romacs-live-keymap-defaults-function (remove file ledger) file vars
                   (lambda (defaults)
                     (push (cons path defaults) results)
                     (when (zerop (cl-decf pending))
                       (funcall callback (append plan (list :keymap-defaults
                                                            (cl-remove-if-not #'cdr results))))))))))))

(defun romacs-live-start (source callback &rest options)
  "Update the session from SOURCE (`romacs-live-survey''s `:source') and
call CALLBACK with the report, or with (:refusals REASONS) when nothing
loaded.  OPTIONS: `:plan-only' (CALLBACK gets the plan's summary, nothing
loads) and `:unbind-gone' (as `romacs-live-apply')."
  (when romacs-live--running (user-error "A live update is already running"))
  (setq romacs-live--running source)
  (let ((finish (lambda (report) (setq romacs-live--running nil romacs-live-last-report report)
                  ;; The tree the ledger came from: where its suites are.
                  (when (and (cl-some (lambda (r) (memq (plist-get r :action) '(load stage revert dependent)))
                                      (plist-get report :rows))
                             (not (plist-get report :plan))
                             (not (eq (plist-get source :kind) 'ledger)))
                    (setq romacs-live-last-source source))
                  (funcall callback report))))
    (condition-case err
        (romacs-live--survey-then
         (romacs-live--request source)
         (lambda (survey)
           (condition-case err
               (if (plist-get survey :error)
                   (funcall finish (list :refusals (list (plist-get survey :error))))
                 (let* ((roots (romacs-live--lisp-roots (plist-get (romacs-live-build) :root)))
                        (plan (romacs-live-plan survey (romacs-live--loaded roots))))
                   (cond ((plist-get plan :refusals)
                          (funcall finish (list :label (plist-get plan :label) :refusals (plist-get plan :refusals)
                                                :dropped (plist-get plan :dropped))))
                         ((plist-get options :plan-only)
                          (funcall finish (list :label (plist-get plan :label) :plan t :warnings (plist-get plan :warnings)
                                                :rows (mapcar (lambda (r) (list :path (plist-get r :path) :action (plist-get r :action)
                                                                                :diff (plist-get r :diff)))
                                                              (plist-get plan :records)))))
                         (t (romacs-live--defaults-then
                             plan (lambda (plan)
                                    (funcall finish (condition-case err
                                                        (append (romacs-live-apply plan (plist-get options :unbind-gone))
                                                                (list :dropped (plist-get plan :dropped)))
                                                      ((error quit) (list :stopped (romacs-live--stopped err)))))))))))
             ((error quit) (funcall finish (list :refusals (list (romacs-live--stopped err))))))))
      ((error quit) (setq romacs-live--running nil) (signal (car err) (cdr err))))))

(defun romacs-live--stopped (condition)
  "What CONDITION, an error or a quit, says of the update it stopped."
  (if (eq (car condition) 'quit)
      "Stopped by C-g: the ledger says what loaded"
    (error-message-string condition)))

(defun romacs-live--repository ()
  "The repository a branch or commit comes from."
  (or romacs-live-repository
      (when-let* ((top (locate-dominating-file default-directory "etc/romacs-runtime-epoch")))
        (expand-file-name top))
      (user-error "No repository: set `romacs-live-repository' or update from a worktree")))

(defun romacs-live-source (text &optional repository)
  "The source TEXT names: an existing directory is a worktree, an existing
file a file, anything else a branch or commit of REPOSITORY."
  (let ((file (expand-file-name text)))
    (cond ((and (file-directory-p file) (file-exists-p (expand-file-name "etc/romacs-runtime-epoch" file)))
           (list :kind 'worktree :directory file))
          ((file-regular-p file) (list :kind 'files :files (list file)))
          (t (list :kind 'rev :rev text :repository (or repository (romacs-live--repository)))))))

;;;; Across a restart, and back to the build

(defun romacs-live--image-part ()
  "The ledger as a session image's part (`warm-restart-image-part-functions'):
the successor puts its files back (`romacs-live-restore-ledger')."
  (and (plist-get romacs-live-ledger :entries)
       (cons 'romacs-live-restore-ledger (copy-tree romacs-live-ledger))))

(add-hook 'warm-restart-image-part-functions #'romacs-live--image-part)

(defun romacs-live--replay-survey (ledger)
  "A survey (`romacs-live-survey''s value) that puts LEDGER's files back
over this session's build, each from its store file while its base is
still the build's copy.  An entry the build now has is `landed', one
whose base the build no longer has `superseded', a partial one or one
whose store file is gone `dropped': each left out, and said."
  (let* ((build (romacs-live-build))
         (roots (romacs-live--lisp-roots (plist-get build :root)))
         (loaded (romacs-live--loaded roots))
         (files nil) (dropped nil))
    (dolist (entry (plist-get ledger :entries))
      (let* ((path (plist-get entry :path))
             (installed (and (plist-get build :commit) (string-prefix-p "lisp/" path) roots
                             (romacs-live--read-file (expand-file-name (substring path 5) (car roots)))))
             (store (romacs-live--read-file (plist-get entry :file)))
             (origin (if (plist-member entry :origin) (plist-get entry :origin) (plist-get entry :base)))
             (why (cond ((equal (car installed) (plist-get entry :new)) 'landed)
                        ((not (equal (car installed) origin)) 'superseded)
                        ((eq (plist-get entry :state) 'partial) 'partial)
                        ((not (equal (car store) (plist-get entry :new))) 'lost))))
        (if why
            (push (list :path path :why why) dropped)
          (push (list :path path :base origin :new (plist-get entry :new)
                      :build-blob (car installed) :bytes (cdr store) :text (romacs-live--text (cdr store))
                      :old (romacs-live--text (cdr installed)) :source (plist-get entry :source)
                      :loaded (and (gethash path loaded) t))
                files))))
    (list :label "the ledger" :files (nreverse files) :refusals nil :warnings nil :dropped (nreverse dropped))))

;;;###autoload
(defun romacs-live-restore-ledger (ledger done)
  "Put LEDGER's live files back over this session, then call DONE: a
restart's successor does, from its image (`romacs-live--image-part'),
before any buffer comes back.  The files the build has since taken, or
whose base it no longer has, are left out, and the report
\(`romacs-live-ledger') says so.  A session with live files or an update
of its own refuses: its patches would run unrecorded."
  (when (or romacs-live--running (plist-get romacs-live-ledger :entries))
    (user-error "This session runs live files of its own: the image's ledger is not put back"))
  (romacs-live-start (list :kind 'ledger :ledger ledger)
                     (lambda (report)
                       (message "%s" (romacs-live-summary report))
                       (romacs-live--refresh-mode-line)
                       (funcall done))))

;;;###autoload
(defun romacs-session-reset-target ()
  "The installed build a reset goes to: the one the server watches
\(`romacs-server-build-watch'), else this session's own executable with
its runtime (`warm-restart--target'): never another build, one a restart
command names or one beside the runtime."
  (let ((watch (bound-and-true-p romacs-server-build-watch)))
    (if (and (stringp watch) (file-exists-p watch))
        watch
      (expand-file-name invocation-name invocation-directory))))

;;;###autoload
(defun romacs-session-reset (&optional keep-buffers)
  "Restart onto the installed build with a fresh session: its image and
init only, no live files (`romacs-live-ledger'), kill ring, registers,
histories or kept globals.  Unsaved text is carried, as every restart
carries it, and so are agent sessions and the keeper's terminals, the
user's work outside the session.  With a prefix argument, KEEP-BUFFERS:
the file buffers and their windows too.  The same supervised restart as
`romacs-restart', once the editor is idle (`warm-restart-request')."
  (interactive (list current-prefix-arg))
  (require 'warm-restart)
  (let ((target (romacs-session-reset-target))
        (live (length (plist-get romacs-live-ledger :entries))))
    (when (or (not (called-interactively-p 'any))
              (yes-or-no-p (format "Reset onto %s%s, dropping histories and the kill ring%s? "
                                   (abbreviate-file-name target)
                                   (if (> live 0) (format " without its %d live files" live) "")
                                   (if keep-buffers "" " and every buffer but unsaved ones"))))
      (warm-restart-request :target target :fresh (if keep-buffers 'buffers 'unsaved))
      (message "Resetting once the editor is idle"))))

;;;; The report and the ledger, shown

(defun romacs-live--key-text (key)
  (format "%s %s" (car key) (cadr key)))

(defconst romacs-live--state-width 9
  "The width of the state column, in the ledger and the report alike, so
paths start at one column.")

(defun romacs-live--count (report what)
  "How many of REPORT's rows WHAT is: `whole' (loaded whole), `partial',
`staged'."
  (cl-count-if (lambda (r)
                 (pcase what
                   ('whole (and (memq (plist-get r :action) '(load revert dependent)) (not (plist-get r :error))))
                   ('partial (plist-get r :error))
                   ('staged (eq (plist-get r :action) 'stage))))
               (plist-get report :rows)))

(defun romacs-live--outcome (report)
  "REPORT's outcome, a symbol: `refused', `stopped', `planned', `current',
`failed' or `done'."
  (cond ((plist-get report :refusals) 'refused)
        ((plist-get report :stopped) 'stopped)
        ((plist-get report :plan) 'planned)
        ((null (plist-get report :rows)) 'current)
        ((plist-get report :failed) 'failed)
        (t 'done)))

(defun romacs-live--counts (report)
  "REPORT's counts, joined by middle dots."
  (string-join (delq nil (list (format "%d loaded" (romacs-live--count report 'whole))
                               (let ((n (romacs-live--count report 'partial))) (and (> n 0) (format "%d partial" n)))
                               (let ((n (romacs-live--count report 'staged))) (and (> n 0) (format "%d staged" n)))
                               (and (plist-get report :gone) (format "%d gone" (length (plist-get report :gone))))
                               (and (plist-get report :ms) (format "%d ms" (plist-get report :ms)))))
               " · "))

(defun romacs-live--dropped-text (report)
  "REPORT's dropped entries, by why: \"2 landed, 1 superseded\"."
  (let ((whys nil))
    (dolist (d (plist-get report :dropped)) (cl-incf (alist-get (plist-get d :why) whys 0)))
    (mapconcat (lambda (w) (format "%d %s" (cdr w) (car w))) (nreverse whys) ", ")))

(defun romacs-live-summary (report)
  "One line saying what REPORT did."
  (pcase (romacs-live--outcome report)
    ('refused (format "Live update refused, nothing loaded: %s%s" (string-join (plist-get report :refusals) "; ")
                      (if (plist-get report :dropped) (concat "; " (romacs-live--dropped-text report)) "")))
    ('stopped (format "Live update %s" (plist-get report :stopped)))
    ('planned (format "Live update from %s would touch %d files" (plist-get report :label) (length (plist-get report :rows))))
    ('current (if (plist-get report :dropped)
                  (format "Live update from %s: nothing put back; %s" (or (plist-get report :label) "the source")
                          (romacs-live--dropped-text report))
                (format "Live update from %s: the session has it already" (or (plist-get report :label) "the source"))))
    ('failed (format "Live update from %s stopped at %s, its changes so far applied: %s" (plist-get report :label)
                     (plist-get report :failed) (romacs-live--counts report)))
    (_ (format "Live update from %s: %s%s" (plist-get report :label) (romacs-live--counts report)
               (if (plist-get report :dropped) (concat "; " (romacs-live--dropped-text report)) "")))))

(defun romacs-live--verb (row)
  "What ROW's file is now, as the report's state column says it."
  (if (plist-get row :error) "◐ partial"
    (pcase (plist-get row :action)
      ('load "loaded") ('stage "staged") ('revert "reverted") ('unstage "unstaged")
      ('dependent "reloaded") (action (symbol-name action)))))

(defun romacs-live--detail (kind text &optional face)
  "Insert a detail line under its path: KIND dim, TEXT in FACE."
  (insert (make-string (+ 3 romacs-live--state-width) ?\s)
          (propertize (format "%-9s" kind) 'face 'romacs-live-dim)
          (if face (propertize text 'face face) text) "\n"))

(defun romacs-live--insert-row (row)
  "Insert report ROW: its state, its path, then each change, one shape:
KIND, the names, what happened to them."
  (let* ((diff (plist-get row :diff))
         ;; Definitions whose effect has its own line are said there.
         (effects (append (mapcar (lambda (k) (plist-get k :variable)) (plist-get row :keymaps))
                          (mapcar (lambda (v) (or (plist-get v :option) (plist-get v :face) (plist-get v :variable)))
                                  (plist-get row :values)))))
    (insert "  " (propertize (string-pad (romacs-live--verb row) romacs-live--state-width) 'face
                             (if (plist-get row :error) 'romacs-live-partial 'default))
            " " (propertize (plist-get row :path) 'face 'romacs-live-path)
            (if (plist-get row :ms) (propertize (format "  %d ms" (plist-get row :ms)) 'face 'romacs-live-dim) "")
            "\n")
    (pcase-dolist (`(,part . ,verb) '((:added . "added") (:changed . "changed") (:removed . "removed")))
      (let ((by-kind nil))
        (dolist (key (plist-get diff part))
          (unless (memq (cadr key) effects)
            (push (cadr key) (alist-get (car key) by-kind))))
        (pcase-dolist (`(,kind . ,names) (nreverse by-kind))
          (romacs-live--detail (symbol-name kind) (format "%s %s" (mapconcat #'symbol-name (nreverse names) ", ") verb)))))
    (dolist (k (plist-get row :keymaps))
      (let ((name (symbol-name (plist-get k :variable))))
        (cond ((plist-get k :kept) (romacs-live--detail "keymap" (format "%s kept: %s" name (plist-get k :kept))))
              ((plist-get k :conflicts)
               (romacs-live--detail "keymap" (format "%s replaced; your keys it now takes: %s" name
                                                     (mapconcat #'key-description (plist-get k :conflicts) " "))
                                    'romacs-live-warning))
              ((plist-get k :unchecked)
               (romacs-live--detail "keymap" (format "%s replaced; your own keys not checked" name) 'romacs-live-warning))
              ((plist-get k :user)
               (romacs-live--detail "keymap" (format "%s replaced; yours kept: %s" name
                                                     (mapconcat #'key-description (plist-get k :user) " "))))
              (t (romacs-live--detail "keymap" (format "%s replaced" name))))))
    (dolist (v (plist-get row :values))
      (let ((name (symbol-name (or (plist-get v :option) (plist-get v :face) (plist-get v :variable))))
            (kind (cond ((plist-get v :option) "option") ((plist-get v :face) "face") (t "variable"))))
        (cond ((plist-get v :kept) (romacs-live--detail kind (format "%s kept: %s" name (plist-get v :kept))))
              ((plist-get v :error) (romacs-live--detail kind (format "%s: %s" name (plist-get v :error)) 'romacs-live-error))
              ((plist-get v :respecified) (romacs-live--detail kind (format "%s takes its new spec" name)))
              (t (romacs-live--detail kind (format "%s takes its new default" name))))))
    (dolist (w (plist-get row :warnings))
      (romacs-live--detail "!" w 'romacs-live-warning))
    (when (plist-get row :error)
      (insert (make-string (+ 3 romacs-live--state-width) ?\s)
              (propertize (concat "✗ signalled: " (plist-get row :error)) 'face 'romacs-live-error) "\n"))))

(defun romacs-live--insert-ledger ()
  (let* ((build (or (plist-get romacs-live-ledger :build) (romacs-live-build)))
         (entries (plist-get romacs-live-ledger :entries))
         (partial (cl-count 'partial entries :key (lambda (e) (plist-get e :state))))
         (width (apply #'max 0 (mapcar (lambda (e) (length (plist-get e :path))) entries))))
    (insert (propertize "Session" 'face 'romacs-live-heading) "  "
            (propertize "build " 'face 'romacs-live-dim)
            (propertize (if (plist-get build :commit) (romacs-live--short (plist-get build :commit)) "unknown")
                        'face 'romacs-live-build)
            (cond ((null entries) (concat "  as built " (propertize "✓" 'face 'romacs-live-ok)))
                  (t (concat (propertize (format "  + %d live" (length entries)) 'face 'romacs-live-patched)
                             (if (> partial 0) (propertize (format " · ◐ %d partial" partial) 'face 'romacs-live-partial) ""))))
            "\n")
    (if (null entries)
        (insert "  " (substitute-command-keys "\\[romacs-live-update] loads a branch's or a worktree's Lisp into this session")
                "\n")
      (dolist (e entries)
        (insert "  " (if (eq (plist-get e :state) 'partial)
                         (propertize (string-pad "◐ partial" romacs-live--state-width) 'face 'romacs-live-partial)
                       (string-pad (symbol-name (plist-get e :state)) romacs-live--state-width))
                " " (propertize (string-pad (plist-get e :path) width) 'face 'romacs-live-path)
                (format "  %s→%s  %s  " (if (plist-get e :base) (romacs-live--short (plist-get e :base)) "new      ")
                        (romacs-live--short (plist-get e :new)) (plist-get e :source))
                (propertize (format-time-string "%H:%M" (plist-get e :time)) 'face 'romacs-live-dim)
                "\n")))))

(defun romacs-live--insert-dropped (report)
  "Insert a row for each ledger entry REPORT's replay left out, and why."
  (dolist (d (plist-get report :dropped))
    (insert "  " (propertize (string-pad (format "%s" (plist-get d :why)) romacs-live--state-width) 'face
                             (if (eq (plist-get d :why) 'landed) 'romacs-live-ok 'romacs-live-partial))
            " " (propertize (plist-get d :path) 'face 'romacs-live-path)
            (propertize (pcase (plist-get d :why)
                          ('landed "  the build has it now")
                          ('superseded "  the build changed it since: not put back")
                          ('partial "  it failed to load before: not put back")
                          (_ "  its copy is gone from the store"))
                        'face 'romacs-live-dim)
            "\n")))

(defun romacs-live--insert-report (report)
  "Insert REPORT: its outcome first, then where it came from, then a row
a file."
  (insert "\n" (propertize "Last update" 'face 'romacs-live-heading) "  ")
  (let ((from (propertize (format "  from %s" (or (plist-get report :label) "the source")) 'face 'romacs-live-dim)))
    (pcase (romacs-live--outcome report)
      ('refused
       (insert (propertize "⊘ refused" 'face 'romacs-live-partial) " · nothing loaded" "\n")
       (dolist (reason (plist-get report :refusals)) (insert "  " reason "\n"))
       (romacs-live--insert-dropped report))
      ('stopped (insert (propertize (concat "✗ " (plist-get report :stopped)) 'face 'romacs-live-error) "\n"))
      (outcome
       (insert (pcase outcome
                 ('failed (concat (propertize "✗ stopped after a partial load" 'face 'romacs-live-error)
                                  " · " (romacs-live--counts report)))
                 ('current (propertize "already current" 'face 'romacs-live-ok))
                 ('planned (propertize (format "plan · %d files" (length (plist-get report :rows))) 'face 'romacs-live-ok))
                 (_ (romacs-live--counts report)))
               from "\n")
       (when (eq outcome 'failed)
         (insert "  Forms before the error ran, so some of its changes took effect; the rows list the file's changes.\n"))
       (dolist (row (plist-get report :rows)) (romacs-live--insert-row row))
       (dolist (w (plist-get report :warnings)) (insert "  " (propertize (concat "! " w) 'face 'romacs-live-warning) "\n"))
       (romacs-live--insert-dropped report)
       (when-let* ((gone (plist-get report :gone)))
         (insert "  " (propertize "gone " 'face 'romacs-live-dim) (mapconcat #'symbol-name gone ", ")
                 (propertize (if (plist-get report :unbound) "  unbound"
                               (substitute-command-keys "  still defined; \\[universal-argument] \\[romacs-live-update] unbinds"))
                             'face 'romacs-live-dim)
                 "\n"))))))

(defvar-keymap romacs-live-ledger-mode-map
  :doc "Keys of the live update's buffer."
  :parent special-mode-map
  "g" #'romacs-live-ledger-refresh)

(define-derived-mode romacs-live-ledger-mode special-mode "Live"
  "The session's ledger of live-loaded files and the last update's report."
  (setq-local revert-buffer-function (lambda (&rest _) (romacs-live-ledger-refresh))
              left-margin-width 1))

(defun romacs-live-ledger-refresh ()
  "Show the ledger and the last report again."
  (interactive nil romacs-live-ledger-mode)
  (let ((inhibit-read-only t))
    (erase-buffer)
    (romacs-live--insert-ledger)
    (when romacs-live-last-report (romacs-live--insert-report romacs-live-last-report))
    (goto-char (point-min))))

;;;###autoload
(defun romacs-live-ledger ()
  "Show what this session runs beyond its build, and the last update."
  (interactive)
  (with-current-buffer (get-buffer-create "*romacs-live*")
    (romacs-live-ledger-mode)
    (romacs-live-ledger-refresh)
    (pop-to-buffer (current-buffer))))

(defun romacs-live--refresh-mode-line ()
  "Show the ledger's mark in the mode line while it has entries."
  (let ((entries (plist-get romacs-live-ledger :entries)))
    (setq romacs-live-mode-line
          (and entries
               (propertize (format " Δ%d%s " (length entries)
                                   (if (cl-find 'partial entries :key (lambda (e) (plist-get e :state))) "!" ""))
                           'face (if (cl-find 'partial entries :key (lambda (e) (plist-get e :state)))
                                     'romacs-live-partial 'romacs-live-patched)
                           'help-echo (format "Build %s + %d live files: %s\nmouse-1: the ledger"
                                              (romacs-live--short (or (plist-get (plist-get romacs-live-ledger :build) :commit) "unknown"))
                                              (length entries)
                                              (mapconcat (lambda (e) (plist-get e :path)) entries ", "))
                           'mouse-face 'mode-line-highlight
                           'local-map (make-mode-line-mouse-map 'mouse-1 #'romacs-live-ledger))))
    (when (and entries (not (and (listp global-mode-string) (memq 'romacs-live-mode-line global-mode-string))))
      ;; A list headed by a string: one headed by a symbol is a conditional.
      (setq global-mode-string (append (cond ((null global-mode-string) (list ""))
                                             ((and (listp global-mode-string) (stringp (car global-mode-string)))
                                              global-mode-string)
                                             (t (list "" global-mode-string)))
                                       '(romacs-live-mode-line))))
    (force-mode-line-update t)))

;;;###autoload
(defun romacs-live-update (source &optional unbind-gone)
  "Load into this session the Lisp SOURCE has that it lacks.
SOURCE is beta, a branch or a commit of `romacs-live-repository', a
worktree directory, or a file.  Every file whose content differs from
what the session loaded loads, in dependency order; a file the session has
not loaded waits, first on `load-path'.  With a prefix argument
\(UNBIND-GONE), offer to unbind the functions the files no longer define.
The report is in the *romacs-live* buffer (`romacs-live-ledger')."
  (interactive (list (read-string (format-prompt "Live update from (beta, a branch, a worktree or a file)" "beta")
                                  nil nil "beta")
                     current-prefix-arg))
  (romacs-live-start (if (stringp source) (romacs-live-source source) source)
                       (lambda (report)
                         (message "%s" (romacs-live-summary report))
                         (when (get-buffer "*romacs-live*")
                           (with-current-buffer "*romacs-live*" (romacs-live-ledger-refresh))))
                       :unbind-gone (and unbind-gone
                                         (lambda (gone) (y-or-n-p (format "Unbind %s? " (mapconcat #'symbol-name gone ", ")))))))

;;;; As an MCP tool

(defun romacs-live--json (value)
  "VALUE, a report, as JSON values: plists as objects, lists as arrays."
  (cond ((and (consp value) (keywordp (car value)))
         (let ((table (make-hash-table :test #'equal)))
           (cl-loop for (key item) on value by #'cddr
                    unless (memq key '(:forms :old-forms :bytes :text :old))
                    do (puthash (substring (symbol-name key) 1) (romacs-live--json item) table))
           table))
        ((and (consp value) (symbolp (car value)) (memq (car value) (mapcar #'cdr romacs-live-forms--kinds)))
         (romacs-live--key-text value))
        ((and (consp value) (vectorp (car value)) (cl-every #'vectorp value))
         (vconcat (mapcar #'key-description value)))
        ((consp value) (vconcat (mapcar #'romacs-live--json value)))
        ((eq value t) t)
        ((null value) :false)
        ((symbolp value) (symbol-name value))
        (t value)))

;;;###autoload
(defun romacs-live-mcp-update (arguments)
  "The `romacs_live_update' tool: ARGUMENTS name the source."
  (romacs-mcp--instance arguments)
  (let* ((files (gethash "files" arguments))
         (text (gethash "source" arguments))
         (source (cond ((and files (> (length files) 0)) (list :kind 'files :files (append files nil)))
                       (text (romacs-live-source text (gethash "repository" arguments)))
                       (t (user-error "Give source or files"))))
         (done nil))
    (romacs-live-start source (lambda (report) (setq done (list (romacs-live--json (append (list :summary (romacs-live-summary report)) report)))))
                       :plan-only (eq (gethash "plan" arguments) t)
                       :unbind-gone (eq (gethash "unbindGone" arguments) t))
    (romacs-mcp-defer (lambda () done))))

(provide 'romacs-live)
;;; romacs-live.el ends here
