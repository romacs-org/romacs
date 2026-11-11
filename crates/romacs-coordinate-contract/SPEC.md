# VM2 character/byte coordinate contract

## Scope and status

**Acceptance seam for the new text owner, not a second text implementation.** VM2 at `2a7b3bdc64a7076352b8f737427513304f26d25b` has no buffer/text state or dependency. The text owner must implement the traits in `src/lib.rs`, then call the generic suites from its own tests. `tests/legacy_probe.rs` is minimal baseline/conformance scaffolding only; it is not a VM2 dependency.

The target includes Greek, English and Cyrillic as normal text, plus GNU internal characters outside Unicode. ASCII must remain a direct arithmetic path. Character/byte compatibility and mixed-script speed are both acceptance requirements.

## Semantic boundary

- Public Elisp positions and slicing use **characters**. Buffers use one-based positions; strings use zero-based indexes. Internal trait positions are zero-based, absolute and may equal the text length. Adapters alone translate the one-based buffer boundary.
- Characters are **integer codes**, not encoded bytes or opaque scalar objects. `aref`, `char-after`, comparisons and arithmetic must agree. Valid GNU internal forms include raw-byte characters and extended five-byte characters; Rust `str` alone is not the encoding contract.
- Multibyte mode counts character heads; unibyte mode counts every byte, including bytes with the high bit set. Mode is observable. Mode reinterpretation changes summaries and invalidates correspondences even when storage bytes do not change. Mode conversion (e.g. `string-as-unibyte`) is a separate operation, not silently equivalent to reinterpretation.
- Every returned byte offset is a character boundary. Interior multibyte byte inputs are rejected by this trait with `InvalidPosition`, never rounded. Elisp `byte-to-position` instead maps an interior byte to its containing character and returns nil outside the full byte-position range; its adapter must implement that policy rather than blindly expose this lower-level rejection.
- Restriction changes accessible **absolute character** bounds, not the text's origin. Index queries remain absolute, including outside a restriction. Buffer operations enforce accessible positions at the buffer owner; `narrow-to-region` is explicitly allowed to expand to any valid full-text bounds. Narrow/widen/save-restriction do not mutate or rebuild the coordinate index.
- Text properties are character-range metadata. Plain insertion used by the contract does not inherit properties; surviving spans retain their values and shift in character units. Property inheritance, overlays, stickiness and `insert-and-inherit` belong to the buffer/property owner, not the coordinate index. Whatever their policy, the same committed edit must update properties and coordinates atomically. Property-only edits do not invalidate byte/character mappings.

Executable assertions: `boundaries`, `edits`, `edit_invalidation`, `rejected_edits`, `reinterpretation`, `narrowing`, `editor_semantics` and `bounded_conversion_work` in `src/lib.rs`; `semantic_contract.el` checks the actual Elisp boundary. The first five suites execute through the baseline adapter; the restriction/editor/work-count generic suites compile but require the new owner's adapters before they can execute. Their Elisp semantic counterparts execute on GNU and the shipping baseline. Fixtures include Greek, English, Cyrillic, emoji, GNU raw-byte and five-byte forms. The last two are byte fixtures, not invalid Rust strings.

## Implemented VM2 alternative

VM2 now implements every reusable seam in `crates/romacs-vm/src/text.rs`.
See `crates/romacs-vm/TEXT.md` for ownership, limitations and commands. Its
relative-boundary AVL directory replaces the proposal below: bounded leaves
(128 characters / 512 bytes), counted ASCII runs, rank lookup in both units and
split/join maintenance on edits. Local prefix arrays mean zero query byte scans,
so no eight-anchor cache is needed. Actual query descent steps are instrumented
in test builds. Ordinary edits do not rebuild this directory or shift every
suffix correspondence. Contiguous byte/decoded storage still moves on middle
edits; that is explicitly storage cost, not hidden index rebuilding.

The lower-level GNU internal/raw-byte coordinate seam passes. The Elisp string
transport remains Unicode-scalar-only; full extended-character string exposure,
buffer lifecycle and other editor services are not proven by these seams. The
historical inherited-editor measurement commands below are not VM2 acceptance:
VM2's exact frozen Guile workload runner lives in `tools/text_bench.py` in its
crate and does not invoke the inherited evaluator.

## Proposed algorithm (historical alternative)

1. Give the text owner's existing logical blocks counted summaries `(bytes, characters)` and prefix/rank lookup by either unit. If that owner has no counted representation yet, add this responsibility **there**, not a competing text/property tree in this contract crate. Property intervals are not byte-storage summaries.
2. Bound text-index leaf scan windows to **512 logical bytes**, with boundaries adjusted to whole characters. A larger physical gap-buffer chunk can contain these small summary windows; physical storage size need not equal index granularity. Counted tree branching/representation is the text owner's choice.
3. Maintain up to **eight** remembered `(character, byte)` pairs per text object, plus natural start/end and gap pairs. Queries consider the closest above/below remembered pair and the located summary window boundaries, using equal byte/character differences to interpolate wholly ASCII spans. On a miss scan from the nearest useful anchor, never unconditionally from text origin. Cache the answer, replacing the nearest non-natural entry for a continuing local stream; preserve distinct distant hot anchors. Replacement policy is a performance choice, not a correctness requirement.
4. For wholly ASCII or unibyte text return the requested offset directly before touching cache/tree state. Repeated exact anchor hits must scan zero text bytes. Adjacent mixed-script motion must not rescan an entire leaf on each query.
5. Cache is owned by one text identity. Clone/snapshot either copies matching text and cache together or starts empty. Buffer switches, storage recycling, indirect buffers and asynchronous readers must never cross-contaminate identities/revisions. No unchecked interior mutability or public `Send`/`Sync` regression is justified by this contract.

Complexity with `M` summary windows, max window `B=512`, `K=8` cached pairs:

| Operation | Bound excluding existing text-storage movement |
|---|---|
| ASCII/unibyte conversion | O(1), zero byte scan |
| Exact cache hit | O(K), zero byte scan |
| Cold/random conversion | O(log M + B + K) |
| Adjacent cached conversion | O(K + character encoded width), at most five bytes per character |
| Replace | O(log M + changed windows + inserted bytes + K); storage gap movement remains its owner's cost |
| Narrow/widen | O(1), zero index maintenance |
| Reinterpret mode | O(text bytes), full summary rebuild |

These are **design obligations, not measured implementation claims**. Tests currently prove semantics and reject the slow inherited baseline through instruction counts; VM2 must expose test-only scanned-byte/tree-work metrics to establish the complexity bounds after implementation. No full-buffer index rebuild per ordinary edit is permitted. If the sibling's representation cannot support bounded windows incrementally, resolve that integration explicitly rather than disguising an O(n) invalidation rebuild as an efficient index.

## Edit invalidation/publication

For replacement of character span `[c0,c1)` and byte span `[b0,b1)` by `(ic,ib)` units:

- Before commit, conversions refer exclusively to the old revision. Validate both boundaries and insertion encoding before publishing mutations. Failed edits leave bytes, lengths, summaries, properties and anchors unchanged.
- Simplest safe policy: discard all eight remembered pairs for **every successful content edit**, then publish new counted summaries and new natural anchors together with bytes/properties. O(K) is bounded. Do not key invalidation only to modification ticks that Lisp can restore.
- A later optimization may preserve anchors strictly before the edit, shift anchors strictly after it by `(ic-(c1-c0), ib-(b1-b0))`, and discard anchors inside or on either ambiguous boundary. That optimization is not required here.
- Insert, delete, replace, bulk/empty load and mode reinterpretation all use the same publication owner. Equal character count does **not** mean unchanged byte offsets. Equal byte count does not mean unchanged character offsets.
- Gap movement alone preserves logical correspondences but must publish its new natural `(char,byte)` pair. Narrow/widen never invalidate text mappings; saved restrictions must be restored through edit-aware buffer markers, not stale raw cached bounds.

## Shipping performance acceptance

`workloads.json` contains the six exact source-derived lexical bytecode forms (the five named rows plus `char-after`), N and original mixed/ASCII readings, explicitly labelled historical. Each row runs mixed ASCII/Greek and an equal-character-count ASCII substitution. New source may use a different internal encoding but must return the same Elisp answer.

`measure.py` reuses the sweep acquisition protocol and the repository's PMU attribute owner. It admits only positive counts, **running == enabled exactly**, successful editor exits, one answer, identical GNU/romacs answers at N and 2N, and positive slopes exceeding the measured **36-instruction** resolution. It records `/proc/loadavg` per execution and asserts the exact loaded shipping image before work. Three interleaved pairs per engine and row; median slope/N includes loop/helper overhead. No empty-loop subtraction, time scaling or elapsed-time claim. Rejected attempts are retained, at most ten retries; they never become admitted evidence.

Build in a fresh worktree with the pinned Rust toolchain exported in that shell:

```sh
export PATH=/path/to/pinned-rust-1.96.1/bin:$PATH
unset ROMACS_RUNTIME_ROOT
cargo build --release -p romacs-vm-gui
python3 crates/romacs-coordinate-contract/measure.py --binary ABS_BINARY --binary-commit BUILD_COMMIT --image ABS_IMAGE --runtime ABS_RUNTIME --output ABS_READINGS
python3 crates/romacs-coordinate-contract/check_performance.py ABS_READINGS --ascii-baseline ABS_BEFORE
```

The `--runtime` argument above selects the runtime for the **measurement**, not a packaging argument to `fresh-build`. Every measurement must load its built image; fallback source boot invalidates it. Freeze/hash binary+image and retain commit/toolchain/build command in the receipt before comparison.

Mixed-script target: **at most 4.0x GNU on every row**, toward rather than worse than the observed 2.3–3.9x ASCII floor. ASCII: record all triplets and require no reproducible regression above baseline spread and the 36-instruction resolution. Overlapping triplets mean inconclusive, not an improvement. `check_performance.py` validates every accepted raw record and reconstructs slopes before enforcing the bar; it is intentionally RED on the inherited baseline.

The current release program is the inherited VM. These measurements do not claim VM2 supports the forms; once it does, build/measure its shipping path with equivalent loaded-runtime proof. Do not route a VM2 benchmark back through the old evaluator to manufacture coverage.

## Running the contract

```sh
cargo test -p romacs-vm --lib text::
emacs -Q --batch -l crates/romacs-coordinate-contract/semantic_contract.el
```

The contract is a library; the VM's text runs it (`crates/romacs-vm/src/text.rs`). Among those
tests, `the_contract_catches_a_stale_coordinate` wraps the text in `contract::Stale`, a cached
coordinate that edits leave stale, and must fail at the char-to-byte assertion: it checks the
suite, not the text.
