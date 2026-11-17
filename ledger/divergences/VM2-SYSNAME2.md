## VM2 host-name bootstrap subset

| # | mechanism | GNU | romacs | why |
|---|---|---|---|---|
| VM2-SYSNAME2 | startup host acquisition | `src/sysdep.c:1608` takes the kernel's raw bytes by `gethostname` (or `uname`), normalizes spaces and tabs, and lets `build_string` detect multibyte text | Romacs reads `/proc/sys/kernel/hostname` as UTF-8, normalizes spaces and tabs and makes a unibyte string; no procfs, a read error or invalid UTF-8 gives `"unknown"`, signalling nothing. A non-ASCII host name can differ in multibyteness, and a host without procfs sees the fallback | The proven target is a Linux host with an ASCII name; the other configurations have not been compared with live GNU | <!-- symbols: system-name -->
