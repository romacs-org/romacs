;;; gnosis-test-model-relational.el --- Cue targets and any-of model answers -*- lexical-binding: t; -*-

;; Copyright (C) 2026 Free Software Foundation, Inc.

;;; Commentary:
;; Relational Find cards: optional cue target IDs after the camera in the
;; hypothesis, and any-of acceptable targets in the answer.  Validation,
;; the Org codec, grading, native cue/reveal layers and scene authoring.

;;; Code:
(require 'ert)
(require 'gnosis-model-test-support)

(defun gnosis-test-relational--scene ()
  "Import a three-object scene and return its managed reference."
  (let ((dir (expand-file-name "relational" gnosis-dir)))
    (make-directory dir t)
    (with-temp-file (expand-file-name "triangle.obj" dir)
      (insert "v 0 0 0\nv 1 0 0\nv 0 1 0\nf 1 2 3\n"))
    (with-temp-file (expand-file-name "scene.json" dir)
      (insert "{\"objects\":["
              "{\"id\":\"triangle\",\"label\":\"Triangle\",\"path\":\"triangle.obj\"},"
              "{\"id\":\"other\",\"label\":\"Other triangle\",\"path\":\"triangle.obj\"},"
              "{\"id\":\"third\",\"label\":\"Third triangle\",\"path\":\"triangle.obj\"}],"
              "\"initial_view\":[0,-90,1],\"license\":\"CC0; original test geometry\","
              "\"source\":\"Gnosis ERT fixture\"}"))
    (gnosis-model-import (expand-file-name "scene.json" dir))))

(ert-deftest gnosis-model-relational-fields-validate-cues-and-answers ()
  "Cue IDs follow the camera; the answer lists every acceptable target."
  (gnosis-test-with-db
    (let* ((reference (gnosis-test-relational--scene))
           (camera (list reference "0" "-90" "1"))
           (plain (gnosis-model-fields "model" camera '("triangle")))
           (fields (gnosis-model-fields "model" (append camera '("triangle"))
                                        '("other" "third"))))
      ;; Ordinary Finds are unchanged.
      (should (equal (plist-get plain :target) "triangle"))
      (should-not (plist-get plain :cues))
      (should (equal (plist-get plain :answers) '("triangle")))
      (should (equal (plist-get fields :target) "other"))
      (should (equal (plist-get fields :cues) '("triangle")))
      (should (equal (plist-get fields :answers) '("other" "third")))
      (should (equal (plist-get fields :view) '(0 -90 1)))
      ;; A cued answer is allowed when another cue is a distractor.
      (should (gnosis-model-fields "model" (append camera '("triangle" "other")) '("other")))
      (dolist (bad `((,(append camera '("missing")) ("other"))
                     (,(append camera '("triangle" "triangle")) ("other"))
                     (,(append camera '("triangle")) ("other" "other"))
                     (,(append camera '("triangle")) ("other" "missing"))
                     (,(append camera '("triangle")) ("other" ""))
                     ;; Cueing exactly the answers would show them.
                     (,(append camera '("other" "third")) ("third" "other"))
                     (,(append camera '("triangle")) nil)
                     (,(butlast camera) ("triangle"))))
        (should-error (gnosis-model-fields "model" (car bad) (cadr bad)) :type 'user-error))
      ;; Name cards keep exactly one target and one typed answer.
      (should-error (gnosis-model-fields "model-name"
                                         (list reference "triangle" "0" "-90" "1" "other")
                                         '("Triangle"))
                    :type 'user-error)
      (should-error (gnosis-model-fields "model-name" (list reference "triangle" "0" "-90" "1")
                                         '("Triangle" "Other"))
                    :type 'user-error))))

(ert-deftest gnosis-model-relational-store-edit-roundtrip ()
  "The native writer, Org editor codec and update path keep every target."
  (gnosis-test-with-db
    (let* ((reference (gnosis-test-relational--scene))
           (hypothesis (list reference "10" "-80" "1.25" "triangle"))
           (id (gnosis-add-thema-fields "model" "Click a structure beside the triangle."
                                        hypothesis '("other" "third") "" '("rel") 0 nil)))
      (should (equal (gnosis-select '[hypothesis answer] 'themata `(= id ,id))
                     `((,hypothesis ("other" "third")))))
      (with-temp-buffer
        (gnosis-edit-mode)
        (gnosis-export--insert-themata (list id))
        (let ((thema (car (gnosis-export-parse-themata))))
          (should (equal (nth 3 thema) hypothesis))
          (should (equal (nth 4 thema) '("other" "third")))
          (should-not (gnosis-save-thema thema))))
      (should (equal (gnosis-select '[hypothesis answer] 'themata `(= id ,id))
                     `((,hypothesis ("other" "third")))))
      ;; Invalid relational content is refused at the shared save path.
      (should-error (gnosis-add-thema-fields "model" "Q" (append hypothesis '("nope"))
                                             '("other") "" nil 0 nil)
                    :type 'user-error)
      (should-error (gnosis-update-thema id "Q" (list reference "0" "-90" "1" "other")
                                         '("other") "" nil nil)
                    :type 'user-error))))

(ert-deftest gnosis-model-relational-attach-preserves-cues-and-alternates ()
  "Reframing a relational draft keeps its cues and other acceptable targets.
Only the camera and primary target come from the viewer."
  (gnosis-test-with-db
    (save-window-excursion
      (let* ((reference (gnosis-test-relational--scene)))
        (with-temp-buffer
          (gnosis-edit-mode)
          (gnosis-export--insert-thema
           "NEW" "model" "Question"
           (mapconcat #'identity (list reference "0" "-90" "1" "triangle") gnosis-export-separator)
           (mapconcat #'identity '("other" "third") gnosis-export-separator) "" nil)
          (cl-letf (((symbol-function 'y-or-n-p) (lambda (&rest _) t))
                    ((symbol-function 'gnosis-model--read-visual)
                     (lambda (hypothesis answer _context)
                       (should (equal hypothesis (list reference "0" "-90" "1")))
                       (should (equal answer '("other")))
                       (list (list reference "-30" "-50" "1.5") '("other")))))
            (call-interactively #'gnosis-model-attach))
          (let ((thema (car (gnosis-export-parse-themata))))
            (should (equal (nth 3 thema) (list reference "-30" "-50" "1.5" "triangle")))
            (should (equal (nth 4 thema) '("other" "third")))))))))

(ert-deftest gnosis-model-relational-review-accepts-any-listed-target ()
  "Any acceptable target grades correct; the cue or another target is wrong.
Feedback lists the whole acceptable set."
  (gnosis-test-with-db
    (save-window-excursion
      (let* ((reference (gnosis-test-relational--scene))
             (gnosis-review-buffer-name " *Gnosis relational*")
             cued)
        (dolist (case '(("third" t) ("other" t) ("triangle" nil)))
          (let* ((model (gnosis-add-thema-fields
                         "model" "Click a structure beside the triangle."
                         (list reference "0" "-90" "1" "triangle") '("other" "third")
                         "" nil 0 nil))
                 (buffer (gnosis-review--setup-buffer (list model) 'practice)))
            (unwind-protect
                (with-current-buffer buffer
                  (cl-letf (((symbol-function 'gnosis-model-backend-cue)
                             (lambda (targets) (setq cued (mapcar (lambda (target) (alist-get 'id target))
                                                                   targets))
                               nil))
                            ((symbol-function 'gnosis-model-backend-highlight) #'ignore))
                    (let* ((display (gnosis-test-model--encounter
                                      (setq-local canvas-3d-selected-id (car case))
                                      (gnosis-test-model--review-pick (car case) canvas-3d--process)
                                      (gnosis-review-model-submit)))
                           (pair (cadr display)))
                      (should (equal cued '("triangle")))
                      (should (eq (and (car pair) t) (cadr case)))
                      (should (string-match-p "Answer: Other triangle or Third triangle"
                                              (buffer-string)))
                      (unless (cadr case)
                        (should (string-match-p "Your answer: Triangle" (buffer-string)))))))
              (kill-buffer buffer))))))))

(ert-deftest gnosis-model-relational-show-cues-whole-set-or-nothing ()
  "Backends without cue layers paint no cue, never a partial set.
A partial set can leak the answer: a contrast card cues its answer and a
distractor, and painting only the first would show the answer alone."
  (let* ((targets '(((id . "answer") (label . "Answer") (mesh . "a") (kind . "object"))
                    ((id . "distractor") (label . "Distractor") (mesh . "d") (kind . "object"))))
         (scene (list (cons 'version 2) (cons 'targets targets)))
         (layers nil) calls)
    (cl-letf (((symbol-function 'gnosis-model-backend-cue)
               (lambda (cues) (push (list 'cue (mapcar (lambda (c) (alist-get 'id c)) cues)) calls)
                 layers))
              ((symbol-function 'gnosis-model-backend-highlight)
               (lambda (target selected) (push (list 'highlight target selected) calls))))
      (should-not (gnosis-model-show-cues (list :cues nil)))
      (should-not calls)
      (dolist (cues '(("answer" "distractor") ("distractor" "answer") ("distractor")))
        (gnosis-model--check-relation scene cues '("answer"))
        (let ((fields (list :target "answer" :answers '("answer") :cues cues
                            :verified (list :manifest scene))))
          (setq calls nil layers nil)
          (should-not (gnosis-model-show-cues fields))
          (should (equal calls (list (list 'cue cues))))
          (should (equal (gnosis-model-cue-text fields)
                         (concat (if (cdr cues) "Cues: " "Cue: ")
                                 (mapconcat #'capitalize cues ", "))))
          (setq calls nil layers t)
          (should (gnosis-model-show-cues fields))
          (should (equal calls (list (list 'cue cues)))))))
    (should-not (gnosis-model-cue-text (list :cues nil)))))

(defun gnosis-test-relational--canvas-review (cues answers pick)
  "Review a canvas Find cueing CUES with ANSWERS, picking PICK.
Return (QUESTION-TEXT QUESTION-TARGET-ID FEEDBACK-REQUESTS SUCCESS) where
FEEDBACK-REQUESTS are the (TARGET-ID . SELECTED-ID) renderer requests
issued after submission.  Only the renderer process boundary is stubbed."
  (let* ((reference (gnosis-test-relational--scene))
         (gnosis-review-buffer-name " *Gnosis relational canvas*")
         (model (gnosis-add-thema-fields
                 "model" "Click the structure." (append (list reference "0" "-90" "1") cues)
                 answers "" nil 0 nil))
         (buffer (gnosis-review--setup-buffer (list model) 'practice))
         requests question question-target submitted)
    (unwind-protect
        (with-current-buffer buffer
          (cl-letf (((symbol-function 'canvas-3d--request)
                     (lambda () (push (cons (alist-get 'id canvas-3d--question-target)
                                            canvas-3d-selected-id)
                                      requests))))
            (let ((display (gnosis-test-model--encounter
                             (setq question (buffer-string)
                                   question-target (alist-get 'id canvas-3d--question-target))
                             (setq-local canvas-3d-selected-id pick)
                             (gnosis-test-model--review-pick pick canvas-3d--process)
                             (setq submitted (length requests))
                             (gnosis-review-model-submit))))
              (list question question-target
                    (reverse (butlast requests submitted))
                    (and (car (cadr display)) t)))))
      (kill-buffer buffer))))

(ert-deftest gnosis-model-relational-canvas-question-and-feedback ()
  "Canvas reviews name a multi-cue set, never paint part of it, and reveal
the correct answer rather than the cue after submission."
  (gnosis-test-with-db
    (save-window-excursion
      (dolist (case '((("other" "triangle") ("other") "other" t "other"
                       "Cues: Other triangle, Triangle")
                      (("triangle" "other") ("other") "triangle" nil "other"
                       "Cues: Triangle, Other triangle")
                      (("triangle") ("other") "triangle" nil "other" "Cue: Triangle")
                      (("triangle") ("other") "other" t "other" "Cue: Triangle")
                      ;; Any-of: a correct alternate shows the alternate picked.
                      (("triangle") ("other" "third") "third" t "third" "Cue: Triangle")
                      (("triangle") ("other" "third") "triangle" nil "other" "Cue: Triangle")))
        (pcase-let ((`(,cues ,answers ,pick ,success ,shown ,text) case))
          (pcase-let ((`(,question ,question-target ,feedback ,graded)
                       (gnosis-test-relational--canvas-review cues answers pick)))
            (should (string-match-p (regexp-quote text) question))
            ;; No question highlight: picks remain visible and no cue leaks alone.
            (should-not question-target)
            (should (eq graded success))
            (should feedback)
            (should (equal (car (last feedback)) (cons shown nil)))))))))

(ert-deftest gnosis-model-relational-plain-find-has-no-cue-text ()
  "Ordinary Finds keep their question unchanged on canvas."
  (gnosis-test-with-db
    (save-window-excursion
      (pcase-let ((`(,question ,question-target ,feedback ,graded)
                   (gnosis-test-relational--canvas-review nil '("other") "other")))
        (should-not (string-match-p "Cue" question))
        (should-not question-target)
        (should graded)
        (should (equal (car (last feedback)) (cons "other" nil)))))))

(ert-deftest gnosis-model-relational-cue-cards-keep-authored-camera ()
  "Varied cameras are prepared for the answer only, so a cue could leave
the frame; relational cards review from their authored camera."
  (gnosis-test-with-db
    (let* ((reference (gnosis-test-relational--scene))
           (camera (list reference "0" "-90" "1")))
      (cl-letf (((symbol-function 'gnosis-model--camera-views)
                 (lambda (&rest _) '((10 -90 1)))))
        (let ((plain (gnosis-model-fields "model" camera '("other") nil '(20 10 0.1)))
              (cued (gnosis-model-fields "model" (append camera '("triangle")) '("other")
                                         nil '(20 10 0.1))))
          (should (plist-get (plist-get plain :verified) :views))
          (should-not (plist-get (plist-get cued :verified) :views))
          (let ((gnosis-model-camera-variation '(20 10 0.1)))
            (should (equal (gnosis-model-review-view cued 1) '(0 -90 1)))))))))

(defconst gnosis-test-relational--mixed
  '(((id . "cue") (label . "Cue") (mesh . "cue-m") (kind . "object"))
    ((id . "obj") (label . "Object") (mesh . "object-m") (kind . "object"))
    ((id . "pt") (label . "Point") (mesh . "landmark-m") (kind . "point") (face . 0)
     (barycentric . (1 0 0)) (tolerance . 0.1))
    ((id . "reg") (label . "Region") (mesh . "region-m") (kind . "region") (faces . (0))))
  "Mixed object, point and region targets plus an object cue.")

(defun gnosis-test-relational--mixed-fields (answers)
  "Return verified Find fields cueing \"cue\" with ANSWERS."
  (list :response 'find :target (car answers) :answers answers :cues '("cue")
        :verified (list :manifest `((version . 2) (targets . ,gnosis-test-relational--mixed))
                        :radius 10.0
                        :counts '(("cue-m" . 1) ("object-m" . 1) ("landmark-m" . 1)
                                  ("region-m" . 1))
                        :points '(("pt" 0 0 0)))))

(ert-deftest gnosis-model-relational-mixed-kind-selection ()
  "Object, point and region answers resolve and grade independently."
  (dolist (answers '(("obj" "pt" "reg") ("reg" "pt" "obj") ("pt" "obj" "reg")))
    (let ((fields (gnosis-test-relational--mixed-fields answers)) current)
      (cl-letf (((symbol-function 'gnosis-model-check-fields) #'ignore)
                ((symbol-function 'gnosis-model-backend-selection-current-p)
                 (lambda (&rest _) t))
                ((symbol-function 'gnosis-model-backend-get) (lambda (_) (copy-tree current))))
        (dolist (case '(("object-m" (0 0 0) "obj") ("landmark-m" (0.05 0 0) "pt")
                        ("region-m" (0 0 0) "reg") ("cue-m" (0 0 0) "cue")
                        ("landmark-m" (0.2 0 0) nil)))
          (setq current (list :mesh (nth 0 case) :face 0 :point (nth 1 case)))
          (let ((id (plist-get (gnosis-model-selection fields) :id)))
            (should (equal id (nth 2 case)))
            (should (eq (gnosis-model-correct-p fields id)
                        (and (member id '("obj" "pt" "reg")) t)))))))))

(ert-deftest gnosis-model-relational-mixed-kind-presentation-ignores-order ()
  "Pick marking and wrong feedback depend on the whole answer set, not its
first entry: any point or region answer marks the picked spot, and only an
all-object set paints a wrong mesh red."
  (let (calls)
    (cl-letf (((symbol-function 'gnosis-model-backend-reveal)
               (lambda (&rest args) (push (nth 2 args) calls) t))
              ((symbol-function 'gnosis-review--feedback-picture-size) (lambda (_) nil)))
      (dolist (case '((("obj" "pt") spot) (("pt" "obj") spot) (("reg" "obj") spot)
                      (("obj") mesh)))
        (let ((fields (gnosis-test-relational--mixed-fields (car case))))
          (should (eq (and (gnosis-review--model-pick-radius fields) t)
                      (eq (cadr case) 'spot)))
          (with-temp-buffer
            (setq-local gnosis-review--model-context
                        (list :process 'p :attachment 'a :view '(0 0 1) :fields fields
                              :selection '(:id "cue" :mesh "cue-m" :face 0 :point (0 0 0)
                                           :barycentric (1 0 0))))
            (setq calls nil)
            (gnosis-review--model-reveal gnosis-review--model-context nil)
            (let ((reveal (car calls)))
              (should (equal (alist-get 'id (plist-get reveal :target)) (car (car case))))
              (if (eq (cadr case) 'spot)
                  (progn (should-not (plist-get reveal :wrong-mesh))
                         (should (equal (plist-get (plist-get reveal :wrong-pick) :mesh)
                                        "cue-m")))
                (should (equal (plist-get reveal :wrong-mesh) "cue-m"))
                (should-not (plist-get reveal :wrong-pick))))))))))

(ert-deftest gnosis-model-relational-reveal-arguments ()
  "Reveal adds the cue and the other acceptable targets for relational cards."
  (let* ((targets '(((id . "a") (label . "A") (mesh . "m1") (kind . "object"))
                    ((id . "b") (label . "B") (mesh . "m2") (kind . "object"))
                    ((id . "c") (label . "C") (mesh . "m3") (kind . "object"))))
         (scene (list (cons 'version 2) (cons 'targets targets)))
         calls)
    (cl-letf (((symbol-function 'gnosis-model-backend-reveal)
               (lambda (&rest args) (push args calls) t))
              ((symbol-function 'gnosis-review--feedback-picture-size) (lambda (_) nil)))
      (with-temp-buffer
        (setq-local gnosis-review--model-context
                    (list :process 'p :attachment 'a :view '(0 0 1)
                          :fields (list :response 'find :target "b" :answers '("b" "c")
                                        :cues '("a")
                                        :verified (list :manifest scene :radius 10.0))
                          :selection '(:id "a" :mesh "m1" :face 0 :point (1 2 3))))
        (gnosis-review--model-reveal gnosis-review--model-context nil)
        (let ((reveal (nth 2 (car calls))))
          (should (equal (plist-get reveal :target) (nth 1 targets)))
          (should (equal (plist-get reveal :wrong-mesh) "m1"))
          (should (equal (plist-get reveal :cues) (list (nth 0 targets))))
          (should (equal (plist-get reveal :alternates) (list (nth 2 targets)))))))))

(ert-deftest gnosis-model-relational-author-from-scene ()
  "r marks the reference, then each acceptable answer; R saves the card."
  (gnosis-test-with-db
    (save-window-excursion
      (let* ((reference (gnosis-test-relational--scene))
             (prompts nil) (cues nil)
             (depth 0))
        (cl-letf (((symbol-function 'gnosis-model-open) #'gnosis-test-model--canvas)
                  ((symbol-function 'gnosis-model--canvas-size) (lambda () 400))
                  ((symbol-function 'canvas-3d--request) #'ignore)
                  ((symbol-function 'recursion-depth) (lambda () depth))
                  ((symbol-function 'gnosis-model-backend-cue)
                   (lambda (targets) (push (mapcar (lambda (target) (alist-get 'id target)) targets)
                                           cues)
                     nil))
                  ((symbol-function 'completing-read)
                   (lambda (prompt &rest _) (push (cons prompt nil) prompts) "posterior to"))
                  ((symbol-function 'read-string)
                   (lambda (prompt &optional initial &rest _)
                     (push (cons prompt initial) prompts) initial))
                  ((symbol-function 'recursive-edit)
                   (lambda ()
                     (setq depth 1)
                     (cl-flet ((pick (id)
                                 (setq-local canvas-3d-selected-id id)
                                 (run-hook-with-args 'gnosis-model-backend-selection-hook
                                                     (list :id id :frame 1 :owner canvas-3d--process)))
                               (press (key) (call-interactively (key-binding (kbd key)))))
                       (should (string-match-p "r relation" (gnosis-model--author-header)))
                       (should-error (press "R") :type 'user-error)
                       (pick "triangle") (press "r")
                       (should (string-match-p "Cue Triangle" (gnosis-model--author-header)))
                       ;; Saving needs an answer besides the cue.
                       (should-error (press "R") :type 'user-error)
                       (pick "other") (press "r")
                       (pick "third") (press "r")
                       (should (string-match-p "Cue Triangle → Other triangle, Third triangle"
                                               (gnosis-model--author-header)))
                       ;; Pressing r again on an answer removes it.
                       (press "r")
                       (pick "third") (press "r")
                       (setq canvas-3d--yaw 20 canvas-3d--pitch -80 canvas-3d--zoom 1.5)
                       (press "R")
                       (should (string-match-p "1 saved" (gnosis-model--author-header)))
                       (should-not (string-match-p "Cue" (gnosis-model--author-header)))
                       ;; r on the cue itself clears a pending relation.
                       (pick "other") (press "r") (press "r")
                       (should-not (plist-get gnosis-model--author-context :relation))
                       (setf (plist-get gnosis-model--author-context :cancelled) t)))))
          (should (= 1 (gnosis-model-author-scene reference '("anatomy")))))
        (should (equal (reverse prompts)
                       '(("Relation: ")
                         ("Relation card question: "
                          . "Click a structure posterior to the triangle."))))
        (should (equal (car (last cues)) '("triangle")))
        (should (equal (car cues) nil))
        (should (equal (gnosis-select '[type keimenon hypothesis answer] 'themata)
                       `(("model" "Click a structure posterior to the triangle."
                          (,reference "20" "-80" "1.5" "triangle") ("other" "third")))))
        (should (equal (gnosis-select 'tag 'thema-tag nil t) '("anatomy")))))))

(provide 'gnosis-test-model-relational)
;;; gnosis-test-model-relational.el ends here
