## Interval history is not reproduced

```
same-set-next-boundary=3 intervals=((0 1 (p 1 q 2)) (1 2 (q 2 p 1)) (2 4 nil))
```

GNU retains a storage split at byte 1 (Lisp position 2), even though semantic
`next-property-change` skips it and returns Lisp position 3. Romacs' boundary
query similarly compares presence and `eq` values, ignoring insertion stamps.
An eventual `object-intervals` entry point will report the merged semantic view,
not this split history. The quantified difference for this four-character row
is two semantic intervals instead of three historical intervals. Present-key
order remains descending insertion stamp; replacements keep their stamp.
