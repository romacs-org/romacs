#!/usr/bin/env bash
# Run only through neomacs-slot props --group props --light -- bash THIS.
set -euo pipefail
# Toolchain: cargo/rustfmt from PATH (the repo's pinned rust-toolchain).
command -v cargo >/dev/null || { echo "cargo not on PATH" >&2; exit 1; }
set -o pipefail
HARNESS_DIR=$(CDPATH= cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)
REPO=$(git -C "$HARNESS_DIR" rev-parse --show-toplevel)
export P1_SCRATCH_ROOT=${P1_SCRATCH_ROOT:-${TMPDIR:-$HOME/.cache}/props-bench}
mkdir -p "$P1_SCRATCH_ROOT"
ROOT=${P1_BENCH_ROOT:-$P1_SCRATCH_ROOT/P1-bench-$(date +%Y%m%d-%H%M%S)-$$}
python3 "$HARNESS_DIR/P1-bench.py" prepare --repo "$REPO" --root "$ROOT"
# Independent driver and library manifests, independent targets and lockfiles.
# Do not reuse the slot's ambient target/profile/rustflags overrides.
unset CARGO_ENCODED_RUSTFLAGS RUSTFLAGS CARGO_BUILD_RUSTFLAGS RUSTC_WRAPPER RUSTC_WORKSPACE_WRAPPER
export CARGO_PROFILE_RELEASE_OPT_LEVEL=3
export CARGO_PROFILE_RELEASE_LTO=off
export CARGO_PROFILE_RELEASE_CODEGEN_UNITS=1
export CARGO_PROFILE_RELEASE_DEBUG=1
export CARGO_PROFILE_RELEASE_DEBUG_ASSERTIONS=false
export CARGO_PROFILE_RELEASE_OVERFLOW_CHECKS=false
export CARGO_INCREMENTAL=0
mkdir "$ROOT/frozen"
rustc -vV > "$ROOT/rustc-version.txt"
cargo -vV > "$ROOT/cargo-version.txt"
for arm in old candidate; do
    flags='-C force-frame-pointers=yes --check-cfg=cfg(p1_candidate)'
    if [[ $arm == candidate ]]; then flags="$flags --cfg p1_candidate"; fi
    (
        cd "$ROOT/$arm"
        env CARGO_TARGET_DIR="$ROOT/target-$arm" RUSTFLAGS="$flags" \
            cargo build --release --manifest-path "$ROOT/$arm/Cargo.toml" -vv \
            2>&1 | tee "$ROOT/build-$arm.log"
    )
    cp "$ROOT/target-$arm/release/props-p1-driver" "$ROOT/frozen/$arm"
    chmod a-w "$ROOT/frozen/$arm"
    cp -r "$ROOT/target-$arm/release/.fingerprint" "$ROOT/fingerprints-$arm"
done
sha256sum "$ROOT/frozen/old" "$ROOT/frozen/candidate" > "$ROOT/binaries.sha256"
printf 'BUILD_SUCCESS %s\n' "$ROOT" | tee "$ROOT/BUILD_SUCCESS"
# P1_RUN=0 allows the parent to build now and run the Python entry point later.
if [[ ${P1_RUN:-1} != 0 ]]; then
    python3 "$HARNESS_DIR/P1-bench.py" run --root "$ROOT" --reps "${P1_REPS:-5}" --perf "${P1_PERF:-perf}"
fi
