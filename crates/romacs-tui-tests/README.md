# Independent terminal test support

This crate survives removal of the legacy `romacs` executable and the package
acquisition suites. Keep its workspace member and workspace dependency entries.

Retained functionality:

- Structured launches, owned HOME/TMPDIR fixtures, PTY input/output and lifecycle,
  erase-character configuration, terminal sizing, and Emacs key translation.
- Asciicast recording, pacing policy, and recording tests.
- Exact display comparison (geometry, text, RGB colors, styles, wrapping, cursor),
  raw terminal snapshots, and inline/file-backed snapshot assertions.
- Deterministic Git fixtures and their isolation/ownership tests.
- Generic paired-session driving and screen/panic helpers, which accept sessions
  supplied by callers rather than selecting an editor executable.
- Independent scenario policy contracts and all five original contract tests.
  The symlink identity test now uses `TuiTempDirectory`, not the removed package
  sandbox; its identity assertions are unchanged.
- The independent Git project fixture cleanup test and four raw-terminal snapshot
  integration tests, with their original assertions.

Removed functionality:

- The legacy executable locator/override, editor launch convenience methods,
  fixed-executable pair boot helpers, and two locator unit tests.
- Integration scenarios that launch that executable directly, through `boot_pair`,
  or through local wrapper helpers. This includes editor redisplay oracles: their
  GNU reference does not make their legacy implementation half independent.
- Third-party package suites, package-specific snapshots, package acquisition and
  startup orchestration, and the helper requiring its package-owned pair type.
- The `romacs-melpa-test-support` dependency and the now-unused `romacs-infra`
  dependency.

No VM2/Neovm identifiers, dependencies, executable launch paths, or VM2-specific
cases were present in the original crate. No assertions or tolerances in retained
tests were weakened.

Full verification (when a lightweight dependency build is permitted):

```sh
cargo test --locked -p romacs-tui-tests -- --test-threads=1
```

Run unit tests serially with Cargo: the hostile Git-environment fixture test
mutates process environment and was originally intended for per-test nextest
isolation. Neither GNU Emacs nor a Romacs executable is needed by retained tests.
