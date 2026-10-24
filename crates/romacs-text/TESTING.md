# Text test loops

`romacs-text` keeps every test in the debug loop. Assertions and edit generators
are shared with the full-strength run; only repetition budgets differ with
`cfg!(debug_assertions)`:

| Workload | Debug | Release |
| --- | ---: | ---: |
| `fixed_seed_20k` (each byte mode) | 1,024 | 20,000 |
| `differential` cases (up to 199 operations each) | 32 | 256 |
| `adversarial_large_and_ends` initial atom repeats | 256 | 50,000 |
| `adversarial_large_and_ends` end insertions | 128 | 1,000 |
| `batch_fuzz_100k` cases (1,000 edits each) | 10 | 100 |

The split-growth complexity test retains 8,192/16,384 edits in both profiles.
Cache invalidation, cached-insertion summary refresh, split/merge reproducers
and the production-size integration reproducer are not weakened in debug.

Run from the repository root:

```sh
scripts/tier0 romacs-text romacs-chars
scripts/premerge-text
python3 scripts/premerge_text_test.py
```

`premerge-text` runs the full-strength models, structural controls, GNU oracle
and character tests in release. It checks the effective release configuration
for the library and an integration target and refuses debug assertions, an
unverifiable configuration, compilation failure or any failing test. Passing
`--release` alone is not proof of full-strength budgets when profile overrides
are present. Keep generated timing and gate reports under `target/evidence/`,
not beside maintained fixtures.
