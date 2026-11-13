# romacs-vm

Romacs's Elisp runtime: a Rust reader, lexical compiler, strand-private
register VM, moving collector, source loader and local agent interface. The
GUI host (`romacs-vm-gui`) runs on it: `--loadup` boots GNU's preloaded Lisp
chain from source or restores it from a world image (`src/world`), and
`--batch` does the same for noninteractive runs.

GNU 32.0.50 remains the semantic oracle. Deliberate departures belong in
[the divergence ledger](../../ledger/divergences/README.md), beginning with scalar `aset` replacement.
A recorded departure changes an exact expected result; it never disables a test.

## Executable images

Compiled IR, stable symbols and a constrained configured-world snapshot can be
persisted in the existing authenticated mapped store. Function bodies are decoded
and lowered only when called. See [CODE-IMAGE.md](CODE-IMAGE.md) for the CLI,
compiler/config identity, integrity/GC proofs, eager metadata costs and limits.
GUI startup does not use these code images; it restores a world image (`src/world`).

## Build and verify

Run from the repository root with the configured Rust toolchain:

```sh
export CARGO_TARGET_DIR="${CARGO_TARGET_DIR:-$PWD/target}"
cargo build -p romacs-vm --bins
cargo nextest run -p romacs-vm $(python3 tools/lib/test_modules.py targets romacs-vm)
python3 crates/romacs-vm/tools/check_source.py \
  --binary "$CARGO_TARGET_DIR/debug/romacs-vm" --output "$CARGO_TARGET_DIR/source-oracle.json"
```

`-p romacs-vm` builds and tests this runtime without the GUI host.
The test line runs the library's tests and the `all` target, which holds every
`tests/FILE.rs` as a module, each test in a process of its own as the gates run
them. A bare `cargo test -p romacs-vm` also builds each file as a target of its
own and runs its tests a second time; `cargo test -p romacs-vm --test FILE`
runs one file.

## Source and agent requests

`romacs-vm --source` evaluates UTF-8 source on stdin, sequentially by top-level
form. `--source --dump-ir` prints readable optimized IR without executing it.
`Vm::read_source`, `Vm::compile_source` and `Vm::eval_source` expose these paths.
The historical already-read JSON form transport remains available; JSON numbers
may be integers or floats, strings denote symbols, arrays denote lists, and
literal strings/vectors use `{"string":"text"}` / `{"vector":[1,2]}`.

`--eval-stream` accepts newline-delimited JSON and flushes one reply per request.
Definitions and globals persist. Source replies retain the printed `value` field
and add a structured `result`. Requests can also register fixed-arity prepared
lambdas or named calls, invoke handles, inspect their IR, or submit a batch:

```json
{"source":"(defun twice (x) (* x 2))"}
{"prepare":"twice","arity":1}
{"invoke":0,"args":[21]}
{"prepare":"(lambda (x y) (+ x y))"}
{"invoke":1,"args":[20,22]}
{"ir_handle":1}
{"source":"(+ 1 2)","ir":true}
{"batch":[{"source":"(setq state 7)"},{"invoke":0,"args":[9]}]}
```

Handles are VM-local, never recycled, and survive cache clearing. Macro or special
binding changes revalidate/recompile their dependencies. Prepared lambdas have
fixed arity and no enclosing lexical captures. Ordinary Lisp functions still
support optional/rest parameters.

Invocation arguments use JSON numbers/strings/booleans/null, `{"symbol":"name"}`,
`{"list":[...]}` or `{"vector":[...]}`. Typed results distinguish integers, floats,
symbols, strings, bytes, conses, vectors, functions and buffers. Float results
include their binary64 bits; nonfinite numeric JSON values are null, with the
bits retaining the exact value. A batch runs sequentially, not transactionally;
errors occupy their own reply element and do not cancel subsequent elements.

This is a trusted local interface, not a sandbox. `process-lines` launches local
programs synchronously without a shell or resource limits.

## Source IR cache and direct tier

The source key binds exact source text, cache ABI and the compiler fingerprint
(all crate Rust sources, build inputs, toolchain, target and features). Entries
retain compiled IR and version/status guards. Macro dependencies include expanded
symbols and constant-expansion contents; arbitrary state-dependent expansion
code recompiles rather than using an incomplete dependency guess. No global
constant inlining is implemented, so global values remain runtime slot reads.

Mutable quoted/string/vector literals are conservatively excluded from source
reuse: rereading source must not reuse a previously mutated literal. Prepared
functions instead retain their literal identity across invocations.

`--cache-store DIRECTORY` opts into persistent, content-addressed authenticated
images. Portable units use the executable-image opcode codec, symbolic slot
relocation and relative function/cache IDs so they can enter an existing VM.
Macro-bearing units currently remain memory-only. Bad hints, compiler mismatches
and invalid artifacts recompile. This does not claim zero-copy execution of
portable units; whole-world images have a separate lazy-body interface below.

Lisp inspection: `(eval-cache-stats)` returns
`[hits compilations invalidations disk-hits direct-calls entries]`;
`(eval-cache-entries)` returns sorted source keys. `(eval-cache-clear)` clears
lookups and persistent source-index hints, not immutable shared blobs or compiled
functions that may still be referenced by closures.

At most eight syntax nodes, atomic arguments, and an installed supported
primitive qualify for direct apply without compilation. This is a bounded
eligibility rule, not a claimed universal measured crossover. The calibration
compares tiny direct calls, cold compilation, warm source reuse and handles
against GNU and SBCL. Warm resubmission is an agent-only pattern; GNU/SBCL cold
eval do not retain matching compiled state.

## Source loading and bootstrap

`load`, `require`, `provide` and `featurep` operate on real files and the Lisp
`features` variable. `load-path` is configurable from Lisp or `Vm::set_load_path`.
Source inputs accept strict UTF-8 and declared Latin-1/ASCII, with BOM and declared
DOS/Mac line endings. Unsupported coding systems and `.elc` files fail explicitly.
Missing-file `noerror` does not suppress evaluation errors or a missing `provide`.

```sh
"$CARGO_TARGET_DIR/debug/romacs-vm-load" lisp/loadup.el
```

The probe emits exact file/form/line progress, including the failing form. It
loads the unmodified `debug-early.el` and enters `byte-run.el`; bootstrap currently
stops at `byte-run.el` form 15, line 156, where a function-unquote inside evaluated
quasiquote needs a real quasiquote/macro-expansion implementation. Successfully
reading many inherited source literals does **not** establish their executability.
`lisp/` is unchanged; its VM2-only coding migration must wait for VM2-owned loading.

Nested loads retain and, when needed, rebase suspended register frames/handlers.
Compaction is inhibited during these nested executions until the outer frame
resumes; tests cover wider newly loaded functions, outer roots and error recovery.
Fatal executable-image errors remain uncatchable across that boundary. General
nonlocal exits across load boundaries, autoloads, load-history/after-load machinery,
full coding-system support and complete bootstrap remain unfinished. Buffer-name
lookup currently supplies the initial `*scratch*`; a full named-buffer lifecycle
is not implemented.

## Executable images and GC

Compiled IR and stable symbol/function cells can be saved in the authenticated
mapped store. Function bodies decode/lower only when called. See
[CODE-IMAGE.md](CODE-IMAGE.md) for compiler/config binding and limits. Integration
restores the source symbol-name table and macro flags; the directory ABI is
`VM2CODE5`. Whole-world image publication still rejects unsupported private object
kinds, including boxed floats; portable source-cache units can persist floats.

Unicode strings own UTF-8; byte strings enforce octets. Floats are boxed binary64
leaf objects. All private object references move during mark/compact collection;
embedder-held values require explicit mutable roots. Mapped graph objects remain
untraced, with `PROT_NONE` tests before and after executable lowering.

Release uses threaded sibling-call handlers; debug drives the same handlers one
at a time. Named call caches guard mutable function cells. Activations consult
the dynamically bound `max-lisp-eval-depth`; register storage and native
reentry stacks grow at guarded boundaries, and activations and the compiler
refuse recoverably on low remaining native stack. There are signed 62-bit
fixnums, plus arbitrary-precision integers read from radix and decimal
constants and returned by `string-to-number`, but not yet produced by arithmetic
or accepted by every consumer (see `ledger/divergences/VM2-TIME1.md` and the root
ledger's RADIX1 and VM2-FMT1 rows). There is no JIT, no general tail-call
promise, no full `error-conditions` hierarchy, no `unwind-protect`, and no shared
concurrent mutator heap. The lexical-only language policy remains in effect.

## Oracle fixture portability

`evidence/` contains maintained test inputs and GNU observations, not a directory
of current performance claims. Store new run logs and timing reports under
`target/evidence/`. Do not put disposable reports beside fixture files.

Recording executable paths and diagnostic home prefixes are redacted as
`<recording-path-redacted>` and `<recording-home>`; executable hashes, GNU
revisions, forms, outputs, conditions and divergence pins remain authoritative.
These placeholders are descriptive metadata, not executable paths.

`evidence/locate-file-gnu.json` uses `<fixture-root>` for the test-owned root in
`tests/locate_file.rs`: directories `first`, `second`, `first/directory.el`;
files `first/library.el`, `first/library.elc`, `second/library.elc`, `first/plain`;
`default-directory` is the absolute root plus `/first/`. Expand the marker to a
fresh temporary root when reproducing the forms. All nine GNU cases were checked
again against that layout before normalizing only the common root. The explicit
`first/./plain` spelling and suffix/search-order outcomes are preserved.
