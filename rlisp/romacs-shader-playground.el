;;; romacs-shader-playground.el --- Live WGSL editing with a GPU preview -*- lexical-binding: t -*-

;; SPDX-License-Identifier: GPL-3.0-or-later
;; Maintainer: Thanos Apollo <public@thanosapollo.org>

;;; Commentary:

;; A Shadertoy-style playground on Romacs shader surfaces
;; (docs/display-engine/SHADER_SURFACES.md): edit WGSL on the left, see the
;; compositor-rendered result on the right.
;;
;;   M-x romacs-shader-playground
;;
;;   C-c C-c   compile the buffer; on success the preview swaps to the new
;;             shader, on failure naga's diagnostics pop up and the preview
;;             keeps the last good shader
;;   C-c C-u   update one uniform live (no recompile)
;;   C-c C-l   toggle live recompile on idle after edits
;;   C-c C-k   close the preview and free its GPU surface
;;
;; Uniform initial values are declared in a `// uniforms:' comment line,
;; e.g. `// uniforms: (speed . 2.0) (tint . [1.0 0.5 0.2])'; each entry
;; generates a WGSL accessor (`u_speed()', `u_tint()').

;;; Code:

(require 'romacs-surface)

(defgroup romacs-shader-playground nil
  "Live WGSL editing with a GPU preview."
  :group 'multimedia
  :prefix "romacs-shader-playground-")

(defcustom romacs-shader-playground-size '(420 . 300)
  "Preview surface size in pixels as (WIDTH . HEIGHT)."
  :type '(cons natnum natnum))

(defcustom romacs-shader-playground-idle-delay 0.4
  "Idle seconds after an edit before a live recompile."
  :type 'number)

(defconst romacs-shader-playground--buffer "*shader-playground*")
(defconst romacs-shader-playground--preview-buffer "*shader-preview*")
(defconst romacs-shader-playground--error-buffer "*shader-playground-errors*")

(defvar romacs-shader-playground--surface nil
  "Surface id currently shown in the preview, or nil.")
(defvar romacs-shader-playground--idle-timer nil)

(defconst romacs-shader-playground--template
  "// Shader playground — press C-c C-c to compile, C-c C-l for live mode.
// On a compile error the preview keeps the last good shader.
//
// Contract: define  fn mainImage(fragCoord: vec2<f32>) -> vec4<f32>
// Built-ins: u.iTime, u.iTimeDelta, u.iFrame,
//            u.iResolution (xy = pixels, z = scale), u.iMouse (reserved).
// fragCoord is y-up (Shadertoy convention); return linear RGBA.
// Each `uniforms' entry below becomes an accessor: (speed . 1.0) -> u_speed().
// uniforms: (speed . 1.0)

fn mainImage(fragCoord: vec2<f32>) -> vec4<f32> {
    let uv = (2.0 * fragCoord - u.iResolution.xy) / u.iResolution.y;
    let t = u.iTime * u_speed();
    var col = 0.5 + 0.5 * cos(t + uv.xyx * 3.0 + vec3<f32>(0.0, 2.0, 4.0));
    let d = abs(length(uv) - 0.6 - 0.15 * sin(t));
    col += vec3<f32>(0.02 / max(d, 0.02));
    return vec4<f32>(col, 1.0);
}
"
  "Starter shader inserted into an empty playground buffer.")

(defvar-keymap romacs-shader-playground-mode-map
  :doc "Keymap for `romacs-shader-playground-mode'."
  "C-c C-c" #'romacs-shader-playground-compile
  "C-c C-u" #'romacs-shader-playground-set-uniform
  "C-c C-l" #'romacs-shader-playground-live-mode
  "C-c C-k" #'romacs-shader-playground-close-preview)

(defvar romacs-shader-playground-mode-syntax-table
  (let ((table (make-syntax-table prog-mode-syntax-table)))
    ;; C-style comments, shared by WGSL and GLSL: `//' to end of line
    ;; and `/* ... */' blocks.  Strings stay the standard `"' syntax.
    (modify-syntax-entry ?/ ". 124b" table)
    (modify-syntax-entry ?* ". 23" table)
    (modify-syntax-entry ?\n "> b" table)
    table)
  "Syntax table for `romacs-shader-playground-mode'.")

(defconst romacs-shader-playground--font-lock-keywords
  `(;; WGSL attributes and GLSL preprocessor directives.
    ("@\\(?:vertex\\|fragment\\|group\\|binding\\|builtin\\|location\\)\\_>"
     . 'font-lock-preprocessor-face)
    ("^[ \t]*#[ \t]*\\(?:define\\|version\\)\\_>"
     . 'font-lock-preprocessor-face)
    ;; Contract uniform-block reads (`u.iTime') and generated
    ;; per-uniform accessors (`u_speed()'), see SHADER_SURFACES.md.
    ("\\_<u\\.[[:alnum:]_]+" . 'font-lock-builtin-face)
    ("\\_<u_[[:alnum:]_]+" . 'font-lock-builtin-face)
    ;; Names the generated prelude provides to both dialects.
    (,(regexp-opt '("mainImage" "iTime" "iTimeDelta" "iFrame" "iResolution"
                    "iMouse" "iChannel0" "iChannel0Sampler" "textureSample"
                    "texture")
                  'symbols)
     . 'font-lock-builtin-face)
    (,(regexp-opt '("fn" "let" "var" "const" "return" "if" "else" "for"
                    "while" "loop" "break" "continue" "struct" "uniform"
                    "void" "in" "out")
                  'symbols)
     . 'font-lock-keyword-face)
    (,(regexp-opt '("f32" "u32" "i32" "bool" "vec2" "vec3" "vec4" "float"
                    "int" "mat2" "mat3" "mat4" "texture_2d" "sampler"
                    "sampler2D")
                  'symbols)
     . 'font-lock-type-face)
    ("\\_<\\(?:0[xX][[:xdigit:]]+\\|[0-9]+\\(?:\\.[0-9]*\\)?\\(?:[eE][-+]?[0-9]+\\)?\\)[fhiu]?\\_>"
     . 'font-lock-constant-face))
  "Font-lock keywords for WGSL and Shadertoy-dialect GLSL.")

(define-derived-mode romacs-shader-playground-mode prog-mode "WGSL-Play"
  "Major mode for editing a WGSL shader with a live Romacs surface preview."
  (setq-local comment-start "// ")
  (setq-local comment-end "")
  (setq-local comment-start-skip "\\(?://+\\|/\\*+\\)[ \t]*")
  (setq-local font-lock-defaults
              '(romacs-shader-playground--font-lock-keywords)))

;;;###autoload
(defun romacs-shader-playground ()
  "Open the shader playground: WGSL buffer plus a live GPU preview."
  (interactive)
  (unless (romacs-surface-available-p)
    (error "The shader playground needs the Romacs GUI"))
  (let ((buffer (get-buffer-create romacs-shader-playground--buffer)))
    (with-current-buffer buffer
      (unless (derived-mode-p 'romacs-shader-playground-mode)
        (romacs-shader-playground-mode))
      (when (zerop (buffer-size))
        (insert romacs-shader-playground--template)))
    (pop-to-buffer buffer '((display-buffer-reuse-window
                             display-buffer-same-window)))
    (romacs-shader-playground-compile)))

(defun romacs-shader-playground--uniforms ()
  "Parse the `// uniforms:' declaration line, if any.
Returns an alist suitable for `romacs-surface-create' :uniforms."
  (save-excursion
    (goto-char (point-min))
    (when (re-search-forward "^// *uniforms: *\\(.+\\)$" nil t)
      (condition-case nil
          (car (read-from-string (concat "(" (match-string 1) ")")))
        (error
         (message "shader-playground: ignoring malformed uniforms line")
         nil)))))

(defun romacs-shader-playground--language ()
  "Return the shader language keyword: `:glsl' when a `// language: glsl'
header line is present (Shadertoy-dialect GLSL, `void mainImage(out vec4,
in vec2)'), `:shader' (WGSL) otherwise."
  (save-excursion
    (goto-char (point-min))
    (if (re-search-forward "^// *language: *glsl *$" nil t)
        :glsl
      :shader)))

(defun romacs-shader-playground--show (id width height)
  "Show surface ID at WIDTH x HEIGHT in the preview window, freeing the old one."
  (let ((old romacs-shader-playground--surface)
        (preview (get-buffer-create romacs-shader-playground--preview-buffer)))
    (with-current-buffer preview
      (let ((inhibit-read-only t))
        (erase-buffer)
        (insert "\n ")
        (romacs-surface-insert id width height)
        (insert "\n"))
      (setq buffer-read-only t)
      (add-hook 'kill-buffer-hook #'romacs-shader-playground--release nil t))
    (setq romacs-shader-playground--surface id)
    (when (and old (/= old id))
      (romacs-surface-destroy old))
    (display-buffer preview
                    `((display-buffer-reuse-window
                       display-buffer-in-side-window)
                      (side . right)
                      (window-width . ,(+ 4 (ceiling (/ (float width)
                                                        (frame-char-width)))))))))

(defun romacs-shader-playground--release ()
  "Free the preview's GPU surface."
  (when romacs-shader-playground--surface
    (ignore-errors (romacs-surface-destroy romacs-shader-playground--surface))
    (setq romacs-shader-playground--surface nil)))

;; naga reports errors against the composed module — generated prelude,
;; then the user source — as `surface.wgsl:N:C' (WGSL) or
;; `surface.frag:N:C' (GLSL) references.  The prelude length is
;; deterministic; the constants below mirror `compose_surface_wgsl' and
;; `compose_surface_glsl' in crates/romacs-renderer-wgpu/src/shader_surface.rs
;; and must be updated in lockstep when the generated prelude changes.

(defconst romacs-shader-playground--wgsl-prelude-fixed-lines (+ 20 12)
  "Fixed WGSL prelude lines around the per-uniform accessors.
18 lines of NeoUniforms/channel declarations before the accessors, plus
12 lines of vertex/fragment entry points ending at the
`// ---- user source ----' marker.  Mirrors `compose_surface_wgsl' in
crates/romacs-renderer-wgpu/src/shader_surface.rs.")

(defconst romacs-shader-playground--glsl-prelude-fixed-lines (+ 20 2)
  "Fixed GLSL prelude lines around the per-uniform accessors.
20 lines of #version/uniform-block/#define declarations before the
accessors, plus 2 lines (the output declaration and the
`// ---- user source ----' marker).  Mirrors `compose_surface_glsl' in
crates/romacs-renderer-wgpu/src/shader_surface.rs.")

(defconst romacs-shader-playground--uniform-slots 8
  "User uniform slots; one accessor line is generated per slot used.
Mirrors SURFACE_USER_UNIFORM_SLOTS in
crates/romacs-renderer-wgpu/src/shader_surface.rs.")

(defun romacs-shader-playground--prelude-lines (uniforms language)
  "Lines the generated prelude occupies before the user source.
UNIFORMS is the parsed `:uniforms' alist and LANGUAGE `:glsl' or
`:shader'; each uniform contributes one accessor line, capped at the
slot count.  Composed-module line N is user-source line N minus this."
  (+ (if (eq language :glsl)
         romacs-shader-playground--glsl-prelude-fixed-lines
       romacs-shader-playground--wgsl-prelude-fixed-lines)
     (min (length uniforms) romacs-shader-playground--uniform-slots)))

(defun romacs-shader-playground--visit-line (line)
  "Select the playground buffer and move point to LINE (1-based)."
  (let ((buffer (get-buffer romacs-shader-playground--buffer)))
    (unless buffer
      (user-error "No shader playground buffer"))
    (pop-to-buffer buffer '((display-buffer-reuse-window)))
    (goto-char (point-min))
    (forward-line (1- line))))

(defun romacs-shader-playground--annotate-error-lines (prelude-lines)
  "Follow each composed-module line reference with a jump button.
Scan the current buffer for naga's `surface.wgsl:N:C' / `surface.frag:N:C'
references and append a clickable \"(your line M)\" hint after each,
where M = N - PRELUDE-LINES, jumping to line M in the playground
buffer.  References into the generated prelude (M < 1) are left alone;
the naga text itself is not modified."
  (save-excursion
    (goto-char (point-min))
    (while (re-search-forward
            "surface\\.\\(?:wgsl\\|frag\\):\\([0-9]+\\):[0-9]+" nil t)
      (let ((line (- (string-to-number (match-string 1)) prelude-lines)))
        (when (>= line 1)
          (insert " "
                  (buttonize
                   (format "(your line %d)" line)
                   #'romacs-shader-playground--visit-line line
                   "mouse-2, RET: jump to this line in the playground")))))))

(defun romacs-shader-playground--show-errors (message &optional prelude-lines)
  "Pop the error buffer with naga's MESSAGE.
Non-nil PRELUDE-LINES is the generated-prelude line count
\(`romacs-shader-playground--prelude-lines'); composed-module line
references in MESSAGE then get clickable playground-line hints."
  (with-current-buffer (get-buffer-create romacs-shader-playground--error-buffer)
    (let ((inhibit-read-only t))
      (erase-buffer)
      (insert message "\n")
      (when prelude-lines
        (romacs-shader-playground--annotate-error-lines prelude-lines))
      (insert "\n(Line numbers refer to the composed module: the generated\n"
              " prelude precedes your code.  The annotated source lines above\n"
              " point at the offending shader source; each \"(your line N)\"\n"
              " button jumps to that line in the playground buffer.)\n")
      (goto-char (point-min)))
    (special-mode)
    (display-buffer (current-buffer)
                    '((display-buffer-reuse-window
                       display-buffer-at-bottom)
                      (window-height . 0.3)))))

(defun romacs-shader-playground--hide-errors ()
  "Bury the error buffer if it is showing."
  (when-let* ((buffer (get-buffer romacs-shader-playground--error-buffer))
              (window (get-buffer-window buffer)))
    (quit-window nil window)))

(defun romacs-shader-playground-compile (&optional quiet)
  "Compile the playground buffer into the preview surface.
With QUIET non-nil (live mode), report errors in the echo area instead of
popping the error buffer."
  (interactive)
  (let* ((source (buffer-substring-no-properties (point-min) (point-max)))
         (uniforms (romacs-shader-playground--uniforms))
         (language (romacs-shader-playground--language))
         (width (car romacs-shader-playground-size))
         (height (cdr romacs-shader-playground-size)))
    (condition-case err
        (let ((id (apply #'romacs-surface-create language source
                         :width width :height height :animate t
                         (and uniforms (list :uniforms uniforms)))))
          (romacs-shader-playground--show id width height)
          (romacs-shader-playground--hide-errors)
          (message "Shader compiled (surface %s)" id)
          t)
      (error
       (if quiet
           (message "shader error: %s"
                    (car (split-string (error-message-string err) "\n")))
         (romacs-shader-playground--show-errors
          (error-message-string err)
          (romacs-shader-playground--prelude-lines uniforms language)))
       nil))))

(defun romacs-shader-playground-set-uniform (name value)
  "Set uniform NAME to VALUE on the current preview surface, live.
VALUE is a number or a vector of up to four numbers."
  (interactive
   (list (intern (read-string "Uniform name: "))
         (car (read-from-string (read-string "Value (number or [x y z]): ")))))
  (unless romacs-shader-playground--surface
    (error "No live preview surface; compile first (C-c C-c)"))
  (romacs-surface-set-uniform romacs-shader-playground--surface name value)
  (message "%s = %s" name value))

(defun romacs-shader-playground-close-preview ()
  "Close the preview window and free its GPU surface."
  (interactive)
  (when-let* ((buffer (get-buffer romacs-shader-playground--preview-buffer)))
    (kill-buffer buffer))
  (romacs-shader-playground--hide-errors))

(define-minor-mode romacs-shader-playground-live-mode
  "Recompile the playground shader after every pause in editing."
  :lighter " Live"
  (if romacs-shader-playground-live-mode
      (add-hook 'after-change-functions
                #'romacs-shader-playground--schedule nil t)
    (remove-hook 'after-change-functions
                 #'romacs-shader-playground--schedule t)
    (when romacs-shader-playground--idle-timer
      (cancel-timer romacs-shader-playground--idle-timer)
      (setq romacs-shader-playground--idle-timer nil))))

(defun romacs-shader-playground--schedule (&rest _)
  "Debounce a live recompile of the playground buffer."
  (when romacs-shader-playground--idle-timer
    (cancel-timer romacs-shader-playground--idle-timer))
  (setq romacs-shader-playground--idle-timer
        (run-with-idle-timer
         romacs-shader-playground-idle-delay nil
         (lambda ()
           (setq romacs-shader-playground--idle-timer nil)
           (when-let* ((buffer (get-buffer romacs-shader-playground--buffer)))
             (with-current-buffer buffer
               (romacs-shader-playground-compile t)))))))

(provide 'romacs-shader-playground)
;;; romacs-shader-playground.el ends here
