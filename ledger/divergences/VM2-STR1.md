## Observable by a package

| # | mechanism | GNU | romacs | why |
|---|---|---|---|---|
| VM2-STR1 | Extended and raw-byte character substrate | GNU `(max-char)` is 4194303, `(max-char t)` is 1114111; `(string-bytes (char-to-string 1114112))` is 4 and raw-byte character 4194303 occupies 2 multibyte bytes; ASCII literals are unibyte | VM2 now matches those character limits, byte counts, conversions, equality/hash shapes and octal raw-byte printing through internal encoded `LispString`; Object remains 32 bytes. ASCII literals are unibyte, as GNU's, and unibyte ASCII of any origin takes the string slot (`data::unibyte_object`), so every reader of text takes it; only bytes above 127 are a byte string | Partial closure: full-range runtime substrate is present; P2 mixed concat widening, P6 raw/extended source constants remain |

## VM2-STR1 / STR5–STR11 — Full-range runtime strings, staged source boundary

P1a stores GNU characters through #x3FFFFF in canonical internal encoding,
including 5-byte extended and 2-byte raw characters. Character limits,
conversions, byte counts, character access, equal/string-equal and equal hashes
now operate across Bytes, String and RawString. Raw-byte printing is octal.
String slots keep valid UTF-8; RawString carries the other internal encodings.
Both variants trace and forward all property values. Object is still 32 bytes.
Literal/result kind flips remain P1b; mixed high-octet concat remains P2;
property primitives and reader syntax remain later pieces.

D001 is retained: aset may replace any multibyte character with any character
in 0..=#x3FFFFF and resize its encoded width, unlike GNU's ASCII-only store.
The arena slot is reclassified in place, preserving eq and property roots.
Clear-string already demotes in place to unibyte zero bytes (GNU SBYTES length).

Unicode-only boundaries refuse `not-unicode-string`, never convert lossily:
intern (prim/widen), file names/source loading (loader/bootstrap/buffer),
process arguments (productivity), buffer names (eval_buffer/display), Unicode
buffer insertion and kill/yank (buffer/annotations/annotation_primitives),
interactive/kbd specifications (command/keymap), message templates (conditions),
and JSON display (lib/protocol). The Lisp printer octal-escapes raw bytes and
refuses extended output that cannot be represented by the Unicode JSON host.
The exact individual sites and GNU/VM2 contrast are rows STR5–STR8 in root
DIVERGENCES.md.

Per contract A3, source_form preserves its existing Form::String/Form::Bytes
mapping, including today's silent property drop. It loudly refuses newly
possible raw/extended multibyte constants (STR9–STR10); P6 owns the source-cache
codec change. The unchanged code_image.rs String-only guard rejects RawString;
mapped image ASCII strings still count as multibyte (STR11).
