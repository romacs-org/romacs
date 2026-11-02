;;; gnosis-agent-eval.el --- Asynchronous free-response review -*- lexical-binding: t; -*-
;; Maintainer: Thanos Apollo <public@thanosapollo.org>

;; Copyright (C) 2026 Free Software Foundation, Inc.
;; Author: Thanos Apollo <public@thanosapollo.org>
;; Keywords: extensions
;; URL: https://thanosapollo.org/projects/gnosis

;; This program is free software; you can redistribute it and/or modify
;; it under the terms of the GNU General Public License as published by
;; the Free Software Foundation, either version 3 of the License, or
;; (at your option) any later version.
;; This program is distributed in the hope that it will be useful,
;; but WITHOUT ANY WARRANTY; without even the implied warranty of
;; MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE.  See the
;; GNU General Public License for more details.
;; You should have received a copy of the GNU General Public License
;; along with this program.  If not, see <https://www.gnu.org/licenses/>.

;;; Commentary:

;; A disposable text buffer collects a multiline response.  Evaluation is
;; asynchronous and never accepts a grade.  The ordinary review actions own
;; acceptance, including overrides, scheduling and non-rescheduling practice.

;;; Code:

(require 'gnosis-review)
(require 'subr-x)

(autoload 'gnosis-agent-eval-hermes "gnosis-agent-eval-hermes")

(defcustom gnosis-agent-eval-function #'gnosis-agent-eval-hermes
  "Function evaluating an agent-eval response asynchronously.
Call with REQUEST, RESOLVE and REJECT; return a cancellation function.
REQUEST is a plain plist of strings at :question, :reference-answer,
:rubric and :response.  RESOLVE accepts a plist with :verdict equal to
pass, fail or ungradable and a nonempty :explanation string.  Failure
explanations must distinguish missing essentials from incorrect assertions
and give a correction.  REJECT accepts an error explanation string.
Callbacks may arrive after cancellation; Gnosis ignores retired attempts.
Do not write grades or mutate REQUEST.  Unknown or malformed results,
rejections and abstentions never become a failed grade.  The default
integration loads Hermes only when an evaluation is requested."
  :type 'function
  :group 'gnosis)

(defcustom gnosis-agent-eval-timeout 120
  "Maximum seconds to wait for a response evaluation.
A timeout leaves the response available for retry and records no grade."
  :type 'natnum
  :group 'gnosis)

(defvar-local gnosis-agent-eval--context nil
  "Owned response encounter, or nil outside agent evaluation.")

(defvar-local gnosis-agent-eval--review-context nil
  "Response encounter owned by this review buffer, or nil.")

(defvar gnosis-agent-eval--initial-context nil
  "Encounter consumed by one native response mode initialization.")

(defvar gnosis-agent-eval-mode-map
  (let ((map (make-sparse-keymap)))
    (define-key map (kbd "C-c C-c") #'gnosis-agent-eval-submit)
    (define-key map (kbd "C-c C-k") #'gnosis-agent-eval-cancel)
    (define-key map (kbd "C-g") #'gnosis-agent-eval-quit)
    ;; Ready feedback is an ordinary command, never a low-level input event.
    (define-key map [gnosis-agent-eval-ready] #'gnosis-agent-eval--ready)
    map)
  "Keymap for multiline agent-evaluated responses.")

(define-derived-mode gnosis-agent-eval-mode text-mode "Gnosis response"
  "Write a response, then use \\[gnosis-agent-eval-submit] to evaluate it.
Cancel evaluation with \\[gnosis-agent-eval-cancel], retaining the response.
Leave without grading with \\[gnosis-agent-eval-quit]."
  (setq gnosis-agent-eval--context gnosis-agent-eval--initial-context
        gnosis-agent-eval--initial-context nil)
  (setq-local truncate-lines nil
              truncate-partial-width-windows nil
              word-wrap t)
  (setq-local header-line-format
              ;; Quote mode-line directives without losing native key faces.
              (replace-regexp-in-string
               "%" (lambda (match) (concat match match))
               (substitute-command-keys
                (concat "\\<gnosis-agent-eval-mode-map>"
                        " Response  \\[gnosis-agent-eval-submit] Evaluate/continue"
                        "  \\[gnosis-agent-eval-cancel] Cancel evaluation"
                        "  \\[gnosis-agent-eval-quit] Quit"))
               t t))
  (add-hook 'kill-buffer-hook #'gnosis-agent-eval--retire nil t)
  (add-hook 'change-major-mode-hook #'gnosis-agent-eval--retire nil t)
  (add-hook 'after-set-visited-file-name-hook #'gnosis-agent-eval--retire nil t))

(defun gnosis-agent-eval--review-valid-p (context)
  "Return non-nil when CONTEXT still owns its review occurrence.
This authority permits section cleanup after the response has retired;
publication additionally requires `gnosis-agent-eval--valid-p'."
  (let ((review (plist-get context :review)))
    (and (eq (plist-get context :database) gnosis-db)
         (buffer-live-p review)
         (with-current-buffer review
           (and (eq major-mode 'gnosis-mode) (not buffer-file-name)
                (eq gnosis-agent-eval--review-context context)
                (condition-case nil
                    (progn
                      (funcall (plist-get context :validate))
                      t)
                  (error nil)))))))

(defun gnosis-agent-eval--valid-p (context)
  "Return non-nil when CONTEXT still owns its response and review encounter."
  (let ((buffer (plist-get context :buffer)))
    (and (not (plist-get context :retired))
         (buffer-live-p buffer)
         (with-current-buffer buffer
           (and (eq gnosis-agent-eval--context context)
                (eq major-mode 'gnosis-agent-eval-mode) (not buffer-file-name)))
         (gnosis-agent-eval--review-valid-p context))))

(defun gnosis-agent-eval--cancel (cancel)
  "Invoke cancellation function CANCEL without leaking its buffer or quit."
  (when (functionp cancel)
    (save-current-buffer
      (condition-case err (funcall cancel)
        ((error quit) (message "Evaluator cancellation: %s" (error-message-string err)))))))

(defun gnosis-agent-eval--stop (context)
  "Retire CONTEXT's attempt before cancelling its timer and evaluator."
  (let ((cancel (plist-get context :cancel)))
    (setf (plist-get context :attempt) nil
          (plist-get context :cancel) nil)
    (when (timerp (plist-get context :timer))
      (cancel-timer (plist-get context :timer)))
    (setf (plist-get context :timer) nil)
    (gnosis-agent-eval--cancel cancel)))

(defun gnosis-agent-eval--retire ()
  "Retire this response encounter without touching any successor."
  (when gnosis-agent-eval--context
    (let ((context gnosis-agent-eval--context))
      (when-let* ((checkpoint (plist-get context :checkpoint)))
        (save-restriction
          (widen)
          (funcall checkpoint "pause"
                   (buffer-substring-no-properties (point-min) (point-max))
                   (plist-get context :result))))
      (setf (plist-get context :retired) t)
      (gnosis-agent-eval--stop context)
      (when (eq gnosis-agent-eval--context context)
        (setq gnosis-agent-eval--context nil))
      (when (= (recursion-depth) (1+ (plist-get context :depth)))
        (abort-recursive-edit)))))

(defun gnosis-agent-eval--retire-review ()
  "Retire the response when its review buffer loses ownership."
  (when-let* ((context gnosis-agent-eval--review-context))
    (setq gnosis-agent-eval--review-context nil)
    (setf (plist-get context :retired) t)
    (gnosis-agent-eval--stop context)))

(defun gnosis-agent-eval--show (context text &optional face)
  "Display TEXT with FACE after CONTEXT's unmodified response.
Preserve TEXT's existing faces, including its key hints."
  (with-current-buffer (plist-get context :buffer)
    (let ((overlay (plist-get context :overlay))
          (text (copy-sequence text)))
      (add-face-text-property 0 (length text) (or face 'shadow) t text)
      (move-overlay overlay (point-max) (point-max))
      (overlay-put overlay 'after-string
                   (concat "\n\n" text "\n")))))

(defun gnosis-agent-eval--prose (context text)
  "Replace CONTEXT's provisional review section with literal TEXT.
Nil TEXT removes only that section, even after response retirement, while
requiring the original review occurrence.  Never edit the response or select
a window.  Ordinary collection callers retain their existing display contract."
  (when (and (plist-get context :prose)
             (if text (gnosis-agent-eval--valid-p context)
               (gnosis-agent-eval--review-valid-p context)))
    (with-current-buffer (plist-get context :review)
      (let ((formatted (when text
                         (gnosis-review--format-string
                          (concat (propertize "Evaluator feedback: " 'face 'gnosis-face-directions)
                                  text) t))))
        ;; Formatting may run native hooks; they may retire this encounter.
        (when (if text (gnosis-agent-eval--valid-p context)
                (gnosis-agent-eval--review-valid-p context))
          (gnosis-review--preserve-view
           (lambda ()
             (let ((inhibit-read-only t)
                   (inhibit-modification-hooks t)
                   (buffer-undo-list t)
                   (section (plist-get context :section)))
               (when (and (overlayp section)
                          (eq (overlay-buffer section) (current-buffer)))
                 (delete-region (overlay-start section) (overlay-end section))
                 (delete-overlay section))
               (setf (plist-get context :section) nil)
               (when formatted
                 (let ((start (point-max)))
                   (gnosis-review--append-section formatted #'ignore "\n\n" "\n")
                   ;; Interior insertion inherits read-only from its left.
                   ;; Only the final character releases the outside boundary.
                   (add-text-properties start (point)
                                        '(read-only t rear-nonsticky nil))
                   (put-text-property (1- (point)) (point) 'rear-nonsticky '(read-only))
                   (setf (plist-get context :section)
                         (make-overlay start (point) nil t))))))))))))

(defun gnosis-agent-eval--ready (event)
  "Present EVENT's settled feedback from the native command loop.
Never leave nested input or take focus from another buffer.  A settled
response can still be continued explicitly after such navigation."
  (interactive "e")
  (let ((context (nth 1 event)))
    (when (and (gnosis-agent-eval--valid-p context)
               (eq (current-buffer) (plist-get context :buffer))
               (= (recursion-depth) (1+ (plist-get context :depth)))
               (not (plist-get context :attempt))
               (eq (nth 2 event) (plist-get context :section)))
      (if (plist-get context :result)
          (exit-recursive-edit)
        ;; Keep the retryable draft selected, but expose the explanation in
        ;; the existing review window rather than an unscrollable overlay.
        (when-let* ((section (plist-get context :section))
                    (window (get-buffer-window (plist-get context :review))))
          (set-window-start window (overlay-start section))
          (set-window-point window (overlay-start section)))))))

(defun gnosis-agent-eval--notify (context)
  "Queue CONTEXT's feedback for its owning native command loop."
  (when (and (plist-get context :prose) (plist-get context :section))
    (setq unread-command-events
          (append unread-command-events
                  (list (list 'gnosis-agent-eval-ready context
                              (plist-get context :section)))))))

(defun gnosis-agent-eval--result-p (result)
  "Return non-nil for a complete, unambiguous evaluator RESULT."
  (and (proper-list-p result) (= (length result) 4)
       (memq (car result) '(:verdict :explanation))
       (memq (nth 2 result) '(:verdict :explanation))
       (not (eq (car result) (nth 2 result)))
       (memq (plist-get result :verdict) '(pass fail ungradable))
       (stringp (plist-get result :explanation))
       (not (string-empty-p (string-trim (plist-get result :explanation))))))

(defun gnosis-agent-eval--settle (context attempt result failure)
  "Settle ATTEMPT in CONTEXT with RESULT or FAILURE, never a grade.
A callback only updates its owned response and optional review section;
it never exits recursive input, selects a window or invokes a scheduler."
  (when (and (eq attempt (plist-get context :attempt))
             (gnosis-agent-eval--valid-p context))
    (when-let* ((checkpoint (plist-get context :checkpoint)))
      (condition-case err
          (funcall checkpoint "settle" (plist-get context :response)
                   (and (gnosis-agent-eval--result-p result) result)
                   (or failure (unless (gnosis-agent-eval--result-p result)
                                 "Malformed evaluator result")))
        (error (setq result nil failure (error-message-string err)))))
    (let* ((valid (and (not failure) (gnosis-agent-eval--result-p result)))
           (graded (and valid (memq (plist-get result :verdict) '(pass fail))))
           (text (cond (failure (if (stringp failure) failure "Evaluation failed"))
                       (valid (plist-get result :explanation))
                       (t "Malformed evaluator result"))))
      (when (gnosis-agent-eval--valid-p context)
        (setf (plist-get context :result) (and graded (copy-sequence result)))
        (with-current-buffer (plist-get context :buffer)
          (setq buffer-read-only (and graded t)))
        (gnosis-agent-eval--prose context text)
        (when (gnosis-agent-eval--valid-p context)
          (gnosis-agent-eval--show
           context
           ;; Only instructions undergo key substitution, never evaluator prose.
           (if (plist-get context :prose)
               (concat (if graded
                           (if (eq (plist-get result :verdict) 'pass)
                               "Recall: correct" "Recall: not yet correct")
                         (if failure "Evaluation error — not graded" "Not graded"))
                       (substitute-command-keys
                        (if graded
                            (concat "\n\\<gnosis-agent-eval-mode-map>"
                                    "Opening feedback; \\[gnosis-agent-eval-submit] continues if needed")
                          (concat "\n\\<gnosis-agent-eval-mode-map>"
                                  "Feedback above; edit or \\[gnosis-agent-eval-submit] to retry; "
                                  "\\[gnosis-agent-eval-quit] to quit"))))
             (if graded
                 (concat (format "%s: %s\n"
                                 (if (eq (plist-get result :verdict) 'pass) "Pass" "Fail")
                                 text)
                         (substitute-command-keys
                          (concat "\\<gnosis-agent-eval-mode-map>"
                                  "\\[gnosis-agent-eval-submit]: Continue to review actions (not yet accepted)")))
               (concat "Not graded: " text "\n"
                       (substitute-command-keys
                        (concat "\\<gnosis-agent-eval-mode-map>"
                                "Edit or \\[gnosis-agent-eval-submit] to retry; \\[gnosis-agent-eval-quit] to quit")))))
           (if graded (if (eq (plist-get result :verdict) 'pass) 'success
                        (if (plist-get context :prose) 'warning 'error)) 'warning))
          (gnosis-agent-eval--notify context)))
      ;; Cancellation can edit and resubmit, even completing synchronously.
      ;; Publish everything first; never overwrite that successor on return.
      (gnosis-agent-eval--stop context))))

(defun gnosis-agent-eval-cancel ()
  "Cancel the evaluation, preserving the response for retry without grading."
  (interactive)
  (let ((context gnosis-agent-eval--context))
    (unless (and context (gnosis-agent-eval--valid-p context))
      (user-error "Response belongs to an outdated encounter"))
    (when-let* ((checkpoint (plist-get context :checkpoint)))
      (save-restriction
        (widen)
        (funcall checkpoint "cancel"
                 (buffer-substring-no-properties (point-min) (point-max)) nil "Cancelled")))
    (unless (gnosis-agent-eval--valid-p context)
      (user-error "Cancellation checkpoint retired the encounter"))
    (gnosis-agent-eval--prose context nil)
    (setf (plist-get context :result) nil)
    (setq buffer-read-only nil)
    (gnosis-agent-eval--show
     context (substitute-command-keys
              (concat "\\<gnosis-agent-eval-mode-map>Not graded: cancelled.  "
                      "Edit or \\[gnosis-agent-eval-submit] to retry"))
     'warning)
    ;; As in settlement, cancellation owns the final callback-capable step.
    (gnosis-agent-eval--stop context)
    (unless (gnosis-agent-eval--valid-p context)
      (user-error "Cancellation retired the response encounter"))))

(defun gnosis-agent-eval-quit ()
  "Leave response input without accepting a grade."
  (interactive)
  (when-let* ((context gnosis-agent-eval--context))
    (gnosis-agent-eval--prose context nil)
    (setf (plist-get context :retired) t)
    (gnosis-agent-eval--stop context)
    (when (= (recursion-depth) (1+ (plist-get context :depth)))
      (abort-recursive-edit))))

(defun gnosis-agent-eval-submit ()
  "Evaluate the response, or continue a completed result to review actions."
  (interactive)
  (let ((context gnosis-agent-eval--context))
    (unless (and context (gnosis-agent-eval--valid-p context))
      (user-error "Response belongs to an outdated encounter"))
    (cond
     ((plist-get context :attempt) (user-error "Evaluation pending; cancel or wait"))
     ((plist-get context :result)
      (unless (= (recursion-depth) (1+ (plist-get context :depth)))
        (user-error "Response input is not current"))
      (exit-recursive-edit))
     (t
      (let* ((attempt (list t))
             (response (save-restriction
                         (widen)
                         (buffer-substring-no-properties (point-min) (point-max))))
             (request (append (copy-tree (plist-get context :request))
                              (list :response response))))
        (when-let* ((checkpoint (plist-get context :checkpoint)))
          (funcall checkpoint "submit" response nil))
        (unless (gnosis-agent-eval--valid-p context)
          (user-error "Submission checkpoint retired the encounter"))
        (gnosis-agent-eval--prose context nil)
        (setf (plist-get context :attempt) attempt
              (plist-get context :response) response)
        (setq buffer-read-only t)
        (gnosis-agent-eval--show
         context (substitute-command-keys
                  "\\<gnosis-agent-eval-mode-map>Evaluating…  \\[gnosis-agent-eval-cancel] to cancel")
         'warning)
        (setf (plist-get context :timer)
              (run-at-time gnosis-agent-eval-timeout nil
                           #'gnosis-agent-eval--settle context attempt nil "Evaluation timed out"))
        (condition-case err
            (let ((cancel
                   (funcall (or (plist-get context :evaluator) gnosis-agent-eval-function) request
                            (lambda (result)
                              (gnosis-agent-eval--settle context attempt result nil))
                            (lambda (failure)
                              (gnosis-agent-eval--settle context attempt nil failure)))))
              ;; Synchronous completion or reentrant cancellation can settle
              ;; before the evaluator returns its resource handle.
              (if (eq attempt (plist-get context :attempt))
                  (if (functionp cancel)
                      (setf (plist-get context :cancel) cancel)
                    (gnosis-agent-eval--settle context attempt nil
                                             "Evaluator did not return a cancellation function"))
                (gnosis-agent-eval--cancel cancel)))
          (error (gnosis-agent-eval--settle context attempt nil (error-message-string err)))))))))

(defun gnosis-agent-eval--read (request validate display &optional evaluator restored checkpoint id prose)
  "Read a multiline REQUEST using VALIDATE and DISPLAY callbacks.
EVALUATOR defaults to `gnosis-agent-eval-function'.  RESTORED contains
:response and an optional provisional :result.  CHECKPOINT receives an
event string, response, result and optional failure before effects and teardown.
ID optionally labels the native encounter for diagnostics.
When PROSE is non-nil, show evaluator prose in the existing review buffer
and continue a graded result from the owning command loop, without resubmission.
Return the response context; never accept a grade.  A failed checkpoint
leaves the draft buffer alive, detached from transport ownership."
  (let* ((review (current-buffer))
         (buffer (generate-new-buffer "*Gnosis response*"))
         (context (list :buffer buffer :review review :database gnosis-db :id id
                        :validate validate :evaluator evaluator :checkpoint checkpoint
                        :depth (recursion-depth) :request request
                        :attempt nil :cancel nil :timer nil
                        :result (plist-get restored :result)
                        :response (plist-get restored :response)
                        :retired nil :overlay nil :prose prose :section nil))
         checkpointed)
    (unwind-protect
        (save-window-excursion
          (funcall validate)
          (setq gnosis-agent-eval--review-context context)
          (dolist (hook '(kill-buffer-hook change-major-mode-hook
                          after-set-visited-file-name-hook))
            (add-hook hook #'gnosis-agent-eval--retire-review nil t))
          (with-current-buffer buffer
            (let ((gnosis-agent-eval--initial-context context))
              (gnosis-agent-eval-mode)))
          (unless (gnosis-agent-eval--valid-p context)
            (user-error "Response setup belongs to an outdated encounter"))
          (with-current-buffer buffer
            (insert (or (plist-get restored :response) ""))
            (setf (plist-get context :overlay) (make-overlay (point-max) (point-max))))
          (funcall display)
          (unless (gnosis-agent-eval--valid-p context)
            (user-error "Response display belongs to an outdated encounter"))
          (pop-to-buffer buffer '(display-buffer-below-selected))
          (unless (and (eq (current-buffer) buffer)
                       (gnosis-agent-eval--valid-p context))
            (user-error "Response input belongs to an outdated encounter"))
          (when (plist-get context :result)
            (gnosis-agent-eval--prose context (plist-get (plist-get context :result) :explanation))
            (setq buffer-read-only t)
            (gnosis-agent-eval--show context "Evaluation restored; continue to review actions"))
          ;; Use ordinary command dispatch, not `special-event-map': a
          ;; callback's `read-event' (for example a D-Bus notification wait)
          ;; handles special events immediately in its foreign current buffer.
          ;; Ordinary events return to that reader for replay by the command
          ;; loop, where the response keymap and ownership checks apply.
          (when (plist-get context :result)
            (gnosis-agent-eval--notify context))
          (recursive-edit)
          (unless (and (gnosis-agent-eval--valid-p context) (plist-get context :result))
            (user-error "Response was not graded"))
          context)
      ;; Checkpoint before retiring transport.  Failure retains exact typed text.
      (unwind-protect
          (when (buffer-live-p buffer)
            (with-current-buffer buffer
              (when (eq gnosis-agent-eval--context context)
                (when checkpoint
                  (save-restriction
                    (widen)
                    (funcall checkpoint "pause"
                             (buffer-substring-no-properties (point-min) (point-max))
                             (plist-get context :result))))
                (setq checkpointed t))))
        (gnosis-agent-eval--prose context nil)
        (setf (plist-get context :retired) t)
        (gnosis-agent-eval--stop context)
        (setq unread-command-events
              (seq-remove (lambda (event)
                            (and (consp event) (eq (car event) 'gnosis-agent-eval-ready)
                                 (eq (nth 1 event) context)))
                          unread-command-events))
        (when (overlayp (plist-get context :overlay))
          (delete-overlay (plist-get context :overlay)))
        (when (buffer-live-p review)
          (with-current-buffer review
            (when (eq gnosis-agent-eval--review-context context)
              (setq gnosis-agent-eval--review-context nil)
              (dolist (hook '(kill-buffer-hook change-major-mode-hook
                              after-set-visited-file-name-hook))
                (remove-hook hook #'gnosis-agent-eval--retire-review t)))))
        (when (buffer-live-p buffer)
          (with-current-buffer buffer
            (when (eq gnosis-agent-eval--context context)
              (setq gnosis-agent-eval--context nil buffer-read-only nil)
              (when checkpointed
                (set-buffer-modified-p nil)
                (kill-buffer buffer)))))))))

;;;###autoload
(defun gnosis-review-agent-eval (id)
  "Collect and evaluate a multiline response to thema ID.
Return a pending binary result to the ordinary review actions.  Evaluation,
abstention, cancellation and retry never accept a scheduled or practice grade."
  (let* ((gnosis-review--display-buffer (current-buffer))
         (owner (gnosis-review--content-owner id))
         (gnosis-review--display-validate
          (lambda () (gnosis-review--content-check id owner)))
         (row (car (cadr owner)))
         (context
          (gnosis-agent-eval--read
           (list :question (nth 1 row) :reference-answer (car (nth 3 row))
                 :rubric (nth 7 row))
           gnosis-review--display-validate
           (lambda ()
             (gnosis-review--display-question id owner (nth 1 row))
             (gnosis-display-hint (mapconcat #'identity (nth 2 row) "\n")))
           nil nil nil id))
         (evaluation (plist-get context :result))
         (success (eq (plist-get evaluation :verdict) 'pass))
         (result (gnosis-review--encounter
                  (plist-put (gnosis-review-algorithm id success) :content owner)
                  row (list :kind "text" :text (plist-get context :response))
                  (nth 2 row) nil)))
    (gnosis-review--content-check id owner)
    (when-let* ((encounter (plist-get result :encounter)))
      (setf (plist-get encounter :coaching)
            (vector (list :kind "agent-eval"
                          :verdict (symbol-name (plist-get evaluation :verdict))
                          :text (plist-get evaluation :explanation)))))
    (gnosis-display-basic-answer (car (nth 3 row)) success (plist-get context :response))
    (gnosis-display-parathema (plist-get evaluation :explanation))
    (gnosis-display-parathema (nth 5 row))
    (gnosis-display-next-review (gnosis-review--result-date result) success)
    (cons success result)))

(provide 'gnosis-agent-eval)
;;; gnosis-agent-eval.el ends here
