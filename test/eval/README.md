# Unwired E2 fixture and experiment

`core-rows.el` copies the five bytecode rows and four helpers from
`test/bench/core-bench.el`. Regenerate with `python3 scripts/dump-eval-rows`.
The command compiles both sources with `/usr/bin/emacs -Q --batch`, compares
all nine functions including nested closures, records GNU answers, and emits
the test-only Rust materializer. The `.elc` and its SHA-256 are retained;
production eval has no reader dependency. No quit-flag mutation is executed.

Build `vm_bench` in release/default through the lane's admitted Cargo slot.
Then run the frozen executable and GNU interleaved, inside a light slot:

```
python3 scripts/eval-vm-bench --binary /absolute/path/to/vm_bench --out /owned/scratch/results -n 3
```

The GNU harness's `--romacs` position deliberately holds GNU itself; that
stand-in is not an E2 or old-core result. Only its GNU bytecode/native fields
are compared with the separately executed VM. Timings include the exact full
row counts, one warmup and five repetitions, with result assertions and private
HOME/TMPDIR. Warm heap traffic excludes Rust allocator activity.

The thesis and strict assembly targets remain experiments, not acceptance
claims. Preserve source hashes, executable hash and clean-build output beside
measurements; sharing a target between copied trees can defeat Cargo freshness.

Host supplies error conditions, primitive identities and lazy-code fetching.
A Host without fetching returns typed `LazyCode` refusal. `make_closure` expects
a fetched prototype. E2's mapcar accepts lists and arithmetic fast paths accept
fixnums; shared `1+` also supports mem floats. Bignum overflow returns typed
refusal pending E3; other arithmetic opcodes remain fixnum-only. In-loop
handler resumption, switch and buffer operations are not implemented here.
Code snapshots are immutable, while constant-vector elements remain live.

Round-2 regressions are in `tests/r2_probes.rs`. `/usr/bin/emacs -Q --batch -l
 test/eval/round2.el` records the identical opcode programs; `round2-gnu.txt`
retains its conditions/results and a separate private/initial-obarray control.
`Obarray::new_initial` owns keyword policy; `Obarray::new` creates an ordinary
private obarray. Reborrowing preserves that policy in the immutable raw tail.
Setting constants preserves the destination in `BindError::Constant(symbol)`;
the fixture translates it to `(setting-constant symbol)`. Numeric overflow and
bignum arithmetic remain `NotYet(84)`; marker support and GNU's exact nonnumber
predicate remain unwired Host boundaries.
