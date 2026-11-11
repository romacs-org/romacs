## Observable by a package

| # | mechanism | GNU | romacs | why |
|---|---|---|---|---|
| VM2-KEYMAPLISP1 | `accessible-keymaps` and `keymap--get-keyelt` are Lisp functions | Both are C subrs (`src/keymap.c:2010`, `src/keymap.c:646`), and `where-is-internal` walks `Faccessible_keymaps` in C. `get_keyelt` follows a circular `(STRING . DEFN)` or `menu-item` wrapper forever | VM2 defines both in its built-in prelude (`lisp/romacs/vm/prelude.el`), since the walk calls `map-keymap`'s Lisp callback (PRIMITIVE-DOCTRINE section 3); `where-is-internal` walks `accessible-keymaps` as GNU's does. Their values, order, cycle rule, meta-prefix folding, string prefixes and `:filter` application match pinned GNU (`tests/journey_primitives.rs`); a circular wrapper signals `Circular menu item` | **Package-observable metadata:** `subrp` of either is nil and their arity errors are a closure's, as for `mapcar` (VM2-MAPCAR1) | <!-- symbols: accessible-keymaps, keymap--get-keyelt, where-is-internal -->
