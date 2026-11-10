;;; gnosis-model-explore.el --- Study a model scene -*- lexical-binding: t; -*-

;; Copyright (C) 2026 Free Software Foundation, Inc.
;; SPDX-License-Identifier: GPL-3.0-or-later
;; Maintainer: Thanos Apollo <public@thanosapollo.org>

;;; Commentary:
;; A study viewer over a managed model scene: orbit and zoom it, click a
;; structure to see which targets contain the click, and turn the picked
;; target into an ordinary Find or Name card.  A click is an observation
;; of where the learner pointed, never a submitted answer: nothing here
;; grades, schedules or records practice.  Agents address a viewer by its
;; id (`gnosis-model-explore-open', `-observe', `-close'); a closed or
;; replaced viewer refuses, so a stale id never reads its successor.

;;; Code:

(require 'gnosis-model)

(declare-function gnosis-agent-model-card-preview "gnosis-agent-model" (spec))
(declare-function gnosis-agent-model-card-save "gnosis-agent-model" (plan))
(declare-function gnosis-get-tags--unique "gnosis-tags")

(defface gnosis-model-explore-label
  '((t :inherit bold))
  "Face of the picked structure's name in the explorer header."
  :group 'gnosis)

(defface gnosis-model-explore-hint
  '((t :inherit shadow))
  "Face of the explorer header's hints."
  :group 'gnosis)

(defvar gnosis-model-explore--viewers (make-hash-table :test 'equal)
  "Viewer id -> its buffer, for the open explorers.")

(defvar gnosis-model-explore--count 0 "Viewers opened, for fresh ids.")

(defvar-local gnosis-model-explore--state nil
  "The explorer's plist: :id, :resource, :scene, :points, :owner, :labels,
:picks (a count), :pick (the latest observation) and :highlight.")

(defun gnosis-model-explore--verified (resource)
  "Return (PATH VIEW SCENE POINTS) of managed scene RESOURCE, validated."
  (let* ((manifest (or (gnosis-model--manifest resource)
                       (user-error "Model scene unavailable: %s" resource)))
         (first (alist-get 'id (car (gnosis-model--targets manifest))))
         (resolved (gnosis-model-resolve
                    (cons resource (mapcar (lambda (n) (format "%s" n))
                                           (alist-get 'initial_view manifest)))
                    (list first)))
         (verified (nth 2 resolved)))
    (list (car resolved) (cadr resolved) (plist-get verified :manifest) (plist-get verified :points))))

(defun gnosis-model-explore--target-record (target)
  "Return TARGET alist as a plist record."
  (list :id (alist-get 'id target) :label (alist-get 'label target)
        :kind (format "%s" (alist-get 'kind target))))

(defun gnosis-model-explore--selection (selection)
  "Record surface SELECTION as the viewer's latest pick observation."
  (let ((state gnosis-model-explore--state))
    (when (and state (eq (plist-get selection :owner) (plist-get state :owner)))
      (let ((hit (and (plist-get selection :mesh) selection)))
        (setf (plist-get state :picks) (1+ (plist-get state :picks))
              (plist-get state :pick)
              (list :serial (plist-get state :picks)
                    :frame (plist-get selection :frame)
                    :mesh (plist-get hit :mesh) :face (plist-get hit :face)
                    :point (plist-get hit :point)
                    :targets (and hit (mapcar #'gnosis-model-explore--target-record
                                              (gnosis-model-hit-targets
                                               (plist-get state :scene) hit
                                               (plist-get state :points))))))
        (force-mode-line-update)))))

(defun gnosis-model-explore--header ()
  "Return the explorer's header line."
  (let* ((state gnosis-model-explore--state)
         (pick (plist-get state :pick))
         (targets (plist-get pick :targets))
         (status (gnosis-model-backend-get :status)))
    (concat " "
            (cond ((not (gnosis-model-backend-get :live))
                   (propertize (format "Unavailable: %s" status) 'face 'error))
                  ((null pick) (propertize "Drag to orbit, wheel to zoom, click a structure"
                                           'face 'gnosis-model-explore-hint))
                  ((null targets) "No target here")
                  ((plist-get state :labels)
                   (propertize (mapconcat (lambda (target) (plist-get target :label)) targets ", ")
                               'face 'gnosis-model-explore-label))
                  (t (format "%d target%s here" (length targets) (if (cdr targets) "s" ""))))
            (propertize (substitute-command-keys
                         (concat "  \\<gnosis-model-explore-mode-map>\\[gnosis-model-explore-save-find] find card"
                                 "  \\[gnosis-model-explore-save-name] name card"
                                 "  \\[gnosis-model-explore-author] author targets"
                                 "  \\[gnosis-model-explore-quit] quit"))
                        'face 'gnosis-model-explore-hint))))

(defvar-keymap gnosis-model-explore-mode-map
  :doc "Keys of a Gnosis model explorer, over the camera keys."
  "F" #'gnosis-model-explore-save-find
  "N" #'gnosis-model-explore-save-name
  "A" #'gnosis-model-explore-author
  "l" #'gnosis-model-explore-toggle-labels
  "q" #'gnosis-model-explore-quit)

(define-minor-mode gnosis-model-explore-mode
  "Study a Gnosis model scene: orbit, zoom and click structures.
A click shows which targets contain it; it is never an answer.
\\{gnosis-model-explore-mode-map}"
  :lighter " Explore"
  :interactive nil)

(defun gnosis-model-explore--retire ()
  "Forget the current buffer's viewer."
  (when-let* ((id (plist-get gnosis-model-explore--state :id)))
    (when (eq (gethash id gnosis-model-explore--viewers) (current-buffer))
      (remhash id gnosis-model-explore--viewers))))

(cl-defun gnosis-model-explore-open (resource &key view (labels t) highlight)
  "Open an explorer of managed scene RESOURCE; return its viewer id.
VIEW is (YAW PITCH ZOOM), default the scene's; LABELS nil hides the
picked structures' names (for unaided probes); HIGHLIGHT lists target
IDs painted as cues.  Signal if the native renderer is unavailable."
  (pcase-let* ((`(,path ,initial ,scene ,points) (gnosis-model-explore--verified resource))
               (view (gnosis-model--view (or view initial)))
               (highlight (mapcar (lambda (id) (gnosis-model-target scene id)) highlight))
               (buffer (let ((display-buffer-overriding-action '(display-buffer-same-window)))
                         (gnosis-model-open path view (gnosis-model--canvas-size))))
               (id (format "model-viewer-%d" (setq gnosis-model-explore--count
                                                   (1+ gnosis-model-explore--count)))))
    (with-current-buffer buffer
      (rename-buffer (format "*Gnosis model: %s*"
                             (or (alist-get 'label (car (alist-get 'objects scene))) id))
                     t)
      (setq gnosis-model-explore--state
            (list :id id :resource resource :scene scene :points points
                  :owner (gnosis-model-backend-get :owner) :labels labels
                  :picks 0 :pick nil :highlight (mapcar (lambda (target) (alist-get 'id target)) highlight)))
      (gnosis-model-explore-mode 1)
      (setq-local header-line-format '(:eval (gnosis-model-explore--header)))
      (add-hook 'gnosis-model-backend-selection-hook #'gnosis-model-explore--selection nil t)
      (add-hook 'kill-buffer-hook #'gnosis-model-explore--retire nil t)
      (when highlight (gnosis-model-backend-cue highlight))
      (puthash id buffer gnosis-model-explore--viewers))
    id))

(defun gnosis-model-explore--buffer (id)
  "Return viewer ID's live buffer whose renderer is still its own, or signal."
  (let ((buffer (gethash id gnosis-model-explore--viewers)))
    (unless (and (buffer-live-p buffer)
                 (with-current-buffer buffer
                   (and (equal id (plist-get gnosis-model-explore--state :id))
                        (eq (plist-get gnosis-model-explore--state :owner)
                            (gnosis-model-backend-get :owner))
                        (gnosis-model-backend-get :owner))))
      (user-error "Model viewer %s is closed or replaced; open it again" id))
    buffer))

(defun gnosis-model-explore-observe (id)
  "Return what viewer ID shows now: its scene, renderer state and picks.
:pick is the learner's latest click as an observation, with :current
nil once the picture changed since; :targets lists the targets that
contain it.  No answer is submitted or graded."
  (with-current-buffer (gnosis-model-explore--buffer id)
    (let* ((state gnosis-model-explore--state)
           (pick (plist-get state :pick)))
      (list :viewer id :resource (plist-get state :resource)
            :live (and (gnosis-model-backend-get :live) t)
            :ready (and (gnosis-model-backend-get :ready) t)
            :status (gnosis-model-backend-get :status)
            :view (gnosis-model-backend-get :view)
            :labels (and (plist-get state :labels) t)
            :highlight (plist-get state :highlight)
            :picks (plist-get state :picks)
            :pick (and pick (append (list :current (equal (plist-get pick :frame)
                                                         (gnosis-model-backend-get :frame)))
                                    pick))))))

(defun gnosis-model-explore-close (id)
  "Close viewer ID, freeing its renderer resources."
  (kill-buffer (gnosis-model-explore--buffer id))
  t)

(defun gnosis-model-explore-viewers ()
  "Return the ids of the open viewers."
  (let (ids)
    (maphash (lambda (id buffer) (when (buffer-live-p buffer) (push id ids)))
             gnosis-model-explore--viewers)
    (sort ids #'string<)))

;;;; Commands

(defun gnosis-model-explore--picked ()
  "Return the picked target record, asking when the click hit several."
  (let* ((pick (or (plist-get gnosis-model-explore--state :pick)
                   (user-error "Click a structure first")))
         (targets (or (plist-get pick :targets) (user-error "No target where you clicked"))))
    (if (null (cdr targets)) (car targets)
      (let ((choices (mapcar (lambda (target) (cons (plist-get target :label) target)) targets)))
        (cdr (assoc (completing-read "Target: " choices nil t) choices))))))

(defun gnosis-model-explore--save (type)
  "Save a TYPE (\"find\" or \"name\") card for the picked target."
  (require 'gnosis-agent-model)
  (let* ((target (gnosis-model-explore--picked))
         (view (or (and (gnosis-model-backend-get :ready) (gnosis-model-backend-get :view))
                   (user-error "Wait for the view to be ready")))
         (find (equal type "find"))
         (text (read-string (if find "Find card question: " "Name card answer: ")
                            (if find (format "Click the %s." (gnosis-model--label-phrase
                                                            (plist-get target :label)))
                              (plist-get target :label))))
         (tags (completing-read-multiple "Tags (separated by ,): " (gnosis-get-tags--unique)))
         (plan (gnosis-agent-model-card-preview
                (append (list :resource (plist-get gnosis-model-explore--state :resource)
                              :type type :target (plist-get target :id)
                              :view (vconcat view)
                              :tags (vconcat tags))
                        (if find (list :question text) (list :answer text))))))
    (when (y-or-n-p (format "Save %s card \"%s\" on %s? " (if find "Find" "Name")
                            (if find (plist-get plan :question) (aref (plist-get plan :answers) 0))
                            (plist-get target :label)))
      (let ((receipt (gnosis-agent-model-card-save plan)))
        (message "Saved thema %s" (plist-get receipt :thema-id))
        receipt))))

(defun gnosis-model-explore-save-find ()
  "Save a Find card asking for the picked structure at the current view."
  (interactive nil gnosis-model-explore-mode)
  (gnosis-model-explore--save "find"))

(defun gnosis-model-explore-save-name ()
  "Save a Name card for the picked structure at the current view."
  (interactive nil gnosis-model-explore-mode)
  (gnosis-model-explore--save "name"))

(defun gnosis-model-explore-toggle-labels ()
  "Show or hide the picked structures' names."
  (interactive nil gnosis-model-explore-mode)
  (setf (plist-get gnosis-model-explore--state :labels)
        (not (plist-get gnosis-model-explore--state :labels)))
  (force-mode-line-update))

(defun gnosis-model-explore-author ()
  "Author point and region targets and cards on this scene natively."
  (interactive nil gnosis-model-explore-mode)
  (let ((resource (plist-get gnosis-model-explore--state :resource)))
    (kill-buffer)
    (gnosis-model-author-scene resource
                               (completing-read-multiple "Tags (separated by ,): "
                                                         (gnosis-get-tags--unique)))))

(defun gnosis-model-explore-quit ()
  "Close the explorer and free its renderer resources."
  (interactive nil gnosis-model-explore-mode)
  (kill-buffer))

;;;###autoload
(defun gnosis-model-explore (resource)
  "Study model scene RESOURCE: orbit, zoom and click its structures.
Interactively, choose a managed scene."
  (interactive
   (list (let* ((choices (gnosis-model-explore--choices))
                (choice (completing-read "Scene: " choices nil t)))
           (cdr (assoc choice choices)))))
  (gnosis-model-explore-open resource))

(defun gnosis-model-explore--choices ()
  "Return (LABEL . RESOURCE) for every managed scene."
  (or (mapcar (pcase-lambda (`(,resource . ,manifest))
                (cons (format "%s (%d objects, %s)"
                              (alist-get 'label (car (alist-get 'objects manifest)))
                              (length (alist-get 'objects manifest))
                              (substring resource 0 8))
                      resource))
              (gnosis-model-managed-scenes))
      (user-error "No model scenes yet: import one with `gnosis-model-author-scene'")))

(provide 'gnosis-model-explore)
;;; gnosis-model-explore.el ends here
