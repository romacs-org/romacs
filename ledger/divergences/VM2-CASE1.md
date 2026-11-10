## Observable by a package

| # | mechanism | GNU | romacs | why |
|---|---|---|---|---|
| VM2-CASE1 | Custom casing and pinned titlecase mappings | GNU `casefiddle.c:prepare_casing_context` and `casify_object` consult current-buffer case/syntax tables and Unicode special/titlecase properties. | `capitalize` and `upcase-initials` take GNU's special casings and titlecase property from pinned GNU default mappings, and otherwise the current case tables, as `case_character` does: in-word characters through the downcase table, a word start with no titlecase change through the upcase table, a nil entry as identity (the native-table exception of VM2-COMPLETION-CASE aside); word boundaries come from the current syntax table. `current-case-table` and `set-case-table` exist; `copy-case-table` remains unavailable. | The titlecase property itself is the pinned default, not read from `char-code-property-alist`. `upcase-region`, `downcase-region`, `capitalize-region` and `upcase-initials-region` case buffer regions (casify_region, VM2-CASE3); the word commands remain absent. `tests/capitalize.rs` keeps the explicit custom-case divergence and tests the supported titlecase/initials domain. | <!-- symbols: capitalize, upcase-initials, copy-case-table, current-case-table, set-case-table -->

## VM2-CASE1 — Native Unicode casing, not GNU Unicode initialization

`upcase`/`downcase` use ICU4X `icu_casemap` 2.3.0 compiled data: simple
scalar mappings for integers, root-locale full/contextual mappings for strings.
The narrow casing crate reuses the workspace's ICU 2.3 dependency family; it
is not the ICU umbrella. No Unicode tables are generated in this repository,
no normalization is performed, and neither GNU `characters.el` nor Mule data
is required. Zed's editing commands at `5a5fbe79917971a4d52a96b6ddb04da198a5012e`
(`crates/editor/src/editor.rs`, `convert_to_upper_case`/`convert_to_lower_case`)
similarly use native full-string casing. Alacritty's terminal grid is not the
editor text model; the existing UTF-8/Ropey and byte-canonical storage stay intact.

Observable differences from GNU pin `45b0f7699df36b3849edc3bdef37d4cb9e75b9ea`:

| Witness | GNU pin | VM2 |
|---|---|---|
| `(upcase 223)` | 7838 (capital sharp S) | 223 (Unicode simple uppercase) |
| `(downcase 304)` | 304 | 105 (Unicode simple lowercase) |
| Copy syntax table, mark A punctuation, `(downcase "AΣ")` | `"aσ"` | `"aς"` (Unicode cased-character context, not Lisp syntax) |

Both return `"SS"` for `(upcase "ß")` and `"i̇"` for `(downcase "İ")`.
Provider data may differ from GNU's Unicode version. Per-buffer down/up tables
are sparse overrides of the native default: effective nil means use Unicode;
an explicit identity mapping suppresses expansion/contextual mapping. Table
parents/defaults are honored. Context is determined from original Unicode text,
not overridden characters. The existing standard case-table graph is not an
exhaustive projection of the native provider. Loading GNU's case initialization
on top would install overrides, so this slice must not be represented as GNU
character/bootstrap parity or wired to that legacy bootstrap.

Raw/unibyte high octets use raw-byte character indices, not Latin-1 indices;
non-Unicode internal characters are retained exactly and delimit Unicode runs.
Unibyte results remain unibyte, preserving property identities. As in the pin,
nonempty multibyte results discard properties, while unibyte results copy them.
Non-natural override entries act as identity; a positive override above
`#x3fffff` is safely rejected with `wrong-type-argument characterp` rather than
entering an invalid encoder. A non-byte override in an unibyte result falls
back to its original byte (ASCII uses native ASCII casing), never truncation.
Oversized nonnegative integer arguments are retained rather than GNU's C-int
narrowing. `capitalize`, region/word editing, regex case folding and a full
editor open/edit/save round trip are outside this slice.
