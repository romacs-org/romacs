;;; romacs-agent-cwd-tests.el --- A task whose directory is gone -*- lexical-binding: t; -*-
;; SPDX-License-Identifier: GPL-3.0-or-later
;; Maintainer: Thanos Apollo <public@thanosapollo.org>
(require 'ert)
(require 'cl-lib)
(require 'romacs-mcp)

;; Loading romacs-agent installs romacs-agent-tools: the endpoint's
;; registrations are left as they were.
(let ((romacs-mcp-call-function romacs-mcp-call-function)
      (romacs-mcp-tools (copy-alist romacs-mcp-tools)))
  (require 'romacs-agent)
  (require 'romacs-agent-claude)
  (require 'romacs-agent-codex))

(defconst romacs-agent-cwd-tests--id "cccccccc-0000-4000-8000-000000000000")

(defmacro romacs-agent-cwd-tests--with (&rest body)
  "BODY with `root' a directory removed after, `home' a Claude Code
configuration directory under it, `gone' a task directory under `parent',
which exists, and `session' a Claude session last run in `gone', which
no longer does; `notes' collects what it journals."
  (declare (indent 0))
  `(let* ((root (file-truename (make-temp-file "romacs-cwd-" t)))
          (home (expand-file-name "claude" root))
          (parent (expand-file-name "repo" root))
          (gone (expand-file-name "agent-1" parent))
          (session (romacs-agent--make :id romacs-agent-cwd-tests--id :harness 'claude :config-dir home
                                       :cwd gone :dir (expand-file-name "session" root) :turns 1))
          (notes nil))
     (make-directory parent t)
     (unwind-protect
         (cl-letf (((symbol-function 'romacs-agent--note) (lambda (_session note) (push note notes))))
           ,@body)
       (delete-directory root t))))

(defun romacs-agent-cwd-tests--project (home directory &optional name)
  "DIRECTORY's Claude Code project under HOME, or NAME in it."
  (let ((project (expand-file-name (concat "projects/" (romacs-agent--claude-project directory)) home)))
    (if name (expand-file-name name project) project)))

(defun romacs-agent-cwd-tests--write (file text)
  (make-directory (file-name-directory file) t)
  (with-temp-file file (insert text)))

(defun romacs-agent-cwd-tests--read (file)
  (with-temp-buffer (insert-file-contents file) (buffer-string)))

(defun romacs-agent-cwd-tests--conversation (home directory text)
  "A transcript of TEXT, and a tool result beside it, in DIRECTORY's project."
  (let ((id romacs-agent-cwd-tests--id))
    (romacs-agent-cwd-tests--write (romacs-agent-cwd-tests--project home directory (concat id ".jsonl")) text)
    (romacs-agent-cwd-tests--write (romacs-agent-cwd-tests--project home directory (concat id "/result.txt")) text)))

;;; The pure parts

(ert-deftest romacs-agent-cwd-nearest-directory ()
  "The directory itself while it is there, else its nearest parent that
is, else the root."
  (let ((there '("/a" "/a/b/c")))
    (cl-flet ((exists (dir) (member dir there)))
      (should (equal (romacs-agent--nearest-directory "/a/b/c/" #'exists) "/a/b/c"))
      (should (equal (romacs-agent--nearest-directory "/a/b/c/d/e" #'exists) "/a/b/c"))
      (should (equal (romacs-agent--nearest-directory "/a/x/y" #'exists) "/a"))
      (should (equal (romacs-agent--nearest-directory "/z/y" #'ignore) "/")))))

(ert-deftest romacs-agent-cwd-note ()
  "The note names both directories and ends its line: the message it
leads starts on a line of its own.  Lost, it says the conversation did
not follow."
  (let ((note (romacs-agent--cwd-note "/w/repo/agent-1" "/w/repo" nil))
        (lost (romacs-agent--cwd-note "/w/repo/agent-1" "/w/repo" t)))
    (should (string-search "/w/repo/agent-1 no longer exists" note))
    (should (string-search "runs from /w/repo " note))
    (should (string-suffix-p "\n" note))
    (should-not (string-search "could not follow" note))
    (should (string-search "could not follow" lost))
    (should (string-suffix-p "\n" lost))))

(ert-deftest romacs-agent-cwd-with-note ()
  "A note goes after a brief waiting, once."
  (should (equal (romacs-agent--with-note nil "n\n") "n\n"))
  (should (equal (romacs-agent--with-note "Renewed.\n\n" "n\n") "Renewed.\n\nn\n"))
  (should (equal (romacs-agent--with-note "Renewed.\n\nn\n" "n\n") "Renewed.\n\nn\n")))

(ert-deftest romacs-agent-cwd-claude-project ()
  "Claude Code's project of a directory: every character but an ASCII
letter or digit a hyphen, as its projects/ directory names show."
  (should (equal (romacs-agent--claude-project "/home/u/.cache/x_y.z/")
                 "-home-u--cache-x-y-z")))

;;; Moving

(ert-deftest romacs-agent-cwd-rehome ()
  "A Claude session whose directory is gone runs from its nearest
parent: the transcript and the directory beside it in the old
directory's project are copied into the parent's, whatever other copy
is newer, the originals left as they were; its next message carries the
note, and the move is journaled, once."
  (romacs-agent-cwd-tests--with
    (let ((id romacs-agent-cwd-tests--id))
      (romacs-agent-cwd-tests--conversation home gone "old project\n")
      ;; Newer, elsewhere: not its conversation as run in GONE.
      (romacs-agent-cwd-tests--conversation home root "elsewhere\n")
      (set-file-times (romacs-agent-cwd-tests--project home gone (concat id ".jsonl")) (time-subtract nil 600))
      (romacs-agent--rehome session)
      (should (equal (romacs-agent-cwd session) parent))
      (should (equal (romacs-agent-brief session) (romacs-agent--cwd-note gone parent nil)))
      (should (equal (mapcar (lambda (n) (list (gethash "type" n) (gethash "from" n) (gethash "to" n) (gethash "lost" n)))
                             notes)
                     (list (list "cwd" gone parent :false))))
      (should (equal (romacs-agent-cwd-tests--read (romacs-agent-cwd-tests--project home parent (concat id ".jsonl")))
                     "old project\n"))
      (should (equal (romacs-agent-cwd-tests--read (romacs-agent-cwd-tests--project home parent (concat id "/result.txt")))
                     "old project\n"))
      (should (equal (romacs-agent-cwd-tests--read (romacs-agent-cwd-tests--project home gone (concat id ".jsonl")))
                     "old project\n"))
      ;; Its directory there now: nothing more.
      (romacs-agent--rehome session)
      (should (= (length notes) 1))
      (should (equal (romacs-agent-brief session) (romacs-agent--cwd-note gone parent nil))))))

(ert-deftest romacs-agent-cwd-resume-refuses-a-missing-worktree ()
  (romacs-agent-cwd-tests--with
    (let ((branch "agent/isolated"))
      (setf (romacs-agent-options session) (list :worktree gone :branch branch))
      (cl-letf (((symbol-function 'romacs-agent--check-resume) #'ignore)
                ((symbol-function 'romacs-agent--launch-when-placed) #'ignore))
        (let ((text (error-message-string (should-error (romacs-agent-resume session) :type 'user-error))))
          (should (string-search gone text))
          (should (string-search branch text))))
      (should (equal (romacs-agent-cwd session) gone))
      (should-not notes))))

(ert-deftest romacs-agent-cwd-rehome-keeps-brief ()
  "A brief already waiting (a renewal's) keeps its place: the note
follows it.  Codex files no conversation by directory: nothing copied."
  (let* ((root (file-truename (make-temp-file "romacs-cwd-" t)))
         (gone (expand-file-name "gone" root))
         (session (romacs-agent--make :id "dddddddd-0000-4000-8000-000000000000" :harness 'codex
                                      :cwd gone :brief "Renewed.\n\n" :dir (expand-file-name "session" root))))
    (unwind-protect
        (cl-letf (((symbol-function 'romacs-agent--note) #'ignore))
          (romacs-agent--rehome session)
          (should (equal (romacs-agent-cwd session) root))
          (should (equal (romacs-agent-brief session)
                         (concat "Renewed.\n\n" (romacs-agent--cwd-note gone root nil)))))
      (delete-directory root t))))

(ert-deftest romacs-agent-cwd-copy-fails-then-retries ()
  "A copy that fails leaves nothing in the new project and the session
where it was; the next resume copies it whole.  A copy already there is
whole: kept, never written over or merged into."
  (romacs-agent-cwd-tests--with
    (let* ((id romacs-agent-cwd-tests--id)
           (transcript (romacs-agent-cwd-tests--project home parent (concat id ".jsonl")))
           (beside (romacs-agent-cwd-tests--project home parent id))
           (copy (symbol-function 'copy-file)))
      (romacs-agent-cwd-tests--conversation home gone "whole\n")
      (cl-letf (((symbol-function 'copy-file)
                 (lambda (&rest args)
                   ;; A partial copy, then the disk is full.
                   (with-temp-file (nth 1 args) (insert "part"))
                   (signal 'file-error (list "Writing" "No space left on device")))))
        (should-error (romacs-agent--rehome session) :type 'file-error))
      (should (equal (romacs-agent-cwd session) gone))
      (should-not (romacs-agent-brief session))
      (should-not notes)
      (should-not (file-exists-p transcript))
      (should (equal (directory-files (romacs-agent-cwd-tests--project home parent) nil "\\`[^.]") nil))
      (should (equal (romacs-agent-cwd-tests--read (romacs-agent-cwd-tests--project home gone (concat id ".jsonl")))
                     "whole\n"))
      ;; A copy beside it made whole meanwhile: kept as it is.
      (romacs-agent-cwd-tests--write (expand-file-name "mine.txt" beside) "mine")
      (cl-letf (((symbol-function 'copy-file) copy))
        (romacs-agent--rehome session))
      (should (equal (romacs-agent-cwd session) parent))
      (should (equal (romacs-agent-cwd-tests--read transcript) "whole\n"))
      (should (equal (directory-files beside nil "\\`[^.]") '("mine.txt")))
      (should (= (length notes) 1)))))

(ert-deftest romacs-agent-cwd-long-project-lost ()
  "A directory whose project name Claude Code would cut and hash: no
copy is made, and the note says the conversation could not follow."
  (romacs-agent-cwd-tests--with
    ;; Just past Claude Code's bound, whatever ROOT's length: a fixed
    ;; padding under a long TMPDIR makes a name the file system refuses.
    (let* ((pad (max 2 (- (+ romacs-agent--claude-project-max 4)
                          (length (romacs-agent--claude-project root)))))
           (long (expand-file-name (make-string (- pad (/ pad 2)) ?l)
                                   (expand-file-name (make-string (/ pad 2) ?k) root)))
           (gone (expand-file-name "agent-1" long)))
      (make-directory long t)
      (setf (romacs-agent-cwd session) gone)
      (romacs-agent-cwd-tests--conversation home gone "whole\n")
      (should (> (length (romacs-agent--claude-project long)) romacs-agent--claude-project-max))
      (should (eq (romacs-agent--claude-cwd-copies home session (list gone) long) 'lost))
      (romacs-agent--rehome session)
      (should (equal (romacs-agent-cwd session) long))
      (should (equal (romacs-agent-brief session) (romacs-agent--cwd-note gone long t)))
      (should (eq (gethash "lost" (car notes)) t))
      (should-not (file-exists-p (romacs-agent-cwd-tests--project home long)))
      (should-not (member "--resume" (romacs-agent-cwd-tests--claude-argv session))))))

;;; Its conversation where Claude Code looks, or begun anew

(defun romacs-agent-cwd-tests--claude-argv (session)
  "SESSION's Claude Code command line as its resume makes it."
  (make-directory (romacs-agent-dir session) t)
  (romacs-agent--claude-argv session (romacs-agent--conversation-p session)))

(ert-deftest romacs-agent-cwd-symlinked-directory ()
  "A directory recorded through a link: Claude Code filed the
conversation under the path it resolves to, which is where its copy is
taken from."
  (romacs-agent-cwd-tests--with
    (let ((id romacs-agent-cwd-tests--id)
          (link (expand-file-name "link" root)))
      (make-symbolic-link parent link)
      (setf (romacs-agent-cwd session) (expand-file-name "agent-1" link))
      (romacs-agent-cwd-tests--conversation home gone "physical\n")
      (romacs-agent--rehome session)
      (should (equal (romacs-agent-cwd session) parent))
      (should (equal (romacs-agent-cwd-tests--read (romacs-agent-cwd-tests--project home parent (concat id ".jsonl")))
                     "physical\n"))
      (should (eq (gethash "lost" (car notes)) :false))
      (should (equal (romacs-agent--native-id session) id))
      (should (member "--resume" (romacs-agent-cwd-tests--claude-argv session))))))

(ert-deftest romacs-agent-cwd-lost-begins-anew ()
  "A conversation found under no name of its directory begins anew where
the task now runs, under a new id its manifest names: its command line
resumes none, the note says so, and the journal names the conversation
it leaves.  Its journal note keeps that across a restart."
  (romacs-agent-cwd-tests--with
    (let ((id romacs-agent-cwd-tests--id)
          (manifest (expand-file-name "manifest.json" (romacs-agent-dir session))))
      (romacs-agent-cwd-tests--write manifest (format "{\"id\":%S,\"cwd\":%S,\"harnessSession\":null}" id gone))
      (romacs-agent--rehome session)
      (should (equal (romacs-agent-cwd session) parent))
      (should (eq (gethash "lost" (car notes)) t))
      (should (equal (gethash "conversation" (car notes)) id))
      (should (equal (gethash "anew" (car notes)) (romacs-agent--native-id session)))
      (let ((kept (json-parse-string (romacs-agent-cwd-tests--read manifest))))
        (should (equal (gethash "harnessSession" kept) (romacs-agent--native-id session)))
        (should (equal (gethash "cwd" kept) parent)))
      (should (equal (romacs-agent-brief session) (romacs-agent--cwd-note gone parent t)))
      (let ((native (romacs-agent--native-id session))
            (argv (romacs-agent-cwd-tests--claude-argv session)))
        (should-not (equal native id))
        (should (romacs-agent--uuid-p native))
        (should-not (member "--resume" argv))
        (should (equal (cadr (member "--session-id" argv)) native)))
      ;; Replayed, its note begins it anew again, until a prompt.
      (setf (romacs-agent-driver-state session) nil)
      (romacs-agent--record session (json-parse-string
                                     (format "{\"seq\":1,\"note\":{\"type\":\"cwd\",\"from\":%S,\"to\":%S,\"lost\":true}}"
                                             gone parent)))
      (should (eq (romacs-agent--claude-head session) 'fresh)))))

(ert-deftest romacs-agent-cwd-lost-replayed-keeps-new-id ()
  "A journal folded whole, an earlier run's init naming the old
conversation and then a move that began a new one: its next launch
begins the new one, never the old."
  (romacs-agent-cwd-tests--with
    (let* ((id romacs-agent-cwd-tests--id)
           (anew "eeeeeeee-0000-4000-8000-000000000000")
           (manifest (expand-file-name "manifest.json" (romacs-agent-dir session))))
      (romacs-agent-cwd-tests--write manifest (format "{\"id\":%S,\"harnessSession\":%S}" id anew))
      (setf (romacs-agent-cwd session) parent (romacs-agent-harness-session session) anew)
      (romacs-agent--record session (json-parse-string
                                     (format "{\"seq\":1,\"out\":{\"type\":\"system\",\"subtype\":\"init\",\"session_id\":%S,\"model\":\"m\"}}" id)))
      (should (equal (romacs-agent--native-id session) id))
      (romacs-agent--record session (json-parse-string
                                     (format "{\"seq\":2,\"note\":{\"type\":\"cwd\",\"from\":%S,\"to\":%S,\"lost\":true,\"conversation\":%S,\"anew\":%S}}"
                                             gone parent id anew)))
      (let ((argv (romacs-agent-cwd-tests--claude-argv session)))
        (should-not (member "--resume" argv))
        (should (equal (cadr (member "--session-id" argv)) anew)))
      (should (equal (gethash "harnessSession" (json-parse-string (romacs-agent-cwd-tests--read manifest))) anew)))))

(ert-deftest romacs-agent-cwd-lost-fork-begins-anew ()
  "A fork not yet spoken to, its directory gone and its conversation lost,
begins anew under a new id: it forks no source, resumes nothing."
  (romacs-agent-cwd-tests--with
    (let ((source "ffffffff-0000-4000-8000-000000000000"))
      (setf (romacs-agent-options session) (list :fork source) (romacs-agent-turns session) 0)
      (should (romacs-agent--claude-forking session))
      (romacs-agent--rehome session)
      (should (eq (gethash "lost" (car notes)) t))
      (let ((argv (romacs-agent-cwd-tests--claude-argv session)))
        (should-not (member "--resume" argv))
        (should-not (member "--fork-session" argv))
        (should-not (member source argv))
        (should (equal (cadr (member "--session-id" argv)) (romacs-agent--native-id session)))
        (should-not (equal (romacs-agent--native-id session) romacs-agent-cwd-tests--id))))))

;;; Not written to a harness in a directory gone

(ert-deftest romacs-agent-cwd-steer-waits ()
  "A steer into a turn under way goes to it while its directory is
there; gone, it waits for the turn's end, never written to that run."
  (romacs-agent-cwd-tests--with
    (let ((item (romacs-agent-item "say hi" :kind 'now)))
      (setf (romacs-agent-item-turn item) 3 (romacs-agent-state session) 'running (romacs-agent-turns session) 3
            (romacs-agent-queue session) (list item) (romacs-agent-cwd session) parent)
      (cl-letf (((symbol-function 'romacs-agent--open-p) (lambda (_session _token) t))
                ((symbol-function 'romacs-agent--steerable-p) (lambda (_session) t)))
        (should (eq (romacs-agent--next-item session) item))
        (setf (romacs-agent-cwd session) gone)
        (should-not (romacs-agent--next-item session))
        (should-not (romacs-agent--stranded-p session))
        ;; Nor a renewal's continuation.
        (setf (romacs-agent-queue session) nil)
        (cl-letf (((symbol-function 'romacs-agent--renewal-item) (lambda (_session) item)))
          (should-not (romacs-agent--next-item session))
          (setf (romacs-agent-cwd session) parent)
          (should (eq (romacs-agent--next-item session) item)))))))

(ert-deftest romacs-agent-cwd-stranded-relaunch ()
  "A message waiting for a live harness whose directory is gone runs it
again instead of going to it, never cutting a turn under way: the turn
ends first.  A directory there, or nothing waiting, changes nothing."
  (let* ((root (make-temp-file "romacs-cwd-" t))
         (session (romacs-agent--make :id "ffffffff-0000-4000-8000-000000000000" :harness 'claude
                                      :state 'running :cwd (expand-file-name "gone" root)))
         (relaunched 0) (sent 0))
    (unwind-protect
        (cl-letf (((symbol-function 'romacs-agent--open-p) (lambda (_session _token) t))
                  ((symbol-function 'romacs-agent--relaunch) (lambda (_session) (cl-incf relaunched)))
                  ((symbol-function 'romacs-agent--dispatch-one) (lambda (_session) (cl-incf sent) nil)))
          (romacs-agent--dispatch session)
          (should (= relaunched 0))
          (setf (romacs-agent-queue session) (list (romacs-agent-item "say hi")))
          (romacs-agent--dispatch session)
          (should (= relaunched 0))
          (setf (romacs-agent-state session) 'idle)
          (romacs-agent--dispatch session)
          (should (= relaunched 1))
          (should (= sent 2))
          (setf (romacs-agent-cwd session) root)
          (romacs-agent--dispatch session)
          (should (= relaunched 1))
          (should (= sent 3)))
      (delete-directory root t))))

;;; The relaunch

(defmacro romacs-agent-cwd-tests--relaunching (stop &rest body)
  "BODY with `session' attached and a message waiting, its relaunch's
stop STOP (a function of the session and what follows the stop), and
`launches' counting its launches."
  (declare (indent 1))
  `(romacs-agent-cwd-tests--with
     (setf (romacs-agent-connection session) (make-pipe-process :name "romacs-agent-cwd-test" :noquery t)
           (romacs-agent-caught-up session) t
           (romacs-agent-driver-state session) (list :ready t))
     (make-directory (romacs-agent-dir session) t)
     (process-put (romacs-agent-connection session) 'romacs-agent session)
     (setf (romacs-agent-queue session) (list (romacs-agent-item "say hi")) (romacs-agent-cwd session) parent)
     (let ((launches 0))
       (cl-letf (((symbol-function 'romacs-agent--stop-then) ,stop)
                 ((symbol-function 'romacs-agent--check-resume) #'ignore)
                 ((symbol-function 'romacs-agent--reattach-later) #'ignore)
                 ((symbol-function 'romacs-agent--launch-when-placed)
                  (lambda (session _launch)
                    (remhash (romacs-agent-id session) romacs-agent--resuming)
                    (cl-incf launches))))
         ,@body))))

(ert-deftest romacs-agent-cwd-relaunch-settled ()
  "A settled task given a message goes on: its relaunch resumes it."
  (let ((then nil))
    (romacs-agent-cwd-tests--relaunching (lambda (_session next) (setq then next))
      (setf (romacs-agent-settled-seq session) romacs-agent--inactive-settlement)
      (should (romacs-agent-settled-p session))
      (romacs-agent--relaunch session)
      (delete-process (romacs-agent-connection session))
      (funcall then)
      (should (= launches 1)))))

(ert-deftest romacs-agent-cwd-relaunch-cancelled ()
  "A task cancelled while its relaunch stops it is not resumed."
  (let ((then nil))
    (romacs-agent-cwd-tests--relaunching (lambda (_session next) (setq then next))
      (romacs-agent--relaunch session)
      (setf (romacs-agent-cancelled session) t)
      (delete-process (romacs-agent-connection session))
      (funcall then)
      (should (= launches 0)))))

(ert-deftest romacs-agent-cwd-relaunch-resumes-once ()
  "A detached run's stop ends its attachment, whose end would resume a
stopped session with a message waiting, and the relaunch resumes it too:
one resume, after what the relaunch runs first."
  (let ((before 0))
    (romacs-agent-cwd-tests--relaunching
        (lambda (session next)
          (let ((connection (romacs-agent-connection session)))
            (romacs-agent--set-stopped session t)
            (delete-process connection)
            (romacs-agent--sentinel connection "killed\n")
            (funcall next)))
      (romacs-agent--relaunch session (lambda () (cl-incf before)))
      (should (= before 1))
      (should (= launches 1))
      ;; A later look of the relaunch's finds it resumed.
      (romacs-agent--resume-when-gone session 0)
      (should (= launches 1)))))

;;; The note across a restart

(ert-deftest romacs-agent-cwd-note-replayed ()
  "Replayed, a move's note waits again for the next message, once, until
an input takes it; folded live, it leaves the brief to the move."
  (romacs-agent-cwd-tests--with
    (let ((note (lambda (seq)
                  (json-parse-string
                   (format "{\"seq\":%d,\"note\":{\"type\":\"cwd\",\"from\":%S,\"to\":%S,\"lost\":false}}"
                           seq gone parent))))
          (expected (romacs-agent--cwd-note gone parent nil)))
      (romacs-agent--record session (funcall note 1))
      (should (equal (romacs-agent-brief session) expected))
      (romacs-agent--record session (funcall note 2))
      (should (equal (romacs-agent-brief session) expected))
      (romacs-agent--record-echo session (json-parse-string "{\"romacsBrief\":true}"))
      (should-not (romacs-agent-brief session))
      (setf (romacs-agent-caught-up session) t)
      (romacs-agent--record session (funcall note 3))
      (should-not (romacs-agent-brief session)))))

;;; romacs-agent-cwd-tests.el ends here
