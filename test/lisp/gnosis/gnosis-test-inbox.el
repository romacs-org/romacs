;;; gnosis-test-inbox.el --- Agent draft inbox -*- lexical-binding: t; -*-

;; Copyright (C) 2026 Free Software Foundation, Inc.

;;; Commentary:
;; Draft schema validation, dedupe, native accept/edit/reject paths, the
;; one-at-a-time review commands and the no-Emacs batch producer.

;;; Code:

(require 'ert)
(require 'gnosis-test-helpers)
(require 'gnosis-inbox)

(defun gnosis-test-inbox--pdf ()
  "Return a stand-in lecture file in the disposable directory."
  (let ((file (expand-file-name "Heme lecture.pdf" gnosis-dir)))
    (unless (file-exists-p file) (write-region "%PDF-1.4\n" nil file nil 'silent))
    file))

(defun gnosis-test-inbox--draft (&rest overrides)
  "Return a valid lecture cloze draft with OVERRIDES applied."
  (let ((draft (list :type "cloze"
                     :keimenon "ALA synthase condenses glycine with succinyl-CoA in the mitochondria."
                     :answer '("glycine" "mitochondria")
                     :parathema "Rate-limiting step of heme synthesis."
                     :tags '("biochemistry")
                     :kind "lecture" :source (gnosis-test-inbox--pdf) :page 13
                     :agent "hermes/test" :reason "slide 13 step 1")))
    (cl-loop for (key value) on overrides by #'cddr
             do (setq draft (plist-put draft key value)))
    draft))

(defun gnosis-test-inbox--mcq ()
  "Return a valid lecture MCQ draft."
  (gnosis-test-inbox--draft
   :type "mcq" :keimenon "Which zinc enzyme of heme synthesis is most sensitive to lead?"
   :hypothesis '("ALA dehydratase" "ALA synthase" "Ferrochelatase")
   :answer '("ALA dehydratase") :page 18 :reason "slide 18"))

(defun gnosis-test-inbox--statuses (results)
  "Return the status symbols of RESULTS."
  (mapcar (lambda (r) (plist-get r :status)) results))

(defun gnosis-test-inbox--rows ()
  "Return a snapshot of every content and scheduling table."
  (mapcar (lambda (table) (sqlite-select gnosis-db (format "SELECT * FROM %s ORDER BY 1" table)))
          '("themata" "extras" "thema_tag" "thema_links" "scheduler_state")))

(defun gnosis-test-inbox--entries ()
  "Return parsed inbox entries."
  (gnosis-inbox-parse (gnosis-inbox--read (gnosis-inbox-file))))

(defmacro gnosis-test-inbox--ui (&rest body)
  "Run BODY in an inbox review buffer."
  (declare (indent 0) (debug t))
  `(save-window-excursion
     (gnosis-inbox)
     (unwind-protect (progn ,@body)
       (when (get-buffer "*Gnosis Inbox Draft*") (kill-buffer "*Gnosis Inbox Draft*"))
       (kill-buffer "*Gnosis Inbox*"))))

;;; Schema and validation

(ert-deftest gnosis-inbox-add-writes-native-codec ()
  (gnosis-test-with-db
    (let ((results (gnosis-inbox-add (list (gnosis-test-inbox--draft) (gnosis-test-inbox--mcq)))))
      (should (equal (gnosis-test-inbox--statuses results) '(added added)))
      (let ((entries (gnosis-test-inbox--entries)))
        (should (= 2 (length entries)))
        (dolist (entry entries)
          (should (eq (plist-get entry :state) 'pending))
          (should-not (plist-get entry :error))
          (should (equal (plist-get entry :agent) "hermes/test")))
        (should (equal (nth 4 (plist-get (car entries) :thema)) '("glycine" "mitochondria")))
        (should (equal (nth 3 (plist-get (cadr entries) :thema))
                       '("ALA dehydratase" "ALA synthase" "Ferrochelatase")))
        (should (= 18 (plist-get (cadr entries) :page)))
        (should (= 2 (gnosis-inbox-count))))
      ;; Each entry is the native export shape: the native parser reads it.
      (with-temp-buffer
        (insert-file-contents (gnosis-inbox-file))
        (should (re-search-forward "^\\* Thema :biochemistry:$" nil t))
        (should (re-search-forward "^:GNOSIS_ID: NEW$" nil t))
        (should (re-search-forward "^:GNOSIS_PROPOSAL: lecture$" nil t))))))

(ert-deftest gnosis-inbox-validation-refuses-bad-drafts ()
  (gnosis-test-with-db
    (let ((results
           (gnosis-inbox-add
            (list (gnosis-test-inbox--draft :answer '("glucose"))
                  (gnosis-test-inbox--draft :page nil :span "x")
                  (gnosis-test-inbox--draft :source "relative.pdf" :span "y")
                  (gnosis-test-inbox--draft :agent nil :span "z")
                  (gnosis-test-inbox--draft :kind "free-for-all" :span "w")
                  (gnosis-test-inbox--draft :type "model" :span "v")
                  (gnosis-test-inbox--draft :type "mcq" :hypothesis '("A" "B")
                                            :answer '("C") :span "u")
                  (gnosis-test-inbox--draft :type "basic" :answer '("A" "B") :span "t")
                  (gnosis-test-inbox--draft :kind "rework-replace" :span "s")))))
      (should (equal (delete-dups (gnosis-test-inbox--statuses results)) '(invalid)))
      ;; The native writer's own message reaches the agent.
      (should (string-match-p "disjoint occurrences" (plist-get (car results) :error)))
      (should-not (file-exists-p (gnosis-inbox-file))))))

(ert-deftest gnosis-inbox-validation-refuses-lossy-codec ()
  (gnosis-test-with-db
    (let ((result (car (gnosis-inbox-add
                        (list (gnosis-test-inbox--draft
                               :type "basic" :keimenon "Q" :answer '("first\n- second")))))))
      (should (eq (plist-get result :status) 'invalid)))))

(ert-deftest gnosis-inbox-add-never-opens-learner-db ()
  (let* ((dir (make-temp-file "gnosis-inbox-test-" t))
         (gnosis-dir (file-name-as-directory dir))
         (gnosis-db nil))
    (unwind-protect
        (progn
          (should (equal (gnosis-test-inbox--statuses
                          (gnosis-inbox-add (list (gnosis-test-inbox--draft))))
                         '(added)))
          (should-not gnosis-db)
          (should-not (file-exists-p (expand-file-name "gnosis.db" dir))))
      (delete-directory dir t))))

;;; Dedupe

(ert-deftest gnosis-inbox-dedupes-same-source-span ()
  (gnosis-test-with-db
    (should (equal (gnosis-test-inbox--statuses (gnosis-inbox-add (list (gnosis-test-inbox--draft))))
                   '(added)))
    (let ((before (gnosis-inbox--read (gnosis-inbox-file))))
      ;; Re-run, reworded question with the same answers on the same page,
      ;; and a duplicate inside one batch.
      (should (equal (gnosis-test-inbox--statuses
                      (gnosis-inbox-add
                       (list (gnosis-test-inbox--draft)
                             (gnosis-test-inbox--draft
                              :keimenon "In the mitochondria, ALA synthase uses glycine.")
                             (gnosis-test-inbox--mcq) (gnosis-test-inbox--mcq))))
                     '(duplicate duplicate added duplicate)))
      (should-not (equal before (gnosis-inbox--read (gnosis-inbox-file)))))
    ;; A different page or explicit span is a different proposal.
    (should (equal (gnosis-test-inbox--statuses
                    (gnosis-inbox-add (list (gnosis-test-inbox--draft :page 16)
                                            (gnosis-test-inbox--draft :span "second card"))))
                   '(added added)))
    (should (= 4 (gnosis-inbox-count)))))

(ert-deftest gnosis-inbox-caps-per-source ()
  (gnosis-test-with-db
    (let ((gnosis-inbox-max-per-source 2))
      (should (equal (gnosis-test-inbox--statuses
                      (gnosis-inbox-add (list (gnosis-test-inbox--draft :span "1")
                                              (gnosis-test-inbox--draft :span "2")
                                              (gnosis-test-inbox--draft :span "3"))))
                     '(added added capped))))))

;;; Accept, reject, edit

(ert-deftest gnosis-inbox-accept-uses-native-writer ()
  (gnosis-test-with-db
    (gnosis-inbox-add (list (gnosis-test-inbox--draft)))
    (let ((calls 0) (hooked nil) (gnosis-save-hook nil))
      (add-hook 'gnosis-save-hook (lambda (id) (push id hooked)))
      (cl-letf* ((original (symbol-function 'gnosis-save-thema))
                 ((symbol-function 'gnosis-save-thema)
                  (lambda (&rest args) (cl-incf calls) (apply original args))))
        (gnosis-test-inbox--ui
          (gnosis-inbox-accept)
          (should (string-match-p "Inbox empty" (buffer-string)))))
      (should (= calls 1))
      (let* ((id (car (gnosis-select 'id 'themata nil t)))
             (entry (car (gnosis-test-inbox--entries))))
        (should (equal hooked (list id)))
        (should (equal (gnosis-get 'answer 'themata `(= id ,id)) '("glycine" "mitochondria")))
        (should (equal (gnosis-get-tags-for-ids (list id)) '("biochemistry")))
        ;; The lecture page is cited, actionably, in the post-answer text.
        (let ((parathema (gnosis-get 'parathema 'extras `(= id ,id))))
          (should (string-match-p "Rate-limiting" parathema))
          (should (string-match-p "gnosis-lecture:13:" parathema)))
        (should (eq (plist-get entry :state) 'applied))
        (should (equal (plist-get entry :themata) (list id)))
        ;; Ledger keeps no content, and blocks re-proposal.
        (should-not (plist-get entry :thema))
        (should (equal (gnosis-test-inbox--statuses
                        (gnosis-inbox-add (list (gnosis-test-inbox--draft))))
                       '(duplicate)))))))

(ert-deftest gnosis-inbox-accept-refuses-invalid-file-entry ()
  (gnosis-test-with-db
    ;; An agent wrote the file directly, bypassing validation.
    (write-region
     (concat "* Thema\n:PROPERTIES:\n:GNOSIS_ID: NEW\n:GNOSIS_TYPE: cloze\n"
             ":GNOSIS_PROPOSAL: lecture\n:GNOSIS_SOURCE: " (gnosis-test-inbox--pdf) "\n"
             ":GNOSIS_PAGE: 3\n:GNOSIS_AGENT: a\n:GNOSIS_REASON: r\n:END:\n"
             "** Keimenon\nNo blank here.\n\n** Hypothesis\n-\n\n** Answer\n- absent\n\n"
             "** Parathema\n\n")
     nil (gnosis-inbox-file) nil 'silent)
    (let ((rows (gnosis-test-inbox--rows))
          (text (gnosis-inbox--read (gnosis-inbox-file))))
      (gnosis-test-inbox--ui
        (should-error (gnosis-inbox-accept) :type 'user-error)
        (should (string-match-p "No blank here" (buffer-string))))
      (should (equal rows (gnosis-test-inbox--rows)))
      (should (equal text (gnosis-inbox--read (gnosis-inbox-file)))))))

(ert-deftest gnosis-inbox-reject-leaves-db-untouched ()
  (gnosis-test-with-db
    (gnosis-test--add-basic-thema "Existing" "card")
    (gnosis-inbox-add (list (gnosis-test-inbox--draft)))
    (let ((rows (gnosis-test-inbox--rows))
          (changes (sqlite-select gnosis-db "SELECT total_changes()")))
      (gnosis-test-inbox--ui
        (let ((current-prefix-arg nil))
          (gnosis-inbox-reject "too easy")))
      (should (equal rows (gnosis-test-inbox--rows)))
      (should (equal changes (sqlite-select gnosis-db "SELECT total_changes()")))
      (let ((entry (car (gnosis-test-inbox--entries))))
        (should (eq (plist-get entry :state) 'rejected)))
      (should (string-match-p ":GNOSIS_REJECT_REASON: too easy"
                              (gnosis-inbox--read (gnosis-inbox-file))))
      (should (equal (gnosis-test-inbox--statuses
                      (gnosis-inbox-add (list (gnosis-test-inbox--draft))))
                     '(duplicate))))))

(ert-deftest gnosis-inbox-skip-previous-and-tally ()
  (gnosis-test-with-db
    (gnosis-inbox-add (list (gnosis-test-inbox--draft) (gnosis-test-inbox--mcq)
                            (gnosis-test-inbox--draft :page 20 :answer '("iron")
                                                      :keimenon "Ferrochelatase inserts iron.")))
    (gnosis-test-inbox--ui
      (let ((first gnosis-inbox--current))
        (should (string-match-p "1/3" (format "%s" header-line-format)))
        (gnosis-inbox-skip)
        (should (string-match-p "Which zinc enzyme" (buffer-string)))
        (gnosis-inbox-previous)
        (should (equal first gnosis-inbox--current))
        (gnosis-inbox-skip)
        (gnosis-inbox-reject)
        ;; Rejecting advances to the next pending draft, which is the third.
        (should (string-match-p "p\\.20" (buffer-string)))
        (gnosis-inbox-accept)
        (should (string-match-p "1/1" (format "%s" header-line-format)))
        (should (equal first gnosis-inbox--current))
        (gnosis-inbox-skip)
        (should (equal first gnosis-inbox--current))
        (gnosis-inbox-accept)
        (should (string-match-p "2 applied  ·  1 rejected" (buffer-string)))
        ;; Receipts name the card they acted on; an empty inbox offers no
        ;; card actions.
        (should (string-match-p "Previous: Saved #[0-9]+: “ALA synthase" (buffer-string)))
        (should-not (string-match-p "accept" (format "%s" header-line-format)))))
    (should (= 2 (length (gnosis-select 'id 'themata nil t))))))

(ert-deftest gnosis-inbox-refuses-stale-view ()
  (gnosis-test-with-db
    (gnosis-inbox-add (list (gnosis-test-inbox--draft)))
    (gnosis-test-inbox--ui
      (gnosis-inbox-add (list (gnosis-test-inbox--mcq)))
      (should-error (gnosis-inbox-accept) :type 'user-error)
      (should-not (gnosis-select 'id 'themata nil t))
      ;; The refusal reloaded the view; acting again now works.
      (should (= 2 (length (gnosis-inbox--pending))))
      (gnosis-inbox-accept)
      (should (= 1 (length (gnosis-select 'id 'themata nil t)))))))

(ert-deftest gnosis-inbox-accept-is-idempotent-after-crash ()
  (gnosis-test-with-db
    (gnosis-inbox-add (list (gnosis-test-inbox--draft)))
    ;; Simulate a crash after the database commit but before the file write.
    (gnosis-inbox-apply (car (gnosis-test-inbox--entries)))
    (let ((rows (gnosis-test-inbox--rows)))
      (gnosis-test-inbox--ui
        (should (string-match-p "already in collection" (buffer-string)))
        (gnosis-inbox-accept))
      (should (equal rows (gnosis-test-inbox--rows)))
      (should (eq (plist-get (car (gnosis-test-inbox--entries)) :state) 'duplicate)))))

(ert-deftest gnosis-inbox-edit-saves-through-native-editor ()
  (gnosis-test-with-db
    (gnosis-inbox-add (list (gnosis-test-inbox--draft) (gnosis-test-inbox--mcq)))
    (gnosis-test-inbox--ui
      (gnosis-inbox-edit)
      (should (eq major-mode 'gnosis-edit-mode))
      (should (eq (key-binding (kbd "C-c C-c")) #'gnosis-inbox-draft-save))
      ;; Long prose wraps instead of clipping at the window edge.
      (should-not truncate-lines)
      (should visual-line-mode)
      (goto-char (point-min))
      (search-forward "condenses glycine")
      (replace-match "joins glycine")
      (gnosis-inbox-draft-save)
      (should-not (get-buffer "*Gnosis Inbox Draft*"))
      (with-current-buffer "*Gnosis Inbox*"
        (should (string-match-p "Saved edited" (buffer-string)))
        (should (string-match-p "Which zinc enzyme" (buffer-string)))))
    (let ((id (car (gnosis-select 'id 'themata nil t))))
      (should (string-match-p "joins glycine" (gnosis-get 'keimenon 'themata `(= id ,id))))
      (should (string-match-p ":GNOSIS_EDITED: t" (gnosis-inbox--read (gnosis-inbox-file)))))))

(ert-deftest gnosis-inbox-edit-cancel-keeps-pending ()
  (gnosis-test-with-db
    (gnosis-inbox-add (list (gnosis-test-inbox--draft)))
    (gnosis-test-inbox--ui
      (gnosis-inbox-edit)
      (gnosis-edit-quit))
    (should-not (gnosis-select 'id 'themata nil t))
    (should (= 1 (gnosis-inbox-count)))))

;;; Leech rework (F7 foundation)

(ert-deftest gnosis-inbox-rework-replaces-and-suspends ()
  (gnosis-test-with-db
    (let* ((target (gnosis-test--add-basic-thema "Long confusing card" "answer"))
           (results (gnosis-inbox-propose-rework
                     target (list (list :type "basic" :keimenon "Short card 1" :answer '("a"))
                                  (list :type "basic" :keimenon "Short card 2" :answer '("b")))
                     :kind "rework-split" :agent "hermes/tutor" :reason "5 lapses")))
      (should (equal (gnosis-test-inbox--statuses results) '(added added)))
      (gnosis-test-inbox--ui
        (should (string-match-p "replaces #" (buffer-string)))
        (gnosis-inbox-accept))
      (should (= 1 (gnosis-get 'suspended 'scheduler-state `(= thema-id ,target))))
      (should (= 2 (length (gnosis-select 'id 'themata nil t))))
      ;; Target changed after drafting: the second split card is refused.
      (gnosis-update-thema target "Edited meanwhile" '("") '("answer") "" nil nil)
      (let ((rows (gnosis-test-inbox--rows)))
        (gnosis-test-inbox--ui
          (should-error (gnosis-inbox-accept) :type 'user-error))
        (should (equal rows (gnosis-test-inbox--rows)))))))

;;; Batch producer

(ert-deftest gnosis-inbox-add-file-and-batch ()
  (gnosis-test-with-db
    (let ((drafts (expand-file-name "drafts.org" gnosis-dir)))
      (write-region (concat (gnosis-inbox--draft-text (gnosis-test-inbox--draft)) "\n\n"
                            (gnosis-inbox--draft-text (gnosis-test-inbox--mcq)) "\n\n"
                            "* Thema\n:PROPERTIES:\n:GNOSIS_ID: NEW\n:END:\n")
                    nil drafts nil 'silent)
      (let* ((dirs (delete-dups
                    (delq nil (mapcar (lambda (lib)
                                        (when-let* ((file (locate-library lib)))
                                          (file-name-directory file)))
                                      '("gnosis-inbox" "keymap-popup" "compat")))))
             (process-environment (cons (concat "GNOSIS_DIR=" gnosis-dir) process-environment))
             (out (with-temp-buffer
                    (list (apply #'call-process
                                 (expand-file-name invocation-name invocation-directory)
                                 nil t nil "-Q" "--batch"
                                 (append (mapcan (lambda (d) (list "-L" d)) dirs)
                                         (list "-l" "gnosis-inbox" "-f" "gnosis-inbox-batch-add"
                                               drafts)))
                          (buffer-string))))
             (json (json-parse-string (car (last (split-string (cadr out) "\n" t)))
                                      :object-type 'plist)))
        (should (= 1 (car out)))
        (should (equal (mapcar (lambda (r) (plist-get r :status)) (plist-get json :results))
                       '("invalid" "added" "added")))
        (should (= 2 (gnosis-inbox-count)))
        ;; Rerunning the agent's batch adds nothing.
        (should (equal (cdr (gnosis-test-inbox--statuses (gnosis-inbox-add-file drafts)))
                       '(duplicate duplicate)))))))

;;; Review fixes: approval boundaries

(ert-deftest gnosis-inbox-edited-rework-saves-and-suspends ()
  (gnosis-test-with-db
    (let ((target (gnosis-test--add-basic-thema "Old card" "old")))
      (gnosis-inbox-propose-rework
       target (list (list :type "basic" :keimenon "New card" :answer '("new")))
       :agent "test" :reason "rework")
      (gnosis-test-inbox--ui
        (gnosis-inbox-edit)
        (execute-kbd-macro (kbd "C-c C-c")))
      (should (= 2 (length (gnosis-select 'id 'themata nil t))))
      (should (= 1 (gnosis-get 'suspended 'scheduler-state `(= thema-id ,target))))
      (let ((entry (car (gnosis-test-inbox--entries))))
        (should (eq 'applied (plist-get entry :state)))
        (should (= 1 (length (plist-get entry :themata))))))))

(ert-deftest gnosis-inbox-edited-rework-refuses-changed-target ()
  (gnosis-test-with-db
    (let ((target (gnosis-test--add-basic-thema "Old card" "old")))
      (gnosis-inbox-propose-rework
       target (list (list :type "basic" :keimenon "New card" :answer '("new")))
       :agent "test" :reason "rework")
      (let ((text (gnosis-inbox--read (gnosis-inbox-file))))
        (gnosis-test-inbox--ui
          (gnosis-inbox-edit)
          (gnosis-update-thema target "Changed after proposal" '("") '("old") "" nil nil)
          (let ((rows (gnosis-test-inbox--rows))
                (draft (buffer-string)))
            (should-error (execute-kbd-macro (kbd "C-c C-c")) :type 'user-error)
            (should (equal rows (gnosis-test-inbox--rows)))
            (with-current-buffer "*Gnosis Inbox Draft*"
              (should (equal draft (buffer-string)))
              (should-not gnosis--draft-saved-p))))
        (should (equal text (gnosis-inbox--read (gnosis-inbox-file)))))
      (should (= 1 (length (gnosis-select 'id 'themata nil t))))
      (should (= 0 (gnosis-get 'suspended 'scheduler-state `(= thema-id ,target)))))))

(ert-deftest gnosis-inbox-edited-rework-suspend-fault-rolls-back ()
  (gnosis-test-with-db
    (let ((target (gnosis-test--add-basic-thema "Old" "old")))
      (gnosis-inbox-propose-rework
       target (list (list :type "basic" :keimenon "New" :answer '("new")))
       :agent "t" :reason "r")
      (let ((rows (gnosis-test-inbox--rows)))
        (gnosis-test-inbox--ui
          (gnosis-inbox-edit)
          (cl-letf (((symbol-function 'gnosis-toggle-suspend-themata)
                     (lambda (&rest _) (error "Injected suspend fault"))))
            (should-error (gnosis-inbox-draft-save)))
          (should (get-buffer "*Gnosis Inbox Draft*")))
        (should (equal rows (gnosis-test-inbox--rows)))
        (should (eq 'pending (plist-get (car (gnosis-test-inbox--entries)) :state)))))))

(ert-deftest gnosis-inbox-edited-save-refuses-replaced-proposal ()
  (gnosis-test-with-db
    (gnosis-inbox-add (list (gnosis-test-inbox--draft)))
    (gnosis-test-inbox--ui
      (gnosis-inbox-edit)
      (let* ((text (gnosis-inbox--read (gnosis-inbox-file)))
             (changed (replace-regexp-in-string "condenses glycine" "DIFFERENT glycine"
                                                text t t)))
        (write-region changed nil (gnosis-inbox-file) nil 'silent)
        (should-error (gnosis-inbox-draft-save) :type 'user-error)
        (should (equal changed (gnosis-inbox--read (gnosis-inbox-file))))
        (with-current-buffer "*Gnosis Inbox Draft*"
          (should (string-match-p "condenses glycine" (buffer-string))))))
    (should-not (gnosis-select 'id 'themata nil t))
    (should (eq 'pending (plist-get (car (gnosis-test-inbox--entries)) :state)))))

(ert-deftest gnosis-inbox-edited-save-refuses-closed-or-reopened ()
  (gnosis-test-with-db
    (gnosis-inbox-add (list (gnosis-test-inbox--draft) (gnosis-test-inbox--mcq)))
    ;; Closed by another owner while the editor was open.
    (gnosis-test-inbox--ui
      (gnosis-inbox-edit)
      (with-current-buffer "*Gnosis Inbox*" (gnosis-inbox-reject))
      (with-current-buffer "*Gnosis Inbox Draft*"
        (should-error (gnosis-inbox-draft-save) :type 'user-error)))
    (should-not (gnosis-select 'id 'themata nil t))
    ;; The inbox was reopened: a new view does not authorize the old draft.
    (gnosis-test-inbox--ui
      (gnosis-inbox-edit)
      (save-window-excursion (gnosis-inbox))
      (with-current-buffer "*Gnosis Inbox Draft*"
        (should-error (gnosis-inbox-draft-save) :type 'user-error)))
    (should-not (gnosis-select 'id 'themata nil t))
    (should (eq 'pending (plist-get (cadr (gnosis-test-inbox--entries)) :state)))))

(ert-deftest gnosis-inbox-edited-save-allows-appended-sibling ()
  (gnosis-test-with-db
    (gnosis-inbox-add (list (gnosis-test-inbox--draft)))
    (gnosis-test-inbox--ui
      (gnosis-inbox-edit)
      (gnosis-inbox-add (list (gnosis-test-inbox--mcq)))
      (gnosis-inbox-draft-save))
    (should (= 1 (length (gnosis-select 'id 'themata nil t))))
    (should (equal (mapcar (lambda (e) (plist-get e :state)) (gnosis-test-inbox--entries))
                   '(applied pending)))))

(ert-deftest gnosis-inbox-reject-prompt-refuses-successor-inbox ()
  (gnosis-test-with-db
    (gnosis-inbox-add (list (gnosis-test-inbox--draft)))
    (let* ((first-file (gnosis-inbox-file))
           (db1 gnosis-db)
           (dir2 (make-temp-file "gnosis-inbox-second-" t))
           (db2 (gnosis-db--open dir2))
           (second-file (expand-file-name "inbox.org" dir2)))
      (unwind-protect
          (gnosis-test-inbox--ui
            (let ((gnosis-db db2) (gnosis-dir (file-name-as-directory dir2)))
              (gnosis-inbox-add (list (gnosis-test-inbox--mcq))))
            (let ((before1 (gnosis-inbox--read first-file))
                  (before2 (gnosis-inbox--read second-file)))
              (cl-letf (((symbol-function 'read-string)
                         (lambda (&rest _)
                           (setq gnosis-db db2 gnosis-dir (file-name-as-directory dir2))
                           (gnosis-inbox)
                           "reason for original")))
                (let ((current-prefix-arg '(4)))
                  (should-error (call-interactively #'gnosis-inbox-reject)
                                :type 'user-error)))
              (should (equal before1 (gnosis-inbox--read first-file)))
              (should (equal before2 (gnosis-inbox--read second-file)))))
        (setq gnosis-db db1)
        (gnosis-sqlite-close db2)
        (delete-directory dir2 t)))))

(ert-deftest gnosis-inbox-reject-prompt-same-owner-and-quit ()
  (gnosis-test-with-db
    (gnosis-inbox-add (list (gnosis-test-inbox--draft)))
    (let ((text (gnosis-inbox--read (gnosis-inbox-file))))
      (gnosis-test-inbox--ui
        (cl-letf (((symbol-function 'read-string) (lambda (&rest _) (signal 'quit nil))))
          (let ((current-prefix-arg '(4)))
            (should (eq 'quit (condition-case nil
                                   (progn (call-interactively #'gnosis-inbox-reject) 'done)
                                 (quit 'quit)))))))
      (should (equal text (gnosis-inbox--read (gnosis-inbox-file)))))
    (gnosis-test-inbox--ui
      ;; Changing the shown entry during the prompt refuses too.
      (cl-letf (((symbol-function 'read-string)
                 (lambda (&rest _) (gnosis-inbox-add (list (gnosis-test-inbox--mcq))) "x")))
        (let ((current-prefix-arg '(4)))
          (should-error (call-interactively #'gnosis-inbox-reject) :type 'user-error))))
    (should (equal '(pending pending)
                   (mapcar (lambda (e) (plist-get e :state)) (gnosis-test-inbox--entries))))
    (gnosis-test-inbox--ui
      (cl-letf (((symbol-function 'read-string) (lambda (&rest _) "too easy")))
        (let ((current-prefix-arg '(4)))
          (call-interactively #'gnosis-inbox-reject))))
    (should (eq 'rejected (plist-get (car (gnosis-test-inbox--entries)) :state)))
    (should (string-match-p ":GNOSIS_REJECT_REASON: too easy"
                            (gnosis-inbox--read (gnosis-inbox-file))))))

(ert-deftest gnosis-inbox-accept-refuses-provenance-errors ()
  (dolist (edit '(("^:GNOSIS_AGENT:.*\n" . "")
                  ("^:GNOSIS_REASON:.*\n" . "")
                  ("^:GNOSIS_PROPOSAL: lecture" . ":GNOSIS_PROPOSAL: anything")
                  ("^:GNOSIS_PROPOSAL: lecture" . ":GNOSIS_PROPOSAL: rework-replace")
                  ("^:GNOSIS_PAGE:.*\n" . ":GNOSIS_TARGET: 1\n")))
    (gnosis-test-with-db
      (gnosis-inbox-add (list (gnosis-test-inbox--draft)))
      (let* ((file (gnosis-inbox-file))
             (text (replace-regexp-in-string (car edit) (cdr edit)
                                             (gnosis-inbox--read file) t t)))
        (write-region text nil file nil 'silent)
        (should (plist-get (car (gnosis-test-inbox--entries)) :error))
        (let ((rows (gnosis-test-inbox--rows)))
          (gnosis-test-inbox--ui
            (should-error (gnosis-inbox-accept) :type 'user-error)
            (should-error (gnosis-inbox-edit) :type 'user-error))
          (should (equal rows (gnosis-test-inbox--rows))))
        (should-error (gnosis-inbox-apply (car (gnosis-test-inbox--entries)))
                      :type 'user-error)
        (should (equal text (gnosis-inbox--read file)))))))

;;; Review fixes: dedupe identity

(ert-deftest gnosis-inbox-key-uses-source-bytes ()
  (gnosis-test-with-db
    (let* ((one (expand-file-name "one/lecture.pdf" gnosis-dir))
           (two (expand-file-name "two/lecture.pdf" gnosis-dir))
           (moved (expand-file-name "moved/renamed.pdf" gnosis-dir)))
      (dolist (file (list one two moved)) (make-directory (file-name-directory file) t))
      (write-region "%PDF-1.4 first" nil one nil 'silent)
      (write-region "%PDF-1.4 second" nil two nil 'silent)
      (write-region "%PDF-1.4 first" nil moved nil 'silent)
      (should (equal '(added added duplicate)
                     (gnosis-test-inbox--statuses
                      (gnosis-inbox-add
                       (list (gnosis-test-inbox--draft :source one)
                             (gnosis-test-inbox--draft :source two)
                             (gnosis-test-inbox--draft :source moved))))))
      ;; A lecture source must exist when drafted.
      (should (eq 'invalid
                  (plist-get (car (gnosis-inbox-add
                                   (list (gnosis-test-inbox--draft
                                          :source (expand-file-name "absent.pdf" gnosis-dir)))))
                             :status))))))

(ert-deftest gnosis-inbox-rework-key-includes-revision ()
  (gnosis-test-with-db
    (let ((target (gnosis-test--add-basic-thema "Old card" "old"))
          (drafts (list (list :type "basic" :keimenon "New card" :answer '("new")))))
      (should (equal '(added) (gnosis-test-inbox--statuses
                               (gnosis-inbox-propose-rework target drafts :agent "a" :reason "r"))))
      (should (equal '(duplicate) (gnosis-test-inbox--statuses
                                   (gnosis-inbox-propose-rework target drafts :agent "a" :reason "r"))))
      (gnosis-update-thema target "Changed card" '("") '("old") "" nil nil)
      (should (equal '(added) (gnosis-test-inbox--statuses
                               (gnosis-inbox-propose-rework target drafts :agent "a" :reason "r")))))))

(ert-deftest gnosis-inbox-existing-requires-full-content ()
  (gnosis-test-with-db
    (gnosis-inbox-add (list (gnosis-test-inbox--mcq)))
    (let ((entry (car (gnosis-test-inbox--entries))))
      (gnosis-inbox-apply entry)
      (should (gnosis-inbox--existing-id (gnosis-inbox--effective-thema entry)))
      ;; Different choices or explanation are a different card.
      (let ((thema (gnosis-inbox--effective-thema entry)))
        (setf (nth 3 thema) '("ALA dehydratase" "Uroporphyrinogen decarboxylase"))
        (should-not (gnosis-inbox--existing-id thema))
        (should (gnosis-inbox--similar-id thema)))
      (let ((thema (gnosis-inbox--effective-thema entry)))
        (setf (nth 5 thema) "Other explanation")
        (should-not (gnosis-inbox--existing-id thema))))))

;;; Review fixes: batch producer errors

(defvar gnosis-test-inbox--batch-unset-dir nil
  "Environment entry replacing GNOSIS_DIR for the batch producer, or nil.")

(defun gnosis-test-inbox--batch (&rest args)
  "Run the batch producer with ARGS; return (EXIT JSON-PLIST)."
  (let* ((dirs (delete-dups
                (delq nil (mapcar (lambda (lib)
                                    (when-let* ((file (locate-library lib)))
                                      (file-name-directory file)))
                                  '("gnosis-inbox" "keymap-popup" "compat")))))
         (process-environment (cons (or gnosis-test-inbox--batch-unset-dir
                                      (concat "GNOSIS_DIR=" gnosis-dir))
                                    process-environment)))
    (with-temp-buffer
      (let ((exit (apply #'call-process
                         (expand-file-name invocation-name invocation-directory)
                         nil (list t nil) nil "-Q" "--batch"
                         (append (mapcan (lambda (d) (list "-L" d)) dirs)
                                 (list "-l" "gnosis-inbox" "-f" "gnosis-inbox-batch-add")
                                 args))))
        (list exit (json-parse-string (string-trim (buffer-string)) :object-type 'plist))))))

(ert-deftest gnosis-inbox-batch-reports-invocation-errors ()
  (gnosis-test-with-db
    (pcase-dolist (`(,args . ,pattern)
                   `((("/nonexistent/DRAFTS.org") . "not a readable")
                     (nil . "Usage")
                     (("a.org" "b.org") . "Usage")))
      (pcase-let ((`(,exit ,json) (apply #'gnosis-test-inbox--batch args)))
        (should (= exit 2))
        (should (string-match-p pattern (plist-get json :error)))
        (should (plist-get json :inbox))))
    (let ((empty (expand-file-name "empty.org" gnosis-dir)))
      (write-region "#+title: nothing\n" nil empty nil 'silent)
      (should (= 2 (car (gnosis-test-inbox--batch empty)))))
    (should-not (file-exists-p (gnosis-inbox-file)))))

;;; Review fixes: retired views

(defun gnosis-test-inbox--associate (&optional detach)
  "Give the inbox buffer a visited file and successor text.
With DETACH, drop the file association again afterwards.  The buffer is
renamed back only so `gnosis-test-inbox--ui' can clean it up."
  (with-current-buffer "*Gnosis Inbox*"
    (set-visited-file-name (expand-file-name "unrelated.org" gnosis-dir) t)
    (auto-save-mode -1)
    (when detach (set-visited-file-name nil))
    (rename-buffer "*Gnosis Inbox*")
    (let ((inhibit-read-only t)) (erase-buffer) (insert "SUCCESSOR DRAFT"))))

(defun gnosis-test-inbox--detach ()
  "Drop any file association of the inbox buffer before cleanup."
  (with-current-buffer "*Gnosis Inbox*"
    (set-visited-file-name nil)
    (set-buffer-modified-p nil)))

(ert-deftest gnosis-inbox-edited-save-refuses-file-associated-view ()
  (dolist (detach '(nil t))
    (gnosis-test-with-db
      (gnosis-inbox-add (list (gnosis-test-inbox--draft)))
      (let ((inbox (gnosis-inbox--read (gnosis-inbox-file))))
        (gnosis-test-inbox--ui
          (gnosis-inbox-edit)
          (gnosis-test-inbox--associate detach)
          (let ((rows (gnosis-test-inbox--rows)))
            (with-current-buffer "*Gnosis Inbox Draft*"
              (let ((draft (buffer-string)))
                (should-error (call-interactively #'gnosis-inbox-draft-save)
                              :type 'user-error)
                (should (equal draft (buffer-string)))
                (should-not gnosis--draft-saved-p)))
            (should (equal rows (gnosis-test-inbox--rows))))
          (should (equal inbox (gnosis-inbox--read (gnosis-inbox-file))))
          (with-current-buffer "*Gnosis Inbox*"
            (should (equal "SUCCESSOR DRAFT" (buffer-string))))
          (gnosis-test-inbox--detach))))))

(ert-deftest gnosis-inbox-reject-prompt-refuses-file-association ()
  (dolist (detach '(nil t))
    (gnosis-test-with-db
      (gnosis-inbox-add (list (gnosis-test-inbox--draft)))
      (let ((inbox (gnosis-inbox--read (gnosis-inbox-file))))
        (gnosis-test-inbox--ui
          (cl-letf (((symbol-function 'read-string)
                     (lambda (&rest _) (gnosis-test-inbox--associate detach) "old reason")))
            (let ((current-prefix-arg '(4)))
              (should-error (call-interactively #'gnosis-inbox-reject) :type 'user-error)))
          (should (equal inbox (gnosis-inbox--read (gnosis-inbox-file))))
          (should (equal "SUCCESSOR DRAFT" (buffer-string)))
          (gnosis-test-inbox--detach))))))

(ert-deftest gnosis-inbox-retired-view-refuses-every-command ()
  (dolist (retire '(file detach))
    (gnosis-test-with-db
      (gnosis-inbox-add (list (gnosis-test-inbox--draft) (gnosis-test-inbox--mcq)))
      (let ((inbox (gnosis-inbox--read (gnosis-inbox-file))))
        (gnosis-test-inbox--ui
          (gnosis-test-inbox--associate (eq retire 'detach))
          (dolist (command '(gnosis-inbox-accept gnosis-inbox-edit gnosis-inbox-reject
                             gnosis-inbox-skip gnosis-inbox-previous gnosis-inbox-reload))
            (should-error (funcall command) :type 'user-error))
          (should-not (get-buffer "*Gnosis Inbox Draft*"))
          ;; Window resizing does not render over the successor text.
          (gnosis-inbox--resize (selected-window))
          (should (equal "SUCCESSOR DRAFT" (buffer-string)))
          (should-not (gnosis-select 'id 'themata nil t))
          (should (equal inbox (gnosis-inbox--read (gnosis-inbox-file))))
          ;; Reopening uses a fresh buffer, never the retired one.
          (let ((retired (current-buffer)))
            (save-window-excursion
              (gnosis-inbox)
              (should-not (eq retired (current-buffer)))
              (should (= 2 (length (gnosis-inbox--pending))))
              (kill-buffer))
            (should (equal "SUCCESSOR DRAFT" (with-current-buffer retired (buffer-string)))))
          (gnosis-test-inbox--detach))))))

(ert-deftest gnosis-inbox-mode-change-retires-view ()
  (gnosis-test-with-db
    (gnosis-inbox-add (list (gnosis-test-inbox--draft)))
    (gnosis-test-inbox--ui
      (gnosis-inbox-edit)
      (with-current-buffer "*Gnosis Inbox*"
        (text-mode)
        ;; Re-entering the mode by hand does not revive the old opening.
        (gnosis-inbox-mode))
      (with-current-buffer "*Gnosis Inbox Draft*"
        (should-error (call-interactively #'gnosis-inbox-draft-save) :type 'user-error))
      (should-not (gnosis-select 'id 'themata nil t)))))

(ert-deftest gnosis-inbox-edited-save-settles-without-rendering-retired-view ()
  (gnosis-test-with-db
    (gnosis-inbox-add (list (gnosis-test-inbox--draft)))
    (gnosis-test-inbox--ui
      (gnosis-inbox-edit)
      (let ((commit (symbol-function 'gnosis-inbox--commit)))
        (cl-letf (((symbol-function 'gnosis-inbox--commit)
                   (lambda (&rest args)
                     (prog1 (apply commit args) (gnosis-test-inbox--associate)))))
          (with-current-buffer "*Gnosis Inbox Draft*"
            (call-interactively #'gnosis-inbox-draft-save))))
      ;; The committed save is recorded in the inbox file, but the
      ;; retired buffer keeps its successor text.
      (should (= 1 (length (gnosis-select 'id 'themata nil t))))
      (should (eq 'applied (plist-get (car (gnosis-test-inbox--entries)) :state)))
      (with-current-buffer "*Gnosis Inbox*"
        (should (equal "SUCCESSOR DRAFT" (buffer-string))))
      (gnosis-test-inbox--detach))))

(ert-deftest gnosis-inbox-renamed-view-keeps-authority ()
  (gnosis-test-with-db
    (gnosis-inbox-add (list (gnosis-test-inbox--draft) (gnosis-test-inbox--mcq)))
    (gnosis-test-inbox--ui
      (gnosis-inbox-edit)
      (with-current-buffer "*Gnosis Inbox*" (rename-buffer "renamed inbox"))
      (with-current-buffer "*Gnosis Inbox Draft*"
        (call-interactively #'gnosis-inbox-draft-save))
      (with-current-buffer "renamed inbox"
        (should (string-match-p "Saved edited" (buffer-string)))
        (cl-letf (((symbol-function 'read-string) (lambda (&rest _) "too easy")))
          (let ((current-prefix-arg '(4)))
            (call-interactively #'gnosis-inbox-reject)))
        (rename-buffer "*Gnosis Inbox*"))
      (should (equal '(applied rejected)
                     (mapcar (lambda (e) (plist-get e :state)) (gnosis-test-inbox--entries)))))))

(ert-deftest gnosis-inbox-key-sees-same-size-rewrite ()
  (gnosis-test-with-db
    (let* ((file (gnosis-test-inbox--pdf))
           (stamp (file-attribute-modification-time (file-attributes file)))
           (key (gnosis-inbox-key file 1 "span")))
      (write-region "%PDF-2.4\n" nil file nil 'silent)
      (set-file-times file stamp)
      (should-not (equal key (gnosis-inbox-key file 1 "span"))))))

(ert-deftest gnosis-inbox-batch-requires-destination ()
  (gnosis-test-with-db
    (let ((drafts (expand-file-name "drafts.org" gnosis-dir)))
      (write-region (gnosis-inbox--draft-text (gnosis-test-inbox--draft)) nil drafts nil 'silent)
      (dolist (env '("GNOSIS_DIR" "GNOSIS_DIR="))
        (let ((gnosis-test-inbox--batch-unset-dir env))
          (pcase-let ((`(,exit ,json) (gnosis-test-inbox--batch drafts)))
            (should (= exit 2))
            (should (string-match-p "GNOSIS_DIR" (plist-get json :error))))))
      (should-not (file-exists-p (gnosis-inbox-file))))))

(provide 'gnosis-test-inbox)
;;; gnosis-test-inbox.el ends here
