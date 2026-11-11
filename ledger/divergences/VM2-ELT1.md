## fns.c sequence access boundary

| # | Scope | GNU | VM2 | Reason / evidence |
|---|---|---|---|---|
| VM2-ELT1 | `elt` absent object constructors | `Felt` (fns.c:1853) accepts bool-vectors and rejects records/byte-code objects with `wrong-type-argument sequencep`; `Faref` accepting the latter does not make them arrays for Felt. | Lists, strings, vectors, bool-vectors and existing char-tables work. The two bool-vector missing pins were removed after live GNU replay. Remaining record/byte-compile construction witnesses retain their existing limitations. These are not fake vectors or successful nil results, and the construction errors are NOT claimed to equal GNU's elt errors. | Remaining construction boundaries are recorded in `tools/primh/evidence/elt.json`. Closure rejection itself agrees with GNU when only condition and predicate are inspected. | <!-- symbols: elt -->
