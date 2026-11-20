;;; romacs-working-tests.el --- The working thread -*- lexical-binding: t; -*-
;; SPDX-License-Identifier: GPL-3.0-or-later
;; Maintainer: Thanos Apollo <public@thanosapollo.org>
(require 'ert)
(require 'romacs-working)

(defun romacs-working-tests--element (id)
  (seq-find (lambda (entry) (equal (nth 3 entry) (plist-get (romacs-working--element id) :start)))
            romacs-animate--elements))

(ert-deftest romacs-working-says-its-work-to-the-display ()
  (with-temp-buffer
    (insert (romacs-working-still 12 'idle) " running\n")
    (unwind-protect
        (progn
          (romacs-working-show 'test (current-buffer) (point-min) 12 'running)
          (should (equal (plist-get (romacs-working--element 'test) :kind)
                         '(working running 0 romacs-working-running 0)))
          ;; Progress steps the bead on.
          (romacs-working-advance 'test)
          (romacs-working-advance 'test)
          (should (equal (nth 4 (plist-get (romacs-working--element 'test) :kind)) 2))
          (should (romacs-animate--get 'test))
          ;; At rest nothing is registered with the display.
          (romacs-working-set 'test 'idle)
          (should-not (romacs-animate--get 'test))
          (romacs-working-advance 'test)
          (should (equal (nth 4 (plist-get (romacs-working--element 'test) :kind)) 2))
          ;; Waiting is still too: no clock runs while the user is asked.
          (romacs-working-set 'test 'waiting)
          (should-not (romacs-animate--get 'test))
          (romacs-working-set 'test 'running)
          (should (romacs-animate--get 'test)))
      (romacs-working-stop 'test))
    (should-not (romacs-working--element 'test))))

(ert-deftest romacs-working-paints-its-still-frame-in-place ()
  (with-temp-buffer
    (insert (propertize (concat (romacs-working-still 6 'idle) " rest\n") 'read-only t 'mine t))
    (set-buffer-modified-p nil)
    (let ((before (copy-marker 1)) (after (copy-marker 1 t)) (undo buffer-undo-list))
      (unwind-protect
          (progn
            (romacs-working-show 'paint (current-buffer) (point-min) 6 'failed)
            (should (string-prefix-p "⠤⠤⠀⠤⠤⠤ rest" (buffer-string)))
            (romacs-working-set 'paint 'waiting)
            (should (string-prefix-p "⠿⠿⠿⠿⠿⠿ rest" (buffer-string)))
            ;; In place: no marker moved, its other properties kept, no undo.
            (should (= before 1))
            (should (= after 1))
            (should (get-text-property 3 'mine))
            (should (eq (get-text-property 3 'face) 'romacs-working-waiting))
            (should (eq buffer-undo-list undo))
            (should-not (buffer-modified-p)))
        (romacs-working-stop 'paint)))))

(ert-deftest romacs-working-follows-its-callers-marker ()
  (with-temp-buffer
    (insert "before\n")
    (let ((start (copy-marker (point))))
      (insert (romacs-working-still 4 'idle) "\n")
      (unwind-protect
          (progn
            (romacs-working-show 'moving (current-buffer) start 4 'running)
            (goto-char (point-min))
            (insert "more\n")
            ;; The display reads the marker itself as it freezes a frame.
            (should (eq (plist-get (romacs-working--element 'moving) :start) start))
            (should (= start 13)))
        (romacs-working-stop 'moving))
      (should (marker-position start)))))

(ert-deftest romacs-working-ends-with-its-buffer ()
  (let ((buffer (generate-new-buffer "working")))
    (with-current-buffer buffer (insert (romacs-working-still 4 'idle) "\n"))
    (romacs-working-show 'killed buffer 1 4 'running)
    (kill-buffer buffer)
    (should-not (gethash 'killed romacs-working--threads))
    (should-not (romacs-animate--get 'killed))))

(ert-deftest romacs-working-still-frames-are-as-wide-as-asked ()
  (dolist (work '(idle running waiting failed))
    (dolist (cols '(1 2 3 9))
      (should (= (length (romacs-working-still cols work)) cols))
      (should (string-match-p "[⠤⠿⣀]" (romacs-working-still cols work))))))

(ert-deftest romacs-working-leaves-text-that-is-not-its-own ()
  (with-temp-buffer
    (insert "ab" (romacs-working-still 2 'idle) "\n")
    (unwind-protect
        (progn (romacs-working-show 'mixed (current-buffer) 1 4 'waiting)
               (should (string-prefix-p "ab⠿⠿" (buffer-string))))
      (romacs-working-stop 'mixed))))

(defmacro romacs-working-tests--theme (palette faces &rest body)
  "BODY with the theme's PALETTE and FACES' foregrounds, alists, its
background near black."
  (declare (indent 2))
  `(cl-letf (((symbol-function 'romacs-theme-color) (lambda (slot &optional _) (alist-get slot ,palette)))
             ((symbol-function 'face-foreground) (lambda (face &rest _) (alist-get face ,faces)))
             ((symbol-function 'face-background) (lambda (&rest _) "#121212")))
     ,@body))

(ert-deftest romacs-working-running-never-takes-the-error-colour ()
  ;; A theme whose accent is its error red: running takes its cool colour.
  (romacs-working-tests--theme '((accent . "#d35f5f") (blue . "#d4605e") (cyan . "#5fafd7"))
      '((error . "#d35f5f") (warning . "#d7af5f"))
    (should (equal (romacs-working--running-colour) "#5fafd7")))
  ;; Omarchy matte black's palette: its yellow and orange are reds, its
  ;; amber is filed as green, its accent orange; running is no red.
  (let ((matte '((accent . "#e68e0d") (yellow . "#b91c1c") (orange . "#c63d3d") (green . "#ffc107")
                 (cyan . "#bebebe") (blue . "#e68e0d") (bright-yellow . "#b90a0a"))))
    (romacs-working-tests--theme matte '((error . "#d35f5f") (warning . "#d35f5f") (default . "#bebebe"))
      (should (equal (romacs-working--waiting-colour) "#e68e0d"))
      ;; Its grey, so the amber stays waiting's.
      (should (equal (romacs-working--running-colour) "#bebebe"))))
  ;; Nothing stands off error: the foreground all the same, never the last
  ;; theme's colour.
  (romacs-working-tests--theme '((accent . "#d35f5f")) '((error . "#d35f5f") (link . "#d35f5f") (default . "#d45f5e"))
    (should (equal (romacs-working--running-colour) "#d45f5e"))))

(ert-deftest romacs-working-reads-any-form-of-colour ()
  (should (equal (romacs-working--hex "#abc123") "#abc123"))
  (should (equal (romacs-working--hex "#ffffff") "#ffffff"))
  (should-not (romacs-working--hex nil)))

(provide 'romacs-working-tests)
;;; romacs-working-tests.el ends here
