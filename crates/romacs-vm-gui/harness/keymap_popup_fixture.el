;;; keymap_popup_fixture.el --- a gnosis-dashboard-shaped keymap-popup menu  -*- lexical-binding: t -*-
;; Maintainer: Thanos Apollo <public@thanosapollo.org>

;;; Commentary:
;; gnosis's dashboard menu (rlisp/gnosis/gnosis-dashboard.el), its
;; commands inert and its submenus small, for comparing keymap-popup's
;; rows and faces in GNU (keymap_popup_oracle.py) and in Romacs's panel
;; (popup_live.py --layer, chrome_live.py's keymap scene).
;; `keymap-popup-fixture-runs' lists a popup buffer's text runs and faces.

;;; Code:

(require 'keymap-popup)

(keymap-popup-define keymap-popup-fixture-nodes-map
  "Nodes"
  :group "Nodes" "n" ("Next node" ignore) "p" ("Previous node" ignore))

(keymap-popup-define keymap-popup-fixture-themata-map
  "Themata"
  :group "Themata" "a" ("All themata" ignore) "d" ("Due themata" ignore))

(keymap-popup-define keymap-popup-fixture-io-map
  "Import/Export"
  :group "Files" "i" ("Import" ignore) "e" ("Export" ignore))

(keymap-popup-define keymap-popup-fixture-maintenance-map
  "Maintenance"
  :group "Database" "v" ("Vacuum" ignore))

(keymap-popup-define keymap-popup-fixture-map
  "Gnosis Dashboard"
  :description "Gnosis Dashboard"
  :group "Navigate"
  "n" ("Nodes" :keymap keymap-popup-fixture-nodes-map)
  "t" ("Themata" :keymap keymap-popup-fixture-themata-map)
  "H" ("History" ignore)
  :group "Actions"
  "r" ("Review" ignore)
  "a" ("Add thema" ignore)
  "SPC" ("Search" ignore)
  :group "Study"
  "s" ("Study topic" ignore)
  "p" ("Practise topics" ignore)
  "w" ("Repair questions" ignore)
  "e" ("Exams" ignore)
  "R" ("Resume batch" ignore)
  :group "More"
  "x" ("Import/Export" :keymap keymap-popup-fixture-io-map)
  "!" ("Maintenance" :keymap keymap-popup-fixture-maintenance-map))

(defun keymap-popup-fixture--faces (value)
  "VALUE, a `face' property, as a sorted list of face names."
  (sort (mapcar #'symbol-name (seq-filter #'symbolp (ensure-list value))) #'string<))

(defun keymap-popup-fixture-runs (string)
  "STRING's runs that are not blank, as (TEXT FACES), in order: each run
a stretch of one `face' value, trimmed."
  (let ((pos 0) runs)
    (while (< pos (length string))
      (let* ((next (or (next-single-property-change pos 'face string) (length string)))
             (text (string-trim (substring-no-properties string pos next))))
        (unless (string-empty-p text)
          (dolist (line (split-string text "\n" t "[ \t]+"))
            (push (list line (keymap-popup-fixture--faces (get-text-property pos 'face string))) runs)))
        (setq pos next)))
    (nreverse runs)))

(defun keymap-popup-fixture--hex (color)
  "COLOR as #rrggbb."
  (apply #'format "#%02x%02x%02x" (mapcar (lambda (c) (/ c 256)) (color-values color))))

(defun keymap-popup-fixture-placed-runs (buffer)
  "BUFFER's runs that are not blank, as (TEXT FACES LINE COLUMN COLOR):
where each starts and the foreground its first face draws in."
  (with-current-buffer buffer
    (save-excursion
      (goto-char (point-min))
      (let (runs)
        (while (not (eobp))
          (let* ((next (or (next-single-property-change (point) 'face nil (line-end-position)) (line-end-position)))
                 (face (get-text-property (point) 'face)))
            (skip-chars-forward " \t" next)
            (when (< (point) next)
              (let ((text (string-trim (buffer-substring-no-properties (point) next)))
                    (first (car (ensure-list face))))
                (push (list text (keymap-popup-fixture--faces face) (1- (line-number-at-pos)) (current-column)
                            (keymap-popup-fixture--hex (or (and first (face-foreground first nil t)) (face-foreground 'default nil t))))
                      runs)))
            (goto-char next)
            (when (eolp) (forward-line 1))))
        (nreverse runs)))))

(provide 'keymap_popup_fixture)
;;; keymap_popup_fixture.el ends here
