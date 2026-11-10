## VM2 replace-match

| # | Mechanism | GNU | VM2 | Why |
|---|---|---|---|---|
| VM2-REPLACE1 | `replace-match` buffer/case operations | `search.c:2344` replaces buffer matches through `replace_range` and applies current-buffer case/syntax tables when FIXEDCASE is nil. | String and buffer replacement both infer all-caps/initial casing from the match. A buffer replacement is `romacs--replace-range` (change hooks, undo, markers, match-data adjustment), then `upcase-region` or `upcase-initials-region` over the new text, then point after it. Conversion of initial letters shares VM2-CASE1 pinned titlecase limitations; a casing that expands a character follows VM2-CASE3. VM2 has no intangible text, so point always ends after the replacement. | Buffer and string cases are compared with pinned GNU in `tests/replace_match.rs`. |
