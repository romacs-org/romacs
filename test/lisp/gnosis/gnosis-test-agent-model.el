;;; gnosis-test-agent-model.el --- Model scenes and cards for agents -*- lexical-binding: t; -*-

;; Copyright (C) 2026 Free Software Foundation, Inc.

;;; Commentary:
;; The agent model API on a disposable database and synthetic scenes:
;; listing and describing scenes, previews that write nothing, explicit
;; saves through the domain validators, retries, refusals, the native
;; renderer's absence in batch, and pick matching against targets.  The
;; native viewer itself is proved in crates/romacs-vm-gui/harness/model_live.py.

;;; Code:
(require 'ert)
(require 'gnosis-agent-model)
(require 'gnosis-model-test-support)

(ert-deftest gnosis-agent-model-scenes-and-describe ()
  (gnosis-test-with-db
    (let ((resource (gnosis-model-import (gnosis-test-model--scene))))
      (gnosis-add-thema-fields "model" "Select triangle" (list resource "0" "-90" "1")
                               '("triangle") "" '("test") 0 nil)
      (let ((scene (aref (plist-get (gnosis-agent-model-scenes) :scenes) 0)))
        (should (equal (plist-get scene :resource) resource))
        (should (equal (plist-get scene :label) "Triangle"))
        (should (equal (list (plist-get scene :objects) (plist-get scene :targets) (plist-get scene :themata))
                       '(2 2 1))))
      (let ((described (gnosis-agent-model-describe resource)))
        (should (equal (plist-get described :initial-view) [0 -90 1]))
        (should (equal (mapcar (lambda (target) (plist-get target :id)) (plist-get described :targets))
                       '("triangle" "other"))))
      (should-error (gnosis-agent-model-describe "../scene.json") :type 'user-error))))

(ert-deftest gnosis-agent-model-preview-writes-nothing-and-save-is-explicit ()
  (gnosis-test-with-db
    (let* ((resource (gnosis-model-import (gnosis-test-model--scene)))
           (plan (gnosis-agent-model-card-preview
                  (list :resource resource :type "find" :target "triangle" :view [30 -60 1.5]
                        :tags ["anatomy"]))))
      (should (equal (plist-get plan :question) "Click the triangle."))
      (should (equal (plist-get plan :hypothesis) (vector resource "30" "-60" "1.5")))
      (should (equal (plist-get plan :answers) ["triangle"]))
      (should-not (gnosis-select 'id 'themata nil t))
      (let* ((receipt (gnosis-agent-model-card-save plan))
             (item (plist-get receipt :item)))
        (should (equal (plist-get item :type) "model"))
        (should (equal (plist-get item :tags) ["anatomy"]))
        (should (equal (plist-get receipt :thema-id) (plist-get item :id)))
        ;; A retry returns the first receipt instead of a second card.
        (should (eq (gnosis-agent-model-card-save plan) receipt))
        (should (= 1 (length (gnosis-select 'id 'themata nil t))))
        ;; Saving writes the card only: no review or practice evidence.
        (should-not (gnosis-select 'event-id 'review-events nil t))))))

(ert-deftest gnosis-agent-model-name-card-uses-the-label-and-hides-it-in-the-question ()
  (gnosis-test-with-db
    (let* ((resource (gnosis-model-import (gnosis-test-model--scene)))
           (plan (gnosis-agent-model-card-preview
                  (list :resource resource :type "name" :target "other"))))
      (should (equal (plist-get plan :type) "model-name"))
      (should (equal (plist-get plan :question) "Name the highlighted structure."))
      (should (equal (plist-get plan :hypothesis) (vector resource "other" "0" "-90" "1")))
      (should (equal (plist-get plan :answers) ["Other triangle"])))))

(ert-deftest gnosis-agent-model-refuses-invalid-and-changed-cards ()
  (gnosis-test-with-db
    (let ((resource (gnosis-model-import (gnosis-test-model--scene))))
      (dolist (spec `((:resource ,resource :type "find" :target "absent")
                      (:resource ,resource :type "quiz" :target "triangle")
                      (:resource ,resource :type "name" :target "triangle" :cues ["other"])
                      (:resource ,resource :type "find" :target "triangle" :answer "x")
                      (:resource ,resource :type "find" :target "triangle" :cues ["triangle"])
                      (:resource ,resource :type "find" :target "triangle" :view [0 0 9])
                      (:resource ,resource :type "find" :target "triangle" :tags ["two words"])
                      (:resource ,resource :type "find" :target "triangle" :unknown 1)))
        (should-error (gnosis-agent-model-card-preview spec)))
      (let ((plan (gnosis-agent-model-card-preview
                   (list :resource resource :type "find" :target "triangle"))))
        (let ((gnosis-review--running "session"))
          (should-error (gnosis-agent-model-card-save plan) :type 'user-error))
        (should-error (gnosis-agent-model-card-save
                       (plist-put (copy-sequence plan) :question "Edited after preview"))
                      :type 'user-error)
        (should-not (gnosis-select 'id 'themata nil t))))))

(ert-deftest gnosis-agent-model-viewer-needs-the-native-renderer ()
  (gnosis-test-with-db
    (let ((resource (gnosis-model-import (gnosis-test-model--scene))))
      (should (eq (plist-get (gnosis-agent-model-status) :available) :false))
      (should (string-match-p "no GPU renderer\\|no native model"
                              (plist-get (gnosis-agent-model-status) :reason)))
      (should (string-match-p "unavailable"
                              (condition-case err (gnosis-agent-model-show resource)
                                (user-error (error-message-string err)))))
      (let ((gnosis-review--running "session"))
        (should (string-match-p "encounter"
                                (condition-case err (gnosis-agent-model-show resource)
                                  (user-error (error-message-string err))))))
      (should (string-match-p "closed or replaced"
                              (condition-case err (gnosis-agent-model-observe "model-viewer-0")
                                (user-error (error-message-string err))))))))

(ert-deftest gnosis-agent-model-backend-choice-refuses-unavailable-rendering ()
  (require 'gnosis-model-backend-romacs)
  (let ((gnosis-model-backend 'romacs))
    (should (string-match-p "Native model renderer unavailable"
                            (condition-case err (gnosis-model-backend--choose)
                              (user-error (error-message-string err))))))
  (let ((gnosis-model-backend 'auto) (gnosis-model-renderer-directory nil))
    (should (string-match-p "off in Romacs: .*text cards still work"
                            (condition-case err (gnosis-model-backend--choose)
                              (user-error (error-message-string err))))))
  (let ((gnosis-model-backend 'auto) (gnosis-model-renderer-directory "/canvas/"))
    (should (eq (gnosis-model-backend--choose) gnosis-model-backend-canvas))))

(ert-deftest gnosis-agent-model-hit-targets-name-every-containing-target ()
  (gnosis-test-with-db
    (let* ((file (gnosis-test-model-targets--fixture))
           (scene (gnosis-model--scene file))
           (geometry (gnosis-model--validate-targets scene (file-name-directory file)))
           (points (list (cons "tip" (gnosis-model--point (gnosis-model-target scene "tip")
                                                          (cdr (assoc "mesh" geometry)))))))
      (should (equal (mapcar (lambda (target) (alist-get 'id target))
                             (gnosis-model-hit-targets scene '(:mesh "mesh" :face 0 :point (8 4 0)) points))
                     '("tip" "whole")))
      (should (equal (mapcar (lambda (target) (alist-get 'id target))
                             (gnosis-model-hit-targets scene '(:mesh "mesh" :face 1 :point (2 8 0)) points))
                     '("patch" "whole")))
      (should-not (gnosis-model-hit-targets scene '(:mesh "elsewhere" :face 0 :point (0 0 0)) points)))))

(ert-deftest gnosis-agent-model-tools-preview-then-save-by-token ()
  (gnosis-test-with-db
    (require 'romacs-agent-gnosis)
    (let* ((resource (gnosis-model-import (gnosis-test-model--scene)))
           (call (lambda (name &rest pairs)
                   (let ((tool (alist-get name romacs-mcp-tools nil nil #'equal))
                         (arguments (apply #'romacs-mcp--object pairs)))
                     (romacs-mcp--arguments (plist-get tool :schema) arguments)
                     (json-parse-string (funcall (plist-get tool :handler) arguments)
                                        :object-type 'plist :false-object :false :null-object nil))))
           (plan (funcall call "gnosis_model_card_preview" "resource" resource "type" "find"
                          "target" "other" "tags" ["anatomy"])))
      (should (equal (plist-get plan :question) "Click the other triangle."))
      (should-not (gnosis-select 'id 'themata nil t))
      (let ((saved (funcall call "gnosis_model_card_save" "plan-token" (plist-get plan :plan-token))))
        (should (equal (plist-get (plist-get saved :item) :answers) ["other"]))
        (should (equal (plist-get (funcall call "gnosis_model_card_save" "plan-token"
                                           (plist-get plan :plan-token))
                                  :thema-id)
                       (plist-get saved :thema-id))))
      (should (eq (plist-get (funcall call "gnosis_model_status") :available) :false)))))

(provide 'gnosis-test-agent-model)
;;; gnosis-test-agent-model.el ends here
