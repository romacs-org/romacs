#!/usr/bin/env bash
# Ledger 214: check that a GNU Emacs is the one this project's parity numbers
# are measured against, and print the stamp a published number must carry.
#
#   tools/parity-reference-attest.sh EDITOR [fingerprint|exhaustive]
#
# Prints ONE line on stdout -- the stamp -- and exits 0 when the editor is the
# pin.  On any disagreement it writes a named refusal to stderr and exits 3.
# `ROMACS_PARITY_REFERENCE=none' declares that no pin is available: the stamp
# then says UNATTESTED and the exit is 0, so a number can still be taken, but
# it can never be mistaken for a comparable one.
#
# A shell reader of the same manifest as `romacs-parity-reference', for a
# caller that should not build Rust.  That crate's tests run THIS script over
# planted fixtures and require it to agree with the Rust reader on every one.
#
# The manifest format is deliberately tiny (see ledger/parity-reference.toml).  This
# parser REFUSES a line it does not understand rather than skipping it: a pin
# that silently stops being read is the failure this whole file exists to
# prevent.
set -u

usage() {
  echo "usage: $0 [--if-gnu] EDITOR [fingerprint|exhaustive]" >&2
  exit 2
}

# --if-gnu: attest only when the editor IS a GNU Emacs, and say so otherwise.
#
# The single-editor runners are handed either peer and cannot know the role, so
# they ask this instead of guessing.  A GNU Emacs is identified by the dump it
# loads: src/emacs.c:1104-1120 looks for basename(argv0) + ".pdmp", and that
# file starts with the magic src/pdumper.c:116 writes.  This port's own dump is
# `romacs.pdump' and is not one, so the two peers are told apart by an
# artifact GNU itself defines rather than by a name or a path.
#
# A GNU whose dump is MISSING is not quietly reclassified as "not GNU": it is
# refused below, and it could not have run anyway -- measured, a GNU with its
# dump removed exits 255 before evaluating anything, which is the failure
# ledger 211 section 10.1 already reports as "the EDITOR could not be RUN".
only_if_gnu=0
if [ "${1:-}" = --if-gnu ]; then
  only_if_gnu=1
  shift
fi

[ "$#" -ge 1 ] || usage
editor="$1"
depth="${2:-fingerprint}"
case "$depth" in
  fingerprint | exhaustive) ;;
  *)
    echo "parity reference: unknown depth ${depth}; expected fingerprint or exhaustive" >&2
    exit 2
    ;;
esac

root="$(cd "$(dirname "$0")/.." && pwd)"
manifest="${ROMACS_PARITY_REFERENCE_FILE:-$root/ledger/parity-reference.toml}"

refuse() {
  echo "parity reference: $*" >&2
  exit 3
}

# The opt-out is exact.  A variable that disables a guard must not be able to
# be disabled and misspelled at the same time.
if [ -n "${ROMACS_PARITY_REFERENCE:-}" ]; then
  if [ "$ROMACS_PARITY_REFERENCE" = none ]; then
    echo "gnu=UNATTESTED (ROMACS_PARITY_REFERENCE=none) attest=none"
    exit 0
  fi
  refuse "ROMACS_PARITY_REFERENCE=\"$ROMACS_PARITY_REFERENCE\" is not understood;" \
         "the only accepted value is \"none\", which brands every number produced as UNATTESTED"
fi

[ -f "$manifest" ] || refuse "cannot read the pin at $manifest: no such file"

# ---------------------------------------------------------------------------
# Parse the manifest, strictly
# ---------------------------------------------------------------------------
m_schema= m_emacs_version= m_mirror_commit= m_build_time=
m_fingerprint= m_executable_sha256= m_executable_size=
m_pdmp_sha256= m_pdmp_size=
lineno=0
while IFS= read -r line || [ -n "$line" ]; do
  lineno=$((lineno + 1))
  line="${line%$'\r'}"
  trimmed="${line#"${line%%[![:space:]]*}"}"
  case "$trimmed" in
    '' | '#'*) continue ;;
  esac
  case "$line" in
    *' = "'*'"') ;;
    *) refuse "$manifest line $lineno: expected 'key = \"value\"', got: $line" ;;
  esac
  key="${line%% = *}"
  value="${line#* = }"
  value="${value#\"}"
  value="${value%\"}"
  case "$key" in
    *[!a-z0-9_]* | '') refuse "$manifest line $lineno: '$key' is not a lower-snake key" ;;
  esac
  case "$value" in
    *'"'* | *'\'*) refuse "$manifest line $lineno: value has a quote or backslash; the format has no escapes" ;;
  esac
  case "$key" in
    schema | emacs_version | mirror_commit | build_time | fingerprint | \
      executable_sha256 | executable_size | pdmp_sha256 | pdmp_size) ;;
    *) refuse "$manifest line $lineno: unknown key '$key' for schema 1" ;;
  esac
  eval "current=\${m_$key}"
  [ -z "$current" ] || refuse "$manifest line $lineno: duplicate key '$key'"
  eval "m_$key=\$value"
done < "$manifest"

for key in schema emacs_version mirror_commit build_time fingerprint \
  executable_sha256 executable_size pdmp_sha256 pdmp_size; do
  eval "value=\${m_$key}"
  [ -n "$value" ] || refuse "$manifest: missing key '$key'"
done
[ "$m_schema" = 1 ] || refuse "$manifest: schema '$m_schema' is not understood by this script; expected 1"

check_hex() {
  # $1 = key name, $2 = value, $3 = required length
  case "$2" in
    *[!0-9a-f]*) refuse "$manifest: key '$1' must be lowercase hex, got '$2'" ;;
  esac
  [ "${#2}" -eq "$3" ] || refuse "$manifest: key '$1' must be $3 hex digits, got ${#2}"
}
check_hex mirror_commit "$m_mirror_commit" 40
check_hex fingerprint "$m_fingerprint" 64
check_hex executable_sha256 "$m_executable_sha256" 64
check_hex pdmp_sha256 "$m_pdmp_sha256" 64
case "$m_executable_size" in *[!0-9]* | '') refuse "$manifest: executable_size is not an integer" ;; esac
case "$m_pdmp_size" in *[!0-9]* | '') refuse "$manifest: pdmp_size is not an integer" ;; esac

# ---------------------------------------------------------------------------
# Resolve the editor the way a shell would, then canonicalize it
# ---------------------------------------------------------------------------
# The pin is reached through a symlink (~/.local/bin/emacs), and it was a
# BROKEN one of those that caused the incident behind ledger 211 section 10.1.
# In --if-gnu mode an editor that cannot be resolved is NOT this script's
# finding.  Ledger 211 section 10.1 bought the distinction between "the EDITOR
# could not be RUN" (exit 127, reported by the runner) and "it ran and wrote
# nothing", and an attestation that refused first would take that diagnosis
# away.  The sweep, which KNOWS which side is GNU, uses the strict mode and
# still refuses -- the guarantee lives where the role is known.
not_the_reference() {
  if [ "$only_if_gnu" = 1 ]; then
    echo "gnu=n/a (peer is not the pinned reference) attest=n/a"
    exit 0
  fi
  refuse "$@"
}

case "$editor" in
  */*) candidate="$editor" ;;
  *)
    candidate="$(command -v -- "$editor" 2> /dev/null)" ||
      not_the_reference "the EDITOR could not be resolved: $editor -- not found on PATH"
    ;;
esac
resolved="$(readlink -f -- "$candidate" 2> /dev/null)" || resolved=
[ -n "$resolved" ] && [ -f "$resolved" ] ||
  not_the_reference "the EDITOR could not be resolved: $candidate -- not found, not a file, or a broken symlink"

# GNU's dump search (src/emacs.c:1027-1120): beside the canonical executable
# first (the build tree), then the installed libexec tree, which
# Makefile.in:628-630 fills with emacs-<fingerprint>.pdmp.  PATH_EXEC is
# compiled in and not knowable without running the editor -- and --if-gnu
# must classify a peer without running it -- so the installed half is
# recognised relative to the binary's prefix, including the libexecdir=lib
# distro spelling.  LC_ALL=C keeps the choice byte-ordered, agreeing with the
# Rust reader (romacs-parity-reference `dump_for`) when a prefix holds more
# than one dump.
locate_dump() {
  local beside="$1.pdmp"
  if [ -f "$beside" ]; then
    printf '%s\n' "$beside"
    return 0
  fi
  local prefix found
  prefix=$(dirname -- "$(dirname -- "$1")")
  found=$(
    find "$prefix/libexec/emacs" "$prefix/lib/emacs" -type f \
      \( -name 'emacs-*.pdmp' -o -name 'emacs.pdmp' -o -name 'Emacs.pdmp' \) \
      2> /dev/null | LC_ALL=C sort | head -n 1
  )
  [ -n "$found" ] || return 1
  printf '%s\n' "$found"
}

pdmp=$(locate_dump "$resolved") ||
  not_the_reference "the editor resolved but its dump is missing at $resolved.pdmp"

mismatch() {
  # $1 = field, $2 = path, $3 = pinned, $4 = found
  {
    echo "parity reference MISMATCH on $1 for $2"
    echo "  pinned: $3"
    echo "  found:  $4"
    echo "  This GNU is NOT the one this project's parity numbers are measured"
    echo "  against, so a number scored against it is not comparable with any"
    echo "  published one.  A rebuild of the GNU mirror is its own change with"
    echo "  its own re-baselining: run"
    echo "    cargo run -p romacs-parity-reference --bin pin-reference -- --emacs PATH --reason \"...\""
    echo "  to re-pin deliberately, or set ROMACS_PARITY_REFERENCE=none to"
    echo "  measure without a pin and have every number branded UNATTESTED."
  } >&2
  exit 3
}

# struct dump_header: char magic[16] then unsigned char fingerprint[32]
# (src/pdumper.c:361-367); the magic itself is src/pdumper.c:116.  The magic is
# checked FIRST so that --if-gnu can tell a non-GNU peer from a wrong GNU
# before anything is run.
magic="$(head -c 14 -- "$pdmp")"
[ "$magic" = DUMPEDGNUEMACS ] ||
  not_the_reference "$pdmp is not a GNU dump file: magic is '$magic', expected 'DUMPEDGNUEMACS'"

# THE IDENTITY IS THE RELEASE.  An installed GNU embeds no repository revision
# -- `emacs-repository-version` computes one by running git in
# `source-directory` (lisp/version.el), which `make install` does not create --
# and make-fingerprint hashes the temacs binary, so a rebuild on another
# toolchain changes the fingerprint without the source moving.  The pin is the
# tagged release (emacs-31.1), and that is what the editor reports; the
# manifest's fingerprint and SHA-256 fields are provenance, recorded by
# pin-reference, not gates.
editor_version="$("$resolved" --batch --quick --eval '(princ emacs-version)' 2> /dev/null)" ||
  refuse "the editor resolved but did not run: $resolved"
[ "$editor_version" = "$m_emacs_version" ] ||
  mismatch "emacs version" "$resolved" "$m_emacs_version" "$editor_version"

# The stamp.  Ledger 210 made every count carry the geometry it was measured
# in; this is the same rule applied to the reference.
printf 'gnu=%s fingerprint=%s mirror=%s attest=%s\n' \
  "$m_emacs_version" "${m_fingerprint:0:12}" "${m_mirror_commit:0:11}" "$depth"
