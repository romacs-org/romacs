;;; gnosis-model-backend.el --- Model display boundary -*- lexical-binding: t; -*-

;; Copyright (C) 2026 Free Software Foundation, Inc.
;; SPDX-License-Identifier: GPL-3.0-or-later
;; Maintainer: Thanos Apollo <public@thanosapollo.org>

;;; Commentary:
;; A backend is a plist of five functions plus optional :reveal, not a
;; class or a registry.
;; :open takes (PATH VIEW SIZE INLINE TARGET) and returns the display buffer;
;; :snapshot takes no arguments and returns the current buffer's observation;
;; :detach takes (OWNER ATTACHMENT &optional KEEP-PICTURE) and refuses a successor;
;; :highlight takes (TARGET SELECTED-ID); :set-view takes VIEW,
;; a (YAW PITCH ZOOM) list.  The optional :reveal takes (OWNER ATTACHMENT
;; REVEAL); see `gnosis-model-backend-reveal'.  The optional :cue takes a
;; list of cue TARGETS; see `gnosis-model-backend-cue'.
;; TARGET is a Gnosis target alist; it describes display geometry, not grading.
;; Snapshot keys: :owner, :attachment (opaque identities), :live, :ready,
;; :dirty (pick delivery is outdated), :frame, :view, :status, :selection,
;; :selected-id, :question-target, :keymap, and optionally :size (the
;; square picture side in pixels).  Selection carries :owner and
;; :frame plus :mesh, :face and original-coordinate :point.  Picks never
;; submit.  Domain code retains encounter authority and target matching.

;;; Code:

(require 'gnosis-model-backend-canvas)

(defcustom gnosis-model-backend 'auto
  "Choose the model renderer.
Auto prefers Romacs's native GPU model view while it is available,
otherwise the optional canvas-3d renderer when one is configured.
Forcing a backend refuses unavailable rendering rather than falling back."
  :type '(choice (const auto) (const romacs) (const canvas-3d))
  :group 'gnosis)

(defvar-local gnosis-model-backend--implementation nil
  "Dispatch plist of the current attachment.")

(defvar-local gnosis-model-backend-selection-hook nil
  "Hook receiving an owner-qualified surface selection, without submitting.")

(defvar gnosis-model-backend-romacs)
(declare-function gnosis-model-backend-romacs-unavailable-reason "gnosis-model-backend-romacs" ())
(declare-function gnosis-model--renderer-directory "gnosis-model" ())

(defun gnosis-model-backend--choose ()
  "Return the requested available backend's dispatch plist."
  (require 'gnosis-model-backend-romacs)
  (let ((native (gnosis-model-backend-romacs-unavailable-reason)))
    (pcase gnosis-model-backend
      ('romacs
       (when native (user-error "Native model renderer unavailable: %s" native))
       gnosis-model-backend-romacs)
      ('canvas-3d gnosis-model-backend-canvas)
      (_ (cond ((not native) gnosis-model-backend-romacs)
               ((gnosis-model--renderer-directory) gnosis-model-backend-canvas)
               (t (user-error "The 3D model display is off in Romacs: %s; text cards still work"
                              native)))))))

(defun gnosis-model-backend-open (path view size inline target)
  "Open PATH at VIEW with SIZE, INLINE and display TARGET."
  (funcall (plist-get (gnosis-model-backend--choose) :open)
           path view size inline target))

(defun gnosis-model-backend-snapshot ()
  "Return the current attachment's backend-neutral observation."
  (funcall (plist-get (or gnosis-model-backend--implementation
                         gnosis-model-backend-canvas) :snapshot)))

(defun gnosis-model-backend-get (key)
  "Return KEY from the current backend observation."
  (plist-get (gnosis-model-backend-snapshot) key))

(defun gnosis-model-backend-detach (owner attachment &optional keep-picture)
  "Retire exactly OWNER's ATTACHMENT, never a successor.
KEEP-PICTURE retains the final displayed picture for feedback."
  (let ((detach (plist-get (or gnosis-model-backend--implementation
                               gnosis-model-backend-canvas) :detach)))
    (if keep-picture
        (funcall detach owner attachment t)
      (funcall detach owner attachment))))

(defun gnosis-model-backend-highlight (target selected-id)
  "Display TARGET and SELECTED-ID on the current attachment."
  (funcall (plist-get (or gnosis-model-backend--implementation
                         gnosis-model-backend-canvas) :highlight) target selected-id))

(defun gnosis-model-backend-cue (targets)
  "Show cue TARGETS in `gnosis-model-cue-color' beneath later picks.
TARGETS is a list of Gnosis target alists, or nil to remove cues.  Cues
are display only: unlike a question target they never stop picks from
painting.  Return non-nil if the backend painted them; a backend without
the optional :cue operation, or without colored layers, returns nil."
  (when-let* ((cue (plist-get (or gnosis-model-backend--implementation
                                  gnosis-model-backend-canvas)
                              :cue)))
    (funcall cue targets)))

(defun gnosis-model-backend-set-view (view)
  "Request camera VIEW on the current attachment."
  (funcall (plist-get (or gnosis-model-backend--implementation
                         gnosis-model-backend-canvas) :set-view) view))

(defcustom gnosis-model-correct-color "#2fbf4f"
  "Color of the correct target revealed on a model after answering."
  :type 'color
  :group 'gnosis)

(defcustom gnosis-model-wrong-color "#e0403a"
  "Color of a wrongly picked structure revealed on a model after answering."
  :type 'color
  :group 'gnosis)

(defcustom gnosis-model-cue-color "#4f8fe0"
  "Color of cue targets on a relational model question and its feedback.
Neutral on purpose: distinct from the pick and the correct/wrong colors."
  :type 'color
  :group 'gnosis)

(defcustom gnosis-model-pick-color "#f2b630"
  "Color of the learner's picked spot on a point or region Find."
  :type 'color
  :group 'gnosis)

(defvar-local gnosis-model-backend-pick-radius nil
  "Scene framing radius while picks mark a spot, or nil to mark meshes.
Point and region Finds set it so a pick shows where it landed, not the
whole picked mesh, on backends that paint colored highlight layers.")

(defcustom gnosis-model-reveal-point-scale 0.06
  "Radius of revealed point markers as a fraction of the model's radius.
Points are the correct landmark of a point target and a wrong pick on a
point or region target.  Display only; grading tolerances are unchanged."
  :type 'number
  :group 'gnosis)

(defcustom gnosis-model-reveal-point-pixels 10
  "Smallest on-screen radius in pixels of a revealed point marker."
  :type 'natnum
  :group 'gnosis)

(defun gnosis-model-backend-marker-radius (radius zoom size)
  "Return the revealed point radius in model units, or nil without RADIUS.
RADIUS is the scene's framing radius, ZOOM the camera zoom (default 1)
and SIZE the picture side in pixels, or nil if unknown.  Use
`gnosis-model-reveal-point-scale' of RADIUS, enlarged so the marker spans
at least `gnosis-model-reveal-point-pixels' at SIZE.  The renderers draw
the scene radius across 0.4 * ZOOM * SIZE pixels."
  (when (and (numberp radius) (< 0 radius 1.0e+INF))
    (let ((zoom (if (and (numberp zoom) (< 0 zoom 1.0e+INF)) zoom 1)))
      (* radius (max gnosis-model-reveal-point-scale
                     (if (and (numberp size) (< 0 size))
                         (/ gnosis-model-reveal-point-pixels (* 0.4 zoom size))
                       0))))))

(defun gnosis-model-backend-reveal (owner attachment reveal)
  "Reveal an answer on exactly OWNER's ATTACHMENT before it is retained.
REVEAL is a plist: :target, the correct Gnosis target alist; optional
:alternates, the other acceptable targets, and :cues, the cue targets of
a relational card; :wrong-mesh, the picked mesh ID of a wrong object Find
or nil; :wrong-pick, the learner's surface hit (:mesh ID :face N
:barycentric (A B C)) of a wrong point or region Find, or nil;
:marker-radius, the display radius in model units of point markers, or
nil; :size, the largest square picture side in pixels that fits the
feedback, or nil.

Backends paint the target and alternates in `gnosis-model-correct-color'
on top, cues in `gnosis-model-cue-color' at the bottom and, between them,
the wrong mesh or a point of :marker-radius at the wrong pick in
`gnosis-model-wrong-color', when they can; with one highlight color, only
the target.  A point target shows at least as large as :marker-radius.
Without a usable :marker-radius a point target keeps its authored
tolerance and no wrong-pick marker shows.  Backends shrink the picture to
:size and present the result before returning.  The optional :reveal
operation receives REVEAL with :correct-color, :wrong-color and
:cue-color added; a backend without it keeps its picture.  Return non-nil
if the backend handled it."
  (when-let* ((reveal-function (plist-get (or gnosis-model-backend--implementation
                                              gnosis-model-backend-canvas)
                                          :reveal)))
    (funcall reveal-function owner attachment
             (append (list :correct-color gnosis-model-correct-color
                           :wrong-color gnosis-model-wrong-color)
                     (and (plist-get reveal :cues)
                          (list :cue-color gnosis-model-cue-color))
                     reveal))))

(defun gnosis-model-backend-selection-current-p (selection &optional settled)
  "Return non-nil if SELECTION names the owned current picture.
SETTLED also requires that the latest renderer request has completed."
  (let ((state (gnosis-model-backend-snapshot)))
    (and (plist-get state :live)
         (or (not settled) (plist-get state :ready))
         (not (plist-get state :dirty))
         (eq (plist-get selection :owner) (plist-get state :owner))
         (equal (plist-get selection :frame) (plist-get state :frame)))))

(provide 'gnosis-model-backend)
;;; gnosis-model-backend.el ends here
