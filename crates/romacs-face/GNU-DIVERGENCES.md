# romacs-face: deliberate differences from GNU Emacs

Reference: GNU Emacs 32.0.50 built at the pin 45b0f7699df (`$ROMACS_GNU_ORACLE -Q --batch`,
checked by `tools/lib/gnu_oracle.py`), throwaway HOME. Fixtures before R228 came from
`/usr/bin/emacs` at b28750b822d; regenerating at the pin changed only `separator-line`.
This crate is not wired to redisplay yet. These are the contracts for wiring,
not claims about the running editor. R90 removes Custom's machinery, not the
user's preferences. `defface` and `face-remapping-alist` remain supported.

## Input and GC boundary

GNU's circular face-ref lists and self-containing `:inherit` plists recurse
in C until SIGSEGV; a circular plist tail hangs. We reject them during the
iterative graph conversion. Every public face-data input requires a
`BoundedDatum`: depth at most 32, at most 2^20 cons cells, and at most 16 MiB
of **expanded** UTF-8 string/symbol payload. The fixture reader also limits
printed source bytes and quote frames. These limits deliberately reject
otherwise valid but unreasonable inputs. Borrowed graph atoms are budgeted
before copying, so sharing one large string cannot amplify without bound.

Raw `Datum` is an untrusted construction language, not an accepted face
input. Its owner remains responsible for disposing of an arbitrarily deep
raw tree iteratively. `BoundedDatum::new` checks by reference before cloning.
`FaceRef` is opaque, and structured attribute values also require the checked
type; neither offers an unchecked route back into recursive face processing.

The core adapter must hold **`&Context` for the whole conversion**. Tagged
`Value` reads do not themselves borrow the heap or inhibit GC. Its `view`
callback must not allocate, intern, execute Lisp or reach a GC safe point.
Only the converter allocates native owned storage. No Lisp handle escapes.
`NodeView::Symbol` means an interned name; the reader must report uninterned
symbols as unsupported, not conflate their identities. There is no core
adapter in this crate, so the actual Context/GC integration still needs its
core-side test when wired; a toy heap test would not prove that contract.

Tests: `cycles.rs`, `readiness.rs` quote/raw-million-node subprocesses,
byte budgets, borrowed-atom budget, and `ui/raw_{defface,remap}.rs`.

## Remapping

GNU's NORMAL/REMAP merge points are distinct. A REMAP hides earlier NORMAL
points **of the same name**; when another remap would repeat that REMAP,
the original face definition is used instead. We preserve this transitively:
`((default solaire-face))` where solaire inherits default, and the dotted
`((mode-line . mode-line-inactive))`, both work. The old rejection of the
inheritance-backreference fixture was a bug and is removed.

A cycle **between distinct remapping entries** (e.g. `((a b) (b a))`) is
still deliberately rejected; GNU falls back/truncates at its merge points.
At the buffer boundary use `View::bind_or_unremapped`: a rejected remapping
returns the unremapped, paintable theme **and** its diagnostic, for this
buffer only. The caller must report that diagnostic. `bind` remains the
strict Result-returning API. `FaceCache` caches valid bound views and faces;
it does not hide binding errors or choose the caller's diagnostic UI.

Tests: `readiness.rs` valid remaps, barrier mutation, fallback;
`cycles.rs` pure remapping-entry cycle; `remap_parity.rs` C-path fixtures.

## Aliases

Alias chains are registered through `Deffaces::alias`, rejected if cyclic,
and resolved before inheritance topology, theme/user spec composition,
remap-key binding and named face-reference lookup. Thus the obsolete smerge
alias names remain usable. Ambiguous duplicate canonical keys within one
explicit theme/user value are rejected rather than choosing hash-map order.
The user alist reader likewise rejects duplicate face entries; the builder's
`set_face` explicitly replaces a previous entry. No Custom priority stack
is implied by alias resolution.

Tests: `readiness.rs` inheritance/lookup/remap/ref aliases, alias cycles,
and alias-named theme/user/runtime attribute edits.

## Heights: use the C merge for redisplay

`Theme::realize`, `View::effective`, `View::realize` and `View::realize_ref`
replay GNU's C merge,
parents cdr-first and own attributes last, truncating each relative-to-absolute
height operation. They must not pre-multiply all scales. The review's
`:height 1.5 :inherit (relative absolute)` example gives **235**, not Lisp's
236. Over default 113, two successive 1.5 scales give **253**, not 254.
`face_attribute` retains the separate Lisp height-query grouping.

`tests/gnu/redisplay.el` produced `tests/gnu/redisplay.eld` using GNU's actual
`face-attributes-as-vector`, not `face-attribute` as a stand-in for redisplay.
Its 16 rows cover default heights 100, 113, 97 and 120, with separate C
unspecified/absolute-base results and Lisp query results. Regenerate only
under `-Q` and an isolated HOME; review the fixture diff.

The older 25-theme plus unthemed fixtures remain useful for selection and
attributes, but mostly compare unspecified/default values. Their default
height is 1, so most face-specific heights truncate to 0. They are **not**
a substitute for the non-unit C-path oracle. `remap_parity.rs` retains the
old 24 Lisp-query height values explicitly as Lisp queries.

## One render target, including visible changes from today's romacs

We collapse for graphical truecolour Wayland: `romacs`, `graphic`, `pgtk`
and `gtk` match; tty/greyscale/monochrome branches are discarded and
`supports` is always true. Background is the remaining light/dark axis.
`gtk` is an intentional compatibility selector, not a claim that this
renderer is GTK or that its Lisp `featurep` says so.

S98's independent **real romacs GUI** probe found window-system `romacs`,
display-graphic-p true, featurep gtk nil. Today's faces.el matches `graphic`
but not `(type x w32 ns haiku pgtk android)` there. Wiring this crate will
select the GUI branch for nine corpus specs. `custom-button` currently gets
**no attributes at all**, not a tty fallback; it gains its GUI attributes.
A running GNU pgtk editor is not affected by today's romacs bug.
Our answer is deliberately graphical, not preservation of that blank face.

Tests: `collapse.rs::specs_written_for_pgtk_match_romacs` and
`readiness.rs::romacs_matches_graphical_specs` (including `graphic`).

## R90: one theme plus one explicit user preference value

GNU replays enabled themes and its always-enabled `user` theme,
`custom-set-faces` and `theme-face` records. We have **none** of that stack.
`Theme::build_with_overrides` composes one `ThemeSpec` with one
`Arc<UserOverrides>`. Unspecified user slots retain the theme's values;
explicit user inheritance replaces the inherited-name list. Changing the
builder cannot mutate an already-built theme. Theme replacement retains
the same user preferences; transient runtime setter edits are discarded.

`examples/user-overrides.eld` is the port of one user's eight face
preferences from custom.el: Iosevka 120 default, black fringe/dividers,
Iosevka italic and variable-pitch, and Iosevka Aile org-modern-symbol.
It is **plain data**, loaded with `BoundedDatum::parse` and
`UserOverrides::from_alist`, not an Elisp Custom form. The old `t` theme
flag on org-modern-symbol disappears: a new explicit preference face may
be added even before its authoring package loads. No user config is edited
by this crate. `readiness.rs::owner_preferences_compose_over_bundled_modus`
builds the bundled modus-vivendi with this exact file and checks all eight
outcomes while preserving the theme's default background.

A built theme swap is an Arc replacement; a rebuild is not. All old numeric
speed comparisons between a prebuilt swap and GNU's full enable/disable
rebuild are withdrawn. No speed advantage is claimed here. Runtime setters
and late defface still rebuild the theme and mint a new generation; cached
realization misses replay the C merge iteratively. Batch mutations before
adding pulse-style per-tick callers; do not infer O(1) setters from O(1) swaps.

## Remaining deliberate differences

| mechanism | GNU | romacs | test |
|---|---|---|---|
| `reset` in realized queries | Lisp query can return `reset`; C substitutes default | realized values substitute default, never paint a sentinel | `parity.rs` reset classification |
| anonymous plist `:inherit` | positional; inherited value can override an earlier own attr | own attrs always win | `remap_parity.rs` anonymous-attribute-before-inherit |  <!-- symbols: face-attribute, defface -->
| function heights | invokes Lisp during C merge | rejected: code is not immutable face data | `readiness.rs` removed_font_mechanisms_have_explicit_tests |  <!-- symbols: set-face-attribute, face-attribute -->
| `:font` | window-frame font backend interprets it | ignored here; font objects belong to romacs-font | same test, height remains base 113 |  <!-- symbols: set-face-attribute, face-font -->
| dashed family | setter can split foundry/family | family stays literal `Foo-Bar`, foundry unspecified | same test |  <!-- symbols: set-face-attribute, face-family -->
| undefined parent | C skips; Lisp query can stop at the first missing face | skipped, including Lisp-style query merge | `readiness.rs` missing_parent_does_not_hide_later_parent |  <!-- symbols: face-attribute, defface -->
| `:bold` / `:italic` | C applies keys in order (contrary to docstring) | applies in order, matching C | `parity.rs` corpus |  <!-- symbols: set-face-attribute -->
| window filters | matches using window context | never matches; this View has no window | `remap_parity.rs` filtered fixture |  <!-- symbols: face-remapping-alist -->

The default-face underline reset is independently tested with a base that
has underline **true**; it cannot pass merely because the base supplies nil.
`tests/mutants.json` and `tests/mutants.py` commit exact preimages, replacement
text, target tests and expected assertion markers. Replay receipts belong
outside the source tree and include restored-green runs; compile failures are
not accepted as assertion-red evidence.
