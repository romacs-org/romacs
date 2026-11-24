;;; romacs-agent-board-tests.el --- The agent board -*- lexical-binding: t; -*-
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
  (require 'romacs-agent-board)
  (require 'romacs-agent-menu)
  (require 'romacs-agent-watch))

(defvar romacs-agent-history-per-project)

(defmacro romacs-agent-board-tests--with (&rest body)
  "BODY with a registry of its own, `mk' making a session in it (ID TITLE
STATE AGE &rest FIELDS), and `rows' the board's rows for a VIEW plist."
  (declare (indent 0))
  `(let ((romacs-agent--sessions (make-hash-table :test #'equal))
         (romacs-agent--children (make-hash-table :test #'equal))
         (romacs-agent-glyphs nil)
         (romacs-agent-event-functions nil)
         (romacs-agent-history-per-project 2)
         (now (float-time)))
     (cl-flet ((mk (id title state age &rest fields)
                 (let ((session (apply #'romacs-agent--make
                                       (append fields (list :id id :harness 'claude :provider "claude" :title title
                                                            :state state :cwd "/p/one/" :last-time (- now age))))))
                   (puthash id session romacs-agent--sessions)
                   (romacs-agent--index session)
                   session))
               (rows (&rest view)
                 (romacs-agent-board-rows (hash-table-values romacs-agent--sessions) (plist-get view :history)
                                          (append view (list :folds (or (plist-get view :folds)
                                                                        (make-hash-table :test #'equal)))))))
       ,@body)))

(defun romacs-agent-board-tests--ids (rows)
  (mapcar (lambda (row) (plist-get row :id)) rows))

(ert-deftest romacs-agent-board-ranks-then-newest ()
  (romacs-agent-board-tests--with
    (mk "old" "old" 'idle 900)
    (mk "new" "new" 'idle 10)
    (mk "busy" "busy" 'running 500)
    (mk "asks" "asks" 'waiting-approval 800 :driver-state '(:asks ("r")))
    (mk "done" "done" 'idle 700 :done-seq 3)
    (mk "far" "far" 'idle 5 :cwd "/p/two/")
    (should (equal (romacs-agent-board-tests--ids (rows))
                   '("/p/one" "asks" "busy" "done" "new" "old" "/p/two" "far")))
    (should (equal (plist-get (car (rows)) :keys) '(approval working completed idle idle)))))

(ert-deftest romacs-agent-board-settled-has-its-own-section ()
  (romacs-agent-board-tests--with
    (mk "a" "a" 'idle 10 :done-seq 3 :settled-seq 3)
    (mk "b" "b" 'idle 20 :done-seq 3)
    (mk "child" "child" 'idle 30 :cwd "/p/worktree/" :options '(:parent "a")
        :settled-seq romacs-agent--inactive-settlement)
    (let* ((active (rows)) (project (car active)))
      (should (equal (romacs-agent-board-tests--ids active) '("/p/one" "b")))
      (should (equal (plist-get project :settled-count) 2))
      (should (string-match-p "✔️ 2" (romacs-agent-board--line project 80))))
    (should (equal (plist-get (car (rows :words '("b"))) :settled-count) 2))
    (let ((folds (make-hash-table :test #'equal)))
      (puthash "a" 'open folds)
      (should (equal (romacs-agent-board-tests--ids (rows :settled t :folds folds)) '("/p/one" "a" "child"))))))

(ert-deftest romacs-agent-board-hides-an-inactive-parent-without-losing-its-child ()
  (romacs-agent-board-tests--with
    (mk "lead" "lead" 'running 10 :settled-seq romacs-agent--inactive-settlement)
    (mk "worker" "worker" 'running 5 :options '(:parent "lead"))
    (should (equal (romacs-agent-board-tests--ids (rows)) '("/p/one" "worker")))
    (should (equal (plist-get (car (rows)) :keys) '(working)))
    (should (equal (romacs-agent-board-tests--ids (rows :settled t)) '("/p/one" "lead")))))

(ert-deftest romacs-agent-board-children-fold-under-their-parent ()
  (romacs-agent-board-tests--with
    (mk "lead" "lead" 'idle 10)
    (mk "w1" "w1" 'idle 30 :done-seq 2 :options '(:parent "lead"))
    (mk "w2" "w2" 'idle 20 :done-seq 2 :options '(:parent "lead"))
    ;; Nothing goes on under it: folded, a rollup of what is done.
    (let ((lead (cadr (rows))))
      (should (equal (romacs-agent-board-tests--ids (rows)) '("/p/one" "lead")))
      (should (equal (plist-get lead :children) '(completed completed)))
      (should (equal (substring-no-properties (romacs-agent-ui--rollup (plist-get lead :children))) "2 completed")))
    ;; One works: unfolded, nested a level, what works first.
    (setf (romacs-agent-state (romacs-agent-get "w1")) 'running)
    (should (equal (romacs-agent-board-tests--ids (rows)) '("/p/one" "lead" "w1" "w2")))
    (should (equal (mapcar (lambda (row) (plist-get row :depth)) (cdr (rows))) '(0 1 1)))
    ;; The user's fold wins.
    (let ((folds (make-hash-table :test #'equal)))
      (puthash "lead" 'closed folds)
      (should (equal (romacs-agent-board-tests--ids (rows :folds folds)) '("/p/one" "lead"))))))

(ert-deftest romacs-agent-board-filters-by-words ()
  (romacs-agent-board-tests--with
    (mk "a" "Fix the parser" 'idle 10)
    (mk "b" "Theme work" 'idle 20 :model "claude-opus-5-5")
    (mk "lead" "lead" 'idle 30)
    (mk "w" "write the docs" 'idle 40 :options '(:parent "lead"))
    (should (equal (romacs-agent-board-tests--ids (rows :words '("parser"))) '("/p/one" "a")))
    (should (equal (romacs-agent-board-tests--ids (rows :words '("opus" "theme"))) '("/p/one" "b")))
    ;; A delegated thread that matches brings its parent, unfolded.
    (should (equal (romacs-agent-board-tests--ids (rows :words '("docs"))) '("/p/one" "lead" "w")))
    (should-not (rows :words '("nothing")))))

(ert-deftest romacs-agent-board-lists-transcripts-per-project ()
  (romacs-agent-board-tests--with
    (mk "a" "a" 'idle 10)
    (let ((history (cl-loop for i below 3 collect (list :path (format "/h/%d" i) :title (format "t%d" i) :cwd "/p/one"
                                                         :mtime (- now i))))
          (other (list :path "/h/x" :title "x" :cwd "/p/three" :mtime now)))
      (should (equal (romacs-agent-board-tests--ids (rows :history history))
                     '("/p/one" "a" "/h/0" "/h/1" (more . "/p/one"))))
      (should (equal (romacs-agent-board-tests--ids (rows :history history :all-history t))
                     '("/p/one" "a" "/h/0" "/h/1" "/h/2")))
      (should (equal (romacs-agent-board-tests--ids (rows :history (list other)))
                     '("/p/one" "a" "/p/three" "/h/x")))
      (should-not (cl-find 'history (rows :history history :settled t) :key (lambda (row) (plist-get row :kind)))))))

(ert-deftest romacs-agent-board-says-the-transcripts-are-being-read ()
  ;; Their first scan under way, none listed: one line says so, never an
  ;; empty section; once listed, or settled, none.
  (romacs-agent-board-tests--with
    (mk "a" "a" 'idle 10)
    (let ((last (car (last (rows :reading t)))))
      (should (eq (plist-get last :kind) 'reading))
      (should (string-match-p "Reading harness transcripts" (substring-no-properties (romacs-agent-board--line last 80)))))
    (should-not (cl-find 'reading (rows :reading t :history (list (list :path "/h/x" :title "x" :cwd "/p/one" :mtime now)))
                         :key (lambda (row) (plist-get row :kind))))
    (should-not (cl-find 'reading (rows :reading t :settled t) :key (lambda (row) (plist-get row :kind))))
    (should-not (cl-find 'reading (rows) :key (lambda (row) (plist-get row :kind))))))

(ert-deftest romacs-agent-board-a-line-says-provider-and-model ()
  (romacs-agent-board-tests--with
    (mk "a" "Port the theme" 'running 30 :model "claude-opus-5-5")
    (let ((line (substring-no-properties (romacs-agent-board--line (cadr (rows)) 100))))
      (should (= (string-width line) 100))
      (should (string-match-p "\\`  Working +Port the theme +claude · opus-5-5 +now *\\'" line)))))

(ert-deftest romacs-agent-board-a-deep-chain-keeps-its-titles-in-view ()
  (romacs-agent-board-tests--with
    (dotimes (i 40)
      (mk (format "c%02d" i) (format "link %d" i) 'idle (- 100 i)
          :options (and (> i 0) (list :parent (format "c%02d" (1- i))))))
    ;; Filtered to the last: every thread above it shows, each a level
    ;; deeper, and each row indents no further than the last level.
    (let* ((rows (rows :words '("39")))
           (deep (car (last rows)))
           (line (substring-no-properties (romacs-agent-board--line deep 80))))
      (should (= (plist-get deep :depth) 39))
      (should (= (string-width line) 80))
      (should (string-match-p (format "\\`%s +link 39 " (make-string (* 2 (1+ romacs-agent-board--max-indent-depth)) ?\s))
                              line)))))

(ert-deftest romacs-agent-board-a-thread-shows-in-its-own-section ()
  (romacs-agent-board-tests--with
    (mk "lead" "lead" 'idle 10)
    (mk "w" "w" 'idle 20 :done-seq 2 :settled-seq 2 :options '(:parent "lead"))
    ;; A settled child leaves the board; the Settled section has it, under
    ;; its parent shown dim for context.
    (should-not (plist-get (cadr (rows)) :children))
    (should (equal (romacs-agent-board-tests--ids (rows :settled t)) '("/p/one" "lead" "w")))
    (should (equal (mapcar (lambda (row) (plist-get row :context)) (cdr (rows :settled t))) '(t nil)))
    ;; A settled parent with a child at work: the child stays on the board.
    (setf (romacs-agent-settled-seq (romacs-agent-get "lead")) 0
          (romacs-agent-settled-seq (romacs-agent-get "w")) nil
          (romacs-agent-state (romacs-agent-get "w")) 'running)
    (should (equal (romacs-agent-board-tests--ids (rows)) '("/p/one" "lead" "w")))
    (should (plist-get (cadr (rows)) :context))
    (should (equal (romacs-agent-board-tests--ids (rows :settled t)) '("/p/one" "lead")))))

(ert-deftest romacs-agent-board-a-filter-leaves-no-empty-project ()
  (romacs-agent-board-tests--with
    (mk "a" "parser" 'idle 10)
    (let ((history (list (list :path "/h/x" :title "theme notes" :cwd "/p/three" :mtime now))))
      (should (equal (romacs-agent-board-tests--ids (rows :history history :words '("theme")))
                     '("/p/three" "/h/x")))
      (should-not (rows :history history :words '("nothing"))))))

(ert-deftest romacs-agent-board-a-narrow-board-names-provider-and-model ()
  (romacs-agent-board-tests--with
    (mk "a" "Port the theme" 'running 30 :model "claude-opus-5-5")
    (let ((line (substring-no-properties (romacs-agent-board--line (cadr (rows)) 60))))
      (should (= (string-width line) 60))
      (should (string-search "claude · opus" line)))))

;;; Keys, on the board's buffer

(defmacro romacs-agent-board-tests--board (&rest body)
  "BODY in the board's buffer, drawn 100 columns wide, with the chats
`romacs-agent-show' would show recorded in `shown', each (SESSION ACTION)."
  (declare (indent 0))
  `(let ((shown nil))
     (with-temp-buffer
       (romacs-agent-board-mode)
       (cl-letf (((symbol-function 'romacs-agent-show) (lambda (session &optional action _source) (push (list session action) shown)))
                 ((symbol-function 'romacs-agent-board--columns) (lambda (_) 100)))
         (romacs-agent-board--redraw)
         ,@body))))

(defun romacs-agent-board-tests--at ()
  (plist-get (romacs-agent-board--row) :id))

(ert-deftest romacs-agent-board-keys-move-and-settle ()
  (romacs-agent-board-tests--with
    (mk "a" "a" 'idle 10 :done-seq 3)
    (mk "b" "b" 'idle 20 :done-seq 4)
    (let ((dir (make-temp-file "romacs-board-" t)))
      (unwind-protect
          (progn
            (dolist (session (hash-table-values romacs-agent--sessions))
              (setf (romacs-agent-dir session) dir))
            (romacs-agent-board-tests--board
              ;; Shown anew, on the first thread; n and p skip the lines
              ;; that are no row.
              (should (equal (romacs-agent-board-tests--at) "a"))
              (romacs-agent-board-previous 3)
              (should (equal (romacs-agent-board-tests--at) "/p/one"))
              (romacs-agent-board-next 1)
              (romacs-agent-board-next 5)
              (should (equal (romacs-agent-board-tests--at) "b"))
              (romacs-agent-board-previous 1)
              ;; s settles: it leaves the board.
              (romacs-agent-board-settle)
              (should (romacs-agent-settled-p (romacs-agent-get "a")))
              (should (equal (romacs-agent-board-tests--ids (romacs-agent-board--shown-rows)) '("/p/one" "b")))
              ;; M-RET: the Settled section, where s brings it back.
              (romacs-agent-board-toggle-settled)
              (should (equal (romacs-agent-board-tests--ids (romacs-agent-board--shown-rows)) '("/p/one" "a")))
              (should (equal (romacs-agent-board-tests--at) "a"))
              (romacs-agent-board-settle)
              (should-not (romacs-agent-settled-p (romacs-agent-get "a")))
              (romacs-agent-board-toggle-settled)
              ;; S: every finished thread at once.
              (cl-letf (((symbol-function 'y-or-n-p) (lambda (_) t)))
                (romacs-agent-board-settle-finished))
              (should (romacs-agent-settled-p (romacs-agent-get "a")))
              (should (romacs-agent-settled-p (romacs-agent-get "b")))))
        (delete-directory dir t)))))

(ert-deftest romacs-agent-board-keys-fold-and-open ()
  (romacs-agent-board-tests--with
    (let ((lead (mk "lead" "lead" 'idle 10)))
      (mk "w" "w" 'idle 20 :done-seq 2 :options '(:parent "lead"))
      (romacs-agent-board-tests--board
        (should (equal (romacs-agent-board-tests--ids (romacs-agent-board--shown-rows)) '("/p/one" "lead")))
        (romacs-agent-board-fold)
        (should (equal (romacs-agent-board-tests--ids (romacs-agent-board--shown-rows)) '("/p/one" "lead" "w")))
        (should (equal (romacs-agent-board-tests--at) "lead"))
        ;; b on a delegated thread folds its parent's.
        (romacs-agent-board-next 1)
        (romacs-agent-board-close-fold)
        (should (equal (romacs-agent-board-tests--ids (romacs-agent-board--shown-rows)) '("/p/one" "lead")))
        (should (equal (romacs-agent-board-tests--at) "lead"))
        ;; RET opens its chat in the board's window.
        (romacs-agent-board-open)
        (should (equal shown (list (list lead '(display-buffer-same-window)))))))))

(ert-deftest romacs-agent-board-keys-open-what-is-there ()
  (romacs-agent-board-tests--with
    (let ((a (mk "a" "a" 'idle 10)))
      (romacs-agent-board-tests--board
        ;; A project opens its first thread.
        (goto-char (point-min))
        (romacs-agent-board-next 1)
        (romacs-agent-board-open)
        (should (= (length shown) 1))
        ;; A thread no longer known: the board is drawn again.
        (remhash "a" romacs-agent--sessions)
        (romacs-agent-board-next 1)
        (should (eq (plist-get (romacs-agent-board--row) :session) a))
        (should-error (romacs-agent-board-open) :type 'user-error)
        (should-not (romacs-agent-board--shown-rows))
        (should (= (length shown) 1))))))

(ert-deftest romacs-agent-board-keys-approve ()
  (romacs-agent-board-tests--with
    (mk "tool" "tool" 'waiting-approval 10 :driver-state '(:asks ("r1")))
    (mk "ask" "ask" 'waiting-approval 20 :driver-state '(:asks ("q1") :questions ("q1")))
    (romacs-agent-board-tests--board
      (let ((answered nil))
        (cl-letf (((symbol-function 'romacs-agent-respond)
                   (lambda (session id allow &rest _) (push (list (romacs-agent-id session) id allow) answered))))
          (should (equal (romacs-agent-board-tests--at) "tool"))
          (romacs-agent-board-approve)
          (should (equal answered '(("tool" "r1" t))))
          ;; A question is answered in its chat.
          (romacs-agent-board-next 1)
          (romacs-agent-board-approve)
          (should (= (length answered) 1))
          (should (= (length shown) 1)))))))

(ert-deftest romacs-agent-board-filter-follows-typing ()
  (romacs-agent-board-tests--with
    (mk "a" "Fix the parser" 'idle 10)
    (mk "b" "Theme work" 'idle 20)
    (romacs-agent-board-tests--board
      (cl-letf (((symbol-function 'read-from-minibuffer) (lambda (&rest _) "theme")))
        (romacs-agent-board-filter))
      (should (equal (romacs-agent-board-tests--ids (romacs-agent-board--shown-rows)) '("/p/one" "b")))
      (should (string-search "/theme" (buffer-substring (point-min) (line-end-position 1))))
      ;; C-g puts the filter back as it was.
      (cl-letf (((symbol-function 'read-from-minibuffer) (lambda (&rest _) (signal 'quit nil))))
        (romacs-agent-board-filter))
      (should (equal romacs-agent-board--words '("theme"))))))

(ert-deftest romacs-agent-board-follows-sessions-only-while-shown ()
  (romacs-agent-board-tests--with
    (mk "a" "a" 'idle 10)
    (let ((drawn 0))
      (cl-letf (((symbol-function 'romacs-agent-board--render) (lambda (&rest _) (cl-incf drawn))))
        (cl-letf (((symbol-function 'romacs-agent-board--shown) #'ignore))
          (run-hooks 'romacs-agent-ui-changed-hook))
        (should (= drawn 0))
        (cl-letf (((symbol-function 'romacs-agent-board--shown) (lambda () (current-buffer))))
          (run-hooks 'romacs-agent-ui-changed-hook))
        (should (= drawn 1))))))

;;; Columns

(ert-deftest romacs-agent-board-columns-are-a-list-of-functions ()
  (romacs-agent-board-tests--with
    (mk "a" "Port the theme" 'idle 120)
    (let ((romacs-agent-board-columns
           (list (cons (lambda (row) (upcase (romacs-agent-title (plist-get row :session)))) 'fill)
                 (cons #'romacs-agent-board-time 4))))
      (should (equal (substring-no-properties (romacs-agent-board--line (cadr (rows)) 30))
                     "  PORT THE THEME          2m  ")))))

(ert-deftest romacs-agent-board-a-lambda-column-on-a-working-row ()
  (romacs-agent-board-tests--with
    (mk "a" "a" 'running 5)
    (let ((romacs-agent-board-columns (list (cons (lambda (_) "x") 'fill))))
      (should (string-prefix-p "  x" (substring-no-properties (romacs-agent-board--line (cadr (rows)) 20)))))))

(ert-deftest romacs-agent-board-a-cut-row-keeps-still ()
  ;; Too narrow for every column: no cell of it is asked again, so none
  ;; can grow the row past the board.
  (romacs-agent-board-tests--with
    (mk "a" "a" 'running 5 :activity "Read" :turn-start (- now 5))
    (let ((line (romacs-agent-board--line (cadr (rows)) 40)))
      (should (= (string-width line) 40))
      (should-not (text-property-not-all 0 (length line) 'romacs-agent-board-live nil line)))
    (let ((line (romacs-agent-board--line (cadr (rows)) 120)))
      (should (text-property-not-all 0 (length line) 'romacs-agent-board-live nil line)))))

(ert-deftest romacs-agent-board-doing-and-time ()
  (romacs-agent-board-tests--with
    (mk "lead" "lead" 'running 5 :activity "mcp__romacs__romacs_eval" :turn-start (- now 75))
    (mk "w" "w" 'idle 20 :done-seq 2 :options '(:parent "lead"))
    (let ((folds (make-hash-table :test #'equal)))
      ;; Unfolded: the tool its turn runs, and how long the turn has.
      (puthash "lead" 'open folds)
      (let ((lead (cadr (rows :folds folds))))
        (should (equal (substring-no-properties (romacs-agent-board-doing lead)) "romacs_eval"))
        (should (equal (substring-no-properties (romacs-agent-board-time lead)) "1m")))
      ;; Folded: what its threads are doing.
      (puthash "lead" 'closed folds)
      (should (equal (substring-no-properties (romacs-agent-board-doing (cadr (rows :folds folds)))) "1 completed")))))

(ert-deftest romacs-agent-board-activity-is-the-turn-s-last-tool ()
  (let ((session (romacs-agent--make :id "s" :harness 'claude :state 'idle))
        (romacs-agent-event-functions nil))
    (romacs-agent--raise session '(:type input :seq 1))
    (romacs-agent--raise session '(:type tool :state started :id "t1" :name "Read"))
    ;; A subagent's own tool is not the turn's.
    (romacs-agent--raise session '(:type tool :state started :id "t2" :name "Bash" :parent "t1"))
    (should (equal (romacs-agent-activity session) "Read"))
    (romacs-agent--raise session '(:type done :seq 2))
    (should-not (romacs-agent-activity session))))

(ert-deftest romacs-agent-board-cells-at-work-follow-the-clock ()
  (romacs-agent-board-tests--with
    (let ((session (mk "a" "a" 'running 5 :activity "Read" :turn-start (- now 5))))
      (mk "b" "b" 'idle 50)
      (romacs-agent-board-tests--board
        (let ((before (buffer-string)) (romacs-agent-board--ticker (run-at-time 3600 nil #'ignore)))
          (setf (romacs-agent-activity session) "Bash")
          (cl-letf (((symbol-function 'romacs-agent-board--shown) (lambda () (current-buffer))))
            (romacs-agent-board--tick))
          (should (string-search "Bash" (buffer-string)))
          (should-not (string-search "Read" (buffer-string)))
          ;; Only the cells at work changed, each its own width.
          (should (= (length before) (length (buffer-string))))
          (should (equal (romacs-agent-board-tests--ids (romacs-agent-board--shown-rows)) '("/p/one" "a" "b")))
          ;; Nothing at work: the clock stops.
          (setf (romacs-agent-state session) 'idle)
          (romacs-agent-board--redraw)
          (cl-letf (((symbol-function 'romacs-agent-board--shown) (lambda () (current-buffer))))
            (romacs-agent-board--tick))
          (should-not romacs-agent-board--ticker))))))

(ert-deftest romacs-agent-board-says-few-words ()
  (romacs-agent-board-tests--with
    (mk "a" "a" 'waiting-approval 10 :driver-state '(:asks ("r")))
    (romacs-agent-board-tests--board
      ;; The counts are the project's, not the title's too.
      (goto-char (point-min))
      (should (equal (buffer-substring-no-properties (point) (line-end-position)) "Agents"))
      (goto-char (point-max))
      (should (string-search "? keys" (buffer-substring-no-properties (line-beginning-position) (point-max)))))))

;;; In a window

(ert-deftest romacs-agent-board-shows-in-the-selected-window ()
  ;; A buffer of its own in the selected window, as wide as it; drawn again,
  ;; each window keeps the row at its point and at its start.
  (romacs-agent-board-tests--with
    (dotimes (i 40) (mk (format "s%02d" i) (format "thread %d" i) 'idle (* 10 (1+ i))))
    (cl-letf (((symbol-function 'romacs-agent-know-sessions) #'ignore)
              ((symbol-function 'romacs-agent-history-list) #'ignore)
              ((symbol-function 'romacs-agent-history-items) #'ignore))
      (save-window-excursion
        (delete-other-windows)
        (unwind-protect
            (progn
              (romacs-agents)
              (should (eq (window-buffer) (get-buffer "*romacs-agents*")))
              (should (derived-mode-p 'romacs-agent-board-mode))
              (should (= romacs-agent-board--width (- (window-body-width) 3)))
              (romacs-agent-board-next 30)
              (set-window-start nil (save-excursion (forward-line -10) (point)))
              (let ((at (plist-get (romacs-agent-board--row) :id))
                    (start (save-excursion (goto-char (window-start)) (plist-get (romacs-agent-board--row) :id))))
                ;; A newer thread lands above both.
                (mk "new" "new" 'idle 1)
                (romacs-agent-board--render (current-buffer))
                (should (equal (plist-get (romacs-agent-board--row) :id) at))
                (should (equal (save-excursion (goto-char (window-start)) (plist-get (romacs-agent-board--row) :id))
                               start))))
          (kill-buffer "*romacs-agents*"))))))

(ert-deftest romacs-agent-board-window-hooks-leave-another-buffer-alone ()
  ;; A window that showed the board and now shows another buffer runs the
  ;; board's window hooks: that buffer is never drawn into.
  (romacs-agent-board-tests--with
    (mk "a" "a" 'idle 10)
    (save-window-excursion
      (with-temp-buffer
        (insert "the user's text")
        (let ((other (current-buffer)))
          (set-window-buffer (selected-window) other)
          (romacs-agent-board--shown-in (selected-window))
          (romacs-agent-board--resized (selected-window))
          (should (equal (with-current-buffer other (buffer-string)) "the user's text")))))))

(provide 'romacs-agent-board-tests)
(ert-deftest romacs-agent-board-a-folded-project-shows-its-heading ()
  ;; Folded, a project is its row alone: how many threads and what they
  ;; are at, a delegated one's state too; a filter shows its threads.
  (romacs-agent-board-tests--with
    (mk "lead" "lead" 'idle 10)
    (mk "w" "w" 'running 20 :options '(:parent "lead"))
    (mk "o" "o" 'idle 30 :cwd "/p/two/")
    (let ((projects (make-hash-table :test #'equal)))
      (puthash "/p/one" t projects)
      (let ((shown (rows :projects projects)))
        (should (equal (romacs-agent-board-tests--ids shown) '("/p/one" "/p/two" "o")))
        (should (= (plist-get (car shown) :folded) 2))
        (let ((line (romacs-agent-board--line (car shown) 100)))
          (should (string-search "▸ 2 threads" line))
          (should (string-search (romacs-agent-ui--rollup '(working)) line))
        ;; States in words, wider than the board: the name stays, the
        ;; line keeps to the width, what is cut says so.
        (let* ((words (romacs-agent-ui--rollup '(approval question working)))
               (line (romacs-agent-board--line (list :kind 'project :cwd "/p/a-long-project-name" :folded 3
                                                     :keys '(approval question working))
                                               40)))
          (should (> (string-width words) 30))
          (should (string-prefix-p "a-long-project-name" line))
          (should (<= (string-width line) 40))
          (should (string-search (substring-no-properties words 0 10) line))
          (should (string-suffix-p (truncate-string-ellipsis) line)))))
      (should (member "lead" (romacs-agent-board-tests--ids (rows :projects projects :words '("lead"))))))))

(ert-deftest romacs-agent-board-keys-fold-projects ()
  ;; TAB on a project folds it and back, S-TAB folds every project, then
  ;; shows them all; the store keeps each.
  (romacs-agent-board-tests--with
    (mk "a" "a" 'idle 10)
    (mk "b" "b" 'idle 20 :cwd "/p/two/")
    (let ((kept (make-hash-table :test #'equal)))
      (cl-letf (((symbol-function 'romacs-agent-store-folded) (lambda () (hash-table-keys kept)))
                ((symbol-function 'romacs-agent-store-fold)
                 (lambda (project folded) (if folded (puthash project t kept) (remhash project kept)))))
        (romacs-agent-board-tests--board
          (should (equal (romacs-agent-board-tests--ids (romacs-agent-board--shown-rows)) '("/p/one" "a" "/p/two" "b")))
          (romacs-agent-board-previous 1)
          (should (equal (romacs-agent-board-tests--at) "/p/one"))
          (romacs-agent-board-fold)
          (should (equal (romacs-agent-board-tests--ids (romacs-agent-board--shown-rows)) '("/p/one" "/p/two" "b")))
          (should (equal (romacs-agent-board-tests--at) "/p/one"))
          (should (equal (hash-table-keys kept) '("/p/one")))
          (romacs-agent-board-fold)
          (should (equal (romacs-agent-board-tests--ids (romacs-agent-board--shown-rows)) '("/p/one" "a" "/p/two" "b")))
          ;; From a thread: every project folds, point on its heading.
          (romacs-agent-board-next 1)
          (romacs-agent-board-cycle-projects)
          (should (equal (romacs-agent-board-tests--ids (romacs-agent-board--shown-rows)) '("/p/one" "/p/two")))
          (should (equal (romacs-agent-board-tests--at) "/p/one"))
          (should (= (hash-table-count kept) 2))
          (romacs-agent-board-cycle-projects)
          (should (equal (romacs-agent-board-tests--ids (romacs-agent-board--shown-rows)) '("/p/one" "a" "/p/two" "b")))
          (should (= (hash-table-count kept) 0)))
        ;; A board shown anew reads what the store kept.
        (puthash "/p/two" t kept)
        (romacs-agent-board-tests--board
          (should (equal (romacs-agent-board-tests--ids (romacs-agent-board--shown-rows)) '("/p/one" "a" "/p/two"))))))))

(ert-deftest romacs-agent-board-folded-heading-past-a-filter-and-narrow ()
  ;; Narrow, a folded heading cuts its path, never its count or state;
  ;; under a filter TAB and S-TAB go by the folds kept; RET on a folded
  ;; project opens the thread it would show first.
  (romacs-agent-board-tests--with
    (mk "a" "alpha" 'running 10 :cwd "/home/someone/Worktrees/romacs/a-long-project-name/")
    (mk "b" "beta" 'idle 20 :cwd "/p/two/")
    (let ((kept (make-hash-table :test #'equal)))
      (puthash "/home/someone/Worktrees/romacs/a-long-project-name" t kept)
      (let* ((row (car (rows :projects kept)))
             (line (romacs-agent-board--line row 40)))
        (should (<= (string-width line) 40))
        ;; What it is at shows before how many, which goes first.
        (should (string-search (romacs-agent-ui--rollup '(working)) line))
        (should (string-search "▸ 1 thread" (romacs-agent-board--line row 60))))
      (cl-letf (((symbol-function 'romacs-agent-store-folded) (lambda () (hash-table-keys kept)))
                ((symbol-function 'romacs-agent-store-fold)
                 (lambda (project folded) (if folded (puthash project t kept) (remhash project kept)))))
        (romacs-agent-board-tests--board
          ;; RET on the folded heading: its thread.
          (goto-char (point-min))
          (romacs-agent-board-next 1)
          (should (plist-get (romacs-agent-board--row) :folded))
          (romacs-agent-board-open)
          (should (equal (romacs-agent-id (car (car shown))) "a"))
          ;; Filtered, both projects show their threads; S-TAB twice folds
          ;; then shows, as kept, and TAB on a kept-folded one shows it.
          (setq romacs-agent-board--words '("a"))
          (romacs-agent-board-cycle-projects)
          (should (= (hash-table-count kept) 2))
          (romacs-agent-board-cycle-projects)
          (should (= (hash-table-count kept) 0))
          (romacs-agent-board-cycle-projects)
          (goto-char (point-min))
          (romacs-agent-board-next 1)
          (romacs-agent-board-fold)
          (should (= (hash-table-count kept) 1)))))))

;;; A job watched by a delegated session

(defun romacs-agent-board-tests--flush-worker-rows ()
  "Run the pending UI batches, including propagation to ancestors."
  (while romacs-agent-ui--children-timer
    (cancel-timer romacs-agent-ui--children-timer)
    (romacs-agent-ui--children-flush)))

(defun romacs-agent-board-tests--activity (session)
  (romacs-agent-log-activity-lines (romacs-agent-children-rows session) 120))

(defun romacs-agent-board-tests--activity-rows (text)
  "Return the rendered thread IDs and their lines in TEXT."
  (cl-loop for line in (split-string text "\n" t)
           for at = (text-property-not-all 0 (length line) 'romacs-agent-log-delegated-thread nil line)
           when at collect (cons (get-text-property at 'romacs-agent-log-delegated-thread line) line)))

(defun romacs-agent-board-tests--word-face (word line)
  "Return the faces on the first character of WORD in LINE."
  (when-let* ((at (string-match (regexp-quote word) line)))
    (ensure-list (get-text-property at 'face line))))

(defvar romacs-agent-log-activity-idle-cap)

(defun romacs-agent-board-tests--more (text)
  "Return the overflow row of rendered activity TEXT, if present."
  (seq-find (lambda (line) (string-match-p "\\+[0-9]+ more\\_>" line))
            (split-string text "\n" t)))

(defun romacs-agent-board-tests--open-more (session text label)
  "Open LABEL in SESSION's rendered activity TEXT and return the board's thread IDs."
  (let ((romacs-agent-board--name "*romacs-board-overflow-test*")
        (romacs-agent-board--ticker nil))
    (unwind-protect
        (save-window-excursion
          (with-temp-buffer
            (switch-to-buffer (current-buffer))
            (romacs-agent-log-mode)
            (setq romacs-agent-log--session session)
            (let ((inhibit-read-only t)) (insert (romacs-agent-log--sealed text)))
            (goto-char (point-min))
            (should (search-forward label nil t))
            (goto-char (match-beginning 0))
            (cl-letf (((symbol-function 'romacs-agent-know-sessions) #'ignore)
                      ((symbol-function 'romacs-agent-history-list) #'ignore)
                      ((symbol-function 'romacs-agent-history-items) #'ignore))
              (let ((action (key-binding (kbd "RET"))))
                (should (and (symbolp action) (fboundp action) (commandp action)))
                (should-not (eq action 'romacs-agent-log-send))
                (call-interactively action)))
            (with-current-buffer (window-buffer (selected-window))
              (should (derived-mode-p 'romacs-agent-board-mode))
              (cl-loop for row in (romacs-agent-board--shown-rows)
                       when (eq (plist-get row :kind) 'thread)
                       collect (plist-get row :id)))))
      (when romacs-agent-board--ticker (cancel-timer romacs-agent-board--ticker))
      (when-let* ((board (get-buffer romacs-agent-board--name))) (kill-buffer board)))))

(defmacro romacs-agent-board-tests--watch-tree (&rest body)
  "Run BODY with lead, lane and writer sessions and a private watch store."
  (declare (indent 0))
  `(romacs-agent-board-tests--with
     (let* ((root (make-temp-file "romacs-board-watch-" t))
            (lead (mk "lead" "Lead" 'idle 3))
            (lane (mk "lane" "Lane" 'idle 2 :options '(:parent "lead")))
            (writer (mk "writer" "Writer" 'idle 1 :dir root :options '(:parent "lane")))
            (romacs-agent-watch--watches (make-hash-table :test #'equal))
            (romacs-agent-watch--timer nil)
            (romacs-agent-ui--announced (make-hash-table :test #'eq))
            (romacs-agent-ui--children-changed (make-hash-table :test #'eq))
            (romacs-agent-ui--children-timer nil)
            (romacs-agent-event-functions '(romacs-agent-ui--on-event)))
       (cl-letf (((symbol-function 'romacs-agent--state-root) (lambda () root))
                 ((symbol-function 'romacs-agent-tools--caller) (lambda () "writer"))
                 ((symbol-function 'romacs-agent-watch--unit-state) (lambda (_) '("active" . "invocation"))))
         (unwind-protect
             (progn
               (mapc #'romacs-agent-ui--children-changed (list writer lane lead))
               (romacs-agent-board-tests--flush-worker-rows)
               (let ((reply (romacs-agent-watch--watch (romacs-mcp--object "unit" "forge-check.service"))))
                 (should (equal (gethash "watching" reply) ["forge-check.service"])))
               (romacs-agent-board-tests--flush-worker-rows)
               ,@body)
           (when romacs-agent-ui--children-timer (cancel-timer romacs-agent-ui--children-timer))
           (when romacs-agent-watch--timer (cancel-timer romacs-agent-watch--timer))
           (delete-directory root t))))))

(ert-deftest romacs-agent-board-watch-keeps-a-lane-visible-in-activity ()
  (romacs-agent-board-tests--watch-tree
    (should (string-match-p "Lane[^\n]*Workers"
                            (romacs-agent-board-tests--activity lead)))
    (should (string-match-p "Writer[^\n]*Job · forge-check\\_>"
                            (romacs-agent-board-tests--activity lane)))
    (should (equal (romacs-agent-board-tests--ids (rows))
                   '("/p/one" "lead" "lane" "writer")))))

(ert-deftest romacs-agent-board-watch-end-keeps-the-idle-lane-visible ()
  (romacs-agent-board-tests--watch-tree
    (let ((before (mapcar #'romacs-agent-ui--state-key (list lead lane)))
          (watch (car (gethash "writer" romacs-agent-watch--watches))))
      (cl-letf (((symbol-function 'romacs-agent-watch--output)
                 (lambda (_program _args then)
                   (funcall then "Id=forge-check.service\nActiveState=inactive\nInvocationID=invocation\n" 0)))
                ((symbol-function 'romacs-agent-tools--tell) #'ignore))
        (should watch)
        (romacs-agent-watch--tick))
      (romacs-agent-board-tests--flush-worker-rows)
      (should-not (gethash "writer" romacs-agent-watch--watches))
      (should (equal (list before (mapcar #'romacs-agent-ui--state-key (list lead lane writer)))
                     '((waiting waiting) (idle idle idle))))
      (dolist (parent (list (cons lead "Lane") (cons lane "Writer")))
        (let ((text (romacs-agent-board-tests--activity (car parent)))
              (title (cdr parent)))
          (should (string-match-p (concat title "[^\n]*\\_<Idle\\_>") text))
          (should (memq 'default
                        (romacs-agent-board-tests--word-face title text)))
          (should-not (text-property-not-all 0 (length text) 'romacs-agent-worker nil text)))))))

(ert-deftest romacs-agent-board-watch-and-live-child-both-show ()
  (romacs-agent-board-tests--watch-tree
    (let ((worker (mk "worker" "Worker" 'starting 0 :options '(:parent "writer"))))
      (setf (romacs-agent-state worker) 'running)
      (romacs-agent-ui--on-event worker '(:type run))
      (romacs-agent-board-tests--flush-worker-rows)
      (should (string-match-p "Worker[^\n]*Working"
                              (romacs-agent-board-tests--activity writer)))
      (should (string-match-p "Writer[^\n]*Job · forge-check\\_>"
                              (romacs-agent-board-tests--activity lane))))))

;;; Delegated threads in chat

(ert-deftest romacs-agent-board-activity-hides-a-thread-when-settled ()
  (romacs-agent-board-tests--with
    (let ((lead (mk "lead" "Lead" 'idle 0))
          (done (mk "done" "Review" 'idle 1 :done-seq 2 :options '(:parent "lead"))))
      (mk "idle" "Standby" 'idle 2 :options '(:parent "lead"))
      (romacs-agent-ui--summarize-children lead)
      (should (assoc "done" (romacs-agent-board-tests--activity-rows
                             (romacs-agent-board-tests--activity lead))))
      (setf (romacs-agent-settled-seq done) 2)
      (romacs-agent-ui--summarize-children lead)
      (should (equal (mapcar #'car (romacs-agent-board-tests--activity-rows
                                   (romacs-agent-board-tests--activity lead)))
                     '("idle"))))))

(ert-deftest romacs-agent-board-activity-lists-live-before-idle-and-finished ()
  (romacs-agent-board-tests--with
    (let ((lead (mk "lead" "Lead" 'idle 0)))
      (mk "idle" "Standby" 'idle 1 :options '(:parent "lead"))
      (mk "done" "Review" 'idle 2 :done-seq 2 :options '(:parent "lead"))
      (mk "failure" "Build" 'idle 3 :done-seq 2 :failed t :options '(:parent "lead"))
      (mk "busy" "Check" 'running 10 :options '(:parent "lead"))
      (romacs-agent-ui--summarize-children lead)
      (let* ((rows (romacs-agent-board-tests--activity-rows
                    (romacs-agent-board-tests--activity lead)))
             (busy (car rows))
             (done (cdr (assoc "done" rows)))
             (failure (cdr (assoc "failure" rows))))
        (should (equal (car busy) "busy"))
        (should (string-match-p "\\_<Working\\_>" (cdr busy)))
        (should (text-property-not-all 0 (length (cdr busy)) 'romacs-agent-worker nil (cdr busy)))
        (should (assoc "idle" (cdr rows)))
        (should (and done (string-prefix-p (romacs-agent-glyph 'completed) done)))
        (should (memq 'default
                      (romacs-agent-board-tests--word-face "Review" done)))
        (should (and failure (string-match-p "\\_<Failed\\_>" failure)))
        (should (seq-intersection '(romacs-agent-error error)
                                  (romacs-agent-board-tests--word-face "Failed" failure)))
        (dolist (line (mapcar #'cdr (cdr rows)))
          (should-not (text-property-not-all 0 (length line) 'romacs-agent-worker nil line)))))))

(ert-deftest romacs-agent-board-activity-heading-includes-overflow ()
  (romacs-agent-board-tests--with
    (let ((lead (mk "lead" "Lead" 'idle 0)))
      (mk "busy" "Check" 'running 5 :options '(:parent "lead"))
      (dotimes (i 4)
        (mk (format "idle-%d" i) (format "Standby %d" i) 'idle i :options '(:parent "lead")))
      (romacs-agent-ui--summarize-children lead)
      (let* ((romacs-agent-log-activity-idle-cap
              (if (boundp 'romacs-agent-log-activity-idle-cap) 2 nil))
             (text (romacs-agent-board-tests--activity lead))
             (shown (length (romacs-agent-board-tests--activity-rows text)))
             (more (romacs-agent-board-tests--more text))
             (hidden (when (and more (string-match "\\+\\([0-9]+\\) more\\_>" more))
                       (string-to-number (match-string 1 more)))))
        (should (equal "Threads · 1 live +2 more" (car (split-string text "\n" t))))
        (should (and hidden (= 5 (+ shown hidden))))))))

(ert-deftest romacs-agent-board-activity-overflow-opens-this-sessions-threads ()
  (romacs-agent-board-tests--with
    (let ((lead (mk "lead" "Lead" 'idle 0))
          (romacs-agent-log-activity-idle-cap
           (if (boundp 'romacs-agent-log-activity-idle-cap) 2 nil)))
      (mk "other" "Other" 'idle 0)
      (mk "other-idle" "Standby" 'idle 0 :options '(:parent "other"))
      (mk "old-idle" "Standby" 'idle 4 :options '(:parent "lead"))
      (mk "new-idle" "Standby" 'idle 1 :options '(:parent "lead"))
      (mk "next-idle" "Standby" 'idle 2 :options '(:parent "lead"))
      (mk "older-idle" "Standby" 'idle 3 :options '(:parent "lead"))
      (dotimes (i 3)
        (mk (format "live-%d" i) (format "Check %d" i) 'running (+ 10 i) :options '(:parent "lead")))
      (romacs-agent-ui--summarize-children lead)
      (let* ((text (romacs-agent-board-tests--activity lead))
             (more (romacs-agent-board-tests--more text)))
        (should (equal (mapcar #'car (romacs-agent-board-tests--activity-rows text))
                       '("live-0" "live-1" "live-2" "new-idle" "next-idle")))
        (should (and more (string-match-p "\\+2 more\\_>" more)))
        (should (memq 'romacs-agent-log-activity-overflow
                      (romacs-agent-board-tests--word-face "+2 more" more)))
        (let ((ids (romacs-agent-board-tests--open-more lead text "+2 more")))
          (should (equal (sort (remove "lead" ids) #'string<)
                         '("live-0" "live-1" "live-2" "new-idle" "next-idle" "old-idle" "older-idle"))))))))

(ert-deftest romacs-agent-board-activity-leaves-header-worker-counts-alone ()
  (romacs-agent-board-tests--with
    (let ((lead (mk "lead" "Lead" 'idle 0)))
      (mk "busy" "Check" 'running 10 :options '(:parent "lead"))
      (mk "idle" "Standby" 'idle 1 :options '(:parent "lead"))
      (mk "done" "Review" 'idle 2 :done-seq 2 :options '(:parent "lead"))
      (mk "failure" "Build" 'idle 3 :done-seq 2 :failed t :options '(:parent "lead"))
      (romacs-agent-ui--summarize-children lead)
      (let* ((rows (romacs-agent-children-rows lead))
             (summary (romacs-agent-ui--workers-summary rows)))
        (should (equal (substring-no-properties summary) "1 failed · 1 threads live"))
        (should (= (length (romacs-agent-board-tests--activity-rows
                            (romacs-agent-board-tests--activity lead)))
                   4))))))

(ert-deftest romacs-agent-board-activity-and-menu-show-the-same-threads-in-order ()
  (romacs-agent-board-tests--with
    (let ((lead (mk "lead" "Lead" 'idle 0))
          (lane (mk "lane" "Lane" 'idle 2 :options '(:parent "lead"))))
      (mk "idle" "Standby" 'idle 1 :options '(:parent "lead"))
      (mk "busy" "Check" 'running 3 :options '(:parent "lead"))
      (mk "nested" "Builder" 'running 4 :options '(:parent "lane"))
      (romacs-agent-ui--summarize-children lane)
      (romacs-agent-ui--summarize-children lead)
      (with-temp-buffer
        (setq-local romacs-agent-log--session lead)
        (let ((panel (mapcar #'car (romacs-agent-board-tests--activity-rows
                                   (romacs-agent-board-tests--activity lead))))
              (menu (cl-loop for i below 9
                             while (romacs-agent-menu--worker-label i)
                             collect (romacs-agent-id (aref (romacs-agent-menu--slots) i)))))
          (should (equal menu panel))
          (should (equal panel '("busy" "lane" "idle"))))))))

(defmacro romacs-agent-board-tests--with-store (&rest body)
  "Run BODY with a private SQLite store and worker refresh batch."
  (declare (indent 0))
  `(romacs-agent-board-tests--with
     (let* ((root (make-temp-file "romacs-thread-store-" t))
            (romacs-agent-store--db nil)
            (romacs-agent-ui--unloaded-waiting (make-hash-table :test #'equal))
            (romacs-agent-ui--announced (make-hash-table :test #'eq))
            (romacs-agent-ui--children-changed (make-hash-table :test #'eq))
            (romacs-agent-ui--children-timer nil)
            (romacs-agent-event-functions '(romacs-agent-ui--on-event)))
       (cl-letf (((symbol-function 'romacs-agent--state-root) (lambda () root)))
         (unwind-protect (progn ,@body)
           (when romacs-agent-ui--children-timer (cancel-timer romacs-agent-ui--children-timer))
           (when (cdr romacs-agent-store--db) (sqlite-close (cdr romacs-agent-store--db)))
           (delete-directory root t))))))

(ert-deftest romacs-agent-board-round-two-reopened-chat-includes-stored-children ()
  (romacs-agent-board-tests--with-store
    (let ((lead (mk "lead" "Lead" 'idle 0 :dir root))
          (romacs-agent-log-activity-idle-cap 3))
      (dotimes (i 4)
        (let* ((id (format "stored-%d" i))
               (child (mk id id 'idle i :dir root :done-seq 2 :options '(:parent "lead"))))
          (should (romacs-agent-store--put child nil))
          (remhash id romacs-agent--sessions)))
      (let* ((romacs-agent-log--chats (make-hash-table :test #'equal))
             (chat (romacs-agent-log-chat lead)))
        (unwind-protect
            (with-current-buffer chat
              (should (equal (mapcar #'romacs-agent-id (romacs-agent-menu--workers))
                             '("stored-0" "stored-1" "stored-2" "stored-3"))))
          (kill-buffer chat)))
      (let ((text (romacs-agent-board-tests--activity lead)))
        (should (equal (mapcar #'car (romacs-agent-board-tests--activity-rows text))
                       '("stored-0" "stored-1" "stored-2")))
        (should (romacs-agent-board-tests--more text))))))

(ert-deftest romacs-agent-board-round-two-watch-crosses-unloaded-ancestors ()
  (romacs-agent-board-tests--with-store
    (let* ((lead (mk "lead" "Lead" 'detached 0 :dir (expand-file-name "lead" root)))
           (middle (mk "middle" "Middle" 'detached 1 :dir (expand-file-name "middle" root) :options '(:parent "lead")))
           (lane (mk "lane" "Lane" 'detached 2 :dir (expand-file-name "lane" root) :options '(:parent "middle")))
           (writer (mk "writer" "Writer" 'idle 3 :dir (expand-file-name "writer" root) :options '(:parent "lane")))
           (romacs-agent-watch--watches (make-hash-table :test #'equal))
           (romacs-agent-watch--timer nil))
      (make-directory (romacs-agent-dir writer))
      (dolist (parent (list middle lane))
        (should (romacs-agent-store--put parent nil))
        (remhash (romacs-agent-id parent) romacs-agent--sessions))
      (romacs-agent--write-json (romacs-agent-watch--file writer)
                               (vector (romacs-mcp--object "unit" "check.service")))
      (unwind-protect
          (progn
            (romacs-agent-watch--restore)
            (romacs-agent-board-tests--flush-worker-rows)
            (should (eq (romacs-agent-ui--state-key lead) 'waiting))
            (should (equal (romacs-agent-board-tests--ids
                            (romacs-agent-board-rows (romacs-agent-board--sessions) nil
                                                     (list :folds (make-hash-table :test #'equal))))
                           '("/p/one" "lead" "middle" "lane" "writer")))
            (romacs-agent-watch--forget "writer" (gethash "writer" romacs-agent-watch--watches))
            (romacs-agent-board-tests--flush-worker-rows)
            (should (eq (romacs-agent-ui--state-key lead) 'detached)))
        (when romacs-agent-watch--timer (cancel-timer romacs-agent-watch--timer))))))

(ert-deftest romacs-agent-board-round-two-stop-retires-watches ()
  (romacs-agent-board-tests--watch-tree
    (let ((watch (car (gethash "writer" romacs-agent-watch--watches))) told)
      (romacs-agent-stop writer t)
      (should (equal (romacs-agent-watch-unit writer) "forge-check.service"))
      (romacs-agent-stop writer)
      (romacs-agent-board-tests--flush-worker-rows)
      (cl-letf (((symbol-function 'romacs-agent-tools--tell) (lambda (&rest _) (setq told t))))
        (romacs-agent-watch--report "writer" watch))
      (should-not told)
      (should-not (gethash "writer" romacs-agent-watch--watches))
      (should-not (file-exists-p (romacs-agent-watch--file writer)))
      (should (equal (mapcar #'romacs-agent-ui--state-key (list lead lane writer)) '(idle idle ended))))))

(ert-deftest romacs-agent-board-failed-retirement-keeps-the-active-watch ()
  (romacs-agent-board-tests--watch-tree
    (let ((file (romacs-agent-watch--file writer))
          (delete (symbol-function 'delete-file)))
      (cl-letf (((symbol-function 'delete-file)
                 (lambda (name &rest args)
                   (if (equal name file) (signal 'file-error (list "Read-only" name))
                     (apply delete name args)))))
        (should-error (romacs-agent-watch--watch
                       (romacs-mcp--object "unit" "forge-check.service" "stop" t))
                      :type 'file-error))
      (romacs-agent-board-tests--flush-worker-rows)
      (should (equal (mapcar #'romacs-agent-ui--state-key (list lead lane writer))
                     '(waiting waiting watching)))
      (should romacs-agent-watch--timer)
      (clrhash romacs-agent-watch--watches)
      (romacs-agent-watch--restore)
      (should (equal (romacs-agent-watch-unit writer) "forge-check.service"))
      (romacs-agent-watch--watch (romacs-mcp--object "unit" "forge-check.service" "stop" t))
      (should-not (file-exists-p file)))))

(ert-deftest romacs-agent-board-relaunch-without-unit-keeps-the-watch ()
  (romacs-agent-board-tests--watch-tree
    (let (continued)
      (should-not (romacs-agent-unit writer))
      (romacs-agent--stop-then writer (lambda () (setq continued t)))
      (should continued)
      (should (equal (romacs-agent-watch-unit writer) "forge-check.service"))
      (should (file-exists-p (romacs-agent-watch--file writer))))))

(ert-deftest romacs-agent-board-settling-a-stored-child-refreshes-chat-rows ()
  (romacs-agent-board-tests--with-store
    (let* ((lead (mk "lead" "Lead" 'idle 0 :dir root))
           (dir (expand-file-name "stored" root))
           (child (mk "stored" "Stored" 'idle 1 :dir dir :done-seq 2 :options '(:parent "lead"))))
      (make-directory dir)
      (romacs-agent--write-json (expand-file-name "manifest.json" dir) (romacs-mcp--object "id" "stored"))
      (should (romacs-agent-store--put child nil))
      (remhash "stored" romacs-agent--sessions)
      (romacs-agent-ui--summarize-children lead)
      (should (assoc "stored" (romacs-agent-board-tests--activity-rows (romacs-agent-board-tests--activity lead))))
      (should (romacs-agent-settle-recorded "stored" t))
      (romacs-agent-board-tests--flush-worker-rows)
      (should (romacs-agent-settled-p (romacs-agent-store-row-session "stored")))
      (should-not (assoc "stored" (romacs-agent-board-tests--activity-rows (romacs-agent-board-tests--activity lead))))
      (should-not (romacs-agent-loaded "stored")))))

(ert-deftest romacs-agent-board-stored-failure-keeps-its-outcome-in-chat ()
  (romacs-agent-board-tests--with-store
    (let ((lead (mk "lead" "Lead" 'idle 0 :dir root))
          (child (mk "failed" "Build" 'idle 1 :dir root :done-seq 2 :failed t :options '(:parent "lead"))))
      (should (romacs-agent-store--put child nil))
      (remhash "failed" romacs-agent--sessions)
      (romacs-agent-ui--summarize-children lead)
      (let ((line (cdr (assoc "failed" (romacs-agent-board-tests--activity-rows
                                      (romacs-agent-board-tests--activity lead))))))
        (should (and line (string-prefix-p "✗" line) (string-search "Failed" line)))))))

;;; romacs-agent-board-tests.el ends here
