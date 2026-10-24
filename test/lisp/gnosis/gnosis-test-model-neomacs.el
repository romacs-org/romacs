;;; gnosis-test-model-neomacs.el --- Native backend tests -*- lexical-binding: t; -*-

;; Copyright (C) 2026 Free Software Foundation, Inc.
;; SPDX-License-Identifier: GPL-3.0-or-later

;;; Commentary:
;; The public primitive stub follows neomacs-model.el's opaque handles,
;; callback signature, request-invalidated readiness and glyph ownership.
;; No GPU or Neomacs installation is required by the ordinary Gnosis suite.

;;; Code:
(require 'ert)
(require 'cl-lib)
(require 'gnosis-neomacs-3d)
(require 'gnosis-model-backend)
(require 'gnosis-model-test-support)

(defvar neomacs-model-view-map)
(defvar gnosis-model-backend--implementation)
(declare-function neomacs-model-view "neomacs-model" (model))
(defvar gnosis-test-neomacs--states)
(defvar gnosis-test-neomacs--created)
(defvar gnosis-test-neomacs--destroyed)
(defvar gnosis-test-neomacs--available)
(defvar gnosis-test-neomacs--insert-error)
(defvar gnosis-test-neomacs--highlights)
(defvar gnosis-test-neomacs--capabilities)
(defvar gnosis-test-neomacs--resizes)

(defun gnosis-test-neomacs--create (&rest data)
  "Create a fresh opaque handle for native creation DATA."
  (let ((model (make-symbol "native-model")))
    (puthash model (list :create data :status '(loading) :current nil :issued 0 :settled nil
                         :view (copy-sequence (plist-get data :view)))
             gnosis-test-neomacs--states)
    (push model gnosis-test-neomacs--created)
    model))

(defun gnosis-test-neomacs--check-spec (spec)
  "Signal like the native primitive for a malformed highlight SPEC.
Mirror `ModelTarget::validated': point weights finite, non-negative and
summing to 1 within 1e-6, tolerance finite and positive; regions nonempty
and distinct."
  (pcase (plist-get spec :kind)
    ('object nil)
    ('point
     (let ((weights (append (plist-get spec :barycentric) nil))
           (tolerance (plist-get spec :tolerance)))
       (unless (and (natnump (plist-get spec :face))
                    (= (length weights) 3)
                    (seq-every-p (lambda (w) (and (numberp w) (<= 0 w) (< w 1.0e+INF)))
                                 weights)
                    (<= (abs (- (apply #'+ weights) 1)) 1e-6))
         (error "Point target needs finite weights >= 0 summing to 1: %S" spec))
       (unless (and (numberp tolerance) (< 0 tolerance 1.0e+INF))
         (error "Point target needs a finite tolerance > 0: %S" spec))))
    ('region
     (let ((faces (plist-get spec :faces)))
       (unless (and faces (seq-every-p #'natnump faces)
                    (= (length faces) (length (delete-dups (copy-sequence faces)))))
         (error "Region target needs distinct faces: %S" spec))))
    (_ (error "Unknown target kind: %S" spec))))

(defun gnosis-test-neomacs--state (model)
  "Return stub state of MODEL, refusing unknown handles."
  (or (gethash model gnosis-test-neomacs--states) (error "Unknown model")))

(defun gnosis-test-neomacs--emit (model event data)
  "Update MODEL state before delivering native EVENT with DATA."
  (let ((state (gnosis-test-neomacs--state model)))
    (pcase event
      ('ready (plist-put state :status '(ready :triangles (2) :meshes ("bone"))))
      ('readiness
       (plist-put state :current (plist-get data :current))
       (plist-put state :settled (plist-get data :req))
       (plist-put state :revision (plist-get data :rev)))
      ('failed (plist-put state :status (cons 'failed data))))
    (funcall (plist-get (plist-get state :create) :callback) model event data)))

(defmacro gnosis-test-neomacs--with-primitive (&rest body)
  "Run BODY with a faithful isolated native API stub and scene fixture."
  (declare (indent 0) (debug t))
  `(cl-progv '(features) (list (cons 'neomacs-model features))
     (let* ((neomacs-model-view-map (let ((map (make-sparse-keymap)))
                                      (define-key map "f" #'ignore)
                                      map))
          (gnosis-test-neomacs--states (make-hash-table :test #'eq))
          (gnosis-test-neomacs--created nil)
          (gnosis-test-neomacs--destroyed nil)
          (gnosis-test-neomacs--highlights nil)
          (gnosis-test-neomacs--capabilities nil)
          (gnosis-test-neomacs--resizes nil)
          (gnosis-test-neomacs--available t)
          (gnosis-test-neomacs--insert-error nil)
          (directory (make-temp-file "gnosis-native-model-" t))
          (path (expand-file-name "scene.json" directory)))
     (unwind-protect
         (progn
           (with-temp-file path
             (insert "{\"objects\":[{\"id\":\"bone\",\"path\":\"bone.obj\",\"sha256\":\"abc\"}],\"initial_view\":[0,0,1]}"))
           (cl-letf
               (((symbol-function 'neomacs-model-available-p)
                 (lambda () gnosis-test-neomacs--available))
                ((symbol-function 'neomacs-model-create) #'gnosis-test-neomacs--create)
                ((symbol-function 'neomacs-model-insert)
                 (lambda (model &rest args)
                   (when gnosis-test-neomacs--insert-error
                     (signal gnosis-test-neomacs--insert-error nil))
                   (insert (propertize " " 'neomacs-model model
                                       'display (list 'surface :id model)
                                       'keymap (plist-get args :keymap)))))
                  ((symbol-function 'neomacs-model-freeze)
                   (lambda (model)
                     (let ((state (gnosis-test-neomacs--state model)))
                       (plist-put state :frozen t)
                       (plist-put state :current nil)
                       (plist-put state :issued (1+ (plist-get state :issued)))
                       (plist-get state :issued))))
                ((symbol-function 'neomacs-model-destroy)
                 (lambda (model)
                   (plist-put (gnosis-test-neomacs--state model) :status '(destroyed))
                   (push model gnosis-test-neomacs--destroyed)))
                ((symbol-function 'neomacs-model-status)
                 (lambda (model) (copy-tree (plist-get (gnosis-test-neomacs--state model) :status))))
                ((symbol-function 'neomacs-model-current-p)
                 (lambda (model)
                   (and gnosis-test-neomacs--available
                          (not (plist-get (gnosis-test-neomacs--state model) :frozen))
                        (eq (car (plist-get (gnosis-test-neomacs--state model) :status)) 'ready)
                        (plist-get (gnosis-test-neomacs--state model) :current)
                        (eql (plist-get (gnosis-test-neomacs--state model) :issued)
                             (plist-get (gnosis-test-neomacs--state model) :settled)))))
                ((symbol-function 'neomacs-model-revision)
                 (lambda (model)
                   (when (neomacs-model-current-p model)
                     (plist-get (gnosis-test-neomacs--state model) :revision))))
                ((symbol-function 'neomacs-model-view)
                 (lambda (model) (copy-sequence (plist-get (gnosis-test-neomacs--state model) :view))))
                ((symbol-function 'neomacs-model-set-view)
                 (lambda (model yaw pitch zoom)
                   (let ((state (gnosis-test-neomacs--state model)))
                     (plist-put state :view (list yaw pitch zoom))
                     (plist-put state :current nil)
                     (plist-put state :issued (1+ (plist-get state :issued)))
                     (plist-get state :issued))))
                ((symbol-function 'neomacs-model-set-highlight)
                 (lambda (model &rest args)
                   (let ((ids (mapcar #'car (plist-get
                                            (plist-get (gnosis-test-neomacs--state model) :create)
                                            :meshes)))
                         (layers (plist-member args :layers)))
                     (when (plist-get (gnosis-test-neomacs--state model) :frozen)
                       (error "Frozen model"))
                     ;; Layers exist only when the renderer advertises them,
                     ;; and replace the single highlight form entirely.
                     (when layers
                       (unless (memq 'highlight-layers gnosis-test-neomacs--capabilities)
                         (error "Unknown keyword :layers"))
                       (when (or (plist-member args :target) (plist-member args :selected))
                         (error "Layers exclude :target and :selected")))
                     ;; Like the primitive: refuse malformed specs synchronously.
                     (mapc #'gnosis-test-neomacs--check-spec
                           (delq nil (cons (plist-get args :target)
                                           (mapcar (lambda (layer) (plist-get layer :target))
                                                   (plist-get args :layers)))))
                     (dolist (id (delq nil (append
                                            (list (plist-get args :selected)
                                                  (plist-get (plist-get args :target) :mesh))
                                            (mapcar (lambda (layer)
                                                      (unless (and (stringp (plist-get layer :color))
                                                                   (xor (plist-member layer :mesh)
                                                                        (plist-member layer :target)))
                                                        (error "Invalid layer %S" layer))
                                                      (or (plist-get layer :mesh)
                                                          (plist-get (plist-get layer :target) :mesh)))
                                                    (plist-get args :layers)))))
                       (unless (member id ids) (error "Unknown model mesh %S" id))))
                   (push args gnosis-test-neomacs--highlights)
                   (let ((state (gnosis-test-neomacs--state model)))
                     (plist-put state :current nil)
                     (plist-put state :issued (1+ (plist-get state :issued)))
                     (plist-get state :issued))))
                ((symbol-function 'neomacs-model-capabilities)
                 (lambda () (copy-sequence gnosis-test-neomacs--capabilities)))
                ((symbol-function 'neomacs-model-resize)
                 (lambda (model width height)
                   (let ((state (gnosis-test-neomacs--state model)))
                     (when (plist-get state :frozen) (error "Frozen model"))
                     (push (list model width height) gnosis-test-neomacs--resizes)
                     (plist-put state :current nil)
                     (plist-put state :issued (1+ (plist-get state :issued)))
                     (plist-get state :issued)))))
             ,@body))
       (delete-directory directory t)))))

(defun gnosis-test-neomacs--ready (model &optional rev)
  "Make MODEL ready and settled at REV, default 1."
  (gnosis-test-neomacs--emit model 'ready nil)
  (gnosis-test-neomacs--emit model 'readiness (list :rev (or rev 1)
                                :req (plist-get (gnosis-test-neomacs--state model) :issued)
                                :current t)))

(ert-deftest gnosis-test-model-neomacs-open-inline ()
  (gnosis-test-neomacs--with-primitive
    (with-temp-buffer
      (text-mode)
      (let ((map (current-local-map))
            (target '((mesh . "bone") (kind . "point") (face . 0)
                      (barycentric . (0.2 0.3 0.5)) (tolerance . 1))))
        (should (eq (current-buffer) (gnosis-neomacs-3d-open path '(5 6 1) 300 t target)))
        (should (eq major-mode 'text-mode))
        (should (eq map (current-local-map)))
        (should (eq gnosis-model-backend--implementation gnosis-neomacs-3d-backend))
        (let* ((model gnosis-neomacs-3d--model)
               (args (plist-get (gnosis-test-neomacs--state model) :create)))
          (should (equal (plist-get args :meshes)
                         (list (list "bone" (expand-file-name "bone.obj" directory) "abc"))))
          (should (= (plist-get args :width) 300))
          (should (= (plist-get args :height) 300))
          (should (equal (car gnosis-test-neomacs--highlights)
                         '(:target (:mesh "bone" :kind point :face 0
                                    :barycentric [0.2 0.3 0.5] :tolerance 1) :selected nil)))
          (should (plist-get (gnosis-neomacs-3d-snapshot) :dirty))
          (gnosis-test-neomacs--ready model)
          (should-not (plist-get (gnosis-neomacs-3d-snapshot) :dirty))
          (should (equal (plist-get (gnosis-neomacs-3d-snapshot) :question-target) target)))))))

(ert-deftest gnosis-test-model-neomacs-picks-are-owned-observations ()
  (gnosis-test-neomacs--with-primitive
    (with-temp-buffer
      (gnosis-neomacs-3d-open path nil nil t nil)
      (let ((model gnosis-neomacs-3d--model) delivered)
        (add-hook 'gnosis-model-backend-selection-hook
                  (lambda (selection) (push selection delivered)) nil t)
        (gnosis-test-neomacs--ready model 4)
        (let ((data '(:rev 4 :hit (:mesh "bone" :face 1 :point [1 2 3]
                                 :barycentric [0.2 0.3 0.5]))))
          (gnosis-test-neomacs--emit model 'pick data)
         (gnosis-test-neomacs--ready model 6)
          (should (= (length delivered) 1))
          (should (equal (car delivered)
                        (list :owner model :frame 6 :id "bone" :mesh "bone" :face 1
                               :point '(1 2 3) :barycentric '(0.2 0.3 0.5))))
         (should (equal (car gnosis-test-neomacs--highlights)
                        '(:target nil :selected "bone")))
          (gnosis-test-neomacs--emit model 'pick '(:rev 3 :hit (:mesh "bone")))
          (should (= (length delivered) 1))
          (gnosis-neomacs-3d-set-view '(15 0 1))
          (should (plist-get (gnosis-neomacs-3d-snapshot) :dirty))
          (should-not (plist-get (gnosis-neomacs-3d-snapshot) :selection))
          (gnosis-test-neomacs--emit model 'pick data)
          (should (= (length delivered) 1))
          (gnosis-test-neomacs--ready model 5)
          (should-not (plist-get (gnosis-neomacs-3d-snapshot) :selection))
          (gnosis-test-neomacs--emit model 'pick '(:rev 5 :hit nil))
         (gnosis-test-neomacs--ready model 7)
          (should (= (length delivered) 2))
          (should-not (plist-get (car delivered) :mesh)))))))

(ert-deftest gnosis-test-model-neomacs-detach-refuses-successor ()
  (gnosis-test-neomacs--with-primitive
    (with-temp-buffer
      (gnosis-neomacs-3d-open path nil nil t nil)
      (let ((old gnosis-neomacs-3d--model))
        (gnosis-neomacs-3d-open path nil nil t nil)
        (let ((new gnosis-neomacs-3d--model))
          (should (memq old gnosis-test-neomacs--destroyed))
          (should-not (gnosis-neomacs-3d-detach old old))
          (should (eq new gnosis-neomacs-3d--model))
          (gnosis-test-neomacs--emit old 'readiness '(:rev 99 :current t))
          (should-not gnosis-neomacs-3d--frame)
          (should (gnosis-neomacs-3d-detach new new))
          (should-not gnosis-neomacs-3d--model)
          (should-not gnosis-model-backend--implementation)
          (should-not (text-property-not-all (point-min) (point-max) 'neomacs-model nil))
          (should-not (gnosis-neomacs-3d-detach new new)))))))

(ert-deftest gnosis-test-model-neomacs-lifecycle-retires ()
  (gnosis-test-neomacs--with-primitive
    (dolist (transition '(mode file kill))
      (let ((buffer (generate-new-buffer " *native-lifecycle*")) model)
        (unwind-protect
            (with-current-buffer buffer
              (gnosis-neomacs-3d-open path nil nil t nil)
              (setq model gnosis-neomacs-3d--model)
              (pcase transition
                ('mode (fundamental-mode))
                ('file (set-visited-file-name (expand-file-name "successor" directory) t))
                ('kill (kill-buffer buffer)))
              (should (memq model gnosis-test-neomacs--destroyed))
              (when (buffer-live-p buffer)
                (should-not gnosis-neomacs-3d--model)
                (when (eq transition 'file)
                  (set-visited-file-name nil)
                  (should-not gnosis-neomacs-3d--model))))
          (when (buffer-live-p buffer) (kill-buffer buffer)))))))

(ert-deftest gnosis-test-model-neomacs-insertion-unwinds-error-and-quit ()
  (gnosis-test-neomacs--with-primitive
    (dolist (failure '(error quit))
      (with-temp-buffer
        (text-mode)
        (let ((map (current-local-map))
              (gnosis-test-neomacs--insert-error failure))
          (should (eq (condition-case err
                          (gnosis-neomacs-3d-open path nil nil t nil)
                        ((error quit) (car err))) failure))
          (should (memq (car gnosis-test-neomacs--created) gnosis-test-neomacs--destroyed))
          (should-not gnosis-neomacs-3d--model)
          (should-not gnosis-model-backend--implementation)
          (should (eq map (current-local-map)))
          (should (eq major-mode 'text-mode))
          (let ((gnosis-test-neomacs--insert-error nil))
            (should (bufferp (gnosis-neomacs-3d-open path nil nil t nil)))))))))

(ert-deftest gnosis-test-model-neomacs-unavailable-allocates-nothing ()
  (gnosis-test-neomacs--with-primitive
    (let ((gnosis-test-neomacs--available nil))
      (should-error (gnosis-neomacs-3d-open path nil) :type 'user-error)
      (should-not gnosis-test-neomacs--created))))

(ert-deftest gnosis-test-model-neomacs-standalone-and-highlight ()
  (gnosis-test-neomacs--with-primitive
    (let ((buffer (gnosis-neomacs-3d-open path nil)))
      (unwind-protect
          (with-current-buffer buffer
            (should (derived-mode-p 'special-mode))
            (should (equal (neomacs-model-view gnosis-neomacs-3d--model) '(0 0 1)))
            (gnosis-test-neomacs--ready gnosis-neomacs-3d--model)
            (gnosis-neomacs-3d-highlight '((mesh . "bone") (kind . "region") (faces . (0 1))) "bone")
            (should (equal (car gnosis-test-neomacs--highlights)
                           '(:target (:mesh "bone" :kind region :faces (0 1)) :selected nil)))
            (should (plist-get (gnosis-neomacs-3d-snapshot) :dirty)))
        (kill-buffer buffer)))))

(ert-deftest gnosis-test-model-neomacs-revision-precedes-readiness-callback ()
  (gnosis-test-neomacs--with-primitive
    (with-temp-buffer
     (gnosis-model-backend-open path '(0 0 1) 256 t
                                '((mesh . "bone") (kind . "object")))
      (let ((model gnosis-neomacs-3d--model))
        (gnosis-test-neomacs--ready model 4)
        (gnosis-test-neomacs--emit model 'pick
                                   '(:rev 4 :hit (:mesh "bone" :face 0 :point [0 0 0])))
        (should (gnosis-model-backend-get :selection))
        ;; Native state records a new current picture before its deferred
        ;; callback reaches this adapter.  The old click is no longer valid.
        (plist-put (gnosis-test-neomacs--state model) :revision 5)
        (should (= 4 gnosis-neomacs-3d--frame))
        (should (= 5 (gnosis-model-backend-get :frame)))
        (should-not (gnosis-model-backend-get :selection))
        (gnosis-test-neomacs--emit model 'pick
                                   '(:rev 5 :hit (:mesh "bone" :face 0 :point [0 0 0])))
        (should (gnosis-model-backend-selection-current-p
                 (gnosis-model-backend-get :selection) t))))))

(ert-deftest gnosis-test-model-backend-selection-and-force ()
  (gnosis-test-neomacs--with-primitive
    (let ((gnosis-model-backend 'auto))
      (should (eq (gnosis-model-backend--choose) gnosis-neomacs-3d-backend))
      (let ((gnosis-test-neomacs--available nil))
        (should (eq (gnosis-model-backend--choose) gnosis-model-backend-canvas))))
    (let ((gnosis-model-backend 'canvas-3d))
      (should (eq (gnosis-model-backend--choose) gnosis-model-backend-canvas)))
    (let ((gnosis-model-backend 'neomacs-3d)
          (gnosis-test-neomacs--available nil))
      (should-error (gnosis-model-backend--choose) :type 'user-error))))

(ert-deftest gnosis-test-model-neomacs-standalone-keymap ()
  (gnosis-test-neomacs--with-primitive
    (save-window-excursion
      (let ((buffer (gnosis-neomacs-3d-open path '(0 0 1) 256)))
        (unwind-protect
            (with-current-buffer buffer
              (should (derived-mode-p 'special-mode))
              (should (eq (window-buffer (selected-window)) buffer))
              (should (equal (gnosis-model-backend-get :view) '(0 0 1)))
              (should (eq (key-binding "f") #'gnosis-neomacs-3d-right))
              (let ((inhibit-read-only t)) (insert "Question away from glyph"))
              (call-interactively (key-binding "f"))
              (should (equal (gnosis-model-backend-get :view) '(15 0 1))))
          (kill-buffer buffer))))))

(ert-deftest gnosis-test-model-neomacs-reframe-target-not-mesh ()
  (gnosis-test-with-db
    (gnosis-test-neomacs--with-primitive
      (save-window-excursion
        (let* ((resource (gnosis-model-import (gnosis-test-model-targets--fixture)))
               (hypothesis (list resource "0" "0" "1")))
          (with-temp-buffer
            (insert "Retained draft")
            (let ((owner (current-buffer)))
              (dolist (target '("tip" "patch" "whole"))
                (dolist (cancel '(nil t))
                  (let* ((fields (list hypothesis (list target)))
                         (context (list :buffer owner :mode major-mode
                                        :tick (buffer-chars-modified-tick)
                                        :database (gnosis--ensure-db) :initial fields))
                         (depth 0))
                    (cl-letf (((symbol-function 'gnosis-model--canvas-size) (lambda () 256))
                              ((symbol-function 'recursion-depth) (lambda () depth))
                              ((symbol-function 'exit-recursive-edit) #'ignore)
                              ((symbol-function 'abort-recursive-edit) (lambda () (signal 'quit nil)))
                              ((symbol-function 'recursive-edit)
                               (lambda ()
                                 (setq depth 1)
                                 (should (equal (gnosis-model-backend-get :selected-id) target))
                                 (should-not (plist-get (car gnosis-test-neomacs--highlights) :selected))
                                 (gnosis-test-neomacs--emit
                                  gnosis-neomacs-3d--model 'ready nil)
                                 (gnosis-test-neomacs--emit
                                  gnosis-neomacs-3d--model 'readiness
                                  '(:rev 1 :req 0 :current t))
                                 (should-error
                                  (call-interactively (key-binding (kbd "RET")))
                                  :type 'user-error)
                                 (gnosis-test-neomacs--ready gnosis-neomacs-3d--model)
                                 (call-interactively (key-binding (kbd (if cancel "q" "RET")))))))
                      (if cancel
                          (should (eq 'quit
                                      (condition-case nil
                                          (gnosis-model--read-visual hypothesis (list target) context)
                                        (quit 'quit))))
                        (should (equal fields
                                       (gnosis-model--read-visual hypothesis (list target) context)))))
                    (should (equal "Retained draft" (with-current-buffer owner (buffer-string)))))))
              (should-not (gnosis-select '* 'themata)))))))))

(ert-deftest gnosis-test-model-neomacs-old-request-cannot-settle ()
  (gnosis-test-neomacs--with-primitive
    (with-temp-buffer
      (gnosis-neomacs-3d-open path nil nil t nil)
      (let ((model gnosis-neomacs-3d--model))
        (gnosis-test-neomacs--ready model 1)
        (let ((req (gnosis-neomacs-3d-set-view '(15 0 1))))
          (gnosis-test-neomacs--emit model 'readiness
                                     (list :rev 2 :req (1- req) :current t))
          (should-not (gnosis-model-backend-get :ready))
          (should-not (gnosis-model-backend-get :frame))
          (gnosis-test-neomacs--emit model 'readiness
                                     (list :rev 3 :req req :current t))
          (should (gnosis-model-backend-get :ready))
          (should (= 3 (gnosis-model-backend-get :frame))))))))

(ert-deftest gnosis-test-model-neomacs-error-recovery ()
  (gnosis-test-neomacs--with-primitive
    (with-temp-buffer
      (gnosis-neomacs-3d-open path nil nil t nil)
      (let ((model gnosis-neomacs-3d--model))
        (gnosis-test-neomacs--ready model 1)
        (let ((req (gnosis-neomacs-3d-set-view '(15 0 1))))
          (gnosis-test-neomacs--emit model 'error
                                     (list :req req :error "Rejected request"))
          (should (equal "Rejected request" (gnosis-model-backend-get :status)))
          (gnosis-neomacs-3d-set-view '(30 0 1))
          (gnosis-test-neomacs--ready model 3)
          (should (equal "Ready" (gnosis-model-backend-get :status)))
          ;; Deferred old errors cannot replace a successful newer request.
          (gnosis-test-neomacs--emit model 'error
                                     (list :req req :error "Late old error"))
          (should (equal "Ready" (gnosis-model-backend-get :status)))
          (gnosis-test-neomacs--emit model 'failed '(:error "Loader failed"))
          (should (equal "Loader failed" (gnosis-model-backend-get :status)))
          (should-not (gnosis-model-backend-get :ready)))))))

(ert-deftest gnosis-test-model-neomacs-status-invalidates-header ()
  (gnosis-test-neomacs--with-primitive
   (with-temp-buffer
     (gnosis-neomacs-3d-open path nil nil t nil)
     (let ((model gnosis-neomacs-3d--model) refreshes)
       (cl-letf (((symbol-function 'force-mode-line-update)
                  (lambda (&optional all) (push (list (current-buffer) all) refreshes))))
         (gnosis-test-neomacs--ready model)
         (should refreshes)
         (should (eq (caar refreshes) (current-buffer)))
         (should (equal "Ready" (gnosis-model-backend-get :status))))))))

(ert-deftest gnosis-test-model-neomacs-paint-settles-exact-pick ()
  (gnosis-test-neomacs--with-primitive
   (with-temp-buffer
     (gnosis-neomacs-3d-open path nil nil t nil)
     (let ((model gnosis-neomacs-3d--model) delivered)
       (add-hook 'gnosis-model-backend-selection-hook
                 (lambda (pick) (push pick delivered)) nil t)
       (gnosis-test-neomacs--ready model 4)
       (gnosis-test-neomacs--emit model 'pick
                                  '(:rev 4 :hit (:mesh "bone" :face 1 :point [1 2 3])))
       (should (equal '(:target nil :selected "bone")
                      (car gnosis-test-neomacs--highlights)))
       (should-not (gnosis-model-backend-get :selection))
       (should-not delivered)
       (gnosis-test-neomacs--emit model 'readiness '(:rev 5 :req 0 :current t))
       (should-not delivered)
       (gnosis-test-neomacs--ready model 6)
       (should (= 1 (length delivered)))
       (should (= 6 (plist-get (car delivered) :frame)))
       (should (equal '(1 2 3) (plist-get (car delivered) :point)))
       (should (gnosis-model-backend-selection-current-p (car delivered) t))
       ;; A camera request supersedes a pending selection paint, even when
       ;; returning to the same camera before delivery.
       (gnosis-test-neomacs--emit model 'pick
                                  '(:rev 6 :hit (:mesh "bone" :face 0 :point [0 0 0])))
       (gnosis-neomacs-3d-set-view '(15 0 1))
       (gnosis-neomacs-3d-set-view '(0 0 1))
       (gnosis-test-neomacs--ready model 9)
       (should (= 1 (length delivered)))
       (should-not (gnosis-model-backend-get :selection))))))

(ert-deftest gnosis-test-model-neomacs-feedback-retains-picture ()
  (gnosis-test-neomacs--with-primitive
   (with-temp-buffer
     (gnosis-neomacs-3d-open path nil nil t nil)
     (let ((model gnosis-neomacs-3d--model))
       (gnosis-test-neomacs--ready model)
       (progn
         (gnosis-model-backend-detach model model t)
         (should (plist-get (gnosis-test-neomacs--state model) :frozen))
         (should-not (memq model gnosis-test-neomacs--destroyed))
         (should (eq (get-text-property (point-min) 'neomacs-model) model))
         (should-not (gnosis-model-backend-get :selection))
         ;; Updating feedback text must not discard the retained picture.
         (let ((inhibit-read-only t))
           (goto-char (point-max))
           (insert "Feedback date")
           (delete-region (1+ (point-min)) (point-max)))
         (should-not (memq model gnosis-test-neomacs--destroyed))
         (should (eq (get-text-property (point-min) 'neomacs-model) model))
         (let ((inhibit-read-only t)) (erase-buffer))
         (should (memq model gnosis-test-neomacs--destroyed))
         (should-not gnosis-neomacs-3d--model))))))

;; Live report: clicking the frozen picture, then RET (the submit key)
;; ran `newline'; electric indentation deleted the glyph's carrier space.
(ert-deftest gnosis-test-model-neomacs-feedback-ret-keeps-picture ()
  "RET during feedback never edits the review or removes its picture."
  (gnosis-test-neomacs--with-primitive
   (with-temp-buffer
     (gnosis-mode)
     (insert "Question\n\n")
     (gnosis-neomacs-3d-open path nil nil t nil)
     (insert "\n\nAnswer: Angle")
     (let ((model gnosis-neomacs-3d--model)
           (text (buffer-string))
           (glyph (text-property-any (point-min) (point-max) 'neomacs-model
                                     gnosis-neomacs-3d--model))
           (electric-indent-mode t)
           shown)
       (gnosis-test-neomacs--ready model)
       (gnosis-model-backend-detach model model t)
       (gnosis-review-feedback-mode 1)
       (cl-letf (((symbol-function 'gnosis-review--feedback-check) #'ignore)
                 ((symbol-function 'gnosis-review--feedback-show)
                  (lambda () (setq shown t))))
         ;; Clicking a picture moves point onto its glyph.
         (dolist (position (list glyph (point-max)))
           (goto-char position)
           (setq last-command-event ?\r)
           (call-interactively (key-binding (kbd "RET")))))
       (should-not (memq model gnosis-test-neomacs--destroyed))
       (should (eq model (get-text-property glyph 'neomacs-model)))
       (should (equal text (buffer-string)))
       ;; RET reopens the feedback actions instead.
       (should shown)
       (gnosis-review-feedback-mode -1)
       (let ((inhibit-read-only t)) (erase-buffer))))))

;; Live report: C-d on the frozen picture destroyed it.
(ert-deftest gnosis-test-model-neomacs-feedback-edits-keep-picture ()
  "Editing commands during feedback leave the review and its picture intact."
  (gnosis-test-neomacs--with-primitive
   (with-temp-buffer
     (gnosis-mode)
     (insert "Question\n\n")
     (gnosis-neomacs-3d-open path nil nil t nil)
     (insert "\n\nAnswer: Angle")
     (let ((model gnosis-neomacs-3d--model)
           (text (buffer-string))
           (glyph (text-property-any (point-min) (point-max) 'neomacs-model
                                     gnosis-neomacs-3d--model))
           (kill-ring (list "yanked")))
       (gnosis-test-neomacs--ready model)
       (gnosis-model-backend-detach model model t)
       (gnosis-review-feedback-mode 1)
       ;; DEL scrolls (special mode); deletion still reaches the text.
       (dolist (case `((,glyph "C-d") (,(1+ glyph) delete-backward-char) (,glyph "C-k")
                       (,glyph "C-y") (,glyph "C-o") (,glyph self-insert-command)))
         (goto-char (car case))
         (setq last-command-event (if (stringp (cadr case))
                                      (aref (kbd (cadr case)) 0)
                                    ?a))
         (should (eq 'buffer-read-only
                     (car (should-error
                           (call-interactively
                            (if (stringp (cadr case))
                                (key-binding (kbd (cadr case)))
                              (cadr case)))))))
         (should-not (memq model gnosis-test-neomacs--destroyed))
         (should (eq model (get-text-property glyph 'neomacs-model)))
         (should (equal text (buffer-string))))
       ;; Leaving feedback restores the writable review.
       (gnosis-review-feedback-mode -1)
       (should-not buffer-read-only)
       (erase-buffer)))))

(ert-deftest gnosis-test-model-review-background-pick-clears-selection ()
  "A background click clears the selection; the header says Select."
  (with-temp-buffer
    (let* ((owner (make-symbol "owner"))
           (gnosis-review--model-context (list :process owner :selection nil))
           (snapshot (list :owner owner :frame 3 :view '(0 0 1) :dirty nil)))
      (cl-letf (((symbol-function 'gnosis-model-backend-get)
                 (lambda (key) (plist-get snapshot key))))
        (gnosis-review--model-selection
         (list :owner owner :frame 3 :id "bone" :mesh "bone" :face 1 :point '(0 0 0)))
        (should (equal (plist-get (plist-get gnosis-review--model-context :selection) :mesh)
                       "bone"))
        (gnosis-review--model-selection
         (list :owner owner :frame 3 :id nil :mesh nil :face nil :point nil))
        (should-not (plist-get gnosis-review--model-context :selection))))))

(ert-deftest gnosis-test-model-review-hides-cursor ()
  "Review text hides the cursor that would frame a model picture."
  (with-temp-buffer
    (let ((gnosis-review--display-buffer (current-buffer)))
      (gnosis-display-keimenon "Question"))
    (should (local-variable-p 'cursor-type))
    (should-not cursor-type)))

(ert-deftest gnosis-test-model-neomacs-feedback-next-quit-cancel-retires ()
  (gnosis-test-neomacs--with-primitive
   (dolist (choice '(?n ?q cancel))
     (with-temp-buffer
       (gnosis-neomacs-3d-open path nil nil t nil)
       (let ((model gnosis-neomacs-3d--model))
         (gnosis-test-neomacs--ready model)
         (setq gnosis-review--model-context
               (list :process model :attachment model))
         (gnosis-review--model-detach gnosis-review--model-context t)
         (should-not (memq model gnosis-test-neomacs--destroyed))
         (cl-letf (((symbol-function 'gnosis-review--check-result-content) #'ignore)
                   ((symbol-function 'gnosis-review--collection-action-check) #'ignore)
                   ((symbol-function 'gnosis-review--accept) (lambda (&rest _) 'accepted))
                   ((symbol-function 'gnosis-review--read-action)
                    (lambda (_)
                      (if (eq choice 'cancel) (signal 'quit nil) choice))))
           (condition-case nil
               (catch 'review-loop (gnosis-review-actions t 1 nil))
             (quit nil)))
         (should (memq model gnosis-test-neomacs--destroyed))
         (should-not gnosis-review--model-picture))))))

(ert-deftest gnosis-test-model-neomacs-entry-validation-error-retires ()
  (gnosis-test-neomacs--with-primitive
   (with-temp-buffer
     (gnosis-neomacs-3d-open path nil nil t nil)
     (let ((model gnosis-neomacs-3d--model))
       (gnosis-test-neomacs--ready model)
       (gnosis-review--model-detach (list :process model :attachment model) t)
       (cl-letf (((symbol-function 'gnosis-review--check-result-content)
                  (lambda (&rest _) (user-error "Stale pending content"))))
         (should-error (gnosis-review-actions t 1 nil) :type 'user-error))
       (should (memq model gnosis-test-neomacs--destroyed))))))

(ert-deftest gnosis-test-model-neomacs-inner-action-error-retires ()
  (gnosis-test-neomacs--with-primitive
   (with-temp-buffer
     (gnosis-neomacs-3d-open path nil nil t nil)
     (let ((model gnosis-neomacs-3d--model))
       (gnosis-test-neomacs--ready model)
       (gnosis-review--model-detach (list :process model :attachment model) t)
       (cl-letf (((symbol-function 'gnosis-review--check-result-content) #'ignore)
                 ((symbol-function 'gnosis-review--read-action)
                  (lambda (&rest _) (error "Reader failed"))))
         (should-error (gnosis-review-actions t 1 nil)))
       (should (memq model gnosis-test-neomacs--destroyed))
       (should-not gnosis-review--model-picture)))))

(ert-deftest gnosis-test-model-neomacs-successor-preserved ()
  (gnosis-test-neomacs--with-primitive
   (with-temp-buffer
     (gnosis-neomacs-3d-open path nil nil t nil)
     (let ((old gnosis-neomacs-3d--model) successor)
       (gnosis-test-neomacs--ready old)
       (gnosis-review--model-detach (list :process old :attachment old) t)
       (cl-letf (((symbol-function 'gnosis-review--check-result-content) #'ignore)
                 ((symbol-function 'gnosis-review--read-action)
                  (lambda (&rest _)
                    (gnosis-neomacs-3d-open path nil nil t nil)
                    (setq successor gnosis-neomacs-3d--model)
                    (gnosis-test-neomacs--ready successor)
                    (gnosis-review--model-detach (list :process successor :attachment successor) t)
                    (error "Old feedback interrupted"))))
         (should-error (gnosis-review-actions t 1 nil)))
       (should (memq old gnosis-test-neomacs--destroyed))
       (should-not (memq successor gnosis-test-neomacs--destroyed))
       (should (eq successor gnosis-neomacs-3d--model))
       (should (eq successor (car gnosis-review--model-picture)))))))

(ert-deftest gnosis-test-model-neomacs-public-render-quit-retires ()
  (gnosis-test-with-db
   (save-window-excursion
     (let* ((model (gnosis-test-model--add))
            (gnosis-review-buffer-name " *Gnosis native feedback failure*")
            (buffer (gnosis-review--setup-buffer (list model) 'due)))
       (unwind-protect
           (with-current-buffer buffer
             (setf (gnosis-review-state-persistent-p gnosis-review--state) t)
             (gnosis-review--save-session gnosis-review--state)
	     (gnosis-test-neomacs--with-primitive
	      (let ((depth 0) submitted)
		(cl-letf (((symbol-function 'gnosis-model--canvas-size) (lambda () 400))
			  ((symbol-function 'recursion-depth) (lambda () depth))
			  ((symbol-function 'exit-recursive-edit) #'ignore)
			  ((symbol-function 'gnosis-display-parathema)
			   (lambda (&rest _) (signal 'quit nil)))
			  ((symbol-function 'recursive-edit)
			   (lambda ()
			     (setq depth 1)
			     (gnosis-test-model--wait-for-preparation)
			     (setq submitted gnosis-neomacs-3d--model)
			     (gnosis-test-neomacs--ready submitted)
			     (gnosis-test-neomacs--emit submitted 'pick
							'(:rev 1 :hit (:mesh "triangle" :face 0 :point [0 0 0])))
			     (gnosis-test-neomacs--ready submitted 2)
			     (gnosis-review-model-submit))))
		  (should (eq 'quit (condition-case nil
					(gnosis-review--display-thema model)
				      (quit 'quit)))))
		(should submitted)
		(should-not (gnosis-select '* 'review-events))
		(let ((retired (memq submitted gnosis-test-neomacs--destroyed)))
		  ;; Retire while the stub is still installed, before the fixture kills
		  ;; its buffer outside the primitive binding.
		  (gnosis-neomacs-3d-detach submitted submitted)
		  (should retired)))))
         (when (buffer-live-p buffer) (kill-buffer buffer)))))))

(defconst gnosis-test-neomacs--reveal-target
  '((id . "t") (label . "Bone") (mesh . "bone") (kind . "region") (faces 0 1))
  "Gnosis region target on the stub scene's only mesh.")

(defun gnosis-test-neomacs--reveal (model reveal)
  "Reveal REVEAL on MODEL, presenting its requests from a timer."
  (let ((timer (run-at-time 0.05 nil (lambda ()
                                       (gnosis-test-neomacs--ready
                                        model (1+ (plist-get (gnosis-test-neomacs--state model)
                                                             :issued)))))))
    (unwind-protect
        (gnosis-neomacs-3d-reveal
         model model
         (append reveal (list :correct-color "#00ff00" :wrong-color "#ff0000")))
      (cancel-timer timer))))

(ert-deftest gnosis-test-model-neomacs-reveal-layers-wrong-and-right ()
  "A wrong pick paints red beneath the green target; a right pick green only."
  (gnosis-test-neomacs--with-primitive
   (dolist (wrong '("bone" nil))
     (with-temp-buffer
       (setq gnosis-test-neomacs--highlights nil
             gnosis-test-neomacs--capabilities '(highlight-layers))
       (gnosis-neomacs-3d-open path nil 400 t nil)
       (let ((model gnosis-neomacs-3d--model))
         (gnosis-test-neomacs--ready model)
         (should (gnosis-test-neomacs--reveal
                  model (list :target gnosis-test-neomacs--reveal-target
                              :wrong-mesh wrong :size 300)))
         (should (equal (car gnosis-test-neomacs--highlights)
                        (list :layers
                              (append
                               (and wrong '((:mesh "bone" :color "#ff0000")))
                               '((:target (:mesh "bone" :kind region :faces (0 1))
                                          :color "#00ff00"))))))
         ;; The revealed picture was presented before it is retained.
         (should (neomacs-model-current-p model))
         (should (equal (car gnosis-test-neomacs--resizes) (list model 300 300)))
         (gnosis-model-backend-detach model model t)
         (should (plist-get (gnosis-test-neomacs--state model) :frozen))
         (let ((inhibit-read-only t)) (erase-buffer)))))))

(defconst gnosis-test-neomacs--point-target
  '((id . "p") (label . "Angle") (mesh . "bone") (kind . "point") (face . 0)
    (barycentric 0.2 0.3 0.5) (tolerance . 0.01))
  "Gnosis point target on the stub scene's only mesh, with a tiny tolerance.")

(defun gnosis-test-neomacs--layer-shape (layer)
  "Return LAYER as (COLOR KIND MESH TOLERANCE), dropping exact weights."
  (let ((target (plist-get layer :target)))
    (list (plist-get layer :color)
          (if target (plist-get target :kind) 'whole-mesh)
          (or (plist-get layer :mesh) (plist-get target :mesh))
          (plist-get target :tolerance))))

(ert-deftest gnosis-test-model-neomacs-reveal-point-and-region-markers ()
  "Point/region reveals mark a wrong pick with a red point, never a red mesh.
The correct target paints green on top; a point target uses the display
radius, not its grading tolerance.  Without a usable pick only green shows."
  (gnosis-test-neomacs--with-primitive
   (dolist (target (list gnosis-test-neomacs--point-target
                         gnosis-test-neomacs--reveal-target))
     (dolist (pick '((:mesh "bone" :face 1 :barycentric (0.5000004 0.5 -1e-9))
                     nil
                     (:mesh nil :face nil :barycentric nil)
                     (:mesh "bone" :face 1)))
       (with-temp-buffer
         (setq gnosis-test-neomacs--highlights nil
               gnosis-test-neomacs--capabilities '(highlight-layers))
         (gnosis-neomacs-3d-open path nil 400 t nil)
         (let ((model gnosis-neomacs-3d--model)
               (point (equal (alist-get 'kind target) "point")))
           (gnosis-test-neomacs--ready model)
           (should (gnosis-test-neomacs--reveal
                    model (list :target target :wrong-mesh nil :wrong-pick pick
                                :marker-radius 0.25 :size 300)))
           (let ((layers (plist-get (car gnosis-test-neomacs--highlights) :layers)))
             (should (equal (mapcar #'gnosis-test-neomacs--layer-shape layers)
                            (append
                             (and (plist-get pick :barycentric) (plist-get pick :mesh)
                                  '(("#ff0000" point "bone" 0.25)))
                             ;; The answer point is twice the pick's radius.
                             (list (list "#00ff00" (if point 'point 'region) "bone"
                                         (and point 0.5))))))
             (when (cdr layers)
               ;; The red marker sits at the learner's pick, normalized.
               (let ((spec (plist-get (car layers) :target)))
                 (should (= 1 (plist-get spec :face)))
                 (should (< (abs (- 1 (apply #'+ (append (plist-get spec :barycentric) nil))))
                            1e-12))
                 (should (seq-every-p (lambda (w) (<= 0 w))
                                      (plist-get spec :barycentric)))))
             (when point
               (should (equal (plist-get (plist-get (car (last layers)) :target) :barycentric)
                              [0.2 0.3 0.5]))))
           (gnosis-model-backend-detach model model t)
           (let ((inhibit-read-only t)) (erase-buffer))))))))

(ert-deftest gnosis-test-model-neomacs-reveal-without-radius-keeps-target ()
  "Missing display radius keeps the authored point and shows no red marker."
  (gnosis-test-neomacs--with-primitive
   (with-temp-buffer
     (setq gnosis-test-neomacs--capabilities '(highlight-layers))
     (gnosis-neomacs-3d-open path nil 400 t nil)
     (let ((model gnosis-neomacs-3d--model))
       (gnosis-test-neomacs--ready model)
       (should (gnosis-test-neomacs--reveal
                model (list :target gnosis-test-neomacs--point-target
                            :wrong-pick '(:mesh "bone" :face 1 :barycentric (0.2 0.3 0.5)))))
       (should (equal (mapcar #'gnosis-test-neomacs--layer-shape
                              (plist-get (car gnosis-test-neomacs--highlights) :layers))
                      '(("#00ff00" point "bone" 0.01))))
       (let ((inhibit-read-only t)) (erase-buffer))))))

(ert-deftest gnosis-test-model-neomacs-reveal-fallback-without-layers ()
  "Without highlight layers the single highlight shows the answer.
The learner's yellow selection is replaced by the correct target; a point
target uses the display radius.  Pictures only shrink."
  (gnosis-test-neomacs--with-primitive
   (dolist (capabilities '(nil unbound))
     (with-temp-buffer
       (setq gnosis-test-neomacs--highlights nil gnosis-test-neomacs--resizes nil)
       (cl-letf (((symbol-function 'neomacs-model-capabilities)
                  (if (eq capabilities 'unbound) nil (lambda () nil))))
         (gnosis-neomacs-3d-open path nil 400 t nil)
         (let ((model gnosis-neomacs-3d--model))
           (gnosis-test-neomacs--ready model)
           (should (gnosis-test-neomacs--reveal
                    model (list :target gnosis-test-neomacs--reveal-target
                                :wrong-mesh "bone" :marker-radius 0.25 :size 200)))
           (should (equal gnosis-test-neomacs--highlights
                          '((:target (:mesh "bone" :kind region :faces (0 1)) :selected nil))))
           (should (neomacs-model-current-p model))
           (should (equal gnosis-test-neomacs--resizes (list (list model 200 200))))
           ;; Never enlarge; the answer is still shown.
           (should (gnosis-test-neomacs--reveal
                    model (list :target gnosis-test-neomacs--point-target
                                :wrong-pick '(:mesh "bone" :face 1 :barycentric (1 0 0))
                                :marker-radius 0.25 :size 600)))
           (should (equal (car gnosis-test-neomacs--highlights)
                          '(:target (:mesh "bone" :kind point :face 0
                                     :barycentric [0.2 0.3 0.5] :tolerance 0.5)
                                    :selected nil)))
           (should (= 1 (length gnosis-test-neomacs--resizes)))
           ;; Nothing to reveal and nothing to shrink issues nothing.
           (should-not (gnosis-test-neomacs--reveal model (list :size 600)))
           (should (= 2 (length gnosis-test-neomacs--highlights)))
           (let ((inhibit-read-only t)) (erase-buffer))))))))

(ert-deftest gnosis-test-model-neomacs-reveal-refuses-stale-frozen-successor ()
  (gnosis-test-neomacs--with-primitive
   (with-temp-buffer
     (setq gnosis-test-neomacs--capabilities '(highlight-layers))
     (gnosis-neomacs-3d-open path nil 400 t nil)
     (let ((old gnosis-neomacs-3d--model))
       (gnosis-test-neomacs--ready old)
       (gnosis-neomacs-3d-open path nil 400 t nil)
       (gnosis-test-neomacs--ready gnosis-neomacs-3d--model)
       (should-not (gnosis-neomacs-3d-reveal old old (list :size 100)))
       (let ((model gnosis-neomacs-3d--model))
         (gnosis-model-backend-detach model model t)
         (should-not (gnosis-neomacs-3d-reveal model model (list :size 100)))
         (should-not gnosis-test-neomacs--resizes)
         (should-not gnosis-test-neomacs--highlights))
       (let ((inhibit-read-only t)) (erase-buffer))))))

(ert-deftest gnosis-test-model-neomacs-reveal-times-out-without-presentation ()
  "A renderer that never presents cannot hold feedback past the timeout."
  (gnosis-test-neomacs--with-primitive
   (with-temp-buffer
     (setq gnosis-test-neomacs--capabilities '(highlight-layers))
     (gnosis-neomacs-3d-open path nil 400 t nil)
     (let ((model gnosis-neomacs-3d--model)
           (gnosis-neomacs-3d-reveal-timeout 0.2)
           (start (float-time)))
       (gnosis-test-neomacs--ready model)
       (should (gnosis-neomacs-3d-reveal
                model model (list :target gnosis-test-neomacs--reveal-target
                                  :correct-color "#00ff00")))
       (should (< (- (float-time) start) 2))
       (should-not (neomacs-model-current-p model))
       (let ((inhibit-read-only t)) (erase-buffer))))))

(ert-deftest gnosis-test-model-backend-reveal-dispatch ()
  "Reveal is optional, owner-qualified and carries the configured colors."
  (let (seen)
    (with-temp-buffer
      (setq-local gnosis-model-backend--implementation '(:snapshot ignore))
      (should-not (gnosis-model-backend-reveal 'owner 'attachment '(:size 1)))
      (setq-local gnosis-model-backend--implementation
                  (list :reveal (lambda (&rest args) (setq seen args) t)))
      (let ((gnosis-model-correct-color "green") (gnosis-model-wrong-color "red"))
        (should (gnosis-model-backend-reveal 'owner 'attachment '(:size 1))))
      (should (equal seen '(owner attachment
                                  (:correct-color "green" :wrong-color "red" :size 1)))))))

(ert-deftest gnosis-test-model-review-reveal-arguments ()
  "Review reveals the correct target.  A wrong object Find adds its mesh;
a wrong point/region Find adds only the learner's pick as a point."
  (let* ((targets '(((id . "a") (label . "A") (mesh . "m1") (kind . "object"))
                    ((id . "b") (label . "B") (mesh . "m1") (kind . "point") (face . 0)
                     (barycentric 1 0 0) (tolerance . 0.01))
                    ((id . "c") (label . "C") (mesh . "m1") (kind . "region") (faces 0 1))))
         (scene (list (cons 'targets targets)))
         (pick '(:mesh "m2" :face 3 :barycentric (0.1 0.2 0.7)))
         calls)
    (cl-letf (((symbol-function 'gnosis-model-target)
               (lambda (s id) (should (eq s scene))
                 (seq-find (lambda (target) (equal id (alist-get 'id target))) targets)))
              ((symbol-function 'gnosis-model-backend-reveal)
               (lambda (&rest args) (push args calls) t))
              ((symbol-function 'gnosis-review--feedback-picture-size) (lambda (_) 321)))
      (with-temp-buffer
        (dolist (case `((find "a" nil "m2" nil) (find "a" t nil nil)
                        (find "b" nil nil ,pick) (find "b" t nil nil)
                        (find "c" nil nil ,pick) (find "c" t nil nil)
                        (name "a" nil nil nil) (name "b" nil nil nil) (name "c" t nil nil)
                        ;; A pick without a surface never paints red.
                        (find "b" nil nil nil :background)))
          (setq-local gnosis-review--model-context
                      (list :process 'p :attachment 'a :view '(10 20 2)
                            :fields (list :response (nth 0 case) :target (nth 1 case)
                                          :verified (list :manifest scene :radius 10.0))
                            :selection (if (nth 5 case) '(:mesh nil :point nil)
                                         (append '(:id nil :point (1 2 3)) pick))))
          (let ((gnosis-model-reveal-point-scale 0.04))
            (gnosis-review--model-reveal gnosis-review--model-context (nth 2 case)))
          (should (equal (pop calls)
                         (list 'p 'a (list :target (gnosis-model-target scene (nth 1 case))
                                          :wrong-mesh (nth 3 case)
                                          :wrong-pick (nth 4 case)
                                          ;; A hidden buffer has no window to fit,
                                          ;; so only the scene fraction applies.
                                          :marker-radius 0.4
                                          :size nil)))))))))

(ert-deftest gnosis-test-model-review-reveal-uses-live-camera ()
  "Markers size from the owned renderer's live zoom and picture size.
Name answers have no selection view; reveal only shrinks the picture."
  (let* ((target '((id . "b") (label . "B") (mesh . "m1") (kind . "point") (face . 0)
                   (barycentric 1 0 0) (tolerance . 0.01)))
         (scene (list (cons 'targets (list target))))
         (owner 'p) (current 400) calls)
    (cl-letf (((symbol-function 'gnosis-model-target) (lambda (_ _) target))
              ((symbol-function 'gnosis-model-backend-snapshot)
               (lambda () (list :owner owner :view '(0 0 0.25) :size current)))
              ((symbol-function 'gnosis-model-backend-reveal)
               (lambda (&rest args) (push args calls) t))
              ((symbol-function 'get-buffer-window) (lambda (&rest _) 'window))
              ((symbol-function 'gnosis-review--feedback-picture-size) (lambda (_) 200)))
      (with-temp-buffer
        (setq-local gnosis-review--model-context
                    (list :process 'p :attachment 'a :view nil :selection nil
                          :fields (list :response 'name :target "b"
                                        :verified (list :manifest scene :radius 10.0))))
        (let ((gnosis-model-reveal-point-scale 0.06)
              (gnosis-model-reveal-point-pixels 10))
          (gnosis-review--model-reveal gnosis-review--model-context t)
          ;; 10 px at zoom 0.25 on 200 px: 10 * 10 / (0.4 * 0.25 * 200) = 5.
          (should (< (abs (- 5.0 (plist-get (nth 2 (pop calls)) :marker-radius))) 1e-12))
          ;; Reveal never enlarges, so a smaller picture keeps the pixel floor.
          (setq current 100)
          (gnosis-review--model-reveal gnosis-review--model-context t)
          (should (< (abs (- 10.0 (plist-get (nth 2 (pop calls)) :marker-radius))) 1e-12))
          ;; Another owner's camera is never used; the retained view is.
          (setq owner 'other)
          (setf (plist-get gnosis-review--model-context :view) '(0 0 1))
          (gnosis-review--model-reveal gnosis-review--model-context t)
          (should (< (abs (- 1.25 (plist-get (nth 2 (pop calls)) :marker-radius))) 1e-12)))))))

(ert-deftest gnosis-test-model-reveal-marker-radius ()
  "Display radius is a scene fraction with an on-screen pixel floor."
  (let ((gnosis-model-reveal-point-scale 0.04)
        (gnosis-model-reveal-point-pixels 8))
    (should-not (gnosis-model-backend-marker-radius nil 1 500))
    (should (= 0.4 (gnosis-model-backend-marker-radius 10 1 nil)))
    ;; 1000 px picture at zoom 1: 0.04 R is 16 px, above the floor.
    (should (= 0.4 (gnosis-model-backend-marker-radius 10 1 1000)))
    ;; 200 px picture: 8 px needs R * 8 / (0.4 * 200) = 1.0.
    (should (< (abs (- 1.0 (gnosis-model-backend-marker-radius 10 1 200))) 1e-12))
    ;; Zooming in enlarges the scene, so the floor needs less model distance.
    (should (< (abs (- 0.5 (gnosis-model-backend-marker-radius 10 2 200))) 1e-12))
    (should (= 0.4 (gnosis-model-backend-marker-radius 10 nil nil)))))

(ert-deftest gnosis-test-model-review-reveal-revalidates-after-wait ()
  "A wait that kills, cancels or replaces the encounter cannot continue it."
  (let ((scene '((targets . (((id . "a") (label . "A") (mesh . "m1") (kind . "object")))))))
    (dolist (change '(kill cancel successor))
      (let* ((owner (generate-new-buffer " *gnosis reveal owner*"))
             (other (generate-new-buffer " *gnosis reveal other*"))
             ;; Live contexts carry :cancelled so it is set in place.
             (context (list :process 'p :attachment 'a :cancelled nil
                            :fields (list :response 'find :target "a"
                                          :verified (list :manifest scene))
                            :selection '(:mesh "m1"))))
        (unwind-protect
            (with-current-buffer owner
              (setq-local gnosis-review--model-context context)
              (cl-letf (((symbol-function 'gnosis-model-target)
                         (lambda (s _id) (car (alist-get 'targets s))))
                        ((symbol-function 'gnosis-model-backend-reveal)
                         (lambda (&rest _)
                           ;; What a timer may do while the backend waits.
                           (pcase change
                             ('kill (kill-buffer owner) (set-buffer other))
                             ('cancel (setf (plist-get context :cancelled) t))
                             ('successor (setq-local gnosis-review--model-context
                                                     (list :process 'q))))
                           t)))
                (should-error (gnosis-review--model-reveal context nil) :type 'user-error))
              (should-not (buffer-local-value 'gnosis-review--layout other)))
          (when (buffer-live-p owner) (kill-buffer owner))
          (kill-buffer other))))))

;; Point and region Finds mark the picked spot; object Finds the mesh.
(ert-deftest gnosis-test-model-neomacs-pick-marks-spot ()
  "With layers, a point/region Find pick paints a point where it landed.
Re-picking moves it; a background pick clears it.  Object Finds, unset
radius and runtimes without layers keep whole-mesh selection."
  (gnosis-test-neomacs--with-primitive
   (dolist (case '((10.0 (highlight-layers) spot) (nil (highlight-layers) mesh)
                   (10.0 nil mesh)))
     (with-temp-buffer
       (setq gnosis-test-neomacs--highlights nil
             gnosis-test-neomacs--capabilities (nth 1 case))
       (setq-local gnosis-model-backend-pick-radius (nth 0 case))
       (gnosis-neomacs-3d-open path nil 400 t nil)
       (let ((model gnosis-neomacs-3d--model) delivered
             (gnosis-model-pick-color "#abcdef")
             (gnosis-model-reveal-point-scale 0.05)
             (gnosis-model-reveal-point-pixels 1))
         (add-hook 'gnosis-model-backend-selection-hook
                   (lambda (pick) (push pick delivered)) nil t)
         (gnosis-test-neomacs--ready model 4)
         (dolist (face '(1 0))
           (gnosis-test-neomacs--emit
            model 'pick `(:rev ,(plist-get (gnosis-test-neomacs--state model) :revision)
                               :hit (:mesh "bone" :face ,face :point [1 2 3]
                                           :barycentric [0.2 0.3 0.5])))
           (should (equal (car gnosis-test-neomacs--highlights)
                          (if (eq (nth 2 case) 'spot)
                              `(:layers ((:target (:mesh "bone" :kind point :face ,face
                                                         :barycentric [0.2 0.3 0.5]
                                                         :tolerance 0.5)
                                                  :color "#abcdef")))
                            '(:target nil :selected "bone"))))
           ;; The pick still grades: it is admitted once its paint shows.
           (gnosis-test-neomacs--ready model (+ 10 face))
           (should (equal (plist-get (car delivered) :face) face))
           (should (equal (plist-get (car delivered) :barycentric) '(0.2 0.3 0.5))))
         (gnosis-test-neomacs--emit model 'pick '(:rev 10 :hit nil))
         (should (equal (car gnosis-test-neomacs--highlights)
                        '(:target nil :selected nil))))))))

(ert-deftest gnosis-test-model-review-pick-radius-by-kind ()
  "Only point and region Finds ask the backend to mark the picked spot."
  (let* ((targets '(((id . "a") (mesh . "m") (kind . "object"))
                    ((id . "b") (mesh . "m") (kind . "point"))
                    ((id . "c") (mesh . "m") (kind . "region"))))
         (scene (list (cons 'targets targets))))
    (cl-letf (((symbol-function 'gnosis-model-target)
               (lambda (_ id) (seq-find (lambda (target) (equal id (alist-get 'id target)))
                                        targets))))
      (dolist (case '((find "a" nil) (find "b" 7.0) (find "c" 7.0)
                      (name "b" nil) (name "c" nil)))
        (should (equal (nth 2 case)
                       (gnosis-review--model-pick-radius
                        (list :response (nth 0 case) :target (nth 1 case)
                              :verified (list :manifest scene :radius 7.0)))))))))

(defconst gnosis-test-relational--cue
  '((id . "cue") (label . "Bone") (mesh . "bone") (kind . "object"))
  "Object cue target on the native stub scene's only mesh.")

(ert-deftest gnosis-model-relational-native-cue-layers ()
  "Native cues paint in the cue color beneath picks and feedback colors."
  (gnosis-test-neomacs--with-primitive
   (with-temp-buffer
     (setq gnosis-test-neomacs--highlights nil
           gnosis-test-neomacs--capabilities '(highlight-layers))
     (gnosis-neomacs-3d-open path nil 400 t nil)
     (let ((model gnosis-neomacs-3d--model)
           (gnosis-model-cue-color "#0000ff")
           (gnosis-model-pick-color "#ffff00"))
       (gnosis-test-neomacs--ready model 2)
       (should (gnosis-model-backend-cue (list gnosis-test-relational--cue)))
       (should (equal (car gnosis-test-neomacs--highlights)
                      '(:layers ((:target (:mesh "bone" :kind object) :color "#0000ff")))))
       ;; The cue is not a question target: picks still paint and deliver.
       (should-not (gnosis-model-backend-get :question-target))
       (gnosis-test-neomacs--ready model 3)
       (gnosis-test-neomacs--emit model 'pick '(:rev 3 :hit (:mesh "bone" :face 0 :point [1 2 3]
                                                                  :barycentric [1 0 0])))
       (should (equal (car gnosis-test-neomacs--highlights)
                      '(:layers ((:target (:mesh "bone" :kind object) :color "#0000ff")
                                 (:mesh "bone" :color "#ffff00")))))
       ;; A background pick keeps the cue.
       (gnosis-test-neomacs--ready model 4)
       (gnosis-test-neomacs--emit model 'pick '(:rev 4 :hit nil))
       (should (equal (car gnosis-test-neomacs--highlights)
                      '(:layers ((:target (:mesh "bone" :kind object) :color "#0000ff")))))
       ;; Feedback: cue beneath, wrong pick red, every acceptable target green.
       (gnosis-test-neomacs--ready model 5)
       (should (gnosis-neomacs-3d-reveal
                model model
                (list :target gnosis-test-relational--cue
                      :alternates (list gnosis-test-relational--cue)
                      :cues (list gnosis-test-relational--cue)
                      :wrong-mesh "bone" :correct-color "#00ff00" :wrong-color "#ff0000"
                      :cue-color "#0000ff")))
       (should (equal (car gnosis-test-neomacs--highlights)
                      '(:layers ((:target (:mesh "bone" :kind object) :color "#0000ff")
                                 (:mesh "bone" :color "#ff0000")
                                 (:target (:mesh "bone" :kind object) :color "#00ff00")
                                 (:target (:mesh "bone" :kind object) :color "#00ff00")))))
       (gnosis-model-backend-detach model model)))
   ;; Without layers the native adapter declines; callers name cues in text.
   (with-temp-buffer
     (setq gnosis-test-neomacs--highlights nil
           gnosis-test-neomacs--capabilities nil)
     (gnosis-neomacs-3d-open path nil 400 t nil)
     (should-not (gnosis-model-backend-cue (list gnosis-test-relational--cue)))
     (should-not gnosis-test-neomacs--highlights)
     ;; Whole set or nothing: a contrast set never paints its first cue alone.
     (let* ((answer '((id . "answer") (label . "Answer") (mesh . "bone") (kind . "object")))
            (scene `((version . 2) (targets . (,gnosis-test-relational--cue ,answer)))))
       (dolist (cues '(("answer" "cue") ("cue" "answer") ("cue")))
         (should-not (gnosis-model-show-cues
                      (list :target "answer" :answers '("answer") :cues cues
                            :verified (list :manifest scene))))
         (should-not gnosis-test-neomacs--highlights)
         (should-not gnosis-neomacs-3d--target)))
     (gnosis-model-backend-detach gnosis-neomacs-3d--model gnosis-neomacs-3d--model))))

;; The adapter is optional: a pick must work without the model domain.
(ert-deftest gnosis-test-model-neomacs-standalone-pick-without-domain ()
  "A fresh process loading only the adapter selects and clears picks."
  (let* ((directory (make-temp-file "gnosis-native-standalone-" t))
         (scene (expand-file-name "scene.json" directory))
         (adapter (file-name-directory (locate-library "gnosis-neomacs-3d"))))
    (unwind-protect
        (progn
          (with-temp-file scene
            (insert "{\"objects\":[{\"id\":\"bone\",\"path\":\"bone.obj\",\"sha256\":\"abc\"}],\"initial_view\":[0,0,1]}"))
          (with-temp-buffer
            (let* ((print-length nil)
                   (print-level nil)
                   (status
                    (call-process
                     (expand-file-name invocation-name invocation-directory)
                     nil t nil "-Q" "--batch" "-L" adapter "--eval"
                     (prin1-to-string
                      `(progn
                         (setq backtrace-on-error-noninteractive nil)
                         (require 'gnosis-neomacs-3d)
                         (provide 'neomacs-model)
                         (defun neomacs-model-available-p () t)
                         (defun neomacs-model-create (&rest _) 'model)
                         (defun neomacs-model-view (_) '(0 0 1))
                         (defun neomacs-model-current-p (_) t)
                         (defun neomacs-model-revision (_) 1)
                         (defun neomacs-model-capabilities () '(highlight-layers))
                         (defun neomacs-model-insert (_ &rest _) (insert " "))
                         (defun neomacs-model-set-highlight (_ &rest args)
                           (princ (format "highlight=%S\n" args))
                           2)
                         (with-temp-buffer
                           (gnosis-neomacs-3d-open ,scene nil 400 t nil)
                           (princ (format "domain=%S\n" (featurep 'gnosis-model-backend)))
                           (gnosis-neomacs-3d--event
                            (current-buffer) 'model 'pick
                            '(:rev 1 :hit (:mesh "bone" :face 0 :point [0 0 0]
                                                 :barycentric [1 0 0])))
                           (gnosis-neomacs-3d--event
                            (current-buffer) 'model 'pick '(:rev 1 :hit nil))))))))
              ;; Compare the output too so a failure shows the child's error.
              (should (equal (list status (buffer-string))
                             (list 0 (concat "domain=nil\n"
                                             "highlight=(:target nil :selected \"bone\")\n"
                                             "highlight=(:target nil :selected nil)\n")))))))
      (delete-directory directory t))))

(provide 'gnosis-test-model-neomacs)
;;; gnosis-test-model-neomacs.el ends here