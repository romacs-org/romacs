;;; romacs-agent-writer-tests.el --- The agents' store written off the editor -*- lexical-binding: t; -*-
;; SPDX-License-Identifier: GPL-3.0-or-later
;; Maintainer: Thanos Apollo <public@thanosapollo.org>
(require 'ert)
(require 'cl-lib)
(require 'romacs-mcp)
(let ((romacs-mcp-call-function romacs-mcp-call-function)
      (romacs-mcp-tools (copy-alist romacs-mcp-tools)))
  (require 'romacs-agent)
  (require 'romacs-agent-claude)
  (require 'romacs-agent-events)
  (require 'romacs-agent-history))

(defmacro romacs-agent-writer-tests--with (&rest body)
  "BODY as the editor, its writes queued to a writer that runs here: what
it is sent waits until `serve' runs the writer's loop over it, with a
connection of its own, as the writer's process has."
  (declare (indent 0))
  `(let ((sent nil) (romacs-agent-writer-queued t) (romacs-agent-writer--job nil)
         (romacs-agent-writer--pending (make-hash-table)) (romacs-agent-writer--unsent nil)
         (romacs-agent-writer--unsent-tail nil) (romacs-agent-writer--next 0) (romacs-agent-writer--in-flight 0))
     (cl-letf (((symbol-function 'isolate-spawn) (lambda (&rest _) (vector 'future)))
               ((symbol-function 'isolate-on-done) #'ignore)
               ((symbol-function 'isolate-future-done-p) #'ignore)
               ((symbol-function 'isolate-future-isolate) (lambda (_) 'writer))
               ((symbol-function 'isolate-send)
                (lambda (to message)
                  (if (eq to 'parent) (romacs-agent-writer--receive 'writer message)
                    (setq sent (append sent (list message)))))))
       (cl-labels ((serve-only ()
                   (let ((inbox sent) (romacs-agent-store--db nil) (romacs-agent-events--made nil)
                         (romacs-agent-turns--made nil) (romacs-agent-store--readonly nil))
                     (setq sent nil)
                     (cl-letf (((symbol-function 'isolate-receive)
                                (lambda (&rest _) (or (pop inbox) '(romacs-agent-writer-stop)))))
                       (unwind-protect (romacs-agent-writer--serve)
                         (when (cdr romacs-agent-store--db) (sqlite-close (cdr romacs-agent-store--db)))))))
                 (serve ()
                   (setq sent (append sent (list '(romacs-agent-writer-stop))))
                   (serve-only)))
         ,@body))))

(ert-deftest romacs-agent-writer-writes-in-order-and-answers ()
  ;; Queued, a write answers later, in the order written, its value or
  ;; its error; nothing waits for it.
  (let ((romacs-agent-writer-start-functions nil) (answers nil))
    (romacs-agent-writer-tests--with
      (should (eq (romacs-agent-write 'list '(1) (lambda (answer) (push answer answers))) 'queued))
      (romacs-agent-write 'error '("Boom") (lambda (answer) (push (car answer) answers)))
      (should-not answers)
      (should (= (romacs-agent-writer-pending-count) 2))
      (serve)
      (should (equal (nreverse answers) '((ok (1)) error)))
      (should (zerop (romacs-agent-writer-pending-count))))
    ;; Not queued (a batch process, a worker): here, now.
    (let ((romacs-agent-writer-queued nil))
      (should (equal (romacs-agent-write 'list '(2)) '(ok (2)))))))

(ert-deftest romacs-agent-writer-a-writer-that-ends-gets-the-rest-again ()
  ;; The writer gone with writes unanswered: a new one is sent them again,
  ;; in order, and each is answered once.
  (let ((romacs-agent-writer-start-functions nil) (answers nil))
    (romacs-agent-writer-tests--with
      (romacs-agent-write 'list '(1) (lambda (answer) (push answer answers)))
      (romacs-agent-write 'list '(2) (lambda (answer) (push answer answers)))
      (setq sent nil)
      (romacs-agent-writer--ended romacs-agent-writer--job)
      (should (equal (mapcar #'car (cadr (car sent))) '(1 2)))
      (serve)
      (should (equal (nreverse answers) '((ok (1)) (ok (2))))))))

(defvar romacs-agent-writer-tests--made nil "What the writer made, in order.")
(defun romacs-agent-writer-tests--make (x) (push x romacs-agent-writer-tests--made) x)

(ert-deftest romacs-agent-writer-a-drain-makes-every-write ()
  ;; Ending (or handing over to a successor), the editor waits for every
  ;; write it queued, however many, saying so; a writer that died is
  ;; started again with what it did not answer.
  (let ((romacs-agent-writer-start-functions nil) (romacs-agent-writer-tests--made nil) (said nil))
    (romacs-agent-writer-tests--with
      (cl-letf (((symbol-function 'isolate-await) (lambda (&rest _) (serve) t))
                ((symbol-function 'redisplay) #'ignore)
                ((symbol-function 'message) (lambda (format &rest args) (push (apply #'format format args) said))))
        (dotimes (i 500) (romacs-agent-write 'romacs-agent-writer-tests--make (list i)))
        (romacs-agent-writer-drain)
        (should (equal (reverse romacs-agent-writer-tests--made) (number-sequence 0 499)))
        (should (zerop (romacs-agent-writer-pending-count)))
        (should (member "Saving 500 agent records..." said))
        (should (member "Saving 500 agent records...done" said))
        ;; The writer gone with two unanswered: a new one makes them.
        (setq romacs-agent-writer-tests--made nil)
        (romacs-agent-write 'romacs-agent-writer-tests--make '(a))
        (romacs-agent-write 'romacs-agent-writer-tests--make '(b))
        (setq sent nil)
        (let ((dead romacs-agent-writer--job))
          (cl-letf (((symbol-function 'isolate-future-done-p) (lambda (future) (eq future dead))))
            (romacs-agent-writer-drain)))
        (should (equal (reverse romacs-agent-writer-tests--made) '(a b)))
        (should (zerop (romacs-agent-writer-pending-count)))
        ;; A write's answer that queues another: that one is made too,
        ;; before the writer stops.
        (setq romacs-agent-writer-tests--made nil)
        (romacs-agent-write 'romacs-agent-writer-tests--make '(c)
                            (lambda (_) (romacs-agent-write 'romacs-agent-writer-tests--make '(d))))
        (cl-letf (((symbol-function 'isolate-await)
                   (lambda (&rest _)
                     ;; Served as answers come; the stop, last, ends it.
                     (unless (member '(romacs-agent-writer-stop) sent) (serve-only))
                     t)))
          (romacs-agent-writer-drain))
        (should (equal (reverse romacs-agent-writer-tests--made) '(c d)))
        (should (zerop (romacs-agent-writer-pending-count)))))))

(ert-deftest romacs-agent-writer-a-burst-waits-for-room-never-errs ()
  ;; Thousands of writes at once: never more batches in the writer's
  ;; queue than the window, never an error to the caller, all made, in order.
  (let ((romacs-agent-writer-start-functions nil) (romacs-agent-writer-tests--made nil) (rounds 0))
    (romacs-agent-writer-tests--with
      (dotimes (i 20000) (should (eq (romacs-agent-write 'romacs-agent-writer-tests--make (list i)) 'queued)))
      (should (<= (length sent) romacs-agent-writer--window))
      (while (and (> (romacs-agent-writer-pending-count) 0) (< (cl-incf rounds) 100))
        (should (<= (length sent) romacs-agent-writer--window))
        (serve-only))
      (should (zerop (romacs-agent-writer-pending-count)))
      (should (equal (reverse romacs-agent-writer-tests--made) (number-sequence 0 19999)))
      ;; Nothing left: the writer only stops.
      (serve))))

(ert-deftest romacs-agent-writer-notes-are-made-before-a-launch ()
  ;; After a move, what waits runs in two steps: the notes' writes, then,
  ;; once the writer has answered a write behind them, the launch.
  (let ((romacs-agent-writer-start-functions nil) (romacs-agent-writer-tests--made nil)
        (romacs-agent-events--waiting (make-hash-table :test #'equal)) (launched nil))
    (romacs-agent-writer-tests--with
      (puthash "s" (list (cons t (lambda () (romacs-agent-write 'romacs-agent-writer-tests--make '(note))))
                         (cons nil (lambda () (setq launched (reverse romacs-agent-writer-tests--made)))))
               romacs-agent-events--waiting)
      (romacs-agent-events--waited "s")
      (should-not launched)
      (serve)
      (should (equal launched '(note))))))

(ert-deftest romacs-agent-writer-a-flip-made-before-a-writer-ended-is-moved ()
  ;; A flip the writer made before it ended, sent again, answers nil: the
  ;; store says imported, so the editor does what a move does.
  (let ((romacs-agent-events--flip-queue nil) (waited nil) (told nil)
        (romacs-agent--sessions (make-hash-table :test #'equal)))
    (cl-letf (((symbol-function 'romacs-agent-events--db) (lambda () 'db))
              ((symbol-function 'sqlite-select) (lambda (_db query &rest _) (if (string-search "imported" query) '((1)) '((10 3)))))
              ((symbol-function 'romacs-agent-events--back-up-moved) #'ignore)
              ((symbol-function 'romacs-agent-events--waited) (lambda (id) (push id waited)))
              ((symbol-function 'romacs-agent-write) (lambda (_f _a done) (funcall done '(ok nil))))
              ((symbol-function 'run-at-time) (lambda (_time _repeat f &rest args) (apply f args))))
      (let ((romacs-agent-events-migrated-functions (list (lambda (ids) (setq told ids)))))
        (romacs-agent-events--flip-soon '("s")))
      (should (equal told '("s")))
      (should (equal waited '("s"))))))

(ert-deftest romacs-agent-writer-the-editor-reads-and-an-import-waits ()
  ;; The editor's connection only reads; a launch on a session whose
  ;; journal is imported by the writer waits for it, and finds it imported.
  (skip-unless (and (fboundp 'sqlite-available-p) (sqlite-available-p)))
  (let* ((root (make-temp-file "romacs-writer-" t))
         (process-environment (append (list (concat "XDG_STATE_HOME=" root "/state") (concat "XDG_RUNTIME_DIR=" root "/run"))
                                      process-environment))
         (romacs-agent-store--db nil) (romacs-agent-store--readonly nil) (romacs-agent-store--making nil)
         (romacs-agent-events--made nil) (romacs-agent-turns--made nil)
         (romacs-agent-events--imported (make-hash-table :test #'equal))
         (romacs-agent-events--importing (make-hash-table :test #'equal))
         (romacs-agent-events--waiting (make-hash-table :test #'equal))
         (romacs-agent-events-store t)
         (id "e7e70000-2222-4333-8444-5555666677aa")
         (dir (expand-file-name id (romacs-agent--state-root)))
         (session (romacs-agent--make :id id :dir dir)))
    (unwind-protect
        (romacs-agent-writer-tests--with
          (make-directory dir t)
          (let ((coding-system-for-write 'utf-8-unix))
            (write-region "{\"note\":{\"type\":\"run\"},\"seq\":1,\"t\":1.0}\n" nil (expand-file-name "journal.jsonl" dir) nil 'silent))
          ;; No store yet: none to read, the writer asked to make it.
          (should-not (romacs-agent-store--open))
          (should (equal (mapcar #'car sent) '(romacs-agent-write-batch)))
          (serve)
          (let ((db (romacs-agent-store--open)))
            (should db)
            (should (romacs-agent-store-readonly-p))
            (should-error (sqlite-execute db "insert into board_folds values ('/p')")))
          ;; Opened to write before writes were queued (early in a start):
          ;; opened again, to read, once they are.
          (let ((romacs-agent-writer-queued nil)) (should (romacs-agent-store--open)) (should-not (romacs-agent-store-readonly-p)))
          (should (romacs-agent-store--open))
          (should (romacs-agent-store-readonly-p))
          ;; A launch waits for the writer's import.
          (let ((ran nil))
            (romacs-agent-events-then session (lambda () (push (romacs-agent-events-p session) ran)))
            (should-not ran)
            (should-not (romacs-agent-events-p session))
            (serve)
            (should (equal ran '(t)))
            (should (equal (caar (sqlite-select (romacs-agent-store--open) "select count(*) from events where session = ?" (list id))) 1))
            ;; Its journal backed up here, once imported.
            (should-not (file-exists-p (expand-file-name "journal.jsonl" dir)))))
      (when (cdr romacs-agent-store--db) (ignore-errors (sqlite-close (cdr romacs-agent-store--db))))
      (delete-directory root t))))

(provide 'romacs-agent-writer-tests)
;;; romacs-agent-writer-tests.el ends here
