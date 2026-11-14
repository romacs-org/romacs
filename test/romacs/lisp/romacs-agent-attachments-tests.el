;;; romacs-agent-attachments-tests.el --- Files an agent attaches to its reply -*- lexical-binding: t; -*-
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
  (require 'romacs-agent-tools))

(defmacro romacs-agent-attachments-tests--with (&rest body)
  "BODY with `session' calling, recorded under `root', and `before',
`after' (images) and `notes' (a text file) there; `events' collects what
its journal's records raise."
  (declare (indent 0))
  `(let* ((root (make-temp-file "romacs-attach-" t))
          (romacs-agent--sessions (make-hash-table :test #'equal))
          (session (romacs-agent--make :id "a" :harness 'claude :dir root :caught-up t))
          (before (expand-file-name "before.png" root)) (after (expand-file-name "after.png" root))
          (notes (expand-file-name "notes.txt" root))
          (events nil))
     (puthash "a" session romacs-agent--sessions)
     (dolist (file (list before after))
       (let ((coding-system-for-write 'binary))
         (write-region (concat "\x89PNG\r\n\x1a\n" (make-string 100 ?\0)) nil file nil 'silent)))
     (write-region "findings\n" nil notes nil 'silent)
     (cl-letf (((symbol-function 'romacs-agent-tools--caller) (lambda () "a")))
       (let ((romacs-agent-event-functions (list (lambda (_s event) (push event events)))))
         (unwind-protect (progn ,@body) (delete-directory root t))))))

(defun romacs-agent-attachments-tests--attach (&rest arguments)
  "The tool's answer to ARGUMENTS, its deferred answer waited for."
  (let ((deferred (romacs-agent-attachments--tool (apply #'romacs-mcp--object arguments)))
        (answer nil) (end (+ (float-time) 30)))
    (should (eq (car deferred) 'romacs-mcp--deferred))
    (while (and (not (setq answer (funcall (nth 1 deferred)))) (< (float-time) end))
      (accept-process-output nil 0.05))
    (car answer)))

(ert-deftest romacs-agent-attachments-go-with-the-reply ()
  ;; Referenced where they are: journaled as a note, raised in the turn as
  ;; an `attachments' event, a before-and-after pair marked.
  (romacs-agent-attachments-tests--with
    (let ((answer (romacs-agent-attachments-tests--attach
                   "paths" (vector before after notes) "alt" (vector "before" "after" :null)
                   "compare" (vector before after))))
      (should (= (length (gethash "attached" answer)) 3))
      (let ((event (cl-find 'attachments events :key (lambda (e) (plist-get e :type)))))
        (should event)
        (should (equal (mapcar (lambda (a) (list (plist-get a :path) (plist-get a :kind) (plist-get a :alt)))
                               (plist-get event :attachments))
                       (list (list before 'image "before") (list after 'image "after") (list notes 'file nil))))
        (should (= (plist-get (nth 2 (plist-get event :attachments)) :size) 9))
        (when (executable-find "sha256sum")
          (should (string-match-p "\\`[0-9a-f]\\{64\\}\\'" (plist-get (car (plist-get event :attachments)) :sha))))
        (should (equal (plist-get event :compare) (list before after))))
      ;; Not copied: the journal names them where they are.
      (should (equal (directory-files root nil "\\.png\\'") '("after.png" "before.png")))
      (let ((note (gethash "note" (car (last (romacs-agent--journal-records session))))))
        (should (equal (gethash "type" note) "attachments"))))))

(ert-deftest romacs-agent-attachments-refusals ()
  (romacs-agent-attachments-tests--with
    (should-error (romacs-agent-attachments-tests--attach "paths" (vector "relative.png")) :type 'user-error)
    (should-error (romacs-agent-attachments-tests--attach "paths" (vector (expand-file-name "gone" root))) :type 'user-error)
    (should-error (romacs-agent-attachments-tests--attach "paths" (vector before after) "alt" "one") :type 'user-error)
    (should-error (romacs-agent-attachments-tests--attach "paths" (vector before) "compare" (vector before after))
                  :type 'user-error)
    (should-error (romacs-agent-attachments-tests--attach "paths" []) :type 'user-error)
    (cl-letf (((symbol-function 'romacs-agent-tools--caller) #'ignore))
      (should-error (romacs-agent-attachments-tests--attach "paths" (vector before)) :type 'user-error))
    (should-not (cl-find 'attachments events :key (lambda (e) (plist-get e :type))))))

(ert-deftest romacs-agent-attachments-links-names-and-places ()
  ;; A link's size is its target's, as its hash is; any file name hashes;
  ;; a /proc file is no file to attach.
  (romacs-agent-attachments-tests--with
    (let ((target (expand-file-name "artifact.bin" root)) (link (expand-file-name "latest.bin" root))
          (odd (expand-file-name "a\\b\nc.txt" root)))
      (write-region (make-string 4096 ?x) nil target nil 'silent)
      (make-symbolic-link target link)
      (write-region "odd" nil odd nil 'silent)
      (let ((items (gethash "attached" (romacs-agent-attachments-tests--attach "paths" (vector link odd)))))
        (should (= (gethash "size" (aref items 0)) 4096))
        (should (equal (gethash "path" (aref items 0)) link))
        (when (executable-find "sha256sum")
          (should (string-match-p "\\`[0-9a-f]\\{64\\}\\'" (gethash "sha" (aref items 1)))))))
    (should-error (romacs-agent-attachments--tool (romacs-mcp--object "paths" (vector "/proc/self/status")))
                  :type 'user-error)))

(ert-deftest romacs-agent-attachments-answered-once-journaled ()
  ;; Its harness live: the note goes through its supervisor, and the tool
  ;; answers only once it is read back, its place in the turn known.
  (romacs-agent-attachments-tests--with
    (let ((sent nil))
      (cl-letf (((symbol-function 'romacs-agent--note) (lambda (_s note) (setq sent note))))
        (let* ((deferred (romacs-agent-attachments--tool (romacs-mcp--object "paths" (vector notes))))
               (end (+ (float-time) 30)))
          (while (and (not sent) (< (float-time) end)) (accept-process-output nil 0.05))
          (should sent)
          (should-not (funcall (nth 1 deferred)))
          ;; Another call's note read back answers this one nothing.
          (romacs-agent--raise session (list :type 'attachments :id "other" :attachments nil))
          (should-not (funcall (nth 1 deferred)))
          (romacs-agent--raise session (list :type 'attachments :id (gethash "id" sent) :attachments nil))
          (should (gethash "attached" (car (funcall (nth 1 deferred))))))))))

(ert-deftest romacs-agent-attachments-a-call-lets-go-of-what-it-owns ()
  ;; Cancelled while hashing: its process, timers and waiter go, and
  ;; nothing is journaled after.  A file rewritten while hashed is
  ;; refused.  No sha256sum: attached with no hash.  Its harness ending
  ;; before the note is read back: an error, its waiter gone.
  (romacs-agent-attachments-tests--with
    (let* ((request (list :peer nil :cancelled nil))
           (romacs-mcp--active request)
           (noted nil))
      (cl-letf* ((make (symbol-function 'make-process))
                 ((symbol-function 'romacs-mcp--live-p) (lambda (&rest _) t))
                 ((symbol-function 'romacs-agent--note) (lambda (&rest _) (setq noted t)))
                 ;; A hash that takes its time.
                 ((symbol-function 'make-process)
                  (lambda (&rest args)
                    (apply make (plist-put (copy-sequence args) :command '("sleep" "30"))))))
        (let ((deferred (romacs-agent-attachments--tool (romacs-mcp--object "paths" (vector notes)))))
          (plist-put request :cancelled t)
          (let ((end (+ (float-time) 5)) (answer nil))
            (while (and (not answer) (< (float-time) end))
              (accept-process-output nil 0.1)
              (setq answer (condition-case err (funcall (nth 1 deferred)) (user-error (list :refused (cadr err))))))
            (should (equal (car answer) :refused)))
          (should-not noted)
          (should-not (cl-some (lambda (p) (and (process-live-p p) (string-prefix-p "romacs-attach-sha" (process-name p))))
                               (process-list))))))
    ;; Rewritten between its check and its hash.
    (cl-letf* ((hash (symbol-function 'romacs-agent-attachments--hash))
               ((symbol-function 'romacs-agent-attachments--hash)
                (lambda (job file size then)
                  (write-region (make-string 37 ?y) nil file nil 'silent)
                  (set-file-times file (time-add (current-time) 5))
                  (funcall hash job file size then))))
      (should (string-match-p "changed while it was attached"
                              (cadr (should-error (romacs-agent-attachments-tests--attach "paths" (vector notes))
                                                  :type 'user-error)))))
    ;; No hasher to run.
    (write-region "findings\n" nil notes nil 'silent)
    (let ((exec-path nil) (process-environment (cons "PATH=/nonexistent" process-environment)))
      (should (eq (gethash "sha" (aref (gethash "attached" (romacs-agent-attachments-tests--attach "paths" (vector notes))) 0))
                  :null)))
    ;; Its harness ends before its note is read back.
    (cl-letf (((symbol-function 'romacs-agent--note)
               (lambda (s _note) (romacs-agent--raise s (list :type 'exit :status 1 :seq 90)))))
      (should (string-match-p "harness ended"
                              (cadr (should-error (romacs-agent-attachments-tests--attach "paths" (vector notes))
                                                  :type 'user-error)))))
    ;; Only the fixture's own collector is left: no waiter.
    (should (= (length romacs-agent-event-functions) 1))))

(ert-deftest romacs-agent-attachments-cancelled-or-ended-while-hashing ()
  ;; Cancelled, or its session's harness ending, while its files hash: it
  ;; answers no success and journals nothing, whenever the hash ends.
  (romacs-agent-attachments-tests--with
    (dolist (stop (list (lambda (request _s) (plist-put request :cancelled t))
                        (lambda (_request s) (romacs-agent--raise s (list :type 'exit :status 1 :seq 70)))))
      (let* ((request (list :peer nil :cancelled nil))
             (romacs-mcp--active request)
             (noted nil) (settle nil))
        (cl-letf (((symbol-function 'romacs-mcp--live-p) (lambda (&rest _) t))
                  ((symbol-function 'romacs-agent--note) (lambda (&rest _) (setq noted t)))
                  ((symbol-function 'romacs-agent-attachments--hash)
                   (lambda (_job _file _size then) (setq settle then))))
          (let ((deferred (romacs-agent-attachments--tool (romacs-mcp--object "paths" (vector notes)))))
            (funcall stop request session)
            ;; The hash ends before any check of the request's.
            (funcall settle "0000000000000000000000000000000000000000000000000000000000000000")
            (should (eq (car (condition-case err (funcall (nth 1 deferred)) (user-error (list :refused))))
                        :refused))
            (should-not noted)))))))

;;; romacs-agent-attachments-tests.el ends here
