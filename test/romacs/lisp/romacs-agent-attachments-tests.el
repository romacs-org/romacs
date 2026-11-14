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
  (romacs-agent-attachments--tool (apply #'romacs-mcp--object arguments)))

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

;;; romacs-agent-attachments-tests.el ends here
