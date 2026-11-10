## Observable by a package

| # | mechanism | GNU | romacs | why |
|---|---|---|---|---|
| VM2-FR5-4 | no arithmetic bignum promotion at `make-vector`'s argument | GNU reads `(1+ most-positive-fixnum)` as a bignum and `make-vector` rejects it with `(wrong-type-argument wholenump 2305843009213693952)` (same for `(- most-negative-fixnum 1)`) | VM2 does not promote arithmetic results to its radix bignum representation: computing the argument signals `(arith-error)` before `make-vector` is called. Pinned: `make-vector/0502`, `make-vector/0503` (class `CONDITION`) | **Package-observable** wherever integers leave the fixnum range; not a `make-vector` defect | <!-- symbols: make-vector, 1+ -->
