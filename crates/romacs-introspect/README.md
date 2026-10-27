# romacs-introspect

Where every Lisp primitive is implemented, how it differs from GNU, and how
sure we are. Derived, never written down (R99); the first half of R100.

```text
$ romacs-introspect describe maphash          # one structured answer, JSON
$ romacs-introspect set core-only             # what blocks removing romacs-core
$ romacs-introspect set observable            # primitives with package-visible divergences
$ romacs-introspect set untested              # no scanned test names it
$ romacs-introspect rust builtin_maphash      # Rust fn -> Lisp names (also Primitive::X, file.rs:LINE)
$ romacs-introspect coverage | provenance
$ romacs-introspect --text describe string-trim
```

Exit status: 0 answered, **3 not found by the index**, 2 usage. Three is
deliberately not one: "not found" is a refusal, not a failure and not an
answer of absence.

The Rust API is the same surface: `Index::embedded().describe(name)` returns
`Lookup::Found(Described)` or `Lookup::NotFound(Absence)`; `query(&[SetQuery])`
intersects filters; `implements(rust)` is the reverse direction. Every answer
carries a `Provenance`. Lisp wiring is a later lane.

## Core retirement and the embedded snapshot

The current embedded registry is VM2-only: core has no entries, and no
core-only or shared-runtime answers should remain. VM2-only answers do not
claim core arity or core evaluator-handler spans; `if` is VM2 syntax, not a
funcall-able primitive in this registry.

`data/registry.json` is now freshly generated alongside introspect-live's identical
snapshot by `cargo run -p romacs-introspect-live -- --write`. Schema v2 records
a deterministic input SHA-256 and platform, never time/HEAD/cleanliness.
`--check` and the mandatory snapshot test compare full bytes of both copies.
Current freshness requires the same input hash and platform; legacy schema data
is readable but cannot establish Current. See [the live generator scope](../romacs-introspect-live/README.md)
for the exact framed hashing and input-selection contract. Historical counts and
core examples below describe the pre-retirement design, not today's population.

## Where each fact comes from

| fact | source | when |
|---|---|---|
| which names are registered, core arity, special form | **the runtimes**: `romacs-core` `Context::new()` enumerated with `mapatoms`/`subrp`/`subr-arity`; VM2 probed with `(funcall 'NAME)` in a fresh `Vm` per name | `romacs-introspect-live`; committed as `data/registry.json` |
| registration site, implementing fn, span | parse of `crates/romacs-core/src` and `crates/romacs-vm/src` | `build.rs`, every build whose inputs changed |
| divergence rows (GNU beside ours, observable) | the prose ledgers `DIVERGENCES.md` and `crates/*/GNU-DIVERGENCES.md`, parsed by a port of `tools/divergence_index.py` | `build.rs` |
| test mentions | `(name` / `#'name` in every scanned test file (8,223 today) | `build.rs` |
| Lisp definitions (for honest negatives) | `(defun NAME` and friends under `lisp/` | `build.rs` |
| commit, input cleanliness, time | `git`, scoped to the inputs above | `build.rs` |

## Why the registry is live and the spans are parsed

The prototype (`tools/primitive_index.py`) parsed Rust for the names and so
reached 1,054 of 1,405 `register_subr` call sites; it called `car` VM2-only and
`looking-at` not a primitive. The fix is a better source of truth, not a better
regex: the live core registry has **1,566** subrs, by construction. Source is
parsed only for *where*, and the two sides are cross-checked:

* a registered name with no recognised registration site is listed in
  `coverage.core_without_site` (today: none), and the test
  `every_registered_core_subr_has_a_registration_site` fails naming it — that
  is how a new registration shape announces itself;
* a name the source registers that the runtime does not is listed too
  (today 18: `w32-*`, `ns-*`, `kqueue-*` are platform-gated, two
  `romacs--vm-profile-*` are feature-gated).

Implementing-fn spans are resolved by name; when several fns share it, the
answer says how one was chosen (`module-path`) or that none was
(`ambiguous`, with every candidate). Today: 1,497 of 1,566 core subrs carry a
fn span, 65 of them chosen by module path or match arm, 51 ambiguous (mostly
`cfg`-paired twins such as the sqlite functions), 18 unresolved. VM2 binds 148
functions, all with a site: 142 from its `PRIMITIVES` table, `not` by a
direct slot write, and five Lisp bodies it embeds as data (`kill-region`,
`yank`, ...), labelled `lisp-definition`.

Set answers at this commit: VM2-only 31, core-only 1,449 (the names that block
removing `romacs-core`), both 117; 39 primitives carry a divergence row, 29 an
observable one; 106 are named by no scanned test.

## Why `build.rs` *and* a committed generator

Both, because each half has a different way to go stale and each is closed
with whichever mechanism makes staleness impossible rather than merely
detectable:

* **Spans move with every edit** to core or VM2. A committed span file would
  be stale after any such edit and conflict in every merge. Derived in
  `build.rs`, they are exactly as new as the binary that embeds them.
* **The registry cannot be derived in `build.rs`** without linking
  `romacs-core` as a build dependency, which this crate must never do (and
  which would compile core twice). So `romacs-introspect-live` boots the
  runtimes and writes `data/registry.json`; its test
  `snapshot_matches_live_registry` fails in the gate when the snapshot differs
  from what the runtimes register — any shape, any crate. On a gated tree the
  snapshot cannot be stale.
* Between gates, this binary still judges the snapshot against its own
  source: a name the source registers that the snapshot lacks (beyond those
  already absent at generation) makes `freshness` **stale**, refuses that name
  with `RegisteredInSourceOnly`, and flags every other answer.
* An in-process caller (the later Lisp wiring, inside the binary that links
  both runtimes) passes its live registry to `Index::with_live_registry` and
  needs no snapshot at all: `freshness: live`.

This is not hypothetical. Rebasing this crate onto a master that had just
merged VM2's fixnum widening, both checks fired before anything else did: the
gate test reported `VM2 registered, snapshot lacks (3): bignump, fixnump,
integerp`, and the binary's own freshness turned `stale` and refused
`fixnump` with `registered-in-source-only` at `crates/romacs-vm/src/lib.rs`.
It also exposed a flaw, now fixed: drift was judged over the *union* of names,
so `integerp` -- which core already had -- did not count, although its answer
had changed from `core` to `vm2+core`. Drift is now judged per runtime, and a
partially drifted name is refused rather than answered from the old snapshot.

Regenerate after changing what is registered:

```text
cargo run -p romacs-introspect-live -- --write     # or --check, --print
```

The snapshot is host-specific (`generated_on`): platform-gated registrations
differ on macOS and Windows, where the check would rightly fail.

## Honest negatives

`describe` on a name the registry lacks returns `NotFound(Absence)`: what was
searched, the coverage, and the likely causes — `LispDefined` (with the file
and line), `RegisteredInSourceOnly`, `NearMiss`, or `NoTrace`. There is no
`Option` accessor. Set answers state the population they ran over and their
limits (test mentions are lexical; ledger symbols that are not primitives are
named, not silently dropped).

## Boundaries

* No dependency on any `romacs-*` crate (`tests/boundary.rs`). No runtime
  `Value` crosses in; `romacs-introspect-live` copies everything into owned
  strings and drops each runtime before returning.
* VM2 declares no arity table, so VM2-only names report no arity rather than a
  guessed one. VM2 handler spans are the first match arm on the variant and are
  labelled heuristic.

## Mutants

`MUTANTS.md` lists every mutant used to prove a test can fail, with the
command that re-runs it (`mutants.py`).
