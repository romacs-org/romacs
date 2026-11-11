;;; romacs-surface.el --- Shader surfaces: GPU textures from Lisp -*- lexical-binding: t -*-

;; SPDX-License-Identifier: GPL-3.0-or-later
;; Maintainer: Thanos Apollo <public@thanosapollo.org>

;;; Commentary:

;; Romacs extension (experimental): compositor-rendered GPU textures as
;; inline display objects.  See docs/display-engine/SHADER_SURFACES.md.
;;
;;   (romacs-surface-create :width W :height H
;;                           :shader WGSL &optional :uniforms ALIST :animate BOOL
;;                           :fps N)   ; :fps caps the animation rate (battery)
;;   (romacs-surface-create :width W :height H :pixels RGBA-UNIBYTE-STRING)
;;     => surface handle (opaque, GC-managed); signals on WGSL compile
;;        errors.  Dropping the handle frees the GPU objects at the next
;;        garbage collection; `romacs-surface-destroy' frees them now.
;;
;; Declarative form (no create call, like image/video specs — the spec
;; content is memoized into a surface; WGSL errors are logged, not signaled):
;;
;;   (insert (propertize " " 'display
;;            '(surface :shader "fn mainImage(...) ..."
;;                      :uniforms ((speed . 2.0))
;;                      :width 320 :height 120)))
;;
;; The shader defines `fn mainImage(fragCoord: vec2<f32>) -> vec4<f32>' and
;; reads Shadertoy-style uniforms `u.iTime', `u.iResolution', plus one
;; generated accessor per :uniforms entry ((speed . 2.0) => `u_speed()').
;;
;; Display a surface with `romacs-surface-insert', or manually via the
;; display property (surface :id ID :width W :height H).
;;
;; Gate uses on (featurep 'romacs-surface) and
;; (romacs-surface-available-p).

;;; Code:

(defun romacs-surface--report-create-error (id error)
  "Default handler for `romacs-surface-error-functions': message ID and ERROR."
  (message "romacs shader surface %s failed to build: %s" id error))

(defvar romacs-surface-error-functions
  (list #'romacs-surface--report-create-error)
  "Abnormal hook run when a shader surface fails to build on the GPU.
Each function is called with two arguments, the surface id and an error
string.  This fires only for the rare case where the render thread's
wgpu pipeline build fails AFTER the synchronous naga validation in
`romacs-surface-create' already accepted the shader (e.g. a
device-specific limit); ordinary shader syntax errors are signaled
synchronously from `romacs-surface-create' instead.  The default member
reports it with `message'.")

(defun romacs-surface--report-frame-shader-error (error)
  "Default handler for `romacs-frame-shader-error-functions'."
  (message "romacs frame shader failed to build: %s" error))

(defvar romacs-frame-shader-error-functions
  (list #'romacs-surface--report-frame-shader-error)
  "Abnormal hook run when a full-frame shader fails to build on the GPU.
Each function is called with the renderer's error string.  Portable syntax
and validation errors are signaled synchronously by `romacs-frame-shader';
this hook covers a later rejection by the active wgpu device or quality
policy.")

(defun romacs-surface-insert (id width height)
  "Insert surface ID at point as a WIDTH x HEIGHT display object."
  (insert (propertize " " 'display (list 'surface :id id :width width :height height)
                      'romacs-surface-id id))
  id)

(defun romacs-surface-attach (id &optional buffer)
  "Tie surface ID's lifetime to BUFFER, defaulting to the current buffer.
Adds a buffer-local `kill-buffer-hook' that destroys ID when BUFFER is
killed (errors are ignored, so an id already freed by an explicit
`romacs-surface-destroy' is harmless).  Returns ID."
  (with-current-buffer (or buffer (current-buffer))
    (add-hook 'kill-buffer-hook
              (lambda () (ignore-errors (romacs-surface-destroy id)))
              nil t))
  id)

(defun romacs-surface-create-and-insert (&rest args)
  "Create a surface from ARGS (see `romacs-surface-create') and insert it.
The surface is attached to the current buffer (`romacs-surface-attach'),
so killing the buffer frees it.  Returns the surface id."
  (let ((id (apply #'romacs-surface-create args)))
    (romacs-surface-insert id (plist-get args :width) (plist-get args :height))
    (romacs-surface-attach id)))

(defconst romacs-surface--demo-shader
  "fn mainImage(fragCoord: vec2<f32>) -> vec4<f32> {
    let uv = fragCoord / u.iResolution.xy;
    let speed = u_speed();
    let col = 0.5 + 0.5 * cos(u.iTime * speed + uv.xyx * 6.2832 + vec3<f32>(0.0, 2.0, 4.0));
    return vec4<f32>(col, 1.0);
}"
  "Shadertoy-style plasma used by `romacs-surface-demo'.")

(defun romacs-surface--demo-checkerboard (size cell)
  "Return RGBA bytes for a SIZE x SIZE checkerboard with CELL-pixel squares."
  (let ((data (make-string (* size size 4) 0)))
    (dotimes (y size)
      (dotimes (x size)
        (let* ((i (* 4 (+ x (* y size))))
               (on (zerop (mod (+ (/ x cell) (/ y cell)) 2)))
               (v (if on 230 40)))
          (aset data i v)
          (aset data (+ i 1) (if on 120 40))
          (aset data (+ i 2) (if on 40 230))
          (aset data (+ i 3) 255))))
    data))

(defun romacs-surface-demo ()
  "Pop a buffer showing an animated shader surface and a pixel surface.
Every surface is attached to the demo buffer (`romacs-surface-attach'),
so killing *surface-demo* frees them all."
  (interactive)
  (unless (romacs-surface-available-p)
    (error "Shader surfaces need the Romacs GUI"))
  (with-current-buffer (get-buffer-create "*surface-demo*")
    (erase-buffer)
    (insert "Animated WGSL plasma (compositor clock, zero Lisp per frame):\n\n  ")
    (romacs-surface-create-and-insert
     :width 320 :height 120
     :shader romacs-surface--demo-shader
     :uniforms '((speed . 1.5))
     :animate t)
    (insert "\n\nStatic RGBA pixels uploaded from Lisp:\n\n  ")
    (romacs-surface-create-and-insert
     :width 96 :height 96
     :pixels (romacs-surface--demo-checkerboard 96 12))
    (insert "\n\nDeclarative display spec (no create call, spec is the identity):\n\n  ")
    (insert (propertize " " 'display
                        (list 'surface
                              :shader romacs-surface--demo-shader
                              :uniforms '((speed . 4.0))
                              :width 200 :height 80)))
    (insert "\n\nChannel input: a shader warping the pixel surface above (:channel0):\n\n  ")
    ;; The channel source is created without inserting it, so attach it to
    ;; the demo buffer explicitly (create-and-insert attaches automatically).
    (let ((source (romacs-surface-attach
                   (romacs-surface-create
                    :width 96 :height 96
                    :pixels (romacs-surface--demo-checkerboard 96 12)))))
      (romacs-surface-create-and-insert
       :width 240 :height 96
       :channel0 source
       :shader "fn mainImage(fragCoord: vec2<f32>) -> vec4<f32> {
    var uv = fragCoord / u.iResolution.xy;
    uv.x += 0.06 * sin(u.iTime * 2.0 + uv.y * 12.0);
    uv.y += 0.06 * cos(u.iTime * 1.7 + uv.x * 12.0);
    return textureSample(iChannel0, iChannel0Sampler, uv);
}"))
    (insert "\n")
    (goto-char (point-min))
    (pop-to-buffer (current-buffer))))

(provide 'romacs-surface)
;;; romacs-surface.el ends here
