## Observable by a package

| # | mechanism | GNU | romacs | why |
|---|---|---|---|---|
| VM2-RADIX1 | Consumers of radix bignums | Pinned GNU accepts `(+ #x2000000000000000 1)` => `2305843009213693953`, numeric `=`/`<=` of that integer with itself => t, and `(car (read-from-string (prin1-to-string #x2000000000000000)))` => the same integer. | VM2 reads, prints and structurally compares radix bignums, but arithmetic, numeric comparison and min still reject them through the fixnum/float-only numeric consumers. The printed decimal spelling reads back as the same bignum. `%d` now accepts bignums and floating coercions (VM2-FMT1). Integerp/numberp returning t does not imply those consumers support the new representation. | **Package-observable staged numeric consumers:** real arbitrary-precision reader storage, not general bignum arithmetic or serialization parity; reject rather than silently round. | <!-- symbols: read-from-string, prin1-to-string, +, =, <=, min, format -->
