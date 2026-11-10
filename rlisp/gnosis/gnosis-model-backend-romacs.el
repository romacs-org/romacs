;;; gnosis-model-backend-romacs.el --- Native Romacs model backend -*- lexical-binding: t; -*-

;; Copyright (C) 2026 Free Software Foundation, Inc.
;; SPDX-License-Identifier: GPL-3.0-or-later
;; Maintainer: Thanos Apollo <public@thanosapollo.org>

;;; Commentary:
;; The `gnosis-model-backend' dispatch plist over `romacs-model', the
;; native GPU model view.  Picks are observations, not answers: Gnosis
;; owns target matching and explicit submit.  Every callback is checked
;; against the buffer's current model, so a replaced, frozen or destroyed
;; attachment never updates its successor.  Loading this library does not
;; touch the renderer.

;;; Code:

(require 'cl-lib)
(require 'seq)
(require 'romacs-model)

(defvar gnosis-model-backend--implementation)
(defvar gnosis-model-backend-selection-hook)
(defvar gnosis-model-backend-pick-radius)
(defvar gnosis-model-pick-color)
(defvar gnosis-model-cue-color)
(declare-function gnosis-model-backend-marker-radius "gnosis-model-backend" (radius zoom size))
(defvar gnosis-model-backend-romacs)
(defvar gnosis-model-backend-romacs-map)

(defun gnosis-model-backend-romacs-unavailable-reason ()
  "Return why native model display is unavailable now, or nil if it is not."
  (cond ((not (fboundp 'romacs-model--create))
         "this Romacs build has no native model display")
        ((not (romacs-model-available-p))
         "no GPU renderer is live (batch session or lost renderer)")))

(defvar-local gnosis-model-backend-romacs--model nil "Owned native model handle.")
(defvar-local gnosis-model-backend-romacs--initial-view nil "Initial camera for reset.")
(defvar-local gnosis-model-backend-romacs--selection nil "Last admitted pick observation.")
(defvar-local gnosis-model-backend-romacs--selected-id nil "Last picked mesh ID.")
(defvar-local gnosis-model-backend-romacs--pending-pick nil
  "(SELECTION REQUEST VIEW) awaiting exact presentation of its paint.")
(defvar-local gnosis-model-backend-romacs--frozen nil "Non-nil during retained feedback.")
(defvar-local gnosis-model-backend-romacs--size nil "Square picture side in pixels.")
(defvar-local gnosis-model-backend-romacs--target nil "Gnosis highlight target alist.")
(defvar-local gnosis-model-backend-romacs--cues nil "Cue target alists painted beneath picks.")
(defvar-local gnosis-model-backend-romacs--status nil "Last native error description.")
(defvar-local gnosis-model-backend-romacs--settled nil
  "Latest request presented successfully by this attachment.")

(defun gnosis-model-backend-romacs--spec (target)
  "Translate Gnosis TARGET alist into a native highlight spec."
  (when target
    (let ((kind (alist-get 'kind target)))
      (append (list :mesh (alist-get 'mesh target)
                    :kind (if (stringp kind) (intern kind) kind))
              (pcase kind
                ((or "point" 'point)
                 (list :face (alist-get 'face target)
                       :barycentric (vconcat (alist-get 'barycentric target))
                       :tolerance (alist-get 'tolerance target)))
                ((or "region" 'region)
                 (list :faces (copy-sequence (alist-get 'faces target)))))))))

(defun gnosis-model-backend-romacs--admit (model data)
  "Return non-nil if event DATA names MODEL's current picture."
  (and (romacs-model-current-p model)
       (eql (plist-get data :rev) (romacs-model-revision model))))

(defun gnosis-model-backend-romacs--event (buffer model event data)
  "Handle MODEL EVENT with DATA only while BUFFER owns MODEL."
  (when (buffer-live-p buffer)
    (with-current-buffer buffer
      (when (eq model gnosis-model-backend-romacs--model)
        (pcase event
          ('readiness
           (when (and (plist-get data :current)
                      (gnosis-model-backend-romacs--admit model data))
             (setq gnosis-model-backend-romacs--status nil
                   gnosis-model-backend-romacs--settled (plist-get data :req))
             (when-let* ((pending gnosis-model-backend-romacs--pending-pick))
               (setq gnosis-model-backend-romacs--pending-pick nil)
               ;; The pick is delivered once its paint is what the user sees.
               (when (and (eql (nth 1 pending) (plist-get data :req))
                          (equal (nth 2 pending) (romacs-model-view model)))
                 (setq gnosis-model-backend-romacs--selection
                       (plist-put (car pending) :frame (plist-get data :rev)))
                 (run-hook-with-args 'gnosis-model-backend-selection-hook
                                     (copy-tree gnosis-model-backend-romacs--selection))))))
          ('failed (setq gnosis-model-backend-romacs--status (plist-get data :error)))
          ('error
           (let ((req (plist-get data :req)))
             (unless (and req gnosis-model-backend-romacs--settled
                          (<= req gnosis-model-backend-romacs--settled))
               (setq gnosis-model-backend-romacs--status (plist-get data :error)))))
          ('view (setq gnosis-model-backend-romacs--pending-pick nil))
          ('pick
           (when (and (not gnosis-model-backend-romacs--frozen)
                      (gnosis-model-backend-romacs--admit model data))
             (gnosis-model-backend-romacs--pick model data))))
        (force-mode-line-update)))))

(defun gnosis-model-backend-romacs--pick (model data)
  "Record pick DATA on MODEL; deliver it once its paint is presented.
A background click carries no hit and selects nothing."
  (let* ((hit (plist-get data :hit))
         (id (plist-get hit :mesh))
         (selection (list :owner model :frame (plist-get data :rev)
                          :id id :mesh id :face (plist-get hit :face)
                          :point (append (plist-get hit :point) nil)
                          :barycentric (append (plist-get hit :barycentric) nil))))
    (setq gnosis-model-backend-romacs--selection nil
          gnosis-model-backend-romacs--pending-pick nil
          gnosis-model-backend-romacs--selected-id id)
    (if gnosis-model-backend-romacs--target
        ;; An authored target owns the highlight; deliver the pick at once.
        (progn
          (setq gnosis-model-backend-romacs--selection selection)
          (run-hook-with-args 'gnosis-model-backend-selection-hook (copy-tree selection)))
      (setq gnosis-model-backend-romacs--pending-pick
            (list selection
                  (apply #'romacs-model-set-highlight model
                         (gnosis-model-backend-romacs--pick-highlight model selection))
                  (romacs-model-view model))))))

(defun gnosis-model-backend-romacs-snapshot ()
  "Return the current buffer's native attachment as a Gnosis observation."
  (let* ((model gnosis-model-backend-romacs--model)
         (status (and model (romacs-model-status model)))
         (live (and model (romacs-model-available-p)
                    (not (memq (car status) '(destroyed failed)))))
         (current (and live (eq (car status) 'ready) (romacs-model-current-p model)))
         (revision (and model (romacs-model-revision model)))
         (selection gnosis-model-backend-romacs--selection))
    (list :owner model :attachment model :live (and live t)
          :ready (and current t) :dirty (not current)
          :frame revision
          :view (and model (romacs-model-view model))
          :size gnosis-model-backend-romacs--size
          :status (or gnosis-model-backend-romacs--status
                      (plist-get (cdr status) :error)
                      (pcase (car status)
                        ('ready (if current "Ready" "Rendering"))
                        ('loading "Loading…")
                        (_ (concat "Unavailable: "
                                   (or (gnosis-model-backend-romacs-unavailable-reason)
                                       "model retired")))))
          :selection (and current selection
                          (eql (plist-get selection :frame) revision)
                          (copy-tree selection))
          :selected-id gnosis-model-backend-romacs--selected-id
          :question-target (copy-tree gnosis-model-backend-romacs--target)
          :keymap gnosis-model-backend-romacs-map)))

(defun gnosis-model-backend-romacs-detach (owner attachment &optional keep-picture)
  "Retire OWNER's ATTACHMENT only while it still owns this buffer.
KEEP-PICTURE freezes the final frame until its glyph or buffer retires.
Return non-nil on retirement; a stale claim never touches a successor."
  (when (and owner (eq owner attachment)
             (eq owner gnosis-model-backend-romacs--model))
    (if (not keep-picture)
        (gnosis-model-backend-romacs--destroy owner)
      (romacs-model-freeze owner)
      (setq gnosis-model-backend-romacs--frozen t
            gnosis-model-backend-romacs--selection nil
            gnosis-model-backend-romacs--pending-pick nil)
      (add-hook 'before-change-functions #'gnosis-model-backend-romacs--retire-picture nil t)
      t)))

(defconst gnosis-model-backend-romacs--retire-hooks
  '(kill-buffer-hook change-major-mode-hook after-set-visited-file-name-hook)
  "Buffer hooks that retire the attachment.")

(defun gnosis-model-backend-romacs--destroy (owner)
  "Destroy exact current OWNER and remove its glyph."
  ;; Retire authority before destruction can deliver callbacks.
  (setq gnosis-model-backend-romacs--model nil
        gnosis-model-backend-romacs--selection nil
        gnosis-model-backend-romacs--selected-id nil
        gnosis-model-backend-romacs--target nil
        gnosis-model-backend-romacs--cues nil
        gnosis-model-backend-romacs--pending-pick nil
        gnosis-model-backend-romacs--size nil
        gnosis-model-backend-romacs--frozen nil)
  (when (eq gnosis-model-backend--implementation gnosis-model-backend-romacs)
    (setq-local gnosis-model-backend--implementation nil))
  (remove-hook 'before-change-functions #'gnosis-model-backend-romacs--retire-picture t)
  (dolist (hook gnosis-model-backend-romacs--retire-hooks)
    (remove-hook hook #'gnosis-model-backend-romacs--retire t))
  (unwind-protect
      (save-restriction
        (widen)
        (let ((pos (point-min)))
          (while (< pos (point-max))
            (let ((next (next-single-property-change pos 'romacs-model nil (point-max))))
              (when (eq (get-text-property pos 'romacs-model) owner)
                (with-silent-modifications
                  (remove-text-properties pos next '(display nil keymap nil romacs-model nil))))
              (setq pos next)))))
    (romacs-model-destroy owner))
  t)

(defun gnosis-model-backend-romacs--retire-picture (beg end)
  "Retire a frozen picture before its glyph between BEG and END changes."
  (when (and gnosis-model-backend-romacs--frozen
             (text-property-any beg end 'romacs-model gnosis-model-backend-romacs--model))
    (gnosis-model-backend-romacs--retire)))

(defun gnosis-model-backend-romacs--retire ()
  "Retire the current buffer's exact native attachment."
  (gnosis-model-backend-romacs-detach gnosis-model-backend-romacs--model
                                      gnosis-model-backend-romacs--model))

(defun gnosis-model-backend-romacs--owned ()
  "Return the owned model, or signal."
  (or gnosis-model-backend-romacs--model (user-error "No native model attachment")))

(defun gnosis-model-backend-romacs-highlight (target selected-id)
  "Display TARGET and SELECTED-ID on the current attachment."
  (let ((model (gnosis-model-backend-romacs--owned)))
    (setq gnosis-model-backend-romacs--pending-pick nil)
    (prog1 (romacs-model-set-highlight
            model :target (gnosis-model-backend-romacs--spec target)
            ;; Target IDs and mesh IDs are different namespaces: a target
            ;; owns the highlight; SELECTED-ID is retained for Gnosis only.
            :selected (unless target selected-id))
      (setq gnosis-model-backend-romacs--target (copy-tree target)
            gnosis-model-backend-romacs--selected-id selected-id))))

(defun gnosis-model-backend-romacs--layers-p ()
  "Return non-nil if the renderer paints colored highlight layers."
  (and (memq 'highlight-layers (romacs-model-capabilities)) t))

(defun gnosis-model-backend-romacs--cue-layers ()
  "Return highlight layers painting the current cues."
  (mapcar (lambda (cue)
            (list :target (gnosis-model-backend-romacs--spec cue)
                  :color gnosis-model-cue-color))
          gnosis-model-backend-romacs--cues))

;; The renderer paints at most `romacs-model-max-highlight-layers'; cues
;; give way to what is drawn above them, so a pick or an answer always shows.
(defun gnosis-model-backend-romacs--fit-layers (beneath above)
  "Return layers BENEATH, then ABOVE, within the renderer's layer limit.
ABOVE (picks, answers) are kept first, its last ones when it alone is
too many; BENEATH (cues) fill the room left."
  (let* ((limit romacs-model-max-highlight-layers)
         (above (last above limit)))
    (append (seq-take beneath (- limit (length above))) above)))

(defun gnosis-model-backend-romacs-cue (targets)
  "Paint cue TARGETS beneath picks; nil TARGETS removes them.
Return non-nil if painted; without colored layers decline."
  (let ((model (gnosis-model-backend-romacs--owned)))
    (when (gnosis-model-backend-romacs--layers-p)
      (setq gnosis-model-backend-romacs--cues (copy-tree targets)
            gnosis-model-backend-romacs--pending-pick nil)
      (apply #'romacs-model-set-highlight model
             (gnosis-model-backend-romacs--pick-highlight
              model (or gnosis-model-backend-romacs--selection
                        (list :mesh gnosis-model-backend-romacs--selected-id))))
      t)))

(defun gnosis-model-backend-romacs-set-view (view)
  "Request VIEW, a (YAW PITCH ZOOM) list, on the current attachment."
  (let ((model (gnosis-model-backend-romacs--owned)))
    (setq gnosis-model-backend-romacs--pending-pick nil)
    (apply #'romacs-model-set-view model view)))

(defun gnosis-model-backend-romacs--pick-spec (pick radius)
  "Return a native point spec of RADIUS at surface PICK, or nil.
Renderer weights may carry rounding error; clamp and renormalize them."
  (let ((weights (mapcar (lambda (w) (if (numberp w) (max 0.0 (float w)) -1.0))
                         (append (plist-get pick :barycentric) nil))))
    (when (and (plist-get pick :mesh) (natnump (plist-get pick :face))
               (numberp radius) (< 0 radius 1.0e+INF)
               (= (length weights) 3)
               (seq-every-p (lambda (w) (<= 0 w 1.0e+INF)) weights)
               (< 0 (apply #'+ weights) 1.0e+INF))
      (let ((sum (apply #'+ weights)))
        (list :mesh (plist-get pick :mesh) :kind 'point :face (plist-get pick :face)
              :barycentric (vconcat (mapcar (lambda (w) (/ w sum)) weights))
              :tolerance radius)))))

(defun gnosis-model-backend-romacs--pick-highlight (model selection)
  "Return `romacs-model-set-highlight' arguments marking SELECTION on MODEL.
With `gnosis-model-backend-pick-radius' and layers, mark the picked spot;
otherwise select the picked mesh.  Cues stay beneath."
  (let* ((layers (gnosis-model-backend-romacs--layers-p))
         (radius (bound-and-true-p gnosis-model-backend-pick-radius))
         (mesh (plist-get selection :mesh))
         (spot (and radius layers
                    (gnosis-model-backend-romacs--pick-spec
                     selection
                     (gnosis-model-backend-marker-radius
                      radius (nth 2 (romacs-model-view model))
                      gnosis-model-backend-romacs--size))))
         (mark (cond (spot (list :target spot :color gnosis-model-pick-color))
                     (mesh (list :mesh mesh :color gnosis-model-pick-color)))))
    (cond ((and layers gnosis-model-backend-romacs--cues)
           (list :layers (gnosis-model-backend-romacs--fit-layers
                          (gnosis-model-backend-romacs--cue-layers) (delq nil (list mark)))))
          (spot (list :layers (list mark)))
          (t (list :target nil :selected mesh)))))

(defcustom gnosis-model-backend-romacs-reveal-timeout 2.0
  "Seconds to wait for a revealed answer to be presented."
  :type 'number
  :group 'gnosis)

(defconst gnosis-model-backend-romacs--answer-point-scale 2.0
  "Revealed answer point radius as a multiple of the marker radius.")

(defun gnosis-model-backend-romacs--reveal-target (target radius)
  "Return TARGET's native spec; a point shows scaled from marker RADIUS."
  (let ((spec (gnosis-model-backend-romacs--spec target)))
    (if (and (eq (plist-get spec :kind) 'point) (numberp radius) (< 0 radius 1.0e+INF))
        (plist-put spec :tolerance (* gnosis-model-backend-romacs--answer-point-scale radius))
      spec)))

(defun gnosis-model-backend-romacs--reveal-layers (reveal)
  "Return layers for REVEAL: cues, then wrong picks, then correct targets."
  (let* ((radius (plist-get reveal :marker-radius))
         (correct (plist-get reveal :correct-color))
         (wrong-color (plist-get reveal :wrong-color))
         (wrong (plist-get reveal :wrong-mesh))
         (pick (gnosis-model-backend-romacs--pick-spec (plist-get reveal :wrong-pick) radius)))
    (gnosis-model-backend-romacs--fit-layers
     (mapcar (lambda (cue) (list :target (gnosis-model-backend-romacs--spec cue)
                                 :color (plist-get reveal :cue-color)))
             (plist-get reveal :cues))
     (delq nil
           (append
            (list (and wrong (list :mesh wrong :color wrong-color))
                  (and pick (list :target pick :color wrong-color)))
            (mapcar (lambda (target)
                      (and target
                           (list :target (gnosis-model-backend-romacs--reveal-target target radius)
                                 :color correct)))
                    (append (plist-get reveal :alternates)
                            (list (plist-get reveal :target)))))))))

(defun gnosis-model-backend-romacs--await (model)
  "Wait until MODEL presents its latest request or the reveal timeout.
Return non-nil if presented.  Pending input is not consumed."
  (let ((deadline (+ (float-time) gnosis-model-backend-romacs-reveal-timeout)))
    (while (and (eq model gnosis-model-backend-romacs--model)
                (not (romacs-model-current-p model))
                (memq (car (romacs-model-status model)) '(ready loading))
                (< (float-time) deadline))
      (unless (sit-for 0.02) (sleep-for 0.02)))
    (and (eq model gnosis-model-backend-romacs--model)
         (romacs-model-current-p model))))

(defun gnosis-model-backend-romacs-reveal (owner attachment reveal)
  "Reveal REVEAL on exact OWNER's ATTACHMENT and wait for its presentation.
Refuse stale, frozen or successor attachments.  Return non-nil if issued."
  (when (and owner (eq owner attachment)
             (eq owner gnosis-model-backend-romacs--model)
             (not gnosis-model-backend-romacs--frozen)
             (eq (car (romacs-model-status owner)) 'ready))
    (let ((size (plist-get reveal :size)) issued)
      (setq gnosis-model-backend-romacs--pending-pick nil)
      ;; Only shrink: the question already chose the largest fitting picture.
      (when (and size gnosis-model-backend-romacs--size
                 (< size gnosis-model-backend-romacs--size))
        (romacs-model-resize owner size size)
        (setq gnosis-model-backend-romacs--size size issued t))
      (when (plist-get reveal :target)
        (if (gnosis-model-backend-romacs--layers-p)
            (romacs-model-set-highlight
             owner :layers (gnosis-model-backend-romacs--reveal-layers reveal))
          (romacs-model-set-highlight
           owner :target (gnosis-model-backend-romacs--reveal-target
                          (plist-get reveal :target) (plist-get reveal :marker-radius))
           :selected nil))
        (setq issued t))
      (when issued (gnosis-model-backend-romacs--await owner))
      issued)))

(defun gnosis-model-backend-romacs--nudge (yaw pitch zoom)
  "Turn the owned model by YAW and PITCH degrees, scaling zoom by ZOOM."
  (let ((view (romacs-model-view (gnosis-model-backend-romacs--owned))))
    (gnosis-model-backend-romacs-set-view
     (list (+ (nth 0 view) yaw) (+ (nth 1 view) pitch)
           (max 0.25 (min 4.0 (* (nth 2 view) zoom)))))))

(defun gnosis-model-backend-romacs-forward ()
  "Turn the model right."
  (interactive nil special-mode)
  (gnosis-model-backend-romacs--nudge 15 0 1))

(defun gnosis-model-backend-romacs-backward ()
  "Turn the model left."
  (interactive nil special-mode)
  (gnosis-model-backend-romacs--nudge -15 0 1))

(defun gnosis-model-backend-romacs-next ()
  "Tilt the model down."
  (interactive nil special-mode)
  (gnosis-model-backend-romacs--nudge 0 15 1))

(defun gnosis-model-backend-romacs-previous ()
  "Tilt the model up."
  (interactive nil special-mode)
  (gnosis-model-backend-romacs--nudge 0 -15 1))

(defun gnosis-model-backend-romacs-zoom-in ()
  "Zoom the model in."
  (interactive nil special-mode)
  (gnosis-model-backend-romacs--nudge 0 0 1.1))

(defun gnosis-model-backend-romacs-zoom-out ()
  "Zoom the model out."
  (interactive nil special-mode)
  (gnosis-model-backend-romacs--nudge 0 0 (/ 1 1.1)))

(defun gnosis-model-backend-romacs-reset ()
  "Restore the model's initial camera."
  (interactive nil special-mode)
  (gnosis-model-backend-romacs-set-view gnosis-model-backend-romacs--initial-view))

(dolist (command '(gnosis-model-backend-romacs-forward gnosis-model-backend-romacs-backward
                   gnosis-model-backend-romacs-next gnosis-model-backend-romacs-previous
                   gnosis-model-backend-romacs-zoom-in gnosis-model-backend-romacs-zoom-out
                   gnosis-model-backend-romacs-reset))
  (put command 'completion-predicate
       (lambda (_symbol buffer) (buffer-local-value 'gnosis-model-backend-romacs--model buffer))))

(defvar-keymap gnosis-model-backend-romacs-map
  :doc "Camera keys of a native Gnosis model."
  :parent special-mode-map
  "f" #'gnosis-model-backend-romacs-forward "b" #'gnosis-model-backend-romacs-backward
  "n" #'gnosis-model-backend-romacs-next "p" #'gnosis-model-backend-romacs-previous
  "<right>" #'gnosis-model-backend-romacs-forward "<left>" #'gnosis-model-backend-romacs-backward
  "<down>" #'gnosis-model-backend-romacs-next "<up>" #'gnosis-model-backend-romacs-previous
  "+" #'gnosis-model-backend-romacs-zoom-in "=" #'gnosis-model-backend-romacs-zoom-in
  "-" #'gnosis-model-backend-romacs-zoom-out "r" #'gnosis-model-backend-romacs-reset)

(defconst gnosis-model-backend-romacs
  '(:open gnosis-model-backend-romacs-open
    :snapshot gnosis-model-backend-romacs-snapshot
    :detach gnosis-model-backend-romacs-detach
    :highlight gnosis-model-backend-romacs-highlight
    :set-view gnosis-model-backend-romacs-set-view
    :reveal gnosis-model-backend-romacs-reveal
    :cue gnosis-model-backend-romacs-cue)
  "Dispatch operations of the native Romacs model backend.")

(defun gnosis-model-backend-romacs--meshes (path)
  "Return the (ID FILE nil) mesh list of validated scene manifest PATH."
  (let ((directory (file-name-directory (expand-file-name path))))
    (mapcar (lambda (object)
              (list (alist-get 'id object)
                    (expand-file-name (alist-get 'path object) directory)))
            (alist-get 'objects
                       (json-parse-string
                        (with-temp-buffer (insert-file-contents path) (buffer-string))
                        :object-type 'alist :array-type 'list)))))

(defun gnosis-model-backend-romacs-open (path view &optional size inline target)
  "Show scene manifest PATH at VIEW with square pixel SIZE; return its buffer.
INLINE inserts at point without changing the host mode or keymap;
otherwise create a standalone `special-mode' buffer.  TARGET is a Gnosis
display highlight, not a grading rule.  Errors and quits free the model."
  (when-let* ((reason (gnosis-model-backend-romacs-unavailable-reason)))
    (user-error "Native model renderer unavailable: %s" reason))
  (let* ((meshes (gnosis-model-backend-romacs--meshes path))
         (size (or size 512))
         (buffer (if inline (current-buffer) (generate-new-buffer "*Gnosis model*")))
         model attached)
    (unwind-protect
        (with-current-buffer buffer
          (unless inline (special-mode))
          (setq model (romacs-model-create
                       :meshes meshes :view view :width size :height size
                       :callback (lambda (handle event data)
                                   (gnosis-model-backend-romacs--event buffer handle event data))))
          (gnosis-model-backend-romacs--retire)
          (setq gnosis-model-backend-romacs--model model
                gnosis-model-backend-romacs--initial-view (romacs-model-view model)
                gnosis-model-backend-romacs--status nil
                gnosis-model-backend-romacs--settled nil
                gnosis-model-backend-romacs--size size)
          (setq-local gnosis-model-backend--implementation gnosis-model-backend-romacs)
          (dolist (hook gnosis-model-backend-romacs--retire-hooks)
            (add-hook hook #'gnosis-model-backend-romacs--retire nil t))
          (let ((inhibit-read-only (not inline)))
            ;; Camera keys belong to the host map, so authoring keys win.
            (romacs-model-insert model :keymap nil))
          (when target (gnosis-model-backend-romacs-highlight target nil))
          (unless inline
            (use-local-map gnosis-model-backend-romacs-map)
            (goto-char (point-min))
            (pop-to-buffer buffer))
          (setq attached t)
          buffer)
      (unless attached
        (when model
          (with-current-buffer buffer
            (if (eq model gnosis-model-backend-romacs--model)
                (gnosis-model-backend-romacs--destroy model)
              (romacs-model-destroy model))))
        (unless inline (kill-buffer buffer))))))

(provide 'gnosis-model-backend-romacs)
;;; gnosis-model-backend-romacs.el ends here
