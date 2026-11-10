## Observable by a package

| # | mechanism | GNU | romacs | why |
|---|---|---|---|---|
| VM2-STR12 | NOESCAPE printing of unibyte high bytes | Fresh GNU `(let ((s (prin1-to-string (unibyte-string 255) t))) (list (length s) (aref s 0) (multibyte-string-p s) (string-bytes s)))` returns `(4 92 nil 4)` (ASCII octal text). Buffer-backed `(with-output-to-string (princ (unibyte-string 255)))` returns `(4 92 t 4)` under the same inspection. | Romacs still returns `(1 255 t 2)` for the first witness: NOESCAPE substitutes Latin-1 Unicode instead of octal text. The buffer-backed capture witness now matches GNU after loading its `with-output-to-string` macro from subr.el; princ and buffer output are native. | Remaining debt is the NOESCAPE printer-result characters and string kind. The unchanged GNU capture macro is an explicit source prerequisite, not a bare-native definition. | <!-- symbols: prin1-to-string, princ, with-output-to-string -->

## VM2-STR12 — NOESCAPE high-byte printer boundary

Fresh isolated GNU prints the unibyte byte 255 as four ASCII characters `\377`
even with NOESCAPE non-nil:

```elisp
(let ((s (prin1-to-string (unibyte-string 255) t)))
  (list (length s) (aref s 0) (multibyte-string-p s) (string-bytes s)))
;; GNU => (4 92 nil 4); VM2 => (1 255 t 2)
```

This pre-existing Latin-1 substitution is not D004's control-escape convention.
The GNU `with-output-to-string` / `princ` equivalent gives `(4 92 t 4)`;
VM2 has no such output-stream/capture functions and signals `(void-function)`.
Removing the printer's `quoted` guard alone fixes the escaped characters but
not GNU's unibyte result kind. Full matching therefore also needs the P1b
printer-result kind change and output capture, rather than only the permitted
printer-path repair. This finding is recorded, not declared fixed; P1a keeps
literal/result kinds unchanged.
