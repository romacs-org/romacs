;;; romacs-model-tests.el --- Native model events in Lisp -*- lexical-binding: t; -*-

;; SPDX-License-Identifier: GPL-3.0-or-later

;;; Commentary:
;; rlisp/romacs-model.el's side of the native model events: the special
;; event `(romacs-model ID TYPE PLIST)' updates state at once, runs the
;; callbacks in order and never nested, drops picks of a picture that is
;; not current and anything for a destroyed model.  Batch has no renderer,
;; so the primitives are replaced by a recording stand-in; their Rust side
;; is crates/romacs-vm/tests/native_models.rs and the native whole is
;; crates/romacs-vm-gui/harness/model_live.py.

;;; Code:
(require 'ert)
(require 'romacs-model)

(defvar romacs-model-tests--sent nil "Requests the stand-in received.")

(defmacro romacs-model-tests--with-renderer (&rest body)
  "Run BODY with a stand-in renderer that accepts every request."
  (declare (indent 0))
  `(let ((romacs-model-tests--sent nil) (next 0)
         (romacs-model--undelivered nil) (romacs-model--queue nil))
     (cl-letf (((symbol-function 'romacs-model--available-p) (lambda () t))
               ((symbol-function 'romacs-model--create)
                (lambda (&rest _) (setq next (1+ next)) (+ 1000 next)))
               ((symbol-function 'romacs-model--set-view)
                (lambda (&rest args) (push (cons 'view args) romacs-model-tests--sent)))
               ((symbol-function 'romacs-model--set-highlight)
                (lambda (&rest args) (push (cons 'highlight args) romacs-model-tests--sent)))
               ((symbol-function 'romacs-surface-destroy)
                (lambda (id) (push (list 'destroy id) romacs-model-tests--sent))))
       ,@body)))

(defun romacs-model-tests--event (model type &rest plist)
  "Deliver TYPE with PLIST for MODEL as the renderer would."
  (romacs-model--handle-event (list 'romacs-model (romacs-model--id model) type plist)))

(ert-deftest romacs-model-events-update-state-and-run-callbacks-in-order ()
  (romacs-model-tests--with-renderer
    (let* (seen model)
      (setq model (romacs-model-create
                   :meshes '(("a" "/m/a.obj") ("b" "/m/b.obj"))
                   :callback (lambda (_ type plist)
                               (push (list type (plist-get plist :rev)) seen)
                               ;; An event arriving inside a callback waits for it.
                               (when (eq type 'ready)
                                 (romacs-model-tests--event model 'readiness :current t :rev 1 :req 0)
                                 (push 'ready-done seen)))))
      (should (eq (lookup-key special-event-map [romacs-model]) #'romacs-model--handle-event))
      (romacs-model-tests--event model 'ready :triangles '(12 4))
      (should (equal (car (romacs-model-status model)) 'ready))
      (should (equal (reverse seen) '((ready nil) ready-done (readiness 1))))
      (should (romacs-model-current-p model))
      (should (eql (romacs-model-revision model) 1)))))

(ert-deftest romacs-model-picks-name-mesh-ids-and-stale-picks-are-dropped ()
  (romacs-model-tests--with-renderer
    (let* (picks
           (model (romacs-model-create :meshes '(("a" "/m/a.obj") ("b" "/m/b.obj"))
                                       :callback (lambda (_ type plist)
                                                   (when (eq type 'pick) (push plist picks))))))
      (romacs-model-tests--event model 'ready :triangles '(12 4))
      (romacs-model-tests--event model 'readiness :current t :rev 2 :req 0)
      (romacs-model-tests--event model 'pick :hit '(:mesh 1 :face 3) :rev 2 :req 0)
      (should (equal (plist-get (plist-get (car picks) :hit) :mesh) "b"))
      ;; A pick of an older picture, and one after a request, name nothing current.
      (romacs-model-tests--event model 'pick :hit '(:mesh 0 :face 1) :rev 1 :req 0)
      (romacs-model-set-view model 10 0 1)
      (romacs-model-tests--event model 'pick :hit '(:mesh 0 :face 1) :rev 2 :req 0)
      (should (= (length picks) 1))
      (should (equal (car romacs-model-tests--sent) (list 'view (romacs-model--id model) 1 10 0 1))))))

(ert-deftest romacs-model-destroyed-models-run-nothing-and-free-once ()
  (romacs-model-tests--with-renderer
    (let* (seen
           (model (romacs-model-create :meshes '(("a" "/m/a.obj"))
                                       :callback (lambda (&rest event) (push event seen))))
           (id (romacs-model--id model)))
      (should (eq (romacs-model--find id) model))
      (romacs-model-destroy model)
      (romacs-model-destroy model)
      (should (equal romacs-model-tests--sent (list (list 'destroy id))))
      (should-not (romacs-model--find id))
      (romacs-model--handle-event (list 'romacs-model id 'ready '(:triangles (1))))
      (should-not seen)
      (should-error (romacs-model-set-view model 0 0 1)))))

(ert-deftest romacs-model-handles-carry-a-finalizer-naming-only-the-id ()
  (romacs-model-tests--with-renderer
    (let ((model (romacs-model-create :meshes '(("a" "/m/a.obj")))))
      (should (integerp (romacs-model--id model)))
      (should (eq (type-of (romacs-model--handle-finalizer model)) 'finalizer)))))

(ert-deftest romacs-model-buffers-forget-destroyed-and-moved-models ()
  ;; A buffer holds only the models still attached to it: one destroyed,
  ;; or moved to another buffer, leaves it, and killing it destroys the rest.
  (romacs-model-tests--with-renderer
    (let ((a (generate-new-buffer " model-a")) (b (generate-new-buffer " model-b"))
          (gone (romacs-model-create :meshes '(("m" "/m/a.obj"))))
          (moved (romacs-model-create :meshes '(("m" "/m/a.obj"))))
          (kept (romacs-model-create :meshes '(("m" "/m/a.obj")))))
      (unwind-protect
          (progn
            (with-current-buffer a (dolist (model (list gone moved kept)) (romacs-model-insert model)))
            (romacs-model-destroy gone)
            (with-current-buffer b (romacs-model-insert moved))
            (with-current-buffer a
              (should (equal (hash-table-keys romacs-model--attached) (list kept))))
            (kill-buffer a)
            (should (eq (car (romacs-model-status kept)) 'destroyed))
            (should-not (eq (car (romacs-model-status moved)) 'destroyed)))
        (kill-buffer b)
        (when (buffer-live-p a) (kill-buffer a))))))

(provide 'romacs-model-tests)
;;; romacs-model-tests.el ends here
