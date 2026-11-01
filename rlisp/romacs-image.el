;;; romacs-image.el --- Image display support for Romacs -*- lexical-binding: t -*-

;; SPDX-License-Identifier: GPL-3.0-or-later
;; Maintainer: Thanos Apollo <public@thanosapollo.org>

;;; Commentary:

;; This package provides GPU-accelerated image display support for Romacs.
;; 
;; Basic usage:
;;   (romacs-image-show-file "/path/to/image.png")
;;
;; API functions:
;;   `romacs-image-load' - Load an image from file, returns image ID
;;   `romacs-image-size' - Get image dimensions as (width . height)
;;   `romacs-image-free' - Free an image from cache
;;   `romacs-image-floating' - Display image as floating layer
;;   `romacs-image-floating-clear' - Remove floating image layer

;;; Code:

(defvar romacs-image--cache (make-hash-table :test 'eq)
  "Hash table mapping image IDs to their metadata.")

(defun romacs-image-show-file (file &optional x y width height)
  "Display image FILE as a floating layer at X, Y with optional WIDTH, HEIGHT.
If WIDTH/HEIGHT are nil, uses the image's natural size.
X and Y default to 50 if not specified.
Returns the image ID on success, nil on failure."
  (interactive "fImage file: ")
  (let* ((path (expand-file-name file))
         (image-id (romacs-image-load path)))
    (when image-id
      (let* ((size (romacs-image-size image-id))
             (img-width (or width (car size)))
             (img-height (or height (cdr size)))
             (img-x (or x 50))
             (img-y (or y 50)))
        (puthash image-id `(:path ,path :width ,img-width :height ,img-height
                            :x ,img-x :y ,img-y)
                 romacs-image--cache)
        (romacs-image-floating image-id img-x img-y img-width img-height)
        (message "Showing image %d at (%d,%d) %dx%d" 
                 image-id img-x img-y img-width img-height)))
    image-id))

(defun romacs-image-hide (image-id)
  "Hide the floating image layer for IMAGE-ID and free it from cache."
  (interactive "nImage ID: ")
  (romacs-image-floating-clear image-id)
  (romacs-image-free image-id)
  (remhash image-id romacs-image--cache)
  (message "Image %d hidden and freed" image-id))

(defun romacs-image-show-scaled (file scale &optional x y)
  "Display image FILE scaled by SCALE factor at X, Y.
SCALE is a float where 1.0 = original size, 0.5 = half, 2.0 = double.
Returns the image ID on success."
  (interactive "fImage file: \nnScale factor: ")
  (let* ((path (expand-file-name file))
         (image-id (romacs-image-load path)))
    (when image-id
      (let* ((size (romacs-image-size image-id))
             (img-width (round (* (car size) scale)))
             (img-height (round (* (cdr size) scale)))
             (img-x (or x 50))
             (img-y (or y 50)))
        (puthash image-id `(:path ,path :width ,img-width :height ,img-height
                            :x ,img-x :y ,img-y :scale ,scale)
                 romacs-image--cache)
        (romacs-image-floating image-id img-x img-y img-width img-height)
        (message "Showing image %d (%.1fx) at (%d,%d) %dx%d" 
                 image-id scale img-x img-y img-width img-height)))
    image-id))

(defun romacs-image-info (image-id)
  "Get information about IMAGE-ID from the cache."
  (gethash image-id romacs-image--cache))

(defun romacs-image-list ()
  "List all loaded images."
  (interactive)
  (let ((images '()))
    (maphash (lambda (id info)
               (push (cons id info) images))
             romacs-image--cache)
    (if images
        (with-help-window "*Romacs Images*"
          (princ "Loaded Images:\n\n")
          (dolist (img (sort images (lambda (a b) (< (car a) (car b)))))
            (let ((id (car img))
                  (info (cdr img)))
              (princ (format "  ID %d: %s\n" id (plist-get info :path)))
              (princ (format "         Size: %dx%d at (%d,%d)\n"
                             (plist-get info :width)
                             (plist-get info :height)
                             (plist-get info :x)
                             (plist-get info :y))))))
      (message "No images loaded"))))

(defun romacs-image-free-all ()
  "Free all loaded images."
  (interactive)
  (maphash (lambda (id _info)
             (romacs-image-floating-clear id)
             (romacs-image-free id))
           romacs-image--cache)
  (clrhash romacs-image--cache)
  (message "All images freed"))

(defun romacs-image-move (image-id x y)
  "Move IMAGE-ID to new position X, Y while keeping its size."
  (interactive "nImage ID: \nnNew X: \nnNew Y: ")
  (let ((info (gethash image-id romacs-image--cache)))
    (when info
      (let ((width (plist-get info :width))
            (height (plist-get info :height)))
        (romacs-image-floating image-id x y width height)
        (puthash image-id (plist-put (plist-put info :x x) :y y)
                 romacs-image--cache)
        (message "Moved image %d to (%d,%d)" image-id x y)))))

(defun romacs-image-resize (image-id width height)
  "Resize IMAGE-ID to WIDTH, HEIGHT while keeping its position."
  (interactive "nImage ID: \nnNew width: \nnNew height: ")
  (let ((info (gethash image-id romacs-image--cache)))
    (when info
      (let ((x (plist-get info :x))
            (y (plist-get info :y)))
        (romacs-image-floating image-id x y width height)
        (puthash image-id (plist-put (plist-put info :width width) :height height)
                 romacs-image--cache)
        (message "Resized image %d to %dx%d" image-id width height)))))

;;; Computed SVG animation (Romacs extension).

;; GNU renders SVG through librsvg, which has no document clock, so an
;; animated SVG is a static frame there and `image-multi-frame-p' returns
;; nil.  Romacs can materialize the frames an SVG's SMIL timeline computes
;; and expose them exactly like a GIF's: `image-multi-frame-p' reports the
;; frame count and delay, `image-animate' walks them, `:index' selects one.
;; Because that changes observable behavior, it is strictly opt-in per
;; image spec.

(defcustom romacs-svg-animation nil
  "Default value for the `:animation' image property on SVG specs.
nil keeps GNU behavior: one static frame regardless of SMIL content.
t enables computed animation at the default sampling ceiling (30 fps);
an integer enables it with that ceiling, which also bounds how many
distinct frames one loop materializes.

Individual image specs override this default through their own
`:animation' property.  See docs/display-engine/ANIMATED_SVG.md."
  :type '(choice (const :tag "Static (GNU-compatible)" nil)
                 (const :tag "Animate at the default ceiling" t)
                 (integer :tag "Animate with an fps ceiling"))
  :group 'image
  :version "31.1")

(defun romacs-image-spec-add-animation (spec)
  "Return SPEC with the `:animation' policy from `romacs-svg-animation'.
Packages building SVG image specs can spread this in so the gate
follows the user's defcustom instead of hard-coding either behavior."
  (if (and romacs-svg-animation
           (not (plist-member (cdr spec) :animation)))
      (append spec (list :animation romacs-svg-animation))
    spec))

(defun romacs-image-animate-svg (image &optional limit)
  "Animate the SVG image IMAGE, in place, if it carries SMIL animation.
IMAGE is an image spec as displayed in a buffer (`image--get-image' gives
you one).  `image-animate' advances frames by mutating the very list it
is handed — its timer `plist-put's `:index' on IMAGE — so IMAGE is
modified in place rather than copied: the frames walked are the frames
the window shows.  The `:animation' property is added from
`romacs-svg-animation' only when the spec does not carry its own.
Returns whatever `image-animate' returns: nil when the image has no
materializable animation, so callers can fall back to a static display.
LIMIT is passed through to `image-animate'."
  (when (and romacs-svg-animation
             (not (plist-member (cdr image) :animation)))
    (setcdr image (plist-put (cdr image) :animation romacs-svg-animation)))
  (image-animate image nil limit))

(provide 'romacs-image)
;;; romacs-image.el ends here
