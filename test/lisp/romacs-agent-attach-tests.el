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

(require 'romacs-agent-images)

(defun romacs-agent-attach-tests--state (&rest names)
  (romacs-agent-images--state (vconcat (mapcar (lambda (name) (concat "/shots/" name)) names))))

(defun romacs-agent-attach-tests--keys (state &rest keys)
  (dolist (key keys state) (setq state (romacs-agent-images--step state key))))

(ert-deftest romacs-agent-images-lists-images-newest-first ()
  (let ((dir (make-temp-file "romacs-images" t)))
    (unwind-protect
        (let ((old (expand-file-name "old.png" dir)) (new (expand-file-name "new.jpg" dir)))
          (write-region "x" nil old nil 'silent)
          (write-region "x" nil new nil 'silent)
          (write-region "x" nil (expand-file-name "notes.txt" dir) nil 'silent)
          (make-directory (expand-file-name "folder.png" dir))
          (set-file-times old (time-subtract nil 100))
          (should (equal (romacs-agent-images--files dir) (list new old))))
      (delete-directory dir t))))

(ert-deftest romacs-agent-images-keys-move-filter-and-end ()
  (let ((state (romacs-agent-attach-tests--state "a-romacs.png" "b-term.png" "c-romacs.png")))
    ;; Moving stays within the matches.
    (should (= 2 (plist-get (romacs-agent-attach-tests--keys state ?\C-n ?\C-n ?\C-n) :selected)))
    (should (= 0 (plist-get (romacs-agent-attach-tests--keys state ?\C-p) :selected)))
    (should (= 2 (plist-get (romacs-agent-attach-tests--keys state ?\C-v) :selected)))
    (should (= 0 (plist-get (romacs-agent-attach-tests--keys state ?\C-v ?\M-v) :selected)))
    ;; Typing filters, every word anywhere in the name, case aside, and
    ;; the selection starts over.
    (let ((typed (romacs-agent-attach-tests--keys state ?\C-n ?R ?o ?m ?\s ?c ?-)))
      (should (equal (plist-get typed :query) "Rom c-"))
      (should (= 0 (plist-get typed :selected)))
      (should (equal (plist-get typed :matches) ["/shots/c-romacs.png"]))
      (should (equal (plist-get (romacs-agent-attach-tests--keys typed ?\C-m) :result) "/shots/c-romacs.png"))
      (should (equal (plist-get (romacs-agent-attach-tests--keys typed ?\d ?\d ?\d) :query) "Rom")))
    ;; RET on no match stays; a stray key changes nothing.
    (let ((none (romacs-agent-attach-tests--keys state ?z ?\C-m 'f5)))
      (should-not (plist-get none :result))
      (should (= 0 (plist-get (romacs-agent-attach-tests--keys none ?\C-p ?\C-n) :selected))))
    (should (equal (romacs-agent-attach-tests--keys state ?\d) state))
    (should (eq (plist-get (romacs-agent-attach-tests--keys state ?\C-g) :result) 'cancel))
    (should (eq (plist-get (romacs-agent-attach-tests--keys state ?\C-i) :result) 'browse))
    ;; A step leaves the old state as it was.
    (romacs-agent-attach-tests--keys state ?a ?\C-n)
    (should (equal (plist-get state :query) ""))))

(ert-deftest romacs-agent-images-a-file-gone-at-ret-shows-the-menu-again ()
  (let ((dir (make-temp-file "romacs-images" t)))
    (unwind-protect
        (let* ((kept (expand-file-name "kept.png" dir)) (gone (expand-file-name "gone.png" dir))
               (state (romacs-agent-images--state (vector gone kept))))
          (write-region "x" nil kept nil 'silent)
          (let ((chosen (romacs-agent-images--step state ?\C-m)))
            (should (equal (plist-get chosen :result) gone))
            (let ((again (romacs-agent-images--checked chosen dir)))
              (should-not (plist-get again :result))
              (should (equal (plist-get again :matches) (vector kept)))))
          ;; What is there, and what is no file, pass as they are.
          (let ((good (plist-put (copy-sequence state) :result kept)))
            (should (eq (romacs-agent-images--checked good dir) good)))
          (let ((cancel (plist-put (copy-sequence state) :result 'cancel)))
            (should (eq (romacs-agent-images--checked cancel dir) cancel))))
      (delete-directory dir t))))

(ert-deftest romacs-agent-images-short-frame-browses ()
  (let ((shown nil))
    (cl-letf (((symbol-function 'frame-text-lines) (lambda (&rest _) 10))
              ((symbol-function 'romacs-popup-show) (lambda (&rest _) (setq shown t)))
              ((symbol-function 'read-key) (lambda (&rest _) (error "No key is read"))))
      (should (eq (romacs-agent-images-pick temporary-file-directory) 'browse)))
    (should-not shown)))

(ert-deftest romacs-agent-images-keeps-the-selection-in-view ()
  (should (= 0 (romacs-agent-images--first-row 0 100)))
  (should (= 0 (romacs-agent-images--first-row 2 100)))
  (should (= 8 (romacs-agent-images--first-row 10 100)))
  (should (= 95 (romacs-agent-images--first-row 99 100)))
  (should (= 0 (romacs-agent-images--first-row 1 3))))

(ert-deftest romacs-agent-images-draws-only-the-rows-in-view ()
  (with-temp-buffer
    (let ((state (apply #'romacs-agent-attach-tests--state
                        (mapcar (lambda (n) (format "%02d.png" n)) (number-sequence 0 49))))
          (shown nil))
      (cl-letf (((symbol-function 'romacs-agent-images--age) (lambda (_) "1d"))
                ((symbol-function 'romacs-agent-images--insert-image)
                 (lambda (file _) (push file shown))))
        (romacs-agent-images--draw (plist-put state :selected 10)))
      (should (= (length shown) (1+ romacs-agent-images--rows)))
      (should (string-match-p "Attach image:   50/50" (buffer-string)))
      (should (string-match-p "10.png  1d" (buffer-string)))
      (should-not (string-match-p "20.png" (buffer-string))))))

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

(ert-deftest romacs-agent-attach-prefix-and-no-clipboard-use-the-picker ()
  (romacs-agent-attach-tests--with-chat session
    (write-region "x" nil (expand-file-name "shot.png" dir) nil 'silent)
    (let ((romacs-agent-image-directory dir) (picked 0))
      (cl-letf (((symbol-function 'romacs-agent-ui--clipboard-image-type) (lambda () "image/png"))
                ((symbol-function 'romacs-agent-images-pick) (lambda (_) (cl-incf picked) "/tmp/x.png")))
        (let ((current-prefix-arg '(4))) (call-interactively #'romacs-agent-chat-attach)))
      (cl-letf (((symbol-function 'romacs-agent-ui--clipboard-image-type) #'ignore)
                ((symbol-function 'romacs-agent-images-pick) (lambda (_) (cl-incf picked) "/tmp/y.png")))
        (call-interactively #'romacs-agent-chat-attach))
      (should (= picked 2))
      (should (equal (romacs-agent-ui--composer-text) "[image: /tmp/x.png] [image: /tmp/y.png]")))))

(ert-deftest romacs-agent-attach-browse-and-empty-directory-read-a-file ()
  (romacs-agent-attach-tests--with-chat session
    (let ((romacs-agent-image-directory dir) (asked 0))
      (cl-letf (((symbol-function 'romacs-agent-ui--clipboard-image-type) #'ignore)
                ((symbol-function 'romacs-agent-images-pick) (lambda (_) (error "No images to pick")))
                ((symbol-function 'read-file-name) (lambda (&rest _) (cl-incf asked) "/tmp/z.png")))
        (call-interactively #'romacs-agent-chat-attach))
      (write-region "x" nil (expand-file-name "shot.png" dir) nil 'silent)
      (cl-letf (((symbol-function 'romacs-agent-ui--clipboard-image-type) #'ignore)
                ((symbol-function 'romacs-agent-images-pick) (lambda (_) 'browse))
                ((symbol-function 'read-file-name) (lambda (&rest _) (cl-incf asked) "/tmp/w.png")))
        (call-interactively #'romacs-agent-chat-attach))
      (should (= asked 2))
      (should (equal (romacs-agent-ui--composer-text) "[image: /tmp/z.png] [image: /tmp/w.png]")))))

(ert-deftest romacs-agent-attach-cancel-adds-nothing ()
  (romacs-agent-attach-tests--with-chat session
    (write-region "x" nil (expand-file-name "shot.png" dir) nil 'silent)
    (let ((romacs-agent-image-directory dir))
      (cl-letf (((symbol-function 'romacs-agent-ui--clipboard-image-type) #'ignore)
                ((symbol-function 'romacs-agent-images-pick) (lambda (_) 'cancel)))
        (should (eq (condition-case nil (call-interactively #'romacs-agent-chat-attach) (quit 'quit)) 'quit))))
    (should (equal (romacs-agent-ui--composer-text) ""))))

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
