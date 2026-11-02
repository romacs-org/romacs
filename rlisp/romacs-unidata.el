;;; romacs-unidata.el --- Unicode property table functions in source -*- lexical-binding: t; -*-
;; Copyright (C) 2008-2026 Free Software Foundation, Inc.
;; Copyright (C) 2005, 2006, 2007, 2008, 2009, 2010, 2011
;;   National Institute of Advanced Industrial Science and Technology (AIST)
;;   Registration Number H13PRO009
;; SPDX-License-Identifier: GPL-3.0-or-later
;; Maintainer: Thanos Apollo <public@thanosapollo.org>

;;; Commentary:
;; Adapted from GNU admin/unidata/unidata-gen.el at the pin.  GNU's
;; generated uni-*.el tables carry these functions as byte code in their
;; extra slots.  Romacs runs Lisp from source, so its copies of those tables
;; name the functions below instead (DIVERGENCES.md VM2-UNIDATA1).  Each
;; body is unidata-gen.el's function of the same suffix; the defsubst
;; `unidata-decode-word' is the plain function `romacs-unidata--decode-word'.

;;; Code:

(defun romacs-unidata--decode-word (code word-table)
  "Return the word of WORD-TABLE that CODE stands for, or nil."
  (setq code (- code 3))
  (if (< code (length word-table))
      (aref word-table code)))

(defun romacs-unidata-get-name (char val table)
  "Return the name of CHAR; VAL is (aref TABLE CHAR).
A word-list block is expanded into TABLE on first use."
  (cond
   ((stringp val)
    (if (> (aref val 0) 0)
	val
      (let* ((first-char (ash (ash char -7) 7))
	     (word-table (aref (char-table-extra-slot table 4) 0))
	     (i 1)
	     (len (length val))
	     (vec (make-vector 128 nil))
	     (idx 0)
	     (case-fold-search nil)
	     c word-list tail-list last-list diff-head)
	(while (< i len)
	  (setq c (aref val i))
	  (if (< c 3)
	      (progn
		(if (or word-list tail-list)
		    (aset vec idx
			  (setq last-list (nconc word-list tail-list))))
		(setq i (1+ i) idx (1+ idx)
		      word-list nil tail-list nil)
		(if (> c 0)
		    (let ((l last-list))
		      (if (= c 1)
			  (setq diff-head
				(prog1 (aref val i) (setq i (1+ i)))))
		      (setq tail-list (nthcdr (% diff-head 16) last-list))
		      (dotimes (_ (/ diff-head 16))
			(setq word-list (nconc word-list (list (car l)))
			      l (cdr l))))))
	    (setq word-list
		  (nconc word-list
			 (list (symbol-name
				(romacs-unidata--decode-word c word-table))))
		  i (1+ i))))
	(if (or word-list tail-list)
	    (aset vec idx (nconc word-list tail-list)))
	(setq val nil)
	(dotimes (i 128)
	  (setq c (+ first-char i))
	  (let ((name (aref vec i)))
	    (if name
		(let ((tail (cdr (setq name (copy-sequence name))))
		      elt)
		  (while tail
		    (setq elt (car tail))
		    (or (string= elt "-")
			(progn
			  (setcdr tail (cons elt (cdr tail)))
			  (setcar tail " ")))
		    (setq tail (cddr tail)))
		  (setq name (apply #'concat name))))
	    (aset table c name)
	    (if (= c char)
		(setq val name))))
	val)))

   ((and (integerp val) (> val 0))
    (let* ((symbol-table (aref (char-table-extra-slot table 4) 1))
	   (sym (aref symbol-table (1- val))))
      (cond ((eq sym 'HANGUL\ SYLLABLE)
	     (let ((jamo-name-table (aref (char-table-extra-slot table 4) 2)))
	       ;; SIndex = S - SBase
	       (setq char (- char #xAC00))
	       (let ( ;; LIndex = SIndex / NCount
		     (L (/ char 588))
		     ;; VIndex = (SIndex % NCount) * TCount
		     (V (/ (% char 588) 28))
		     ;; TIndex = SIndex % TCount
		     (T (% char 28)))
		 (format "HANGUL SYLLABLE %s%s%s"
			 ;; U+110B is nil in this table.
			 (or (aref (aref jamo-name-table 0) L) "")
			 (aref (aref jamo-name-table 1) V)
			 (if (= T 0) ""
			   (aref (aref jamo-name-table 2) (1- T)))))))
	    ((eq sym 'CJK\ IDEOGRAPH)
	     (format "%s-%04X" sym char))
	    ((eq sym 'TANGUT\ IDEOGRAPH)
	     (format "%s-%04X" sym char))
	    ((eq sym 'CJK\ COMPATIBILITY\ IDEOGRAPH)
	     (format "%s-%04X" sym char))
	    ((eq sym 'HIGH\ SURROGATE)
	     (format "%s-%04X" sym char))
	    ((eq sym 'LOW\ SURROGATE)
	     (format "%s-%04X" sym char))
	    ((eq sym 'VARIATION\ SELECTOR)
	     (format "%s-%d" sym (+ (- char #xe0100) 17))))))))

(defun romacs-unidata-put-name (char val table)
  "Store VAL as the name of CHAR in TABLE."
  (let ((current-val (aref table char)))
    (if (and (stringp current-val) (= (aref current-val 0) 0))
	(funcall (char-table-extra-slot table 1) char current-val table))
    (aset table char val)))

(defun romacs-unidata-get-decomposition (char val table)
  "Return the decomposition of CHAR; VAL is (aref TABLE CHAR)."
  (cond
   ((not val)
    (list char))

   ((consp val)
    val)

   ((stringp val)
    (if (> (aref val 0) 0)
	val
      (let* ((first-char (ash (ash char -7) 7))
	     (word-table (char-table-extra-slot table 4))
	     (i 1)
	     (len (length val))
	     (vec (make-vector 128 nil))
	     (idx 0)
	     (case-fold-search nil)
	     c word-list tail-list last-list diff-head)
	(while (< i len)
	  (setq c (aref val i))
	  (if (< c 3)
	      (progn
		(if (or word-list tail-list)
		    (aset vec idx
			  (setq last-list (nconc word-list tail-list))))
		(setq i (1+ i) idx (1+ idx)
		      word-list nil tail-list nil)
		(if (> c 0)
		    (let ((l last-list))
		      (if (= c 1)
			  (setq diff-head
				(prog1 (aref val i) (setq i (1+ i)))))
		      (setq tail-list (nthcdr (% diff-head 16) last-list))
		      (dotimes (_ (/ diff-head 16))
			(setq word-list (nconc word-list (list (car l)))
			      l (cdr l))))))
	    (setq word-list
		  (nconc word-list
			 (list (or (romacs-unidata--decode-word c word-table) c)))
		  i (1+ i))))
	(if (or word-list tail-list)
	    (aset vec idx (nconc word-list tail-list)))
	(dotimes (i 128)
	  (aset table (+ first-char i) (aref vec i)))
	(setq val (aref vec (- char first-char)))
	(or val (list char)))))

   ;; Hangul syllable
   ((and (eq val 0) (>= char #xAC00) (<= char #xD7A3))
    ;; SIndex = S (char) - SBase (#xAC00)
    (setq char (- char #xAC00))
    (let (;; L = LBase + SIndex / NCount
	  (L (+ #x1100 (/ char 588)))
	  ;; V = VBase + (SIndex % NCount) * TCount
	  (V (+ #x1161 (/ (% char 588) 28)))
	  ;; LV = SBase + (SIndex / TCount) * TCount
	  (LV (+ #xAC00 (* (/ char 28) 28)))
	  ;; T = TBase + SIndex % TCount
	  (T (+ #x11A7 (% char 28))))
      (if (= T #x11A7)
	  (list L V)
	(list LV T))))

   ))

(defun romacs-unidata-put-decomposition (char val table)
  "Store VAL as the decomposition of CHAR in TABLE."
  (let ((current-val (aref table char)))
    (if (and (stringp current-val) (= (aref current-val 0) 0))
	(funcall (char-table-extra-slot table 1) char current-val table))
    (aset table char val)))

(defun romacs-unidata-describe-general-category (val)
  "Describe general category VAL."
  (cdr (assq val
	     '((nil . "Unknown")
	       (Lu . "Letter, Uppercase")
	       (Ll . "Letter, Lowercase")
	       (Lt . "Letter, Titlecase")
	       (Lm . "Letter, Modifier")
	       (Lo . "Letter, Other")
	       (Mn . "Mark, Nonspacing")
	       (Mc . "Mark, Spacing Combining")
	       (Me . "Mark, Enclosing")
	       (Nd . "Number, Decimal Digit")
	       (Nl . "Number, Letter")
	       (No . "Number, Other")
	       (Pc . "Punctuation, Connector")
	       (Pd . "Punctuation, Dash")
	       (Ps . "Punctuation, Open")
	       (Pe . "Punctuation, Close")
	       (Pi . "Punctuation, Initial quote")
	       (Pf . "Punctuation, Final quote")
	       (Po . "Punctuation, Other")
	       (Sm . "Symbol, Math")
	       (Sc . "Symbol, Currency")
	       (Sk . "Symbol, Modifier")
	       (So . "Symbol, Other")
	       (Zs . "Separator, Space")
	       (Zl . "Separator, Line")
	       (Zp . "Separator, Paragraph")
	       (Cc . "Other, Control")
	       (Cf . "Other, Format")
	       (Cs . "Other, Surrogate")
	       (Co . "Other, Private Use")
	       (Cn . "Other, Not Assigned")))))

(defun romacs-unidata-describe-canonical-combining-class (val)
  "Describe canonical combining class VAL."
  (cdr (assq val
	     '((0 . "Spacing, split, enclosing, reordrant, and Tibetan subjoined")
	       (1 . "Overlays and interior")
	       (7 . "Nuktas")
	       (8 . "Hiragana/Katakana voicing marks")
	       (9 . "Viramas")
	       (10 . "Start of fixed position classes")
	       (199 . "End of fixed position classes")
	       (200 . "Below left attached")
	       (202 . "Below attached")
	       (204 . "Below right attached")
	       (208 . "Left attached (reordrant around single base character)")
	       (210 . "Right attached")
	       (212 . "Above left attached")
	       (214 . "Above attached")
	       (216 . "Above right attached")
	       (218 . "Below left")
	       (220 . "Below")
	       (222 . "Below right")
	       (224 . "Left (reordrant around single base character)")
	       (226 . "Right")
	       (228 . "Above left")
	       (230 . "Above")
	       (232 . "Above right")
	       (233 . "Double below")
	       (234 . "Double above")
	       (240 . "Below (iota subscript)")))))

(defun romacs-unidata-describe-bidi-class (val)
  "Describe bidi class VAL."
  (cdr (assq val
	     '((L . "Left-to-Right")
	       (LRE . "Left-to-Right Embedding")
	       (LRO . "Left-to-Right Override")
	       (R . "Right-to-Left")
	       (AL . "Right-to-Left Arabic")
	       (RLE . "Right-to-Left Embedding")
	       (RLO . "Right-to-Left Override")
	       (PDF . "Pop Directional Format")
	       (LRI . "Left-to-Right Isolate")
	       (RLI . "Right-to-Left Isolate")
	       (FSI . "First Strong Isolate")
	       (PDI . "Pop Directional Isolate")
	       (EN . "European Number")
	       (ES . "European Number Separator")
	       (ET . "European Number Terminator")
	       (AN . "Arabic Number")
	       (CS . "Common Number Separator")
	       (NSM . "Non-Spacing Mark")
	       (BN . "Boundary Neutral")
	       (B . "Paragraph Separator")
	       (S . "Segment Separator")
	       (WS . "Whitespace")
	       (ON . "Other Neutrals")))))

(defun romacs-unidata-describe-decomposition (val)
  "Describe decomposition VAL."
  (mapconcat
   #'(lambda (x)
       (if (symbolp x) (symbol-name x)
	 (concat (string ?')
		 (compose-string (string x) 0 1 (string ?\t x ?\t))
		 (string ?'))))
   val " "))

(defun romacs-unidata-describe-bidi-bracket-type (val)
  "Describe bidi bracket type VAL."
  (cdr (assq val
	     '((n . "Not a paired bracket character.")
	       (o . "Opening paired bracket character.")
	       (c . "Closing paired bracket character.")))))

(provide 'romacs-unidata)
;;; romacs-unidata.el ends here
