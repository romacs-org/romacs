# Variable differential harness

varh compares variable behavior with recorded GNU Emacs observations. Each
recording runs a fresh GNU process per case and regime; Rust replay creates a
fresh production VM at collection floor 1 per case. This prevents persistent
variable state from contaminating later cases. The optional single-VM replay is
a separate state-leak diagnostic, never the source of accepted pins.

## Maintained inputs

- `gen_cases.py` generates the case definitions; `cases.json` fixes their IDs,
  forms and setup expressions.
- `gnu_driver.el` executes one case in one GNU regime.
- `evidence/varh.json` contains the recorded answers, regime variants, divergence
  pins and recorder identity. Consult it for the actual GNU version and revision.
- `replay.rs.in` and `varh.py` are original byte-pinned recording inputs.
- `../../tests/varh.rs` replays the corpus and checks its integrity.

The byte-compiled GNU result is the primary answer. A different interpreted
result is retained as a regime variant; matching either is recorded explicitly.
A new divergence, stale pin, changed class, unknown pin or stale input hash fails.
The harness also exercises those refusal paths with negative controls.

## Check and replay

Run from the repository root:

```sh
V=crates/romacs-vm/tools/varh
python3 "$V/gen_cases.py" --check
cargo test --locked -p romacs-vm --test varh
cargo build --locked -p romacs-vm --bin romacs-vm
python3 "$V/varh.py" replay --fresh --vm2 "${CARGO_TARGET_DIR:-target}/debug/romacs-vm"
python3 crates/romacs-vm/tools/primh/render_replay.py varh --out "$V/../../tests/varh.rs"
```

The historical `--vm2` option is retained by the byte-pinned recorder. Use
`render_replay.py` for generated Rust targeting the current crate: it validates
through the original producer, then adapts emitted imports and value-display
calls without rewriting recording provenance.

## Recording and review

Build the reference described by `parity-reference.toml` with
`tools/build-gnu-oracle.sh`. An intentional new recording uses
`python3 "$V/varh.py" record --gnu "$ROMACS_GNU_ORACLE" --jobs 8`.
Inspect every changed answer and its oracle identity. Do not refresh hashes or
accept pins just to make a test pass. `varh.py pin --vm2 BIN` changes the fixed
pin set and requires review of every affected case.

Keep execution logs and exploratory JSON reports outside the checkout, or in a
dedicated ignored output directory. The fixed corpus is a test input, not a run
receipt. Historical development scores are not the current compatibility status.
