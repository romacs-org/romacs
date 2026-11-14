;;; probes.el --- scaling probes: one workload at size N and k*N  -*- lexical-binding: t -*-

;; Maintainer: Thanos Apollo <public@thanosapollo.org>

;;; Commentary:
;; scaling_live.py runs each probe of `scaling-probes' in a fresh session
;; at N and at K*N and compares the work by the evaluating thread's
;; instruction count (`romacs-instructions'): its empirical exponent,
;; log(work(K*N) / work(N)) / log(K), must stay within the probe's
;; budget.  A probe holds a growing collection of one subsystem, at sizes
;; real sessions reach.
;;
;; A probe is (scaling-probe NAME :n N [:k K] [:budget B] [:known TICKET]
;; [:gui t] [:steps R] :setup FORM :body FORM):
;;   :setup  runs uncounted with `n' bound; its value is `it' in :body.
;;   :body   the counted work at size `n'.  A batch probe's body runs once
;;           with collection held off, so where a collection lands does
;;           not decide the count; a :gui probe's body is one step, run
;;           :steps R times from timers, each laid out by the host before
;;           the next, `i' bound to the step.
;;   :budget the largest exponent allowed (default 1.25: n log n).
;;   :slope  instead, the most instructions the work may grow by per unit
;;           of N, per step: (work(K*N) - work(N)) / ((K - 1) * N * steps).
;;           For a probe whose claim is that N costs nothing, where an
;;           exponent over total work moves whenever a fixed cost does.
;;   :known  a ticket naming a known super-linear cost: the probe still
;;           runs and reports, but over budget it is debt, not a failure.
;;   :ready  a GUI probe's condition for its next step, besides the host's
;;           redisplay of the step before: the chain waits for it, as it
;;           does for the redisplay, rather than for a time.
;;   :gap    how often, in seconds, a GUI probe looks for its step's
;;           redisplay (default 0.1).
;;   :idle   a GUI probe whose steps change nothing shown: each step's
;;           instructions are reported apart, and the harness counts the
;;           steps no paint followed (the host repaints for reasons of its
;;           own now and then), where any other probe's measurement is
;;           refused unless every step paints.
;;   :stack  also report how much native stack the body keeps
;;           (`romacs-native-stack-room' at `scaling-stack-mark' calls).
;;
;; Each probe lives in probes/NAME.el, loaded at the end of this file; this
;; file is the harness: the macro, the helpers probes call, and the runner.
;;
;; Batch: romacs -Q --batch -l probes.el --eval '(scaling-run (quote NAME) N)'
;; prints one line, SCALING <json>.  `scaling-list' prints the probes.

;;; Code:

(require 'cl-lib)

(defvar scaling-probes nil
  "The probes, newest first: (NAME . PLIST).")

(defmacro scaling-probe (name &rest plist)
  "Define probe NAME from PLIST, its :setup, :body and :ready forms made functions."
  (let ((setup (plist-get plist :setup)) (body (plist-get plist :body)) (ready (plist-get plist :ready)))
    `(setf (alist-get ',name scaling-probes)
           (list ,@(cl-loop for (key value) on plist by #'cddr
                            unless (memq key '(:setup :body :ready)) append (list key value))
                 :setup (lambda (n) (ignore n) ,setup)
                 :body (lambda (n it &optional i) (ignore n it i) ,body)
                 ,@(and ready `(:ready (lambda () ,ready)))))))

(defvar scaling-stack-marks nil
  "`romacs-native-stack-room' at each `scaling-stack-mark' of a body.")

(defun scaling-stack-mark ()
  "Record the native stack room here, for a :stack probe."
  (push (romacs-native-stack-room) scaling-stack-marks))

(defun scaling--names (prefix n)
  "N strings PREFIX-0 ... PREFIX-(N-1)."
  (let (names) (dotimes (i n) (push (format "%s-%d" prefix i) names)) (nreverse names)))

(defun scaling--lines-buffer (name n)
  "A buffer NAME of N numbered lines of prose."
  (with-current-buffer (get-buffer-create name)
    (erase-buffer)
    (dotimes (i n) (insert (format "line %d of some ordinary prose text, about sixty bytes\n" i)))
    (goto-char (point-min))
    (current-buffer)))

(defun scaling--file (n form-of)
  "A temporary .el file of N top-level forms, (FORM-OF I) the I-th."
  (let ((file (make-temp-file "scaling-" nil ".el")))
    (with-temp-file file
      (insert ";;; -*- lexical-binding: t -*-\n")
      (dotimes (i n) (prin1 (funcall form-of i) (current-buffer)) (insert "\n")))
    file))

(defun scaling--defun-form (i)
  "The I-th generated form: a defun with a body worth expanding, and every
thousandth a stack mark."
  (if (= (% i 1000) 999)
      '(scaling-stack-mark)
    `(defun ,(intern (format "scaling-f%d" i)) (x)
       (let ((a x) (b ,i))
         (setq a (if (> a 0) (* b 2) (- b 1)))
         (list a ,(format "s%d" i) '(q . ,i))))))

(defun scaling--candidates (n)
  "N file-like completion candidates."
  (let (names)
    (dotimes (i n)
      (push (format "src/module-%d/%s-%d.el" (% i 37) (nth (% i 5) '("abbrev" "buffer" "cabinet" "dired" "editor")) i)
            names))
    (nreverse names)))

;;;; User options (lisp/romacs/romacs-option.el)

(defun scaling--declare-options (prefix n)
  "Declare N integer options named from PREFIX in one group, and name them."
  (require 'romacs-option)
  (mapcar (lambda (name)
            (let ((option (intern name)))
              (custom-declare-variable option 0 "A scaling option." :type 'integer :group 'scaling-options)
              option))
          (scaling--names prefix n)))

(defun scaling--saving-session ()
  "Give this batch session an init and a directory, as a session has."
  (let ((directory (file-name-as-directory (make-temp-file "scaling-custom" t))))
    (setq user-emacs-directory directory custom-file nil
          user-init-file (expand-file-name "init.el" directory))))

;;;; Faces

;; A preview's save and restore: what it copies grows with the faces, and
;; with the frames that have faces of their own, and not with those that
;; show another's.
(defun scaling--preview-theme ()
  (deftheme scaling-candidate "One face.")
  (custom-theme-set-faces 'scaling-candidate '(highlight ((t :foreground "#102030")))))

(defun scaling--frames (n owned)
  "N child frames, OWNED ones with faces of their own."
  (unless (fboundp 'pgtk-register-dnd-targets) (defalias 'pgtk-register-dnd-targets #'ignore))
  (dotimes (_ n)
    (let ((frame (make-frame `((parent-frame . ,(selected-frame)) (visibility . nil) (minibuffer . nil)))))
      (when owned (set-face-attribute 'highlight frame :weight 'bold)))))

(defun scaling--preview-once (buffer)
  (romacs-theme-preview--session
    (romacs-theme-preview--show 'scaling-candidate))
  (with-current-buffer buffer (insert "x")))

;; N terminals open, each with a table of its own, which what is shown
;; must not walk: an ordinary buffer finds none, the last terminal opened
;; finds its own, and a face change makes it again.  Constant in N.
(defvar romacs-keeper-enabled)
(defun scaling--terminals (n)
  "N terminals, each shown once; the last.
In this process: what is measured is their display, not a keeper."
  (require 'romacs-term)
  (setq romacs-keeper-enabled nil)
  (dotimes (_ n) (romacs-term "/bin/cat") (redisplay t))
  (current-buffer))

;; A tab bar of N characters, each in a face of its own colour, so N rows
;; (each_distinct_face_in_the_tab_bar_is_published_once): found on each step, and made
;; again on each step after a face change.
(defun scaling--tab-bar (n)
  "A tab bar of N distinct faces, and a buffer to type in."
  (let* ((faces (mapcar #'intern (scaling--names "scaling-tab" n)))
         (row (let ((i 0))
                (mapconcat
                 (lambda (face)
                   (setq i (1+ i))
                   (custom-declare-face face `((t :foreground ,(format "#%06x" (* 7 i)))) "A scaling face." :group 'scaling)
                   (propertize "x" 'face face))
                 faces ""))))
    (defalias 'romacs--tab-bar-string (lambda () row))
    (modify-frame-parameters nil '((tab-bar-lines . 1)))
    (switch-to-buffer (get-buffer-create "scaling-tab"))))

;;;; Buffers, markers, overlays, text properties, undo

;;;; Lambda lists

;;;; Tree-sitter

;;;; Syntax summaries (crates/romacs-vm/src/syntax_index.rs)

(defun scaling--syntax-buffer (name n &optional lines)
  "A buffer NAME of N copies of LINES (C-like code by default), in a C
comment and string syntax, its summaries made by one parse to the end."
  (with-current-buffer (get-buffer-create name)
    (erase-buffer)
    (let ((table (make-char-table 'syntax-table)))
      (set-char-table-parent table (standard-syntax-table))
      (dolist (entry '((?/ . ". 124b") (?* . ". 23") (?\n . "> b") (?\" . "\"") (?\\ . "\\") (?# . "' 14") (?| . "\" 23bn")))
        (modify-syntax-entry (car entry) (cdr entry) table))
      (set-syntax-table table))
    (dotimes (i n)
      (insert (or lines (format "int f%d(int a) { /* (c */ return a + \"s(\" + %d; } // d {\n" i i))))
    (parse-partial-sexp (point-min) (point-max))
    (current-buffer)))

;;;; Windows

;;;; Symbols, hash tables, features

;;;; Loading and evaluating many forms

;;;; Completion

(defmacro scaling--completion-probe (name style &optional library)
  `(scaling-probe ,name :n 2000
     :setup (progn (require 'minibuffer) ,@(and library `((require ',library))) (scaling--candidates n))
     :body (let ((completion-styles '(,style)) (completion-ignore-case nil))
             (completion-all-completions "ed" it nil 2)
             (completion-try-completion "ed" it nil 2))))

;;;; Processes and timers

;;;; Input methods (lisp/romacs/vm/keyboard.el `romacs--read-char')

;;;; Mode line (lisp/romacs/romacs-mode-line.el)

(defvar scaling-mode-line-hook nil)

;;;; Redisplay (GUI: the host lays out each step's change)

;;;; Server clients and agent sessions (romacs-server.el, romacs-agent*.el)

(defun scaling--server ()
  "romacs-server.el's waiter index with its GNU advice, and no socket."
  (require 'romacs-server)
  (advice-add 'server-buffer-done :before #'romacs-server--buffer-done)
  (advice-add 'server-done :around #'romacs-server--done))

(defun scaling--agent ()
  "romacs-agent.el with a private state directory."
  (setenv "XDG_STATE_HOME" (make-temp-file "scaling-state-" t))
  (require 'romacs-agent))

;; The agent board over N projects of one session each, and over N roots in one project.
(defun scaling--agent-sessions (n project)
  (scaling--agent)
  (clrhash romacs-agent--sessions)
  (dotimes (i n)
    (let ((s (romacs-agent--make :id (format "%08d-proj" i) :harness 'claude :provider "claude"
                                 :title (format "session %d" i) :cwd (funcall project i)
                                 :last-time (+ 1e9 i) :state 'idle :turns 1)))
      (puthash (romacs-agent-id s) s romacs-agent--sessions)))
  (scaling--agent-board))

(defun scaling--agent-board ()
  "The agent board's buffer, not shown."
  (require 'romacs-agent-board)
  (romacs-agent-board--buffer))

;; The harnesses' history: N Claude Code transcripts in one configuration
;; directory, listed (stat only) once described, and listed and described anew.
(defun scaling--history (n)
  "A Claude Code configuration directory of N transcripts over 7 projects."
  (scaling--agent)
  (require 'romacs-agent-history)
  (let ((dir (make-temp-file "scaling-claude-" t)))
    (dotimes (i n)
      (let ((project (expand-file-name (format "projects/-tmp-project-%d" (% i 7)) dir)))
        (make-directory project t)
        (with-temp-file (expand-file-name (format "%08x-0000-4000-8000-%012x.jsonl" i i) project)
          (insert (format "{\"type\":\"user\",\"cwd\":\"/tmp/project-%d\",\"message\":{\"role\":\"user\",\"content\":\"task %d\"}}\n"
                          (% i 7) i)))))
    (setq romacs-agent-history-limit (max n 1)
          romacs-agent-history--descriptions (make-hash-table :test #'equal))
    dir))

(defmacro scaling--with-history-place (dir &rest body)
  "BODY with DIR as the only place history is kept."
  `(cl-letf (((symbol-function 'romacs-agent-history--places) (lambda () (list (list ,dir 'claude "claude" ,dir)))))
     ,@body))

;; The endpoint sweep over N sessions, N routes of sessions gone, N ended
;; editors' sockets and N earlier builds' directories: one pass each.
(defvar romacs-mcp--socket)
;; The history's places over N provider instances.
(defvar romacs-agent-providers)
;; The instances of N Lisp entries and T3's settings of N instances: one
;; read of the file, one name table, linear; read again (as after the file
;; changed) or kept from the last read.
(defun scaling--t3-instances (n)
  (scaling--agent)
  (require 'romacs-agent-t3)
  (let ((file (make-temp-file "scaling-t3-" nil ".json"))
        (instances (make-hash-table :test #'equal)))
    (dotimes (i n)
      (puthash (format "claudeAgent_%d" i)
               (romacs-mcp--object "driver" "claudeAgent" "displayName" (format "Claude %d" i)
                                   "config" (romacs-mcp--object "homePath" (format "/tmp/t3-claude-%d" i)
                                                                "launchArgs" "--chrome --x 'a b'"))
               instances))
    (with-temp-file file (insert (json-serialize (romacs-mcp--object "providerInstances" instances))))
    (cons (list 't3 :settings file)
          (cl-loop for i below n collect (cons (format "claude-lisp-%d" i) (format "/tmp/claude-config-%d" i))))))

;; T3's thread titles: a store of N threads read cold, read again after
;; its file changed (as its write-ahead log does), and kept (the read
;; skipped, 200 lookups).
(defun scaling--t3-store (n)
  (scaling--agent)
  (require 'romacs-agent-t3)
  (let* ((dir (make-temp-file "scaling-t3s-" t))
         (store (expand-file-name "statev2.sqlite" dir))
         (db (sqlite-open store)))
    (sqlite-execute db "create table orchestration_v2_projection_threads (thread_id text, title text)")
    (sqlite-execute db "create table orchestration_v2_projection_provider_threads (thread_id text, payload_json text, updated_at text, provider_instance_id text)")
    (sqlite-transaction db)
    (dotimes (i n)
      (sqlite-execute db "insert into orchestration_v2_projection_threads values (?, ?)" (list (format "t%d" i) (format "Title %d" i)))
      (sqlite-execute db "insert into orchestration_v2_projection_provider_threads values (?, ?, '1', 'claudeAgent')"
                      (list (format "t%d" i) (format "{\"nativeThreadRef\":{\"nativeId\":\"n%d\"}}" i))))
    (sqlite-commit db)
    (sqlite-close db)
    store))

;; A chat of N characters over a fixed 100 turns.
(defun scaling--big-chat (n)
  (scaling--agent)
  (let ((session (romacs-agent--make :id "scaling-big-chat" :harness 'claude :provider "claude" :title "big"
                                     :dir temporary-file-directory))
        (reply (concat (make-string (max 1 (/ n 100)) ?x) "\n")))
    (romacs-agent-show session)
    (dotimes (i 100)
      (let ((seq (* 10 i)) (id (format "m%d" i)))
        (romacs-agent-ui--render session (list :type 'input :text "a prompt" :seq (+ seq 1) :last t))
        (romacs-agent-ui--render session (list :type 'delta :text reply :message id :seq (+ seq 2) :last t))
        (romacs-agent-ui--render session (list :type 'done :duration 1.0 :seq (+ seq 3) :last t))))
    (romacs-agent-buffer session)))

;;;; Session images (lisp/romacs/warm-restart.el)

(defun scaling--session (n)
  "N text buffers, and a private image directory."
  (require 'warm-restart)
  (setq warm-restart-directory (make-temp-file "scaling-images-" t))
  (dolist (name (scaling--names "scaling-session" n))
    (with-current-buffer (get-buffer-create name) (text-mode) (insert name "\nsecond line\n"))))

;; N Hermes chats restored through Hermes's own desktop handler beside N
;; text buffers, then attached at activation (offline: no dashboard).
(defun scaling--offline-hermes (function)
  "Call FUNCTION with Hermes's dashboard transport off."
  (cl-letf (((symbol-function 'hermes-chat--dashboard-default-transport-p) #'ignore))
    (funcall function)))

;; Motion and measurement in one long wrapped line of N words, around its
;; middle: a step's rows are laid out from the line's start, so the work
;; is linear in the line; a rescan per row would make it quadratic.
(defun scaling--wrapped-line (n word-wrap)
  "Show one line of N words, wrapped at words when WORD-WRAP; return its buffer."
  (switch-to-buffer (get-buffer-create "scaling-wrapped")) (erase-buffer)
  (dotimes (i n) (insert (format "word%d " i)))
  (insert "\n")
  (setq-local truncate-lines nil)
  (if word-wrap (visual-line-mode 1) (setq-local word-wrap nil))
  (goto-char (/ (point-max) 2))
  (current-buffer))

(defun scaling--wrapped-step (i)
  "Move vertically from the middle, to the visual line's ends, and measure."
  (goto-char (/ (point-max) 2))
  (vertical-motion (if (cl-evenp i) 3 -3))
  (end-of-visual-line)
  (beginning-of-visual-line)
  (window-text-pixel-size nil (line-beginning-position) (point)))

;;;; Jabber: the archive catch-up's collections (lisp/romacs/jabber/jabber-mam.el,
;;;; jabber-mam-sync.el).  A connection here is an FSM symbol with state
;;;; data and no transport: the probes count bookkeeping, not traffic.

(defvar jabber-connections)
(defvar jabber-buffer-connection)
(defvar jabber-group)
(defvar jabber-mam--dirty-peers)
(defvar jabber-mam--room-nicks)
(defvar jabber-mam-sync--accounts)
(defvar jabber-mam-sync--stored)
(defvar jabber-mam-sync--merge-queue)
(defvar jabber-mam-sync--merge-tail)
(defvar jabber-mam-sync--merge-queued)
(defvar jabber-mam-sync--live-jobs)

(defun scaling--jabber ()
  "Jabber loaded, and a connection for romeo@montague.net, made current."
  (require 'jabber)
  (let ((jc (make-symbol "scaling-jc")))
    (put jc :state :session-established)
    (put jc :state-data (list :username "romeo" :server "montague.net" :resource "scaling"
                              :password "pw" :connection-type 'network))
    (setq jabber-connections (list jc))
    jc))

;; N distinct nicknames rendered on one background, each twice: the
;; colour cache (jabber-muc.el) computes each once.  kN stays under its
;; bound (`jabber-muc-nick-colors-limit', 4096), past which it starts over.
(defvar jabber-muc--nick-colors)
;; N archived results marking N/2 peers dirty, twice each, and their page's redraw.
(defvar jabber-mam--redraw-peers)
(defvar jabber-mam--redraw-timer)

;;;; Fontsets (fontsets.rs, romacs_layout_input::fontset)

;; Each edit publishes, so a snapshot holds what the next edit touches, as
;; one edit per redisplay does: N disjoint rules, N alternatives on one
;; range, N fontsets each found by name, and an edit and a lookup among N
;; unrelated rules, N unrelated fontsets.
(defvar scaling--fontset-serial 0)
(defun scaling--fontset-name ()
  (format "-misc-fixed-medium-r-normal--0-0-0-0-c-0-fontset-p%d"
          (setq scaling--fontset-serial (1+ scaling--fontset-serial))))

;;;; Images (prim/leaf/graphics.rs: the semantic image cache)

;; N distinct images, each measured, then all N again from the cache: a
;; lookup is one hash of the spec's copied plist, so the work is linear
;; in the images and a lookup never scans the cache.
(defun scaling--image-specs (n)
  "N distinct SVG image specs, a size each."
  (let (specs)
    (dotimes (i n)
      (push (list 'image :type 'svg
                  :data (format "<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"%d\" height=\"8\"/>" (1+ i)))
            specs))
    (nreverse specs)))

(defun scaling--image-buffer (n)
  "A buffer of N lines, each showing its own image beside text; return it."
  (switch-to-buffer (get-buffer-create "scaling-images")) (erase-buffer)
  (let ((i 0))
    (dolist (spec (scaling--image-specs n))
      (insert (format "line %d " i))
      (insert-image spec)
      (insert " text\n")
      (setq i (1+ i))))
  (goto-char (point-min))
  (current-buffer))

;;;; Attachments (lisp/romacs/romacs-attach.el)

(defun scaling--attach-files (n)
  "N small distinct PNG files in a temporary directory: their names."
  (let ((dir (make-temp-file "scaling-attach" t)) (files nil)
        (png (base64-decode-string "iVBORw0KGgoAAAANSUhEUgAAAAIAAAACCAIAAAD91JpzAAAAEElEQVR4nGP4z8AARAwQCgAf7gP9i18U1AAAAABJRU5ErkJggg==")))
    (dotimes (i n)
      (let ((file (expand-file-name (format "a%d.png" i) dir)))
        (let ((coding-system-for-write 'binary)) (write-region png nil file nil 'silent))
        (push file files)))
    (nreverse files)))

(defun scaling--attach-host (files)
  "The attach test host with FILES attached: its buffer."
  (romacs-attach-test-host)
  (romacs-attach-set-items nil)
  (erase-buffer)
  (when files (romacs-attach-files files))
  (current-buffer))

;;;; Collection

(defvar scaling--keeper nil
  "The cons each `gc-free-lists' round keeps above its garbage.")

(defun scaling--churn (n)
  "Garbage in every table: N conses, N/2 vectors and N/100 strings, N/10
closures over a mutable variable (each a closure and a cell)."
  (make-list n nil)
  (dotimes (_ (/ n 2)) (make-vector 1 nil))
  (dotimes (_ (/ n 100)) (make-string 16 ?x))
  (dotimes (i (/ n 10)) (let ((x i)) (lambda () (setq x (1+ x))))))

(defvar scaling--live nil
  "What the `gc-mark' and `gc-long-vector' probes keep alive.")

(defvar scaling--finalized 0
  "Finalizers the `gc-doomed-finalizers' collection has run.")

;; Code retirement (request_code.rs, Program::retire_code): N functions
;; compiled, lowered by a call and dropped in each step, each with a site
;; on a primitive and one on a leaf; the host's idle
;; pass between steps collects at the top level, the only place request
;; code retires, and the next step takes their sites and cache slots back.
;; A session no key has reached has no `romacs--idle-collect' (the lazy
;; lisp/romacs/vm/keyboard.el) and shows a startup message, so the probe supplies
;; the one and clears the other.  Each step waits (:ready) for a collection
;; after the last, however long a loaded host takes to reach one.  Retirement that scanned its free lists, or
;; rebuilt tables per function, would grow faster than N.
(defvar scaling--retire-collections 0
  "`gcs-done' when the last `gc-retired-code' step ended.")

;; Compiled `(lambda ...)' lists (source_cache.rs `lambda_lists'): what a
;; collection costs with N live entries, each list called once.  An entry,
;; its closure and the snapshot a call compares, is traced by the weak
;; round that finds its list live (gc.rs `weak_round'), and a dead list's
;; entry goes after the mark.  Every list here is live by the first round,
;; so the rounds do not grow with N: that each entry is traced once per
;; collection is `a_live_entry_is_traced_once_per_collection''s, in
;; crates/romacs-vm/src/source_cache/cache_tests.rs, not this probe's.
(defvar scaling--lambda-lists nil
  "The lists the `gc-lambda-lists' probe calls and keeps.")

;; The debugger's frame model: `backtrace-get-frames' (every frame with
;; its locals, as a *Backtrace* buffer is built) over a stack N deep with a
;; dynamic binding per frame.  One walk (`backtrace--frames-and-locals');
;; GNU's per-frame `backtrace--locals' would be quadratic.
(defvar scaling--bt-depth nil)
(defun scaling--bt-frames (n)
  (let ((scaling--bt-depth n))
    (if (> n 0) (scaling--bt-frames (1- n)) (length (backtrace-get-frames)))))
;; A snapshot of the stack (`romacs--inspect-frames') walks within a work
;; budget: the time is flat in the size of an argument, and in the depth of
;; the stack past the frames it keeps.
(defun scaling--inspect-value (_value) (romacs--inspect-frames))
(defun scaling--inspect-deep (n)
  (if (> n 0) (list (scaling--inspect-deep (1- n))) (romacs--inspect-frames)))
;; `save-excursion' records nested N deep, then `backtrace-eval' at the
;; outermost frame, whose unrewind swaps all N excursions and bindings and
;; back: linear in N.
(defun scaling--bt-excursions (n top)
  (let ((scaling--bt-depth n))
    (save-excursion
      (if (> n 0) (scaling--bt-excursions (1- n) top)
        (backtrace-eval '(list scaling--bt-depth (point)) top 'scaling--bt-excursions)))))
;; `backtrace-debug' flags: every frame of a stack N deep flags itself and
;; clears the flag before it returns, then N calls each leave theirs set
;; (whose tokens go stale once they return): O(log N) amortized a flag.
(defun scaling--bt-flag-deep (n)
  (backtrace-debug 0 t 'scaling--bt-flag-deep)
  (prog1 (if (> n 0) (scaling--bt-flag-deep (1- n)) 0)
    (backtrace-debug 0 nil 'scaling--bt-flag-deep)))
(defun scaling--bt-flag-leave () (backtrace-debug 0 t 'scaling--bt-flag-leave) nil)
(defun scaling--bt-flag-any (n)
  ;; Flags the N frames below the leaf newest to oldest, then clears them
  ;; oldest first: insertion and removal away from the end of the set.
  (if (> n 0) (scaling--bt-flag-any (1- n))
    (let ((depth (length (backtrace-frames 'scaling--bt-flag-any))))
      (dotimes (i depth) (backtrace-debug i t 'scaling--bt-flag-any))
      (dotimes (i depth) (backtrace-debug (- depth 1 i) nil 'scaling--bt-flag-any)))))
;; `backtrace-eval' over N buffer-local bindings whose locals are gone (each
;; becomes inert in place) below N live bindings: one unrewind, linear.
(defun scaling--bt-dead-inner () (backtrace-eval nil 1 'scaling--bt-dead-inner))
(defun scaling--bt-dead-outer (locals globals)
  (cl-progv locals (make-list (length locals) 1)
    (kill-all-local-variables)
    (cl-progv globals (make-list (length globals) 2)
      (scaling--bt-dead-inner))))
;;;; Running

(defun scaling--probe (name)
  (or (alist-get name scaling-probes) (error "No scaling probe %s" name)))

(defun scaling--print (plist)
  "Print PLIST as one SCALING line of JSON."
  (princ (format "SCALING %s\n" (json-encode plist))))

(defun scaling-list ()
  "Print each probe's parameters, one SCALING line each."
  (require 'json)
  (dolist (probe (reverse scaling-probes))
    (let ((plist (cdr probe)))
      (scaling--print (list :name (symbol-name (car probe)) :n (plist-get plist :n)
                            :k (or (plist-get plist :k) 4) :budget (or (plist-get plist :budget) 1.25)
                            :slope (plist-get plist :slope)
                            :known (or (plist-get plist :known) nil) :gui (if (plist-get plist :gui) t :json-false)
                            :steps (or (plist-get plist :steps) 1) :stack (if (plist-get plist :stack) t :json-false)
                            :idle (if (plist-get plist :idle) t :json-false))))))

(defun scaling--perf (command)
  "Send COMMAND to the `perf record --control' fifo SCALING_PERF_CTL names, if any.
scaling_live.py --record profiles a probe's body alone this way."
  (let ((fifo (getenv "SCALING_PERF_CTL")))
    (when fifo
      (call-process "sh" nil nil nil "-c" (format "printf '%s\\n' > %s" command (shell-quote-argument fifo)))
      (sleep-for 0.1))))

(defun scaling-run (name n)
  "Run batch probe NAME at size N: print its counted instructions."
  (require 'json)
  (let* ((plist (scaling--probe name))
         (it (funcall (plist-get plist :setup) n))
         (body (plist-get plist :body))
         (start nil) (end nil))
    (garbage-collect)
    (setq scaling-stack-marks nil)
    (scaling--perf "enable")
    (let ((gc-cons-threshold most-positive-fixnum) (gc-cons-percentage 1.0))
      (setq start (romacs-instructions))
      (funcall body n it)
      (setq end (romacs-instructions)))
    (scaling--perf "disable")
    (scaling--print (list :name (symbol-name name) :n n
                          :instructions (if (and start end) (- end start) nil)
                          :stack (vconcat (nreverse scaling-stack-marks))))))

(defun scaling--quiet ()
  "Hide the toasts start-up left (the startup message's): one hides
`romacs-layer-toast-timeout' after it showed, which on a quick host lands
in the first steps, redisplays its window in each of them and paints once
as it goes.  Called before a setup: the toasts a setup makes are what its
probe measures (`toasts-clear', `toasts-follow')."
  (when (fboundp 'romacs-layer-toast-clear) (romacs-layer-toast-clear)))

(defun scaling--mark (plist)
  "Append PLIST as a JSON line to stderr, among the host's paint diagnostics."
  (write-region (concat (json-encode plist) "\n") nil "/dev/stderr" t 'silent))

(defun scaling--differences (readings)
  "The differences of consecutive READINGS as a vector, nil where one is nil."
  (vconcat (cl-mapcar (lambda (before after) (and before after (- after before))) readings (cdr readings))))

(defun scaling-gui-start (name n out)
  "Run GUI probe NAME at size N in this session, then exit.
Once the session is idle, set the probe up; then run its steps from a chain
of timers, each after the host's redisplay of the step before: a paint still
being drawn holds the next one back, so no fixed time means the step was
laid out.  Count the instructions from before the first step to after the
last step's layout; write (:name :n :instructions), and for an :idle probe
each step's instructions (:steps), to OUT as JSON and kill Emacs.  Stderr marks the start, each step and the end among the paint
diagnostics, so the harness can check a paint was submitted after each
step, before the next step or the count's end."
  (require 'json)
  (let* ((plist (scaling--probe name))
         (steps (or (plist-get plist :steps) 1))
         (body (plist-get plist :body))
         (gap (or (plist-get plist :gap) 0.1))
         (idle (plist-get plist :idle))
         (ready (or (plist-get plist :ready) #'always)) it start readings)
    (setq blink-cursor-mode nil)
    (run-with-idle-timer
     0.5 nil
     (lambda ()
       (scaling--quiet)
       (setq it (funcall (plist-get plist :setup) n))
       (garbage-collect)
       (run-with-timer
        0.5 nil
        (lambda ()
          (scaling--mark (list :scaling "start"))
          (setq start (romacs-instructions))
          (let ((step nil) (laid-out nil) (i 0))
            (setq laid-out (lambda ()
                             (if (and (romacs--redisplay-since-mark) (funcall ready))
                                 (funcall step)
                               (run-with-timer gap nil laid-out))))
            (setq step (lambda ()
                         ;; An :idle probe's steps are counted one by one.
                         (when idle (push (romacs-instructions) readings))
                         (if (< i steps)
                             (progn (scaling--mark (list :scaling "step" :i i))
                                    (funcall body n it i) (setq i (1+ i))
                                    (romacs--redisplay-since-mark t)
                                    (run-with-timer gap nil laid-out))
                           (let ((end (romacs-instructions)))
                             (scaling--mark (list :scaling "end"))
                             (with-temp-file out
                               (insert (json-encode
                                        `(:name ,(symbol-name name) :n ,n
                                          :instructions ,(and start end (- end start))
                                          ,@(and idle (list :steps (scaling--differences (nreverse readings))))))))
                             (kill-emacs 0)))))
            (funcall step))))))))

;; Every probe lives in probes/NAME.el beside this file.
(dolist (file (directory-files
               (expand-file-name "probes" (file-name-directory (or load-file-name buffer-file-name)))
               t "\\.el\\'"))
  (load file nil t))

(provide 'scaling-probes)
;;; probes.el ends here
