;;; gnosis-model-camera.el --- Model camera geometry -*- lexical-binding: t; -*-

;; Copyright (C) 2026 Free Software Foundation, Inc.
;; SPDX-License-Identifier: GPL-3.0-or-later

;;; Commentary:
;; Pure camera computations shared by review and scene authoring.  A view
;; is (YAW PITCH ZOOM) in degrees.  Both renderers rotate a scene point P,
;; normalized as (P - CENTRE) / RADIUS, by Ry(YAW) Rx(PITCH) into Q and
;; draw Q's x and y at clip coordinates Q * ZOOM * 0.8; larger Q z is
;; nearer.  CENTRE is the bounding-box centre of every vertex and RADIUS
;; the farthest vertex from it.
;;
;; Visibility is decided in Lisp, without the renderer, and only when it
;; is certain: every triangle that could lie between the target and the
;; viewer at any candidate camera is kept, and each target test point is
;; checked exactly against them.  When too many triangles could hide the
;; target, nothing is known and review keeps the authored camera.

;;; Code:

(require 'cl-lib)
(require 'seq)

(defcustom gnosis-model-camera-variation '(25 15 0.1)
  "Bounds of the starting camera varied on each model review, or nil.
A list (YAW PITCH ZOOM): each review starts within YAW and PITCH degrees
and a ZOOM fraction of the authored camera, at a view that keeps the
target about as visible as the authored camera does.  The choice is
deterministic for a thema and its number of reviews.  Nil always starts
at the authored camera.  Authoring and feedback are never varied.
Cameras are prepared with the card for the bounds then in effect; a card
prepared under other bounds starts at the authored camera."
  :type '(choice (const :tag "Authored camera" nil)
                 (list (number :tag "Yaw degrees")
                       (number :tag "Pitch degrees")
                       (number :tag "Zoom fraction")))
  :group 'gnosis)

(defconst gnosis-model-camera--keep 0.7
  "Fraction of the authored visibility a varied camera must retain.")

(defconst gnosis-model-camera--target-faces 128
  "Most target triangles whose vertices and centroids test visibility.")

(defconst gnosis-model-camera--occluders 20000
  "Most triangles that may hide a target before its visibility is unknown.")

(defconst gnosis-model-camera--offsets
  '((1 0 0) (-1 0 0) (0.6 1 0.5) (-0.6 -1 -0.5)
    (1 -0.6 -1) (-1 0.6 1) (0.4 -1 1) (-0.4 1 -1))
  "Candidate offsets as fractions of the variation bounds, in a fixed order.")

(defun gnosis-model-camera-unwrap (view reference)
  "Return VIEW with angles shifted by whole revolutions toward REFERENCE.
Renderers report angles wrapped to 0..360; authored cameras keep the
author's sign, so -30 stays -30 rather than becoming 330."
  (append (cl-mapcar (lambda (angle initial)
                       (+ angle (* 360 (round (/ (- initial angle) 360.0)))))
                     (seq-take view 2) (seq-take reference 2))
          (list (nth 2 view))))

(defun gnosis-model-camera--rotation (view)
  "Return the rows of Ry(yaw) Rx(pitch) for VIEW as a nine-float vector."
  (let* ((yaw (degrees-to-radians (float (nth 0 view))))
         (pitch (degrees-to-radians (float (nth 1 view))))
         (cy (cos yaw)) (sy (sin yaw)) (cp (cos pitch)) (sp (sin pitch)))
    (vector cy (* sy sp) (* sy cp)
            0.0 cp (- sp)
            (- sy) (* cy sp) (* cy cp))))

(defun gnosis-model-camera-project (view point)
  "Return clip (X Y DEPTH) of normalized POINT seen from VIEW.
X and Y lie in -1..1 on the picture; larger DEPTH is nearer."
  (let ((r (gnosis-model-camera--rotation view))
        (scale (* 0.8 (nth 2 view)))
        (x (nth 0 point)) (y (nth 1 point)) (z (nth 2 point)))
    (list (* scale (+ (* (aref r 0) x) (* (aref r 1) y) (* (aref r 2) z)))
          (* scale (+ (* (aref r 3) x) (* (aref r 4) y) (* (aref r 5) z)))
          (+ (* (aref r 6) x) (* (aref r 7) y) (* (aref r 8) z)))))

(defun gnosis-model-camera-candidates (view bounds)
  "Return varied cameras around VIEW within BOUNDS, a (YAW PITCH ZOOM) list.
Each candidate moves VIEW by fixed fractions of BOUNDS, never further:
yaw and pitch by at most their bounds in degrees and zoom by at most its
bound as a fraction of VIEW's zoom, kept within 0.25..4.  VIEW itself is
not listed, so zero bounds give no candidates."
  (pcase-let ((`(,yaw ,pitch ,zoom) view)
              (`(,dyaw ,dpitch ,dzoom) bounds))
    (seq-filter
     (lambda (candidate)
       (pcase-let ((`(,y ,p ,z) candidate))
         (and (not (and (= y yaw) (= p pitch) (= z zoom)))
              ;; Guard against rounding past a bound.
              (<= (abs (- y yaw)) dyaw)
              (<= (abs (- p pitch)) dpitch)
              (<= (abs (- z zoom)) (* dzoom zoom)))))
     (delete-dups
      (mapcar (lambda (offset)
                (list (float (+ yaw (* dyaw (nth 0 offset))))
                      (float (+ pitch (* dpitch (nth 1 offset))))
                      (float (max 0.25 (min 4 (+ zoom (* zoom dzoom (nth 2 offset))))))))
              gnosis-model-camera--offsets)))))

(defun gnosis-model-camera-centre (vertex-sets)
  "Return the bounding-box centre of VERTEX-SETS, as the renderers frame it.
VERTEX-SETS is a list of sequences of (X Y Z) vertices."
  (let ((low (make-vector 3 1.0e+INF)) (high (make-vector 3 -1.0e+INF)))
    (dolist (vertices vertex-sets)
      (seq-doseq (vertex vertices)
        (cl-loop for x in vertex for axis from 0
                 do (aset low axis (min x (aref low axis)))
                 (aset high axis (max x (aref high axis))))))
    (cl-mapcar (lambda (a b) (/ (+ a b) 2.0)) low high)))

(defun gnosis-model-camera--normalize-triangle (triangle centre radius)
  "Return TRIANGLE's vertices normalized by CENTRE and RADIUS, as 9 floats."
  (let ((v (make-vector 9 0.0)) (i 0))
    (dolist (vertex triangle)
      (cl-loop for x in vertex for c in centre
               do (aset v i (/ (- x c) radius)) (cl-incf i)))
    v))

(defun gnosis-model-camera-target-points (triangles frame &optional faces)
  "Return normalized points testing visibility of a target in TRIANGLES.
FRAME is (CENTRE . RADIUS) from `gnosis-model-camera-centre' and the
scene radius.  The target is FACES of TRIANGLES, or all of them; take
the vertices and centroid of at most `gnosis-model-camera--target-faces'
of its triangles."
  (let* ((faces (and faces (vconcat faces)))
         (count (if faces (length faces) (length triangles)))
         (stride (max 1 (ceiling count gnosis-model-camera--target-faces))))
    (cl-loop
     for i from 0 below count by stride
     append (let ((v (gnosis-model-camera--normalize-triangle
                      (aref triangles (if faces (aref faces i) i))
                      (car frame) (float (cdr frame)))))
              (cons (list (/ (+ (aref v 0) (aref v 3) (aref v 6)) 3)
                          (/ (+ (aref v 1) (aref v 4) (aref v 7)) 3)
                          (/ (+ (aref v 2) (aref v 5) (aref v 8)) 3))
                    (cl-loop for j from 0 below 9 by 3
                             collect (list (aref v j) (aref v (+ j 1)) (aref v (+ j 2)))))))))

(defun gnosis-model-camera-normalize (point frame)
  "Return original-coordinate POINT normalized within FRAME."
  (mapcar (lambda (pair) (/ (- (car pair) (cdr pair)) (float (cdr frame))))
          (cl-mapcar #'cons point (car frame))))

(defun gnosis-model-camera--direction (view)
  "Return the unit direction towards the viewer of VIEW, as a vector."
  (let ((r (gnosis-model-camera--rotation view)))
    (vector (aref r 6) (aref r 7) (aref r 8))))

(defun gnosis-model-camera-occluders (geometry frame target points views)
  "Return the triangles of GEOMETRY that may hide POINTS from any of VIEWS.
GEOMETRY is an alist of (MESH . TRIANGLES) and FRAME its (CENTRE . RADIUS).
TARGET is (MESH . FACES), FACES nil meaning the whole mesh; its triangles
never hide it.  Every other triangle is kept, normalized as a vector of
9 floats, unless its bounding sphere lies off every line of sight from
the sphere around POINTS within the angle VIEWS span.  Throw
`gnosis-model-camera--too-many' when more than
`gnosis-model-camera--occluders' would be kept."
  (let* ((centre (car frame)) (radius (float (cdr frame)))
         (directions (mapcar #'gnosis-model-camera--direction views))
         (axis (car directions))
         (spread (cl-loop for d in directions
                          maximize (acos (max -1.0 (min 1.0 (cl-loop for i below 3
                                                                    sum (* (aref d i) (aref axis i))))))))
         (cos-spread (cos spread)) (sin-spread (sin spread))
         ;; Beyond a right angle the cosine bound below fails; keep all.
         (wide (> spread (/ float-pi 2)))
         (low (apply #'cl-mapcar #'min points)) (high (apply #'cl-mapcar #'max points))
         (middle (cl-mapcar (lambda (a b) (/ (+ a b) 2.0)) low high))
         ;; The target's bounding sphere, in scene units.
         (tc (cl-mapcar (lambda (m c) (+ c (* radius m))) middle centre))
         (tr (* radius (cl-loop for p in points
                                maximize (sqrt (cl-loop for x in p for m in middle
                                                        sum (* (- x m) (- x m)))))))
         (tx (nth 0 tc)) (ty (nth 1 tc)) (tz (nth 2 tc))
         (ax (aref axis 0)) (ay (aref axis 1)) (az (aref axis 2))
         (left gnosis-model-camera--occluders)
         kept)
    (pcase-dolist (`(,mesh . ,triangles) geometry)
      (let ((skip (and (equal mesh (car target))
                       (if (cdr target)
                           (let ((faces (make-bool-vector (length triangles) nil)))
                             (dolist (face (cdr target) faces) (aset faces face t)))
                         t))))
        (unless (eq skip t)
          (dotimes (face (length triangles))
            (unless (and skip (aref skip face))
              (let* ((triangle (aref triangles face))
                     (p0 (car triangle)) (p1 (cadr triangle)) (p2 (caddr triangle))
                     (cx (/ (+ (car p0) (car p1) (car p2)) 3.0))
                     (cy (/ (+ (cadr p0) (cadr p1) (cadr p2)) 3.0))
                     (cz (/ (+ (caddr p0) (caddr p1) (caddr p2)) 3.0))
                     (size 0.0)
                     (vx (- cx tx)) (vy (- cy ty)) (vz (- cz tz))
                     (gap (sqrt (+ (* vx vx) (* vy vy) (* vz vz)))))
                (dolist (p triangle)
                  (let ((dx (- (car p) cx)) (dy (- (cadr p) cy)) (dz (- (caddr p) cz)))
                    (setq size (max size (+ (* dx dx) (* dy dy) (* dz dz))))))
                ;; A line of sight through both bounding spheres deviates
                ;; from the centres' direction by at most B = asin(REACH),
                ;; REACH being their radii over their gap.  It can lie
                ;; within SPREAD of AXIS only when the centres' direction
                ;; is within SPREAD + B of AXIS: cosine at least
                ;; cos SPREAD cos B - sin SPREAD sin B.
                (let ((reach (/ (+ (sqrt size) tr) (max gap 1e-300))))
                  (when (or wide (>= reach 1.0)
                            (>= (/ (+ (* vx ax) (* vy ay) (* vz az)) gap)
                                (- (* cos-spread (sqrt (- 1.0 (* reach reach))))
                                   (* sin-spread reach)
                                   1e-9)))
                    (when (< (setq left (1- left)) 0)
                      (throw 'gnosis-model-camera--too-many nil))
                    (push (gnosis-model-camera--normalize-triangle triangle centre radius)
                          kept)))))))))
    kept))

(defun gnosis-model-camera-visibility (occluders view points)
  "Return (VISIBLE . FRAMED) counts of target POINTS seen from VIEW.
POINTS are normalized; FRAMED counts those on the picture and VISIBLE
those of them that no triangle of OCCLUDERS covers at a nearer depth.
The test is exact up to rounding: a point on an occluder's edge counts
as covered, and only a depth difference below 1e-6 of the scene radius
counts as not nearer."
  (let* ((s (* 0.8 (nth 2 view)))
         (r (gnosis-model-camera--rotation view))
         (r0 (aref r 0)) (r1 (aref r 1)) (r2 (aref r 2)) (r3 (aref r 3)) (r4 (aref r 4))
         (r5 (aref r 5)) (r6 (aref r 6)) (r7 (aref r 7)) (r8 (aref r 8))
         (xs nil) (ys nil) (ds nil))
    (pcase-dolist (`(,px ,py ,pz) points)
      (let ((x (* s (+ (* r0 px) (* r1 py) (* r2 pz))))
            (y (* s (+ (* r3 px) (* r4 py) (* r5 pz)))))
        (when (and (<= (abs x) 0.95) (<= (abs y) 0.95))
          (push x xs) (push y ys) (push (+ (* r6 px) (* r7 py) (* r8 pz)) ds))))
    (if (null xs) (cons 0 0)
      (let* ((xs (vconcat xs)) (ys (vconcat ys)) (ds (vconcat ds))
             (count (length xs))
             (hidden (make-bool-vector count nil))
             (n 16)
             (x-low (seq-min xs)) (y-low (seq-min ys))
             (x-high (seq-max xs)) (y-high (seq-max ys))
             (cell-x (/ (max 1e-9 (- x-high x-low)) n))
             (cell-y (/ (max 1e-9 (- y-high y-low)) n))
             (farthest (seq-min ds))
             (cells (make-vector (* n n) nil)))
        (cl-flet ((column (x) (max 0 (min (1- n) (floor (- x x-low) cell-x))))
                  (row (y) (max 0 (min (1- n) (floor (- y y-low) cell-y)))))
          (dotimes (i count)
            (push i (aref cells (+ (column (aref xs i)) (* n (row (aref ys i)))))))
          (dolist (v occluders)
            (let* ((a0 (aref v 0)) (b0 (aref v 1)) (c0 (aref v 2))
                   (a1 (aref v 3)) (b1 (aref v 4)) (c1 (aref v 5))
                   (a2 (aref v 6)) (b2 (aref v 7)) (c2 (aref v 8))
                   (d0 (+ (* r6 a0) (* r7 b0) (* r8 c0)))
                   (d1 (+ (* r6 a1) (* r7 b1) (* r8 c1)))
                   (d2 (+ (* r6 a2) (* r7 b2) (* r8 c2))))
              ;; Only a surface nearer than some target point can hide it.
              (when (> (max d0 d1 d2) (+ farthest 1e-6))
                (let ((x0 (* s (+ (* r0 a0) (* r1 b0) (* r2 c0))))
                      (x1 (* s (+ (* r0 a1) (* r1 b1) (* r2 c1))))
                      (x2 (* s (+ (* r0 a2) (* r1 b2) (* r2 c2))))
                      (y0 (* s (+ (* r3 a0) (* r4 b0) (* r5 c0))))
                      (y1 (* s (+ (* r3 a1) (* r4 b1) (* r5 c1))))
                      (y2 (* s (+ (* r3 a2) (* r4 b2) (* r5 c2)))))
                  (when (and (<= (min x0 x1 x2) x-high) (>= (max x0 x1 x2) x-low)
                             (<= (min y0 y1 y2) y-high) (>= (max y0 y1 y2) y-low))
                    (let ((area (- (* (- x1 x0) (- y2 y0)) (* (- x2 x0) (- y1 y0)))))
                      ;; Seen edge-on a triangle covers nothing.
                      (unless (< (abs area) 1e-12)
                        (cl-loop
                         for row from (row (min y0 y1 y2)) to (row (max y0 y1 y2))
                         do (cl-loop
                             for col from (column (min x0 x1 x2)) to (column (max x0 x1 x2))
                             do (dolist (i (aref cells (+ col (* n row))))
                                  (unless (aref hidden i)
                                    (let* ((x (aref xs i)) (y (aref ys i))
                                           ;; Barycentric weights of the point.
                                           (w0 (/ (- (* (- x2 x1) (- y y1)) (* (- y2 y1) (- x x1))) area))
                                           (w1 (/ (- (* (- x0 x2) (- y y2)) (* (- y0 y2) (- x x2))) area))
                                           (w2 (- 1 w0 w1)))
                                      (when (and (>= w0 -1e-9) (>= w1 -1e-9) (>= w2 -1e-9)
                                                 (> (+ (* w0 d0) (* w1 d1) (* w2 d2))
                                                    (+ (aref ds i) 1e-6)))
                                        (aset hidden i t)))))))))))))))
          (cons (- count (cl-count t hidden)) count)))))

(defun gnosis-model-camera-views (geometry frame view bounds target points)
  "Return cameras within BOUNDS of VIEW keeping the target visible.
GEOMETRY, FRAME and TARGET are as for `gnosis-model-camera-occluders';
POINTS are the target's normalized test points.  A candidate must keep
`gnosis-model-camera--keep' of both the visible and the framed target
points at VIEW.  Return nil, so review keeps the authored camera, when
no target point is visible at VIEW or too many triangles could hide it."
  (catch 'gnosis-model-camera--too-many
    (let* ((candidates (gnosis-model-camera-candidates view bounds))
           (occluders (and candidates
                           (gnosis-model-camera-occluders
                            geometry frame target points (cons view candidates))))
           (base (and candidates (gnosis-model-camera-visibility occluders view points))))
      (when (and base (> (car base) 0))
        (seq-filter
         (lambda (candidate)
           (let ((seen (gnosis-model-camera-visibility occluders candidate points)))
             (and (> (car seen) 0)
                  (>= (car seen) (* gnosis-model-camera--keep (car base)))
                  (>= (cdr seen) (* gnosis-model-camera--keep (cdr base))))))
         candidates)))))

(defun gnosis-model-camera-choose (views id count)
  "Return the view among VIEWS for thema ID's COUNTth review, or nil.
Successive counts step through VIEWS from an ID-specific offset."
  (when views
    (let ((offset (string-to-number
                   (substring (secure-hash 'sha256 (format "%s" id)) 0 6) 16)))
      (nth (mod (+ offset count) (length views)) views))))

(provide 'gnosis-model-camera)
;;; gnosis-model-camera.el ends here
