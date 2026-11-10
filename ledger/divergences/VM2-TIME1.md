## Observable by a package

| # | mechanism | GNU | romacs | why |
|---|---|---|---|---|
| VM2-TIME1 | `current-time` and timestamp bignum output | `timefns.c:1817,820,720` returns exact realtime nanoseconds as `(TICKS . 1000000000)` with nil `current-time-list`, promoting TICKS to a bignum as needed; non-nil selects `(HIGH LOW USEC PSEC)` | VM2 `current-time` preserves every native nanosecond in either representation, but nil format signals `(error "VM2 current-time requiring bignum ticks is unsupported")` exactly when TICKS is outside `[-2305843009213693952, 2305843009213693951]`. Non-nil format is exact without bignum ticks; a HIGH outside the same fixnum interval explicitly signals `VM2 current-time requiring bignum HIGH is unsupported` (not reachable with this host's native clock range). Existing `visited-file-modtime` has the analogous tick refusal, and timestamp consumers still do not accept reader bignums | Package-observable numeric-substrate limitation, never float rounding, clamping, frequency reduction or an automatic list fallback. Bind `current-time-list` non-nil to request GNU's lossless list representation. `current-time` is a realtime clock, not a monotonic-clock promise | <!-- symbols: current-time, current-time-list, visited-file-modtime, set-visited-file-modtime -->

## VM2-TIME1 — Timestamp bignums require the missing integer substrate

GNU `fileio.c:6157` and `timefns.c:820` at revision
`45b0f7699df36b3849edc3bdef37d4cb9e75b9ea` return nanosecond ticks by default,
including bignums when needed. VM2 stores arbitrary-precision integers read
from source constants, but its timestamp substrate consumes only fixnums and
no arithmetic produces a bignum. For example:

```elisp
(set-visited-file-modtime '(40000 0))
(visited-file-modtime)
```

GNU returns `(2621440000000000000 . 1000000000)`; VM2 signals
`(error "VM2 visited-file-modtime requiring bignum ticks is unsupported")`.
The native recorded timestamp remains intact and can still be retrieved with
`current-time-list` non-nil. The reader now accepts bignum literals, but bignum
input timestamps remain unsupported by the numeric substrate that consumes them. This is an unresolved package-observable
compatibility gap, not an alternate valid GNU result. Fixnum timestamp forms,
legacy dotted microseconds, ignored trailing components and finite-float
nanosecond flooring otherwise follow the pinned implementation.

`current-time` (`timefns.c:1817`, encoding at :820/:720) now reads the
native realtime clock with integer nanosecond precision. With nil
`current-time-list`, its `(TICKS . 1000000000)` output is exact while
`-2305843009213693952 <= TICKS <= 2305843009213693951`; outside that
interval it signals exactly
`(error "VM2 current-time requiring bignum ticks is unsupported")`.
No frequency reduction, rounding, clamping, or automatic list fallback occurs.
With **any non-nil** `current-time-list`, it returns GNU's exact four-fixnum
list `(HIGH LOW USEC PSEC)`, including negative pre-epoch normalization.
This works even when nanosecond ticks require a bignum. HIGH outside the
same fixnum interval signals
`(error "VM2 current-time requiring bignum HIGH is unsupported")`;
that branch is unreachable for the native clock's range on this host.
These refusals are numeric representation gaps, not GNU conditions.
Clock changes can make successive realtime readings decrease in either VM;
this primitive does not promise a monotonic clock.

The deterministic encoder boundary tests and live clock contract tests live
in `tests/current_time.rs` and the `prims::time::tests` unit module. The pinned
GNU probe `tests/current_time_oracle.el` confirms the exact encodings, including
GNU's successful bignum results on both sides of VM2's refusal boundary.
