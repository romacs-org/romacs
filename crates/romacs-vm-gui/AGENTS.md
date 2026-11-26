# crates/romacs-vm-gui
The GUI host: the native window's keys and display, and the command loop's
host side.

## Contracts
- A host key reaches Lisp as `crates/romacs-vm-gui/src/native.rs` makes it,
  as GNU's X and pgtk frames do, except Ctrl+m, Ctrl+i and Ctrl+[: the
  symbols `C-m`, `C-i` and `C-[`, which `function-key-map` makes RET, TAB
  and ESC where unbound (`ledger/divergences/VM2-GUI-KEYS.md`).

## Tests
- `crates/romacs-vm-gui/tests/lisp_tree_compile_failures.txt` holds stable
  compiler diagnostics on trunk. The tree check reports recovered files
  and fails new or changed errors. The recorder runs the same test with
  ROMACS_RECORD_LISP_COMPILE_FAILURES=1 on beta or under FORGE_TRUNK_JOB=1.

## Traps
- A key a test types is the event `native::translate` makes of a host key,
  not one written by hand: Return is `return`, Tab `tab`, Ctrl+m `C-m`, so
  `Character(13)` or `Character(9)` is no key a user can press.
- A keymap with a default (`[t]`) binding takes `C-m`, `C-i`, `C-[`, `tab`
  and `return` before `function-key-map` makes them RET, TAB or ESC.
