#!/usr/bin/env bash
# Capture every scene in a directory from GNU and romacs, sequentially.
# usage: capture-all.sh SCENE_DIR OUT_DIR ROMACS_BIN RUNTIME_ROOT
set -u
scenes=$1 out=$2 romacs=$3 root=$4
here=$(cd "$(dirname "$0")" && pwd)
mkdir -p "$out"
fail=0
for spec in "$scenes"/*.json; do
  name=$(basename "$spec" .json)
  for ed in gnu romacs; do
    if timeout 120 python3 "$here/capture.py" --editor "$ed" --spec "$spec" \
         --out "$out/$ed/$name.png" --romacs "$romacs" --runtime-root "$root" \
         > "$out/$ed-$name.log" 2>&1; then
      echo "ok   $ed $name"
    else
      echo "FAIL $ed $name (exit $?)"; fail=1
    fi
  done
done
exit $fail
