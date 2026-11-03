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
;;   :known  a ticket naming a known super-linear cost: the probe still
;;           runs and reports, but over budget it is debt, not a failure.
;;   :gap    a GUI probe's seconds between steps (default 0.1).
;;   :stack  also report how much native stack the body keeps
;;           (`romacs-native-stack-room' at `scaling-stack-mark' calls).
;;
;; Batch: romacs -Q --batch -l probes.el --eval '(scaling-run (quote NAME) N)'
;; prints one line, SCALING <json>.  `scaling-list' prints the probes.

;;; Code:

(require 'cl-lib)

(defvar scaling-probes nil
  "The probes, newest first: (NAME . PLIST).")

(defmacro scaling-probe (name &rest plist)
  "Define probe NAME from PLIST, its :setup and :body forms made functions."
  (let ((setup (plist-get plist :setup)) (body (plist-get plist :body)))
    `(setf (alist-get ',name scaling-probes)
           (list ,@(cl-loop for (key value) on plist by #'cddr
                            unless (memq key '(:setup :body)) append (list key value))
                 :setup (lambda (n) (ignore n) ,setup)
                 :body (lambda (n it &optional i) (ignore n it i) ,body)))))

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

;;;; Faces

(scaling-probe faces-defface :n 200
  :known "ticket faces-defface: custom-add-to-group is GNU's member + nconc over the whole group"
  :setup (mapcar #'intern (scaling--names "scaling-face" n))
  :body (dolist (face it) (custom-declare-face face '((t :weight bold)) "A scaling face." :group 'scaling)))

(scaling-probe faces-set-attribute :n 200
  :setup (let ((faces (mapcar #'intern (scaling--names "scaling-face" n))))
           (dolist (face faces) (custom-declare-face face '((t)) "A scaling face." :group 'scaling))
           faces)
  :body (let ((k 0))
          (dolist (face it)
            (set-face-attribute face nil :foreground (if (cl-oddp k) "red" "blue") :weight 'bold)
            (setq k (1+ k)))))

;; A theme application over N more package faces: loading and enabling
;; modus-operandi in a -Q window, ~1.3 G instructions, most of it loading
;; the theme file; nearly flat in the faces, which only the root frame
;; recomputes (VM2-CHILD-FRAME).
(scaling-probe faces-theme :gui t :n 50 :steps 1
  :setup (let ((faces (mapcar #'intern (scaling--names "scaling-face" n))))
           (dolist (face faces)
             (custom-declare-face face '((t :inherit font-lock-keyword-face)) "A package face." :group 'scaling))
           faces)
  :body (load-theme 'modus-operandi t))

(scaling-probe faces-edit-read :gui t :n 100 :steps 1
  :setup (let ((faces (mapcar #'intern (scaling--names "scaling-face" n))))
           (dolist (face faces) (custom-declare-face face '((t)) "A scaling face." :group 'scaling))
           faces)
  :body (dolist (face it)
          (set-face-attribute face nil :foreground "red")
          (face-attribute face :foreground)))

;;;; Buffers, markers, overlays, text properties, undo

(scaling-probe buffers :n 300
  :known "ticket buffers: creating or killing a buffer copies and rebuilds the whole buffer order list"
  :setup (scaling--names "scaling-buffer" n)
  :body (progn
          (dolist (name it) (get-buffer-create name))
          (dolist (name it) (get-buffer name))
          (length (buffer-list))
          (dolist (name it) (kill-buffer name))))

(scaling-probe markers :n 2000
  :setup (with-current-buffer (scaling--lines-buffer " scaling-markers" 200) (current-buffer))
  :body (with-current-buffer it
          (let ((size (buffer-size)) markers)
            (dotimes (i n) (push (copy-marker (1+ (% (* i 7919) size))) markers))
            (dotimes (i 100) (goto-char (1+ (% (* i 104729) (buffer-size)))) (insert "x"))
            (dolist (m markers) (marker-position m))
            (dolist (m markers) (set-marker m nil)))))

(scaling-probe overlays :n 1000
  :setup (scaling--lines-buffer " scaling-overlays" n)
  :body (with-current-buffer it
          (let (overlays)
            (goto-char (point-min))
            (dotimes (_ n)
              (let ((o (make-overlay (point) (line-end-position))))
                (overlay-put o 'face 'bold)
                (push o overlays))
              (forward-line 1))
            (goto-char (point-min))
            (dotimes (_ n)
              (overlays-at (point))
              (overlays-in (point) (line-end-position))
              (forward-line 1))
            (dolist (o overlays) (move-overlay o (overlay-start o) (1+ (overlay-start o))))
            (mapc #'delete-overlay overlays))))

(scaling-probe text-properties :n 2000
  :setup (scaling--lines-buffer " scaling-props" n)
  :body (with-current-buffer it
          (goto-char (point-min))
          (dotimes (i n)
            (put-text-property (point) (line-end-position) 'face (if (cl-oddp i) 'bold 'italic))
            (forward-line 1))
          (goto-char (point-min))
          (dotimes (_ n) (get-text-property (point) 'face) (forward-line 1))
          (let ((pos (point-min)))
            (while (setq pos (next-single-property-change pos 'face))))))

(scaling-probe undo :n 2000
  :setup (with-current-buffer (get-buffer-create " scaling-undo")
           (buffer-enable-undo)
           (current-buffer))
  :body (with-current-buffer it
          (dotimes (_ n) (insert "x") (undo-boundary))
          (undo-start)
          (undo-more n)))

;;;; Windows

(scaling-probe windows :n 6
  :setup (progn
           (setq window-min-height 1 window-min-width 2)
           (delete-other-windows)
           nil)
  :body (progn
          (dotimes (i n)
            (split-window (get-largest-window) nil (if (cl-oddp i) 'right 'below)))
          (let ((count 0)) (walk-windows (lambda (_) (setq count (1+ count)))) count)
          (dolist (w (window-list)) (ignore-errors (window-resize w 1 nil)))
          (balance-windows)
          (delete-other-windows)))

;;;; Symbols, hash tables, features

(scaling-probe obarray-intern :n 20000
  :setup (scaling--names "scaling-symbol" n)
  :body (progn (dolist (name it) (intern name)) (dolist (name it) (intern-soft name))))

(scaling-probe hash-equal :n 20000
  :setup (scaling--names "scaling-key" n)
  :body (let ((table (make-hash-table :test #'equal)))
          (dolist (key it) (puthash key t table))
          (dolist (key it) (gethash key table))
          (dolist (key it) (remhash key table))))

(scaling-probe hash-eq :n 50000
  :setup nil
  :body (let ((table (make-hash-table :test #'eq)))
          (dotimes (i n) (puthash i i table))
          (dotimes (i n) (gethash i table))
          (dotimes (i n) (remhash i table))))

(scaling-probe features :n 300
  :known "ticket features: featurep and provide are GNU's memq over the features list"
  :setup (mapcar #'intern (scaling--names "scaling-feature" n))
  :body (dolist (feature it) (provide feature) (featurep feature) (require feature)))

(scaling-probe load-files :n 50
  :setup (mapcar (lambda (i) (scaling--file 1 (lambda (_) `(provide ',(intern (format "scaling-file-%d" i))))))
                 (number-sequence 1 n))
  :body (dolist (file it) (load file nil t)))

;;;; Loading and evaluating many forms

(scaling-probe load-forms :n 12500 :stack t
  :setup (scaling--file n #'scaling--defun-form)
  :body (load it nil t))

(scaling-probe eval-buffer-forms :n 12500 :stack t
  :setup (let ((file (scaling--file n #'scaling--defun-form)))
           (with-current-buffer (get-buffer-create " scaling-eval-buffer")
             (insert-file-contents file)
             (current-buffer)))
  :body (eval-buffer it))

;;;; Completion

(defmacro scaling--completion-probe (name style &optional library)
  `(scaling-probe ,name :n 2000
     :setup (progn (require 'minibuffer) ,@(and library `((require ',library))) (scaling--candidates n))
     :body (let ((completion-styles '(,style)) (completion-ignore-case nil))
             (completion-all-completions "ed" it nil 2)
             (completion-try-completion "ed" it nil 2))))

(scaling--completion-probe completion-basic basic)
(scaling--completion-probe completion-partial partial-completion)
(scaling--completion-probe completion-substring substring)
(scaling--completion-probe completion-flex flex)
(scaling--completion-probe completion-initials initials)
(scaling--completion-probe completion-orderless orderless romacs-orderless)
(scaling--completion-probe completion-fzf fzf romacs-fzf)

;;;; Processes and timers

(scaling-probe process-output :n 200000
  :setup (get-buffer-create " scaling-process")
  :body (let ((process (make-process :name "scaling" :buffer it :command (list "seq" (number-to-string n))
                                     :connection-type 'pipe :noquery t)))
          (while (process-live-p process) (accept-process-output process 1))
          (accept-process-output process 0)
          (with-current-buffer it (count-lines (point-min) (point-max)))))

(scaling-probe timers :n 200
  :known "ticket timers: timer--activate walks the sorted timer-list, ~12k instructions a step"
  :setup nil
  :body (let (timers)
          (dotimes (i n) (push (run-with-timer (+ 3600 i) nil #'ignore) timers))
          (dotimes (_ 20) (sit-for 0))
          (mapc #'cancel-timer timers)))

;;;; Redisplay (GUI: the host lays out each step's change)

(scaling-probe redisplay-lines :gui t :n 2000 :steps 20 :budget 0.5
  :setup (let ((buffer (scaling--lines-buffer "scaling-lines" n)))
           (with-current-buffer buffer
             (goto-char (point-min))
             (dotimes (i n)
               (put-text-property (point) (line-end-position) 'face (if (cl-oddp i) 'bold 'font-lock-keyword-face))
               (forward-line 1))
             (goto-char (/ (point-max) 2)))
           (switch-to-buffer buffer)
           buffer)
  :body (with-current-buffer it (forward-line 1) (insert "x")))

(scaling-probe redisplay-windows :gui t :n 3 :steps 20 :gap 0.4
  :setup (let ((buffer (scaling--lines-buffer "scaling-windows" 200)))
           (setq window-min-height 1 window-min-width 2)
           (switch-to-buffer buffer)
           (delete-other-windows)
           (dotimes (i n) (split-window (get-largest-window) nil (if (cl-oddp i) 'right 'below)))
           (balance-windows)
           buffer)
  :body (with-current-buffer it (goto-char (point-min)) (forward-line i) (insert "x")))

(scaling-probe redisplay-buffers :gui t :n 100 :steps 20 :budget 0.5
  :setup (progn
           (dolist (name (scaling--names "scaling-chrome" n))
             (with-current-buffer (get-buffer-create name) (insert name)))
           (switch-to-buffer (get-buffer-create "scaling-chrome-shown"))
           (current-buffer))
  :body (with-current-buffer it (insert "x")))

;;;; Server clients and agent sessions (romacs-server.el, romacs-agent*.el)

(defun scaling--server ()
  "romacs-server.el's waiter index with its GNU advice, and no socket."
  (require 'romacs-server)
  (advice-add 'server-buffer-done :before #'romacs-server--buffer-done)
  (advice-add 'server-done :around #'romacs-server--done))

;; One native client waiting on N files, each finished with server-done,
;; last first: each costs its own waiters, never the files left.  The
;; index alone (bury-buffer, which every server-done runs as GNU's does,
;; left out) and the whole command.
(scaling-probe server-waiters-index :n 200
  :setup (progn (scaling--server)
                (mapcar (lambda (name) (get-buffer-create name)) (scaling--names " scaling-wait" n)))
  :body (cl-letf (((symbol-function 'romacs-mcp-reply) #'ignore)
                  ((symbol-function 'bury-buffer) #'ignore))
          (romacs-server--wait (list :probe t) 'scaling-peer it)
          (dolist (buffer (reverse it)) (with-current-buffer buffer (server-done)))))

(scaling-probe server-waiters :n 200
  :known "ticket buffers: bury-buffer, which server-done runs per finished buffer, rebuilds the buffer order list"
  :setup (progn (scaling--server)
                (mapcar (lambda (name) (get-buffer-create name)) (scaling--names " scaling-wait" n)))
  :body (cl-letf (((symbol-function 'romacs-mcp-reply) #'ignore))
          (romacs-server--wait (list :probe t) 'scaling-peer it)
          (dolist (buffer (reverse it)) (with-current-buffer buffer (server-done)))))

;; N requests held on one connection, then the connection lost.
(scaling-probe server-peer :n 200
  :setup (progn (scaling--server)
                (mapcar (lambda (name) (get-buffer-create name)) (scaling--names " scaling-peer" n)))
  :body (progn
          (dolist (buffer it) (romacs-server--wait (list :probe t) 'scaling-one-peer (list buffer)))
          (romacs-server--peer-closed 'scaling-one-peer)))

(defun scaling--agent ()
  "romacs-agent.el with a private state directory."
  (setenv "XDG_STATE_HOME" (make-temp-file "scaling-state-" t))
  (require 'romacs-agent))

;; One event to each of N sessions.
(scaling-probe agent-dispatch :n 200
  :setup (progn (scaling--agent)
                (clrhash romacs-agent--sessions)
                (mapcar (lambda (id) (let ((s (romacs-agent--make :id id :harness 'claude :title id)))
                                       (puthash id s romacs-agent--sessions) s))
                        (scaling--names "scaling-session" n)))
  :body (let ((record (json-parse-string "{\"seq\": 1, \"out\": {\"type\": \"result\", \"total_cost_usd\": 0}}"
                                         :object-type 'hash-table :null-object :null :false-object :false)))
          (dolist (session it) (romacs-agent--record (romacs-agent-get (romacs-agent-id session)) record))))

;; N calls recorded in a session's call log (each an appended line) and
;; each looked up.
(scaling-probe agent-calls :n 200
  :setup (progn (scaling--agent)
                (make-directory (romacs-agent-tools--dir "scaling-calls") t)
                (scaling--names "relay:call" n))
  :body (progn
          (dolist (key it) (romacs-agent-tools--note "scaling-calls" key (romacs-mcp--object "state" "done")))
          (dolist (key it) (gethash key (romacs-agent-tools--table "scaling-calls")))))

;; N queued calls, each withdrawn by its relay.
(scaling-probe agent-withdraw :n 64
  :setup (progn (require 'romacs-mcp) (number-sequence 0 (1- n)))
  :body (let ((romacs-mcp--queue nil) (romacs-mcp--queued (make-hash-table :test #'equal)))
          (dolist (i it)
            (let ((request (list :peer 'scaling-peer :message (romacs-mcp--object "id" i) :cancelled nil)))
              (push request romacs-mcp--queue)
              (puthash (cons 'scaling-peer i) request romacs-mcp--queued)))
          (cl-letf (((symbol-function 'romacs-mcp--send) #'ignore))
            (dolist (i it) (romacs-mcp--withdraw 'scaling-peer "w" i)))))

;; A chat of N tool calls, started then each answered.
(scaling-probe agent-chat-tools :n 200
  :setup (progn (scaling--agent)
                (let ((session (romacs-agent--make :id "scaling-chat" :harness 'claude :title "scaling chat"
                                                   :dir temporary-file-directory)))
                  (romacs-agent-show session)
                  session))
  :body (progn
          (dotimes (i n)
            (romacs-agent-ui--render it (list :type 'tool :state 'started :id (format "t%d" i) :name "Bash"
                                              :seq (1+ i) :last t)))
          (dotimes (i n)
            (romacs-agent-ui--render it (list :type 'tool :state 'completed :id (format "t%d" i) :output "ok"
                                              :seq (+ 1 n i) :last t)))))

;; Cursor motion in a truncated window, which places point's line for its
;; hscroll each redisplay: N faces defined, then N distinct faces on
;; point's line.  Setup returns the buffer: a step's timer runs in whatever
;; buffer is current then.
(scaling-probe hscroll-faces-defined :gui t :n 500 :steps 40
  :setup (progn (dotimes (i n) (custom-declare-face (intern (format "scaling-hs-%d" i)) '((t :weight bold)) "x" :group 'scaling))
                (switch-to-buffer (get-buffer-create "hs")) (erase-buffer)
                (insert (make-string 400 ?x) "\n") (setq-local truncate-lines t) (goto-char 10)
                (current-buffer))
  :body (with-current-buffer it (if (cl-evenp i) (forward-char 1) (backward-char 1))))

(scaling-probe hscroll-faces-on-line :gui t :n 500 :steps 40
  :setup (progn (switch-to-buffer (get-buffer-create "hs")) (erase-buffer)
                (dotimes (i n) (let ((face (intern (format "scaling-hs-%d" i))))
                                 (custom-declare-face face '((t :weight bold)) "x" :group 'scaling)
                                 (insert (propertize "x" 'face face))))
                (insert "\n") (setq-local truncate-lines t) (goto-char 10)
                (current-buffer))
  :body (with-current-buffer it (if (cl-evenp i) (forward-char 1) (backward-char 1))))

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
                            :known (or (plist-get plist :known) nil) :gui (if (plist-get plist :gui) t :json-false)
                            :steps (or (plist-get plist :steps) 1) :stack (if (plist-get plist :stack) t :json-false))))))

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

(defun scaling--mark (plist)
  "Append PLIST as a JSON line to stderr, among the host's paint diagnostics."
  (write-region (concat (json-encode plist) "\n") nil "/dev/stderr" t 'silent))

(defun scaling-gui-start (name n out)
  "Run GUI probe NAME at size N in this session, then exit.
Once the session is idle, set the probe up; then run its steps from a chain
of timers, the host laying out each step's change as the command loop
returns; count the instructions from before the first step to after the
last step's layout; write (:name :n :instructions) to OUT as JSON and kill
Emacs.  Stderr marks the start, each step and the end among the paint
diagnostics, so the harness can check a paint was submitted after each
step, before the next step or the count's end."
  (require 'json)
  (let* ((plist (scaling--probe name))
         (steps (or (plist-get plist :steps) 1))
         (body (plist-get plist :body))
         (gap (or (plist-get plist :gap) 0.1)) it start)
    (setq blink-cursor-mode nil)
    (run-with-idle-timer
     0.5 nil
     (lambda ()
       (setq it (funcall (plist-get plist :setup) n))
       (garbage-collect)
       (run-with-timer
        0.5 nil
        (lambda ()
          (scaling--mark (list :scaling "start"))
          (setq start (romacs-instructions))
          (let ((step nil) (i 0))
            (setq step (lambda ()
                         (if (< i steps)
                             (progn (scaling--mark (list :scaling "step" :i i))
                                    (funcall body n it i) (setq i (1+ i)) (run-with-timer gap nil step))
                           (let ((end (romacs-instructions)))
                             (scaling--mark (list :scaling "end"))
                             (with-temp-file out
                               (insert (json-encode (list :name (symbol-name name) :n n
                                                          :instructions (and start end (- end start))))))
                             (kill-emacs 0)))))
            (funcall step))))))))

(provide 'scaling-probes)
;;; probes.el ends here
