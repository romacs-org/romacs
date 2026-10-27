# romacs-mem (unwired S1–S5)

A dependency-free memory-core crate, registered as a workspace member but not
used by the inherited core or any inherited crate. No build script.

- S1: GNU LSB words, signed 62-bit fixnums, symbol constants, registered root
  and fixed-chunk stack storage, checked frames, pure-subr Args, isolate threads.
- S2: 64 KiB aligned pages from 4 MiB mappings, 4 KiB headers, allocation and
  injective per-size-class mark bitmaps (including 8 B floats), 25 vector classes,
  separate large mappings, a 30-bit symbol arena with 1,280 slots per page,
  allocator-only counters, two-empty-cycle payload release and chunk unmapping.

The virtual-memory backend currently supports **Linux with 4 KiB OS pages**.
Symbol storage reserves 54,975,594,496 virtual bytes, not resident memory; pages
are committed on demand. The builtin table currently contains only nil and t;
L2 will extend it. Heap construction panics if its initial reservation fails;
subsequent storage allocation reports `AllocError`.

## Boundaries

S3 adds initialized cons/float/string/symbol/vectorlike layouts, relaxed atomic
Slots, heap-borrowed typed views, owned boxed `Payload: Send + 'static`, and the
unsafe `raw::ForeignHooks`/`LegacyWeak` adapter contracts. `raw::RawValue` is
mutator-local (`!Send + !Sync`): native payloads cannot contain it, including in
collections, while legacy adapters can retain and report it. String bytes and payload
boxes are dropped eagerly during collection and at heap teardown.

S4 adds the strong closure of full stop-the-world collection. `Heap::safepoint` reports
`WantCycle` on allocation debt >= max(threshold, percentage * last live bytes).
`gc_event` also accepts explicit requests and idle >= one second with half-threshold
debt. The owner splits its heap and root view via safe field destructuring and calls
`collect`; allocation itself never collects. Root tables, stack prefixes, builtin
symbols, owner `Trace` edges and reachable legacy adapters feed one strong closure.
The sweep is eager. Interim symbols have sticky pin metadata separate from the
public flags/aux word; writes to value/function/plist pin even nil. Full ignores
pins and reclaims unreachable symbols. The transition uses `pin_symbol` at its
audited raw-ID stores; native constructors are reclaimable-born.

The `test-drivers` feature exposes unsafe zeroed storage reservation, explicit
storage marking and an unsafe storage-only sweep. Reserved words must never be
decoded with typed accessors, and the sweep refuses owned typed resources. That sweep **does not trace registered roots
or child references**. Default builds expose no such constructors. `verify`
adds frame birth checks that reject popped/regrown slots while retaining outer
frames when an inner frame is popped. Neither feature is enabled by default.

GNU counter order is `(conses floats vector-cells symbols string-chars intervals
strings)`. Layout constructors supply logical vector slot/string character
counts independently of rounded storage bytes. String character counts come
from UTF-8 validation in multibyte mode, or byte length in unibyte mode. External
bytes, native payload `bytes()` and legacy `bytes()` contribute to accounted live
bytes and allocation debt. Call `account_external` at each external resize; shrink
reduces live bytes but not debt. Each full records saturating debt-minus-threshold
overshoot, including the threshold-crossing allocation. This is measured, not an
a-priori bound on heap, allocator retention or RSS.

Every collection appends an unbounded event record with sequence, trigger, three
timings, before/after bytes and overshoot, plus exact online count/sum/max. Core-only
callers initialize all timings to core stop; an owner wrapper calls
`finish_gc_timings` to supply its elapsed and input-blocking intervals. The `verify`
feature separately traverses the graph before any destruction using independent
per-slot bitmaps and large-span bits, validates allocation before following an
edge, and panics with `Dangling` or `Lost` before sweep. S5 also verifies compacted
undo heads, pending finalizers and every surviving registered weak entry as strong.

S5 runs GNU's special-edge order: compact and mark each `Trace::undo_heads` head,
discover all unreachable finalizers before marking any doomed function graph,
iterate native/legacy weak containers to a fixpoint, remove dead entries, and
call `Trace::unchain_markers`. The owner must enumerate each shared undo state
once, skip live-buffer undo in its strong trace, and detach unmarked markers in
the marker hook; the hooks cannot borrow the heap or call Lisp. Dotted undo tails
and arbitrary marker-entry data are preserved. A killed buffer uses ordinary
strong tracing (its undo is normally already nil).

`make_weak_pairs` creates a fixed-capacity native slot array with occupancy bytes;
`WeakPairsRef::set/get` are plain stop-the-world accesses. All four GNU weakness
policies share one predicate with `LegacyWeak`. Only reachable weak containers
are registered, including ones discovered through retained weak entries.
`WeakRef` owns a non-rooting registry entry and a heap-bound `WeakId`; collection
clears dead entries before freeing cells, IDs never recycle, and dropping the
handle removes its entry. Upgrades cannot cross heaps.

`make_finalizer` owns one function slot. After `collect`, the owner's elapsed-time
wrapper must call `run_finalizers`; collection itself never invokes Lisp. Pending
finalizers remain roots until consumed. The runner dequeues and clears each
function before invoking its callback with a `Root`, so recursive collection or
a callback panic cannot run that finalizer twice. Callbacks run in creation order;
Lisp condition handling and post-GC hooks belong to the evaluator, not this crate.
An unwinding collection poisons further collection; discard that heap. Dead
resources are unregistered before destruction, so teardown cannot double-drop
an interrupted resource.

## In-place inherited cells (W1)

`Heap::for_legacy_owner` is an unsafe current-thread exception to isolate creation.
Install one `raw::ForeignHooks` owner before allocation. `alloc_foreign` returns
unbranded, non-rooting words; initialize owner fields immediately before any
other heap operation. Vectorlikes overlay a mem-owned 16-byte header: the owner
must preserve it and initializes only bytes [16, n). Safe vector views expose
`Other`, zero Lisp slots and no raw bytes/payload/code. Strings use dedicated
size classes (including large spans) and never yield a native `StringRef`.
Hooks enumerate strong edges, weak tables, external bytes and destruction.
Call `account_external` after initialization and external resizes; collection
also refreshes foreign accounting. Finalizer hooks exclude their function from
ordinary tracing; mem owns its registered/doomed lifetime and once-only delivery.
Hooks cannot allocate, invoke Lisp, retain callbacks or borrow across collection.
The bridge replaces the old per-object boxed legacy adapter; nothing is wired
into the machine in this step.

With `HeapConfig::external_symbols`, symbols are owner indices, not arena cells:
mem never follows/frees them, `make_symbol` errors and typed symbol views refuse
them. Weak liveness combines per-cycle side marks with the owner pin query.
Registered static address ranges are permanent leaves; all other unallocated
edges still panic with `Dangling edge`. Raw words and heaps remain thread-local.

Not implemented here: old-core adapter, native old-core floats, arena migration
of external symbols, pvec reconciliation, generations or editor integration.

## Eval-private closure code

Every `PvecType::Closure` constructor initializes one extra nil word after the
visible Lisp slot prefix and before any raw/payload tail. It uses the same
relaxed atomic `Slot` representation as other `RawValue` edges. `VectorRef::code`
and `set_code` are eval-private Rust accessors: the setter accepts only nil or a
live `PvecType::CodeBlock` belonging to the heap. GC traces this word, including
on native payload closures; it is not a root after the last owner disappears.
`len`, `get`, `set`, `raw_len` and `raw_bytes` exclude the word.

`Heap::make_code_block` copies an opaque eval-defined byte encoding into a
GC-managed immutable leaf with no Lisp slots, payload or Lisp edges. Eval must
encode its copied instructions, max depth, argdesc and switch target sets here;
mem neither verifies nor interprets that encoding. Other constructors reject
`CodeBlock`, preventing mutable/payload-backed lookalikes. First-call admission
and prototype-to-instance sharing are eval's responsibility: allocate once and
copy the prototype's `code()` into the instance's `set_code()`.

The logical cost is **8 bytes per closure**. Allocator classes round this: with
no raw tail, four visible slots grow from 48 to 64 bytes (+16), five remain 64
(+0), and six grow from 64 to 80 (+16). CodeBlock costs a 16-byte header plus
encoded bytes, rounded to a vector allocation class (or a large mapping), shared
by all closures referring to it. Visible vector-cell counters remain unchanged.

The unwired obj crate has no Lisp aref/length/printer/equal/copy-sequence APIs
yet; tests assert their visible-slot/raw-tail projection, not an integrated
Lisp printer or equality implementation. GNU Emacs 32.0.50 rejects
`(copy-sequence (make-byte-code 0 (unibyte-string 192 135) [42] 1))` with
`wrong-type-argument sequencep`; there is no closure copy-sequence path to extend.
Code-string mutation after admission (design divergence D1) belongs to eval,
not these storage APIs.

## Verification

Run the crate's tests through `python3 scripts/dev-loop romacs-mem`, or
directly:

```sh
cargo test -p romacs-mem --offline
cargo test -p romacs-mem --offline --features verify,test-drivers
cargo clippy -p romacs-mem --offline --all-targets --features verify,test-drivers -- -D warnings
```

The compile-contract harness builds a fresh default-feature copy of the crate,
checks three positive controls and eighteen negative cases with expected diagnostics,
including RawValue/Vec<RawValue>/Value/Root/ValueStack-bearing payloads, an unsafe raw-word round-trip control, borrowed payload state and typed
view escape. Layout tests cover exact sizes/offsets, slot mutation, raw tails,
large vectors, strings and exactly-once payload/legacy teardown. Allocation tests
cover all classes, adjacent live/dead float cells, arena page boundaries and
reuse, large objects, counters, actual kernel residency after `madvise`, header
preservation, class-independent page reuse and whole-chunk unmapping.

## Informational allocation benchmark

```sh
cargo run -p romacs-mem --offline --release --features test-drivers \
  --example alloc_bench -- 10000000
emacs -Q --batch -l crates/romacs-mem/bench/alloc-gnu.el
```

Rust measures **storage reservation only** for cons cells, string headers and
header-plus-two-slot vectors. GNU measures Lisp constructors, including a
16-character string's bytes and evaluator overhead. Both disable collection
inside each measured loop and verify allocation counters. These are reference
measurements, **not equivalent workloads or a Lisp/runtime speedup claim**.
The complete-collector benchmark is described below.

## Initialized small-object allocation benchmark

`examples/small_alloc_bench.rs` times actual cons, 8/16/32-byte unibyte string,
four-slot vector and symbol constructors at 100,000 and 1,000,000 objects;
`bench/small-alloc-gnu.el` is the byte-compiled GNU counterpart. Both warm up
once and fully collect before timing; GC-enabled rows use 800,000 bytes / 10%, while no-GC rows assert zero cycles.
Rust checks pacing every 1,000 discarded objects; GNU uses its bytecode safepoints.
Symbols reuse a prebuilt name. Timings include constructor initialization, string
backing storage and any spontaneous collections, but exclude startup and teardown.
GNU includes evaluator overhead and its preexisting live heap: these measure a
constructor budget, not a C-allocator or end-to-end editor speedup.

Strings' immutable backing capacity is the sole external-size authority; mutable
payloads/legacy objects retain their resize index. See [measured results and cost
splits](bench/small-alloc-results.md). The regression test guards exact string
accounting, reclamation/reuse and the absence of per-string tree insertions rather
than asserting wall time in a test.

## Preliminary strong-only collection benchmark

Run `cargo run -p romacs-mem --offline --release --example collect_bench` through
an admitted slot. Compare `emacs -Q --batch -l
crates/romacs-mem/bench/collect-gnu.el` through a light oracle slot. Both retain
4096 strings of 1024 bytes in two-slot vectors and a cons spine, and report the
median of seven full collections per live MiB. Rust measures core stop; GNU
includes its builtin runtime heap and primitive overhead. No weak tables, undo,
finalizers or editor integration are exercised: this is not full-path H1 evidence.

## Complete-collector benchmark and GNU differential

Through a light slot, run `cargo run -p romacs-mem --offline --release --example
complete_bench` and `emacs -Q --batch -l crates/romacs-mem/bench/complete-gnu.el`.
The strong graph matches the preliminary shape, augmented each cycle with two
48-entry key-weak tables, 16 marker undo entries and 16 doomed finalizers. The
Rust path uses one native and one legacy table and eagerly drops 16 boxed native
payloads; GNU uses two hash tables and temporary vectors. Both assert special
phase counts and report a seven-cycle median per accounted live MiB. Rust's
measurement is core stop; GNU includes callbacks, builtin runtime live bytes,
allocator differences and primitive overhead. This tests the complete shape
behind H1, not editor latency or an equivalent-runtime speedup.

The ordinary crate suite requires `emacs` for its differential: 28 undo cases
(head/middle/adjacent removals, non-fixnum data, dotted tail), plus all 16 GNU
weakness/reachability combinations, a mixed ephemeron chain, and mutually
referencing unreachable finalizers with a second-GC once-only control (48 rows).
It compares exact case inventories against both native and legacy behavior. Seeding helpers return before GNU collection
to avoid incidental conservative local roots. No unit test asserts wall time.
