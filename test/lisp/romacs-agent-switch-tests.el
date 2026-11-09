;;; romacs-agent-switch-tests.el --- Choosing a session by its status -*- lexical-binding: t; -*-
(require 'ert)
(require 'romacs-mcp)

(let ((romacs-mcp-call-function romacs-mcp-call-function)
      (romacs-mcp-tools (copy-alist romacs-mcp-tools)))
  (require 'romacs-agent))

(defun romacs-agent-switch-tests--session (title &rest fields)
  (apply #'romacs-agent--make
         (append fields (list :id (format "%s-1111-4222-8333-444444444444" (md5 title)) :harness 'claude :title title
                              :state 'idle :cwd "/home/me/projects/romacs/"))))

(ert-deftest romacs-agent-switch-puts-what-wants-you-first ()
  (let* ((old (romacs-agent-switch-tests--session "old" :last-time 1))
         (new (romacs-agent-switch-tests--session "new" :last-time 9))
         (busy (romacs-agent-switch-tests--session "busy" :state 'running :last-time 2))
         (done (romacs-agent-switch-tests--session "done" :done-seq 5 :seen-seq 1 :last-time 3))
         (asking (romacs-agent-switch-tests--session "asking" :state 'waiting-approval :last-time 0))
         (given (list old new busy done asking)))
    (should (equal (mapcar #'romacs-agent-title (romacs-agent-ui--by-attention given))
                   '("asking" "done" "busy" "new" "old")))
    ;; The list it was given stays as it was.
    (should (eq (car given) old))))

(ert-deftest romacs-agent-switch-puts-finished-tasks-last ()
  (let* ((old (romacs-agent-switch-tests--session "old" :last-time 1))
         (task (romacs-agent-switch-tests--session
                "task" :options (list :parent "p") :done-seq 5 :seen-seq 1 :last-time 9))
         (asking (romacs-agent-switch-tests--session
                  "asking" :options (list :parent "p") :state 'waiting-approval :last-time 0))
         (busy (romacs-agent-switch-tests--session
                "busy" :options (list :parent "p") :state 'running :last-time 2)))
    (should (equal (mapcar #'romacs-agent-title (romacs-agent-ui--by-attention (list task old busy asking)))
                   '("asking" "busy" "old" "task")))))

(ert-deftest romacs-agent-switch-names-are-unique ()
  (let* ((id "abcdef12-1111-4222-8333-444444444444")
         (a (romacs-agent-switch-tests--session "same"))
         (b (romacs-agent--make :id id :harness 'claude :title "same"))
         (c (romacs-agent--make :id "99999999-1111-4222-8333-444444444444" :harness 'claude :title "same <abcdef12>"))
         (d (romacs-agent--make :id id :harness 'claude :title "same"))
         (names (romacs-agent-ui--session-names (list a b c d))))
    (should (= (length (cdr names)) 4))
    (should (= (hash-table-count (car names)) 4))
    (should (eq (gethash "same" (car names)) a))
    (should (eq (gethash "same <abcdef12>" (car names)) b))
    (should (equal (mapcar (lambda (name) (gethash name (car names))) (cdr names)) (list a b c d)))))

(ert-deftest romacs-agent-switch-resolves-what-the-prompt-accepted ()
  (let* ((session (romacs-agent-switch-tests--session "Alpha"))
         (names (romacs-agent-ui--session-names (list session))))
    (should (eq (romacs-agent-ui--resolve-session (car names) (cdr names) "Alpha") session))
    (should (eq (romacs-agent-ui--resolve-session (car names) (cdr names) "alpha") session))
    (should-error (romacs-agent-ui--resolve-session (car names) (cdr names) "") :type 'user-error)))

(ert-deftest romacs-agent-switch-keeps-a-long-title-s-value ()
  (let* ((title (make-string 60 ?x))
         (session (romacs-agent-switch-tests--session title))
         (row (romacs-agent-ui--switch-row title session 44 120 nil)))
    (should (equal (substring-no-properties (car row)) title))
    (should (= (string-width (get-text-property 0 'display (car row))) 44))))

(ert-deftest romacs-agent-switch-cuts-a-title-with-the-session-s-ellipsis ()
  (let* ((truncate-string-ellipsis "...")
         (row (romacs-agent-ui--switch-row
               (make-string 60 ?x) (romacs-agent-switch-tests--session "s") 44 120 nil))
         (shown (get-text-property 0 'display (car row))))
    (should (string-suffix-p "..." shown))
    (should (= (string-width shown) 44))))

(ert-deftest romacs-agent-switch-row-fits-its-width ()
  (let ((session (romacs-agent-switch-tests--session
                  "s" :model "claude-opus-5-5" :provider "a-long-provider" :context-used 250000 :context-window 1000000
                  :queue-length 12 :last-time (float-time) :reply "text")))
    (dolist (width '(120 58 30 12))
      (should (<= (string-width (romacs-agent-ui--switch-suffix session width nil)) width)))
    ;; A quarter full is nothing to act on.
    (should-not (string-match-p "25%" (romacs-agent-ui--switch-suffix session 120 nil)))))

(ert-deftest romacs-agent-switch-reads-only-what-a-reply-can-show ()
  (let ((reply (concat (make-string 40 ?a) (make-string 1000 ?\s) "tail")))
    (should-not (string-match-p "tail" (romacs-agent-ui--one-line reply 40)))
    (should (<= (string-width (romacs-agent-ui--one-line reply 40)) 40))))

(ert-deftest romacs-agent-switch-offers-every-session ()
  (let* ((root (romacs-agent-switch-tests--session "root"))
         (done (romacs-agent-switch-tests--session "done" :options (list :parent "p") :done-seq 3))
         (busy (romacs-agent-switch-tests--session "busy" :options (list :parent "p") :state 'running)))
    (cl-letf (((symbol-function 'romacs-agent-sessions) (lambda () (list root done busy))))
      (with-temp-buffer
        (should (equal (mapcar #'romacs-agent-title (romacs-agent-ui--switch-sessions))
                       '("busy" "root" "done")))))))

(ert-deftest romacs-agent-switch-chat-offers-only-open-chats ()
  (let* ((buffer (generate-new-buffer " *open*"))
         (dead (generate-new-buffer " *dead*"))
         (open (romacs-agent-switch-tests--session "open" :buffer buffer))
         (closed (romacs-agent-switch-tests--session "closed"))
         (gone (romacs-agent-switch-tests--session "gone" :buffer dead))
         (shown nil))
    (kill-buffer dead)
    (unwind-protect
        (cl-letf (((symbol-function 'romacs-agent-sessions) (lambda () (list open closed gone)))
                  ((symbol-function 'romacs-agent-show) (lambda (session) (setq shown session)))
                  ((symbol-function 'completing-read)
                   (lambda (_ table &rest _)
                     (should (equal (all-completions "" table) '("open")))
                     "open")))
          (with-temp-buffer
            (call-interactively #'romacs-agent-switch-chat))
          (should (eq shown open)))
      (kill-buffer buffer))))

(ert-deftest romacs-agent-switch-chat-makes-no-chat ()
  (let ((closed (romacs-agent-switch-tests--session "closed")))
    (cl-letf (((symbol-function 'romacs-agent-sessions) (lambda () (list closed)))
              ((symbol-function 'romacs-agent-show) (lambda (_) (error "Made a chat"))))
      (with-temp-buffer
        (should (equal (cadr (should-error (call-interactively #'romacs-agent-switch-chat) :type 'user-error))
                       "No other agent chats are open"))))))

(ert-deftest romacs-agent-switch-uses-this-window ()
  (let ((action nil) (session (romacs-agent-switch-tests--session "s")))
    (cl-letf (((symbol-function 'romacs-agent-show) (lambda (_) (setq action display-buffer-overriding-action))))
      (romacs-agent-switch session)
      (should (equal action '(display-buffer-same-window)))
      (setq action nil)
      (romacs-agent-switch-chat session)
      (should (equal action '(display-buffer-same-window))))))

(ert-deftest romacs-agent-switch-resolves-the-provider-once-per-table ()
  (let* ((calls 0)
         (names (romacs-agent-ui--session-names
                 (list (romacs-agent-switch-tests--session "a" :provider "mine")
                       (romacs-agent-switch-tests--session "b" :provider "other")))))
    (cl-letf (((symbol-function 'romacs-agent--default-provider) (lambda () (cl-incf calls) "mine")))
      (let* ((table (romacs-agent-ui--session-table (car names) (cdr names) 120))
             (rows (funcall (alist-get 'affixation-function (cdr (funcall table "" nil 'metadata))) (cdr names))))
        (should (= calls 1))
        (should-not (string-match-p "mine" (nth 2 (nth 0 rows))))
        (should (string-match-p "other" (nth 2 (nth 1 rows))))))))

(ert-deftest romacs-agent-switch-row-says-what-matters-on-one-line ()
  (let* ((session (romacs-agent-switch-tests--session
                   "s" :model "claude-opus-5-5" :context-used 850000 :context-window 1000000
                   :done-seq 2 :seen-seq 1 :queue-length 2 :last-time (- (float-time) 120)
                   :reply "**The first** line of [the reply](https://x.org)\n- and `a second`"))
         (row (romacs-agent-ui--switch-suffix session 120 nil)))
    ;; Its model is noise; a nearly full context is not.
    (should-not (string-match-p "opus-5-5" row))
    (should (string-match-p "85%" row))
    (should (eq (get-text-property (string-search "85%" row) 'face row) 'warning))
    (should (eq (get-text-property (string-search "romacs" row) 'face row) 'romacs-agent-project))
    (should (string-match-p "2 queued" row))
    (should (string-match-p "●" row))
    ;; The reply, as words.
    (should (string-match-p "The first line of the reply and a second" row))
    (should-not (string-match-p "\n" row))
    (should (<= (string-width row) 120))
    ;; Narrow: the facts stay, the reply goes first.
    (let ((narrow (romacs-agent-ui--switch-suffix session 60 nil)))
      (should (string-match-p "85%" narrow))
      (should-not (string-match-p "second" narrow)))))

(ert-deftest romacs-agent-switch-plain-words ()
  (should (equal (romacs-agent-ui--plain "*Important* and _urgent_, snake_case_name stays") "Important and urgent, snake_case_name stays"))
  (should (equal (romacs-agent-ui--plain "- **bold** item\n## Head") "bold item\nHead"))
  (should (equal (romacs-agent-ui--plain "*one* *two* _three_ _four_") "one two three four")))

(ert-deftest romacs-agent-switch-row-names-only-states-that-want-you ()
  (should-not (string-match-p "done\\|idle" (romacs-agent-ui--switch-suffix
                                             (romacs-agent-switch-tests--session "s" :turns 1 :state 'idle) 80 nil)))
  (should (string-match-p "working" (romacs-agent-ui--switch-suffix
                                     (romacs-agent-switch-tests--session "s" :state 'running) 80 nil))))

(ert-deftest romacs-agent-switch-copes-with-a-bare-session ()
  (let ((row (romacs-agent-ui--switch-suffix
              (romacs-agent--make :id "abcdef12-1111-4222-8333-444444444444" :harness 'claude :state 'ended) 80 nil)))
    (should (stringp row))
    (should-not (string-match-p "nil" row))))

(ert-deftest romacs-agent-switch-table-keeps-its-order-and-affixes ()
  (let* ((names (romacs-agent-ui--session-names
                 (list (romacs-agent-switch-tests--session "zed") (romacs-agent-switch-tests--session "alpha"))))
         (table (romacs-agent-ui--session-table (car names) (cdr names) 100))
         (metadata (cdr (funcall table "" nil 'metadata))))
    (should (eq (alist-get 'display-sort-function metadata) 'identity))
    (should (equal (all-completions "" table) '("zed" "alpha")))
    (let ((rows (funcall (alist-get 'affixation-function metadata) '("zed" "alpha"))))
      (should (equal (mapcar #'car rows) '("zed" "alpha")))
      ;; Aligned: what follows the names starts in one column.
      (should (= (+ (string-width (nth 0 (nth 0 rows))) (string-match-p "[^ ]" (nth 2 (nth 0 rows))))
                 (+ (string-width (nth 0 (nth 1 rows))) (string-match-p "[^ ]" (nth 2 (nth 1 rows)))))))))

(ert-deftest romacs-agent-switch-leaves-out-the-current-chat ()
  (let* ((here (romacs-agent-switch-tests--session "here"))
         (there (romacs-agent-switch-tests--session "there"))
         (offered nil))
    (cl-letf (((symbol-function 'romacs-agent-sessions) (lambda () (list here there)))
              ((symbol-function 'completing-read)
               (lambda (_ table &rest _) (setq offered (all-completions "" table)) "there")))
      (with-temp-buffer
        (romacs-agent-chat-mode)
        (setq romacs-agent-ui--session here)
        (should (eq (romacs-agent-ui--read-session "Agent: " (romacs-agent-ui--switch-sessions) "None") there))))
    (should (equal offered '("there")))))

(ert-deftest romacs-agent-switch-prompt-is-as-wide-as-its-rows ()
  (let ((seen nil) (session (romacs-agent-switch-tests--session "s")) (romacs-agent-switch-width 0.7))
    (cl-letf (((symbol-function 'romacs-agent-sessions) (lambda () (list session)))
              ((symbol-function 'completing-read)
               (lambda (&rest _) (setq seen romacs-complete-center-width) "s")))
      (romacs-agent-ui--read-session "Agent: " (list session) "None"))
    (should (equal seen 0.7))))

(ert-deftest romacs-agent-switch-says-when-there-is-nowhere-to-go ()
  (should (equal (cadr (should-error (romacs-agent-ui--read-session "Agent: " nil "No 100% match") :type 'user-error))
                 "No 100% match")))

(ert-deftest romacs-agent-switch-owns-the-chat-keys ()
  (should (eq (lookup-key romacs-agent-chat-mode-map (kbd "C-c C-b")) #'romacs-agent-switch-chat))
  (should (eq (lookup-key romacs-agent-chat-mode-map (kbd "C-c C-t")) #'romacs-agent-switch)))
