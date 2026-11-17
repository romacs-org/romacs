## VM2 host-name bootstrap subset

| # | mechanism | GNU | romacs | why |
|---|---|---|---|---|
| VM2-SYSNAME1 | `system-name` and GNU's identity cache | `src/editfns.c:1283` refreshes the kernel host name when `Vsystem_name` is EQ to its cached object (`src/sysdep.c:1608`), repairing in-place string edits and noticing kernel host-name changes | Romacs makes the forwarded initial value on first access and returns its current binding as it is, without refresh: `(let ((before (copy-sequence system-name))) (aset system-name 0 88) (equal (system-name) before))` is `t` in pinned GNU, `nil` in Romacs (`crates/romacs-vm/tools/primh/evidence/system-name.json`, case `system-name/0010`) | The startup call needs the name; explicit string, non-string and nil overrides are kept, the cache is deferred | <!-- symbols: system-name -->
