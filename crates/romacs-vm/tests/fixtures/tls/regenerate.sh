#!/usr/bin/env bash
# Re-record GNU's answers for tests/gnutls.rs from the pinned oracle:
#   ROMACS_GNU_ORACLE=/abs/path/to/emacs crates/romacs-vm/tests/fixtures/tls/regenerate.sh
# The oracle must be the pin (scripts/gnu_oracle.py check). The session cases
# are written to gnu-answers.json with the build that gave them; the
# capability shape is recorded beside them and merged in.
set -euo pipefail
[ $# -eq 0 ] || { echo "regenerate.sh takes no arguments; set ROMACS_GNU_ORACLE" >&2; exit 2; }
here=$(cd "$(dirname "$0")" && pwd)
root=$(cd "$here/../../../../.." && pwd)
python3 -B "$root/scripts/gnu_oracle.py" check
cd "$root"
ROMACS_GNUTLS_RECORD=1 cargo test --profile gate --locked --offline -p romacs-vm --test gnutls -- \
  tls_sessions_match_pinned_gnu tls_capability_claims_sessions_only --test-threads 1
python3 - "$here" <<'PY'
import json, pathlib, sys
here = pathlib.Path(sys.argv[1])
pins = json.loads((here / "gnu-answers.json").read_text())
extra = here / "gnu-capability.json"
pins.update(json.loads(extra.read_text()))
extra.unlink()
(here / "gnu-answers.json").write_text(json.dumps(pins, indent=2, ensure_ascii=False) + "\n")
PY
