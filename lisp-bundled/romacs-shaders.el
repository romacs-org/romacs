;;; romacs-shaders.el --- Shader showcase: gallery and frame effects -*- lexical-binding: t -*-

;; Copyright (C) 2026 Free Software Foundation, Inc.

;; Author: Romacs Contributors
;; Keywords: multimedia, graphics

;; This file is part of GNU Emacs.

;; GNU Emacs is free software: you can redistribute it and/or modify
;; it under the terms of the GNU General Public License as published by
;; the Free Software Foundation, either version 3 of the License, or
;; (at your option) any later version.

;; GNU Emacs is distributed in the hope that it will be useful,
;; but WITHOUT ANY WARRANTY; without even the implied warranty of
;; MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE.  See the
;; GNU General Public License for more details.

;; You should have received a copy of the GNU General Public License
;; along with GNU Emacs.  If not, see <https://www.gnu.org/licenses/>.

;;; Commentary:

;; A small collection built on shader surfaces
;; (docs/display-engine/SHADER_SURFACES.md):
;;
;;   M-x romacs-shaders-gallery   inline surfaces: raymarched 3D, an
;;                                 iMouse toy, Shadertoy GLSL pasted verbatim
;;   M-x romacs-shaders-crt       CRT frame shader over the whole editor
;;                                 (barrel distortion, scanlines, vignette;
;;                                 Shadertoy-dialect GLSL)
;;   M-x romacs-shaders-crt-curvature
;;                                 retune the CRT barrel distortion live
;;                                 (frame-shader custom uniform; no recompile)
;;   M-x romacs-shaders-glow      soft bloom frame shader (WGSL)
;;   M-x romacs-shaders-matrix    digital-rain frame shader (GLSL)
;;   M-x romacs-shaders-off       remove the frame shader

;;; Code:

(require 'romacs-surface)

;;;; Gallery surfaces

(defconst romacs-shaders--raymarch-wgsl
  "fn mainImage(fragCoord: vec2<f32>) -> vec4<f32> {
    let res = u.iResolution.xy;
    let p = (2.0 * fragCoord - res) / res.y;
    let t = u.iTime * 0.7;
    let ro = vec3<f32>(0.0, 0.0, -2.6);
    let rd = normalize(vec3<f32>(p, 1.6));
    var d = 0.0;
    var glow = 0.0;
    var it = 0;
    for (var i = 0; i < 72; i++) {
        it = i;
        let pos = ro + rd * d;
        let c = cos(t);
        let s = sin(t);
        let q = vec3<f32>(c * pos.x - s * pos.z, pos.y, s * pos.x + c * pos.z);
        let dist = length(vec2<f32>(length(q.xz) - 0.9, q.y)) - 0.32;
        if (dist < 0.001 || d > 8.0) { break; }
        glow += 0.02 / (0.1 + dist);
        d += dist;
    }
    if (d > 8.0) {
        return vec4<f32>(0.02 + glow * 0.02, 0.02 + glow * 0.03, 0.06 + glow * 0.05, 1.0);
    }
    let g = 1.0 - f32(it) / 72.0;
    return vec4<f32>(0.2 + 0.8 * g, 0.15 + 0.5 * g, 0.35 + 0.55 * g, 1.0);
}"
  "Raymarched spinning torus — 3D from a fragment shader, no mesh API.")

(defconst romacs-shaders--ripple-wgsl
  "fn mainImage(fragCoord: vec2<f32>) -> vec4<f32> {
    let d = distance(fragCoord, u.iMouse.xy) / u.iResolution.y;
    let ring = sin(42.0 * d - u.iTime * 5.0);
    let glow = smoothstep(0.45, 0.0, d);
    let c = 0.5 + 0.5 * ring;
    return vec4<f32>(0.08 + 0.55 * c * glow,
                     0.15 + 0.55 * glow,
                     0.45 + 0.45 * c,
                     1.0);
}"
  "Rings radiating from the pointer (u.iMouse) — hover to play.")

(defconst romacs-shaders--shadertoy-glsl
  "void mainImage(out vec4 fragColor, in vec2 fragCoord) {
    vec2 uv = fragCoord / iResolution.xy;
    vec3 col = 0.5 + 0.5 * cos(iTime + uv.xyx + vec3(0, 2, 4));
    fragColor = vec4(col, 1.0);
}"
  "Shadertoy's default new-shader template, pasted verbatim.")

;;;###autoload
(defun romacs-shaders-gallery ()
  "Pop a buffer showing the shader showcase; killing it frees the surfaces."
  (interactive)
  (unless (romacs-surface-available-p)
    (error "Shader surfaces need the Romacs GUI"))
  (with-current-buffer (get-buffer-create "*shader-gallery*")
    (erase-buffer)
    (insert "Raymarched 3D (WGSL fragment shader; no mesh API):\n\n  ")
    (romacs-surface-create-and-insert
     :width 320 :height 180 :animate t
     :shader romacs-shaders--raymarch-wgsl)
    (insert "\n\nPointer ripple (u.iMouse — move the mouse over it):\n\n  ")
    (romacs-surface-create-and-insert
     :width 320 :height 140 :animate t
     :shader romacs-shaders--ripple-wgsl)
    (insert "\n\nShadertoy GLSL, pasted verbatim (:glsl):\n\n  ")
    (romacs-surface-create-and-insert
     :width 320 :height 140 :animate t
     :glsl romacs-shaders--shadertoy-glsl)
    (let ((splash (expand-file-name "etc/images/splash.png" source-directory)))
      (when (file-exists-p splash)
        (insert "\n\nAn image sampled through a shader (:channel0 (image ...)):\n\n  ")
        (romacs-surface-create-and-insert
         :width 320 :height 200 :animate t
         :channel0 (list 'image :type 'png :file splash)
         :shader "fn mainImage(fragCoord: vec2<f32>) -> vec4<f32> {
    var uv = fragCoord / u.iResolution.xy;
    uv.y = 1.0 - uv.y;
    uv.x += 0.04 * sin(u.iTime * 1.5 + uv.y * 9.0);
    var col = textureSample(iChannel0, iChannel0Sampler, uv).rgb;
    let wave = 0.5 + 0.5 * sin(u.iTime + uv.x * 6.2832);
    col = mix(col, vec3<f32>(col.g, col.b, col.r), wave * 0.6);
    return vec4<f32>(col, 1.0);
}")))
    (insert "\n\nFrame effects: M-x romacs-shaders-crt / -glow / -matrix / -off\n")
    (goto-char (point-min))
    (pop-to-buffer (current-buffer))))

;;;; Frame shaders

(defconst romacs-shaders--crt-glsl
  "void mainImage(out vec4 fragColor, in vec2 fragCoord) {
    vec2 uv = vec2(fragCoord.x, iResolution.y - fragCoord.y) / iResolution.xy;
    vec2 c = uv - 0.5;
    float r2 = dot(c, c);
    uv = 0.5 + c * (1.0 + u_curvature() * r2);
    vec3 col;
    if (uv.x < 0.0 || uv.x > 1.0 || uv.y < 0.0 || uv.y > 1.0) {
        col = vec3(0.0);
    } else {
        col = texture(iChannel0, uv).rgb;
    }
    float scan = 0.88 + 0.12 * sin(fragCoord.y * 2.8);
    col *= scan;
    col *= vec3(1.03, 0.99, 0.93);
    col *= 1.0 - 0.4 * r2;
    fragColor = vec4(col, 1.0);
}"
  "CRT: barrel distortion, scanlines, warm tint, vignette (GLSL).
The distortion strength is the `curvature' custom uniform
\(`u_curvature()'), retunable live with `romacs-shaders-crt-curvature'.")

(defconst romacs-shaders--glow-wgsl
  "fn mainImage(fragCoord: vec2<f32>) -> vec4<f32> {
    let res = u.iResolution.xy;
    let uv = vec2<f32>(fragCoord.x, res.y - fragCoord.y) / res;
    var col = textureSample(iChannel0, iChannel0Sampler, uv).rgb;
    var glow = vec3<f32>(0.0);
    let px = 2.5 / res;
    for (var i = -1; i <= 1; i++) {
        for (var j = -1; j <= 1; j++) {
            let offs = vec2<f32>(f32(i), f32(j)) * px;
            let s = textureSample(iChannel0, iChannel0Sampler, uv + offs).rgb;
            glow += max(s - vec3<f32>(0.65), vec3<f32>(0.0));
        }
    }
    col += glow * 0.10;
    return vec4<f32>(col, 1.0);
}"
  "Soft bloom: bright pixels bleed into their neighborhood (WGSL).")

(defconst romacs-shaders--matrix-glsl
  "float mhash(vec2 p) { return fract(sin(dot(p, vec2(127.1, 311.7))) * 43758.5453); }
void mainImage(out vec4 fragColor, in vec2 fragCoord) {
    vec2 uv = vec2(fragCoord.x, iResolution.y - fragCoord.y) / iResolution.xy;
    vec3 col = texture(iChannel0, uv).rgb * 0.6;
    float colw = 14.0;
    float x = floor(fragCoord.x / colw);
    float speed = 90.0 * (0.4 + 0.6 * mhash(vec2(x, 1.0)));
    float y = fragCoord.y + iTime * speed + mhash(vec2(x, 7.0)) * 900.0;
    float cell = floor(y / colw);
    float ch = step(0.6, mhash(vec2(x, cell)));
    float fade = pow(fract(y / 320.0), 3.0);
    col += vec3(0.15, 1.0, 0.35) * ch * fade * 0.55;
    fragColor = vec4(col, 1.0);
}"
  "Digital rain falling over the editor content (GLSL).")

;;;###autoload
(defun romacs-shaders-crt ()
  "Apply the CRT frame shader to the whole editor."
  (interactive)
  (romacs-frame-shader romacs-shaders--crt-glsl 'glsl
                        '((curvature . 0.10)))
  (message "CRT on — M-x romacs-shaders-crt-curvature tunes it, M-x romacs-shaders-off removes"))

;;;###autoload
(defun romacs-shaders-crt-curvature (amount)
  "Set the CRT frame shader's barrel-distortion AMOUNT live.
0.0 is flat, 0.10 is the default, 0.3 is heavy.  Updates the
`curvature' uniform on the installed frame shader without
recompiling; signals an error if no frame shader is installed
\(run `romacs-shaders-crt' first)."
  (interactive "nCRT curvature (0.0 flat, 0.10 default, 0.3 heavy): ")
  (romacs-frame-shader-set-uniform 'curvature (float amount))
  (message "CRT curvature: %s" amount))

;;;###autoload
(defun romacs-shaders-glow ()
  "Apply the bloom frame shader to the whole editor."
  (interactive)
  (romacs-frame-shader romacs-shaders--glow-wgsl)
  (message "Glow on — M-x romacs-shaders-off to remove"))

;;;###autoload
(defun romacs-shaders-matrix ()
  "Apply the digital-rain frame shader to the whole editor."
  (interactive)
  (romacs-frame-shader romacs-shaders--matrix-glsl 'glsl)
  (message "Rain on — M-x romacs-shaders-off to remove"))

;;;###autoload
(defun romacs-shaders-off ()
  "Remove the frame shader."
  (interactive)
  (romacs-frame-shader nil)
  (message "Frame shader removed"))

;;;; Ghostty ports
;;
;; Three shaders taken verbatim from the public Ghostty custom-shader
;; collection <https://github.com/hackr-sh/ghostty-shaders>.  Ghostty's
;; `custom-shader' contract is the Shadertoy dialect Romacs accepts
;; (`void mainImage(out vec4, in vec2)' with iTime/iResolution/iMouse
;; and the terminal frame bound as iChannel0), so the corpus pastes in:
;; all three compile under naga 29 UNCHANGED — including starfield's
;; uninitialized `for (int l; ...)' loop variable and `1 - step(...)'
;; int/float mix, and the Lottes CRT's #ifdef/function-like-macro
;; preprocessor use (validated through `compose_surface_glsl').
;;
;; The single adaptation, identical in each shader and marked with a
;; `Romacs:' comment, is one added fragCoord flip line at the top of
;; mainImage: the v1 frame-post texture is top-left origin while
;; fragCoord is y-up (SHADER_SURFACES.md "Full-frame post shader"),
;; whereas Ghostty samples its terminal texture y-up.  Flipping
;; fragCoord once converts the whole shader to the frame texture's
;; coordinate system; everything downstream is untouched upstream code.

(defconst romacs-shaders--ghostty-starfield-glsl
  "// transparent background
const bool transparent = false;

// terminal contents luminance threshold to be considered background (0.0 to 1.0)
const float threshold = 0.15;

// divisions of grid
const float repeats = 30.;

// number of layers
const float layers = 21.;

// star colors
const vec3 white = vec3(1.0); // Set star color to pure white

float luminance(vec3 color) {
    return dot(color, vec3(0.2126, 0.7152, 0.0722));
}

float N21(vec2 p) {
    p = fract(p * vec2(233.34, 851.73));
    p += dot(p, p + 23.45);
    return fract(p.x * p.y);
}

vec2 N22(vec2 p) {
    float n = N21(p);
    return vec2(n, N21(p + n));
}

mat2 scale(vec2 _scale) {
    return mat2(_scale.x, 0.0,
        0.0, _scale.y);
}

// 2D Noise based on Morgan McGuire
float noise(in vec2 st) {
    vec2 i = floor(st);
    vec2 f = fract(st);

    // Four corners in 2D of a tile
    float a = N21(i);
    float b = N21(i + vec2(1.0, 0.0));
    float c = N21(i + vec2(0.0, 1.0));
    float d = N21(i + vec2(1.0, 1.0));

    // Smooth Interpolation
    vec2 u = f * f * (3.0 - 2.0 * f); // Cubic Hermite Curve

    // Mix 4 corners percentages
    return mix(a, b, u.x) +
        (c - a) * u.y * (1.0 - u.x) +
        (d - b) * u.x * u.y;
}

float perlin2(vec2 uv, int octaves, float pscale) {
    float col = 1.;
    float initScale = 4.;
    for (int l; l < octaves; l++) {
        float val = noise(uv * initScale);
        if (col <= 0.01) {
            col = 0.;
            break;
        }
        val -= 0.01;
        val *= 0.5;
        col *= val;
        initScale *= pscale;
    }
    return col;
}

vec3 stars(vec2 uv, float offset) {
    float timeScale = -(iTime + offset) / layers;
    float trans = fract(timeScale);
    float newRnd = floor(timeScale);
    vec3 col = vec3(0.);

    // Translate uv then scale for center
    uv -= vec2(0.5);
    uv = scale(vec2(trans)) * uv;
    uv += vec2(0.5);

    // Create square aspect ratio
    uv.x *= iResolution.x / iResolution.y;

    // Create boxes
    uv *= repeats;

    // Get position
    vec2 ipos = floor(uv);

    // Return uv as 0 to 1
    uv = fract(uv);

    // Calculate random xy and size
    vec2 rndXY = N22(newRnd + ipos * (offset + 1.)) * 0.9 + 0.05;
    float rndSize = N21(ipos) * 100. + 200.;

    vec2 j = (rndXY - uv) * rndSize;
    float sparkle = 1. / dot(j, j);

    // Set stars to be pure white
    col += white * sparkle;

    col *= smoothstep(1., 0.8, trans);
    return col; // Return pure white stars only
}

void mainImage(out vec4 fragColor, in vec2 fragCoord)
{
    // Romacs: v1 frame texture is top-left origin; flip fragCoord so uv
    // sampling matches Ghostty (which samples its terminal texture y-up).
    fragCoord = vec2(fragCoord.x, iResolution.y - fragCoord.y);
    // Normalized pixel coordinates (from 0 to 1)
    vec2 uv = fragCoord / iResolution.xy;

    vec3 col = vec3(0.);

    for (float i = 0.; i < layers; i++) {
        col += stars(uv, i);
    }

    // Sample the terminal screen texture including alpha channel
    vec4 terminalColor = texture(iChannel0, uv);

    if (transparent) {
        col += terminalColor.rgb;
    }

    // Make a mask that is 1.0 where the terminal content is not black
    float mask = 1 - step(threshold, luminance(terminalColor.rgb));

    vec3 blendedColor = mix(terminalColor.rgb, col, mask);

    // Apply terminal's alpha to control overall opacity
    fragColor = vec4(blendedColor, terminalColor.a);
}"
  "Ghostty `starfield.glsl', verbatim except the Romacs fragCoord flip.
Layered scrolling stars behind the editor content; text (luminance
above the threshold) stays opaque.  From
<https://github.com/hackr-sh/ghostty-shaders>.")

(defconst romacs-shaders--ghostty-gradient-glsl
  "// credits: https://github.com/unkn0wncode
void mainImage(out vec4 fragColor, in vec2 fragCoord)
{
    // Romacs: v1 frame texture is top-left origin; flip fragCoord so uv
    // sampling matches Ghostty (which samples its terminal texture y-up).
    fragCoord = vec2(fragCoord.x, iResolution.y - fragCoord.y);
    vec2 uv = fragCoord.xy / iResolution.xy;

    // Create seamless gradient animation
    float speed = 0.2;
    float gradientFactor = (uv.x + uv.y) / 2.0;

    // Use smoothstep and multiple sin waves for smoother transition
    float t = sin(iTime * speed) * 0.5 + 0.5;
    gradientFactor = smoothstep(0.0, 1.0, gradientFactor);

    // Create smooth circular animation
    float angle = iTime * speed;
    vec3 color1 = vec3(0.1, 0.1, 0.5);
    vec3 color2 = vec3(0.5, 0.1, 0.1);
    vec3 color3 = vec3(0.1, 0.5, 0.1);

    // Smooth interpolation between colors using multiple mix operations
    vec3 gradientStartColor = mix(
            mix(color1, color2, smoothstep(0.0, 1.0, sin(angle) * 0.5 + 0.5)),
            color3,
            smoothstep(0.0, 1.0, sin(angle + 2.0) * 0.5 + 0.5)
        );

    vec3 gradientEndColor = mix(
            mix(color2, color3, smoothstep(0.0, 1.0, sin(angle + 1.0) * 0.5 + 0.5)),
            color1,
            smoothstep(0.0, 1.0, sin(angle + 3.0) * 0.5 + 0.5)
        );

    vec3 gradientColor = mix(gradientStartColor, gradientEndColor, gradientFactor);

    vec4 terminalColor = texture(iChannel0, uv);
    float mask = 1.0 - step(0.5, dot(terminalColor.rgb, vec3(1.0)));
    vec3 blendedColor = mix(terminalColor.rgb, gradientColor, mask);

    fragColor = vec4(blendedColor, terminalColor.a);
}"
  "Ghostty `animated-gradient-shader.glsl' (credits unkn0wncode),
verbatim except the Romacs fragCoord flip.  A slow diagonal color
gradient behind the editor content.  From
<https://github.com/hackr-sh/ghostty-shaders>.")

(defconst romacs-shaders--ghostty-crt-glsl
  "// source: https://gist.github.com/qwerasd205/c3da6c610c8ffe17d6d2d3cc7068f17f
// credits: https://github.com/qwerasd205
//==============================================================
//
//    [CRTS] PUBLIC DOMAIN CRT-STYLED SCALAR by Timothy Lottes
//
//    [+] Adapted with alterations for use in Ghostty by Qwerasd.
//    For more information on changes, see comment below license.
//
//==============================================================
//
//      LICENSE = UNLICENSE (aka PUBLIC DOMAIN)
//
//--------------------------------------------------------------
// This is free and unencumbered software released into the
// public domain.
//--------------------------------------------------------------
// Anyone is free to copy, modify, publish, use, compile, sell,
// or distribute this software, either in source code form or as
// a compiled binary, for any purpose, commercial or
// non-commercial, and by any means.
//--------------------------------------------------------------
// In jurisdictions that recognize copyright laws, the author or
// authors of this software dedicate any and all copyright
// interest in the software to the public domain. We make this
// dedication for the benefit of the public at large and to the
// detriment of our heirs and successors. We intend this
// dedication to be an overt act of relinquishment in perpetuity
// of all present and future rights to this software under
// copyright law.
//--------------------------------------------------------------
// THE SOFTWARE IS PROVIDED \"AS IS\", WITHOUT WARRANTY OF ANY
// KIND, EXPRESS OR IMPLIED, INCLUDING BUT NOT LIMITED TO THE
// WARRANTIES OF MERCHANTABILITY, FITNESS FOR A PARTICULAR
// PURPOSE AND NONINFRINGEMENT. IN NO EVENT SHALL THE AUTHORS BE
// LIABLE FOR ANY CLAIM, DAMAGES OR OTHER LIABILITY, WHETHER IN
// AN ACTION OF CONTRACT, TORT OR OTHERWISE, ARISING FROM, OUT
// OF OR IN CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER
// DEALINGS IN THE SOFTWARE.
//--------------------------------------------------------------
// For more information, please refer to
// <http://unlicense.org/>
//==============================================================

// This shader is a modified version of the excellent
// FixingPixelArtFast by Timothy Lottes on Shadertoy.
//
// The original shader can be found at:
// https://www.shadertoy.com/view/MtSfRK
//
// Modifications have been made to reduce the verbosity,
// and many of the comments have been removed / reworded.
// Additionally, the license has been moved to the top of
// the file, and can be read above. I (Qwerasd) choose to
// release the modified version under the same license.

// The appearance of this shader can be altered
// by adjusting the parameters defined below.

// \"Scanlines\" per real screen pixel.
// e.g. SCALE 0.5 means each scanline is 2 pixels.
// Recommended values:
//  o High DPI displays: 0.33333333
//  - Low DPI displays:  0.66666666
#define SCALE 0.33333333

// \"Tube\" warp
#define CRTS_WARP 1

// Darkness of vignette in corners after warping
//  0.0 = completely black
//  1.0 = no vignetting
#define MIN_VIN 0.5

// Try different masks
// #define CRTS_MASK_GRILLE 1
// #define CRTS_MASK_GRILLE_LITE 1
// #define CRTS_MASK_NONE 1
#define CRTS_MASK_SHADOW 1

// Scanline thinness
//  0.50 = fused scanlines
//  0.70 = recommended default
//  1.00 = thinner scanlines (too thin)
#define INPUT_THIN 0.75

// Horizonal scan blur
//  -3.0 = pixely
//  -2.5 = default
//  -2.0 = smooth
//  -1.0 = too blurry
#define INPUT_BLUR -2.75

// Shadow mask effect, ranges from,
//  0.25 = large amount of mask (not recommended, too dark)
//  0.50 = recommended default
//  1.00 = no shadow mask
#define INPUT_MASK 0.65

float FromSrgb1(float c) {
  return (c <= 0.04045) ? c * (1.0 / 12.92) :
  pow(c * (1.0 / 1.055) + (0.055 / 1.055), 2.4);
}
vec3 FromSrgb(vec3 c) {
  return vec3(
    FromSrgb1(c.r), FromSrgb1(c.g), FromSrgb1(c.b));
}

vec3 CrtsFetch(vec2 uv) {
  return FromSrgb(texture(iChannel0, uv.xy).rgb);
}

#define CrtsRcpF1(x) (1.0/(x))
#define CrtsSatF1(x) clamp((x),0.0,1.0)

float CrtsMax3F1(float a, float b, float c) {
  return max(a, max(b, c));
}

vec2 CrtsTone(
  float thin,
  float mask) {
  #ifdef CRTS_MASK_NONE
  mask = 1.0;
  #endif

  #ifdef CRTS_MASK_GRILLE_LITE
  // Normal R mask is {1.0,mask,mask}
  // LITE   R mask is {mask,1.0,1.0}
  mask = 0.5 + mask * 0.5;
  #endif

  vec2 ret;
  float midOut = 0.18 / ((1.5 - thin) * (0.5 * mask + 0.5));
  float pMidIn = 0.18;
  ret.x = ((-pMidIn) + midOut) / ((1.0 - pMidIn) * midOut);
  ret.y = ((-pMidIn) * midOut + pMidIn) / (midOut * (-pMidIn) + midOut);

  return ret;
}

vec3 CrtsMask(vec2 pos, float dark) {
  #ifdef CRTS_MASK_GRILLE
  vec3 m = vec3(dark, dark, dark);
  float x = fract(pos.x * (1.0 / 3.0));
  if (x < (1.0 / 3.0)) m.r = 1.0;
  else if (x < (2.0 / 3.0)) m.g = 1.0;
  else m.b = 1.0;
  return m;
  #endif

  #ifdef CRTS_MASK_GRILLE_LITE
  vec3 m = vec3(1.0, 1.0, 1.0);
  float x = fract(pos.x * (1.0 / 3.0));
  if (x < (1.0 / 3.0)) m.r = dark;
  else if (x < (2.0 / 3.0)) m.g = dark;
  else m.b = dark;
  return m;
  #endif

  #ifdef CRTS_MASK_NONE
  return vec3(1.0, 1.0, 1.0);
  #endif

  #ifdef CRTS_MASK_SHADOW
  pos.x += pos.y * 3.0;
  vec3 m = vec3(dark, dark, dark);
  float x = fract(pos.x * (1.0 / 6.0));
  if (x < (1.0 / 3.0)) m.r = 1.0;
  else if (x < (2.0 / 3.0)) m.g = 1.0;
  else m.b = 1.0;
  return m;
  #endif
}

vec3 CrtsFilter(
  vec2 ipos,
  vec2 inputSizeDivOutputSize,
  vec2 halfInputSize,
  vec2 rcpInputSize,
  vec2 rcpOutputSize,
  vec2 twoDivOutputSize,
  float inputHeight,
  vec2 warp,
  float thin,
  float blur,
  float mask,
  vec2 tone
) {
  // Optional apply warp
  vec2 pos;
  #ifdef CRTS_WARP
  // Convert to {-1 to 1} range
  pos = ipos * twoDivOutputSize - vec2(1.0, 1.0);

  // Distort pushes image outside {-1 to 1} range
  pos *= vec2(
      1.0 + (pos.y * pos.y) * warp.x,
      1.0 + (pos.x * pos.x) * warp.y);

  // TODO: Vignette needs optimization
  float vin = 1.0 - (
      (1.0 - CrtsSatF1(pos.x * pos.x)) * (1.0 - CrtsSatF1(pos.y * pos.y)));
  vin = CrtsSatF1((-vin) * inputHeight + inputHeight);

  // Leave in {0 to inputSize}
  pos = pos * halfInputSize + halfInputSize;
  #else
  pos = ipos * inputSizeDivOutputSize;
  #endif

  // Snap to center of first scanline
  float y0 = floor(pos.y - 0.5) + 0.5;
  // Snap to center of one of four pixels
  float x0 = floor(pos.x - 1.5) + 0.5;

  // Inital UV position
  vec2 p = vec2(x0 * rcpInputSize.x, y0 * rcpInputSize.y);
  // Fetch 4 nearest texels from 2 nearest scanlines
  vec3 colA0 = CrtsFetch(p);
  p.x += rcpInputSize.x;
  vec3 colA1 = CrtsFetch(p);
  p.x += rcpInputSize.x;
  vec3 colA2 = CrtsFetch(p);
  p.x += rcpInputSize.x;
  vec3 colA3 = CrtsFetch(p);
  p.y += rcpInputSize.y;
  vec3 colB3 = CrtsFetch(p);
  p.x -= rcpInputSize.x;
  vec3 colB2 = CrtsFetch(p);
  p.x -= rcpInputSize.x;
  vec3 colB1 = CrtsFetch(p);
  p.x -= rcpInputSize.x;
  vec3 colB0 = CrtsFetch(p);

  // Vertical filter
  // Scanline intensity is using sine wave
  // Easy filter window and integral used later in exposure
  float off = pos.y - y0;
  float pi2 = 6.28318530717958;
  float hlf = 0.5;
  float scanA = cos(min(0.5, off * thin) * pi2) * hlf + hlf;
  float scanB = cos(min(0.5, (-off) * thin + thin) * pi2) * hlf + hlf;

  // Horizontal kernel is simple gaussian filter
  float off0 = pos.x - x0;
  float off1 = off0 - 1.0;
  float off2 = off0 - 2.0;
  float off3 = off0 - 3.0;
  float pix0 = exp2(blur * off0 * off0);
  float pix1 = exp2(blur * off1 * off1);
  float pix2 = exp2(blur * off2 * off2);
  float pix3 = exp2(blur * off3 * off3);
  float pixT = CrtsRcpF1(pix0 + pix1 + pix2 + pix3);

  #ifdef CRTS_WARP
  // Get rid of wrong pixels on edge
  pixT *= max(MIN_VIN, vin);
  #endif

  scanA *= pixT;
  scanB *= pixT;

  // Apply horizontal and vertical filters
  vec3 color =
    (colA0 * pix0 + colA1 * pix1 + colA2 * pix2 + colA3 * pix3) * scanA +
      (colB0 * pix0 + colB1 * pix1 + colB2 * pix2 + colB3 * pix3) * scanB;

  // Apply phosphor mask
  color *= CrtsMask(ipos, mask);

  // Tonal control, start by protecting from /0
  float peak = max(1.0 / (256.0 * 65536.0),
      CrtsMax3F1(color.r, color.g, color.b));
  // Compute the ratios of {R,G,B}
  vec3 ratio = color * CrtsRcpF1(peak);
  // Apply tonal curve to peak value
  peak = peak * CrtsRcpF1(peak * tone.x + tone.y);
  // Reconstruct color
  return ratio * peak;
}

float ToSrgb1(float c) {
  return (c < 0.0031308 ? c * 12.92 : 1.055 * pow(c, 0.41666) - 0.055);
}
vec3 ToSrgb(vec3 c) {
  return vec3(
    ToSrgb1(c.r), ToSrgb1(c.g), ToSrgb1(c.b));
}

void mainImage(out vec4 fragColor, in vec2 fragCoord) {
    // Romacs: v1 frame texture is top-left origin; flip fragCoord so uv
    // sampling matches Ghostty (which samples its terminal texture y-up).
    fragCoord = vec2(fragCoord.x, iResolution.y - fragCoord.y);
  float aspect = iResolution.x / iResolution.y;
  fragColor.rgb = CrtsFilter(
      fragCoord.xy,
      vec2(1.0),
      iResolution.xy * SCALE * 0.5,
      1.0 / (iResolution.xy * SCALE),
      1.0 / iResolution.xy,
      2.0 / iResolution.xy,
      iResolution.y,
      vec2(1.0 / (50.0 * aspect), 1.0 / 50.0),
      INPUT_THIN,
      INPUT_BLUR,
      INPUT_MASK,
      CrtsTone(INPUT_THIN, INPUT_MASK)
    );

  // Linear to SRGB for output.
  fragColor = vec4(ToSrgb(fragColor.rgb), 1.0);
}"
  "Ghostty `crt.glsl': Timothy Lottes' public-domain CRT filter
\(Shadertoy MtSfRK), adapted for Ghostty by Qwerasd (Unlicense, header
kept), verbatim except the Romacs fragCoord flip.  Warped tube,
scanlines, shadow mask, vignette.  From
<https://github.com/hackr-sh/ghostty-shaders>.")

;;;###autoload
(defun romacs-shaders-ghostty-starfield ()
  "Apply the Ghostty starfield shader over the whole editor."
  (interactive)
  (romacs-frame-shader romacs-shaders--ghostty-starfield-glsl 'glsl)
  (message "Ghostty starfield on — M-x romacs-shaders-off to remove"))

;;;###autoload
(defun romacs-shaders-ghostty-gradient ()
  "Apply the Ghostty animated-gradient shader over the whole editor."
  (interactive)
  (romacs-frame-shader romacs-shaders--ghostty-gradient-glsl 'glsl)
  (message "Ghostty gradient on — M-x romacs-shaders-off to remove"))

;;;###autoload
(defun romacs-shaders-ghostty-crt ()
  "Apply the Ghostty (Lottes) CRT shader over the whole editor."
  (interactive)
  (romacs-frame-shader romacs-shaders--ghostty-crt-glsl 'glsl)
  (message "Ghostty CRT on — M-x romacs-shaders-off to remove"))

(provide 'romacs-shaders)
;;; romacs-shaders.el ends here
