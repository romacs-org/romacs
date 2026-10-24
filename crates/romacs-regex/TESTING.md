# Literal kernel verification

Run from the repository root:

```sh
scripts/tier0 romacs-regex
scripts/premerge-regex
python3 scripts/premerge_regex_test.py
```

The premerge command checks the effective release configuration for the library
and `gnu_scenarios`, refusing debug assertions before running regex and character
tests. Debug and release use the same assertions; only randomized repetition
budgets differ. No test is intentionally skipped in the debug loop.

The independent character-wise model uses 32 rounds in debug and 512 in release.
Both profiles retain the 1,300,306-byte counter fixture, 502 planted hits and
20 passes, every seam, both directions, byte-mode pairs, case folding, bounds,
registers, cache-key isolation and LRU assertions.

## Cache controls

`each_key_component_separates_programs` checks generation, identity, POSIX mode,
target and pattern byte modes, whitespace and program kind through actual search
results. `lru_promotes_hits_and_evicts_tail` checks compile/hit counts.
`case_table_mutation_rebuilds_snapshot` mutates canonical/equivalence data and
changes both identity and generation. Failed compilations never enter the cache.

The working-set cache holds at most 128 programs. Hits update recency; a full
miss evicts the oldest entry. The clock saturates at `u64::MAX`, after which
recency stamps may tie. The owned key is constructed only after a successful miss.
`cyclic_org_working_set_only_compiles_first_round` visits 60 patterns for 100
rounds and checks 60 compiles and 5,940 hits. Eviction tests use 129 keys.

The deterministic ASCII prefilter fixture contains frequent isolated anchor
decoys and requires at most three verifier candidates per hit in both directions.
This bounds verifier work, not memchr probes or elapsed time.

## GNU differences and diagnostics

The accepted-divergence ledger records D-R8 (full canonical equality across
variable-byte-length folds) and D-R9 (full-width unibyte canonical values).
Tests retain the representable controls, both directions and all seams.

`examples/regex_bench.rs` and `bench/gnu-kernel.el` are kernel diagnostics,
not editor speed measurements. For example:

```sh
cargo run --offline --release -p romacs-regex --example regex_bench -- k-cache-org
```

The Org-like corpus compiles literal pattern bytes; it is not an Org parser or
a regexp-engine throughput test. Keep generated timing reports outside fixtures,
for example under `target/evidence/`.
