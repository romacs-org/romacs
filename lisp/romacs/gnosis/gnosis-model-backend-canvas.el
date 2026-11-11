;;; gnosis-model-backend-canvas.el --- Canvas model display adapter -*- lexical-binding: t; -*-

;; Copyright (C) 2026 Free Software Foundation, Inc.
;; SPDX-License-Identifier: GPL-3.0-or-later
;; Maintainer: Thanos Apollo <public@thanosapollo.org>

;;; Commentary:
;; Confine canvas-3d representation knowledge here.  Loading this adapter
;; does not load the optional renderer or start a process.

;;; Code:

(declare-function canvas-3d--request "canvas-3d" ())
(declare-function canvas-3d-open "canvas-3d" (path &optional name view size))
(declare-function canvas-3d-attach "canvas-3d" (path &optional name view size))
(declare-function canvas-3d-detach "canvas-3d" ())
(declare-function canvas-3d--python "canvas-3d" ())
(declare-function gnosis-model--renderer-directory "gnosis-model" ())
(defvar canvas-3d--directory)
(defvar canvas-3d--process)
(defvar canvas-3d--image)
(defvar canvas-3d--frame)
(defvar canvas-3d--busy)
(defvar canvas-3d--size)
(defvar canvas-3d--dirty)
(defvar canvas-3d--yaw)
(defvar canvas-3d--pitch)
(defvar canvas-3d--zoom)
(defvar canvas-3d--status)
(defvar canvas-3d--selection)
(defvar canvas-3d--question-target)
(defvar canvas-3d-selected-id)
(defvar canvas-3d-mode-map)
(defvar canvas-3d-selection-hook)
(defvar gnosis-model-backend-selection-hook)
(defvar gnosis-model-backend--implementation)

(defun gnosis-model-backend-canvas--value (symbol &optional default)
  "Return renderer SYMBOL's value, or DEFAULT before renderer loading."
  (if (boundp symbol) (symbol-value symbol) default))

(defun gnosis-model-backend-canvas-snapshot ()
  "Return the current buffer's canvas observation."
  (let* ((owner (gnosis-model-backend-canvas--value 'canvas-3d--process))
         (frame (gnosis-model-backend-canvas--value 'canvas-3d--frame))
         (live (and (process-live-p owner) t))
         (dirty (gnosis-model-backend-canvas--value 'canvas-3d--dirty)))
    (list :owner owner
          :attachment (gnosis-model-backend-canvas--value 'canvas-3d--image)
          :live live
          :ready (and live (eq (plist-get frame :owner) owner)
                      (not (gnosis-model-backend-canvas--value 'canvas-3d--busy))
                      (not dirty))
          :dirty dirty :frame (plist-get frame :seq)
          :view (mapcar (lambda (symbol)
                          (gnosis-model-backend-canvas--value symbol))
                        '(canvas-3d--yaw canvas-3d--pitch canvas-3d--zoom))
          :status (gnosis-model-backend-canvas--value 'canvas-3d--status "Starting")
          :selection (copy-tree (gnosis-model-backend-canvas--value 'canvas-3d--selection))
          :selected-id (gnosis-model-backend-canvas--value 'canvas-3d-selected-id)
          :question-target (copy-tree (gnosis-model-backend-canvas--value 'canvas-3d--question-target))
          :keymap (gnosis-model-backend-canvas--value 'canvas-3d-mode-map))))

(defun gnosis-model-backend-canvas-detach (owner attachment &optional _keep-picture)
  "Detach exactly OWNER's ATTACHMENT, including a stopped renderer.
The canvas always retains its last picture."
  (let ((process (gnosis-model-backend-canvas--value 'canvas-3d--process)))
    (when (and (or attachment (and owner process))
               (eq attachment (gnosis-model-backend-canvas--value 'canvas-3d--image))
               (or (null process) (eq owner process))
               (fboundp 'canvas-3d-detach))
      (canvas-3d-detach))))

(defun gnosis-model-backend-canvas-highlight (target selected-id)
  "Display TARGET and SELECTED-ID through canvas-3d."
  (setq-local canvas-3d--question-target (copy-tree target))
  (setq canvas-3d-selected-id selected-id)
  (canvas-3d--request))

(defun gnosis-model-backend-canvas-set-view (view)
  "Request camera VIEW through canvas-3d."
  (setq canvas-3d--yaw (nth 0 view) canvas-3d--pitch (nth 1 view)
        canvas-3d--zoom (nth 2 view))
  (canvas-3d--request))

(defun gnosis-model-backend-canvas-reveal (owner attachment reveal)
  "Reveal REVEAL's :target on exactly OWNER's ATTACHMENT, shrunk to :size.
The canvas renderer paints one highlight color, so the correct target
replaces any question highlight and the learner's selection; wrong picks,
cues and alternates are not painted.  Wait for the frame at most five
seconds.  Return non-nil if a frame was requested."
  (let ((size (plist-get reveal :size))
        (target (plist-get reveal :target))
        (process (gnosis-model-backend-canvas--value 'canvas-3d--process))
        requested)
    (when (and (process-live-p process) (eq owner process)
               (eq attachment (gnosis-model-backend-canvas--value 'canvas-3d--image)))
      (when target
        (setq-local canvas-3d--question-target (copy-tree target))
        (setq canvas-3d-selected-id nil requested t))
      (when (and size (< size canvas-3d--size))
        (setq canvas-3d--size size requested t))
      (when requested
        (canvas-3d--request)
        (let ((deadline (+ (float-time) 5)))
          (while (and (eq process canvas-3d--process) (process-live-p process)
                      (or canvas-3d--busy canvas-3d--dirty)
                      (< (float-time) deadline))
            (accept-process-output process 0.05))))
      requested)))

(defun gnosis-model-backend-canvas--selection (selection)
  "Forward canvas SELECTION without accepting an answer."
  (run-hook-with-args 'gnosis-model-backend-selection-hook selection))

(defconst gnosis-model-backend-canvas
  '(:open gnosis-model-backend-canvas-open
    :snapshot gnosis-model-backend-canvas-snapshot
    :detach gnosis-model-backend-canvas-detach
    :highlight gnosis-model-backend-canvas-highlight
    :set-view gnosis-model-backend-canvas-set-view
    :reveal gnosis-model-backend-canvas-reveal)
  "Dispatch operations of the optional canvas backend.")

(defun gnosis-model-backend-canvas-open (path view size inline target)
  "Open scene PATH at VIEW, SIZE, INLINE and display TARGET."
  (let* ((directory (or (gnosis-model--renderer-directory)
                        (user-error "The 3D model display is off in Romacs for now")))
         (load-path (cons directory load-path)))
    (when (file-remote-p directory)
      (user-error "The model renderer must be installed locally"))
    (unless (require 'canvas-3d nil t)
      (user-error "Model unavailable: no canvas-3d in gnosis-model-renderer-directory"))
    (let ((canvas-3d--directory directory))
      (unless (and (display-graphic-p) (fboundp 'canvas-refresh)
                   (image-type-available-p 'canvas))
        (user-error "3D requires graphical GNU Emacs built with native canvas image support; ordinary Gnosis does not"))
      (condition-case nil (canvas-3d--python)
        (user-error
         (user-error "Model renderer dependencies missing: run uv sync --locked --project %s (OpenGL/EGL required)"
                     (shell-quote-argument canvas-3d--directory))))
      (let ((buffer (if inline
                        (progn
                          (unless (fboundp 'canvas-3d-attach)
                            (user-error "Update the optional canvas backend for inline review"))
                          (canvas-3d-attach path "Gnosis model" view (or size 512)))
                      (canvas-3d-open path "Gnosis model" view (or size 512)))))
        (with-current-buffer buffer
          (let ((image canvas-3d--image) (process canvas-3d--process) ready)
            (unwind-protect
                (progn
                  (setq-local gnosis-model-backend--implementation gnosis-model-backend-canvas)
                  (add-hook 'canvas-3d-selection-hook
                            #'gnosis-model-backend-canvas--selection nil t)
                  (when target
                    (setq-local canvas-3d--question-target (copy-tree target))
                    (canvas-3d--request))
                  (setq ready t))
              (unless ready
                (gnosis-model-backend-canvas-detach process image)))))
        buffer))))

(provide 'gnosis-model-backend-canvas)
;;; gnosis-model-backend-canvas.el ends here
