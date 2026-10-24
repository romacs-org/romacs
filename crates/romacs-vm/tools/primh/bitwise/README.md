# logior bootstrap frontier

Base: `vm2/radix-reader` / `10b484302d7971453c06989f5070413d16003451`.
GNU source and binary revision: `45b0f7699df36b3849edc3bdef37d4cb9e75b9ea`.

`oracle.py` uses `scripts/gnu_oracle.py` to verify the recording binary's own
revision and records its executable SHA256. `cases.json` and the recorder are
hashed in `gnu.json`. All expected values and complete condition payloads are
recorded, not handwritten. The 29 rows cover empty/many arguments, overlap,
negative and positive bignums, normalization, single-argument identity,
first-versus-later type predicates, floats, error ordering and markers.

## Verification

```sh
cargo test -p romacs-vm --test bitwise -- --nocapture
cargo test -p romacs-vm --test bootstrap_descriptors
```

`mutate.py` requires a nonzero test exit, `BITWISE_GNU_MISMATCH`, and a mismatch
in the intended oracle row. Overlapping bit patterns distinguish OR from XOR.
The GC test roots an allocated result, proves its index moved, and compares its
forwarded value with GNU. No assertion is disabled for an unsupported case.

GNU implementation: `data.c:3506-3515`, `:2688-2702`, `:3199-3304`. Leaf
allocation cannot collect; integer payloads are owned BigInts. The returned Value
is published in traced result storage before a wait point. GNU Lisp bootstrap
frontiers are checked in `bootstrap_descriptors.rs`, not inferred from the
existence of a native primitive. Run outputs belong under `target/evidence/`.
