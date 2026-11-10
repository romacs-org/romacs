## Detail

| crate | file | entries |
|---|---|---|
| VM2-VARALIAS2 | `defvaralias` overwrite warning is printed, not dispatched | When both variables are bound to different values GNU calls `display-warning` with `(defvaralias losing-value NEW)` and the formatted message (`src/eval.c:686-698`); in batch that prints `Warning (defvaralias): Overwriting value of ‘NEW’ by aliasing to ‘BASE’` to stderr | VM2 writes that same line to stderr directly; `display-warning` is not called, so `warning-minimum-level`, `warning-suppress-types` and a redefined `display-warning` have no effect, and an interactive session gets no `*Warnings*` buffer | **Package-observable** only through warning customisation or a `display-warning` override; the variable state after the call is GNU's | <!-- symbols: defvaralias, display-warning -->
