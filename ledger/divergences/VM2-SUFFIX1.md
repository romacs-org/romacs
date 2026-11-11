## Detail

| crate | file | entries |
|---|---|---|
| VM2-SUFFIX1 | Invalid representation suffix signals instead of crashing | GNU 32.0.50 `Fget_load_suffixes` (`src/lread.c:961`) exits 139 (SIGSEGV) for `(progn (setq load-file-rep-suffixes (list 42)) (get-load-suffixes))`; its concatenation does not check the element type | VM2's checked `concat` signals `(wrong-type-argument sequencep 42)` instead. Regression `get_load_suffixes_rejects_gnu_crashing_rep_suffix` rejects a mutant that silently returns nil | **Intentional safety divergence:** malformed input is catchable with `condition-case`, never a process crash | <!-- symbols: get-load-suffixes, load-file-rep-suffixes -->
