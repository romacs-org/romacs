;;; gnosis-test-model-camera.el --- Model camera and scene authoring tests -*- lexical-binding: t; -*-

;; Copyright (C) 2026 Free Software Foundation, Inc.

;;; Commentary:
;; Varied review cameras and click-to-author scene sessions.  Synthetic
;; triangles and imported fixtures; no renderer.

;;; Code:
(require 'ert)
(require 'gnosis-model-test-support)
(require 'gnosis-model-camera)

(defun gnosis-test-model-camera--pick ()
  "Select the other triangle through the authoring callback."
  (setq-local canvas-3d-selected-id "other")
  (run-hook-with-args 'gnosis-model-backend-selection-hook
                      (list :id "other" :frame 1 :owner canvas-3d--process)))

(ert-deftest gnosis-model-camera-projection-matches-renderer ()
  "R = Ry(yaw) Rx(pitch); zoom scales clip space; larger depth is nearer."
  (cl-flet ((project (view point)
              (mapcar (lambda (n) (/ (round (* n 1000)) 1000.0))
                      (gnosis-model-camera-project view point))))
    (should (equal (project '(0 0 1) '(0 0 1)) '(0.0 0.0 1.0)))
    (should (equal (project '(90 0 1) '(0 0 1)) '(0.8 0.0 0.0)))
    ;; Z-up anatomy at the usual pitch -90: +Z is up, -Y faces the viewer.
    (should (equal (project '(0 -90 2) '(0 0 1)) '(0.0 1.6 0.0)))
    (should (equal (project '(0 -90 2) '(0 -1 0)) '(0.0 0.0 1.0))))
  (should (equal (gnosis-model-camera-unwrap '(330 350 1.5) '(-30 0 1)) '(-30 -10 1.5))))

(ert-deftest gnosis-model-camera-candidates-stay-in-bounds ()
  (let ((views (gnosis-model-camera-candidates '(10 -90 1.0) '(25 15 0.1))))
    (should (= (length views) (length (delete-dups (copy-sequence views)))))
    (should-not (member '(10.0 -90.0 1.0) views))
    (should (equal (car views) '(35.0 -90.0 1.0)))
    (dolist (view views)
      (should (<= (abs (- (nth 0 view) 10)) 25))
      (should (<= (abs (- (nth 1 view) -90)) 15))
      (should (<= 0.9 (nth 2 view) 1.1)))))

(ert-deftest gnosis-model-camera-rejects-off-picture-target ()
  "A target near the picture edge keeps only views that leave it framed."
  (let* ((geometry '(("target" . [((0.95 0 0.5) (0.951 0 0.5) (0.95 0.001 0.5))])))
         (points '((0.95 0 0.5)))
         (views (gnosis-model-camera-views geometry '((0 0 0) . 1) '(0 0 1.2) '(25 0 0)
                                           '("target") points)))
    (should (equal (gnosis-model-camera-visibility nil '(0 0 1.2) points) '(1 . 1)))
    (should (member '(-25.0 0.0 1.2) views))
    (should-not (member '(25.0 0.0 1.2) views))
    (should (equal (gnosis-model-camera-visibility nil '(25 0 1.2) points) '(0 . 0)))))

(defun gnosis-test-model-camera--occluder-scene ()
  "Import a low-poly scene whose single wall hides the target at negative yaw.
The target triangle is visible at yaw 0; at yaw -10 and beyond each of
its vertices projects inside the nearer wall triangle."
  (let ((dir (expand-file-name "occluder" gnosis-dir)))
    (make-directory dir t)
    (with-temp-file (expand-file-name "target.obj" dir)
      (insert "v -0.01 -0.01 0\nv 0.01 -0.01 0\nv 0 0.01 0\nf 1 2 3\n"))
    (with-temp-file (expand-file-name "wall.obj" dir)
      (insert "v 0.05 -0.5 0.5\nv 0.8 -0.5 0.5\nv 0.05 0.5 0.5\nf 1 2 3\n"))
    (with-temp-file (expand-file-name "scene.json" dir)
      (insert "{\"version\":2,\"objects\":[{\"id\":\"target\",\"label\":\"Target\",\"path\":\"target.obj\"},{\"id\":\"wall\",\"label\":\"Wall\",\"path\":\"wall.obj\"}],\"targets\":[{\"id\":\"target\",\"label\":\"Target\",\"mesh\":\"target\",\"kind\":\"object\"},{\"id\":\"wall\",\"label\":\"Wall\",\"mesh\":\"wall\",\"kind\":\"object\"},{\"id\":\"tip\",\"label\":\"Tip\",\"mesh\":\"target\",\"kind\":\"point\",\"face\":0,\"barycentric\":[0.34,0.33,0.33],\"tolerance\":0.01},{\"id\":\"patch\",\"label\":\"Patch\",\"mesh\":\"target\",\"kind\":\"region\",\"faces\":[0]}],\"initial_view\":[0,0,1],\"license\":\"CC0\",\"source\":\"Gnosis ERT fixture\"}"))
    (gnosis-model-import (expand-file-name "scene.json" dir))))

(ert-deftest gnosis-model-camera-rejects-low-poly-occluder ()
  "One large wall triangle hiding the whole target rejects those cameras."
  (gnosis-test-with-db
    (let ((reference (gnosis-test-model-camera--occluder-scene)))
      (dolist (target '("target" "tip" "patch"))
        (let ((views (plist-get
                      (plist-get (gnosis-model-fields
                                  "model-name" (list reference target "0" "0" "1")
                                  '("Target") nil '(25 0 0))
                                 :verified)
                      :views)))
          (should (member '(25.0 0.0 1.0) views))
          (should-not (seq-find (lambda (view) (< (car view) 0)) views)))))))

(defun gnosis-test-model-camera--import (name objects)
  "Import scene NAME of OBJECTS, a list of (ID . OBJ-TEXT), viewed at (0 0 1)."
  (let ((dir (expand-file-name name gnosis-dir)))
    (make-directory dir t)
    (pcase-dolist (`(,id . ,text) objects)
      (with-temp-file (expand-file-name (concat id ".obj") dir) (insert text)))
    (with-temp-file (expand-file-name "scene.json" dir)
      (insert (format "{\"objects\":[%s],\"initial_view\":[0,0,1],\"license\":\"CC0\",\"source\":\"Gnosis ERT fixture\"}"
                      (mapconcat (lambda (object)
                                   (format "{\"id\":\"%s\",\"label\":\"%s\",\"path\":\"%s.obj\"}"
                                           (car object) (capitalize (car object)) (car object)))
                                 objects ","))))
    (gnosis-model-import (expand-file-name "scene.json" dir))))

(defun gnosis-test-model-camera--name-views (reference)
  "Return the varied cameras prepared for a Name card on REFERENCE's target."
  (plist-get (plist-get (gnosis-model-fields "model-name" (list reference "target" "0" "0" "1")
                                             '("Target") nil '(25 15 0.1))
                        :verified)
             :views))

(ert-deftest gnosis-model-camera-rejects-close-foreground-wall ()
  "A wall only slightly nearer than the target still hides it completely."
  (gnosis-test-with-db
    (let ((views (gnosis-test-model-camera--name-views
                  (gnosis-test-model-camera--import
                   "close"
                   '(("target" . "v -0.015 -0.015 0\nv 0.015 -0.015 0\nv 0 0.015 0\nf 1 2 3\n")
                     ("wall" . "v 0.02 -0.5 0.09\nv 0.5 -0.5 0.09\nv 0.02 0.5 0.09\nf 1 2 3\n")
                     ("background" . "v -1 -1 -0.7\nv 1 -1 -0.7\nv 0 1 -0.7\nf 1 2 3\n"))))))
      (should (member '(25.0 0.0 1.0) views))
      (should-not (member '(-25.0 0.0 1.0) views)))))

(ert-deftest gnosis-model-camera-considers-every-occluder-face ()
  "The only large face of a dense wall hides the target wherever it is listed."
  (gnosis-test-with-db
    (dolist (index '(1 2001 3999 0))
      (let ((views (gnosis-test-model-camera--name-views
                    (gnosis-test-model-camera--import
                     (format "dense-%d" index)
                     `(("target" . "v -0.01 -0.01 0\nv 0.01 -0.01 0\nv 0 0.01 0\nf 1 2 3\n")
                       ("wall" . ,(concat "v 0.05 -0.5 0.5\nv 0.8 -0.5 0.5\nv 0.05 0.5 0.5\n"
                                          "v 0.75 -0.48 0.5\nv 0.76 -0.48 0.5\nv 0.75 -0.47 0.5\n"
                                          (mapconcat (lambda (face)
                                                       (if (= face index) "f 1 2 3\n" "f 4 5 6\n"))
                                                     (number-sequence 0 4000) ""))))))))
        (should (member '(25.0 0.0 1.0) views))
        (should-not (seq-find (lambda (view) (< (car view) 0)) views))))))

(ert-deftest gnosis-model-camera-occlusion-uses-triangle-coverage ()
  "The wall's centroid lies elsewhere; its area still hides the target."
  (let* ((geometry '(("target" . [((-0.01 -0.01 0) (0.01 -0.01 0) (0 0.01 0))])
                     ("wall" . [((0.05 -0.5 0.5) (0.8 -0.5 0.5) (0.05 0.5 0.5))])))
         (frame '((0 0 0) . 1))
         (points (gnosis-model-camera-target-points (cdr (assoc "target" geometry)) frame))
         (occluders (gnosis-model-camera-occluders geometry frame '("target") points
                                                   '((0 0 1) (-25 0 1)))))
    (should (= (length occluders) 1))
    (should (equal (gnosis-model-camera-visibility occluders '(0 0 1) points)
                   (cons (length points) (length points))))
    (should (zerop (car (gnosis-model-camera-visibility occluders '(-25 0 1) points))))
    (should (equal (gnosis-model-camera-views geometry frame '(0 0 1) '(25 0 0) '("target") points)
                   '((25.0 0.0 1.0) (15.0 0.0 1.0) (10.0 0.0 1.0))))))

(ert-deftest gnosis-model-camera-hidden-authored-target-has-no-views ()
  "Without a visible baseline nothing is known, so keep the authored camera."
  (let* ((geometry '(("target" . [((-0.01 -0.01 0) (0.01 -0.01 0) (0 0.01 0))])
                     ("wall" . [((-0.5 -0.5 0.5) (0.5 -0.5 0.5) (0 0.5 0.5))])))
         (frame '((0 0 0) . 1)))
    (should-not (gnosis-model-camera-views
                 geometry frame '(0 0 1) '(25 0 0) '("target")
                 (gnosis-model-camera-target-points (cdr (assoc "target" geometry)) frame)))))

(ert-deftest gnosis-model-camera-candidates-respect-fractional-bounds ()
  "Non-grid authored cameras never move further than the bounds allow."
  (should-not (gnosis-model-camera-candidates '(10.24 -90.24 1.004) '(0 0 0)))
  (should-not (gnosis-model-camera-candidates '(0 -90 1) '(0 0 0)))
  (dolist (case '(((10.24 -90.24 1.004) (0.01 0.01 0.001))
                  ((10.24 -90.24 1.004) (25 15 0.1))
                  ((-33.3 12.7 3.98) (0.7 0.3 0.1))
                  ((0 0 0.26) (25 15 0.5))
                  ((0.25 -0.25 1.0) (0.25 0.25 0.005))))
    (pcase-let ((`((,yaw ,pitch ,zoom) (,dyaw ,dpitch ,dzoom)) case))
      (dolist (view (gnosis-model-camera-candidates (car case) (cadr case)))
        (should (<= (abs (- (nth 0 view) yaw)) dyaw))
        (should (<= (abs (- (nth 1 view) pitch)) dpitch))
        (should (<= (abs (- (nth 2 view) zoom)) (* dzoom zoom)))
        (should (<= 0.25 (nth 2 view) 4))
        (should-not (equal view (car case))))))
  (should (equal (car (gnosis-model-camera-candidates '(10.24 -90.24 1.004) '(25 15 0.1)))
                 '(35.24 -90.24 1.004))))

(ert-deftest gnosis-model-camera-occluders-are-bounded ()
  "Surfaces off every line of sight are skipped; too many others refuse."
  (let* ((tiny (lambda (x y z) (list (list x y z) (list (+ x 0.001) y z) (list x (+ y 0.001) z))))
         (geometry `(("target" . [((-0.01 -0.01 0) (0.01 -0.01 0) (0 0.01 0))])
                     ;; Behind the target, and beside it beyond the bounds' angle.
                     ("behind" . ,(make-vector 500 (funcall tiny 0 0 -0.5)))
                     ("beside" . ,(make-vector 500 (funcall tiny 0.9 0 0.1)))
                     ("wall" . [((0.05 -0.5 0.5) (0.8 -0.5 0.5) (0.05 0.5 0.5))])))
         (frame '((0 0 0) . 1))
         (points (gnosis-model-camera-target-points (cdr (assoc "target" geometry)) frame)))
    (should (= 1 (length (gnosis-model-camera-occluders
                          geometry frame '("target") points
                          (cons '(0 0 1) (gnosis-model-camera-candidates '(0 0 1) '(25 15 0.1)))))))
    ;; Views facing opposite ways see past the target from behind too.
    (should (= 1001 (length (gnosis-model-camera-occluders
                             geometry frame '("target") points '((0 0 1) (180 0 1))))))
    (should (gnosis-model-camera-views geometry frame '(0 0 1) '(25 15 0.1) '("target") points))
    (let ((gnosis-model-camera--occluders 0))
      (should-not (gnosis-model-camera-views geometry frame '(0 0 1) '(25 15 0.1)
                                             '("target") points)))))

(ert-deftest gnosis-model-review-view-requires-current-bounds ()
  "Views prepared under other bounds fall back to the authored camera."
  (gnosis-test-with-db
    (let* ((id (gnosis-test-model--add))
           (fields (list :view '(0 -90 1)
                         :verified (list :views '((25 -90 1)) :camera '(25 15 0.1)))))
      (let ((gnosis-model-camera-variation '(25 15 0.1)))
        (should (equal (gnosis-model-review-view fields id) '(25 -90 1))))
      (let ((gnosis-model-camera-variation '(5 5 0.05)))
        (should (equal (gnosis-model-review-view fields id) '(0 -90 1)))))))

(ert-deftest gnosis-model-camera-choice-is-deterministic-per-review ()
  (let ((views '((1 0 1) (2 0 1) (3 0 1))))
    (should-not (gnosis-model-camera-choose nil 7 0))
    (should (equal (gnosis-model-camera-choose views 7 4) (gnosis-model-camera-choose views 7 4)))
    (should (= 3 (length (delete-dups (mapcar (lambda (n) (gnosis-model-camera-choose views 7 n))
                                              '(0 1 2))))))))

(ert-deftest gnosis-model-review-view-varies-by-review-count ()
  "Reviews step through prepared views; the off switch keeps the authored view."
  (gnosis-test-with-db
    (let* ((id (gnosis-test-model--add))
           (fields (list :view '(0 -90 1)
                         :verified (list :views '((5 -90 1) (-5 -90 1))
                                         :camera gnosis-model-camera-variation)))
           (first (gnosis-model-review-view fields id)))
      (should (member first '((5 -90 1) (-5 -90 1))))
      (should (equal first (gnosis-model-review-view fields id)))
      (gnosis-scheduler-accept-review (gnosis-scheduler-event-id) id (quote success)
                                      1780000000000000 (gnosis--today-int))
      (should-not (equal first (gnosis-model-review-view fields id)))
      (let ((gnosis-model-camera-variation nil))
        (should (equal (gnosis-model-review-view fields id) '(0 -90 1))))
      (should (equal (gnosis-model-review-view (list :view '(0 -90 1) :verified nil) id)
                     '(0 -90 1))))))

(ert-deftest gnosis-model-fields-prepare-bounded-visible-views ()
  (gnosis-test-with-db
    (let* ((reference (gnosis-model-import (gnosis-test-model--scene)))
           (plain (gnosis-model-fields "model" (list reference "0" "-90" "1") '("triangle")))
           (views (plist-get (plist-get (gnosis-model-fields
                                         "model" (list reference "0" "-90" "1") '("triangle")
                                         nil '(20 10 0.1))
                                        :verified)
                             :views)))
      (should-not (plist-get (plist-get plain :verified) :views))
      (should views)
      (dolist (view views)
        (should (<= (abs (nth 0 view)) 20))
        (should (<= (abs (+ 90 (nth 1 view))) 10))
        (should (<= 0.9 (nth 2 view) 1.1))))))

(ert-deftest gnosis-model-author-scene-saves-find-and-name-cards ()
  "Click, key, next: each key saves a card at the current camera."
  (gnosis-test-with-db
    (save-window-excursion
      (let* ((reference (gnosis-model-import (gnosis-test-model--scene)))
             (prompts nil)
             (depth 0))
        (cl-letf (((symbol-function 'gnosis-model-open) #'gnosis-test-model--canvas)
                  ((symbol-function 'gnosis-model--canvas-size) (lambda () 400))
                  ((symbol-function 'canvas-3d--request) #'ignore)
                  ((symbol-function 'recursion-depth) (lambda () depth))
                  ((symbol-function 'read-string)
                   (lambda (prompt &optional initial &rest _)
                     (push (cons prompt initial) prompts) initial))
                  ((symbol-function 'recursive-edit)
                   (lambda ()
                     (setq depth 1)
                     (should (string-match-p "F find card.*0 saved"
                                             (gnosis-model--author-header)))
                     (should-error (call-interactively (key-binding (kbd "F")))
                                   :type 'user-error)
                     (gnosis-test-model-camera--pick)
                     (setq canvas-3d--yaw 347.5 canvas-3d--pitch 315 canvas-3d--zoom 1.25)
                     (call-interactively (key-binding (kbd "F")))
                     (call-interactively (key-binding (kbd "N")))
                     (should (string-match-p "2 saved" (gnosis-model--author-header)))
                     ;; RET would end the session with an unsaved result.
                     (should (eq (key-binding (kbd "RET")) 'ignore))
                     (setf (plist-get gnosis-model--author-context :cancelled) t))))
          (should (= 2 (gnosis-model-author-scene reference '("anatomy")))))
        (should (equal (reverse prompts)
                       '(("Find card question: " . "Click the other triangle.")
                         ("Name card answer: " . "Other triangle"))))
        (should (equal (sort (gnosis-select '[type keimenon hypothesis answer] 'themata)
                             (lambda (a b) (string< (car a) (car b))))
                       `(("model" "Click the other triangle."
                          (,reference "-12.5" "-45" "1.25") ("other"))
                         ("model-name" "Name the highlighted structure."
                          (,reference "other" "-12.5" "-45" "1.25") ("Other triangle")))))
        (should (equal (gnosis-select 'tag 'thema-tag nil t) '("anatomy" "anatomy")))))))

(ert-deftest gnosis-model-author-scene-quit-without-cards-is-cancelled ()
  (gnosis-test-with-db
    (save-window-excursion
      (let ((reference (gnosis-model-import (gnosis-test-model--scene))))
        (cl-letf (((symbol-function 'gnosis-model-open) #'gnosis-test-model--canvas)
                  ((symbol-function 'gnosis-model--canvas-size) (lambda () 400))
                  ((symbol-function 'recursive-edit)
                   (lambda () (setf (plist-get gnosis-model--author-context :cancelled) t))))
          (should-error (gnosis-model-author-scene reference nil) :type 'user-error)
          (should-error (gnosis-model-author-scene reference '("bad tag")) :type 'user-error)
          (should-error (gnosis-model-author-scene (concat (make-string 64 ?0) "/scene.json") nil)
                        :type 'user-error))
        (should-not (gnosis-select '* 'themata))
        (should-not (gnosis-model--scene-resources))
        (gnosis-add-thema-fields "model" "Find" (list reference "0" "-90" "1") '("triangle")
                                 "" nil 0 nil)
        (should (equal (gnosis-model--scene-resources)
                       `((,(format "Triangle (2 objects, %s)" (substring reference 0 8))
                          . ,reference))))))))

(provide 'gnosis-test-model-camera)
;;; gnosis-test-model-camera.el ends here
