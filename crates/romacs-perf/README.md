# romacs-perf: independent performance evidence tooling

This crate is now a library. It does not locate, build, launch, or install the
retired `romacs` executable or third-party editor packages.

## Retirement classification

Removed:

- The CLI/binary and editor-launching `PerfHarness::{run,profile,compare,suite}`
  pipelines, GUI/PTY orchestration, sandbox/package preparation, and live editor
  provenance/portable-dump probes.
- MELPA package loading and fixtures for lsp-mode/Rust typing, Magit status, and
  org-journal. The mixed editor-workload fixture retains its GNU-compatible
  operations, with its Magit branches and retired startup-GC-ceiling hook removed.
- The old executable's sustained-native-video scenario, optimized-build-profile
  acceptance, and GStreamer discovery path.
- All dependencies on MELPA support, romacs-infra, clap, and GStreamer.

Retained independently:

- Artifact schemas and historical scenario identities, correctness verdicts,
  metric units, provenance identity, comparison scheduling/statistics/rejection,
  suite regression/drift evaluation, and suite-history validation.
- Pure validators for bytecode, VM-loop/first-entry, builtin calls, search shapes,
  bounded searches, completion, scrolling, first-party editor workloads, and
  GNU elisp-benchmarks result documents. `validate_fixture_result` takes observed
  process wall time explicitly; it never launches an editor or invents timing.
- Native latency JSONL analysis, portable-dump file/content verification and
  capture-argument construction, host policy checks, perf CSV/binary parsing,
  command wrapping, and acknowledged/cancellation-safe profiling gates.
- Independent GNU fixtures/tests, including all eight elisp-benchmarks protocol
  ERT regressions. Their Rust test still requires `ROMACS_FORCE_ORACLE_PATH` and
  still fails rather than skipping an unavailable oracle.

Historical package/video scenario enum values remain readable so old evidence
can be analyzed. They are not runnable scenarios. Fixture validation rejects
Rust-LSP, Magit, org-journal, and sustained-native-video identities explicitly;
this boundary has a regression test.

There were no direct VM2 dependencies or VM2-named tests in this crate. Both
existing VM-loop/first-entry test files are retained byte-for-byte, including
work-count, restoration, malformed-schema, per-function identity, and shortened
work rejection assertions. Their JSON/schema tests are not a claim of executing
VM2 itself.

The fixture-publication harness and historical environment allowlist now exist
only under `cfg(test)`. Production requests default to an empty explicitly owned
environment, not the ambient process environment. One previously missing exact
allowlist key (`ROMACS_REGEX_CACHE_DIAGNOSTIC`) was restored to satisfy the
existing GC-sidecar regression without changing that test's assertions.

## Verification

Run the crate's library tests with a real GNU oracle:

```sh
ROMACS_FORCE_ORACLE_PATH=/absolute/path/to/emacs cargo test -p romacs-perf --lib
cargo check -p romacs-perf --lib
```

During concurrent workspace retirement, this library was checked and tested in
an isolated offline Cargo graph with the same retained registry dependencies,
without building the editor, VM2, graphics stack, or the rest of the workspace.
Final isolated verification: **134 Rust tests passed, zero failed/ignored**,
including the test that proves all **eight GNU ERT protocol regressions** ran
without skips. The non-test library check completed without warnings.
`rustfmt --check`, `git diff --check`, and GNU Emacs `check-parens` on the
mixed editor-workload fixture passed.
Workspace-level membership/dependency reconciliation remains the parent's scope.
