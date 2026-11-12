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
  (require 'romacs-agent-board))

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
    (should (equal (romacs-agent-board-tests--ids (rows)) '("/p/one" "b")))
    (should (equal (romacs-agent-board-tests--ids (rows :settled t)) '("/p/one" "a")))))

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

(ert-deftest romacs-agent-board-a-line-says-provider-and-model ()
  (romacs-agent-board-tests--with
    (mk "a" "Port the theme" 'running 30 :model "claude-opus-5-5")
    (let ((line (substring-no-properties (romacs-agent-board--line (cadr (rows)) 100))))
      (should (= (string-width line) 100))
      (should (string-match-p "Working +Port the theme +claude · opus-5-5 +now\\'" line)))))

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

;;; Keys, on the board's buffer (the layer is the popup's)

(defmacro romacs-agent-board-tests--board (&rest body)
  "BODY in the board's buffer, drawn, with `romacs-popup-show' and
`romacs-popup-then' recorded instead of shown or deferred."
  (declare (indent 0))
  `(let ((then nil))
     (with-temp-buffer
       (romacs-agent-board-mode)
       (cl-letf (((symbol-function 'romacs-popup-show) #'ignore)
                 ((symbol-function 'romacs-popup-then) (lambda (function) (push function then)))
                 ((symbol-function 'romacs-agent-board--columns) (lambda () 100)))
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
        ;; RET opens its chat, in the user's window, after the board hides.
        (romacs-agent-board-open)
        (should (= (length then) 1))
        (let ((shown nil))
          (cl-letf (((symbol-function 'romacs-agent-show) (lambda (session) (setq shown session))))
            (funcall (car then)))
          (should (eq shown lead)))))))

(ert-deftest romacs-agent-board-keys-open-what-is-there ()
  (romacs-agent-board-tests--with
    (let ((a (mk "a" "a" 'idle 10)))
      (romacs-agent-board-tests--board
        ;; A project opens its first thread.
        (goto-char (point-min))
        (romacs-agent-board-next 1)
        (romacs-agent-board-open)
        (should (= (length then) 1))
        ;; A thread no longer known: the board is drawn again.
        (remhash "a" romacs-agent--sessions)
        (romacs-agent-board-next 1)
        (should (eq (plist-get (romacs-agent-board--row) :session) a))
        (should-error (romacs-agent-board-open) :type 'user-error)
        (should-not (romacs-agent-board--shown-rows))
        (should (= (length then) 1))))))

(ert-deftest romacs-agent-board-keys-approve ()
  (romacs-agent-board-tests--with
    (mk "tool" "tool" 'waiting-approval 10 :driver-state '(:asks ("r1")))
    (mk "ask" "ask" 'waiting-approval 20 :driver-state '(:asks ("q1") :questions ("q1")))
    (romacs-agent-board-tests--board
      (let ((answered nil))
        (cl-letf (((symbol-function 'romacs-agent-respond)
                   (lambda (session id allow) (push (list (romacs-agent-id session) id allow) answered))))
          (should (equal (romacs-agent-board-tests--at) "tool"))
          (romacs-agent-board-approve)
          (should (equal answered '(("tool" "r1" t))))
          ;; A question is answered in its chat.
          (romacs-agent-board-next 1)
          (romacs-agent-board-approve)
          (should (= (length answered) 1))
          (should (= (length then) 1)))))))

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
      (cl-letf (((symbol-function 'romacs-agent-board--show) (lambda (_) (cl-incf drawn))))
        (cl-letf (((symbol-function 'romacs-agent-board--shown) #'ignore))
          (run-hooks 'romacs-agent-ui-changed-hook))
        (should (= drawn 0))
        (cl-letf (((symbol-function 'romacs-agent-board--shown) (lambda () (current-buffer))))
          (run-hooks 'romacs-agent-ui-changed-hook))
        (should (= drawn 1))))))

(provide 'romacs-agent-board-tests)
;;; romacs-agent-board-tests.el ends here
