#!/usr/bin/env bash
# Re-record GNU's answers for tests/gnutls.rs from the pinned oracle:
#   ROMACS_GNU_ORACLE=/abs/path/to/emacs crates/romacs-vm/tests/fixtures/tls/regenerate.sh
# The oracle must be the pin (scripts/gnu_oracle.py check). The session cases
# and the capability shape are recorded to staging files, and merged into
# gnu-answers.json, with the build that gave them, only if both tests pass.
set -euo pipefail
[ $# -eq 0 ] || { echo "regenerate.sh takes no arguments; set ROMACS_GNU_ORACLE" >&2; exit 2; }
here=$(cd "$(dirname "$0")" && pwd)
root=$(cd "$here/../../../../.." && pwd)
python3 -B "$root/scripts/gnu_oracle.py" check
cd "$root"
ROMACS_GNUTLS_RECORD=1 cargo test --profile gate --locked --offline -p romacs-vm --test gnutls -- \
  tls_sessions_match_pinned_gnu tls_capability_claims_sessions_only --test-threads 1
# Publish only now that both recordings passed: merge the staged files
# and replace the pins in one rename.
python3 - "$here" <<'PY'
import json, os, pathlib, sys
here = pathlib.Path(sys.argv[1])
staged = [here / "gnu-answers.recorded.json", here / "gnu-capability.recorded.json"]
pins = json.loads(staged[0].read_text())
pins.update(json.loads(staged[1].read_text()))
fresh = here / "gnu-answers.json.new"
fresh.write_text(json.dumps(pins, indent=2, ensure_ascii=False) + "\n")
os.replace(fresh, here / "gnu-answers.json")
for path in staged:
    path.unlink()
PY
