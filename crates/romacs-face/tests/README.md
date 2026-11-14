# romacs-face tests

| file | proves |
|---|---|
| `cycles.rs` | cyclic Lisp input rejected at conversion; named inheritance, distinct remap-entry and palette cycles rejected; 20,000-face inheritance resolves on a 128 KiB stack |
| `generation.rs` | immutable themes and remaps have distinct generations; generation-keyed cache entries cannot go stale; reviewed compile-fail API contracts |
| `collapse.rs` | defface selection for the fixed truecolour graphical target; background remains live; corpus specs match the bundled tree |
| `parity.rs` | old 25-theme plus unthemed GNU corpus, attribute selection/inheritance and modus/rum palettes; explicit reset classification |
| `remap_parity.rs` | GNU C remapping and anonymous refs; old height fixtures explicitly checked as Lisp queries, not mislabelled redisplay |
| `readiness.rs` | typed depth/byte bounds, million-deep subprocess regressions, aliases, valid inherited remaps and rejected-entry fallback, C stepwise heights, explicit user preferences over bundled modus-vivendi, underline reset and recorded smaller divergences |

## GNU fixtures

`gnu/fixtures/*.eld` come from `gnu/regenerate.sh` using GNU `-Q --batch`
(version in `gnu/fixtures/ORACLE`). `gnu/dump.el` simulates pgtk/truecolour
predicates; selection and merging are GNU's own. Most old attribute rows
are unspecified/default-equal, and default height 1 makes their height
coverage weak. Do not describe those counts as independent meaningful
height proofs.

`gnu/fixtures/rum-configured.eld` is the look the stock theme `default`
ports: modus-themes 5.3.0 (`etc/modus/modus-themes.el`, the copy Romacs adapts too), Omarchy's Rum
theme file (`gnu/inputs/rum-emacs.el`), the reference configuration's
Modus options and font faces, and notmuch's faces, captured by
`gnu/rum-configured.el` in `dump.el`'s regime. Regenerate with
`gnu/regenerate-rum.sh` (pinned GNU, `NOTMUCH_LISP` naming notmuch's Lisp).
`romacs-vm-gui/tests/romacs-theme-rum-test.el` compares every face.

`gnu/redisplay.el` instead calls GNU C `face-attributes-as-vector` over
non-unit defaults 100, 113, 97, 120; `gnu/redisplay.eld` records C-effective,
C-realized and Lisp-query heights separately. Regeneration:

```sh
# GNU at the pin only: tools/lib/gnu_oracle.py refuses anything else (R228).
"$(python3 tools/lib/gnu_oracle.py path)" -Q --batch -l crates/romacs-face/tests/gnu/redisplay.el > "${TMPDIR:-/tmp}/redisplay.eld"
# Check exit status directly, then review the diff before replacing the fixture.
```

No live daemon, user config or user database is used. The recorded C 235 vs
Lisp 236, and C 253 vs Lisp 254 over default 113, catch grouping/truncation
regressions the old fixtures cannot see.

## Compile-fail contracts

`ui/*.stderr` are reviewed, tracked expectations for pinned rustc 1.96.1.
Never set `TRYBUILD=overwrite` to make a failing run pass. New expectations
were copied from trybuild's `wip/` output after inspecting the intended
E0308 error; an unreviewed compile error is not red evidence.

The old guards cover private theme/attribute storage and generation minting.
`raw_defface.rs` and `raw_remap.rs` prove that an unchecked Datum cannot be
passed to those APIs. Other public face-data inputs likewise require the
checked type, and `FaceRef` is opaque. Mutation replay relaxes each tested
raw-input signature independently and requires the **test runner** to fail
with “Expected test case to fail to compile, but it succeeded.”

## Reproducible mutation evidence

`mutants.json` contains exact file preimages/replacements, target tests and
expected assertion markers. `mutants.py` applies one at a time, demands an
executed assertion-red (never a crate compile error), restores byte-for-byte
source snapshots in `finally`, then demands restored-green. Core dumps are
disabled for the two isolated stack-overflow mutants. Run only in an owned
worktree with the prepared project environment:

```sh
python3 crates/romacs-face/tests/mutants.py --apply --logs "${TMPDIR:-/tmp}/face-mutants"
```

Use `--only M19-default-underline` for one mutant; `--results PATH` writes
machine-readable results outside the source checkout. Historical replay receipts
are not versioned; `mutants.json` retains the executable mutation definitions.
The list includes the review's surviving underline-reset and undefined-parent
faults, plus quote/raw stack overflow, expanded bytes, aliases, C height
order, user preferences, font/function/dashed-family policy and type guards.
Some entries target the same budget fault through different public paths;
the count is surgical probes, not a count of independent semantic facts.

The initial readiness red run compiled and failed the nine newly defective
behaviours at their intended assertions. Existing-correct underline, undefined
parent and font-policy behaviours were additionally forced red by the
committed mutants, then restored green. User-preference loading and alias-key
composition also have separately recorded initial failing assertion runs in
the private lane report. No gate or performance comparison is implied by
these tests.
