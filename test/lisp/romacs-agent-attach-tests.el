;;; romacs-agent-attach-tests.el --- Images in the composer and transcript -*- lexical-binding: t; -*-
(require 'ert)
(require 'romacs-mcp)

(let ((romacs-mcp-call-function romacs-mcp-call-function)
      (romacs-mcp-tools (copy-alist romacs-mcp-tools)))
  (require 'romacs-agent))

(defmacro romacs-agent-attach-tests--with-chat (var &rest body)
  "BODY in a chat of a session VAR, its directory temporary."
  (declare (indent 1))
  `(let* ((dir (make-temp-file "romacs-attach" t))
          (,var (romacs-agent--make :id "11111111-2222-4333-8444-666666666666" :harness 'claude :state 'idle
                                    :title "chat" :dir dir :tools (make-hash-table :test #'equal))))
     (unwind-protect
         (with-temp-buffer
           (romacs-agent-chat-mode)
           (setq romacs-agent-ui--session ,var
                 romacs-agent-ui--tool-lines (make-hash-table :test #'equal)
                 romacs-agent-ui--tasks (make-hash-table :test #'equal)
                 romacs-agent-ui--messages (make-hash-table :test #'equal)
                 romacs-agent-ui--end (point-marker)
                 romacs-agent-ui--composer (make-overlay (point) (point) nil nil nil))
           ,@body)
       (delete-directory dir t))))

(ert-deftest romacs-agent-attach-owns-c-c-c-a ()
  (should (eq (lookup-key romacs-agent-chat-mode-map (kbd "C-c C-a")) #'romacs-agent-chat-attach))
  (should-not (fboundp 'romacs-agent-chat-respond)))

(ert-deftest romacs-agent-attach-puts-a-token-in-the-composer ()
  (romacs-agent-attach-tests--with-chat session
    (romacs-agent-ui--insert "transcript\n")
    (goto-char (point-min))
    (romacs-agent-chat-attach "/tmp/shot one.png")
    (should (equal (romacs-agent-ui--composer-text) "[image: /tmp/shot one.png]"))
    (should (= (point) (point-max)))
    ;; Typed text goes on after it, and a second image too.
    (insert "look ")
    (romacs-agent-chat-attach "/tmp/b.png")
    (should (equal (romacs-agent-ui--composer-text) "[image: /tmp/shot one.png] look [image: /tmp/b.png]"))))

(ert-deftest romacs-agent-attach-prefers-the-clipboard ()
  (romacs-agent-attach-tests--with-chat session
    (cl-letf (((symbol-function 'romacs-agent-ui--clipboard-image-type) (lambda () "image/png"))
              ((symbol-function 'call-process)
               (lambda (program _in destination _display &rest args)
                 (should (equal program "wl-paste"))
                 (should (equal args '("--type" "image/png")))
                 (with-temp-file (cadr destination) (insert "PNG"))
                 0)))
      (call-interactively #'romacs-agent-chat-attach))
    (let ((text (romacs-agent-ui--composer-text)))
      (should (string-match "\\`\\[image: \\(.*/attachments/clipboard-.*\\.png\\)\\]\\'" text))
      (should (string-prefix-p (expand-file-name dir) (match-string 1 text)))
      (should (file-exists-p (match-string 1 text))))))

(ert-deftest romacs-agent-attach-prefix-and-no-clipboard-read-a-file ()
  (romacs-agent-attach-tests--with-chat session
    (let ((romacs-agent-image-directory dir) (asked 0))
      (cl-letf (((symbol-function 'romacs-agent-ui--clipboard-image-type) (lambda () "image/png"))
                ((symbol-function 'read-file-name) (lambda (&rest _) (cl-incf asked) "/tmp/x.png")))
        (let ((current-prefix-arg '(4))) (call-interactively #'romacs-agent-chat-attach)))
      (cl-letf (((symbol-function 'romacs-agent-ui--clipboard-image-type) #'ignore)
                ((symbol-function 'read-file-name) (lambda (&rest _) (cl-incf asked) "/tmp/y.png")))
        (call-interactively #'romacs-agent-chat-attach))
      (should (= asked 2))
      (should (equal (romacs-agent-ui--composer-text) "[image: /tmp/x.png] [image: /tmp/y.png]")))))

(ert-deftest romacs-agent-attach-offers-the-newest-image ()
  (let ((dir (make-temp-file "romacs-images" t)))
    (unwind-protect
        (let ((old (expand-file-name "old.png" dir)) (new (expand-file-name "new.jpg" dir)))
          (write-region "x" nil old nil 'silent)
          (write-region "x" nil new nil 'silent)
          (write-region "x" nil (expand-file-name "notes.txt" dir) nil 'silent)
          (set-file-times old (time-subtract nil 100))
          (should (equal (romacs-agent-ui--newest-image dir) new)))
      (delete-directory dir t))
    (should-not (romacs-agent-ui--newest-image dir))))

(ert-deftest romacs-agent-attach-refuses-a-name-no-token-can-hold ()
  (romacs-agent-attach-tests--with-chat session
    (should-error (romacs-agent-chat-attach "/tmp/shot [1].png") :type 'user-error)
    (should-error (romacs-agent-chat-attach "/tmp/a\nb.png") :type 'user-error)
    (should-error (romacs-agent-chat-attach "/tmp/shot,one.png") :type 'user-error)
    (should (equal (romacs-agent-ui--composer-text) ""))))

(ert-deftest romacs-agent-comma-in-a-typed-token-reaches-no-harness-as-an-image ()
  (romacs-agent-attach-tests--with-chat session
    (let ((image (expand-file-name "a,b.png" dir)))
      (write-region "x" nil image nil 'silent)
      (should-not (romacs-agent-image-files (format "[image: %s]" image))))))

(ert-deftest romacs-agent-attach-leaves-no-file-when-capture-fails ()
  (romacs-agent-attach-tests--with-chat session
    (dolist (failure (list (lambda (&rest _) 1) (lambda (&rest _) (error "No wl-paste"))))
      (cl-letf (((symbol-function 'call-process) failure))
        (should-error (romacs-agent-ui--save-clipboard-image session "image/png")))
      (should-not (directory-files (expand-file-name "attachments" dir) nil "clipboard-")))))

(ert-deftest romacs-agent-quoted-token-never-breaks-the-transcript ()
  ;; A directory, and a broken image: the replay goes on, the text stays.
  (romacs-agent-attach-tests--with-chat session
    (let ((broken (expand-file-name "b.png" dir)))
      (write-region "x" nil broken nil 'silent)
      (cl-letf (((symbol-function 'display-images-p) (lambda (&rest _) t))
                ((symbol-function 'create-image) (lambda (&rest _) (error "Bad image"))))
        (romacs-agent-ui--draw session (list :type 'input :seq 1 :text (format "[image: /tmp/] [image: %s]" broken))
                               (current-buffer))
        (romacs-agent-ui--draw session (list :type 'input :seq 2 :text "next") (current-buffer)))
      (should (string-match-p "> next" (buffer-string)))
      (should-not (next-single-property-change (point-min) 'display)))))

(ert-deftest romacs-agent-codex-gets-a-prompt-s-images-on-its-command-line ()
  (romacs-agent-attach-tests--with-chat session
    (let ((image (expand-file-name "a.png" dir)))
      (write-region "x" nil image nil 'silent)
      (write-region (format "see [image: %s] and [image: /gone/b.png] and [image: /tmp/]" image)
                    nil (expand-file-name "input" dir) nil 'silent)
      (cl-letf (((symbol-function 'romacs-agent--instance) (lambda (&rest _) (list :program "codex")))
                ((symbol-function 'romacs-agent--environment-words) #'ignore)
                ((symbol-function 'romacs-agent--codex-mcp-words) #'ignore))
        (setf (romacs-agent-harness-session session) "thread")
        (let ((argv (romacs-agent--codex-argv session t)))
          (should (equal (seq-take argv 6) (list "codex" "exec" "resume" "thread" "--image" image)))
          ;; The prompt stays the last word: --image takes no more than its file.
          (should (equal (nth 6 argv) "--json"))
          (should (equal (car (last argv)) "-"))))
      (delete-file (expand-file-name "input" dir))
      (cl-letf (((symbol-function 'romacs-agent--instance) (lambda (&rest _) (list :program "codex")))
                ((symbol-function 'romacs-agent--environment-words) #'ignore)
                ((symbol-function 'romacs-agent--codex-mcp-words) #'ignore))
        (should-not (member "--image" (romacs-agent--codex-argv session t)))))))

(ert-deftest romacs-agent-transcript-shows-attached-images ()
  (romacs-agent-attach-tests--with-chat session
    (let ((image (expand-file-name "a.png" dir)))
      (write-region "x" nil image nil 'silent)
      (cl-letf (((symbol-function 'display-images-p) (lambda (&rest _) t))
                ((symbol-function 'create-image) (lambda (file &rest _) (list 'image :file file))))
        (romacs-agent-ui--draw session (list :type 'input :seq 1 :text (format "see [image: %s] and [image: /gone/b.png]" image))
                               (current-buffer)))
      (goto-char (point-min))
      (search-forward "[image: /")
      (should (equal (get-text-property (1- (point)) 'display) (list 'image :file image)))
      (search-forward "/gone/b.png")
      (should-not (get-text-property (1- (point)) 'display))
      ;; The text stays beneath: the turn can be copied and edited.
      (should (string-match-p (regexp-quote (format "see [image: %s] and" image))
                              (buffer-substring-no-properties (point-min) (point-max)))))))
