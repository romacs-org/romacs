;;; romacs-agent-attach-tests.el --- Images in the composer and transcript -*- lexical-binding: t; -*-
(require 'ert)
(require 'romacs-mcp)
(require 'romacs-test-socket)

(let ((romacs-mcp-call-function romacs-mcp-call-function)
      (romacs-mcp-tools (copy-alist romacs-mcp-tools)))
  (require 'romacs-agent))
(require 'romacs-agent-test-attached)

(defmacro romacs-agent-attach-tests--with-chat (var &rest body)
  "BODY with a session VAR, its directory temporary."
  (declare (indent 1))
  `(let* ((dir (make-temp-file "romacs-attach" t))
          (,var (romacs-agent--make :id "11111111-2222-4333-8444-666666666666" :harness 'claude :state 'idle
                                    :title "chat" :dir dir :tools (make-hash-table :test #'equal))))
     (unwind-protect (with-temp-buffer ,@body)
       (delete-directory dir t))))

(defun romacs-agent-attach-tests--pick-through (layouts)
  "Pick with the frame's layout going through LAYOUTS, one per key, the
size hook firing as `read-key' waits: the result and the layouts shown."
  (let* ((dir (make-temp-file "romacs-images" t)) (shown nil) (now (car layouts)) (next (cdr layouts)))
    (unwind-protect
        (progn
          (write-region "x" nil (expand-file-name "a.png" dir) nil 'silent)
          (cl-letf (((symbol-function 'romacs-agent-images--frame-layout) (lambda () now))
                    ((symbol-function 'romacs-agent-images--show) (lambda (_ _p layout) (push layout shown)))
                    ((symbol-function 'romacs-agent-images--draw-preview) #'ignore)
                    ((symbol-function 'romacs-layer-hide) #'ignore)
                    ((symbol-function 'romacs-layer--root-frame) (lambda () 'root))
                    ((symbol-function 'read-key)
                     (lambda (&rest _)
                       (setq now (if next (pop next) now))
                       (run-hook-with-args 'window-size-change-functions 'root)
                       (if unread-command-events (pop unread-command-events) ?\C-g))))
            (list (romacs-agent-images-pick dir) (reverse shown) window-size-change-functions)))
      (delete-directory dir t))))

(ert-deftest romacs-agent-only-the-supervisor-s-record-names-a-prompt-s-images ()
  ;; As the journal holds them: a per-turn prompt, and words shaped like one.
  (let* ((events nil)
         (record (lambda (json) (json-parse-string json :array-type 'array)))
         (session (romacs-agent--make :id "33333333-2222-4333-8444-666666666666" :harness 'codex :title "c"))
         (romacs-agent-event-functions (list (lambda (_ event) (push event events)))))
    (let ((entry (make-hash-table :test #'equal)))
      (puthash "seq" 1 entry)
      (puthash "in" (funcall record "{\"prompt\":\"see\",\"images\":[\"/a.png\"]}") entry)
      (romacs-agent--record session entry))
    (should (equal (plist-get (car events) :text) "see"))
    (should (equal (plist-get (car events) :images) '("/a.png")))
    (setq events nil)
    (let ((entry (make-hash-table :test #'equal))
          (words "{\"text\":\"quoted\",\"images\":[\"/a.png\"]}"))
      (puthash "seq" 2 entry)
      (puthash "in" (funcall record (json-serialize (let ((h (make-hash-table :test #'equal)))
                                                      (puthash "prompt" words h)
                                                      (puthash "images" [] h)
                                                      h)))
               entry)
      (romacs-agent--record session entry)
      (should (equal (plist-get (car events) :text) words))
      (should-not (plist-get (car events) :images)))))

(defun romacs-agent-attach-tests--codex-images (session send)
  "The image paths of the message SEND makes SESSION write to Codex."
  (let ((written nil))
    (cl-letf (((symbol-function 'romacs-agent--line) (lambda (_ object &rest _) (setq written object))))
      (funcall send))
    (cl-loop for item across (gethash "input" (gethash "params" written))
             when (equal (gethash "type" item) "localImage") collect (gethash "path" item))))

(ert-deftest romacs-agent-sweep-keeps-a-route-that-answers ()
  "A process of another HOME has no record of a live session: its route stays."
  (let* ((root (file-name-as-directory (romacs-test-socket-directory "sweep")))
         (routes (expand-file-name "route/" root))
         (socket (expand-file-name "s.sock" root))
         (server nil))
    (unwind-protect
        (cl-letf (((symbol-function 'romacs-agent--runtime-root) (lambda () root))
                  ((symbol-function 'romacs-agent--state-root) (lambda () (expand-file-name "state/" root)))
                  ((symbol-function 'romacs-agent--sessions-base) (lambda () (expand-file-name "none/" root))))
          (make-directory routes)
          (set-file-modes routes #o700)
          (setq server (make-network-process :name "sweep" :family 'local :service socket :server t :noquery t))
          (make-symbolic-link socket (expand-file-name "live.sock" routes))
          (make-symbolic-link (expand-file-name "gone.sock" root) (expand-file-name "dead.sock" routes))
          (romacs-agent--sweep)
          (should (file-symlink-p (expand-file-name "live.sock" routes)))
          (should-not (file-symlink-p (expand-file-name "dead.sock" routes))))
      (when server (delete-process server))
      (delete-directory root t))))

(ert-deftest romacs-agent-claude-is-given-attached-images-as-content ()
  (romacs-agent-attach-tests--with-chat session
    (let ((image (expand-file-name "a.png" dir)))
      (write-region "PNG" nil image nil 'silent)
      ;; Plain text, however it reads, stays a string.
      (let* ((line (romacs-agent--claude-user session (format "see [image: %s]" image) 1))
             (content (gethash "content" (gethash "message" line))))
        (should (stringp content)))
      (let* ((romacs-agent--message-images (list image))
             (line (romacs-agent--claude-user session "see" 1))
             (content (gethash "content" (gethash "message" line)))
             (block (aref content 2)))
        (should (equal (gethash "text" (aref content 0)) "see"))
        ;; Each image after the line saying where it is saved.
        (should (equal (gethash "text" (aref content 1)) (romacs-agent--image-caption image)))
        (should (equal (gethash "type" block) "image"))
        (should (equal (gethash "media_type" (gethash "source" block)) "image/png"))
        (should (equal (gethash "data" (gethash "source" block)) (base64-encode-string "PNG" t)))
        ;; What the journal shows of it is the text alone.
        (should (equal (romacs-agent--text-of content) "see"))))))

(ert-deftest romacs-agent-only-attached-images-reach-codex ()
  (romacs-agent-attach-tests--with-chat session
    (let ((image (expand-file-name "secret.png" dir))
          (codex (romacs-agent--make :id "22222222-2222-4333-8444-666666666666" :harness 'codex :state 'idle
                                     :title "codex" :dir dir :harness-session "thread"
                                     :driver-state (list :ready t))))
      (write-region "x" nil image nil 'silent)
      (romacs-agent-test-attach codex)
      ;; A child's result quoting a file uploads nothing.
      (should-not (romacs-agent-attach-tests--codex-images
                   codex (lambda () (romacs-agent-send codex (format "Delegated result: [image: %s]" image)))))
      ;; Its turn ended.
      (setf (romacs-agent-state codex) 'idle)
      ;; Attached, it goes.
      (should (equal (romacs-agent-attach-tests--codex-images
                      codex (lambda () (romacs-agent-send codex "see" nil nil (list image))))
                     (list image)))
      ;; Queued, it waits with its message.
      (setf (romacs-agent-state codex) 'running)
      (romacs-agent-send-later codex "later" nil (list image))
      (setf (romacs-agent-state codex) 'idle)
      (should (equal (romacs-agent-attach-tests--codex-images codex (lambda () (romacs-agent--dispatch codex)))
                     (list image))))))

(ert-deftest romacs-agent-the-journal-keeps-references-not-bytes ()
  (romacs-agent-attach-tests--with-chat session
    (let ((image (expand-file-name "a.png" dir)) (wire nil) (kept nil))
      (write-region "PNGBYTES" nil image nil 'silent)
      (cl-letf (((symbol-function 'romacs-agent--line) (lambda (_ w j) (setq wire w kept j))))
        (romacs-agent--input session "see" (list image)))
      ;; The harness gets the bytes; the journal the words and the file.
      (should (string-match-p (regexp-quote (base64-encode-string "PNGBYTES" t)) (romacs-agent--json wire)))
      (should-not (string-match-p (regexp-quote (base64-encode-string "PNGBYTES" t)) (romacs-agent--json kept)))
      (let ((event (car (romacs-agent--claude-input-events session kept))))
        (should (equal (plist-get event :text) "see"))
        (should (equal (plist-get event :images) (list image))))
      ;; Without images the journal keeps none: only the turn's request id.
      (cl-letf (((symbol-function 'romacs-agent--line) (lambda (_ w &optional j) (setq wire w kept j))))
        (romacs-agent--input session "plain"))
      (should-not (gethash "images" kept))
      (should (integerp (gethash "romacsRequest" kept)))
      (should-not (plist-get (car (romacs-agent--claude-input-events session wire)) :images)))))

(ert-deftest romacs-agent-the-size-bound-sees-through-a-symlink ()
  (romacs-agent-attach-tests--with-chat session
    (let ((target (expand-file-name "target.bin" dir)) (link (expand-file-name "link.png" dir))
          (romacs-agent-image-max-bytes 4) (kept :none))
      (write-region "123" nil target nil 'silent)
      (make-symbolic-link target link)
      (should (romacs-agent-image-file-p link))
      ;; The target grows past the bound before dispatch: it is not sent.
      (write-region "123456789" nil target nil 'silent)
      (should-not (romacs-agent-image-file-p link))
      (cl-letf (((symbol-function 'romacs-agent--line) (lambda (_ wire &optional journal) (setq kept (list wire journal)))))
        (romacs-agent--input session "see" (list link)))
      (should-not (gethash "images" (cadr kept)))
      (should (stringp (gethash "content" (gethash "message" (car kept))))))))

