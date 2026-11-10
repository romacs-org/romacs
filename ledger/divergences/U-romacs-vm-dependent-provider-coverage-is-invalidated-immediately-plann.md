## Dependent provider coverage is invalidated immediately (planned)

The probe fontifies two function definitions, inserts an opening string quote at
the start of the buffer and reads the later definition without redisplay:

```
before-context-edit=(t font-lock-keyword-face)
immediate-dependent-state=(t font-lock-keyword-face)
```

GNU retains both coverage and a stale keyword face until contextual processing.
Romacs will immediately clear dependent coverage; display must recompute before
using it. Old values remain queryable until recomputation. The contextual timer
variables will be no-ops. This requires provider read hulls and syntax state; the
raw storage layer does not claim to implement invalidation.
