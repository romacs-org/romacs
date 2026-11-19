;;; tibetan.el --- support for Tibetan language -*- coding: utf-8; lexical-binding: t; -*-

;; Copyright (C) 1997, 2001-2026 Free Software Foundation, Inc.
;; Copyright (C) 1997, 1998, 1999, 2000, 2001, 2002, 2003, 2004, 2005,
;;   2006, 2007, 2008, 2009, 2010, 2011
;;   National Institute of Advanced Industrial Science and Technology (AIST)
;;   Registration Number H14PRO021
;; Copyright (C) 2003
;;   National Institute of Advanced Industrial Science and Technology (AIST)
;;   Registration Number H13PRO009

;; Author: Toru TOMABECHI <Toru.Tomabechi@orient.unil.ch>
;; Created: Feb. 17. 1997
;; Keywords: multilingual, Tibetan, i18n

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

;;; History:

;; 1997.03.13 Modification for special signs and punctuation.

;;; Commentary:

;; Note: Romacs keeps Tibetan text in Unicode.  GNU's precomposed
;; glyph codes of the Emacs `tibetan' charset, above #x10FFFF, are
;; written here as the Unicode stacks they decompose to, and its
;; invisible vowel sign "a" as nothing, the inherent vowel
;; (ledger/divergences/VM2-TIBETAN-UNICODE.md).

;;; Code:

(define-coding-system 'tibetan-iso-8bit
  "8-bit encoding for ASCII (MSB=0) and TIBETAN (MSB=1)."
  :coding-type 'iso-2022
  :mnemonic ?Q
  :designation [ascii tibetan nil nil]
  :charset-list '(ascii tibetan))

(define-coding-system-alias 'tibetan 'tibetan-iso-8bit)

(set-language-info-alist
 "Tibetan" '((charset tibetan tibetan-1-column)
	     (coding-system tibetan-iso-8bit)
	     (coding-priority iso-2022-7bit tibetan-iso-8bit)
	     (input-method . "tibetan-wylie")
	     (features tibet-util)
	     (documentation . t)
	     (sample-text . "Tibetan (བོད་སྐད་) ༄༅༅༎བཀྲ་ཤིས་བདེ་ལེགས༎ཨོཾ་མ་ཎི་པདྨེ་ཧའུྂ༎")))

;; `འ' is included in the pattern for subjoined consonants because we
;; treat it specially in tibetan-add-components.
;; modified by Tomabechi 1999/12/10
;; modified by Tomabechi 2000/06/08
;;          To allow infinite addition of vowels/modifiers
;;          as specified in Unicode v.3
;; འ is removed from the class of subjoined. Tomabechi 2000/06/08
;; (for Unicode support)
(defconst tibetan-composable-pattern
  "[ཀ-ཀྵཪ][ྐ-ྐྵྺྻྼ]*[཰ཱི-྄ཱཽྀྀ]*[ཾྂྃ྆-ྋ༙༵༷]*"
  "Regexp matching a composable sequence of Tibetan characters.")

;;;
;;; Definitions of conversion data.
;;;

(eval-and-compile

;;; alists for tibetan char <-> transcription conversion
;;; longer transcription should come first
(defconst tibetan-consonant-transcription-alist
  '(("tsh" . "ཚ")
    ("dzh" . "ཛྷ")
    ("kSH" . "ཀྵ")
    ("kh" . "ཁ")
    ("gh" . "གྷ")
    ("ng" . "ང")
    ("ch" . "ཆ")
    ("ny" . "ཉ")
    ("TH" . "ཋ")
    ("DH" . "ཌྷ")
    ("th" . "ཐ")
    ("dh" . "དྷ")
    ("ph" . "ཕ")
    ("bh" . "བྷ")
    ("ts" . "ཙ")
    ("dz" . "ཛ")
    ("zh" . "ཞ")
    ("sh" . "ཤ")
    ("SH" . "ཥ")
    ("k" . "ཀ")
    ("g" . "ག")
    ("c" . "ཅ")
    ("j" . "ཇ")
    ("T" . "ཊ")
    ("D" . "ཌ")
    ("N" . "ཎ")
    ("t" . "ཏ")
    ("d" . "ད")
    ("n" . "ན")
    ("p" . "པ")
    ("b" . "བ")
    ("m" . "མ")
    ("w" . "ཝ")
    ("z" . "ཟ")
    ("'" . "འ")
    ("y" . "ཡ")
    ("r" . "ར")
    ("l" . "ལ")
    ("s" . "ས")
    ("h" . "ཧ")
    ("H" . "ཧ")
    ("A" . "ཨ")
    ;; Added by Tomabechi 1999/12/10
    ("R" . "ཪ") ;; fixed form RA
    ))


(defconst tibetan-vowel-transcription-alist
  '(
    ;; Composite Vowels
    ;; Added by Tomabechi 2000/06/08
    ("frr" . "ཷ")
    ("fll" . "ཹ")
    ("fa" . "ཱ")
    ("fi" . "ཱི")
    ("fu" . "ཱུ")
    ("fr" . "ྲྀ")
    ("fl" . "ླྀ")
    ("fI" . "ཱྀ")
    ;; Normal Vowels
    ("ai" . "ཻ")
    ("au" . "ཽ")
    ("ee" . "ཻ")
    ("oo" . "ཽ")
    ("a" . "")			; the inherent vowel, no character
    ("i" . "ི")
    ("u" . "ུ")
    ("e" . "ེ")
    ("o" . "ོ")
    ("E" . "ཻ")
    ("O" . "ཽ")
    ("I" . "ྀ")
    ("," . "྄")			; idem.
    ))

(defconst tibetan-modifier-transcription-alist
  '(("M" . "ཾ")
    ("~" . "ྂ")
    ("`" . "ྃ")
    ("x" . "ྈ")
    ("X" . "ྉ")
    ("v" . "྆")
    ("V" . "྇")
    ("q" . "ྊ")
    ("Q" . "ྋ")
    ("_/" . "༙")
    ("_o" . "༷")
    ("_O" . "༵")))

(defconst tibetan-precomposed-transcription-alist
  '(("phyw" . "ཕྱྭ")
    ("tshw" . "ཚྭ")
    ("rtsw" . "རྩྭ")
    ("khw" . "ཁྭ")
    ("nyw" . "ཉྭ")
    ("tsw" . "ཙྭ")
    ("zhw" . "ཞྭ")
    ("shw" . "ཤྭ")
    ("khy" . "ཁྱ")
    ("phy" . "ཕྱ")
    ("khr" . "ཁྲ")
    ("thr" . "ཐྲ")
    ("phr" . "ཕྲ")
    ("shr" . "ཤྲ")
    ("dzr" . "ཛྲ")
    ("grw" . "གྲྭ")
    ("rng" . "རྔ")
    ("rny" . "རྙ")
    ("rts" . "རྩ")
    ("rdz" . "རྫ")
    ("rgw" . "རྒྭ")
    ("rky" . "རྐྱ")
    ("rgy" . "རྒྱ")
    ("rmy" . "རྨྱ")
    ("lng" . "ལྔ")
    ("sng" . "སྔ")
    ("sny" . "སྙ")
    ("sts" . "སྩ")
    ("sky" . "སྐྱ")
    ("sgy" . "སྒྱ")
    ("spy" . "སྤྱ")
    ("sby" . "སྦྱ")
    ("smy" . "སྨྱ")
    ("skr" . "སྐྲ")
    ("sgr" . "སྒྲ")
    ("snr" . "སྣྲ")
    ("spr" . "སྤྲ")
    ("sbr" . "སྦྲ")
    ("smr" . "སྨྲ")
    ("kw" . "ཀྭ")
    ("gw" . "གྭ")
    ("cw" . "ཅྭ")
    ("tw" . "ཏྭ")
    ("dw" . "དྭ")
    ("zw" . "ཟྭ")
    ("rw" . "རྭ")
    ("lw" . "ལྭ")
    ("sw" . "སྭ")
    ("hw" . "ཧྭ")
    ("ky" . "ཀྱ")
    ("gy" . "གྱ")
    ("py" . "པྱ")
    ("by" . "བྱ")
    ("my" . "མྱ")
    ("kr" . "ཀྲ")
    ("gr" . "གྲ")
    ("tr" . "ཏྲ")
    ("dr" . "དྲ")
    ("pr" . "པྲ")
    ("brk" . "བརྐ")
    ("brg" . "བརྒ")
    ("brng" . "བརྔ")
    ("brj" . "བརྗ")
    ("brny" . "བརྙ")
    ("brt" .  "བརྟ")
    ("brd" . "བརྡ")
    ("brn" . "བརྣ")
    ("brts" . "བརྩ")
    ("brdz" . "བརྫ")
    ("brl" . "བརླ")
    ("br" . "བྲ")
    ("mr" . "མྲ")
    ("sr" . "སྲ")
    ("hr" . "ཧྲ")
    ("jr" . "ཇྲ")
    ("kl" . "ཀླ")
    ("gl" . "གླ")
    ("blt" . "བལྟ")
    ("bld" . "བལྡ")
    ("bl" . "བླ")
    ("zl" . "ཟླ")
    ("rl" . "རླ")
    ("sl" . "སླ")
    ("rk" . "རྐ")
    ("rg" . "རྒ")
    ("rj" . "རྗ")
    ("rt" . "རྟ")
    ("rd" . "རྡ")
    ("rn" . "རྣ")
    ("rb" . "རྦ")
    ("rm" . "རྨ")
    ("lk" . "ལྐ")
    ("lg" . "ལྒ")
    ("lc" . "ལྕ")
    ("lj" . "ལྗ")
    ("lt" . "ལྟ")
    ("ld" . "ལྡ")
    ("ln" . "ལྣ")			; dummy \x2121
    ("lp" . "ལྤ")
    ("lb" . "ལྦ")
    ("lh" . "ལྷ")
    ("sk" . "སྐ")
    ("sg" . "སྒ")
    ("st" . "སྟ")
    ("sd" . "སྡ")
    ("sn" . "སྣ")
    ("sp" . "སྤ")
    ("sb" . "སྦ")
    ("sm" . "སྨ"))
  )


(defconst tibetan-subjoined-transcription-alist
 (eval-when-compile
  (sort
   (copy-sequence
	'(("+k"  . "ྐ")
	  ("+kh" . "ྑ")
	  ("+g"  . "ྒ")
	  ("+gh" . "ྒྷ")
	  ("+ng" . "ྔ")
	  ("+c"  . "ྕ")
	  ("+ch" . "ྖ")
	  ("+j"  . "ྗ")
	  ("+ny"  . "ྙ")
	  ("+T"  . "ྚ")
	  ("+TH" . "ྛ")
	  ("+D"  . "ྜ")
	  ("+DH" . "ྜྷ")
	  ("+N"  . "ྞ")
	  ("+t"  . "ྟ")
	  ("+th" . "ྠ")
	  ("+d"  . "ྡ")
	  ("+dh" . "ྡྷ")
	  ("+n"  . "ྣ")
	  ("+p"  . "ྤ")
	  ("+ph" . "ྥ")
	  ("+b"  . "ྦ")
	  ("+bh" . "ྦྷ")
	  ("+m"  . "ྨ")
	  ("+ts" . "ྩ")
	  ("+tsh" . "ྪ")
	  ("+dz" . "ྫ")
	  ("+dzh" . "ྫྷ")
	  ("+w"  . "ྭ")
	  ("+zh" . "ྮ")
	  ("+z"  . "ྯ")
	  ("+'"  . "ྰ")
	  ("+y"  . "ྱ")
	  ("+r"  . "ྲ")
	  ("+l"  . "ླ")
	  ("+sh" . "ྴ")
	  ("+SH" . "ྵ")
	  ("+s"  . "ྶ")
	  ("+h"  . "ྷ")
	  ("+A"  . "ྸ")
	  ("+kSH" . "ྐྵ")
	  ;; Added by Tomabechi 1999/12/10
	  ("+W" . "ྺ") ;; fixed form subscribed WA
	  ("+Y" . "ྻ") ;; fixed form subscribed YA
	  ("+R" . "ྼ") ;; fixed form subscribed RA
	  ))
   (lambda (x y) (> (length (car x)) (length (car y)))))))

;;;
;;; alist for Tibetan base consonant <-> subjoined consonant conversion.
;;;
(defconst tibetan-base-to-subjoined-alist
  '(("ཀ" . "ྐ")
    ("ཁ" . "ྑ")
    ("ག" . "ྒ")
    ("གྷ" . "ྒྷ")
    ("ང" . "ྔ")
    ("ཅ" . "ྕ")
    ("ཆ" . "ྖ")
    ("ཇ" . "ྗ")
    ("ཉ" . "ྙ")
    ("ཊ" . "ྚ")
    ("ཋ" . "ྛ")
    ("ཌ" . "ྜ")
    ("ཌྷ" . "ྜྷ")
    ("ཎ" . "ྞ")
    ("ཏ" . "ྟ")
    ("ཐ" . "ྠ")
    ("ད" . "ྡ")
    ("དྷ" . "ྡྷ")
    ("ན" . "ྣ")
    ("པ" . "ྤ")
    ("ཕ" . "ྥ")
    ("བ" . "ྦ")
    ("བྷ" . "ྦྷ")
    ("མ" . "ྨ")
    ("ཙ" . "ྩ")
    ("ཚ" . "ྪ")
    ("ཛ" . "ྫ")
    ("ཛྷ" . "ྫྷ")
    ("ཝ" . "ྭ")
    ("ཞ" . "ྮ")
    ("ཟ" . "ྯ")
    ("འ" . "ྰ")
    ("ཡ" . "ྱ")
    ("ར" . "ྲ")
    ("ལ" . "ླ")
    ("ཤ" . "ྴ")
    ("ཥ" . "ྵ")
    ("ས" . "ྶ")
    ("ཧ" . "ྷ")
    ("ཨ" . "ྸ")
    ("ཀྵ" . "ྐྵ")
    ;; Added by Tomabechi 1999/12/10
    ("ཪ" . "ྼ") ;; Fixed form RA (224B->234D)
    ))

;;; alist for Tibetan composite vowels (long i, vocalic r, etc.)
;;; New variable. created by Tomabechi 2000/06/08
(defconst tibetan-composite-vowel-alist
  '(;; LONG A
    ;; ("ཱ" . ((bc . tc) ?ཱ))
    ;; LONG I
    ("ཱི" . (?ཱ (tc . bc) ?ི))
    ;; LONG U
    ("ཱུ" . (?ཱ (bc . tc) ?ུ))
    ;; VOCALIC R
    ("ྲྀ" . (?ྲ (tc . bc) ?ྀ))
    ;; LONG VOCALIC R
    ("ཷ" . (?ྲ (bc . tc) ?ཱ (tc . bc) ?ྀ))
    ;; VOCALIC L
    ("ླྀ" . (?ླ (tc . bc) ?ྀ))
    ;;་LONG VOCALIC L
    ("ཹ" . (?ླ (bc . tc) ?ཱ (tc . bc) ?ྀ))
    ;; LONG REVERSE I
    ("ཱྀ" . (?ཱ (tc . bc) ?ྀ))
    ))



)   ; eval-and-compile

(defconst tibetan-regexp
  (eval-when-compile
    (regexp-opt (mapcar #'car
                        (append tibetan-precomposed-transcription-alist
		                tibetan-consonant-transcription-alist
		                tibetan-vowel-transcription-alist
		                tibetan-modifier-transcription-alist
		                tibetan-subjoined-transcription-alist))))
  "Regexp matching a Tibetan transcription of a composable Tibetan sequence.
The result of matching is to be used for indexing alists at conversion
from a roman transcription to the corresponding Tibetan character.")

(defvar tibetan-precomposed-regexp
  (eval-when-compile
    (concat "^"
            (regexp-opt (mapcar #'car tibetan-precomposed-transcription-alist)
                        t)))
  "Regexp string to match a romanized Tibetan complex consonant.
The result of matching is to be used for indexing alists when the input key
from an input method is converted to the corresponding precomposed glyph.")

(defvar tibetan-precomposition-rule-regexp
  (eval-when-compile
    (regexp-opt (mapcar #'cdr tibetan-precomposed-transcription-alist) t))
  "Regexp string to match a sequence of Tibetan consonantic components.
That is, one base consonant and one or more subjoined consonants, as
`tibetan-precomposed-transcription-alist' transcribes it.")

(defvar tibetan-decomposed nil)
(defvar tibetan-decomposed-temp nil)

;; For automatic composition.
(set-char-table-range
 composition-function-table '(#xF00 . #xFD1)
 (list (vector tibetan-composable-pattern 0 #'font-shape-gstring)))

(provide 'tibetan)

;;; tibetan.el ends here
