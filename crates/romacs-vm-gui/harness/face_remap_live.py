#!/usr/bin/env python3
"""`face-remapping-alist' in pinned GNU and in Romacs GUI --loadup, side by side.

A buffer of plain text, a keyword in `font-lock-keyword-face', a pixel-sized
and an `em'-sized image, measured as each engine displays it (the GUI pair of
images_live.py and parity_live.py, Iosevka 14 on both sides): line heights,
`window-text-pixel-size' of the buffer and of one line, and `posn-at-point'
of the keyword and of the `em' image, under

  text-scale  text-scale-set 2, then -1, then 0;
  family      buffer-face-mode with another family (DejaVu Serif);
  face        face-remap-add-relative of font-lock-keyword-face to twice
              its height, then its removal;
  filtered    a remapping of `default' to twice its height filtered on a
              window parameter, before and after the window has it.

COMPARISON POLICY: every form's value exactly; under `:filtered' the
keyword's position but not its size (VM2-FACE-REMAP).

usage: face_remap_live.py --romacs BIN [--gnu EMACS] --output DIR [--workflow NAME]...
"""
import sys

import images_live

SETUP = ('(progn (switch-to-buffer (get-buffer-create "fr")) (delete-other-windows) (erase-buffer) '
         '(insert "plain line\\n" "with " (propertize "keyword" (quote face) (quote font-lock-keyword-face)) " after\\n") '
         '(insert "px ") (insert-image (im-spec "plain.png" :width 40)) (insert " em ") '
         '(insert-image (im-spec "plain.png" :width (quote (2 . em)))) (insert "\\nend\\n") (goto-char (point-min)) (redisplay t) t)')

PROBE = ('(progn (redisplay t) (list '
         '(save-excursion (goto-char (point-min)) (let (h) (dotimes (_ 4) (push (line-pixel-height) h) (forward-line 1)) (nreverse h))) '
         '(window-text-pixel-size nil (point-min) (point-max)) '
         '(save-excursion (goto-char (point-min)) (forward-line 1) (window-text-pixel-size nil (point) (line-end-position))) '
         '(save-excursion (goto-char (point-min)) (search-forward "key") (let ((p (posn-at-point (match-beginning 0)))) (list (posn-x-y p) (posn-object-width-height p)))) '
         '(save-excursion (goto-char (point-min)) (search-forward "em ") (let ((p (posn-at-point))) (list (posn-x-y p) (posn-object-width-height p))))))')


# Under a `:filtered' remapping GNU's posn-at-point sizes a remapped glyph
# without the window's filter (10x25 where it paints 19x48); Romacs answers
# the painted size (DIVERGENCES.md VM2-FACE-REMAP).  The filtered workflow
# compares the keyword's position, not its size.
PROBE_FILTERED = PROBE.replace('(list (posn-x-y p) (posn-object-width-height p)))) (save-excursion (goto-char (point-min)) (search-forward "em ")',
                               '(posn-x-y p))) (save-excursion (goto-char (point-min)) (search-forward "em ")')
assert PROBE_FILTERED != PROBE


def in_buffer(form):
    return f'(with-selected-window (get-buffer-window "fr") (with-current-buffer "fr" {form}))'


WORKFLOWS = {
    'text-scale': [SETUP, in_buffer(PROBE),
                   in_buffer('(progn (text-scale-set 2) t)'), in_buffer(PROBE),
                   in_buffer('(progn (text-scale-set -1) t)'), in_buffer(PROBE),
                   in_buffer('(progn (text-scale-set 0) face-remapping-alist)'), in_buffer(PROBE)],
    'family': [SETUP, in_buffer('(progn (buffer-face-set :family "DejaVu Serif") t)'), in_buffer(PROBE),
               in_buffer('(progn (buffer-face-mode -1) face-remapping-alist)'), in_buffer(PROBE)],
    'face': [SETUP,
             in_buffer('(progn (setq fr-cookie (face-remap-add-relative (quote font-lock-keyword-face) :height 2.0)) t)'),
             in_buffer(PROBE),
             in_buffer('(progn (face-remap-remove-relative fr-cookie) face-remapping-alist)'), in_buffer(PROBE)],
    'filtered': [SETUP,
                 in_buffer('(progn (setq-local face-remapping-alist (quote ((default (:filtered (:window fr-wide t) (:height 2.0)) default)))) t)'),
                 in_buffer(PROBE_FILTERED),
                 in_buffer('(progn (set-window-parameter nil (quote fr-wide) t) t)'), in_buffer(PROBE_FILTERED),
                 in_buffer('(progn (set-window-parameter nil (quote fr-wide) nil) t)'), in_buffer(PROBE_FILTERED)],
}

if __name__ == '__main__':
    images_live.WORKFLOWS = WORKFLOWS
    sys.exit(images_live.main())
