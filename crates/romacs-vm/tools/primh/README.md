# Primitive differential harness

Specs come from the production build's `OUT_DIR/primitives.json`. Repeated
`--only` selects declarations explicitly; unknown filters fail before GNU starts.
Both GNU arity bounds and native identity must match the declaration.

## Recording and replay

Run from the repository root. Build the pinned oracle once, then use `record.py`,
not direct `primh.py record` or `bulk.py record` calls:

```sh
tools/build-gnu-oracle.sh
H=crates/romacs-vm/tools/primh
PIN=$(python3 -c 'import tomllib; print(tomllib.load(open("ledger/parity-reference.toml", "rb"))["mirror_commit"])')
export ROMACS_GNU_ORACLE="${GNU_ORACLE_ROOT:-${XDG_CACHE_HOME:-$HOME/.cache}/gnu-emacs-pin}/$PIN/bin/emacs"
export CARGO_TARGET_DIR="${CARGO_TARGET_DIR:-$PWD/target}"
cargo build -p romacs-vm
python3 "$H/record.py" primh "$H/specs/take.json"
python3 "$H/record.py" bulk core
python3 "$H/primh.py" replay "$H/specs/take.json" --vm2 "$CARGO_TARGET_DIR/debug/romacs-vm"
python3 -m unittest discover -s "$H" -p test_primh.py -v
```

`record.py` requires an absolute executable that reports the declared GNU pin;
it hashes source files from the oracle's own clean source checkout and runs with
stdin at EOF. `tests/gnu_oracle_pin.rs` checks recording headers. Recording
resets pins: inspect every changed case and never turn a new failure into a pin
merely to obtain a passing replay. `oracle-drift/rerecord.py` carries existing
pins only if all case results are unchanged.

To derive new specs, pass the current Cargo build-script output explicitly to
`primh.py from-manifest MANIFEST --out DIR --gnu "$ROMACS_GNU_ORACLE" --only NAME`.
Do not copy a fingerprinted OUT_DIR from another build. Every primitive's
replay is one `#[test]` line of `tests/primh.rs`, under one copy of the
template: add new ones with
`python3 "$H/render_replay.py" primh SPEC... --into crates/romacs-vm/tests/primh.rs`, which
keeps the replays the file has. The replays edited by hand after rendering
(`primh_eval.rs`, `primh_mapc.rs`, `primh_mapcan.rs`, `primh_map_keymap.rs`,
`primh_cmdloop_pins.rs`) keep their own files. `--out TEST.rs` renders a file
of its own. The original producer and template remain byte-pinned recording
inputs; the renderer adapts only their emitted crate import and value-display
calls. Review generated changes with the corresponding specs.

Run reports belong under `target/evidence/`; the `evidence/` corpus and code
currently under `logs/` are maintained inputs, not disposable output directories.
Use `record.py --out` to keep exploratory recordings separate from fixtures.

## Contracts and limits

- Deterministic typed-cartesian, wrong-arity-through-apply, GC, alias and printer
  families preserve the original harness's finite domain, not universal parity.
- GNU runs with a private HOME, byte-compiled and interpreted. Pure declarations
  batch in one process; other effects get one process per case. This is not a
  sandbox for arbitrary destructive/global probes.
- A pure batch timeout preserves every complete streamed row, including both GNU
  regimes, then retries the first incomplete case **alone** with a fresh ten-second
  budget. Only an isolated timeout without a completed row becomes `GNU-TIMEOUT`;
  a slow aggregate batch cannot exclude a terminating case. Every attempt validates
  header identity/arity/consistency and reply count/order/regime pairs, including
  interrupted prefixes. A fully streamed row survives a later process timeout.
  GNU process crashes and VM crashes are failures, not timeout evidence.
  `HARNESS-ERROR` is never valid evidence.
- Evidence hashes declaration, generator, domains, driver, replay template, GNU
  executable and referenced GNU source files. Python validates deterministic case
  generation before replay/emission/inference. Atomic same-directory writes use
  unique pending files, fsync and replace. Reference recording precedes replay.
- Rust includes the **current build output** using
  `include_str!(concat!(env!("OUT_DIR"), "/primitives.json"))`. Every spec must match
  its current name, min/max arity, effect, GNU reference and Rust reference before
  replay. No new public registry API or GNU executable is needed for Rust replay.
- Python replays the production binary with `--eval-stream --gc-floor 1`; Rust
  creates a fresh production `Vm` at collection floor 1 per nonexcluded row.
- Either GNU regime counts as parity. New/stale/changed-class/orphan pins and
  missing/extra Python replies fail. `PRINTER` pins are instrument limitations,
  **not verified primitive parity**. Rust trusts recorded case sources; Python's
  deterministic-generation check supplies the additional source-integrity gate.
- Harness mutation tests prove evaluated-source divergence, stale pins and six
  declaration mutations (name, min, max, effect, GNU reference, Rust reference).
  They are not a production primitive-body mutation proof; that belongs to the
  main integration lane.
- Rust emission currently keeps its smoke fixture at `cdr-safe`; retain that
  fixture/spec when selecting a different primitive subset.

## Bulk and prompt cases

`bulk/core.spec.json` declares finite domains and probes, never answers.
`tests/bulk_core.rs` replays the corpus with fresh production VMs and rejects
changed hashes, insufficient successful GNU rows, unpinned differences and stale
or unknown pins. `bulk_mutants.py` requires corpus assertion failures, not compile
failures. `examples/bulk_replay.rs` prints individual verdicts.

`minibuf_specs.py` builds `read-from-minibuffer` and `read-string` probes using
GNU's batch keyboard-macro regime. The corpus covers history, initial values,
recursive prompts, C-g, GC, stdin and prompt properties. Record through
`record.py primh` with stdin at EOF. See the exact current spec and divergence
ledger rather than historical development logs for the accepted cases.
