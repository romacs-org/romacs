# VM2 live registry

This crate links VM2, not the retired evaluator. `fboundp` queries function
cells without invoking candidate functions. Candidates are VM2 source string
literals; dynamically constructed names without source literals are outside
this enumerator's coverage, as they were before retirement.

The two `data/registry.json` files in introspect and introspect-live are the
same VM2-only schema-v2 artifact. `--write` regenerates both; `--check` compares
full deterministic bytes of both, including provenance and formatting. The
mandatory `snapshot_matches_live_registry` test checks the same byte contract.
VM2 entries are names, not arities; core entries are empty (retired).

## Input identity and freshness

The shared `romacs_introspect::fingerprint::input_hash` computes SHA-256 of
sorted workspace-relative paths and file bytes. Each path and byte sequence is
prefixed by its u64 little-endian length, under a v2 domain separator. It does
not include HEAD, Git status, wall-clock time, absolute checkout paths, or either
registry output. Schema v2 records `input_hash` and `generated_on` (OS-arch),
not commit/time/cleanliness. Live answers may retain an in-memory enumeration
time; derived query answers may retain build time and Git provenance.

Scope: workspace Cargo.toml, Cargo.lock, optional rust-toolchain files and .cargo
configuration; then each crate's Cargo.toml, src/ tree, build.rs and build/ helpers
in the generator's recursive local normal/build dependency closure. This includes
target-specific and inherited workspace dependencies. All local workspace patches
and their dependency closures are conservatively included, across every registry
and regardless of alias/package name or whether currently resolved.

Source trees include all file types, not only Rust: embedded prelude.el/minibuf.el,
other embedded data, and newly added untracked primitive modules are inputs.
Inspection of the current closure's build scripts and include!/include_str!/
include_bytes! paths found no additional non-generated runtime/build inputs outside
these roots. OUT_DIR products are derived from these inputs. The introspection index
also reads tests, Lisp and divergence ledgers for query metadata; those do not define
the live VM registry identity. Its generated registry input is deliberately excluded
to avoid a circular identity.

Top-level crate docs, tests/, evidence/, tools/, and logs outside the selected roots
are not inputs, even when ignored by Git. Within selected trees, deterministic name
rules exclude .git, target and __pycache__ directories, .#* editor locks (before
symlink inspection), #*# autosaves, *~ backups and *.swp/*.swo swap files. No Git
ignore rules or tracked-file history participate: a Git-free source export yields
the same identity. Other symlinked source inputs and missing required inputs fail
closed. Registry and Git dependency resolution is pinned by Cargo.lock; downloaded
external source caches and unrelated workspace crates are not inputs.

Changing the dependency graph automatically expands the closure. New included files
or custom build inputs outside the selected roots require explicitly expanding this
scope. Custom environment, feature or toolchain variations beyond the checked
default/platform contract are not attested by this source identity.

Enumeration uses the input hash baked into Index at compilation, and compares it
with the runtime source hash both before and after probing. A mismatch refuses the
result with a rebuild instruction; an old executable cannot --write provenance for
new source. Source edits require rebuilding and regenerating both snapshots even
when registered names do not change; unrelated docs and evidence edits do not.

`Current` requires a matching input hash and platform and no source drift.
Legacy snapshots remain readable but have no trustworthy v2 fingerprint and
cannot become Current. A different hash/platform with no known registration drift
is `NoDriftDetected`, not proof of parity. Live byte comparison is authoritative.

Regenerate with `cargo run -p romacs-introspect-live -- --write`; verify with
`cargo run -p romacs-introspect-live -- --check` and targeted crate tests. The
mandatory twice-enumeration test proves runtime timestamps cannot change bytes.
