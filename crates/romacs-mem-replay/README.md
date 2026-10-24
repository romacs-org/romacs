# romacs-mem-replay (S7 strong foundation)

Dev-only (`publish = false`), depending solely on `romacs-mem`. Runs a scaled,
deterministic wl-hermes-shaped mutator for **7,200 logical seconds**, without
sleeping. Release runtime is well below the 60-second tier-0 budget. Physical
time is used only to report real pauses, total runtime and GC share; tests have
no wall-clock assertions. This is not an editor, an allocation-trace replay, or
an RSS/GNU-parity benchmark.

## Calibration and explicit scaling

Inputs, under `~/.hermes/cache/scratch/`:

- `memory-growth/wlh/out/gnu-long-1.tsv`:
  SHA256 `a57011de510b34dc463a58d7228cea5ea3272918f1ca644993eb024771bb5751`.
  721 numeric samples, 7,200 seconds; delta **8,133 GCs**, **807.285 seconds**
  GC elapsed (11.2123% share). Cumulative GNU `memory-use-counts` deltas:
  conses 39,539,679,319; floats 68,037,547; vector cells 10,732,743,063;
  symbols 57,766,178; string characters 71,565,740,286;
  intervals 2,726,795,663; strings 2,672,999,878.
- `memroots/census/out/census-2.tsv.census`:
  SHA256 `b8ddc81ee46e3e0387ae571fe0357a6f50ad9039076b080a3762b53590e2110b`.
  60 snapshots: medians 12 live buffers, 1,010,983 live characters,
  265,713.5 undo entries, 229,919.5 buffer-id slots; final buffer id 429,331,
  marker id 53,544,963; cumulative reclaimed symbols 15,302,492.

The replay rounds the measured ratios per string: 14.792 conses to **15**,
4.015 vector cells to **one four-slot vector**, .021611 symbols to **one per
46 strings**, .025454 floats to **one per 40 strings**. Alternating ASCII/UTF-8
strings have about 27 characters (measured mean 26.774); the cons chains and
vectors have real traced edges, not inert reservation cells.

It scales total traffic to 16 strings/logical-second (115,200 streamed strings,
about 1/23,203 of GNU traffic), retaining a 96-entry ring rather than the native
undo/live set. Three chat-like legacy payloads reserve 400,000 bytes apiece and
stream/rotate text inside that capacity, matching the original three-chat,
400,000-character rotation shape approximately, not its UTF-8 byte limit.
A temporary buffer every ten events approximates census buffer-id churn; the
remaining native managers are not modeled. The fixed text capacity stabilizes
storage independently of the changing text length. The threshold is scaled to
16,000 bytes, with percentage zero; it is **not** shipping collector tuning.
Explicit collections and logical one-second input waits exercise all triggers.

## Machine gate and report

- At least 200 full collections and 200 threshold collections; nonzero explicit
  and idle counts; nonzero reclaim counts for every strong churn kind.
- Exact per-kind graph inventory equals each cycle's eager freed-object count;
  legacy drop observations independently agree with the inventory.
- Read back the heap's complete per-GC record; reconcile sequence, count/epoch,
  online sum/max and maximum overshoot, requiring positive overshoot/page release.
- Discard the first 20% of GCs, split the remaining sequence into four equal-count
  windows: accounted-after-full **and allocator committed-storage** medians must
  differ by no more than 10%. Forced-every-60-events median must be within 5% of
  natural. The forced CLI actually runs the natural comparison too.
- Leak arm roots an 8,192-byte string every 30 events; its final planted bytes
  exceed 30% of normal live storage. The same classifier returns **Fail**;
  tests require natural=Pass, forced=Pass, leak=Fail. CLI failure exits 1.

Slope is OLS of accounted-after-full versus logical seconds for t >= 600,
reported as MiB/logical-hour. Quantiles use nearest-rank across **all** real heap
GC records, including warmup. GC share is sum(gc_elapsed_ns)/physical runtime.
Allocator committed bytes count initialized page budgets, released-page headers,
large spans and symbol pages. They exclude untouched chunk tails and external
Rust storage, and are **not RSS**. The small additive `AllocationStats` accessor
is necessary: mapped chunk capacity alone cannot certify page commitment.

## Run

```sh
cargo test --offline --locked --profile gate -p romacs-mem-replay -p romacs-mem --features romacs-mem-replay/verify
cargo run --offline --locked --release -p romacs-mem-replay -- normal normal.tsv
```

CLI: `[normal|forced|leak] [records.tsv]`. TSV preserves every real GC record and
its logical timestamp. No special churn is claimed: `special_churn_hook` names
S5 weak removal, finalizers, marker unchaining and RefCell undo-owner work, each
requiring its own nonzero counters once that collector exists.

The m3 native RSS +10 MiB/hour / 1.1x-GNU gates, native forced RSS comparison,
and same-run GNU individual-pause gate remain **unmeasured** here. The old GNU
TSV samples cumulative GC elapsed, not individual pauses, so its approximate
window-average p99 must not be used as a per-GC p99 oracle. Tight scaled replay
GC share likewise cannot be compared fairly to native GNU's interpreter/session
share or used to trigger the native generations decision.
