;;; gnosis-agent-model.el --- Model scenes and cards for agents -*- lexical-binding: t; -*-

;; Copyright (C) 2026 Free Software Foundation, Inc.
;; SPDX-License-Identifier: GPL-3.0-or-later
;; Maintainer: Thanos Apollo <public@thanosapollo.org>

;;; Commentary:
;; Optional companion to `gnosis-agent-content' for 3D model scenes.
;; Agents list managed scenes, read a scene's targets, show a scene to
;; the learner in a native viewer and observe where the learner clicked,
;; and propose permanent Find or Name themata on existing targets.
;; A click is an observation of where the learner pointed, never a
;; submitted answer.  A preview writes
;; nothing; saving takes the exact preview back and recomputes it in the
;; write transaction, so the shown plan is the saved card.  Saving never
;; grades, schedules beyond a new card's initial state, or touches
;; practice and review sessions.  Results follow API v1 conventions:
;; keyword plists, vectors, strings, numbers, t, :false and nil.
;;
;; Scene targets (objects, surface points and triangle regions) are the
;; vocabulary of a scene, not answers of any thema: nothing here reads an
;; existing thema's answer.  Points and regions are authored natively with
;; `gnosis-model-author-scene'.

;;; Code:

(require 'gnosis-agent-content)
(require 'gnosis-model)
(require 'gnosis-model-explore)

(defvar gnosis-review--running)
(declare-function gnosis-model-backend-romacs-unavailable-reason "gnosis-model-backend-romacs" ())
(declare-function gnosis--thema-expected-links "gnosis-links" (keimenon parathema))

(defconst gnosis-agent-model--resource-re "\\`\\([0-9a-f]\\{64\\}\\)/scene\\.json\\'"
  "Managed scene resource reference.")

(defun gnosis-agent-model--resource (resource)
  "Validate RESOURCE and return its validated manifest."
  (unless (and (stringp resource) (string-match gnosis-agent-model--resource-re resource))
    (user-error "Resource must be a managed scene reference: HEX64/scene.json"))
  (gnosis-model--scene
   (gnosis-assets-file (expand-file-name (match-string 1 resource) (gnosis-assets-root))
                       "scene.json")))

(defun gnosis-agent-model--usage ()
  "Return an alist of (RESOURCE . THEMATA-COUNT) for model themata."
  (let (counts)
    (dolist (hypothesis (gnosis-select 'hypothesis 'themata '(in type ["model" "model-name"]) t))
      (cl-incf (alist-get (car hypothesis) counts 0 nil #'equal)))
    counts))

(defun gnosis-agent-model-scenes ()
  "Return the managed model scenes beside the connected database.
Each item has :resource, :label (its first object's label), :objects,
:targets and :themata, the number of model themata using it.  Manifests
are read for labels only; describe a scene to validate it."
  (let ((usage (gnosis-agent-model--usage)))
    (list :api-version 1
          :scenes
          (vconcat
           (mapcar (pcase-lambda (`(,resource . ,manifest))
                     (list :resource resource
                           :label (or (alist-get 'label (car (alist-get 'objects manifest))) "")
                           :objects (length (alist-get 'objects manifest))
                           :targets (length (gnosis-model--targets manifest))
                           :themata (alist-get resource usage 0 nil #'equal)))
                   (gnosis-model-managed-scenes))))))

(defun gnosis-agent-model--target-record (target)
  "Return TARGET alist as an API record."
  (list :id (alist-get 'id target) :label (alist-get 'label target)
        :kind (format "%s" (alist-get 'kind target)) :mesh (alist-get 'mesh target)))

(defun gnosis-agent-model-describe (resource)
  "Validate scene RESOURCE and return its provenance, camera and targets."
  (let ((scene (gnosis-agent-model--resource resource)))
    (list :api-version 1 :resource resource
          :license (alist-get 'license scene) :source (alist-get 'source scene)
          :initial-view (vconcat (alist-get 'initial_view scene))
          :targets (vconcat (mapcar #'gnosis-agent-model--target-record
                                    (gnosis-model--targets scene))))))

;;; Viewers

(defun gnosis-agent-model--status ()
  "Return the native renderer's availability as API data."
  (require 'gnosis-model-backend-romacs)
  (let ((reason (gnosis-model-backend-romacs-unavailable-reason)))
    (list :available (if reason :false t) :reason reason)))

(defun gnosis-agent-model--observation (observation)
  "Return viewer OBSERVATION as API data."
  (let ((pick (plist-get observation :pick)))
    (list :api-version 1 :viewer (plist-get observation :viewer)
          :resource (plist-get observation :resource)
          :live (if (plist-get observation :live) t :false)
          :ready (if (plist-get observation :ready) t :false)
          :status (plist-get observation :status)
          :view (vconcat (plist-get observation :view))
          :labels (if (plist-get observation :labels) t :false)
          :highlight (vconcat (plist-get observation :highlight))
          :picks (plist-get observation :picks)
          :pick (and pick
                     (list :serial (plist-get pick :serial)
                           :current (if (plist-get pick :current) t :false)
                           :mesh (plist-get pick :mesh) :face (plist-get pick :face)
                           :point (vconcat (plist-get pick :point))
                           :targets (vconcat (plist-get pick :targets))
                           :kind "observation")))))

(cl-defun gnosis-agent-model-show (resource &key view (labels t) highlight)
  "Show scene RESOURCE to the learner in a native viewer; return its state.
VIEW is [YAW PITCH ZOOM]; LABELS :false hides picked names (an unaided
probe); HIGHLIGHT is a vector of target IDs painted as cues.  The viewer
replaces nothing else; close it with `gnosis-agent-model-close'."
  (gnosis-agent-model--resource resource)
  (when gnosis-review--running
    (user-error "Native encounter active; show the model after it"))
  (let ((reason (plist-get (gnosis-agent-model--status) :reason)))
    (when reason (user-error "Native model renderer unavailable: %s" reason)))
  (gnosis-agent-model--observation
   (gnosis-model-explore-observe
    (gnosis-model-explore-open resource :view (and view (append view nil))
                               :labels (not (memq labels '(nil :false)))
                               :highlight (gnosis-agent-model--ids highlight "highlight")))))

(defun gnosis-agent-model-observe (viewer)
  "Return what VIEWER shows: renderer state, camera and the learner's
latest click as an observation, with the targets containing it."
  (gnosis-agent-model--observation (gnosis-model-explore-observe viewer)))

(defun gnosis-agent-model-close (viewer)
  "Close VIEWER, freeing its renderer resources."
  (gnosis-model-explore-close viewer)
  (list :api-version 1 :viewer viewer :closed t))

(defun gnosis-agent-model-status ()
  "Return renderer availability and the open viewers."
  (append (list :api-version 1) (gnosis-agent-model--status)
          (list :viewers (vconcat (gnosis-model-explore-viewers)))))

;;; Cards

(defconst gnosis-agent-model--spec-keys
  '(:resource :type :target :question :answer :view :cues :alternates :tags :parathema)
  "Keys of a card specification.")

(defun gnosis-agent-model--ids (value what)
  "Return VALUE, a vector of target ID strings, as a list."
  (unless (and (or (null value) (vectorp value)) (seq-every-p #'stringp value))
    (user-error "%s must be an array of target IDs" what))
  (append value nil))

(defun gnosis-agent-model--text (value what &optional default)
  "Return VALUE as trimmed nonempty text, or DEFAULT when it is nil."
  (cond ((and (null value) default) default)
        ((and (stringp value) (not (string-empty-p (string-trim value)))) (string-trim value))
        (t (user-error "%s must be nonempty text" what))))

(defun gnosis-agent-model--number (number)
  "Return NUMBER as model camera text."
  (if (and (floatp number) (= number (ffloor number)) (< (abs number) 1e15))
      (number-to-string (truncate number))
    (number-to-string number)))

(defun gnosis-agent-model--card (spec)
  "Return the native fields of card SPEC after domain validation.
Value is (TYPE QUESTION HYPOTHESIS ANSWER PARATHEMA TAGS TARGET)."
  (let ((unknown (cl-loop for key in spec by #'cddr
                          unless (memq key gnosis-agent-model--spec-keys) collect key)))
    (when (or unknown (cl-oddp (length spec)))
      (user-error "Unknown card fields: %S" unknown)))
  (let* ((resource (plist-get spec :resource))
         (scene (gnosis-agent-model--resource resource))
         (name (pcase (plist-get spec :type)
                 ("find" nil) ("name" t)
                 (_ (user-error "Card type must be \"find\" or \"name\""))))
         (target (gnosis-model-target scene (gnosis-agent-model--text
                                             (plist-get spec :target) "target")))
         (id (alist-get 'id target))
         (label (alist-get 'label target))
         (view (gnosis-model--view (append (or (plist-get spec :view)
                                               (alist-get 'initial_view scene))
                                           nil)))
         (camera (mapcar #'gnosis-agent-model--number view))
         (cues (gnosis-agent-model--ids (plist-get spec :cues) "cues"))
         (alternates (gnosis-agent-model--ids (plist-get spec :alternates) "alternates"))
         (tags (gnosis-agent-model--ids (plist-get spec :tags) "tags"))
         (parathema (or (plist-get spec :parathema) "")))
    (unless (stringp parathema) (user-error "parathema must be text"))
    (when (and name (or cues alternates))
      (user-error "A Name card has no cues or alternate targets"))
    (when (and name (plist-get spec :question))
      (user-error "A Name card asks to name the highlighted structure; give answer text instead"))
    (when (and (not name) (plist-get spec :answer))
      (user-error "A Find card's answer is its target; give question text instead"))
    (gnosis-tags--check-org tags)
    (let* ((type (if name "model-name" "model"))
           (question (if name "Name the highlighted structure."
                       (gnosis-agent-model--text
                        (plist-get spec :question) "question"
                        (format "Click the %s." (gnosis-model--label-phrase label)))))
           (hypothesis (if name (cons resource (cons id camera))
                         (append (cons resource camera) cues)))
           (answer (if name (list (gnosis-agent-model--text (plist-get spec :answer) "answer" label))
                     (cons id alternates))))
      (gnosis-add-thema--assert-common question tags 0 nil)
      (gnosis-model-fields type hypothesis answer)
      (list type question hypothesis answer parathema tags target))))

(defun gnosis-agent-model-card-preview (spec)
  "Preview a permanent model thema from card SPEC; write nothing.
SPEC is a plist: :resource, :type \"find\" or \"name\", :target (an
existing target ID of the scene), optional :question (Find; default
\"Click the LABEL.\"), :answer (Name; default the target's label),
:view [YAW PITCH ZOOM] (default the scene's initial view), :cues and
:alternates (Find only: target ID vectors), :tags and :parathema.
Return the exact plan to pass to `gnosis-agent-model-card-save'."
  (pcase-let ((`(,type ,question ,hypothesis ,answer ,parathema ,tags ,target)
               (gnosis-agent-model--card spec)))
    (list :api-version 1 :owner (gnosis-agent-content--owner) :spec (copy-tree spec t)
          :type type :question question :hypothesis (vconcat hypothesis)
          :answers (vconcat answer) :parathema parathema :tags (vconcat tags)
          :target (gnosis-agent-model--target-record target))))

(defvar gnosis-agent-model--receipts (make-hash-table :test 'eq :weakness 'key)
  "Exact saved plan -> its receipt, so a retry never saves twice.")

(defun gnosis-agent-model-card-save (plan)
  "Save the exact PLAN `gnosis-agent-model-card-preview' returned.
Refuse a changed plan, scene, database owner or an active native
encounter.  The new thema starts unreviewed like any authored card; no
grade, practice or review evidence is written.  Return a receipt with
decimal-string :thema-id and the fetched :item.  Saving the same plan
again returns its first receipt."
  (or (gethash plan gnosis-agent-model--receipts)
      (let ((db (gnosis--ensure-db)))
        (when gnosis-review--running
          (user-error "Native encounter active; save the card after it"))
        (unless (equal (plist-get plan :owner) (gnosis-agent-content--owner))
          (user-error "Database owner changed; preview again"))
        (gnosis-sqlite-with-transaction db
          (unless (equal plan (gnosis-agent-model-card-preview (plist-get plan :spec)))
            (user-error "Model card preview changed; preview again"))
          (pcase-let* ((`(,type ,question ,hypothesis ,answer ,parathema ,tags ,_)
                        (gnosis-agent-model--card (plist-get plan :spec)))
                       (id (gnosis-add-thema-fields
                            type question hypothesis answer parathema tags 0
                            (gnosis--thema-expected-links question parathema)))
                       (key (number-to-string id)))
            (puthash plan
                     (list :api-version 1 :thema-id key
                           :item (aref (plist-get (gnosis-agent-content-fetch (vector key)) :items) 0))
                     gnosis-agent-model--receipts))))))

(provide 'gnosis-agent-model)
;;; gnosis-agent-model.el ends here
