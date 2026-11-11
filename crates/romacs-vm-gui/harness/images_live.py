#!/usr/bin/env python3
"""GNU's image primitives in pinned GNU and in Romacs GUI --loadup, side by side.

image.c's primitives need a window-system frame, so each workflow starts a
pair of GUI sessions (parity_live.py): pinned GNU Emacs (pgtk, Cairo, librsvg,
no ImageMagick) and `romacs-vm-gui --loadup', each in its own private headless
labwc with the same font, and evaluates the same forms in both.

  primitives  init-image-library and image-types; imagep over valid and
              invalid specs, XBM's data forms included; create-image;
              image-size in pixels and characters for each fixture under
              :width :height :max-width :max-height (pixels and em/ch/cw),
              :scale (a number and `default'), :rotation and :margin/:relief;
              image-mask-p; image-metadata of animated GIF and WebP; a
              corrupt and a missing file (the size and the *Messages* line);
              lookup-image-map; image-transforms-p.
  cache       image-flush and clear-image-cache: a file rewritten with
              another size reads the old size until flushed, then the new.
  display     images in a buffer: notmuch's flagged star (:ascent center,
              :mask heuristic) in a line of text, an image alone, :ascent
              100 with :margin, a slice: line-pixel-height,
              window-text-pixel-size, posn-at-point and vertical-motion
              over them.
  org         Org's inline images (org-display-inline-images) with
              org-image-actual-width nil, 30 and (60), an #+ATTR_ORG width,
              and org-redisplay-inline-images after a file changes: each
              overlay's span, :width, :scale and image-size, and the line
              heights.
  gnosis      gnosis's managed image in a private gnosis-dir (a fixture,
              never a user's database): its decode validation
              (gnosis-image--decode, image-size against the manifest), its
              review image (gnosis-image-format-string) and its occlusion
              scene masked and revealed (gnosis-image-mask, svg-image with
              :scale 1.0) laid out in a buffer.
  hidpi       run with PARITY_OUTPUT_SCALE=2: frame metrics, image-size of
              an SVG, a PNG and a :scale 2 JPEG, and an image line's
              height, size and posn-at-point, all in logical pixels.
  text-scale  a pixel-sized and an em-sized image before, under and after
              text-scale-set 2: line heights, the glyphs' sizes in
              posn-at-point, and image-size (the frame's default face).

Fixtures are in harness/fixtures/images.  `canvas' is GNU's Android-only
type (DIVERGENCES VM2-IMAGE-CANVAS), so the forms leave it out.  COMPARISON POLICY: every form's
value exactly, an error as (ERROR . DATA), and the *Messages* lines gained.

usage: images_live.py --romacs BIN [--gnu EMACS] --output DIR [--workflow NAME]...
"""
import json
from pathlib import Path
import shutil
import sys

import parity_live
from parity_live import gnu_engine, lisp_string, private_tree, romacs_engine

FIXTURES = Path(__file__).resolve().parent / 'fixtures' / 'images'

INIT = ''';; -*- lexical-binding: t -*-
{chrome}(setq inhibit-startup-screen t initial-scratch-message nil)
(setq parity-probe-variables nil)
(load {probe} nil t)
(defmacro im-try (&rest body) `(condition-case err (progn ,@body) (error (cons 'ERROR err))))
(defun im-file (name) (expand-file-name name {fixtures}))
(defvar im-root {root})
(defun im-spec (name &rest props) (append (list 'image :type (image-type (im-file name)) :file (im-file name)) props))
(with-temp-file (expand-file-name "alive" parity-probe-dir) (insert (number-to-string (float-time))))
'''

SIZES = [f'(im-try (image-size (im-spec "{f}") t))' for f in
         ('plain.png', 'alpha.png', 'photo.jpg', 'anim.gif', 'anim.webp', 'sized.svg', 'viewbox.svg', 'current.svg',
          'star.xbm', 'dot.xpm')]

PROPS = ['(:width 80)', '(:height 12)', '(:width 30 :height 30)', '(:max-width 20)', '(:max-height 10)',
         '(:width 100 :max-width 20)', '(:scale 2)', '(:scale 0.5)', '(:scale default)', '(:rotation 90)',
         '(:rotation 180)', '(:rotation 270)', '(:rotation 45)', '(:margin 3)', '(:margin (2 . 5))',
         '(:relief 2)', '(:relief -3 :margin 1)', "(:width '(2 . em))", "(:height '(1.5 . ch))", "(:max-width '(4 . cw))",
         '(:width 80 :rotation 90)']

WORKFLOWS = {
    'primitives': [
        '(list (remq (quote canvas) image-types) (mapcar (lambda (ty) (init-image-library ty)) (quote (png jpeg gif webp tiff svg xpm xbm pbm imagemagick bogus))))',
        '(mapcar (lambda (ty) (image-type-available-p ty)) (quote (png svg xbm)))',
        '(mapcar #\'imagep (list (im-spec "plain.png") (list (quote image) :type (quote png)) '
        '(list (quote image) :type (quote png) :file "a" :data "b") (list (quote image) :type (quote png) :file 3) '
        '(list (quote image) :type (quote bogus) :file "a") (list (quote image) :type (quote png) :file "a" :margin -1) '
        '(list (quote image) :type (quote png) :file "a" :ascent 101) (list (quote image) :type (quote png) :file "a" :ascent (quote center)) '
        '(list (quote image) :type (quote png) :file "a" :file "b") (list (quote image) :type (quote png) :file "a" :unknown 1) '
        '(list (quote notimage) :type (quote png) :file "a") (list (quote image) :type (quote xbm) :data "abc") '
        '(list (quote image) :type (quote xbm) :data "abc" :data-width 8 :data-height 3) '
        '(list (quote image) :type (quote xbm) :data (make-bool-vector 16 t) :data-width 8 :data-height 2) '
        '(list (quote image) :type (quote xbm) :data (vector "a" "b") :data-width 8 :data-height 2) '
        '(list (quote image) :type (quote xbm) :data (vector "a") :data-width 8 :data-height 2) '
        '(list (quote image) :type (quote xbm) :data "ab" :data-width 8 :data-height 2 :stride 16)))',
        '(let ((s (create-image (im-file "plain.png")))) (list (car s) (plist-get (cdr s) :type) (plist-get (cdr s) :scale)))',
        *SIZES,
        *[f'(im-try (image-size (im-spec "plain.png" {p[1:-1]}) t))' for p in PROPS],
        *[f'(im-try (image-size (im-spec "sized.svg" {p[1:-1]}) t))' for p in PROPS],
        '(im-try (image-size (im-spec "plain.png")))',
        '(im-try (image-size (im-spec "photo.jpg" :margin 2)))',
        '(mapcar (lambda (s) (im-try (image-mask-p s))) (list (im-spec "plain.png") (im-spec "alpha.png") '
        '(im-spec "plain.png" :mask (quote heuristic)) (im-spec "alpha.png" :mask nil) (im-spec "plain.png" :heuristic-mask t) '
        '(im-spec "dot.xpm") (im-spec "star.xbm")))',
        '(mapcar (lambda (f) (im-try (image-metadata (im-spec f)))) (list "anim.gif" "anim.webp" "plain.png"))',
        '(im-try (image-metadata (im-spec "anim.gif" :index 1)))',
        '(list (im-try (image-size (im-spec "corrupt.png") t)) (im-try (image-size (list (quote image) :type (quote png) :file "/nonexistent/x.png" :width 7) t)))',
        '(im-try (image-size (list (quote image) :type (quote xbm) :data (make-bool-vector 16 t) :data-width 8 :data-height 2) t))',
        '(im-try (image-size (list (quote image) :type (quote xbm) :data (vector "ab" "cd" "ef") :data-width 9 :data-height 3) t))',
        '(im-try (image-size (list (quote image) :type (quote xbm) :data "abcdef" :data-width 8 :data-height 3) t))',
        '(im-try (image-size (list (quote image) :type (quote svg) :data "<svg xmlns=\\"http://www.w3.org/2000/svg\\" width=\\"11\\" height=\\"7\\"/>") t))',
        '(let ((m (list (list (cons (quote rect) (cons (cons 0 0) (cons 10 10))) (quote r) nil) '
        '(list (cons (quote circle) (cons (cons 30 30) 5)) (quote c) nil) '
        '(list (cons (quote poly) [40 40 60 40 50 60]) (quote p) nil)))) '
        '(mapcar (lambda (xy) (car (cdr (lookup-image-map m (car xy) (cdr xy))))) (list (cons 5 5) (cons 11 5) (cons 30 34) (cons 30 36) (cons 50 45) (cons 41 59))))',
        '(list (im-try (lookup-image-map nil 1 1)) (im-try (lookup-image-map (list 1) 1.5 2)))',
        '(image-transforms-p)',
        '(im-try (image-size (list (quote image) :type (quote png))))',
    ],
    'cache': [
        '(progn (copy-file (im-file "plain.png") (expand-file-name "flush.png" temporary-file-directory) t) '
        '(im-try (image-size (list (quote image) :type (quote png) :file (expand-file-name "flush.png" temporary-file-directory)) t)))',
        '(progn (copy-file (im-file "photo.jpg") (expand-file-name "flush.png" temporary-file-directory) t) '
        '(im-try (image-size (list (quote image) :type (quote png) :file (expand-file-name "flush.png" temporary-file-directory)) t)))',
        '(let ((s (list (quote image) :type (quote png) :file (expand-file-name "flush.png" temporary-file-directory)))) '
        '(image-flush s) (im-try (image-size s t)))',
        '(progn (copy-file (im-file "alpha.png") (expand-file-name "flush.png" temporary-file-directory) t) '
        '(clear-image-cache (expand-file-name "flush.png" temporary-file-directory)) '
        '(im-try (image-size (list (quote image) :type (quote png) :file (expand-file-name "flush.png" temporary-file-directory)) t)))',
        '(progn (copy-file (im-file "plain.png") (expand-file-name "flush.png" temporary-file-directory) t) '
        '(clear-image-cache) (im-try (image-size (list (quote image) :type (quote png) :file (expand-file-name "flush.png" temporary-file-directory)) t)))',
        '(progn (clear-image-cache t) (clear-image-cache nil (list (quote image) :type (quote gif) :file (im-file "anim.gif"))) t)',
        # A collection between the load and a frame's clear: the entry is still the frame's.
        '(progn (copy-file (im-file "plain.png") (expand-file-name "flush.png" temporary-file-directory) t) '
        '(im-try (image-size (list (quote image) :type (quote png) :file (expand-file-name "flush.png" temporary-file-directory)) t)))',
        '(progn (garbage-collect) (copy-file (im-file "photo.jpg") (expand-file-name "flush.png" temporary-file-directory) t) '
        '(clear-image-cache (selected-frame)) '
        '(im-try (image-size (list (quote image) :type (quote png) :file (expand-file-name "flush.png" temporary-file-directory)) t)))',
        # em/ch/cw follow the default face's font, also right after it changes.
        '(progn (set-face-attribute (quote default) nil :height 200) '
        '(list (im-try (image-size (im-spec "plain.png" :width (quote (2 . em))) t)) '
        '(im-try (image-size (im-spec "plain.png" :height (quote (1 . ch))) t)) '
        '(im-try (image-size (im-spec "plain.png" :width (quote (3 . cw))) t))))',
    ],
    'display': [
        # notmuch's flagged star (notmuch-tag-format-image-data) and its logo.
        '(progn (switch-to-buffer (get-buffer-create "im-display")) (delete-other-windows) (erase-buffer) '
        '(insert "ab " (propertize "flagged" (quote display) (list (quote image) :type (quote svg) :data '
        '(with-temp-buffer (insert-file-contents (im-file "star.svg")) (buffer-string)) :ascent (quote center) :mask (quote heuristic))) " cd\\n") '
        '(insert-image (im-spec "plain.png")) (insert " plain\\n") '
        '(insert-image (im-spec "sized.svg" :ascent 100 :margin 2)) (insert " a100\\n") '
        '(insert-image (im-spec "photo.jpg") nil nil (list 0 0 0.5 0.5)) (insert " slice\\nlast\\n") '
        '(goto-char (point-min)) (redisplay t) (list (buffer-size) (count-lines (point-min) (point-max))))',
        *[f'(with-selected-window (get-buffer-window "im-display") (redisplay t) {form})' for form in (
            '(save-excursion (goto-char (point-min)) (let (h) (dotimes (_ 5) (push (line-pixel-height) h) (forward-line 1)) (nreverse h)))',
            '(list (window-text-pixel-size nil (point-min) (point-max)) (window-text-pixel-size nil (point-min) (save-excursion (goto-char (point-min)) (line-end-position))))',
            '(save-excursion (goto-char 4) (let ((p (posn-at-point))) (list (posn-point p) (posn-x-y p) (posn-object-x-y p) (posn-object-width-height p) (car-safe (posn-image p)))))',
            '(save-excursion (goto-char (point-min)) (forward-line 1) (let ((p (posn-at-point))) (list (posn-point p) (posn-x-y p) (posn-object-width-height p) (car-safe (posn-image p)))))',
            '(save-excursion (goto-char (point-min)) (forward-line 2) (end-of-line) (posn-x-y (posn-at-point)))',
            '(save-excursion (goto-char (point-min)) (list (vertical-motion 2) (point) (vertical-motion 1) (point)))',
        )],
    ],
    'org': [
        '(progn (setq im-org-dir (make-temp-file "im-org" t)) (dolist (f (list "plain.png" "photo.jpg" "sized.svg")) (copy-file (im-file f) (expand-file-name f im-org-dir) t)) (find-file (expand-file-name "notes.org" im-org-dir)) (delete-other-windows) (erase-buffer) (insert "Images, no heading: the theme sizes headings\\n[[file:plain.png]]\\n#+ATTR_ORG: :width 50\\n[[file:photo.jpg]]\\n[[file:sized.svg]]\\nend\\n") (org-mode) (goto-char (point-min)) t)',
        '(with-selected-window (get-buffer-window "notes.org") (with-current-buffer "notes.org" (progn (setq org-image-actual-width nil) (org-display-inline-images) t)))',
        '(with-selected-window (get-buffer-window "notes.org") (with-current-buffer "notes.org" (progn (redisplay t) (mapcar (lambda (o) (let ((d (overlay-get o (quote display)))) (list (overlay-start o) (overlay-end o) (plist-get (cdr d) :width) (plist-get (cdr d) :scale) (image-size d t)))) (sort (seq-filter (lambda (o) (eq (car-safe (overlay-get o (quote display))) (quote image))) (overlays-in (point-min) (point-max))) (lambda (a b) (< (overlay-start a) (overlay-start b))))))))',
        '(with-selected-window (get-buffer-window "notes.org") (with-current-buffer "notes.org" (save-excursion (redisplay t) (goto-char (point-min)) (let (h) (dotimes (_ 6) (push (line-pixel-height) h) (forward-line 1)) (nreverse h)))))',
        '(with-selected-window (get-buffer-window "notes.org") (with-current-buffer "notes.org" (progn (org-remove-inline-images) (setq org-image-actual-width 30) (org-display-inline-images) t)))',
        '(with-selected-window (get-buffer-window "notes.org") (with-current-buffer "notes.org" (progn (redisplay t) (mapcar (lambda (o) (let ((d (overlay-get o (quote display)))) (list (overlay-start o) (overlay-end o) (plist-get (cdr d) :width) (plist-get (cdr d) :scale) (image-size d t)))) (sort (seq-filter (lambda (o) (eq (car-safe (overlay-get o (quote display))) (quote image))) (overlays-in (point-min) (point-max))) (lambda (a b) (< (overlay-start a) (overlay-start b))))))))',
        '(with-selected-window (get-buffer-window "notes.org") (with-current-buffer "notes.org" (progn (org-remove-inline-images) (setq org-image-actual-width (list 60)) (org-display-inline-images) t)))',
        '(with-selected-window (get-buffer-window "notes.org") (with-current-buffer "notes.org" (progn (redisplay t) (mapcar (lambda (o) (let ((d (overlay-get o (quote display)))) (list (overlay-start o) (overlay-end o) (plist-get (cdr d) :width) (plist-get (cdr d) :scale) (image-size d t)))) (sort (seq-filter (lambda (o) (eq (car-safe (overlay-get o (quote display))) (quote image))) (overlays-in (point-min) (point-max))) (lambda (a b) (< (overlay-start a) (overlay-start b))))))))',
        '(with-selected-window (get-buffer-window "notes.org") (with-current-buffer "notes.org" (save-excursion (redisplay t) (goto-char (point-min)) (let (h) (dotimes (_ 6) (push (line-pixel-height) h) (forward-line 1)) (nreverse h)))))',
        '(with-selected-window (get-buffer-window "notes.org") (with-current-buffer "notes.org" (progn (copy-file (im-file "alpha.png") (expand-file-name "plain.png" im-org-dir) t) (org-redisplay-inline-images) t)))',
        '(with-selected-window (get-buffer-window "notes.org") (with-current-buffer "notes.org" (progn (redisplay t) (mapcar (lambda (o) (let ((d (overlay-get o (quote display)))) (list (overlay-start o) (overlay-end o) (plist-get (cdr d) :width) (plist-get (cdr d) :scale) (image-size d t)))) (sort (seq-filter (lambda (o) (eq (car-safe (overlay-get o (quote display))) (quote image))) (overlays-in (point-min) (point-max))) (lambda (a b) (< (overlay-start a) (overlay-start b))))))))',
    ],
    'gnosis': [
        '(progn (dolist (d (list "gnosis" "emacs-lisp")) (add-to-list (quote load-path) (expand-file-name (concat "lisp/romacs/" d) im-root))) (require (quote gnosis)) (require (quote gnosis-image)) (setq gnosis-dir (file-name-as-directory (make-temp-file "im-gnosis" t))) (setq gn-ref (gnosis-image-import (im-file "photo.jpg") (list (list (cons (quote id) "a") (cons (quote label) "A") (cons (quote rect) (list 0.25 0.25 0.5 0.5)))))) (stringp gn-ref))',
        '(let ((scene (gnosis-image-resolve gn-ref))) (list (alist-get (quote width) scene) (alist-get (quote height) scene) (image-size (gnosis-image--decode scene) t)))',
        '(progn (switch-to-buffer (get-buffer-create "gn")) (delete-other-windows) (erase-buffer) (insert "review: " (gnosis-image-format-string (format "see [[gnosis-image:%s]] text" gn-ref)) "\\n") (insert "mask: " (gnosis-image-mask (gnosis-image-resolve gn-ref) "a" nil) "\\nshown: " (gnosis-image-mask (gnosis-image-resolve gn-ref) "a" t) "\\nend\\n") (goto-char (point-min)) (redisplay t) t)',
        '(with-selected-window (get-buffer-window "gn") (with-current-buffer "gn" (redisplay t) (list (save-excursion (goto-char (point-min)) (let (h) (dotimes (_ 4) (push (line-pixel-height) h) (forward-line 1)) (nreverse h))) (let (out (pos (point-min))) (while (setq pos (next-single-property-change pos (quote display))) (let ((d (get-text-property pos (quote display)))) (when (eq (car-safe d) (quote image)) (push (list (plist-get (cdr d) :type) (image-size d t) (posn-object-width-height (posn-at-point pos))) out)))) (nreverse out)) (window-text-pixel-size nil (point-min) (point-max)))))',
    ],
    # Under PARITY_OUTPUT_SCALE=2 (a HiDPI output): sizes stay logical.
    'hidpi': [
        '(list (frame-char-width) (frame-char-height) (image-size (im-spec "sized.svg") t) (image-size (im-spec "plain.png") t) (image-size (im-spec "photo.jpg" :scale 2) t))',
        '(progn (switch-to-buffer (get-buffer-create "im-hidpi")) (delete-other-windows) (erase-buffer) (insert "svg ") (insert-image (im-spec "sized.svg")) (insert " png ") (insert-image (im-spec "plain.png")) (insert "\\nend\\n") (goto-char (point-min)) (redisplay t) t)',
        '(with-selected-window (get-buffer-window "im-hidpi") (with-current-buffer "im-hidpi" (redisplay t) (list (line-pixel-height) (window-text-pixel-size nil (point-min) (point-max)) (save-excursion (goto-char 5) (let ((p (posn-at-point))) (list (posn-x-y p) (posn-object-width-height p)))))))',
    ],
    'text-scale': [
        '(progn (switch-to-buffer (get-buffer-create "im-scale")) (delete-other-windows) (erase-buffer) (insert "px ") (insert-image (im-spec "plain.png" :width 40)) (insert "\\nem ") (insert-image (im-spec "plain.png" :width (quote (2 . em)))) (insert "\\nend\\n") (goto-char (point-min)) (redisplay t) t)',
        '(with-selected-window (get-buffer-window "im-scale") (with-current-buffer "im-scale" (progn (redisplay t) (list (save-excursion (goto-char (point-min)) (let (h) (dotimes (_ 3) (push (line-pixel-height) h) (forward-line 1)) (nreverse h))) (save-excursion (goto-char 4) (posn-object-width-height (posn-at-point))) (save-excursion (goto-char (point-min)) (forward-line 1) (goto-char (+ (point) 3)) (posn-object-width-height (posn-at-point))) (image-size (im-spec "plain.png" :width (quote (2 . em))) t)))))',
        '(with-selected-window (get-buffer-window "im-scale") (with-current-buffer "im-scale" (progn (text-scale-set 2) t)))',
        '(with-selected-window (get-buffer-window "im-scale") (with-current-buffer "im-scale" (progn (redisplay t) (list (save-excursion (goto-char (point-min)) (let (h) (dotimes (_ 3) (push (line-pixel-height) h) (forward-line 1)) (nreverse h))) (save-excursion (goto-char 4) (posn-object-width-height (posn-at-point))) (save-excursion (goto-char (point-min)) (forward-line 1) (goto-char (+ (point) 3)) (posn-object-width-height (posn-at-point))) (image-size (im-spec "plain.png" :width (quote (2 . em))) t)))))',
        '(with-selected-window (get-buffer-window "im-scale") (with-current-buffer "im-scale" (progn (text-scale-set 0) t)))',
        '(with-selected-window (get-buffer-window "im-scale") (with-current-buffer "im-scale" (progn (redisplay t) (list (save-excursion (goto-char (point-min)) (let (h) (dotimes (_ 3) (push (line-pixel-height) h) (forward-line 1)) (nreverse h))) (save-excursion (goto-char 4) (posn-object-width-height (posn-at-point))) (save-excursion (goto-char (point-min)) (forward-line 1) (goto-char (+ (point) 3)) (posn-object-width-height (posn-at-point))) (image-size (im-spec "plain.png" :width (quote (2 . em))) t)))))',
    ],
}


def launch(engine, base, args):
    root, home, env = private_tree(engine, base, args.tools, f'{engine}-'.ljust(len('romacs-'), '_'))
    # The collector's checks (gc.rs), when the run asks for them.
    env.update({k: v for k, v in parity_live.os.environ.items() if k in ('ROMACS_GC_STRESS', 'ROMACS_GC_VERIFY')})
    chrome = '(menu-bar-mode -1) (tool-bar-mode -1) (scroll-bar-mode -1)\n' if engine == 'gnu' else ''
    init = INIT.format(chrome=chrome, probe=lisp_string(str(parity_live.PROBE)), fixtures=lisp_string(str(FIXTURES) + '/'),
                       root=lisp_string(str(parity_live.ROOT) + '/'))
    if engine == 'gnu':
        return gnu_engine(root, home, env, init, None, args.timeout, args.tools)
    return romacs_engine(args.romacs, root, home, env, init, None, args.timeout, args.tools)


def main():
    import argparse
    import hashlib
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument('--romacs', required=True)
    parser.add_argument('--gnu', default=parity_live.os.environ.get('ROMACS_GNU_ORACLE'))
    parser.add_argument('--output', required=True)
    parser.add_argument('--work', default=parity_live.os.environ.get('TMPDIR', '/tmp'))
    parser.add_argument('--tools', default=parity_live.TOOLS)
    parser.add_argument('--workflow', action='append', choices=list(WORKFLOWS))
    parser.add_argument('--timeout', type=float, default=120)
    args = parser.parse_args()
    parity_live.GNU = args.gnu
    output = Path(args.output)
    output.mkdir(parents=True)
    report = {'romacs': args.romacs, 'romacs_sha256': hashlib.sha256(Path(args.romacs).read_bytes()).hexdigest(),
              'gnu': args.gnu, 'workflows': []}
    for name in args.workflow or list(WORKFLOWS):
        entry = {'workflow': name, 'cases': [], 'status': 'FAIL'}
        report['workflows'].append(entry)
        engines = []
        try:
            for engine in ('gnu', 'romacs'):
                engines.append(launch(engine, Path(args.work), args))
            for engine in engines:
                engine.ask('(progn (parity-probe-mark-messages) t)')
            for form in WORKFLOWS[name]:
                values = {e.name: e.ask(form) for e in engines}
                entry['cases'].append({'form': form, 'gnu': values['gnu'], 'romacs': values['romacs'],
                                       'same': values['gnu'] == values['romacs']})
            messages = {e.name: e.ask('(parity-probe--messages)') for e in engines}
            entry['messages'] = messages
            entry['messages_same'] = messages['gnu'] == messages['romacs']
            entry['status'] = 'PASS' if all(c['same'] for c in entry['cases']) and entry['messages_same'] else 'DIFFER'
        except Exception as error:  # noqa: BLE001
            entry['error'] = repr(error)
        finally:
            for engine in engines:
                for log in ('engine.log', 'compositor.log'):
                    if (engine.root / log).exists():
                        shutil.copy(engine.root / log, output / f'{name}-{engine.name}-{log}')
                engine.stop()
                shutil.rmtree(engine.root, ignore_errors=True)
        differ = [c for c in entry['cases'] if not c['same']]
        print(name, entry['status'], entry.get('error', ''), f'{len(entry["cases"]) - len(differ)}/{len(entry["cases"])} same', flush=True)
        (output / 'report.json').write_text(json.dumps(report, indent=1, ensure_ascii=False, default=str) + '\n')
    return 0 if all(w['status'] == 'PASS' for w in report['workflows']) else 1


if __name__ == '__main__':
    sys.exit(main())
