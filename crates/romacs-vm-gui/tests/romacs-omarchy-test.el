;;; romacs-omarchy-test.el --- `omarchy-mode' -*- lexical-binding: t -*-
;; SPDX-License-Identifier: GPL-3.0-or-later
;; Maintainer: Thanos Apollo <public@thanosapollo.org>

;; rlisp/romacs-omarchy.el over a fake Omarchy state directory, in a
;; `--loadup' session (gnu_completion.rs `romacs_omarchy_tests_pass').
;; These call the follow directly; harness/omarchy_live.py proves the
;; watch, the delay and the frame.

(require 'romacs-omarchy)
(require 'romacs-defaults)

(defconst romacs-omarchy-test--palette
  "mode = \"dark\"\naccent = \"#7aa2f7\"\nbackground = \"#101418\"\nforeground = \"#d8dee9\"\nred = \"#e06c75\"\ngreen = \"#98c379\"\nyellow = \"#e5c07b\"\nblue = \"#61afef\"\nmagenta = \"#c678dd\"\ncyan = \"#56b6c2\"\n"
  "A colors.toml.")

(defun romacs-omarchy-test--write (generation name &optional palette)
  "Write what Omarchy writes on a switch, in its order, to the state directory."
  (let ((theme (expand-file-name "theme" omarchy-state-directory)))
    (make-directory theme t)
    (let ((file (expand-file-name "colors.toml" theme)))
      (if palette
          (with-temp-file file (insert palette))
        (when (file-exists-p file) (delete-file file)))))
  (with-temp-file (expand-file-name "theme.generation" omarchy-state-directory) (insert generation "\n"))
  (with-temp-file (expand-file-name "theme.name" omarchy-state-directory) (insert name "\n")))

(defmacro romacs-omarchy-test--with-state (&rest body)
  "BODY with a fake state directory, the stock theme restored after it."
  (declare (indent 0))
  `(let* ((omarchy-state-directory (file-name-as-directory (make-temp-file "romacs-omarchy-" t)))
          (omarchy--seen nil) (omarchy-mode-paused nil) (omarchy--pause-told nil)
          (romacs-theme--stock t))
     (unwind-protect
         (progn ,@body)
       (remove-hook 'enable-theme-functions #'omarchy--note-theme)
       (mapc #'disable-theme custom-enabled-themes)
       (enable-theme 'default)
       (delete-directory omarchy-state-directory t))))

(romacs-deftest romacs-omarchy-is-detected-by-its-state ()
  (romacs-omarchy-test--with-state
    (romacs-should (omarchy-detected-p)))
  (let ((omarchy-state-directory "/nonexistent/omarchy/"))
    (romacs-should (not (omarchy-detected-p)))))

(romacs-deftest romacs-omarchy-the-mode-is-a-default-only-with-omarchy ()
  ;; Its setting reads the system each session, not the world.
  (romacs-should (equal (assq 'omarchy-mode romacs-defaults-settings) '(omarchy-mode (omarchy-detected-p) idle))))

(romacs-deftest romacs-omarchy-names-map-to-themes ()
  (romacs-should (eq (omarchy-romacs-theme "rum") 'rum))
  (romacs-should (eq (omarchy-romacs-theme "tokyo-night") 'omarchy-tokyo-night))
  (romacs-should (eq (omarchy-romacs-theme "something-new") 'omarchy-something-new)))

(romacs-deftest romacs-omarchy-a-shipped-theme-follows ()
  (romacs-omarchy-test--with-state
    (romacs-omarchy-test--write "g1" "tokyo-night" romacs-omarchy-test--palette)
    (omarchy--changed)
    (romacs-should (equal custom-enabled-themes '(omarchy-tokyo-night)))
    (romacs-omarchy-test--write "g2" "rum" romacs-omarchy-test--palette)
    (omarchy--changed)
    (romacs-should (equal custom-enabled-themes '(rum)))))

(romacs-deftest romacs-omarchy-an-unshipped-theme-is-made-from-its-palette ()
  (romacs-omarchy-test--with-state
    (romacs-omarchy-test--write "g1" "romacs-test-unshipped" romacs-omarchy-test--palette)
    (omarchy--changed)
    (romacs-should (equal custom-enabled-themes '(omarchy-romacs-test-unshipped)))
    (romacs-should (equal (face-attribute 'default :background) "#101418"))
    ;; Its palette may change under one name.
    (romacs-omarchy-test--write "g2" "romacs-test-unshipped"
                                (replace-regexp-in-string "#101418" "#201418" romacs-omarchy-test--palette))
    (omarchy--changed)
    (romacs-should (equal (face-attribute 'default :background) "#201418"))))

(romacs-deftest romacs-omarchy-a-palette-without-a-mode-is-read-from-its-background ()
  (romacs-omarchy-test--with-state
    (romacs-omarchy-test--write "g1" "romacs-test-modeless"
                                "background = \"#fafafa\"\nforeground = \"#2a2f38\"\nred = \"#c0343c\"\ngreen = \"#3e7d2f\"\nyellow = \"#8a6a00\"\nblue = \"#2a62c9\"\nmagenta = \"#8b3fb5\"\ncyan = \"#1f7f8c\"\n")
    (omarchy--changed)
    (romacs-should (eq (plist-get (get 'omarchy-romacs-test-modeless 'theme-properties) :background-mode) 'light))))

(romacs-deftest romacs-omarchy-a-theme-with-no-palette-says-so ()
  (romacs-omarchy-test--with-state
    (romacs-omarchy-test--write "g1" "romacs-test-bare")
    (let ((said nil))
      (cl-letf (((symbol-function 'message)
                 (lambda (format &rest args) (push (apply #'format-message format args) said))))
        (omarchy--changed))
      (romacs-should (string-prefix-p "omarchy-mode: " (car said))))
    ;; The theme showing stays.
    (romacs-should (equal custom-enabled-themes '(default)))))

(romacs-deftest romacs-omarchy-a-palette-short-of-colours-leaves-the-theme ()
  (romacs-omarchy-test--with-state
    (romacs-omarchy-test--write "g1" "romacs-test-short" "mode = \"dark\"\nbackground = \"#101418\"\nforeground = \"#d8dee9\"\n")
    (omarchy--changed)
    (romacs-should (equal custom-enabled-themes '(default)))))

(romacs-deftest romacs-omarchy-a-name-not-yet-written-waits ()
  ;; Omarchy writes the counter, then the name: until the name's write the
  ;; state on disk is no theme, and the name's own event brings it.
  (romacs-omarchy-test--with-state
    (romacs-omarchy-test--write "g1" "tokyo-night" romacs-omarchy-test--palette)
    (omarchy--changed)
    (sleep-for 0.02)
    (with-temp-file (expand-file-name "theme.generation" omarchy-state-directory) (insert "g2\n"))
    (romacs-should (null (omarchy-current)))
    (omarchy--changed)
    (romacs-should (equal custom-enabled-themes '(omarchy-tokyo-night)))
    (sleep-for 0.02)
    (with-temp-file (expand-file-name "theme.name" omarchy-state-directory) (insert ""))
    (romacs-should (null (omarchy-current)))
    (with-temp-file (expand-file-name "theme.name" omarchy-state-directory) (insert "rum\n"))
    (omarchy--changed)
    (romacs-should (equal custom-enabled-themes '(rum)))))

(romacs-deftest romacs-omarchy-a-state-written-while-read-is-no-state ()
  ;; The writer finishes between the two reads: the name read is the old
  ;; one, the counter the new one, and no theme may take that palette.
  (romacs-omarchy-test--with-state
    (romacs-omarchy-test--write "g1" "romacs-test-old" romacs-omarchy-test--palette)
    (omarchy--changed)
    (let* ((real (symbol-function 'omarchy--read)) (done nil)
           (other (replace-regexp-in-string "#101418" "#301418" romacs-omarchy-test--palette)))
      (cl-letf (((symbol-function 'omarchy--read)
                 (lambda (file)
                   (prog1 (funcall real file)
                     (when (and (equal file "theme.generation") (not done))
                       (setq done t)
                       (sleep-for 0.02)
                       (romacs-omarchy-test--write "g2" "romacs-test-new" other))))))
        (romacs-should (null (omarchy-current)))))
    (romacs-should (equal (cdr (assq 'bg (romacs-theme-palette 'omarchy-romacs-test-old))) "#101418"))))

(romacs-deftest romacs-omarchy-a-palette-of-a-later-switch-is-left-to-it ()
  (romacs-omarchy-test--with-state
    (romacs-omarchy-test--write "g1" "romacs-test-old" romacs-omarchy-test--palette)
    (omarchy--changed)
    (romacs-omarchy-test--write "g2" "romacs-test-mid" romacs-omarchy-test--palette)
    (let ((real (symbol-function 'omarchy--palette)))
      (cl-letf (((symbol-function 'omarchy--palette)
                 (lambda ()
                   (prog1 (funcall real)
                     (sleep-for 0.02)
                     (romacs-omarchy-test--write "g3" "romacs-test-late" romacs-omarchy-test--palette)))))
        (omarchy--changed)))
    (romacs-should (equal custom-enabled-themes '(omarchy-romacs-test-old)))
    (omarchy--changed)
    (romacs-should (equal custom-enabled-themes '(omarchy-romacs-test-late)))))

(romacs-deftest romacs-omarchy-a-command-finishes-first ()
  ;; The debounce's end follows only at rest, or at the command's end.
  (romacs-omarchy-test--with-state
    (romacs-omarchy-test--write "g1" "tokyo-night" romacs-omarchy-test--palette)
    (unwind-protect
        (cl-letf (((symbol-function 'omarchy--at-rest-p) (lambda () nil))
                  ((symbol-function 'run-with-idle-timer) #'ignore))
          (omarchy--wait-for-boundary)
          (omarchy--idle)
          (romacs-should omarchy--pending)
          (romacs-should (member 'default custom-enabled-themes))
          (run-hooks 'post-command-hook)
          (romacs-should (not omarchy--pending))
          (romacs-should (equal custom-enabled-themes '(omarchy-tokyo-night)))
          (romacs-should (not (memq #'omarchy--boundary post-command-hook))))
      (remove-hook 'post-command-hook #'omarchy--boundary))))

(romacs-deftest romacs-omarchy-no-change-follows-nothing ()
  (romacs-omarchy-test--with-state
    (romacs-omarchy-test--write "g1" "tokyo-night" romacs-omarchy-test--palette)
    (omarchy--changed)
    (let ((follows 0))
      (cl-letf (((symbol-function 'omarchy-follow) (lambda () (setq follows (1+ follows)))))
        (omarchy--changed)
        (romacs-should (= follows 0))
        ;; A new counter with its name is a change, even under one name.
        (romacs-omarchy-test--write "g2" "tokyo-night" romacs-omarchy-test--palette)
        (omarchy--changed)
        (romacs-should (= follows 1))))))

(romacs-deftest romacs-omarchy-a-theme-by-hand-pauses-until-the-next-change ()
  (romacs-omarchy-test--with-state
    (add-hook 'enable-theme-functions #'omarchy--note-theme)
    (romacs-omarchy-test--write "g1" "tokyo-night" romacs-omarchy-test--palette)
    (omarchy--changed)
    (romacs-should (not omarchy-mode-paused))
    (let ((said nil))
      (cl-letf (((symbol-function 'message)
                 (lambda (format &rest args) (push (apply #'format-message format args) said))))
        (load-theme 'nord t)
        (load-theme 'dracula t))
      (romacs-should omarchy-mode-paused)
      (romacs-should (equal (seq-filter (lambda (text) (string-prefix-p "omarchy-mode" text)) said)
                            '("omarchy-mode: following paused until the next Omarchy theme change"))))
    (romacs-should (equal custom-enabled-themes '(dracula)))
    (romacs-omarchy-test--write "g2" "rum" romacs-omarchy-test--palette)
    (omarchy--changed)
    (romacs-should (not omarchy-mode-paused))
    (romacs-should (equal custom-enabled-themes '(rum)))))

(romacs-deftest romacs-omarchy-a-third-party-scheme-yields-to-the-desktop ()
  (romacs-omarchy-test--with-state
    (deftheme romacs-omarchy-test-third)
    (custom-theme-set-faces 'romacs-omarchy-test-third '(default ((t :background "#123456"))))
    (enable-theme 'romacs-omarchy-test-third)
    (romacs-omarchy-test--write "g1" "tokyo-night" romacs-omarchy-test--palette)
    (omarchy--changed)
    (romacs-should (equal custom-enabled-themes '(omarchy-tokyo-night)))))

(romacs-deftest romacs-omarchy-an-init-s-theme-stays ()
  (romacs-omarchy-test--with-state
    (romacs-omarchy-test--write "g1" "tokyo-night" romacs-omarchy-test--palette)
    (setq romacs-theme--stock nil)
    (unwind-protect
        (progn
          (omarchy-mode 1)
          (romacs-should omarchy-mode-paused)
          (romacs-should (member 'default custom-enabled-themes)))
      (omarchy-mode -1))
    (romacs-should (null omarchy--watch))
    (romacs-should (not (memq #'omarchy--note-theme enable-theme-functions)))))

;;; romacs-omarchy-test.el ends here
