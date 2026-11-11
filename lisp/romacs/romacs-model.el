;;; romacs-model.el --- Native 3D model views -*- lexical-binding: t -*-

;; SPDX-License-Identifier: GPL-3.0-or-later
;; Maintainer: Thanos Apollo <public@thanosapollo.org>

;;; Commentary:

;; Romacs extension (experimental): OBJ scenes drawn natively by the GPU
;; renderer inside an ordinary buffer.  A MODEL is an opaque handle owning
;; one native surface: collecting the handle frees the surface (a
;; finalizer), as does `romacs-model-destroy'.  It is shown by a
;; `(surface :id ID ...)' display spec and loaded asynchronously.
;;
;;   (setq m (romacs-model-create
;;            :meshes '(("humerus" "/data/humerus.obj" SHA256-OR-NIL) ...)
;;            :view '(0 -90 1.1) :callback #'my-model-callback))
;;   (romacs-model-insert m)                 ; glyph at point
;;   (romacs-model-open MESHES :name "Arm")  ; standalone buffer
;;
;; Requests (`romacs-model-set-view', `-set-highlight', `-resize',
;; `-freeze') return a request number.  `romacs-model-current-p' is non-nil
;; once the renderer has settled the latest request and presents the latest
;; picture in every window showing MODEL.  The callback is called with
;; (MODEL EVENT PLIST) for the events `ready', `failed', `error', `view',
;; `readiness' and `pick'; a `pick' is delivered only when it names the
;; picture Lisp holds as current when the callback runs, with its hit
;; mesh given by ID.
;;
;; `romacs-model-set-highlight' paints either one target (or a selected
;; mesh) in the highlight colour, or up to
;; `romacs-model-max-highlight-layers' coloured :layers at once.  Clients
;; detect layer support with
;;   (and (fboundp 'romacs-model-capabilities)
;;        (memq 'highlight-layers (romacs-model-capabilities)))
;;
;; Left-dragging a model orbits it and a click picks; the wheel zooms.
;; Mesh IDs never reach the renderer: they are mapped to indices here.
;;
;; The renderer's events arrive as special events `(romacs-model ID TYPE
;; PLIST)', which `special-event-map' runs at once, inside a read too
;; (`romacs-model--handle-event').

;;; Code:

(require 'cl-lib)
(require 'color)

(declare-function romacs-model--available-p "prims/model.rs" ())
(declare-function romacs-model--create "prims/model.rs"
                  (meshes view background width height))
(declare-function romacs-model--set-view "prims/model.rs"
                  (id req yaw pitch zoom))
(declare-function romacs-model--set-highlight "prims/model.rs"
                  (id req target selected))
(declare-function romacs-model--set-highlight-layers "prims/model.rs"
                  (id req layers))
(declare-function romacs-model--resize "prims/model.rs" (id req width height))
(declare-function romacs-model--freeze "prims/model.rs" (id req))
(declare-function romacs-surface-destroy "prims/model.rs" (id))

(defgroup romacs-model nil
  "Native 3D model views."
  :group 'multimedia
  :prefix "romacs-model-")

(defvar romacs-model-event-functions nil
  "Abnormal hook run for each native model event.
Each function is called with (ID TYPE PLIST): the integer surface
id, the event symbol and its plist.  Events are delivered in order
and never nested: one arriving while a member runs (for example
inside its `sit-for') waits until every member has handled the
current one.  A member's error is logged and the others still run.
`romacs-model--dispatch' is the member that serves
`romacs-model-create' callbacks.")

(defvar romacs-model--state-functions nil
  "Internal hook run with (ID TYPE PLIST) as soon as an event arrives.
Unlike `romacs-model-event-functions' it also runs while a member
of that hook is active, so input read afterwards always sees the
event's effect on the model state.  Members must only record
state: never call user code, wait or read input.")

;;;; Registry

(cl-defstruct (romacs-model--entry
               (:constructor romacs-model--entry-create)
               (:copier nil))
  "Lisp-side state of one model (design §6.3)."
  id callback buffer
  (status '(loading))
  ;; Vector of mesh ID strings, in renderer index order.
  mesh-ids
  view initial-view
  ;; Size the glyph shows: the latest pending resize, else the accepted one.
  width height
  ;; Size of the last resize the renderer applied, (WIDTH HEIGHT).
  accepted-size
  ;; Resizes not yet settled, ((REQ WIDTH HEIGHT) ...), oldest first.
  resizes
  ;; Latest request number sent to the renderer.
  (issued 0)
  ;; Last `readiness' as (CURRENT REV REQ), or nil.
  readiness
  ;; A requested camera stays authoritative until its request is settled;
  ;; older interaction events may still be queued when Lisp issues it.
  pending-view
  frozen)

(cl-defstruct (romacs-model--handle
               (:constructor romacs-model--handle-create (id finalizer))
               (:copier nil))
  "A model handle: its native surface ID, freed by FINALIZER when the
handle is collected."
  id finalizer)

(defvar romacs-model--registry (make-hash-table :test 'eq :weakness 'key)
  "MODEL handle -> `romacs-model--entry'.
Weak in the key, so an unreferenced model is collected even when
its callback closes over it; collection frees its GPU objects.")

(defvar romacs-model--by-id (make-hash-table :test 'eql :weakness 'value)
  "Surface id -> live MODEL handle, for events, which name ids.")

(defun romacs-model--entry (model)
  "Return MODEL's registry entry, or signal."
  (or (gethash model romacs-model--registry)
      (signal 'wrong-type-argument (list 'romacs-model model))))

(defun romacs-model--find (id)
  "Return the live MODEL handle whose surface id is ID, or nil."
  (gethash id romacs-model--by-id))

(defun romacs-model--id (model)
  "Return MODEL's surface id."
  (romacs-model--handle-id model))

(defun romacs-model--mesh-index (entry mesh-id)
  "Return ENTRY's renderer index of MESH-ID, or signal."
  (or (cl-position mesh-id (romacs-model--entry-mesh-ids entry)
                   :test #'equal)
      (error "Unknown model mesh %S" mesh-id)))

;;;; Creation

;;;###autoload
(defun romacs-model-available-p ()
  "Return non-nil if a renderer that can draw native models is live now."
  (and (fboundp 'romacs-model--available-p)
       (romacs-model--available-p)))

(defun romacs-model--rgb (color)
  "Return colour name COLOR as three numbers in 0..1, or nil."
  (let ((rgb (ignore-errors (color-name-to-rgb color))))
    (and (proper-list-p rgb)
         (= (length rgb) 3)
         (cl-every (lambda (c) (and (numberp c) (<= 0 c 1))) rgb)
         rgb)))

(defun romacs-model--background (color)
  "Return COLOR as three numbers in 0..1.
COLOR is a colour name, a list or vector of three numbers, or nil
for the `default' face background (black when it has none)."
  (cond
   ((null color)
    (or (romacs-model--rgb (face-background 'default nil t)) (list 0 0 0)))
   ((stringp color)
    (or (romacs-model--rgb color) (error "Unknown model background %S" color)))
   (t color)))

;;;###autoload
(defun romacs-model-create (&rest plist)
  "Create a native model from PLIST and return it.
Keys:
  :meshes ((ID PATH [SHA256]) ...)  required, 1..255 meshes with
            unique string IDs and absolute .obj PATHs; SHA256 is the
            lowercase hex digest the loader checks.
  :width, :height  logical pixels, 32..2048 (default 512).
  :view (YAW PITCH ZOOM)  initial camera, a list or vector
            (default (0 0 1)).
  :background  colour name or (R G B) in 0..1 (default: the
            `default' face background).
  :callback FUNCTION  called with (MODEL EVENT PLIST), see Commentary.
Loading is asynchronous: file, digest, parse and GPU failures arrive
as a `failed' event.  Signals a `user-error' without a graphical
renderer."
  (let* ((meshes (plist-get plist :meshes))
         (ids (mapcar #'car meshes))
         ;; Kept as a list: `romacs-model-view' and the camera keys
         ;; take one apart with `nth'.
         (view (let ((view (or (plist-get plist :view) (list 0 0 1))))
                 (if (vectorp view) (append view nil) view)))
         (width (or (plist-get plist :width) 512))
         (height (or (plist-get plist :height) 512)))
    (unless (and (consp meshes) (cl-every #'consp meshes)
                 (cl-every #'stringp ids))
      (error "Model :meshes must be ((ID PATH [SHA256]) ...)"))
    (when (/= (length ids) (length (delete-dups (copy-sequence ids))))
      (error "Model mesh IDs must be unique"))
    (let* ((id (romacs-model--create
                (mapcar (lambda (mesh) (list (nth 1 mesh) (nth 2 mesh)))
                        meshes)
                view
                (append (romacs-model--background (plist-get plist :background)) nil)
                width height))
           ;; The finalizer names the id only, so it never keeps the handle.
           (model (romacs-model--handle-create
                   id (make-finalizer (lambda () (romacs-surface-destroy id))))))
      (puthash id model romacs-model--by-id)
      (puthash model
               (romacs-model--entry-create
                :id (romacs-model--id model)
                :callback (plist-get plist :callback)
                :mesh-ids (vconcat ids)
                :view (copy-sequence view)
                :initial-view (copy-sequence view)
                :width width :height height
                :accepted-size (list width height))
               romacs-model--registry)
      model)))

;;;; Requests

(defun romacs-model--request (model send)
  "Send a request to MODEL with SEND, called with the request number.
Return the request number once SEND succeeds."
  (let* ((entry (romacs-model--entry model))
         (req (1+ (romacs-model--entry-issued entry))))
    (when (eq (car (romacs-model--entry-status entry)) 'destroyed)
      (error "Model is destroyed"))
    (when (romacs-model--entry-frozen entry)
      (user-error "Model is frozen"))
    (funcall send req)
    ;; Raising `issued' makes `romacs-model-current-p' nil at once.
    (setf (romacs-model--entry-issued entry) req)
    req))

(defun romacs-model--wrap-angle (angle)
  "Return ANGLE (degrees) in 0..360, as the renderer's views report it.
An angle the renderer refuses (not a number, not finite or beyond 1e6
in magnitude) is returned unchanged, to be refused."
  (if (and (numberp angle) (<= (abs angle) 1e6))
      (mod angle 360)
    angle))

(defun romacs-model-set-view (model yaw pitch zoom)
  "Request camera YAW PITCH (degrees) and ZOOM (0.25..4) for MODEL.
The angles are kept wrapped into 0..360.  Return the request number."
  (let* ((entry (romacs-model--entry model))
         (yaw (romacs-model--wrap-angle yaw))
         (pitch (romacs-model--wrap-angle pitch))
         (req (romacs-model--request
               model (lambda (req)
                       (romacs-model--set-view (romacs-model--id model)
                                               req yaw pitch zoom)))))
    (setf (romacs-model--entry-view entry) (list yaw pitch zoom)
          (romacs-model--entry-pending-view entry) req)
    req))

(defun romacs-model--target (entry spec)
  "Return highlight SPEC with its mesh ID replaced by ENTRY's index."
  (when spec
    (let ((copy (copy-sequence spec)))
      (plist-put copy :mesh
                 (romacs-model--mesh-index entry (plist-get spec :mesh))))))

(defconst romacs-model-max-highlight-layers 8
  "Largest number of :layers one `romacs-model-set-highlight' paints.")

;;;###autoload
(defun romacs-model-capabilities ()
  "Return a list of the optional native model features available.
Members:
  `highlight-layers'  `romacs-model-set-highlight' accepts :layers."
  (and (fboundp 'romacs-model--set-highlight-layers)
       (list 'highlight-layers)))

(defun romacs-model--color-rgb (color)
  "Return colour string COLOR as three numbers in 0..1, or nil.
Numeric specs such as \"#00ff00\" are exact on any frame; names go
through `color-name-to-rgb' and so depend on the selected frame."
  (or (let ((values (ignore-errors (color-values-from-color-spec color))))
        (and values (mapcar (lambda (c) (/ c 65535.0)) values)))
      (romacs-model--rgb color)))

(defun romacs-model--layer (entry layer)
  "Return highlight LAYER for the renderer: (TARGET R G B).
ENTRY is the model's registry entry; LAYER is (:target SPEC :color
COLOR) or (:mesh ID :color COLOR)."
  (unless (and (proper-list-p layer) (cl-evenp (length layer)))
    (error "Model highlight layer must be a plist: %S" layer))
  (let ((target (plist-get layer :target))
        (mesh (plist-get layer :mesh))
        (color (plist-get layer :color)))
    (unless (and (stringp color) (romacs-model--color-rgb color))
      (error "Unknown model highlight layer color %S" color))
    ;; By key, not value: (:mesh "a" :target nil ...) names both.
    (unless (and (xor (plist-member layer :target) (plist-member layer :mesh))
                 (or target mesh))
      (error "Model highlight layer needs exactly one of :target and :mesh: %S"
             layer))
    (cons (romacs-model--target
           entry (or target (list :mesh mesh :kind 'object)))
          (romacs-model--color-rgb color))))

(defun romacs-model-set-highlight (model &rest plist)
  "Replace MODEL's highlight with PLIST; return the request number.
:target SPEC or nil, SPEC one of
  (:mesh ID :kind object)
  (:mesh ID :kind point :face N :barycentric [A B C] :tolerance R)
  (:mesh ID :kind region :faces (N ...))
:selected MESH-ID or nil, painted only when there is no target.
:layers LAYERS, instead of :target and :selected: a list of at most
  `romacs-model-max-highlight-layers' plists, each either
  (:target SPEC :color COLOR) or (:mesh ID :color COLOR), COLOR a
  colour name.  They are painted together, a later layer winning
  where layers overlap; nil clears the highlight.
Target and selection use the highlight colour.  Highlights never
change what a click picks."
  (let* ((entry (romacs-model--entry model))
         (keys (cl-loop for key in plist by #'cddr collect key))
         (send
          (if (memq :layers keys)
              (let ((layers (plist-get plist :layers)))
                (unless (cl-evenp (length plist))
                  (error "Model :layers needs KEYWORD VALUE pairs: %S" plist))
                (when (or (memq :target keys) (memq :selected keys))
                  (error "Model :layers cannot be combined with :target or :selected"))
                (unless (proper-list-p layers)
                  (error "Model :layers must be a list"))
                (when (> (length layers) romacs-model-max-highlight-layers)
                  (error "At most %d model highlight layers, got %d"
                         romacs-model-max-highlight-layers (length layers)))
                (let ((layers (mapcar (lambda (layer)
                                        (romacs-model--layer entry layer))
                                      layers)))
                  (lambda (req)
                    (romacs-model--set-highlight-layers (romacs-model--id model) req layers))))
            (let ((target (romacs-model--target entry (plist-get plist :target)))
                  (selected (let ((id (plist-get plist :selected)))
                              (and id (romacs-model--mesh-index entry id)))))
              (lambda (req)
                (romacs-model--set-highlight (romacs-model--id model) req target selected))))))
    (romacs-model--request model send)))

(defun romacs-model-resize (model width height)
  "Resize MODEL to WIDTH x HEIGHT logical pixels (32..2048).
Return the request number.  The glyph inserted by
`romacs-model-insert' or `romacs-model-open' follows at once, and
returns to the previous size if the renderer refuses the request."
  (let ((req (romacs-model--request
              model (lambda (req)
                      (romacs-model--resize (romacs-model--id model) req width height))))
        (entry (romacs-model--entry model)))
    (setf (romacs-model--entry-resizes entry)
          (nconc (romacs-model--entry-resizes entry)
                 (list (list req width height))))
    (romacs-model--show-size model entry)
    req))

(defun romacs-model--show-size (model entry)
  "Give MODEL's glyphs ENTRY's latest pending size, else its accepted one."
  (let ((size (or (cdr (car (last (romacs-model--entry-resizes entry))))
                  (romacs-model--entry-accepted-size entry))))
    (unless (and (eql (car size) (romacs-model--entry-width entry))
                 (eql (cadr size) (romacs-model--entry-height entry)))
      (setf (romacs-model--entry-width entry) (car size)
            (romacs-model--entry-height entry) (cadr size))
      (romacs-model--update-glyphs model entry))))

(defun romacs-model--settle-resizes (entry settled)
  "Accept ENTRY's pending resizes up to request SETTLED.
The renderer settles requests in order, so each of them it did not
refuse (see `romacs-model--refuse-resize') was applied."
  (let (applied)
    (while (and (romacs-model--entry-resizes entry)
                (<= (car (car (romacs-model--entry-resizes entry))) settled))
      (setq applied (pop (romacs-model--entry-resizes entry))))
    (when applied
      (setf (romacs-model--entry-accepted-size entry) (cdr applied)))))

(defun romacs-model--refuse-resize (model entry req)
  "Forget the pending resize REQ in MODEL's ENTRY: the renderer refused it.
The glyph shows the newest resize still pending, else the accepted size."
  (let ((resize (assq req (romacs-model--entry-resizes entry))))
    (when resize
      (setf (romacs-model--entry-resizes entry)
            (delq resize (romacs-model--entry-resizes entry)))
      (romacs-model--show-size model entry))))

(defun romacs-model-freeze (model)
  "Keep MODEL's last picture and stop interaction and requests.
Irreversible; `romacs-model-destroy' still works.  Return the
request number."
  (prog1 (romacs-model--request
          model (lambda (req) (romacs-model--freeze (romacs-model--id model) req)))
    (setf (romacs-model--entry-frozen (romacs-model--entry model)) t)))

;;;; Queries

(defun romacs-model-view (model)
  "Return the latest camera of MODEL known to Lisp, (YAW PITCH ZOOM)."
  (copy-sequence (romacs-model--entry-view (romacs-model--entry model))))

(defun romacs-model-status (model)
  "Return MODEL's status.
One of (loading), (ready :triangles (N ...) :meshes (ID ...)),
\(failed :error STRING) or (destroyed)."
  (copy-tree (romacs-model--entry-status (romacs-model--entry model))))

(defun romacs-model-current-p (model)
  "Return non-nil if MODEL shows its latest content everywhere.
That is: loaded, not frozen, a renderer live, no interaction in
progress, and the renderer's last readiness settles the latest
request and reports the latest picture presented in every window
showing MODEL."
  (let ((entry (romacs-model--entry model)))
    ;; A lost renderer presents nothing: its last readiness is void even
    ;; after a replacement starts, until that one reports again.
    (unless (romacs-model-available-p)
      (setf (romacs-model--entry-readiness entry) nil))
    (let ((readiness (romacs-model--entry-readiness entry)))
      (and (eq (car (romacs-model--entry-status entry)) 'ready)
           (not (romacs-model--entry-frozen entry))
           (car readiness)
           (eql (nth 2 readiness) (romacs-model--entry-issued entry))))))

(defun romacs-model-revision (model)
  "Return the revision of MODEL's current authoritative picture, or nil.
Readiness is recorded synchronously on native event arrival, before
user callbacks run.  This query therefore identifies the settled
picture even while its callback is deferred.  Return nil without
current presentation authority, including pending requests, active
interaction, renderer loss, loading, frozen and destroyed models."
  (when (romacs-model-current-p model)
    (nth 1 (romacs-model--entry-readiness (romacs-model--entry model)))))

;;;; Destruction

(defun romacs-model-destroy (model)
  "Free MODEL's renderer resources now.  Idempotent."
  (let ((entry (romacs-model--entry model)))
    (unless (eq (car (romacs-model--entry-status entry)) 'destroyed)
      (romacs-model--unattach model (romacs-model--entry-buffer entry))
      (setf (romacs-model--entry-status entry) (list 'destroyed)
            (romacs-model--entry-buffer entry) nil)
      (remhash (romacs-model--id model) romacs-model--by-id)
      (romacs-surface-destroy (romacs-model--id model))))
  nil)

(defvar-local romacs-model--attached nil
  "The models attached to this buffer, a table of model to t, so killing
it finds them without a scan of every model.  A model leaves it when it
moves to another buffer or is destroyed.")
(put 'romacs-model--attached 'permanent-local t)

(defun romacs-model--attach (model)
  "Note MODEL as attached to the current buffer."
  (puthash model t (or romacs-model--attached
                       (setq romacs-model--attached (make-hash-table :test #'eq)))))

(defun romacs-model--unattach (model buffer)
  "MODEL is no longer attached to BUFFER."
  (when (buffer-live-p buffer)
    (with-current-buffer buffer
      (when romacs-model--attached (remhash model romacs-model--attached)))))

(defun romacs-model--kill-buffer ()
  "Destroy the models attached to the current buffer."
  (when romacs-model--attached
    (let ((buffer (current-buffer)) (doomed (hash-table-keys romacs-model--attached)))
      (clrhash romacs-model--attached)
      (dolist (model doomed)
        (when (eq (romacs-model--entry-buffer (romacs-model--entry model)) buffer)
          (romacs-model-destroy model))))))

;;;; Events

(defvar romacs-model--event nil
  "Serial number of the model event whose hooks run now, or nil.
`romacs-model--handle-event' binds it around both hooks, so
`romacs-model--note' can tag the callback it queues with its event
and `romacs-model--dispatch' run only the current event's callbacks.")

(defvar romacs-model--queue nil
  "Callbacks (SERIAL ID TYPE PLIST) waiting to run, oldest first.
SERIAL is the event's `romacs-model--event'.  By id, so a queued
callback does not keep its model alive.")

(defvar romacs-model--draining nil
  "Non-nil while `romacs-model--dispatch' runs callbacks.")

(defun romacs-model--note (id type plist)
  "Apply the model event (ID TYPE PLIST) and queue its callback.
Runs as soon as the event arrives, even inside a callback, so it
only records state; `romacs-model--dispatch' runs the callback."
  (let* ((model (romacs-model--find id))
         (entry (and model (gethash model romacs-model--registry))))
    (when (and entry
               (not (eq (car (romacs-model--entry-status entry)) 'destroyed)))
      (pcase type
        ('ready
         (setf (romacs-model--entry-status entry)
               (list 'ready
                     :triangles (plist-get plist :triangles)
                     :meshes (append (romacs-model--entry-mesh-ids entry)
                                     nil))))
        ('failed
         (setf (romacs-model--entry-status entry)
               (list 'failed :error (plist-get plist :error))))
        ('error
         ;; Error{REQ} settles REQ and everything before it; a refused
         ;; resize changes nothing in the renderer.
         (let ((req (plist-get plist :req)))
           (when req
             (romacs-model--settle-resizes entry (1- req))
             (romacs-model--refuse-resize model entry req))))
        ('view
         ;; The channel settles a camera request before reporting an
         ;; interaction begun after it.  Until then, these reports can
         ;; only name an older camera, queued before Lisp's request.
         (unless (romacs-model--entry-pending-view entry)
           (setf (romacs-model--entry-view entry) (plist-get plist :view))))
        ('readiness
         (setf (romacs-model--entry-readiness entry)
               (list (plist-get plist :current)
                     (plist-get plist :rev)
                     (plist-get plist :req)))
         (romacs-model--settle-resizes entry (plist-get plist :req))
         (let ((pending (romacs-model--entry-pending-view entry)))
           (when (and pending (>= (plist-get plist :req) pending))
             (setf (romacs-model--entry-pending-view entry) nil))))
        ('pick
         ;; Only a pick of the picture Lisp holds as current names what
         ;; the user saw (design §6.2); anything else is stale.  It is
         ;; checked again when its callback runs.
         (setq plist
               (and (romacs-model--pick-current-p model entry plist)
                    (romacs-model--pick-plist entry plist)))
         (unless plist (setq type nil))))
      (when (and type (romacs-model--entry-callback entry))
        (setq romacs-model--queue
              (nconc romacs-model--queue
                     (list (list romacs-model--event id type plist))))))))

(add-hook 'romacs-model--state-functions #'romacs-model--note)

(defun romacs-model--dispatch (&rest _event)
  "Run the queued callbacks of the current model event, oldest first.
A member of `romacs-model-event-functions'; the queue holds the
callbacks of the events `romacs-model--note' recorded.  Only those
of the event being delivered run, after any an earlier delivery's
non-local exit left behind.  The callback of an event that arrives
meanwhile (for example inside a callback's `sit-for') runs when that
event is delivered, once every member has handled this one."
  (unless romacs-model--draining
    (let ((romacs-model--draining t)
          (current romacs-model--event))
      (while (and romacs-model--queue
                  (let ((serial (car (car romacs-model--queue))))
                    (or (null current) (null serial) (<= serial current))))
        (apply #'romacs-model--run-callback
               (cdr (pop romacs-model--queue)))))))

(add-hook 'romacs-model-event-functions #'romacs-model--dispatch)

(defvar romacs-model--serial 0 "Serial number of the latest model event.")

(defvar romacs-model--undelivered nil
  "Model events (SERIAL ID TYPE PLIST) whose event functions wait to run.")

(defvar romacs-model--delivering nil
  "Non-nil while `romacs-model-event-functions' runs for an event.")

(defun romacs-model--run-member (function id type plist)
  "Call event FUNCTION with ID TYPE PLIST, logging its error; return nil."
  (condition-case err
      (funcall function id type plist)
    (error (message "romacs-model: event function %S failed: %S" function err)))
  nil)

(defun romacs-model--handle-event (event)
  "Deliver the native model EVENT, (romacs-model ID TYPE PLIST).
Bound in `special-event-map', so it runs as the event arrives, inside
a read too.  `romacs-model--state-functions' run at once; the event
functions run in order and never nested: an event arriving while they
run (inside a callback's `sit-for') waits until they finish."
  (interactive "e")
  (pcase-let ((`(,_ ,id ,type ,plist) event))
    (let ((serial (setq romacs-model--serial (1+ romacs-model--serial))))
      (let ((romacs-model--event serial))
        (run-hook-with-args 'romacs-model--state-functions id type plist))
      (setq romacs-model--undelivered
            (nconc romacs-model--undelivered (list (list serial id type plist))))
      (unless romacs-model--delivering
        (let ((romacs-model--delivering t))
          (while romacs-model--undelivered
            (pcase-let ((`(,serial ,id ,type ,plist) (pop romacs-model--undelivered)))
              (let ((romacs-model--event serial))
                (run-hook-wrapped 'romacs-model-event-functions
                                  #'romacs-model--run-member id type plist)))))))))

(put 'romacs-model--handle-event 'completion-predicate #'ignore)
(define-key special-event-map [romacs-model] #'romacs-model--handle-event)

(defun romacs-model--pick-current-p (model entry plist)
  "Return non-nil if pick PLIST names the picture MODEL holds as current.
ENTRY is MODEL's registry entry."
  (and (romacs-model-current-p model)
       (eql (plist-get plist :rev)
            (nth 1 (romacs-model--entry-readiness entry)))))

(defun romacs-model--pick-plist (entry plist)
  "Return pick PLIST with its hit mesh index mapped to ENTRY's mesh ID."
  (let ((hit (plist-get plist :hit))
        (copy (copy-sequence plist)))
    (when hit
      (setq hit (copy-sequence hit))
      (plist-put hit :mesh (aref (romacs-model--entry-mesh-ids entry)
                                 (plist-get hit :mesh)))
      (plist-put copy :hit hit))
    copy))

(defun romacs-model--run-callback (id type plist)
  "Call the callback of model ID with (MODEL TYPE PLIST), logging any error.
Nothing runs once MODEL is destroyed, nor for a `pick' whose picture
is no longer current: a request, readiness or renderer loss may have
come since it was queued.  The attached buffer is current while it
is live."
  (let* ((model (romacs-model--find id))
         (entry (and model (gethash model romacs-model--registry)))
         (callback (and entry (romacs-model--entry-callback entry)))
         (buffer (and entry (romacs-model--entry-buffer entry))))
    (when (and callback
               (not (eq (car (romacs-model--entry-status entry)) 'destroyed))
               (or (not (eq type 'pick))
                   (romacs-model--pick-current-p model entry plist)))
      (condition-case err
          (if (buffer-live-p buffer)
              (with-current-buffer buffer
                (funcall callback model type plist))
            (funcall callback model type plist))
        (error
         (message "romacs-model: callback for %S failed: %S" type err))))))

;;;; Keymaps and commands

(defun romacs-model-mouse-select (event)
  "Select the window of mouse EVENT and move point to its position.
Bound on `down-mouse-1' over a model so the press never starts
`mouse-drag-region': the renderer owns the drag (orbit and pick)."
  (interactive "e")
  (let* ((posn (event-start event))
         (window (posn-window posn))
         (pos (posn-point posn)))
    (when (window-live-p window)
      (select-window window)
      (when (integer-or-marker-p pos)
        (goto-char pos)))))

(defvar romacs-model-mouse-map
  (let ((map (make-sparse-keymap)))
    (define-key map [down-mouse-1] #'romacs-model-mouse-select)
    (define-key map [mouse-1] #'ignore)
    (define-key map [drag-mouse-1] #'ignore)
    map)
  "Mouse bindings every model glyph has, searched before its keymap.")

(defvar-local romacs-model--buffer-model nil
  "The model of a `romacs-model-view-mode' buffer.")

(defun romacs-model--at-point ()
  "Return the model at point, or the current buffer's model."
  (or (get-char-property (point) 'romacs-model)
      (and (> (point) (point-min))
           (get-char-property (1- (point)) 'romacs-model))
      romacs-model--buffer-model
      (user-error "No model here")))

(defun romacs-model--nudge (dyaw dpitch zoom-factor)
  "Turn the model at point by DYAW, DPITCH degrees; scale zoom by ZOOM-FACTOR."
  (let* ((model (romacs-model--at-point))
         (view (romacs-model-view model)))
    (romacs-model-set-view model
                            (+ (nth 0 view) dyaw)
                            (+ (nth 1 view) dpitch)
                            (min 4.0 (max 0.25 (* (nth 2 view) zoom-factor))))))

(defun romacs-model-yaw-forward ()
  "Turn the model at point by 15 degrees of yaw."
  (interactive)
  (romacs-model--nudge 15 0 1))

(defun romacs-model-yaw-backward ()
  "Turn the model at point by -15 degrees of yaw."
  (interactive)
  (romacs-model--nudge -15 0 1))

(defun romacs-model-pitch-next ()
  "Tilt the model at point by 15 degrees of pitch."
  (interactive)
  (romacs-model--nudge 0 15 1))

(defun romacs-model-pitch-previous ()
  "Tilt the model at point by -15 degrees of pitch."
  (interactive)
  (romacs-model--nudge 0 -15 1))

(defun romacs-model-zoom-in ()
  "Zoom the model at point in by 10%."
  (interactive)
  (romacs-model--nudge 0 0 1.1))

(defun romacs-model-zoom-out ()
  "Zoom the model at point out by 10%."
  (interactive)
  (romacs-model--nudge 0 0 (/ 1 1.1)))

(defun romacs-model-reset-view ()
  "Return the model at point to the view it was created with."
  (interactive)
  (let ((model (romacs-model--at-point)))
    (apply #'romacs-model-set-view model
           (romacs-model--entry-initial-view (romacs-model--entry model)))))

(defun romacs-model--command-p (_symbol buffer)
  "Whether BUFFER has a model at point for the camera commands."
  (with-current-buffer buffer (ignore-error user-error (romacs-model--at-point))))

(dolist (command '( romacs-model-yaw-forward romacs-model-yaw-backward romacs-model-pitch-next
                    romacs-model-pitch-previous romacs-model-zoom-in romacs-model-zoom-out
                    romacs-model-reset-view))
  (put command 'completion-predicate #'romacs-model--command-p))

;; Its press on a glyph is what it needs: M-x has no event to give it.
(put 'romacs-model-mouse-select 'completion-predicate #'ignore)

(defvar romacs-model-view-map
  (let ((map (make-sparse-keymap)))
    (define-key map "f" #'romacs-model-yaw-forward)
    (define-key map "b" #'romacs-model-yaw-backward)
    (define-key map "n" #'romacs-model-pitch-next)
    (define-key map "p" #'romacs-model-pitch-previous)
    (define-key map "+" #'romacs-model-zoom-in)
    (define-key map "-" #'romacs-model-zoom-out)
    (define-key map "0" #'romacs-model-reset-view)
    map)
  "Default keymap of an inserted model glyph: camera keys.")

(defvar romacs-model-view-mode-map
  (let ((map (make-sparse-keymap)))
    (set-keymap-parent map (make-composed-keymap romacs-model-view-map
                                                 special-mode-map))
    (define-key map "q" #'quit-window)
    map)
  "Keymap of `romacs-model-view-mode': camera keys and `q'.")

(define-derived-mode romacs-model-view-mode special-mode "Model"
  "Major mode of a standalone native model buffer.
\\{romacs-model-view-mode-map}"
  :interactive nil)

;;;; Insertion

(defun romacs-model--display-spec (model entry)
  "Return MODEL's display spec at the size recorded in ENTRY."
  (list 'surface :id (romacs-model--id model)
        :width (romacs-model--entry-width entry)
        :height (romacs-model--entry-height entry)))

(defun romacs-model--map-glyphs (buffer model function)
  "Call FUNCTION with START END of each MODEL glyph in BUFFER."
  (when (buffer-live-p buffer)
    (with-current-buffer buffer
      (save-restriction
        (widen)
        (let ((pos (point-min)) next)
          (while (< pos (point-max))
            (setq next (next-single-property-change pos 'romacs-model
                                                    nil (point-max)))
            (when (eq (get-text-property pos 'romacs-model) model)
              (funcall function pos next))
            (setq pos next)))))))

(defun romacs-model--update-glyphs (model entry)
  "Give MODEL's glyphs in its attached buffer ENTRY's current size."
  (romacs-model--map-glyphs
   (romacs-model--entry-buffer entry) model
   (lambda (start end)
     (with-silent-modifications
       (put-text-property start end 'display
                          (romacs-model--display-spec model entry))))))

(defun romacs-model--detach (model entry)
  "Blank MODEL's glyphs in the buffer ENTRY attaches it to; forget it.
Nothing of MODEL stays there: a later resize cannot revive an old
glyph, and the camera keys no longer find MODEL in that buffer."
  (let ((old (romacs-model--entry-buffer entry)))
    (romacs-model--map-glyphs
     old model
     (lambda (start end)
       (with-silent-modifications
         (remove-text-properties start end
                                 '(display nil keymap nil romacs-model nil)))))
    (when (buffer-live-p old)
      (with-current-buffer old
        (when romacs-model--attached (remhash model romacs-model--attached))
        (when (eq romacs-model--buffer-model model)
          (setq romacs-model--buffer-model nil))))
    (setf (romacs-model--entry-buffer entry) nil)))

(defun romacs-model-insert (model &rest plist)
  "Insert MODEL's glyph at point and attach MODEL to the current buffer.
A model is attached to one buffer: inserting it elsewhere moves it,
and killing that buffer destroys MODEL.  PLIST keys:
:keymap  the glyph's keymap (default `romacs-model-view-map', nil
         for none); `romacs-model-mouse-map' always comes first.
:string  the text carrying the glyph, not empty (default \" \")."
  (let* ((entry (romacs-model--entry model))
         (keymap (if (plist-member plist :keymap)
                     (plist-get plist :keymap)
                   romacs-model-view-map))
         (string (or (plist-get plist :string) " ")))
    (when (eq (car (romacs-model--entry-status entry)) 'destroyed)
      (error "Model is destroyed"))
    (unless (and (stringp string) (> (length string) 0))
      (error "Model :string must be a non-empty string"))
    ;; Do not transfer attachment until insertion succeeds: a read-only
    ;; destination must leave the old glyph live.
    (insert (propertize string
                        'display (romacs-model--display-spec model entry)
                        'keymap (make-composed-keymap
                                 (delq nil (list romacs-model-mouse-map
                                                 keymap)))
                        'romacs-model model
                        'rear-nonsticky t))
    (unless (eq (romacs-model--entry-buffer entry) (current-buffer))
      (romacs-model--detach model entry)
      (romacs-model--attach model))
    (setf (romacs-model--entry-buffer entry) (current-buffer))
    (add-hook 'kill-buffer-hook #'romacs-model--kill-buffer nil t)
    nil))

;;;###autoload
(defun romacs-model-open (meshes &rest plist)
  "Show MESHES in a buffer *Model: NAME* in `romacs-model-view-mode'.
MESHES and PLIST are as for `romacs-model-create', plus :name
\(default: the first mesh ID).  Return the buffer."
  (let* ((name (or (plist-get plist :name) (car (car meshes))))
         (model (apply #'romacs-model-create :meshes meshes plist))
         (buffer (get-buffer-create (format "*Model: %s*" name))))
    (with-current-buffer buffer
      ;; A reused buffer drops the model it showed before.
      (romacs-model--kill-buffer)
      (romacs-model-view-mode)
      (setq romacs-model--buffer-model model)
      (let ((inhibit-read-only t))
        (erase-buffer)
        (romacs-model-insert model))
      (goto-char (point-min)))
    (pop-to-buffer buffer)
    buffer))

(provide 'romacs-model)
;;; romacs-model.el ends here
