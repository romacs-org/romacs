# Deliberate divergences from GNU Emacs

One file per divergence, named by its ID. A file holds the section the
divergence belongs to, the table header and its row, and any longer
explanation below. A new divergence is a new file; `tools/lib/divergence_index.py
SYMBOL` finds the ones that name a symbol. Files named `U-...` hold entries
that never had an ID. What follows is the ledgers' prose that belongs to no
single entry.

## Observable by a package: owned local editor commands

## Observable by a package: scoped GUI command bootstrap

## Observable by a package: columns and minibuffer identity

## Observable by a package: save batch (WIP, not accepted)

## Observable by a package: executable image immutability

## Observable by a package: isolates and frozen values

## Font selection order

## Native leaf batch boundaries

## fns.c sequence access boundary

**This file is the ledger. A divergence that is not listed here does not
exist — it is a bug.** Detail lives in the per-crate files linked below; this
page is the one place to look to find out what romacs does differently and
why.

Every entry answers four things, in this order:

1. **What GNU does**, from the implementation in `src/*.c` — not the
   docstrings, which have been wrong here more than once.
2. **What romacs does.**
3. **Can a package observe it?** This is the test that decides whether a
   divergence is acceptable at all.
4. **Why**, in one line.

A divergence that a package can observe is not forbidden; an unrecorded one
is. Unsupported operations should fail loudly unless their row explicitly
records a non-signalling alternative. Existing exceptions include ignored face
`:font`, integer default `match-data` in binary buffers (VM2-MD2), ignored syntax
descriptor flags (VM2-SY1), overlay enumeration/tie order and weak-hash slot reuse
order (H4), and the silent DOCSTRING difference below (VM2-ALIAS2). Other rows explicitly describe changed return values,
selection/precedence, metadata, allocation, or ignored options without a runtime
diagnostic; their disclosure is not a claim that the operation signals. Every
observable difference must be recorded here, whether or not it fails loudly.

---

## Residual interpreted macro calls — KNOWN, PARKED and UNFIXED

## Ordered ERT measurement

## Filesystem predicate boundary

## VM2 explicit garbage collection

## Observable by a package: case-table error routing

## VM2 standard case-table startup stage

## VM2 GNU bytecode representation

## VM2 buffer-local substrate subset

- **VM2-BUFLOCAL1 — native local-variable inventories are incomplete.** GNU
  `src/buffer.c:1485` enumerates ordinary locals plus every native buffer slot,
  in buffer-alist/native-slot order. VM2 enumerates ordinary locals and every
  `DEFVAR_PER_BUFFER` slot local in the buffer, in symbol-slot order; a
  package can observe the ordering. `buffer-local-value`/`local-variable-p` for unrepresented native
  buffer fields retain their existing ordinary-variable behavior. Why: this
  lane adds clear-one storage and modeled slots, not the entire GNU buffer
  structure. The live-GNU corpus compares explicit supported projections,
  never equates that corpus with complete inventory parity.
  <!-- symbols: buffer-local-variables, buffer-local-value, local-variable-p -->

- **VM2-BUFLOCAL2 — major-mode reset covers represented state only.** GNU
  `src/buffer.c:1137,3031` resets case tables, abbrev tables, invisibility state,
  redisplay flags and (for `reset`) the entire native buffer state; it also
  notifies variable watchers except for the documented suppressing arguments.
  VM2 runs `change-major-mode-hook`, removes eligible ordinary/forwarded
  locals, preserves permanent hooks and modeled visit metadata, and resets
  major-mode/mode-name, local keymap and syntax table. `reset` additionally
  clears modeled file name/read-only/format and visit timestamp/identity/
  truename metadata. Missing case/abbrev/redisplay state and variable watchers
  are not reset/notified. Packages can observe this. Why: the missing native subsystems are outside
  the buffer-local storage slice, rather than silently claimed as implemented.
  <!-- symbols: kill-all-local-variables, kill-local-variable -->

## VM2 source-position and alias diagnostics

- **VM2-SYMPOS1 — positioned symbols are not represented.** GNU
  `src/data.c:362,840,854,876` provides `symbol-with-pos-p`, `bare-symbol`,
  `symbol-with-pos-pos` and `position-symbol` over `PVEC_SYMBOL_WITH_POS`.
  VM2 lacks that object type and these callable primitives. Packages observe
  `(void-function symbol-with-pos-p)` while expanding GNU `define-inline`
  through macroexp.el. A predicate returning nil unconditionally would conceal
  this missing substrate. Why: full positioned-symbol identity, reader,
  evaluator and GC integration is deferred rather than supplied as a stub.
  <!-- symbols: symbol-with-pos-p, bare-symbol, symbol-with-pos-pos, position-symbol -->

- **VM2-ALIAS-DIAG1 — investigated, no GNU divergence.** A two-hop alias whose
  terminal target is void reports the ORIGINAL alias in GNU and VM2, not the
  missing terminal. Pinned GNU 45b0f7699df returns `(void-function alias-a)`
  for direct, funcall and apply calls through `alias-a -> alias-b ->
  alias-missing`. This can obscure the dependency in diagnostics, but changing
  the Lisp condition would introduce a divergence. Evidence and mutation-tested
  regression: `INLINE-CHAIN.md`, `tests/alias_void_chain.rs` in romacs-vm.
  <!-- symbols: funcall, apply -->
## VM2 replace-match

GNU pin: `45b0f7699df36b3849edc3bdef37d4cb9e75b9ea`, `src/search.c:2344`.

## VM2 host-name bootstrap subset

- **VM2-SYSNAME1 — `system-name` does not refresh GNU's identity cache.** GNU
  `src/editfns.c:1283` refreshes the kernel host name when `Vsystem_name` is EQ
  to its cached object (`src/sysdep.c:1608`), repairing in-place string edits
  and noticing kernel host-name changes. VM2 materialises its forwarded initial
  value on first access and returns its current binding verbatim, without refresh.
  Packages can observe this with `(let ((before (copy-sequence system-name)))
  (aset system-name 0 88) (equal (system-name) before))`: the pinned live GNU
  returns `t`, this implementation returns `nil` on the measured host.
  Evidence: `crates/romacs-vm/tools/primh/evidence/system-name.json`, case
  `system-name/0010` (VALUE). Why: this lane unblocks the startup call and
  preserves explicit string/non-string/nil overrides; cache state is deferred.

- **VM2-SYSNAME2 — startup host acquisition is Linux/UTF-8-only.** GNU
  `src/sysdep.c:1608` obtains raw kernel bytes via `gethostname` (or `uname`),
  normalizes spaces/tabs, and uses `build_string`'s multibyte detection. VM2
  reads `/proc/sys/kernel/hostname` as UTF-8, normalizes spaces/tabs and exposes
  a unibyte string; absent procfs, read errors or invalid UTF-8 yield the
  explicitly non-signalling fallback `"unknown"`. Non-ASCII hosts can differ
  in string multibyteness; non-Linux or restricted-procfs packages can see the
  fallback rather than their host name. Why: the proven target is the measured
  Linux ASCII host; no new unsafe syscall boundary or platform dependency is
  introduced. Those other host configurations have not been live-oracle tested.

- **VM2-SYSNAME3 — host acquisition is deferred; build-details policy is absent.**
  GNU `init_editfns` initializes host state at startup, including after dump
  reload, and `init_system_name` returns nil when build details are disabled.
  VM2 acquires the host name on first access, not at `Vm::new`; a host-name
  change before that access can therefore affect the initial value. Code images
  do not save forwarded storage, so their fresh VM also acquires the current
  host on first access. No `--no-build-details` host-name policy is implemented;
  packages can see a non-nil name in a reproducible-build regime. Why: preserve
  VM2's allocation-free constructor; build-details CLI policy remains deferred.

- **VM2-SYSNAME4 — buffer-local host-name bindings are unsupported.** GNU permits
  `make-local-variable` / `make-variable-buffer-local` on `system-name`. As a
  forwarded built-in, VM2 instead signals `VM2 cannot make built-in variable
  buffer-local: system-name` (VM2-CVAR2). Global assignment and dynamic
  nil/string/non-string overrides remain supported. Why: localized forwarding
  is not yet implemented; the host-name variable uses the same storage contract
  as its peer built-ins.

## Buffer lifecycle remaining boundaries

- **VM2-BUFLIFE3 — RESOLVED, superseded by VM2-EXCUR1.** `save-excursion` is
  implemented natively (`src/save_excursion.rs`, GNU `src/editfns.c:818`) and
  `src/buffer-tests.el::test-remove-overlays` passes again; the file's floor
  rose 108 to 113 in the same recording. Its unibyte boundary, VM2-EXCUR1,
  is withdrawn.

- **VM2-IFC-HELD1 — the overlays of a buffer `insert-file-contents` is
  deciding are held aside.** GNU makes the empty buffer unibyte while
  `set-auto-coding-function` reads the file's bytes, and its markers stay at
  BEG. VM2's unibyte buffer takes the markers, at BEG (`insert_from_gap_1`
  moves none), and holds the empty multibyte buffer's overlays aside until
  the unwind restores it (`fileio_set_multibyte_flag`). While it is held,
  overlay queries on that buffer signal, and `set-buffer-multibyte t`
  signals `cannot decode buffer while insert-file-contents decides its
  coding`, where GNU succeeds. GNU's
  `set-auto-coding` only reads the buffer, so only a custom
  `set-auto-coding-function` can observe this; the buffer and its markers
  are intact afterwards (`insert_file_contents_refuses_unsupported_decoding_intact`).
  <!-- symbols: insert-file-contents, set-auto-coding-function -->

- **VM2-INDIRECT1 — indirect buffers share only multibyte text.** GNU makes
  an indirect buffer of any buffer and lets `set-buffer-multibyte` convert a
  base buffer's text for all its indirect buffers. VM2 shares a multibyte
  (UTF-8) buffer's text only: `make-indirect-buffer` of a unibyte buffer
  signals `VM2 has no indirect buffer of a unibyte buffer`, and making a base
  buffer unibyte while it has indirect buffers (by `set-buffer-multibyte` or
  `insert-file-contents`'s coding toggle) signals `VM2 cannot make a buffer
  sharing its text unibyte`, before anything changes. In an indirect buffer
  `set-buffer-multibyte` signals GNU's own error. No caller is known to make
  an indirect buffer of unibyte text; comint, org and markdown-ts use
  multibyte ones (`tests/indirect_buffer.rs`).
  <!-- symbols: make-indirect-buffer, set-buffer-multibyte -->

- **VM2-BUFLIFE1 — missing process subsystem.** Pinned GNU
  `45b0f7699df36b3849edc3bdef37d4cb9e75b9ea` runs the default
  `process-kill-buffer-query-function` and returns t when killing a normal
  process-free buffer. VM2 runs that function too, but it signals
  `(void-function get-buffer-process)` after completed `subr.el` loads: VM2 has
  no process object or buffer-process association. Custom query functions and
  kill hooks genuinely run; nil queries veto. Inhibited buffers skip all three
  lifecycle hooks for their lifetime. **Package-observable unresolved
  subsystem**, not an ignored optional argument or a disabled GNU query hook.
- **VM2-BUFLIFE2 — replacement selection.** As GNU's Fkill_buffer, the
  Lisp half (`lisp/romacs/vm/buffer_lifecycle.el`) calls `replace-buffer-in-windows`
  once window.el defines it, then makes `other-buffer`'s choice current in
  place of a killed current buffer, and keeps the buffer when it is its own
  choice (a sole `*scratch*`) or the minibuffer's only fallback. Where those leave the buffer shown or current
  (the bare VM, or `other-buffer` finding none), the native kill takes
  other_buffer_safely's: the first live Unicode buffer not beginning with a
  space and not the minibuffer's, otherwise a new `*scratch*`. VM2 keeps no
  frame buffer list (VM2-FRAME1), so `other-buffer` scans the buffer list
  alone and `buffer-list` of a live frame is the buffer list. **Package-observable** only where GNU's frame buffer list would order
  candidates differently.

## Observable by a package

These change what a package sees. Each is deliberate.

"`vertical-motion`, and the recentering, scrolling and `move-to-window-line` built on it, never signal such a refusal: what the host's layout cannot measure moves as on a terminal frame, by the frame's character cells (GNU's line-based paths), and what the iterator cannot follow at all by buffer lines; `line-pixel-height` answers the frame's line height there. `pos-visible-in-window-p` and `posn-at-point` place a position a replacing display covers where pos_visible_p does (on the display line where it begins, at the first element at or after the position) and one an ellipsis shows where the character before it ends, and a string's glyph names the string in the position. In a displayed window none of these signals in interactive use: where the host's measurement refuses, `window-text-pixel-size`, `buffer-text-pixel-size`, `pos-visible-in-window-p`, `posn-at-point`, `window-end` with UPDATE and `line-pixel-height` answer an approximation (the frame's character cells as on a terminal frame, else every buffer line a row of the frame's line height and every character a column, the recorded end for `window-end`) and count the refusal by its message and the buffer's major mode (`romacs--layout-refusals`, which the usage log writes as `refusal` records); batch signals them as before. The hosted measurement refuses where the host does not draw what GNU does: a display table's truncation or continuation glyph code with a face of its own, where the host draws one that is a character in the default face (`with a display table's faced truncation or continuation glyphs`); a line or wrap prefix (`of a line or wrap prefix`); a space with `:height`, `:relative-height` or `:ascent` (`of a space with its own height`), a `:relative-width` space over a TAB or newline and a width in a font's units (`width`, `height`), which GNU takes from the glyph's font (`of a space of relative width`, `of a font-relative display width`); any other `:relative-width` space (Org's table separators) is FACTOR times the host's glyph of the character it covers; visual, widened or offset line numbers, and a `line-number` or `line-number-current-line` face whose digits and spaces differ in width or whose row is taller (`with visual line numbers`, `with widened or offset line numbers`, `with a line-number face of its own widths or height`); line numbers on batch's terminal frame, where GNU counts them in their width but not in the iterator's x (`with line numbers`); a display table that sets characters' glyphs or the truncation, continuation, escape or control glyph (`with a display table`); `from a vertical offset`; and, as on the terminal frame, strings from two overlays that share a start, where the window shows its characters alone. Overlay faces stack in the order `get-char-property` uses, which reads a cons `priority` as 0 where GNU's compare_overlays compares its primary and secondary priorities. A face's geometry is its merged family and size: weight and slant are not realized (the host paints them upright and regular, which Iosevka's advances share), nor is `face-remapping-alist`. The mini-window still splices its overlay strings by its own older mechanism (`src/mini_window.rs`), which these elements should replace. The last visible x is the text area, less the continuation or truncation glyph's advance in a window without a right fringe, as on a GUI frame. MODE-LINES adds the host's band: the last painted band's height, else the mode-line face's (GNU's CURRENT_MODE_LINE_HEIGHT). With no host layout attached a displayed window signals `VM2 window-text-pixel-size in a displayed window is unsupported` (`buffer-text-pixel-size` names itself) | **Package-observable** in the GUI only for the refused forms, which answer approximately there instead of signalling; keymap-popup's and `fit-window-to-buffer`'s plain popups and emacs-hermes's chat rule size from the host's layout. `crates/romacs-vm-gui/harness/wtps_live.py` asks pinned GNU (pgtk, no fringes) and Romacs, both in Iosevka 14, the hermes chat rule in its work buffer and in a window (`(space :width 2)`, faced text, an `:align-to right` fill, X-LIMIT, Y-LIMIT, MODE-LINES), the `(space :width text)` separator, `:align-to` columns and a tabulated list, invisible text and ellipses, overlay strings, an isearch overlay face, wrapped lines under `visual-line-mode` (`kill-visual-line`, the visual line's ends, `vertical-motion`, `posn-at-point`) and under plain continuation, truncated lines, `display` strings, empty replacements and taller overlay faces, and every answer is GNU's; `crates/romacs-vm-gui/src/measure.rs` compares the hosted answers with the host's painted glyphs, stretches, cursors, rows and band for text in several fonts, sizes, widths and `tab-width`s, and for those display elements, and each refusal with a twin that answers; `harness/lines_live.py` checks every painted glyph of a visited lisp/gnu/ file with TABs at its GNU column; `harness/popup_live.py` fits a hermes keymap popup in a native session and judges it by the painted scene. In batch, `tests/window_text_pixel_size.rs` compares 70 primitive cases, the refusal of overlays sharing a start (with GNU's answer for a moved overlay), window.el's side-window fitting and a keymap-popup popup with pinned GNU | A host that draws prefixes, escapes as GNU's glyphs and fonts' own measures would lift the remaining refusals; the motion and elements already model them | <!-- symbols: window-text-pixel-size, buffer-text-pixel-size, fit-window-to-buffer, string-pixel-width -->

### Planned, not yet in effect

Recorded now so they are not a surprise later. See
`milestones/01-lisp-machine/SURFACE-REMOVAL.md` in project-context.

---

## Not observable by a package

Internal choices that produce identical Lisp-visible behaviour. Listed so
nobody re-litigates them.

- Plist ordering internally, where GNU's own order is unspecified.
- `reset` resolved to the default face's value, matching GNU's C.
- Dependent provider coverage invalidated immediately.
- Identical overlays given a deterministic final tie-break.
- H4: weak hash entries freed by a collection go back on the free list in slot
  order, not GNU's address-dependent bucket order.
- H5: keys whose hash reads a moving heap index are re-hashed after compaction;
  differs from GNU only for keys mutated after insertion (undefined in GNU).
- H7: `make-hash-table :size` is recorded exactly but slots are materialized on
  first use; `:size` ≥ 2^30 fails before allocating.
- Regexp matching runs a linear-time Pike VM for a forward search once its
  candidates cost more than the text justifies, and for any catastrophic
  candidate; backreferences, intervals, `??`, nullable non-greedy loops and
  POSIX patterns stay on the backtracker.  Same matches and registers as GNU's
  backtracker ([RX1](RX1.md)).

---

## Withdrawn

Kept visible on purpose: a corrected claim is worth more than a quietly
deleted one.

- **VM2-CVAR-PRED1 — `scroll-up-aggressively` and
  `scroll-down-aggressively` accepted any value.** Withdrawn 2026-10-30.
  Every `DEFVAR_PER_BUFFER` now checks GNU's predicate on a set: `fraction`'s
  number in [0.0, 1.0] with `wrong_range`'s error, `vertical-scroll-bar`'s and
  `overwrite-mode`'s `choice` with `wrong_choice`'s, and `integerp`,
  `stringp` and `symbolp` (romacs-vm-gui `tests/buffer_locals.rs`).

- **VM2-BOOT1, VM2-BOOT2, VM2-BOOT3, VM2-BOOT6, VM2-BOOTSTRAP1 — bootstrap
  rewrites of GNU's own Lisp.** Withdrawn 2026-10-20 (Phase 0, lane
  `vm2/oracle`). Each row described an edit made IN PLACE to a GNU file under
  `lisp/` (`byte-run.el`'s `eval-and-compile`; `subr.el`'s
  `buffer-local-restore-state`, `dolist` length guard, octal/`logior`/`setf`
  spellings; `macroexp.el`'s `cadr`), which R221 forbids: a number measured on
  an edited oracle file is not a number. GNU's files are restored byte for
  byte (`tools/lib/lisp_oracle.py check`), so the divergences no longer exist; the
  VM gaps they hid were tracked as tickets G1-G8.
  The old row text is in git history (`git show 18e249d7:DIVERGENCES.md`).
- **VM2-EXCUR1, VM2-RESTRICT1 — a unibyte buffer with a saved excursion
  or restriction stayed unibyte.** Withdrawn 2026-10-29. Unibyte buffers
  now hold markers in an anchor tree, as text buffers do (`bytes.rs`), so
  `save-excursion` and `save-restriction` save real markers there and
  `set-buffer-multibyte` carries every marker across: converting inside the
  form answers pinned GNU's `(t 2 3 t)` (`real_ert_load.rs`), and
  `tests/unibyte_markers.rs` matches GNU on setting, moving, insertion
  types, deletion and conversion.
- **VM2-RUNHOOKS1 — `run-hooks` could not call a quoted `(lambda ...)` hook
  function.** Withdrawn 2026-10-28. The engine now calls a `(lambda ...)` list
  as GNU's funcall_lambda does (VM2-ALIAS1), so the six pinned `run-hooks`
  rows and `run-hook-with-args/0518`, `/0519`, `/0529` match GNU and are
  unpinned; `prims::t2_tests::run_hooks_calls_lambda_lists_whole` logs which
  object each hook call received.
- **D003 — `not` retains the primitive `null` value.** Withdrawn. The
  comparison used GNU *interpreted* against VM2 *compiled*. GNU byte-compiles
  `(not x)` to `Bnot` and agrees with VM2. There is no divergence.

---

## Detail

The longer explanations, each with its live-GNU transcript, follow the row in
the entry's own file. A divergence with no row of its own (the crate ledgers'
sections without an ID) is a `U-` file.

## VM2 format-message boundary

## Observable by a package: mini-window display

## Observable by a package: circular completion tables

## Explicit OClosure compatibility

## GNU ERT batch execution

## Observable by a user: workspace file coding

## World snapshots

## Observable by a package: syntax summaries

## romacs-face

Reference: GNU Emacs 32.0.50 built at the pin 45b0f7699df (`$ROMACS_GNU_ORACLE -Q --batch`,
checked by `tools/lib/gnu_oracle.py`), throwaway HOME. Fixtures before R228 came from
`/usr/bin/emacs` at b28750b822d; regenerating at the pin changed only `separator-line`.
This crate is not wired to redisplay yet. These are the contracts for wiring,
not claims about the running editor. R90 removes Custom's machinery, not the
user's preferences. `defface` and `face-remapping-alist` remain supported.

## romacs-regex

Reference: GNU Emacs 32.0.50, `src/regex-emacs.c` and `src/search.c` (the
implementation, not the docstrings).

**One introduced by the linear-time matcher (R92 step 2), RX1 below.**
Everything else the matcher returns is GNU's: the linear matcher reproduces
the backtracker's leftmost-first priority and registers, including GNU's
empty-loop rule, and `tools/regex_oracle.py` checks it against live GNU
(106 cases, fresh process each, byte-compiled).

## romacs-vm

## Deliberate annotation differences from GNU Emacs

The annotation layer implements per-key runs, raw presence/value lookup,
category/alias/default resolution, insertion stickiness and property-bearing
kill/yank strings. Markers and the overlay interval index are described in
ANCHORS.md. Provider extents, dependent invalidation and a Lisp
`object-intervals` entry point remain later milestones. The first two rules below
are **planned contracts, not implemented-feature claims**. They do not authorize
ignoring other GNU mismatches.
