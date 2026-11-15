;;; gnosis-model.el --- Managed model resources -*- lexical-binding: t; -*-

;; Copyright (C) 2026 Free Software Foundation, Inc.
;; Author: Thanos Apollo <public@thanosapollo.org>
;; SPDX-License-Identifier: GPL-3.0-or-later
;; Maintainer: Thanos Apollo <public@thanosapollo.org>

;;; Commentary:
;; Model themata store (RESOURCE YAW PITCH ZOOM) strings in hypothesis and
;; one stable target ID in answer.  A relational Find appends cue target
;; IDs to the hypothesis and lists further acceptable targets in the answer.
;; Resources are immutable scene directories, addressed by a digest of the
;; manifest and every geometry file.  No renderer
;; is needed to load Gnosis, import resources, or edit model themata.

;;; Code:

(require 'gnosis-assets)
(require 'json)
(require 'seq)
(require 'subr-x)
(require 'org)

(declare-function gnosis-add-thema "gnosis")
(declare-function gnosis-add-thema--assert-common "gnosis")
(declare-function gnosis-add-thema--dispatch "gnosis")
(declare-function gnosis-add-thema-fields "gnosis")
(declare-function gnosis-get-tags--unique "gnosis-tags")
(declare-function gnosis-tags--check-org "gnosis-tags")
(declare-function gnosis-export--insert-thema "gnosis-export-import")
(declare-function gnosis-export-parse-themata "gnosis-export-import")
(require 'gnosis-model-backend)
(require 'gnosis-model-camera)
(defvar gnosis-export-separator)

(defconst gnosis-model--directory
  (file-name-directory (or load-file-name buffer-file-name))
  "Directory of the installed Gnosis model library.")

(defvar-local gnosis-model--author-context nil
  "Ownership and tentative fields for the active visual authoring session.")

(defcustom gnosis-model-renderer-directory nil
  "Directory containing canvas-3d.el, render.py and .venv.
Romacs does not ship canvas-3d for now, so nil leaves the 3D model display
off; the rest of Gnosis works without it.  A checkout named here must
implement scene opening and the selection hook protocol."
  :type '(choice (const nil) directory)
  :group 'gnosis)

(defalias 'gnosis-model--root #'gnosis-assets-root)

(defun gnosis-model--view (view)
  "Validate and return camera VIEW as three finite numbers."
  (unless (and (proper-list-p view) (= (length view) 3)
               (seq-every-p (lambda (n) (and (numberp n) (<= (abs n) 1000000))) view)
               (<= 0.25 (nth 2 view) 4))
    (user-error "Camera requires finite yaw, pitch and zoom 0.25..4"))
  view)

(defun gnosis-model--scene (file &optional license source geometry-p)
  "Read and validate scene FILE, optionally supplying LICENSE and SOURCE.
Return the manifest alist; it owns stable targets and provenance.
With GEOMETRY-P return (MANIFEST GEOMETRY REVISION), verifying that asset
bytes did not change during topology validation."
  (when (or (file-remote-p file) (file-symlink-p file)
            (not (file-regular-p file))
            (> (file-attribute-size (file-attributes file)) 65536))
    (user-error "Model scene must be a local JSON file below 64 KiB"))
  (let* ((manifest-hash (and geometry-p (gnosis-assets-hash file)))
         (scene (json-parse-string
                 (with-temp-buffer (insert-file-contents file) (buffer-string))
                 :object-type 'alist :array-type 'list))
         (objects (alist-get 'objects scene))
         (directory (file-name-directory file)))
    (when license (setf (alist-get 'license scene) license))
    (when source (setf (alist-get 'source scene) source))
    (unless (and (proper-list-p objects) (<= 1 (length objects) 255)
                 (seq-every-p
                  (lambda (object)
                    (and (stringp (alist-get 'id object))
                         (string-match-p "\\`[[:alnum:]_-]+\\'" (alist-get 'id object))
                         (stringp (alist-get 'label object))
                         (not (string-empty-p (alist-get 'label object)))
                         (equal (file-name-extension (alist-get 'path object)) "obj")
                         (gnosis-assets-file directory (alist-get 'path object))))
                  objects)
                 (= (length objects)
                    (length (delete-dups (mapcar (lambda (o) (alist-get 'id o)) objects)))))
      (user-error "Scene requires 1..255 unique stable targets and OBJ files"))
    (dolist (field '(license source))
      (unless (and (stringp (alist-get field scene))
                   (not (string-empty-p (string-trim (alist-get field scene)))))
        (user-error "Scene requires %s metadata" field)))
    (gnosis-model--view (alist-get 'initial_view scene))
    (when (and geometry-p (not (equal manifest-hash (gnosis-assets-hash file))))
      (user-error "Model manifest changed during preparation"))
    (let* ((names (cons "scene.json" (mapcar (lambda (o) (alist-get 'path o)) objects)))
           (before (and geometry-p (gnosis-assets-revision directory names)))
           (geometry (gnosis-model--validate-targets scene directory)))
      (if (not geometry-p) scene
        (unless (equal before (gnosis-assets-revision directory names))
          (user-error "Model resource changed during preparation"))
        (list scene geometry before)))))

(defun gnosis-model--revision (directory)
  "Return validated scene digest for DIRECTORY."
  (let ((scene (gnosis-model--scene (gnosis-assets-file directory "scene.json"))))
    (gnosis-assets-revision
     directory (cons "scene.json" (mapcar (lambda (o) (alist-get 'path o))
                                         (alist-get 'objects scene))))))

(defun gnosis-model-import (file &optional license source)
  "Import local scene FILE into managed assets and return its reference.
Optional LICENSE and SOURCE supply missing provenance.  Stage all files,
validate their bytes, then publish by rename.  Retry shares identical resources.
Failure or quit removes only the unpublished staging directory."
  (let* ((database (gnosis--ensure-db))
         (file (gnosis-assets-file (file-name-directory (expand-file-name file))
                                   (file-name-nondirectory file)))
         (scene (gnosis-model--scene file license source))
         (json-encoding-pretty-print nil)
         (json-encoding-separator ",")
         (text (json-encode scene)))
    (when (> (string-bytes (encode-coding-string text 'utf-8-unix)) 65536)
      (user-error "Model scene must be below 64 KiB"))
    (gnosis-assets-root database)
    (concat (gnosis-assets-import
             (file-name-directory file)
             (mapcar (lambda (o) (alist-get 'path o)) (alist-get 'objects scene))
             (list (cons "scene.json" text)))
            "/scene.json")))

(defvar gnosis-model--vertex-sets nil
  "When a cons, OBJ parsing pushes each file's vertex vector on its car.")

(defun gnosis-model-resolve (hypothesis answer &optional root camera)
  "Validate model HYPOTHESIS and ANSWER; return scene path and camera.
HYPOTHESIS is (RESOURCE YAW PITCH ZOOM), all strings.  ANSWER contains one
stable target ID.  Refuse missing or changed resources, never score them.
ROOT defaults to the connected database asset root.  The third return value
is a compact verified scene with face counts and target point coordinates.
With CAMERA, variation bounds as in `gnosis-model-camera-variation', it
also lists under :views the varied cameras keeping the target visible
and under :camera the bounds they were prepared for."
  (unless (and (proper-list-p hypothesis) (= (length hypothesis) 4)
               (seq-every-p #'stringp hypothesis)
               (string-match-p "\\`[0-9a-f]\\{64\\}/scene\\.json\\'" (car hypothesis))
               (proper-list-p answer) (= (length answer) 1) (stringp (car answer)))
    (user-error "Invalid model resource reference or target"))
  (let* ((root (or root (gnosis-assets-root)))
         (revision (car (split-string (car hypothesis) "/")))
         (directory (expand-file-name revision root))
         (file (expand-file-name "scene.json" directory))
         (view (mapcar
                (lambda (text)
                  (unless (string-match-p
                           "\\`[-+]?[0-9]+\\(?:\\.[0-9]+\\)?\\(?:[eE][-+]?[0-9]+\\)?\\'" text)
                    (user-error "Invalid model camera number"))
                  (string-to-number text)) (cdr hypothesis))))
    (let* ((gnosis-model--vertex-sets (list nil))
           (validated (gnosis-model--scene file nil nil t))
           (scene (car validated))
           (geometry (cadr validated))
           (names (cons "scene.json" (mapcar (lambda (o) (alist-get 'path o))
                                            (alist-get 'objects scene))))
           (radius (gnosis-model--scene-radius (car gnosis-model--vertex-sets))))
      (unless (and (not (file-symlink-p root))
                   (equal revision (nth 2 validated)))
        (user-error "Model unavailable: revision changed"))
      (gnosis-model-target scene (car answer))
      (list file (gnosis-model--view view)
            (list :manifest scene :root root :revision revision :names names
                  :radius radius
                  :views (and camera
                              (gnosis-model--camera-views
                               scene geometry (cons (gnosis-model-camera-centre
                                                     (car gnosis-model--vertex-sets))
                                                    radius)
                               (car answer) (gnosis-model--view view) camera))
                  :camera camera
                  :counts (mapcar (lambda (entry) (cons (car entry) (length (cdr entry))))
                                  geometry)
                  :points (mapcar
                           (lambda (target)
                             (cons (alist-get 'id target)
                                   (gnosis-model--point
                                    target (cdr (assoc (alist-get 'mesh target) geometry)))))
                           (seq-filter (lambda (target) (equal (alist-get 'kind target) "point"))
                                       (gnosis-model--targets scene))))))))

(defun gnosis-model--target-faces (target)
  "Return the faces of TARGET's mesh it covers, or nil for the whole mesh."
  (if (equal (alist-get 'kind target) "point") (list (alist-get 'face target))
    (alist-get 'faces target)))

(defun gnosis-model--camera-views (scene geometry frame id view bounds)
  "Return cameras within BOUNDS of VIEW keeping target ID of SCENE visible.
GEOMETRY is the validated triangle alist and FRAME its (CENTRE . RADIUS).
A point target is judged at its surface point, others at sampled
vertices and centroids of their triangles."
  (let* ((target (gnosis-model-target scene id))
         (mesh (alist-get 'mesh target))
         (faces (gnosis-model--target-faces target))
         (triangles (cdr (assoc mesh geometry))))
    (gnosis-model-camera-views
     geometry frame view bounds (cons mesh faces)
     (if (equal (alist-get 'kind target) "point")
         (list (gnosis-model-camera-normalize (gnosis-model--point target triangles) frame))
       (gnosis-model-camera-target-points triangles frame faces)))))

(defun gnosis-model-review-view (fields id)
  "Return the starting camera for reviewing model FIELDS of thema ID.
While `gnosis-model-camera-variation' equals the bounds the :views were
prepared for, choose among them by ID and its number of scheduled and
practice reviews; otherwise return the authored camera."
  (or (and gnosis-model-camera-variation
           (equal gnosis-model-camera-variation
                  (plist-get (plist-get fields :verified) :camera))
           (gnosis-model-camera-choose
            (plist-get (plist-get fields :verified) :views) id
            (+ (length (gnosis-select 'event-id 'review-events `(= thema-id ,id) t))
               (length (gnosis-select 'event-id 'practice-events `(= thema-id ,id) t)))))
      (plist-get fields :view)))

(cl-defun gnosis-model--save (id type keimenon hypothesis answer parathema tags suspend links
                               &optional (accepted-aliases nil aliases-p))
  "Save model ID of TYPE with validated content fields.
Validate KEIMENON, HYPOTHESIS, ANSWER, PARATHEMA, TAGS, SUSPEND and LINKS.
Forward ACCEPTED-ALIASES when supplied; omission preserves stored aliases."
  (gnosis-add-thema--assert-common keimenon tags suspend links)
  (gnosis-model-fields type hypothesis answer)
  (apply #'gnosis-add-thema--dispatch id type keimenon hypothesis answer parathema tags suspend links
         (and aliases-p (list accepted-aliases))))

(defun gnosis-model--object-file (file)
  "Return readable local OBJ FILE, rejecting symlinks and directories."
  (when (or (file-remote-p file) (file-symlink-p file)
            (not (file-regular-p file)) (not (file-readable-p file))
            (not (equal (downcase (or (file-name-extension file) "")) "obj")))
    (user-error "Choose a readable local OBJ file, not a symlink"))
  (file-truename file))

(defun gnosis-model-import-objects (objects license source)
  "Import local OBJECTS with explicit LICENSE and SOURCE; return a reference.
OBJECTS is an alist of (FILE . LABEL).  Files must share scene coordinates.
Generate stable IDs from canonical source paths, independent of geometry bytes
and input order.  Copy to safe distinct basenames; retain no source paths in
those names.  Geometry changes create new immutable revisions, not new IDs."
  (unless (and (proper-list-p objects) (<= 1 (length objects) 255))
    (user-error "Choose 1..255 objects"))
  (dolist (value (list license source))
    (unless (and (stringp value) (not (string-empty-p (string-trim value))))
      (user-error "Supply the actual license/attribution and source")))
  (let* ((database (gnosis--ensure-db))
         (entries (sort (mapcar (lambda (entry)
                                 (unless (and (consp entry) (stringp (cdr entry))
                                              (not (string-empty-p (string-trim (cdr entry)))))
                                   (user-error "Every object requires a label"))
                                 (cons (gnosis-model--object-file (car entry)) (cdr entry)))
                               objects)
                        (lambda (a b) (string< (car a) (car b)))))
         (paths (mapcar #'car entries))
         (stage (make-temp-file "gnosis-model-scene-" t)))
    (unwind-protect
        (progn
          (unless (= (length paths) (length (delete-dups (copy-sequence paths))))
            (user-error "Choose each OBJ file only once"))
          (let* ((manifest-objects
                  (mapcar
                   (lambda (entry)
                     (let* ((id (concat "object-" (secure-hash
                                                  'sha256 (encode-coding-string
                                                           (car entry) 'utf-8-unix))))
                            (name (concat id ".obj")))
                       (copy-file (car entry) (expand-file-name name stage))
                       `((id . ,id) (label . ,(cdr entry)) (path . ,name))))
                   entries))
                 (scene `((objects . ,(vconcat manifest-objects))
                          (initial_view . [0 0 1]) (license . ,license) (source . ,source)))
                 (file (expand-file-name "scene.json" stage))
                 (json-encoding-pretty-print nil)
                 (json-encoding-separator ",")
                 (coding-system-for-write 'utf-8-unix))
            (with-temp-file file (insert (json-encode scene)))
            (gnosis-assets-root database)
            (gnosis-model-import file)))
      (delete-directory stage t))))

(defun gnosis-model--author-check (context)
  "Reject changed authoring owner or database in CONTEXT."
  (when (eq context gnosis-model--author-context)
    (unless (and context (eq (plist-get context :process) (gnosis-model-backend-get :owner))
                 (gnosis-model-backend-get :live))
      (user-error "Model authoring renderer changed")))
  (let ((owner (plist-get context :buffer)))
    (unless (and (buffer-live-p owner)
                 (eq (plist-get context :database) (gnosis--ensure-db))
                 (with-current-buffer owner
                   (and (eq major-mode (plist-get context :mode))
                        (= (buffer-chars-modified-tick) (plist-get context :tick)))))
      (user-error "Model authoring owner or database changed"))))

(defun gnosis-model--read-source (context)
  "Read and import OBJ files or an advanced scene manifest for CONTEXT."
  (let* ((file (expand-file-name
                (read-file-name "Model OBJ (or scene JSON): " nil nil t)))
         (manifest-p (equal (downcase (or (file-name-extension file) "")) "json"))
         (objects
          (unless manifest-p
            (let ((files (list (gnosis-model--object-file file))))
              (while (and (< (length files) 255) (y-or-n-p "Add another OBJ to this scene? "))
                (setq files (append files
                                    (list (gnosis-model--object-file
                                           (read-file-name "Next OBJ: " nil nil t))))))
              (mapcar (lambda (path)
                        (cons path (read-string (format "Label for %s: " path)
                                                (file-name-base path)))) files))))
         (license (read-string (if manifest-p
                                   "License/attribution (blank uses manifest): "
                                 "License/attribution (include required notices): ")))
         (source (read-string (if manifest-p "Source (blank uses manifest): "
                                "Source/provenance: "))))
    (gnosis-model--author-check context)
    (if manifest-p
        (gnosis-model-import file (unless (string-empty-p license) license)
                             (unless (string-empty-p source) source))
      (gnosis-model-import-objects objects license source))))

(defun gnosis-model--read-numeric (hypothesis answer)
  "Read advanced target and camera fields from HYPOTHESIS and ANSWER."
  (let* ((resolved (gnosis-model-resolve hypothesis answer))
         (objects (gnosis-model--targets (gnosis-model--scene (car resolved))))
         (choices (mapcar (lambda (o)
                            (cons (format "%s (%s)" (alist-get 'label o) (alist-get 'id o))
                                  (alist-get 'id o))) objects))
         (target (cdr (assoc (completing-read "Expected target: " choices nil t) choices)))
         (view (gnosis-model--view
                (cl-mapcar (lambda (label value) (read-number label value))
                           '("Starting yaw: " "Starting pitch: " "Starting zoom: ")
                           (cadr resolved)))))
    (list (cons (car hypothesis) (mapcar #'number-to-string view)) (list target))))

(defun gnosis-model--read-fields (&optional existing)
  "Read visual model authoring fields, optionally reframing EXISTING fields.
Return (HYPOTHESIS ANSWER).  A prefix argument selects advanced numeric
input instead of the canvas.  Import immutable assets before visual input."
  (let* ((context (list :buffer (current-buffer) :mode major-mode
                        :tick (buffer-chars-modified-tick) :database (gnosis--ensure-db)))
         (advanced current-prefix-arg)
         (keep (and existing (car (car existing))
                    (y-or-n-p "Reframe the current scene (no imports)? ")))
         (fields
          (if keep existing
            (let* ((reference (gnosis-model--read-source context))
                   (scene (gnosis-model--scene
                           (expand-file-name reference (gnosis-assets-root)))))
              (list (cons reference (mapcar #'number-to-string (alist-get 'initial_view scene)))
                    (list (alist-get 'id (car (gnosis-model--targets scene)))))))))
    (gnosis-model--author-check context)
    (apply #'gnosis-model-resolve fields)
    (let ((result (if advanced (apply #'gnosis-model--read-numeric fields)
                    (gnosis-model--read-visual (car fields) (cadr fields)
                                              (append context (list :initial (and keep fields)))))))
      (gnosis-model--author-check context)
      (apply #'gnosis-model-resolve result)
      result)))

(defun gnosis-model--author-label (id)
  "Return the label of target ID in the current authoring session, or nil."
  (alist-get 'label (seq-find (lambda (o) (equal id (alist-get 'id o)))
                              (plist-get gnosis-model--author-context :objects))))

(defun gnosis-model--author-header ()
  "Return selected object label and live renderer status for authoring."
  (let* ((context gnosis-model--author-context)
         (label (gnosis-model--author-label (plist-get context :target)))
         (relation (plist-get context :relation))
         (saved (plist-get context :saved)))
    (concat " " (or label "Click target; drag to frame")
            " | " (if (gnosis-model-backend-get :live) (gnosis-model-backend-get :status)
                    (concat "Unavailable: " (gnosis-model-backend-get :status)))
            (cond (relation
                   (format " | Cue %s → %s | r toggles answer, R saves | %d saved"
                           (gnosis-model--author-label (car relation))
                           (if (cdr relation)
                               (mapconcat #'gnosis-model--author-label (cdr relation) ", ")
                             "click an answer, press r")
                           (car saved)))
                  (saved (format " | F find card, N name card, r relation, p point, g region | %d saved | q done"
                                 (car saved)))
                  (t " | p point, g region, a triangle, t target, e edit, m move, d delete | RET accept, q cancel, ? help")))))

(defun gnosis-model--author-selection (selection)
  "Retain surface SELECTION as a tentative authoring target, never accept it."
  (when (and gnosis-model--author-context
             (eq (plist-get selection :owner) (gnosis-model-backend-get :owner))
             (equal (plist-get selection :frame) (gnosis-model-backend-get :frame)))
    (unless (gnosis-model-backend-get :question-target)
      (setf (plist-get gnosis-model--author-context :target)
            (or (alist-get 'id
                           (seq-find (lambda (target)
                                       (and (equal (alist-get 'kind target) "object")
                                            (equal (alist-get 'mesh target) (plist-get selection :id))))
                                     (plist-get gnosis-model--author-context :objects)))
                (plist-get selection :id))))
    (force-mode-line-update)))

(defun gnosis-model--author-retire ()
  "Retire the current viewer claim before a buffer ownership change."
  (when gnosis-model--author-context
    (setf (plist-get gnosis-model--author-context :retired) t)
    (remove-hook 'change-major-mode-hook #'gnosis-model--author-retire t)
    (remove-hook 'after-set-visited-file-name-hook #'gnosis-model--author-retire t)
    (remove-hook 'kill-buffer-hook #'gnosis-model-author-cancel t)
    (gnosis-model-backend-detach (plist-get gnosis-model--author-context :process)
                                 (gnosis-model-backend-get :attachment))
    (gnosis-model-author-cancel)))

(defun gnosis-model-author-cancel ()
  "Cancel visual authoring without changing the original draft."
  (interactive)
  (when gnosis-model--author-context
    (setf (plist-get gnosis-model--author-context :cancelled) t)
    (when (= (recursion-depth) (1+ (plist-get gnosis-model--author-context :depth)))
      (abort-recursive-edit))))

(defun gnosis-model--author-fields ()
  "Return (HYPOTHESIS ANSWER) for the selected target and current camera.
Publish a changed private scene first and continue from its revision."
  (let* ((context gnosis-model--author-context)
         (target (plist-get context :target)))
    (unless (and context (not (plist-get context :result))
                 (not (plist-get context :cancelled))
                 (= (recursion-depth) (1+ (plist-get context :depth))))
      (user-error "No active visual model authoring"))
    (gnosis-model--author-check context)
    (unless (and target
                 (or (equal target (gnosis-model-backend-get :selected-id))
                     (equal target (alist-get 'id (gnosis-model-backend-get :question-target)))
                     (let ((item (seq-find (lambda (item) (equal target (alist-get 'id item)))
                                           (plist-get context :objects))))
                       (and (equal (alist-get 'kind item) "object")
                            (equal (alist-get 'mesh item) (gnosis-model-backend-get :selected-id)))))
                 (gnosis-model-backend-get :live)
                 (eq (plist-get context :process) (gnosis-model-backend-get :owner))
                 (gnosis-model-backend-get :ready))
      (user-error "Select a target and wait for the view to be ready"))
    (let* ((view (gnosis-model--view
                  (gnosis-model-camera-unwrap (gnosis-model-backend-get :view)
                                              (plist-get context :view))))
           (reference (if (plist-get context :changed)
                          (gnosis-model--publish-scene (plist-get context :scene)
                                                       (plist-get context :reference))
                        (plist-get context :reference)))
           (fields (list (cons reference (mapcar #'number-to-string view))
                         (list target))))
      (setf (plist-get context :reference) reference
            (plist-get context :changed) nil)
      fields)))

(defun gnosis-model-author-accept ()
  "Accept the selected target and current ready camera into authoring fields."
  (interactive)
  (let ((fields (gnosis-model--author-fields)))
    (apply #'gnosis-model-resolve fields)
    (setf (plist-get gnosis-model--author-context :result) fields)
    (exit-recursive-edit)))

(defun gnosis-model--canvas-size ()
  "Return a canvas size fitting the selected window, reserving header space."
  (let ((size (min 768 (window-body-width nil t)
                   (- (window-body-height nil t) (* 2 (frame-char-height))))))
    (when (< size 128) (user-error "Enlarge the window to display a model canvas"))
    size))

(defun gnosis-model--read-visual (hypothesis answer owner)
  "Read target and camera visually from HYPOTHESIS, ANSWER and OWNER context."
  (let* ((resolved (gnosis-model-resolve hypothesis answer))
         (initial (plist-get owner :initial))
         ;; One validation pass yields both the manifest and its geometry.
         (validated (gnosis-model--scene (car resolved) nil nil t))
         (scene (car validated))
         (directory (file-name-directory (car resolved)))
         (context (append owner
                          (list :reference (car hypothesis) :view (cadr resolved) :depth (recursion-depth)
                                :objects (gnosis-model--targets scene)
                                :scene scene :directory directory :serial (or (alist-get 'target_serial scene) 0)
                                :geometry (cadr validated)
                                :used-ids (mapcar (lambda (o) (alist-get 'id o)) (gnosis-model--targets scene))
                                :target (and initial (car answer)) :process nil
                                :result nil :cancelled nil :retired nil)))
         viewer)
    (save-window-excursion
      (unwind-protect
          (progn
            (let ((display-buffer-overriding-action '(display-buffer-same-window)))
              (setq viewer (gnosis-model-open (car resolved) (cadr resolved)
                                               (gnosis-model--canvas-size))))
            (with-current-buffer viewer
              (setq-local gnosis-model--author-context context)
              (setf (plist-get context :process) (gnosis-model-backend-get :owner))
              (use-local-map (copy-keymap (current-local-map)))
              (local-set-key (kbd "RET") #'gnosis-model-author-accept)
              (local-set-key (kbd "p") #'gnosis-model-author-point)
              (local-set-key (kbd "m") #'gnosis-model-author-move)
              (local-set-key (kbd "g") #'gnosis-model-author-region)
              (local-set-key (kbd "a") #'gnosis-model-author-region-toggle)
              (local-set-key (kbd "t") #'gnosis-model-author-target)
              (local-set-key (kbd "e") #'gnosis-model-author-edit)
              (local-set-key (kbd "d") #'gnosis-model-author-remove)
              (local-set-key (kbd "q") #'gnosis-model-author-cancel)
              (local-set-key (kbd "C-g") #'gnosis-model-author-cancel)
              (pcase-dolist (`(,key . ,command) (plist-get owner :keys))
                (local-set-key (kbd key) command))
              (setq-local header-line-format '(:eval (gnosis-model--author-header)))
              (add-hook 'gnosis-model-backend-selection-hook #'gnosis-model--author-selection nil t)
              (add-hook 'kill-buffer-hook #'gnosis-model-author-cancel nil t)
              (add-hook 'change-major-mode-hook #'gnosis-model--author-retire nil t)
              (add-hook 'after-set-visited-file-name-hook #'gnosis-model--author-retire nil t)
              (when initial
                (gnosis-model-backend-highlight (gnosis-model-target scene (car answer))
                                                (car answer)))
              (goto-char (point-min)))
            (recursive-edit)
            (unless (and (not (plist-get context :cancelled)) (plist-get context :result))
              (user-error "Model authoring cancelled"))
            (gnosis-model--author-check context)
            (plist-get context :result))
        (when (and (buffer-live-p viewer)
                   (eq (buffer-local-value 'gnosis-model--author-context viewer) context))
          (with-current-buffer viewer (setq gnosis-model--author-context nil))
          (unless (plist-get context :retired)
            (kill-buffer viewer)))))))

;;;###autoload
(defun gnosis-add-model-thema (&optional type)
  "Import objects and visually choose a target and view for a new thema.
TYPE defaults to \"model\" (Find); \"model-name\" creates a typed Name card.
With a prefix argument, use advanced numeric input instead of the canvas."
  (interactive)
  (when (get-buffer "*Gnosis NEW*") (user-error "Finish the existing draft first"))
  (pcase-let ((`(,hypothesis ,answer) (gnosis-model--read-fields)))
    (let* ((type (or type "model"))
           (fields (gnosis-model-fields "model" hypothesis answer))
           (target (gnosis-model-target (gnosis-model--scene (plist-get fields :scene)) (car answer))))
      (unless (member type '("model" "model-name")) (user-error "Invalid model type"))
      (gnosis-add-thema type nil
                       (mapconcat #'identity (if (equal type "model-name")
                                                (cons (car hypothesis) (cons (car answer) (cdr hypothesis)))
                                              hypothesis) gnosis-export-separator)
                       (if (equal type "model-name") (alist-get 'label target) (car answer))))))

(defun gnosis-model--manifest (resource)
  "Return the unvalidated manifest of managed RESOURCE, or nil.
Only for labels and defaults; opening the scene validates it."
  (ignore-errors
    (let ((file (expand-file-name resource (gnosis-assets-root))))
      (and (string-match-p "\\`[0-9a-f]\\{64\\}/scene\\.json\\'" resource)
           (file-regular-p file) (not (file-symlink-p file))
           (< (file-attribute-size (file-attributes file)) 65536)
           (json-parse-string (with-temp-buffer (insert-file-contents file) (buffer-string))
                              :object-type 'alist :array-type 'list)))))

(defun gnosis-model-managed-scenes ()
  "Return (RESOURCE . MANIFEST) for every managed scene, by directory name.
Manifests are read for labels only; opening a scene validates it."
  (let ((root (gnosis-assets-root)))
    (and (file-directory-p root)
         (delq nil (mapcar (lambda (name)
                             (let ((resource (concat name "/scene.json")))
                               (when-let* ((manifest (gnosis-model--manifest resource)))
                                 (cons resource manifest))))
                           (directory-files root nil "\\`[0-9a-f]\\{64\\}\\'"))))))

(defun gnosis-model--scene-resources ()
  "Return (LABEL . RESOURCE) for each scene used by model themata."
  (let (resources)
    (dolist (hypothesis (gnosis-select 'hypothesis 'themata
                                       '(in type ["model" "model-name"]) t))
      (cl-pushnew (car hypothesis) resources :test #'equal))
    (mapcar (lambda (resource)
              (let ((labels (mapcar (lambda (object) (alist-get 'label object))
                                    (alist-get 'objects (gnosis-model--manifest resource)))))
                (cons (format "%s (%s, %s)" (or (car labels) "Unreadable scene")
                              (if labels (format "%d objects" (length labels)) "?")
                              (substring resource 0 8))
                      resource)))
            resources)))

(defun gnosis-model--author-save (type)
  "Save a TYPE model thema from the selected target and current camera.
Prefill the question or answer from the target label."
  (let* ((context gnosis-model--author-context)
         (fields (gnosis-model--author-fields))
         (hypothesis (car fields))
         (target (car (cadr fields)))
         (label (alist-get 'label (gnosis-model-target (plist-get context :scene) target)))
         (name (equal type "model-name"))
         (text (read-string (if name "Name card answer: " "Find card question: ")
                            (if name label
                              (format "Click the %s." (gnosis-model--label-phrase label))))))
    (gnosis-model--author-check context)
    (when (string-empty-p (string-trim text)) (user-error "Empty card text"))
    (gnosis-add-thema--assert-common text (plist-get context :tags) 0 nil)
    ;; Thema insertion validates the model fields itself.
    (gnosis-add-thema-fields type (if name "Name the highlighted structure." text)
                             (if name (cons (car hypothesis) (cons target (cdr hypothesis)))
                               hypothesis)
                             (list (if name text target))
                             "" (plist-get context :tags) 0 nil)
    (cl-incf (car (plist-get context :saved)))
    (force-mode-line-update)
    (message "Saved %s card for %s" (if name "Name" "Find") label)))

(defun gnosis-model--label-phrase (label)
  "Return LABEL lowercased at its start, for use inside a sentence."
  (if (string-empty-p label) label
    (concat (downcase (substring label 0 1)) (substring label 1))))

(defconst gnosis-model-relations
  '("posterior to" "anterior to" "superior to" "inferior to" "medial to"
    "lateral to" "deep to" "superficial to" "within" "articulating with"
    "draining into" "supplied by")
  "Relations offered when authoring a relational card; free text is allowed.")

(defun gnosis-model-author-relation ()
  "Build a relation card from clicked structures.
The first press makes the selected target the reference, shown as a cue;
each later press adds or removes the selected target as an acceptable
answer.  Pressing it on the reference clears the relation; \`R' saves."
  (interactive)
  (let* ((context gnosis-model--author-context)
         (target (car (cadr (gnosis-model--author-fields))))
         (relation (plist-get context :relation)))
    (setq relation (cond ((null relation) (list target))
                         ((equal target (car relation)) nil)
                         ((member target relation) (remove target relation))
                         (t (append relation (list target)))))
    (setf (plist-get context :relation) relation)
    (gnosis-model-backend-cue
     (and relation (list (gnosis-model-target (plist-get context :scene) (car relation)))))
    (force-mode-line-update)))

(defun gnosis-model-author-save-relation ()
  "Save a relational Find card for the pending relation at the current camera.
Prefill the question from a chosen relation and the reference's label."
  (interactive)
  (let* ((context gnosis-model--author-context)
         (relation (plist-get context :relation)))
    (unless (cdr relation)
      (user-error "Press r on a reference structure, then r on each acceptable answer"))
    (let* ((hypothesis (car (gnosis-model--author-fields)))
           (word (string-trim (completing-read "Relation: " gnosis-model-relations)))
           (text (read-string "Relation card question: "
                              (format "Click %s %s the %s."
                                      (if (cddr relation) "a structure" "the structure")
                                      (if (string-empty-p word) "related to" word)
                                      (gnosis-model--label-phrase
                                       (gnosis-model--author-label (car relation)))))))
      (gnosis-model--author-check context)
      (unless (equal relation (plist-get context :relation))
        (user-error "Relation changed while saving"))
      (when (string-empty-p (string-trim text)) (user-error "Empty card text"))
      (gnosis-add-thema--assert-common text (plist-get context :tags) 0 nil)
      ;; Thema insertion validates cues and answers itself.
      (gnosis-add-thema-fields "model" text (append hypothesis (list (car relation)))
                               (cdr relation) "" (plist-get context :tags) 0 nil)
      (cl-incf (car (plist-get context :saved)))
      (setf (plist-get context :relation) nil)
      (gnosis-model-backend-cue nil)
      (force-mode-line-update)
      (message "Saved relation card: %s" text))))

(defun gnosis-model-author-save-find ()
  "Save a Find card for the selected target at the current camera."
  (interactive)
  (gnosis-model--author-save "model"))

(defun gnosis-model-author-save-name ()
  "Save a Name card for the selected target at the current camera."
  (interactive)
  (gnosis-model--author-save "model-name"))

;;;###autoload
(defun gnosis-model-author-scene (resource &optional tags)
  "Author Find and Name cards from scene RESOURCE by clicking structures.
Orbit, click a structure, then press F for a Find card or N for a Name
card at the current camera; p and g first make a point or region target.
For a relational card press r on a reference structure, then r on each
acceptable answer, and R to save it with an editable question.
Every card gets TAGS.  Interactively, choose a scene used by existing
model themata, or import a new one; q finishes."
  (interactive
   (let* ((choices (append (gnosis-model--scene-resources)
                           (list (cons "Import a new scene..." nil))))
          (choice (cdr (assoc (completing-read "Scene: " choices nil t) choices))))
     (list (or choice
               (gnosis-model--read-source
                (list :buffer (current-buffer) :mode major-mode
                      :tick (buffer-chars-modified-tick) :database (gnosis--ensure-db))))
           (delete-dups (completing-read-multiple "Tags (separated by ,): "
                                                  (gnosis-get-tags--unique))))))
  (gnosis-tags--check-org tags)
  (let* ((scene (or (gnosis-model--manifest resource)
                    (user-error "Model unavailable: %s" resource)))
         (saved (list 0)))
    (condition-case err
        ;; The reader validates the whole scene before opening it.
        (gnosis-model--read-visual
         (cons resource (mapcar (lambda (n) (format "%s" n)) (alist-get 'initial_view scene)))
         (list (alist-get 'id (car (alist-get 'objects scene))))
         (list :buffer (current-buffer) :mode major-mode
               :tick (buffer-chars-modified-tick) :database (gnosis--ensure-db)
               :tags tags :saved saved :relation nil
               :keys '(("F" . gnosis-model-author-save-find)
                       ("N" . gnosis-model-author-save-name)
                       ("r" . gnosis-model-author-relation)
                       ("R" . gnosis-model-author-save-relation)
                       ("RET" . ignore))))
      ;; Finishing a session that saved cards is not a cancellation.
      (user-error (when (zerop (car saved)) (signal (car err) (cdr err)))))
    (message "Saved %d model %s" (car saved) (if (= (car saved) 1) "thema" "themata"))
    (car saved)))

(defun gnosis-model-attach ()
  "Attach a scene to the single model thema in the current authoring buffer.
Preserve all draft text on prompt cancellation or an outdated draft.
Offer to reframe the current scene without importing it again.
With a prefix argument, use advanced numeric input instead of the canvas."
  (interactive nil gnosis-edit-mode)
  (let* ((owner (current-buffer))
         (mode major-mode)
         (tick (buffer-chars-modified-tick))
         (themata (save-restriction (widen) (gnosis-export-parse-themata)))
         (thema (car themata)))
    (unless (and (= (length themata) 1) (member (downcase (nth 1 thema)) '("model" "model-name")))
      (user-error "Attach a scene in a single model thema draft"))
    (pcase-let ((`(,hypothesis ,answer)
                 (gnosis-model--read-fields
                  (if (equal (downcase (nth 1 thema)) "model-name")
                      (let ((hypothesis (nth 3 thema)) (answer (nth 4 thema)))
                        (unless (and (= (length hypothesis) 5) (seq-every-p #'stringp hypothesis)
                                     (= (length answer) 1) (stringp (car answer))
                                     (not (string-empty-p (string-trim (car answer)))))
                          (user-error "Model Name needs resource, target, camera and an authored answer"))
                        (list (cons (car hypothesis) (cddr hypothesis))
                              (list (cadr hypothesis))))
                    (when (and (>= (length (nth 3 thema)) 4) (nth 4 thema))
                      (list (seq-take (nth 3 thema) 4) (list (car (nth 4 thema)))))))))
      ;; Reframing keeps a relational Find's cues and other acceptable targets.
      (unless (equal (downcase (nth 1 thema)) "model-name")
        (setq hypothesis (append hypothesis (nthcdr 4 (nth 3 thema)))
              answer (cons (car answer) (remove (car answer) (cdr (nth 4 thema))))))
      (unless (and (buffer-live-p owner)
                   (with-current-buffer owner
                     (and (eq mode major-mode)
                          (= tick (buffer-chars-modified-tick)))))
        (user-error "Model draft changed during attachment"))
      (with-current-buffer owner
        (atomic-change-group
          (let ((inhibit-read-only t))
            (erase-buffer)
            (gnosis-export--insert-thema
             (nth 0 thema) (downcase (nth 1 thema)) (nth 2 thema)
             (mapconcat #'identity (if (equal (downcase (nth 1 thema)) "model-name")
                                      (cons (car hypothesis) (cons (car answer) (cdr hypothesis)))
                                    hypothesis) gnosis-export-separator)
             (if (equal (downcase (nth 1 thema)) "model-name") (car (nth 4 thema))
               (mapconcat #'identity answer gnosis-export-separator))
             (nth 5 thema) (nth 6 thema) nil (nth 8 thema))
            (goto-char (point-min))))))))

(defun gnosis-model--renderer-directory ()
  "The configured backend directory, or nil while the 3D display is off."
  (and gnosis-model-renderer-directory
       (expand-file-name gnosis-model-renderer-directory)))

(defun gnosis-model-open (path view &optional size inline question-target verified)
  "Open validated scene PATH at VIEW through the selected model backend.
SIZE defaults to 512 pixels.  INLINE attaches at point in the current buffer.
QUESTION-TARGET names a label-free display highlight, not a grading rule.
VERIFIED retains prepared topology; recheck bytes without reparsing geometry."
  (when verified (gnosis-model--verified-check verified path))
  (let ((target (when question-target
                  (gnosis-model-target (if verified (plist-get verified :manifest)
                                        (gnosis-model--scene path)) question-target))))
    (gnosis-model-backend-open path view size inline target)))

(defun gnosis-model--geometry (file)
  "Return deterministic original-coordinate triangle vector from OBJ FILE."
  (when (> (file-attribute-size (file-attributes file)) 100000000)
    (user-error "OBJ exceeds 100 MB"))
  (with-temp-buffer
    (insert-file-contents file)
    (let (vertices faces (count 0))
      (dolist (line (split-string (buffer-string) "\n" t))
        (let ((parts (split-string (car (split-string line "#")) "[ \t\r]+" t)))
          (pcase (car parts)
            ("v"
             (unless (>= (length parts) 4) (user-error "OBJ vertex requires three coordinates"))
             (push (mapcar #'gnosis-model--number (seq-take (cdr parts) 3)) vertices)
             (cl-incf count))
            ("f"
             (unless (>= (length parts) 4) (user-error "OBJ face requires three vertices"))
             (let ((indices
                    (mapcar (lambda (part)
                              (let ((text (car (split-string part "/"))))
                                (unless (string-match-p "\\`-?[0-9]+\\'" text)
                                  (user-error "Invalid OBJ index"))
                                (let* ((n (string-to-number text))
                                       (i (if (> n 0) (1- n) (+ count n))))
                                  (unless (and (/= n 0) (<= 0 i) (< i count))
                                    (user-error "OBJ index outside vertices"))
                                  i))) (cdr parts))))
               (cl-loop for rest on (cdr indices) while (cdr rest)
                        do (push (list (car indices) (car rest) (cadr rest)) faces)))))))
      (unless (and faces (<= (length faces) 2000000))
        (user-error "OBJ requires 1..2000000 triangles"))
      (let ((points (vconcat (nreverse vertices))))
        (when (consp gnosis-model--vertex-sets)
          (push points (car gnosis-model--vertex-sets)))
        (vconcat (mapcar (lambda (face)
                           (let ((triangle (mapcar (lambda (i) (aref points i)) face)))
                             (gnosis-model--triangle-frame triangle)
                             triangle))
                         (nreverse faces)))))))

(defun gnosis-model--number (text)
  "Parse finite numeric TEXT without accepting trailing junk."
  (unless (and (stringp text)
               (string-match-p "\\`[-+]?\\(?:[0-9]+\\(?:\\.[0-9]*\\)?\\|\\.[0-9]+\\)\\(?:[eE][-+]?[0-9]+\\)?\\'" text))
    (user-error "Invalid model number"))
  (let ((n (string-to-number text)))
    (unless (<= (abs n) 1000000) (user-error "Model number out of bounds")) n))

(defun gnosis-model--targets (scene)
  "Return targets from SCENE, projecting legacy objects without mutation."
  (if (equal (alist-get 'version scene) 2) (alist-get 'targets scene)
    (mapcar (lambda (o) `((id . ,(alist-get 'id o)) (label . ,(alist-get 'label o))
                         (mesh . ,(alist-get 'id o)) (kind . "object")))
            (alist-get 'objects scene))))

(defun gnosis-model-target (scene id)
  "Return target ID from validated SCENE or signal an error."
  (or (seq-find (lambda (target) (equal id (alist-get 'id target)))
                (gnosis-model--targets scene))
      (user-error "Model target is absent from its pinned scene")))

(defun gnosis-model--validate-targets (scene directory)
  "Validate SCENE topology and target geometry in DIRECTORY."
  (unless (or (not (assq 'version scene)) (equal (alist-get 'version scene) 2))
    (user-error "Unknown model scene version"))
  (when (assq 'target_serial scene)
    (unless (and (integerp (alist-get 'target_serial scene))
                 (<= 0 (alist-get 'target_serial scene) 1000000000))
      (user-error "Invalid target serial")))
  (when (and (not (assq 'version scene)) (assq 'targets scene))
    (user-error "Targets require scene version 2"))
  (let* ((geometry (mapcar (lambda (o)
                             (cons (alist-get 'id o)
                                   (gnosis-model--geometry
                                    (gnosis-assets-file directory (alist-get 'path o)))))
                           (alist-get 'objects scene)))
         (targets (gnosis-model--targets scene)))
    (when (> (apply #'+ (mapcar (lambda (entry) (length (cdr entry))) geometry)) 2000000)
      (user-error "Scene exceeds two million triangles"))
    (unless (and (proper-list-p targets) (<= 1 (length targets) 4096)
                 (= (length targets) (length (delete-dups
                                             (mapcar (lambda (o) (alist-get 'id o)) targets)))))
      (user-error "Scene requires unique targets"))
    (dolist (target targets)
      (let* ((mesh (cdr (assoc (alist-get 'mesh target) geometry)))
             (kind (alist-get 'kind target))
             (face (alist-get 'face target))
             (bary (alist-get 'barycentric target))
             (radius (alist-get 'tolerance target))
             (faces (alist-get 'faces target)))
        (unless (and mesh (stringp (alist-get 'id target))
                     (string-match-p "\\`[[:alnum:]_-]+\\'" (alist-get 'id target))
                     (stringp (alist-get 'label target))
                     (not (string-empty-p (string-trim (alist-get 'label target)))))
          (user-error "Invalid model target identity"))
        (pcase kind
          ("object" (when (or face bary radius faces) (user-error "Object target has geometry")))
          ("point"
           (unless (and (integerp face) (<= 0 face) (< face (length mesh))
                        (proper-list-p bary) (= (length bary) 3)
                        (seq-every-p (lambda (n) (and (numberp n) (<= 0 n 1))) bary)
                        (< (abs (- (apply #'+ bary) 1)) 0.000001)
                        (numberp radius) (< 0 radius) (<= radius 1000000) (not faces))
             (user-error "Invalid surface point or tolerance")))
          ("region"
           (unless (and (proper-list-p faces) faces (not face) (not bary) (not radius)
                        (= (length faces) (length (delete-dups (copy-sequence faces))))
                        (seq-every-p (lambda (n) (and (integerp n) (<= 0 n) (< n (length mesh)))) faces))
             (user-error "Invalid surface region")))
          (_ (user-error "Unknown model target kind")))))
    geometry))

(defun gnosis-model--check-relation (scene cues answers)
  "Refuse relational CUES and ANSWERS that SCENE cannot show fairly.
Every ID must name a target; neither list may repeat one.  Cues may
include an answer, but cueing exactly the answers would reveal them."
  (dolist (id (append cues answers)) (gnosis-model-target scene id))
  (unless (and (equal cues (seq-uniq cues)) (equal answers (seq-uniq answers)))
    (user-error "Model cue and answer targets must be unique"))
  (when (and cues (null (seq-difference cues answers)) (null (seq-difference answers cues)))
    (user-error "Model cues must not be exactly the answers")))

(defun gnosis-model-fields (type hypothesis answer &optional root camera)
  "Validate TYPE, HYPOTHESIS and ANSWER and return resolved model fields.
A Find HYPOTHESIS may follow its camera with cue target IDs, and its
ANSWER may list further acceptable target IDs; :target is the first.
ROOT defaults to the connected database asset root.  Retain verified scene
data in the result; only byte integrity needs rechecking within its owner.
CAMERA requests varied review cameras; see `gnosis-model-resolve'."
  (let ((find (equal type "model")))
    (unless (and (member type '("model" "model-name"))
                 (proper-list-p hypothesis)
                 (if find (>= (length hypothesis) 4) (= (length hypothesis) 5))
                 (seq-every-p #'stringp hypothesis)
                 (proper-list-p answer) answer (or find (= (length answer) 1))
                 (seq-every-p (lambda (text) (and (stringp text)
                                                 (not (string-empty-p (string-trim text)))))
                              answer))
      (user-error "Invalid model fields")))
  (let* ((name (equal type "model-name"))
         (target (if name (nth 1 hypothesis) (car answer)))
         (legacy (if name (cons (car hypothesis) (cddr hypothesis)) (seq-take hypothesis 4)))
         (cues (and (not name) (nthcdr 4 hypothesis)))
         (answers (if name (list target) answer))
         ;; Varied cameras are qualified for the answer only and could
         ;; hide a cue, so relational cards keep their authored camera.
         (resolved (gnosis-model-resolve legacy (list target) root (and (null cues) camera))))
    (gnosis-model--check-relation (plist-get (nth 2 resolved) :manifest) cues answers)
    (list :resource (car hypothesis) :scene (car resolved) :view (cadr resolved)
          :target target :response (if name 'name 'find) :answer (and name (car answer))
          :verified (nth 2 resolved) :cues cues :answers answers)))

(defun gnosis-model-correct-p (fields id)
  "Return non-nil if picked target ID is acceptable for FIELDS."
  (and id (member id (or (plist-get fields :answers) (list (plist-get fields :target)))) t))

(defun gnosis-model--field-targets (fields key)
  "Return target alists of FIELDS for the ID list under KEY."
  (let ((scene (plist-get (plist-get fields :verified) :manifest)))
    (mapcar (lambda (id) (gnosis-model-target scene id)) (plist-get fields key))))

(defun gnosis-model-show-cues (fields)
  "Paint every cue target of FIELDS on the current attachment, or none.
Return non-nil if the backend painted the whole cue set in cue layers.
Backends without cue layers paint nothing: their single question
highlight could show only part of the set, and a contrast card cueing
its answer and a distractor would then highlight the answer alone.  The
caller names the cues in text instead; see `gnosis-model-cue-text'."
  (when-let* ((cues (gnosis-model--field-targets fields :cues)))
    (and (gnosis-model-backend-cue cues) t)))

(defun gnosis-model-cue-text (fields)
  "Return a line naming every cue target of FIELDS in order, or nil."
  (when-let* ((cues (gnosis-model--field-targets fields :cues)))
    (concat (if (cdr cues) "Cues: " "Cue: ")
            (mapconcat (lambda (target) (alist-get 'label target)) cues ", "))))

(defun gnosis-model-all-objects-p (fields)
  "Return non-nil if every acceptable target of FIELDS is a whole object.
Presentation of picks and wrong answers follows the whole answer set,
never its order."
  (seq-every-p (lambda (target) (member (alist-get 'kind target) '("object" object)))
               (or (gnosis-model--field-targets fields :answers)
                   (list (gnosis-model-target (plist-get (plist-get fields :verified) :manifest)
                                              (plist-get fields :target))))))

(defun gnosis-model-reveal-extras (fields &optional shown)
  "Return reveal plist entries for the cues and alternates of FIELDS.
SHOWN is the acceptable target ID revealed as the answer; the other
acceptable targets become alternates."
  (when (or (plist-get fields :cues) (cdr (plist-get fields :answers)))
    (list :cues (gnosis-model--field-targets fields :cues)
          :alternates (seq-remove (lambda (target)
                                    (equal (alist-get 'id target)
                                           (or shown (plist-get fields :target))))
                                  (gnosis-model--field-targets fields :answers)))))

(defun gnosis-model-answer-text (fields)
  "Return the answer shown in feedback for FIELDS.
A Find lists every acceptable target's label."
  (or (plist-get fields :answer)
      (let ((labels (mapcar (lambda (target) (alist-get 'label target))
                            (or (gnosis-model--field-targets fields :answers)
                                (list (gnosis-model-target
                                       (plist-get (plist-get fields :verified) :manifest)
                                       (plist-get fields :target)))))))
        (if (cdr labels)
            (concat (string-join (butlast labels) ", ") " or " (car (last labels)))
          (car labels)))))

(defun gnosis-model--verified-check (verified path)
  "Recheck VERIFIED asset bytes and require their exact scene PATH."
  (unless (and verified
               (equal path (expand-file-name
                            (concat (plist-get verified :revision) "/scene.json")
                            (plist-get verified :root))))
    (user-error "Model scene does not belong to its preparation"))
  (gnosis-assets-validate (plist-get verified :root) (plist-get verified :revision)
                          (plist-get verified :names)))

(defun gnosis-model-check-fields (fields)
  "Recheck retained FIELDS against the current asset root and literal bytes.
Return FIELDS without reparsing verified topology.  Callers must separately
validate the encounter that owns these retained values."
  (let ((verified (plist-get fields :verified)))
    (unless (and (equal (plist-get verified :root) (gnosis-assets-root))
                 (equal (plist-get fields :resource)
                        (concat (plist-get verified :revision) "/scene.json")))
      (user-error "Model resource belongs to another asset root"))
    (gnosis-model--verified-check verified (plist-get fields :scene)))
  fields)

(defun gnosis-model-cancel-preparation (job)
  "Retire JOB's child, deferred delivery and private buffers idempotently."
  (when job
    (setf (plist-get job :cancelled) t)
    (when (timerp (plist-get job :timer)) (cancel-timer (plist-get job :timer)))
    (when-let* ((process (plist-get job :process)))
      (set-process-sentinel process #'ignore)
      (when (process-live-p process) (delete-process process)))
    (dolist (key '(:output :errors))
      (when (buffer-live-p (plist-get job key)) (kill-buffer (plist-get job key))))))

(defun gnosis-model--deliver-preparation (job callback)
  "Deliver JOB to CALLBACK outside the process sentinel.
CALLBACK receives (FIELDS ERROR); exactly one is non-nil.  It runs in timer
context and must validate its own buffer and encounter before applying data."
  (unless (plist-get job :cancelled)
    (let ((result
           (condition-case err
               (progn
                 (unless (zerop (process-exit-status (plist-get job :process)))
                   (error "Model preparation process failed"))
                 (with-current-buffer (plist-get job :output)
                   (goto-char (point-min))
                   (let ((value (read (current-buffer))))
                     (skip-chars-forward " \t\r\n")
                     (unless (eobp) (error "Invalid model preparation response"))
                     value)))
             (error (list :error (error-message-string err))))))
      (gnosis-model-cancel-preparation job)
      (funcall callback (plist-get result :fields)
               (or (plist-get result :error)
                   (unless (plist-get result :fields) "Empty model preparation response"))))))

(defun gnosis-model-prepare (type hypothesis answer callback)
  "Prepare TYPE, HYPOTHESIS and ANSWER in an owned child Emacs process.
Return a cancellable job immediately.  CALLBACK receives (FIELDS ERROR)
from a deferred timer, never the process sentinel.  No database is opened
in the child; only the explicitly captured asset root is read.  The child
also prepares varied review cameras for `gnosis-model-review-view'."
  (let* ((root (gnosis-assets-root))
         (job (list :process nil :timer nil :cancelled nil
                    :output (generate-new-buffer " *Gnosis model preparation*")
                    :errors (generate-new-buffer " *Gnosis model preparation errors*")))
         (expression
          `(progn
             (setq load-prefer-newer t)
             (require 'gnosis-model)
             (princ (gnosis-sqlite--serialize
                     (condition-case err
                         (list :fields (gnosis-model-fields ,type ',hypothesis ',answer ,root
                                                        ',gnosis-model-camera-variation))
                       (error (list :error (error-message-string err))))))))
         started)
    (unwind-protect
        (progn
          (setf (plist-get job :process)
                (make-process
                 :name "gnosis-model-prepare" :noquery t :connection-type 'pipe
                 :buffer (plist-get job :output) :stderr (plist-get job :errors)
                 :coding 'utf-8-unix
                 :command (list (expand-file-name invocation-name invocation-directory)
                                "-Q" "--batch" "-L" gnosis-model--directory
                                "--eval" (gnosis-sqlite--serialize expression))
                 :sentinel
                 (lambda (process _event)
                   (when (and (memq (process-status process) '(exit signal))
                              (not (plist-get job :cancelled)))
                     (setf (plist-get job :timer)
                           (run-at-time 0 nil #'gnosis-model--deliver-preparation job callback))))))
          (setq started t)
          job)
      (unless started (gnosis-model-cancel-preparation job)))))

(defun gnosis-model--point (target geometry)
  "Return original coordinate of point TARGET in GEOMETRY."
  (apply #'cl-mapcar (lambda (&rest coordinates)
                      (apply #'+ (cl-mapcar #'* coordinates (alist-get 'barycentric target))))
         (aref geometry (alist-get 'face target))))

(defun gnosis-model--distance (a b)
  "Return Euclidean distance between original-coordinate points A and B.
Scale differences before squaring so tiny model distances do not underflow."
  (let* ((differences (cl-mapcar (lambda (x y) (abs (- x y))) a b))
         (scale (apply #'max differences)))
    (if (zerop scale) 0.0
      (* scale (sqrt (apply #'+ (mapcar (lambda (d) (expt (/ d (float scale)) 2))
                                       differences)))))))

(defun gnosis-model--scene-radius (vertex-sets)
  "Return the framing radius of VERTEX-SETS, a list of (X Y Z) sequences.
Like the renderers, measure the farthest vertex, referenced or not, from
the centre of the bounding box."
  (let ((low (make-vector 3 1.0e+INF)) (high (make-vector 3 -1.0e+INF)))
    (dolist (vertices vertex-sets)
      (seq-doseq (vertex vertices)
        (cl-loop for x in vertex for axis from 0
                 do (aset low axis (min x (aref low axis)))
                 (aset high axis (max x (aref high axis))))))
    (let* ((centre (cl-mapcar (lambda (a b) (/ (+ a b) 2.0)) low high))
           (cx (nth 0 centre)) (cy (nth 1 centre)) (cz (nth 2 centre))
           ;; Scale by the half extent so tiny coordinates cannot underflow.
           (scale (/ (max (- (aref high 0) (aref low 0)) (- (aref high 1) (aref low 1))
                          (- (aref high 2) (aref low 2)))
                     2.0))
           (farthest 0.0))
      (if (not (< 0 scale 1.0e+INF)) 0.0
        (dolist (vertices vertex-sets)
          (seq-doseq (vertex vertices)
            (let ((x (/ (- (nth 0 vertex) cx) scale))
                  (y (/ (- (nth 1 vertex) cy) scale))
                  (z (/ (- (nth 2 vertex) cz) scale)))
              (setq farthest (max farthest (+ (* x x) (* y y) (* z z)))))))
        (* scale (sqrt farthest))))))

(defun gnosis-model--hits (scene geometry hit &optional points)
  "Return (DISTANCE . TARGET) for every target of SCENE containing HIT.
An object contains every hit on its mesh, a region the hits on its
faces, and a point those within its tolerance.  GEOMETRY is the
validated triangle alist; POINTS optionally supplies verified point
target coordinates instead."
  (let ((mesh (plist-get hit :mesh))
        (point (plist-get hit :point))
        (face (plist-get hit :face)))
    (cl-loop for target in (gnosis-model--targets scene)
             for distance = (and (equal mesh (alist-get 'mesh target))
                                 (pcase (alist-get 'kind target)
                                   ("object" 0)
                                   ("region" (and (member face (alist-get 'faces target)) 0))
                                   ("point"
                                    (when point
                                      (let ((d (gnosis-model--distance
                                                point
                                                (if points (cdr (assoc (alist-get 'id target) points))
                                                  (gnosis-model--point target (cdr (assoc mesh geometry)))))))
                                        (and (<= d (alist-get 'tolerance target)) d))))))
             when distance collect (cons distance target))))

(defun gnosis-model--nearest (hits)
  "Return the ID of the nearest of HITS, ties by ID, or nil."
  (cdar (sort (mapcar (lambda (hit) (cons (car hit) (alist-get 'id (cdr hit)))) hits)
              (lambda (a b) (if (= (car a) (car b)) (string< (cdr a) (cdr b))
                              (< (car a) (car b)))))))

(defun gnosis-model--candidate (scene geometry expected hit &optional points)
  "Resolve HIT against SCENE GEOMETRY using EXPECTED target kind, not grading.
POINTS optionally supplies verified target coordinates instead of GEOMETRY."
  (let ((kind (alist-get 'kind (gnosis-model-target scene expected))))
    (gnosis-model--nearest
     (seq-filter (lambda (hit) (equal kind (alist-get 'kind (cdr hit))))
                 (gnosis-model--hits scene geometry hit points)))))

(defun gnosis-model-hit-targets (scene hit points)
  "Return the targets of SCENE containing surface HIT, most specific first:
points, nearest first, then regions, then whole objects.  POINTS are
verified point coordinates (`gnosis-model-resolve').  An observation of
where a pick landed, never an answer."
  (let ((rank (lambda (hit) (list (pcase (alist-get 'kind (cdr hit)) ("point" 0) ("region" 1) (_ 2))
                                  (car hit)))))
    (mapcar #'cdr (sort (gnosis-model--hits scene nil hit points)
                        (lambda (a b)
                          (let ((a (funcall rank a)) (b (funcall rank b)))
                            (or (< (car a) (car b))
                                (and (= (car a) (car b)) (< (cadr a) (cadr b))))))))))

(defun gnosis-model-selection (fields)
  "Return current owned surface selection resolved against FIELDS.
Retain :mesh, :face, :point, :frame and :owner for every valid surface hit.
Its :id is nil outside eligible targets, so callers may grade a wrong hit.
Return nil for background; reject stale or in-flight renderer state."
  (let* ((hit (gnosis-model-backend-get :selection))
         (verified (plist-get fields :verified))
         (scene (plist-get verified :manifest))
         (count (cdr (assoc (plist-get hit :mesh) (plist-get verified :counts)))))
    (unless (gnosis-model-backend-selection-current-p hit t)
      (user-error "No current owned model selection"))
    (gnosis-model-check-fields fields)
    (when (plist-get hit :mesh)
      (unless (and count (integerp (plist-get hit :face))
                   (<= 0 (plist-get hit :face)) (< (plist-get hit :face) count)
                   (proper-list-p (plist-get hit :point)) (= (length (plist-get hit :point)) 3)
                   (seq-every-p (lambda (n) (and (numberp n) (<= (abs n) 1000000)))
                                (plist-get hit :point)))
        (user-error "Invalid renderer surface hit"))
      (setf (plist-get hit :id)
            ;; Answers may differ in kind; prefer a hit any of them accepts.
            (let ((ids (mapcar (lambda (id)
                                 (gnosis-model--candidate scene nil id hit
                                                          (plist-get verified :points)))
                               (or (plist-get fields :answers)
                                   (list (plist-get fields :target))))))
              ;; A wrong hit names the same target whatever the answer order.
              (or (seq-find (lambda (id) (gnosis-model-correct-p fields id)) ids)
                  (seq-find #'identity ids))))
      hit)))

(defun gnosis-model--author-hit ()
  "Return current owned surface hit for the authoring session."
  (gnosis-model--author-check gnosis-model--author-context)
  (let ((hit (gnosis-model-backend-get :selection)))
    (unless (and (eq (plist-get gnosis-model--author-context :process)
                     (gnosis-model-backend-get :owner))
                 (gnosis-model-backend-selection-current-p hit t)
                 (plist-get hit :mesh) (integerp (plist-get hit :face)))
      (user-error "Click a surface and wait for the renderer"))
    (copy-tree hit)))

(defun gnosis-model--product-error (a b product)
  "Return the rounding residual of finite A times B stored in PRODUCT.
Split normalized binary64 significands so even large off-plane coordinates
cannot overflow the splitter.  Underflow still limits representable residuals."
  (let* ((a-parts (frexp a)) (b-parts (frexp b))
         (shift (+ (cdr a-parts) (cdr b-parts)))
         (a (car a-parts)) (b (car b-parts))
         (as (* 134217729.0 a)) (bs (* 134217729.0 b))
         (ah (- as (- as a))) (bh (- bs (- bs b)))
         (al (- a ah)) (bl (- b bh))
         (scaled (ldexp product (- shift))))
    ;; Dekker's product residual; keep the subtractions in this order.
    (ldexp (- (* al bl) (- (- (- scaled (* ah bh)) (* al bh)) (* ah bl)))
           shift)))

(defun gnosis-model--determinant (a b c d)
  "Return A times B minus C times D, compensating cancelled products."
  (let* ((ab (* a b)) (cd (* c d)) (difference (- ab cd)))
    (if (or (zerop ab) (zerop cd)
            (not (< (abs difference) 1.0e+INF))
            (>= (abs difference) (* 0.5 (max (abs ab) (abs cd)))))
        difference
      ;; Close same-sign products subtract exactly.  Recover the residuals
      ;; their individual multiplications rounded away; this is not an area
      ;; epsilon, and an exactly zero determinant remains zero.
      (+ difference (- (gnosis-model--product-error a b ab)
                       (gnosis-model--product-error c d cd))))))

(defun gnosis-model--cross (u v)
  "Return the compensated cross product of three-coordinate vectors U and V."
  (list (gnosis-model--determinant (nth 1 u) (nth 2 v) (nth 2 u) (nth 1 v))
        (gnosis-model--determinant (nth 2 u) (nth 0 v) (nth 0 u) (nth 2 v))
        (gnosis-model--determinant (nth 0 u) (nth 1 v) (nth 1 u) (nth 0 v))))

(defun gnosis-model--triangle-frame (triangle)
  "Return scaled edges, nonzero normal and binary scale exponent of TRIANGLE.
Power-of-two scaling avoids uniform-scale underflow without rounding edges by
an arbitrary divisor.  This is not an exact orientation predicate."
  (let* ((u (cl-mapcar #'- (nth 1 triangle) (car triangle)))
         (v (cl-mapcar #'- (nth 2 triangle) (car triangle)))
         (shift (- (cdr (frexp (apply #'max (mapcar #'abs (append u v)))))))
         (u (mapcar (lambda (x) (ldexp x shift)) u))
         (v (mapcar (lambda (x) (ldexp x shift)) v))
         (normal (gnosis-model--cross u v)))
    (when (seq-every-p #'zerop normal) (user-error "Degenerate surface triangle"))
    (list u v normal shift)))

(defun gnosis-model--barycentric (point triangle)
  "Return clamped barycentric coordinates of POINT projected onto TRIANGLE."
  (pcase-let* ((`(,u ,v ,normal ,shift) (gnosis-model--triangle-frame triangle))
               (r (mapcar (lambda (x) (ldexp x shift))
                          (cl-mapcar #'- point (car triangle))))
               (m (apply #'max (mapcar #'abs normal)))
               (q (mapcar (lambda (x) (/ x m)) normal))
               (dot (lambda (a b) (apply #'+ (cl-mapcar #'* a b))))
               (den (* m (funcall dot q q)))
               (beta (/ (funcall dot (gnosis-model--cross r v) q) den))
               (gamma (/ (funcall dot (gnosis-model--cross u r) q) den))
               (raw (list (- 1 beta gamma) beta gamma)))
    ;; Do not let clamping turn nonrepresentable arithmetic into a target.
    (unless (seq-every-p (lambda (x) (< (abs x) 1.0e+INF)) raw)
      (user-error "Surface coordinates exceed numeric precision"))
    (let* ((values (mapcar (lambda (n) (max 0.0 (min 1.0 n))) raw))
           (sum (apply #'+ values)))
      (mapcar (lambda (n) (/ n sum)) values))))

(defun gnosis-model--author-change (target &optional remove)
  "Replace private TARGET, or REMOVE it, without publishing resources."
  (let* ((context gnosis-model--author-context)
         (scene (copy-tree (plist-get context :scene)))
         (targets (seq-remove (lambda (item) (equal (alist-get 'id item) (alist-get 'id target)))
                              (gnosis-model--targets scene))))
    (setf (alist-get 'version scene) 2
          (alist-get 'target_serial scene) (or (plist-get context :serial) 0)
          (alist-get 'targets scene) (if remove targets (append targets (list target))))
    (gnosis-model--validate-targets scene (plist-get context :directory))
    ;; The reader and viewer share this list.  Adding a missing plist key
    ;; with `plist-put' may replace only the local head, losing the flag.
    (unless (plist-member context :changed)
      (nconc context (list :changed nil)))
    (setf (plist-get context :scene) scene
          (plist-get context :objects) (gnosis-model--targets scene)
          (plist-get context :changed) t
          (plist-get context :target) (unless remove (alist-get 'id target)))
    (gnosis-model-backend-highlight (unless remove target) nil)))

(defun gnosis-model-author-point ()
  "Create a surface point at the last click with explicit world-unit tolerance."
  (interactive)
  (let* ((context gnosis-model--author-context)
         (hit (gnosis-model--author-hit))
         (label (read-string "Landmark label: "))
         (tolerance (read-number "Tolerance (original mesh units): "))
         (geometry (cdr (assoc (plist-get hit :mesh) (plist-get context :geometry))))
         (id (gnosis-model--author-id context)))
    (gnosis-model--author-check context)
    (unless (equal hit (gnosis-model--author-hit)) (user-error "Surface selection changed"))
    (gnosis-model--author-change
     `((id . ,id) (label . ,label) (mesh . ,(plist-get hit :mesh)) (kind . "point")
       (face . ,(plist-get hit :face))
       (barycentric . ,(gnosis-model--barycentric (plist-get hit :point)
                                                (aref geometry (plist-get hit :face))))
       (tolerance . ,tolerance)))))

(defun gnosis-model--author-id (context)
  "Allocate a non-recycled target ID in private CONTEXT."
  (let (id)
    (while (or (not id) (member id (plist-get context :used-ids)))
      (setf (plist-get context :serial) (1+ (or (plist-get context :serial) 0)))
      (setq id (format "landmark-%d" (plist-get context :serial))))
    (push id (plist-get context :used-ids)) id))

(defun gnosis-model-author-move ()
  "Move the selected point to the clicked surface, retaining its ID and tolerance."
  (interactive)
  (let* ((context gnosis-model--author-context)
         (hit (gnosis-model--author-hit))
         (target (copy-tree (gnosis-model-target (plist-get context :scene) (plist-get context :target))))
         (geometry (cdr (assoc (plist-get hit :mesh) (plist-get context :geometry)))))
    (unless (equal (alist-get 'kind target) "point") (user-error "Select a point target first"))
    (setf (alist-get 'mesh target) (plist-get hit :mesh)
          (alist-get 'face target) (plist-get hit :face)
          (alist-get 'barycentric target)
          (gnosis-model--barycentric (plist-get hit :point) (aref geometry (plist-get hit :face))))
    (gnosis-model--author-change target)))

(defun gnosis-model-author-region ()
  "Create a surface region from the clicked triangle."
  (interactive)
  (let* ((context gnosis-model--author-context)
         (hit (gnosis-model--author-hit))
         (label (read-string "Region label: ")))
    (gnosis-model--author-check context)
    (unless (equal hit (gnosis-model--author-hit)) (user-error "Surface selection changed"))
    (gnosis-model--author-change
     `((id . ,(gnosis-model--author-id context)) (label . ,label)
       (mesh . ,(plist-get hit :mesh)) (kind . "region") (faces . ,(list (plist-get hit :face)))))))

(defun gnosis-model-author-region-toggle ()
  "Add or remove the clicked triangle from the selected region."
  (interactive)
  (let* ((context gnosis-model--author-context)
         (hit (gnosis-model--author-hit))
         (target (copy-tree (gnosis-model-target (plist-get context :scene) (plist-get context :target))))
         (face (plist-get hit :face))
         (faces (alist-get 'faces target)))
    (unless (and (equal (alist-get 'kind target) "region")
                 (equal (alist-get 'mesh target) (plist-get hit :mesh)))
      (user-error "Select a region on the clicked mesh first"))
    (when (and (equal faces (list face))) (user-error "Remove the target to delete its last triangle"))
    (setf (alist-get 'faces target) (if (member face faces) (remove face faces) (append faces (list face))))
    (gnosis-model--author-change target)))

(defun gnosis-model-author-target ()
  "Select an existing target without changing its stable ID."
  (interactive)
  (let* ((context gnosis-model--author-context)
         (targets (gnosis-model--targets (plist-get context :scene)))
         (choices (mapcar (lambda (item) (cons (format "%s (%s)" (alist-get 'label item)
                                                       (alist-get 'id item)) (alist-get 'id item))) targets))
         (id (cdr (assoc (completing-read "Target: " choices nil t) choices))))
    (gnosis-model--author-check context)
    (setf (plist-get context :target) id)
    (gnosis-model-backend-highlight (gnosis-model-target (plist-get context :scene) id) nil)))

(defun gnosis-model-author-edit ()
  "Edit selected target label and point tolerance, preserving geometry and ID."
  (interactive)
  (let* ((context gnosis-model--author-context)
         (target (copy-tree (gnosis-model-target (plist-get context :scene) (plist-get context :target))))
         (label (read-string "Target label: " (alist-get 'label target)))
         (tolerance (when (equal (alist-get 'kind target) "point")
                      (read-number "Tolerance (original mesh units): " (alist-get 'tolerance target)))))
    (gnosis-model--author-check context)
    (setf (alist-get 'label target) label)
    (when tolerance (setf (alist-get 'tolerance target) tolerance))
    (gnosis-model--author-change target)))

(defun gnosis-model-author-remove ()
  "Remove selected target from the private draft, retaining other targets."
  (interactive)
  (let* ((context gnosis-model--author-context)
         (target (gnosis-model-target (plist-get context :scene) (plist-get context :target))))
    (when (yes-or-no-p "Remove selected target? ")
      (gnosis-model--author-check context)
      (gnosis-model--author-change target t))))

(defun gnosis-model--publish-scene (scene reference)
  "Publish edited SCENE, retaining the exact geometry of original REFERENCE.
Verify both the original resource and its private copy before replacing the
copied manifest.  Refuse observed drift without publishing a new revision."
  (let* ((root (gnosis-assets-root))
         (revision (car (split-string reference "/")))
         (names (cons "scene.json" (mapcar (lambda (object) (alist-get 'path object))
                                          (alist-get 'objects scene))))
         (directory (gnosis-assets-validate root revision names))
         (stage (make-temp-file "gnosis-model-targets-" t)))
    (unwind-protect
        (progn
          (dolist (name (delete-dups (copy-sequence names)))
            (copy-file (gnosis-assets-file directory name) (expand-file-name name stage)))
          ;; Checking the source alone misses changed bytes copied before restore.
          (unless (equal revision (gnosis-assets-revision stage names))
            (user-error "Model resource changed during copying"))
          (gnosis-assets-validate root revision names)
          (let ((coding-system-for-write 'utf-8-unix)
                (json-encoding-pretty-print nil))
            (with-temp-file (expand-file-name "scene.json" stage) (insert (json-encode scene))))
          (gnosis-model-import (expand-file-name "scene.json" stage)))
      (delete-directory stage t))))

(provide 'gnosis-model)
;;; gnosis-model.el ends here
