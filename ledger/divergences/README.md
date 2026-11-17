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



## VM2 source-position and alias diagnostics


## VM2 replace-match

GNU pin: `45b0f7699df36b3849edc3bdef37d4cb9e75b9ea`, `src/search.c:2344`.

## VM2 host-name bootstrap subset





## Buffer lifecycle remaining boundaries




## Observable by a package

These change what a package sees. Each is deliberate.

"`vertical-motion`, and the recentering, scrolling and `move-to-window-line` built on it, never signal such a refusal: what the host's layout cannot measure moves as on a terminal frame, by the frame's character cells (GNU's line-based paths), and what the iterator cannot follow at all by buffer lines; `line-pixel-height` answers the frame's line height there. `pos-visible-in-window-p` and `posn-at-point` place a position a replacing display covers where pos_visible_p does (on the display line where it begins, at the first element at or after the position) and one an ellipsis shows where the character before it ends, and a string's glyph names the string in the position. In a displayed window none of these signals in interactive use: where the host's measurement refuses, `window-text-pixel-size`, `buffer-text-pixel-size`, `pos-visible-in-window-p`, `posn-at-point`, `window-end` with UPDATE and `line-pixel-height` answer an approximation (the frame's character cells as on a terminal frame, else every buffer line a row of the frame's line height and every character a column, the recorded end for `window-end`) and count the refusal by its message and the buffer's major mode (`romacs--layout-refusals`, which the usage log writes as `refusal` records); batch signals them as before. The hosted measurement refuses where the host does not draw what GNU does: a display table's truncation or continuation glyph code with a face of its own, where the host draws one that is a character in the default face (`with a display table's faced truncation or continuation glyphs`); a line or wrap prefix (`of a line or wrap prefix`); a space with `:height`, `:relative-height` or `:ascent` (`of a space with its own height`), a `:relative-width` space over a TAB or newline and a width in a font's units (`width`, `height`), which GNU takes from the glyph's font (`of a space of relative width`, `of a font-relative display width`); any other `:relative-width` space (Org's table separators) is FACTOR times the host's glyph of the character it covers; visual, widened or offset line numbers, and a `line-number` or `line-number-current-line` face whose digits and spaces differ in width or whose row is taller (`with visual line numbers`, `with widened or offset line numbers`, `with a line-number face of its own widths or height`); line numbers on batch's terminal frame, where GNU counts them in their width but not in the iterator's x (`with line numbers`); a display table that sets characters' glyphs or the truncation, continuation, escape or control glyph (`with a display table`); `from a vertical offset`; and, as on the terminal frame, strings from two overlays that share a start, where the window shows its characters alone. Overlay faces stack in the order `get-char-property` uses, which reads a cons `priority` as 0 where GNU's compare_overlays compares its primary and secondary priorities. A face's geometry is its merged family and size: weight and slant are not realized (the host paints them upright and regular, which Iosevka's advances share), nor is `face-remapping-alist`. The mini-window still splices its overlay strings by its own older mechanism (`src/mini_window.rs`), which these elements should replace. The last visible x is the text area, less the continuation or truncation glyph's advance in a window without a right fringe, as on a GUI frame. MODE-LINES adds the host's band: the last painted band's height, else the mode-line face's (GNU's CURRENT_MODE_LINE_HEIGHT). With no host layout attached a displayed window signals `VM2 window-text-pixel-size in a displayed window is unsupported` (`buffer-text-pixel-size` names itself) | **Package-observable** in the GUI only for the refused forms, which answer approximately there instead of signalling; keymap-popup's and `fit-window-to-buffer`'s plain popups and an agent chat's rule size from the host's layout. `crates/romacs-vm-gui/harness/wtps_live.py` asks pinned GNU (pgtk, no fringes) and Romacs, both in Iosevka 14, a chat rule in its work buffer and in a window (`(space :width 2)`, faced text, an `:align-to right` fill, X-LIMIT, Y-LIMIT, MODE-LINES), the `(space :width text)` separator, `:align-to` columns and a tabulated list, invisible text and ellipses, overlay strings, an isearch overlay face, wrapped lines under `visual-line-mode` (`kill-visual-line`, the visual line's ends, `vertical-motion`, `posn-at-point`) and under plain continuation, truncated lines, `display` strings, empty replacements and taller overlay faces, and every answer is GNU's; `crates/romacs-vm-gui/src/measure.rs` compares the hosted answers with the host's painted glyphs, stretches, cursors, rows and band for text in several fonts, sizes, widths and `tab-width`s, and for those display elements, and each refusal with a twin that answers; `harness/lines_live.py` checks every painted glyph of a visited lisp/ file with TABs at its GNU column; `harness/popup_live.py` fits a keymap popup in a native session and judges it by the painted scene. In batch, `tests/window_text_pixel_size.rs` compares 70 primitive cases, the refusal of overlays sharing a start (with GNU's answer for a moved overlay), window.el's side-window fitting and a keymap-popup popup with pinned GNU | A host that draws prefixes, escapes as GNU's glyphs and fonts' own measures would lift the remaining refusals; the motion and elements already model them | <!-- symbols: window-text-pixel-size, buffer-text-pixel-size, fit-window-to-buffer, string-pixel-width -->

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

- **VM2-ALIAS-DIAG1 — investigated, no GNU divergence.** A two-hop alias whose
  terminal target is void reports the ORIGINAL alias in GNU and VM2, not the
  missing terminal. Pinned GNU 45b0f7699df returns `(void-function alias-a)`
  for direct, funcall and apply calls through `alias-a -> alias-b ->
  alias-missing`. This can obscure the dependency in diagnostics, but changing
  the Lisp condition would introduce a divergence. Evidence and mutation-tested
  regression: `INLINE-CHAIN.md`, `tests/alias_void_chain.rs` in romacs-vm.
- **VM2-BUFLIFE3 — RESOLVED, superseded by VM2-EXCUR1.** `save-excursion` is
  implemented natively (`src/save_excursion.rs`, GNU `src/editfns.c:818`) and
  `src/buffer-tests.el::test-remove-overlays` passes again; the file's floor
  rose 108 to 113 in the same recording. Its unibyte boundary, VM2-EXCUR1,
  is withdrawn.
- **VM2-BUFLIFE1 — missing process subsystem.** Withdrawn 2026-11-17.
  `get-buffer-process` is defined, and killing a process-free buffer runs
  the default `process-kill-buffer-query-function` and returns t, as pinned
  GNU does (both asked on a beta build).
- **VM2-SYSNAME4 — buffer-local host-name bindings were unsupported.**
  Withdrawn 2026-11-17. `make-local-variable` of `system-name` returns the
  symbol in Romacs as in pinned GNU (both asked on a beta build).
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
  spellings; `macroexp.el`'s `cadr`), which R221 then forbade (retired
  2026-11-15, when lisp/ became ours). The files were restored byte for byte
  (by a check since retired), so the divergences no longer exist; the
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
