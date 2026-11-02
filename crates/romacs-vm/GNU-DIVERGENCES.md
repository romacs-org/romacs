# Deliberate GNU divergences

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


## VM2-LOCATE1 — Readable regular-file lookup subset

`locate-file-internal` implements GNU openp's search for every PREDICATE:
nil or t (readable), a natural-number access mode, and a function called on
each candidate (nil rejects it; a directory needs `dir-ok`). GNU defines `locate-file` itself in Lisp
(`lisp/files.el:1113`) and only `locate-file-internal` in C
(`src/lread.c:1590`), so the substrate provides the C entry point ONLY: a bare
VM has no `locate-file` until GNU's own `files.el` loads, and anything needing
the wrapper must load that Lisp rather than call a native stand-in.
The existing native file-name expansion/handler restrictions also apply:
nonempty `file-name-handler-alist` is refused rather than invoking handlers,
and `~user` and above-root expansion are unsupported. This lookup accepts
readable regular files, not GNU's additional non-directory device/FIFO cases.
These are unresolved compatibility gaps, not alternative GNU answers.


## VM2-TIME1 — Timestamp bignums require the missing integer substrate

GNU `fileio.c:6157` and `timefns.c:820` at revision
`45b0f7699df36b3849edc3bdef37d4cb9e75b9ea` return nanosecond ticks by default,
including bignums when needed. VM2 stores arbitrary-precision integers read
from source constants, but its timestamp substrate consumes only fixnums and
no arithmetic produces a bignum. For example:

```elisp
(set-visited-file-modtime '(40000 0))
(visited-file-modtime)
```

GNU returns `(2621440000000000000 . 1000000000)`; VM2 signals
`(error "VM2 visited-file-modtime requiring bignum ticks is unsupported")`.
The native recorded timestamp remains intact and can still be retrieved with
`current-time-list` non-nil. The reader now accepts bignum literals, but bignum
input timestamps remain unsupported by the numeric substrate that consumes them. This is an unresolved package-observable
compatibility gap, not an alternate valid GNU result. Fixnum timestamp forms,
legacy dotted microseconds, ignored trailing components and finite-float
nanosecond flooring otherwise follow the pinned implementation.

`current-time` (`timefns.c:1817`, encoding at :820/:720) now reads the
native realtime clock with integer nanosecond precision. With nil
`current-time-list`, its `(TICKS . 1000000000)` output is exact while
`-2305843009213693952 <= TICKS <= 2305843009213693951`; outside that
interval it signals exactly
`(error "VM2 current-time requiring bignum ticks is unsupported")`.
No frequency reduction, rounding, clamping, or automatic list fallback occurs.
With **any non-nil** `current-time-list`, it returns GNU's exact four-fixnum
list `(HIGH LOW USEC PSEC)`, including negative pre-epoch normalization.
This works even when nanosecond ticks require a bignum. HIGH outside the
same fixnum interval signals
`(error "VM2 current-time requiring bignum HIGH is unsupported")`;
that branch is unreachable for the native clock's range on this host.
These refusals are numeric representation gaps, not GNU conditions.
Clock changes can make successive realtime readings decrease in either VM;
this primitive does not promise a monotonic clock.

The deterministic encoder boundary tests and live clock contract tests live
in `tests/current_time.rs` and the `prims::time::tests` unit module. The pinned
GNU probe `tests/current_time_oracle.el` confirms the exact encodings, including
GNU's successful bignum results on both sides of VM2's refusal boundary.

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

## VM2-G1A — One-window display subset

G1a implements one VM-owned window and frame, not GNU window/frame objects.
The selected identities are stable VM-interned symbol tokens; GNU returns
window/frame objects, so `symbolp` can observe this limited representation.
There is no splitting, scrolling, minibuffer or per-window point storage yet.
`window-start` is 1 and the snapshot starts at byte zero; narrowing does not
restrict the G1a display view. `window-point` follows the displayed buffer point.
The default frame is Liberation Mono at 20px, white on black, for R102.

`defface` delegates condition collapse/inheritance to `romacs-face::Theme`.
GNU `custom.el`, `cus-face.el`, and `faces.el` are excluded from the native
bootstrap: Romacs owns `defface`, `defcustom`, and `defgroup`. `:type` is
retained only as debugger/documentation/agent metadata; there is no Customize
UI, Custom group machinery, or theme priority stack. `defgroup` records its
documentation, members and keyword metadata, not GNU group registration.
Literal `:initialize #'custom-initialize-delay` (also quoted symbol syntax)
leaves an uninitialized variable unbound and queues its name in
`custom-delayed-init-variables` while that value is a list. With a non-list
value it uses ordinary native variable initialization. Other initialization
functions and Custom setters remain metadata rather than callbacks; native
bootstrap does not yet drain the delayed queue. Seam L exports
foreground/background/family/absolute pixel size only. G1a face properties
accept a named face or a singleton list of a named face; a longer list currently
uses its first element, unlike GNU face-list merging. Only `#rrggbb`, white,
black and red colour spellings are converted by this producer.
These limits are package-observable and are not claims of general GNU display
compatibility. The acceptance scene uses only the supported subset.

### Observable by a package — native declaration boundary

| Surface | Intentional difference from GNU |
|---|---|
| `custom.el`, `cus-face.el`, `faces.el` | Excluded from the native bootstrap; declarations use native lowering, not Customize or GNU's mutable face registry. |
| `defgroup` | Documentation, members and keywords are stored as metadata only; no group registration, membership updates, hooks or Customize UI. |
| `defcustom` | Type/set/get/initialize keywords are metadata, not Custom callbacks. Literal `custom-initialize-delay` defers into a native queue, leaving an uninitialized symbol unbound; no automatic startup drain is implemented yet. A non-list queue uses ordinary `defvar` initialization rather than GNU's reset/set machinery. |


GNU 32.0.50 is the behavioural oracle except for decisions explicitly recorded here. An entry changes an exact expected answer, not whether a test runs or how strictly it compares values.

## VM2-MD2 — Binary-buffer match-data uses integer positions

GNU `Fmatch_data` returns markers by default after both multibyte and unibyte
buffer searches. VM2 returns markers for Unicode text buffers, but integer
positions for binary byte buffers even when INTEGERS is nil. Its existing
marker/anchor substrate belongs to Unicode text buffers; pretending binary
positions were live edit-tracking markers would be false. With INTEGERS
non-nil, both buffer kinds return integer positions and append their buffer,
as GNU does. Search registers, `match-beginning`, `match-end`, and
`match-string` remain usable for binary buffers.

**Observable by a package: yes.** Fresh isolated byte-compiled GNU 32.0.50
versus compiled VM2, executable revision
`b28750b822dc45ffdceda3ea44a3c8b93f4e5a6b`:

```elisp
(with-temp-buffer
  (set-buffer-multibyte nil) (insert "ab") (goto-char 1)
  (looking-at "ab") (integerp (car (match-data))))
;; GNU => nil
;; VM2 => t
```

The integer choice is deliberate and bounded to the binary storage edge.
It has a compiled red equality assertion and a reversal mutant, rather than
silently failing through the text-only marker adapter.

## VM2-MD1 — Cyclic match-data REUSE is rejected

GNU `src/search.c:Fmatch_data` walks REUSE through `XCDR` without a cycle
check. In a fresh isolated GNU 32.0.50 byte-compiled lambda, the following
exceeded an external two-second deadline; the subprocess was killed and
reaped. The C loop has no progress condition for this circular tail.

```elisp
(progn (string-match "a" "a")
       (let ((r (list 0))) (setcdr r r) (match-data nil r)))
```

VM2 instead signals `(error "Circular match-data reuse list")`.
**Observable by a package: yes.** This keeps malformed optional reuse data
from wedging the agent entry point; it does not change finite proper-list
reuse, extension, or marker reseating. The bounded result has its own test
and a mutant that silently accepts the cycle. This is deadline/safety evidence,
not a speed comparison.

## VM2-SY1 — Regexp-host syntax tables, not general char-tables

GNU `src/syntax.c` stores complete syntax descriptors in mutable char-tables;
`aref`, `aset`, parent manipulation, printer traversal, and parser flags can
observe those descriptors. VM2's native syntax tables currently retain only
regexp syntax classes and standard-table inheritance. `syntax-table`,
`standard-syntax-table`, `copy-syntax-table`, `set-syntax-table`, and
`modify-syntax-entry` are provided to configure regexp matching; they do not
claim the general char-table or syntax-parser protocols. They print as opaque
`#<syntax-table>` objects. Non-class descriptor flags do not affect this
regexp-only host, and generic array access rejects the table.

**Observable by a package: yes.** Fresh isolated byte-compiled GNU 32.0.50,
revision `b28750b822dc45ffdceda3ea44a3c8b93f4e5a6b`, compared with compiled VM2:

```elisp
(car (aref (copy-syntax-table) 45))
;; GNU => 3
;; VM2 condition-case => (signal (wrong-type-argument integerp 45))
```

This is a bounded host prerequisite for the five regexp primitives, not a
claim to finish the syntax-table subsystem. Generic array-access diagnostics
inherit VM2's existing primitive error adapter. See `REGEXP.md` for supported
syntax-class and syntax-property matching. RX1's matcher-overflow-budget
behaviour is inherited unchanged from `romacs-regex-engine` and remains in
the root ledger.

## D001 — Unicode scalar string mutation

**Deliberate Romacs decision:** `aset` on a Unicode string permits replacement of any Unicode scalar by any Unicode scalar, even when UTF-8 width changes. Byte strings remain octet arrays: only integers 0–255 may be stored. Invalid scalars, octets and indices still signal.

GNU's storage restriction is not reproduced in the one-model UTF-8 representation. In particular, replacing β with the identical β is meaningful and safe, but GNU rejects it. The restriction concerns representation, not the meaning of indexed scalar replacement.

Live GNU 32.0.50 transcript (`copy-sequence` before each mutation; result is `aref` at the changed index):

```text
("abc" 1 98)       => 98
("abc" 1 955)      => error "Attempt to store non-byte value into unibyte string"
("αβγ" 1 946)      => error "Attempt to store non-ASCII char into multibyte string"
("αβγ" 1 955)      => error "Attempt to store non-ASCII char into multibyte string"
("αβγ" 1 97)       => error "Attempt to replace non-ASCII char in multibyte string"
("aβc" 0 955)      => error "Attempt to store non-ASCII char into multibyte string"
("aβc" 0 98)       => 98
("αβγ" 1 128512)   => error "Attempt to store non-ASCII char into multibyte string"
("αβγ" 1 256)      => error "Attempt to store non-ASCII char into multibyte string"
```

The strict source gate retains the GNU answer and records D001 alongside the exact romacs expectation for its scalar-mutation case. `tests/s2.rs` references this entry for the frozen historical condition-case case: GNU returns 18; romacs returns 955. These are intentional answers, not tolerated mismatches.

## D002 — Formatting raw high octets with Unicode text

VM2 text holds Unicode scalars; byte strings hold octets. `format` preserves directly interpolated high octets as bytes when the rest of the output is ASCII. It does not decode even valid UTF-8 in those byte strings. `%S` and nested byte strings use GNU's octal escapes. ASCII-only output remains VM2 canonical Unicode text (so its multibyte predicate differs from GNU's unibyte ASCII result).

When output would combine non-ASCII Unicode text with raw high octets, VM2 signals `format cannot mix raw high octets with Unicode text`. GNU instead creates non-Unicode raw-byte pseudocharacters, which are outside VM2's Unicode-scalar text model. Explicitly decode bytes before mixing them with Unicode text; do not silently replace, Latin-1-map, or UTF-8-decode them inside `format`.

Live GNU 32.0.50 (`lane_emacs --batch`, isolated HOME, `-Q`):

```elisp
(string-to-list (format "%s" (unibyte-string 128 195 169 255)))
;; => (128 195 169 255); multibyte-string-p => nil
(string-to-list (format "λ%s" (unibyte-string 128 195 169 255)))
;; => (955 4194176 4194243 4194217 4194303)
(format "%S" (unibyte-string 128 195 169 255))
;; => "\"\\200\\303\\251\\377\""
```

The generated-temporary compile-time rejection is **not** a deliberate divergence: escaped generated reads and writes now use their existing uninterned global slot, resolving unbound reads at execution time, like GNU. Generated identities still never enter the intern table.

## D003 — `not` retains the primitive `null` value

VM2 initializes `not` with the same primitive callable value as `null`, not a symbolic function-cell alias. Redefining `null` therefore leaves `not` unchanged. Ordinary named calls still observe their own live function cells; no primitive is made globally un-redefinable. Explicit `(defalias 'not 'null)` opts into symbolic alias semantics.

**Correction (2026-10-18): the original transcript below compared VM2's compiled
path against GNU's *interpreted* path, which overstated this divergence.** GNU's
byte-compiler open-codes `(not x)` into the dedicated `Bnot` bytecode — the
compiled body of `(defun probe-not (x) (not x))` is `"?\207"`, that is `Bnot`
(63) then `Breturn` — and that opcode never consults a function cell. So in
compiled code, which is the only regime VM2 has, **GNU behaves exactly as VM2
does.** Measured on live isolated GNU 32.0.50:

```elisp
(defun probe-not (x) (not x)) (byte-compile 'probe-not)
(defalias 'null (lambda (x) 'nn))
(not nil)         ; GNU interpreted => nn   VM2 => t
(probe-not nil)   ; GNU compiled    => t    VM2 => t   ← same
```

The residual difference is therefore against GNU's interpreter, not against
compiled Emacs. Note also that VM2 is the stricter of the two here: our
traversal kernels re-check the function cell on every call and fall back to
generic dispatch when it changes, whereas `Bnot` is hardwired into GNU's
instruction set and can never be invalidated.

The remaining smell is ours and is not about behaviour: seeding `not` is a
hand-written special case standing in for a general mechanism we lack — alias
resolution during lowering with dependency tracking on the traversed cells.
With that, `not` could be a genuine alias, still receive a kernel, and be
invalidated by the existing version guard, removing the exception entirely. The
exact difference is covered by `tests/traversal.rs`.

Live isolated GNU 32.0.50 (`--batch -Q`, throwaway HOME), interpreted path:

```elisp
(let ((old (symbol-function 'null)))
  (unwind-protect
      (progn (defalias 'null (lambda (x) 'nn))
             (list (null nil) (not nil)))
    (fset 'null old)))
;; GNU interpreted => (nn nn)
;; VM2             => (nn t)
```

## D005 — Reader raw-byte/Unicode mixtures (resolved)

The integer-character reader now preserves raw high octets as BYTE8 codes in
mixed multibyte literals, through the canonical 22-bit string codec. Pure raw
literals remain unibyte. The banked reader corpus covers raw, mixed, surrogate
and extended character inputs. Text buffers hold raw bytes too, as placeholder
characters listed per text body (DIVERGENCES.md VM2-RAWTEXT1); file-source
decoding retains its separate boundary.

The host Rust-str compiler adapter retains its pre-existing multibyte-ASCII
literal policy during migration. Lisp `read`/`read-from-string` use GNU's
content-based literal representation. Non-Unicode symbol names are refused.

### Reader/evaluator compatibility gaps, not deliberate completion claims

S106 does not implement named character escapes `\\N{...}`, GNU's non-scalar
Unicode/surrogate string representation, exact reader condition payloads or
error offsets, declaration property installation (compiler-macro, indent,
pure, etc.), backquote vector/string *splice operands* in non-final list
positions, or the other GNU Lisp dependencies. Declaration forms are consumed
as metadata; this is not a claim to execute their annotation handlers. These
remain compatibility work, not permission to change the live oracle answers.
See [READER-GNU.md](READER-GNU.md) for measured accepted-form counts.

## D004 — Unicode `%S` uses JSON control-character escapes

Pre-existing limitation, now explicit: Unicode strings printed through `%S`
use JSON-style escapes (`\n`, `\u0001`, `\u001b`) where GNU's default Lisp
printer emits raw control characters. Quotes and backslashes remain escaped.
The result reads back to the same Unicode string; this is a printed-spelling
mismatch, not data loss. The byte-string `%S` path already uses Lisp escapes.
A follow-up may unify Unicode output on Lisp printer rules.

Live GNU 32.0.50, isolated `lane_emacs --batch -Q`:

```elisp
(list
 (let ((s (format "%S" "a\nb"))) (list (length s) (aref s 2)))
 (let ((s (format "%S" "é\1"))) (list (length s) (aref s 2))))
;; GNU => ((5 10) (4 1))
;; VM2 => ((6 92) (9 92))
```

Code points 10/1 are raw newline/control-A; 92 is a backslash introducing
an escape. The transcript records the current difference; it does not claim
GNU printer compatibility.

# Deliberate annotation differences from GNU Emacs

The annotation layer implements per-key runs, raw presence/value lookup,
category/alias/default resolution, insertion stickiness and property-bearing
kill/yank strings. Markers and the overlay interval index are described in
ANCHORS.md. Provider extents, dependent invalidation and a Lisp
`object-intervals` entry point remain later milestones. The first two rules below
are **planned contracts, not implemented-feature claims**. They do not authorize
ignoring other GNU mismatches.

## Provider property writes do not advance the modified tick (planned)

GNU advances `buffer-modified-tick` even under `with-silent-modifications`:

```
silent-property-tick-delta=1
```

Provider-origin writes will not advance it. Redisplay must not invalidate
unrelated tick-keyed caches. Authored writes retain ordinary mutation semantics.
The storage checkpoint has one origin and does not yet implement this rule.

## Dependent provider coverage is invalidated immediately (planned)

The probe fontifies two function definitions, inserts an opening string quote at
the start of the buffer and reads the later definition without redisplay:

```
before-context-edit=(t font-lock-keyword-face)
immediate-dependent-state=(t font-lock-keyword-face)
```

GNU retains both coverage and a stale keyword face until contextual processing.
Romacs will immediately clear dependent coverage; display must recompute before
using it. Old values remain queryable until recomputation. The contextual timer
variables will be no-ops. This requires provider read hulls and syntax state; the
raw storage layer does not claim to implement invalidation.

## Identical overlays use a deterministic final tie-break

```
identical-overlays-newer-winner=B
```

At equal priority, start and end, Romacs chooses the newer overlay identity. The
rule survives interval-tree traversal order and moving GC. GNU resolves the
final tie by object address: passing this form directly to `--eval` returned 3,
whereas `(prin1 (eval (read SOURCE) t))` returned 1 on the review host:

```elisp
(with-temp-buffer (insert "abc")
  (let ((a (make-overlay 1 4)) (b (make-overlay 1 4)) (c (make-overlay 1 4)))
    (overlay-put a 'face 1) (overlay-put b 'face 2) (overlay-put c 'face 3)
    (get-char-property 2 'face)))
```

An allocator address is not a compatibility oracle. This case is excluded from
live-GNU equality replay and retained as a Romacs-only determinism/compaction test.
GNU's implementation order is not the portable contract. Overlay endpoints belong to the shared marker/overlay anchor store,
never to text-property runs.

## Buffer destruction and temporary-buffer limits

Explicit `kill-buffer` runs query functions and kill hooks with the target
current, unwinds current-buffer/point on veto or error, and destroys native
registry identity, buffer-local storage, windows, markers and overlays on
success. The buffer object remains a dead `bufferp` identity across collection.
`tests/kill_buffer.rs` covers those paths, including collection inside callbacks.

This is not complete GNU buffer lifecycle parity. The current `with-temp-buffer`
expansion marks its temporary buffer dead and removes its name on exit, but uses
an internal cleanup path without query functions, kill hooks, window replacement,
or attachment and local-table destruction. Anonymous internal buffers have no
registered name and are absent from `buffer-list`.
`buffer-list` uses creation-slot order (not GNU MRU/frame order) and refuses a
non-nil FRAME. Window replacement chooses the earliest surviving visible named
buffer outside minibuffer windows, or creates a named scratch buffer; it does
not restore per-window history.
Killing the GUI minibuffer is refused to protect its echo storage. There is no
process termination, indirect-buffer cascade, autosave deletion, lock handling,
`buffer-list-update-hook`, or inhibit-buffer-hooks creation argument yet.
Unibyte excursion restoration preserves the numeric point because its marker
substrate is not implemented; Unicode restoration uses a moving marker.

For example GNU Emacs 32.0.50 prints `[nil nil nil nil]` below, but VM2's current
temporary-buffer expansion retains the attachments until explicitly killed:

```elisp
(let ((m (with-temp-buffer (insert "abc") (copy-marker 2)))
      (o (with-temp-buffer (insert "abc") (make-overlay 1 3))))
  (vector (marker-position m) (marker-buffer m) (overlay-start o) (overlay-end o)))
```

## Interval history is not reproduced

```
same-set-next-boundary=3 intervals=((0 1 (p 1 q 2)) (1 2 (q 2 p 1)) (2 4 nil))
```

GNU retains a storage split at byte 1 (Lisp position 2), even though semantic
`next-property-change` skips it and returns Lisp position 3. Romacs' boundary
query similarly compares presence and `eq` values, ignoring insertion stamps.
An eventual `object-intervals` entry point will report the merged semantic view,
not this split history. The quantified difference for this four-character row
is two semantic intervals instead of three historical intervals. Present-key
order remains descending insertion stamp; replacements keep their stamp.

## Ordinary M1 semantics, not additional divergences

Replay the checked-in GNU answers with a private Emacs home:

```
python3 crates/romacs-vm/tools/annotation_m1_oracle.py --oracle emacs
```

The runner creates a private HOME/XDG tree even when invoked from a normal
shell. `tests/annotations.rs` replays the same forms in the VM, including raw
property lists and callback parameter identity. GNU observations include:

```
resolve-nil=(default default nil)
resolve-(p nil)=(nil nil (p nil))
resolve-(q alias)=(alias alias (q alias))
resolve-(category cat)=(category category (category cat))
category-nil=alias
alias-not-recursive=nil
kill-ring=#("xy" 0 2 (secret t p value))
yank-props=(p value)
handler-param="ALT"
noexclude=(rear-nonsticky t p v yank-handler (nil nil t))
exclude-all=nil
```

A directly present nil masks category/aliases/defaults; nil from a category or
an alias does not. Aliases are not recursively expanded. Defaults are resolved
per key without being stored as raw properties. Plain `insert` grafts string
properties but does not inherit neighbours. `insert-and-inherit` keeps inherited
keys, even nil, over conflicting string values; missing string keys are prepended
in a plist traversal order GNU also produces, though not necessarily the one GNU
produces for this history (see "Raw plist order after multi-key inheritance"
below). Rear/front stickiness is per property, including
`text-property-default-nonsticky` overrides.

Kill-ring strings retain the original properties. Exclusion happens in the
destination after each yank-handler invocation; NOEXCLUDE bypasses it. FUNCTION,
PARAM, UNDO and COMMAND fields are honoured; the function's return is ignored.
Handler runs are processed incrementally so a callback can mutate the unprocessed
suffix. Calls use ordinary VM function/closure dispatch, not reentrant native
execution or macro replacement, and observe function redefinition and GC.

Default and custom `yank-handled-properties` callbacks run over property-value
runs before exclusion. The defaults materialize `font-lock-face` into `face`
when font locking is absent, and materialize category plists before `category`
is removed. NOEXCLUDE skips both handling and exclusion. Temporary buffers start
without a buffer-local font-lock configuration, rather than inheriting another
buffer's dynamic value.

This does not claim a complete interactive command loop, clipboard integration,
undo engine, mark handling or yank-pop. Those require their owning editor layers.
`get-char-property` consults the overlay index before resolving text properties
through category, alias and default fallbacks. None of the remaining boundaries
is an additional licensed GNU divergence.

## Raw plist order after multi-key inheritance

`insert-and-inherit` from a neighbour carrying two or more value keys can leave
the inherited character's raw property list in a different ORDER from the one
GNU produces for the same history. Values are identical; only the plist
traversal order differs, and `equal-including-properties` returns t.

The investigation that settled this is worth keeping, because the obvious
reading -- "VM2 is wrong, GNU is right" -- is not what the oracle says.

GNU is deterministic for a FIXED history: padding, no-op edits and copying do
not change its order. But EQUIVALENT histories produce every possible order.
For two of the three measured rows GNU produced six of six possible orders, and
for the third two of two, depending only on how the identical final string was
built. Five public routes to the same string give three different orders, and
`equal-including-properties` returns t across all of them. VM2's output for
each row is an order GNU itself produces for an equivalent history.

GNU's actual rule is that an inherited character copies its SOURCE NEIGHBOUR's
plist order. Matching it exactly is a cheap optional follow-up, not a
correctness fix.

Exposure, measured rather than assumed:
  - GNU's own Lisp contains no order-sensitive comparison of raw plists.
  - Neither do the seventeen installed ELPA packages.
  - GNU's test suites do compare raw plists in five places (term, ansi-color,
    visual-wrap, and fns `object-intervals`), but none reaches them through
    multi-key inheritance.
  - The one idiom that could observe it is
    `(equal (text-properties-at (1- p)) (text-properties-at p))`,
    which is t in GNU and can be nil here. No such code was found.

Detection: a 6,000-case matrix over seeds 53 and 54 found nothing, because it
only ever placed ONE value key on a neighbour. Adding neighbours with two or
more value keys produced 8 order-only differences in 2,000 cases, all from
`insert-and-inherit`, none differing in value. A sweep that cannot produce the
shape cannot clear it.
## Overlay enumeration order is not GNU tree order

Live GNU Emacs 32.0.50, isolated with `lane_emacs` (`env -i`, `-Q`, throwaway HOME), returns the following. These forms reverse the returned list while collecting IDs; the distinction affects both enumeration APIs, not property-priority selection. Romacs order is deterministic for the same operation history, but is not GNU order.

```elisp
(with-temp-buffer (insert "abcdefgh") (overlay-put (make-overlay 2 5) (quote id) 0) (overlay-put (make-overlay 2 5) (quote id) 1) (overlay-put (make-overlay 1 6) (quote id) 2) (overlay-put (make-overlay 3 4) (quote id) 3) (overlay-put (make-overlay 2 5) (quote id) 4) (let ((l (overlays-at 3)) (r nil)) (while l (setq r (cons (overlay-get (car l) (quote id)) r)) (setq l (cdr l))) r))
```

```text
GNU: (3 0 1 4 2)
Romacs: (3 4 1 0 2)
```

```elisp
(with-temp-buffer (insert "abcdefgh") (overlay-put (make-overlay 2 5) (quote id) 0) (overlay-put (make-overlay 2 5) (quote id) 1) (overlay-put (make-overlay 1 6) (quote id) 2) (overlay-put (make-overlay 3 4) (quote id) 3) (overlay-put (make-overlay 2 5) (quote id) 4) (let ((l (overlays-in 1 9)) (r nil)) (while l (setq r (cons (overlay-get (car l) (quote id)) r)) (setq l (cdr l))) r))
```

```text
GNU: (3 0 1 4 2)
Romacs: (3 4 1 0 2)
```

`overlays-at` with SORTED, character properties and overlay faces order
overlays by GNU's `compare_overlays` (buffer.c), which is not transitive, so
its input is GNU's too: ascending start, equal starts latest inserted first
(itree_insert_node; `move-overlay` reinserts when the start moves), merged
as glibc's qsort does (`src/overlay_order.rs`). An edit that brings two
overlay starts together keeps Romacs's insertion order where GNU keeps its
tree's. Where two overlays tie on priority, secondary priority and range,
GNU orders them by object address and Romacs by heap index; both follow
creation order for overlays made in sequence
(`crates/romacs-vm-gui/tests/gnu_arity_fixes.rs` against pinned GNU).

## Reversed overlays-in bounds are swapped

GNU returns no overlays for reversed arguments; Romacs swaps the endpoints. The last two vector entries below expose this existing difference. Live isolated GNU Emacs 32.0.50 transcript:

```elisp
(with-temp-buffer (insert "abcdefgh") (overlay-put (make-overlay 1 9) (quote id) 0) (overlay-put (make-overlay 9 9) (quote id) 1) (overlay-put (make-overlay 5 5) (quote id) 2) (vector (length (overlays-in 9 9)) (length (overlays-in 8 9)) (length (overlays-in 1 9)) (length (overlays-in 5 5)) (length (overlays-in 9 5)) (length (overlays-in 5 1))))
```

```text
GNU: [1 2 3 2 0 0]
Romacs: [1 2 3 2 3 1]
```

## Structured marker and overlay results are opaque type tags

The agent `eval` entry point serializes markers as `{"type":"marker"}` and
overlays as `{"type":"overlay"}`, without signaling. This follows the existing
bare `buffer`, `byte-buffer`, and `function` convention for known, unreadable,
identity-bearing values. Unknown tag classes still signal an unsupported result.

GNU prints these live objects as `#<marker at 1 in foo>`,
`#<overlay from 1 to 5 in foo>`, and `#<buffer foo>` respectively; it returns
values, not errors. The structured protocol deliberately omits position, buffer,
and identity payloads. Moving mark/compact GC relocates Value indices, so no
index is exposed as a reusable reference. Reading positions here would also
couple serialization to anchor state. This is an extensible protocol choice:
additional object fields can be specified later without removing existing data.

## An overlays-in end beyond point-max errors

GNU clamps an end beyond the full buffer extent; Romacs rejects it with `index out of range`. This error is not silent truncation. After the deletion below, point-max is 8, while the query ends at 10. Live isolated GNU Emacs 32.0.50 transcript:

```elisp
(with-temp-buffer (insert "abcdefghi") (let ((o (make-overlay 2 8))) (overlay-put o (quote evaporate) t) (remove-overlays 4 6) (delete-region 2 4) (vector (overlay-start o) (length (overlays-in 1 10)))))
```

```text
GNU: [4 1]
Romacs: vm2: index out of range
```

## H1 — Indirect `maphash` on a non-empty table signals

GNU `Fmaphash` (`src/fns.c`) calls FUNCTION through `calln` for every used
slot, however `maphash` itself was reached. VM2 primitives cannot call Lisp, so
a direct `(maphash F TABLE)` and `(funcall #'maphash F TABLE)` are compiled into
an inline walk of GNU's slot order (`DOHASH_SAFE`). Only a truly indirect call
(`apply`, or `funcall` of a computed function value) reaches the `maphash` subr:
with an empty table it returns nil, as GNU does; with entries it signals
`(error "VM2 cannot call maphash indirectly on a non-empty table" FUNCTION)`
instead of silently skipping. Arity errors keep GNU's
`(wrong-number-of-arguments #<subr maphash> N)` shape.

**Observable:** yes, loudly, only through `apply`/computed `funcall` of `maphash`.

```text
GNU 32.0.50 (byte-compiled):
(let ((h (make-hash-table)) (r nil)) (puthash 'a 1 h) (apply #'maphash (list (lambda (k v) (push k r)) h)) r)
  => (a)
VM2 (pinned in tests/hash_tables.rs):
(let ((h (make-hash-table))) (puthash 'a 1 h) (condition-case e (apply #'maphash (list #'ignore h)) (error e)))
  => (error "VM2 cannot call maphash indirectly on a non-empty table" ignore)
```

## H2 — User-defined hash table tests are refused

GNU `get_hash_table_user_test` accepts any symbol whose `hash-table-test`
property is a two-element list installed by `define-hash-table-test`, then calls
the user's comparison and hash functions during lookup. VM2 records the property
exactly as GNU does (`define-hash-table-test` returns `(TEST HASH)`), but
`make-hash-table` with such a test signals
`(error "VM2 does not support user-defined hash table tests" NAME)`. A symbol
without a valid property still gets GNU's `(error "Invalid hash table test" NAME)`.
Reason: lookup would have to call Lisp from inside a primitive (see H1).

**Observable:** yes, loudly, at table creation.

```text
GNU: (progn (define-hash-table-test 'my-test #'eq #'sxhash-eq) (make-hash-table :test 'my-test))
  => #s(hash-table test my-test)
VM2: (error "VM2 does not support user-defined hash table tests" my-test)
```

## H3 — `equal` on closures requires the same compiled lambda

GNU `internal_equal` compares `PVEC_CLOSURE` objects element-wise (bytecode,
constants, captured values), so textually identical lambdas from different
sites are `equal`, and `sxhash-equal` hashes them as vectors. VM2 closures are a
function identity plus captures: `equal` holds when the function is the same
compiled lambda and the captures are `equal`; otherwise nil. Closures from one
site with equal captures are found in an `equal` table, as in GNU.

**Observable:** yes, silently, for `equal` keys that are closures from different
lambda sites. Pinned in `tests/hash_tables.rs::divergences_are_loud_and_pinned`.

```text
GNU: (let ((h (make-hash-table :test 'equal))) (puthash (lambda (x) x) 1 h) (gethash (lambda (x) x) h))
  => 1
VM2: nil
```

## H4 — Weak-entry removal frees slots in slot order

GNU `sweep_weak_table` pushes freed slots onto the free list in bucket order,
which depends on object addresses. VM2 frees them in ascending slot order, so
the next insertions reuse the highest freed slot first. Iteration order after a
collection removed weak entries can therefore differ. GNU's own order there
varies with addresses and collection timing, so no package can depend on it.

**Observable:** only through `maphash` order after a collection removed weak
entries; not reproducible in GNU either.

## H5 — Index-hashed keys are re-hashed after compaction

GNU hashes `eq`/`eql` keys and `equal` keys containing identity-compared objects
(buffers, closures, hash tables, markers' buffers) by address and never
re-hashes them. VM2's collector moves objects, so such entries are re-hashed
from their current contents after a compaction moved them. A key mutated after
insertion (undefined behaviour in GNU, whose lookups then miss) may become
findable again under its new contents after a collection, if its hash also read
a moving object. Content-only keys (numbers, symbols, strings and trees of
them) are never re-hashed and behave exactly like GNU when mutated:

```text
GNU and VM2: (let ((h (make-hash-table :test 'equal)) (k (list 1 2))) (puthash k 'v h) (setcar k 9)
               (list (gethash k h) (gethash (list 1 2) h) (gethash (list 9 2) h)))
  => (nil nil nil)
```

**Observable:** only after mutating a key, which GNU documents as undefined.

## H6 — `hash-table-keys`/`hash-table-values` are always defined

In GNU these are `defsubst`s in `subr-x.el`, unbound until `(require 'subr-x)`.
VM2 provides them natively with GNU's result order (reverse slot order, from
`push` inside `maphash`). `hash-table-contains-p` is native in both.

**Observable:** calling them without requiring `subr-x` works instead of
signalling `void-function`.

```text
GNU -Q: (let ((h (make-hash-table))) (puthash 1 2 h) (hash-table-keys h)) => (void-function hash-table-keys)
VM2:    (1)
```

## H7 — `:size` is not preallocated

GNU allocates `2 * size` key/value slots plus hash, next and index vectors
eagerly, and `:size` 2^30 or more fails with "Hash table too large" only after
attempting that allocation (2^30 - 1 exhausted the measurement's 60 s budget).
VM2 records `size` exactly (`hash-table-size` and growth are identical) but
materializes slots on first use, and signals `(error "Hash table too large")`
for 2^30 and above without allocating.

**Observable:** only as memory and time.


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

## Observable by a package — Character-set motion

| # | Mechanism | GNU | Romacs |
|---|---|---|---|
| ROMACS-SKIP-CLASSES | `skip-chars-forward` / `skip-chars-backward` named classes | Accepts `[:class:]` sets | Signals an explicit error before moving point; literal/range/escape/complement sets are supported |

`skip-chars-forward` and `skip-chars-backward` implement literal characters,
ranges, escapes and complement sets, including Unicode, byte-buffer octets,
marker limits and narrowing. GNU's `[:class:]` syntax is not implemented;
these sets signal an error before moving point instead of approximating
syntax-table-dependent classification. `tests/skip_chars.rs` preserves that
explicit refusal. This subset supports GNU's unchanged `mail-header-extract`;
it does not establish full GNU mail-library or MIME-encoding compatibility.

## VM2-SQLITE1 — Bundled SQLite library

sqlite.c's primitives run on SQLite 3.53.2, compiled from the amalgamation
bundled with `libsqlite3-sys` 0.38, so no system library is needed. GNU pin
`45b0f7699df36b3849edc3bdef37d4cb9e75b9ea` links the host's library (3.54.0
on the reference host). `sqlite-version` and SQLite's own error messages
follow the bundled version. Text columns are decoded by the native `utf-8`
decoder with line-end detection, as GNU's `utf-8` coding system decodes
them, without looking `utf-8` up: a Lisp redefinition of that coding system
does not affect rows.

## VM2-WIDTH1 — String widths without compositions

`char-width` and `string-width` follow character.c's `char_width` over
`char-width-table`, `tab-width`, `ctl-arrow` and the buffer or standard
display table. `string-width` does not apply automatic compositions from
`composition-function-table`: each character is measured on its own, so a
sequence GNU composes is counted per character. In pinned GNU batch that
differs for a character followed by an emoji modifier or variation
selector: 👍🏽 and ❤️ are 2 columns in GNU, 4 and 1 here. Text with a `composition` property signals an
explicit not-implemented error, since VM2 has no composition table.
