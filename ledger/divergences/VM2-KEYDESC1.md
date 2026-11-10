## Observable by a package

| # | mechanism | GNU | romacs | why |
|---|---|---|---|---|
| VM2-KEYDESC1 | `key-description` prerequisite boolean-vector representation | Pinned GNU accepts boolean vectors as sequences for the initial length checks, then signals `(wrong-type-argument arrayp BOOL-VECTOR)` from `key-description`; `(key-description (make-bool-vector 2 nil))` preserves the actual boolean vector in condition data. | VM2 now has a packed boolean-vector representation, `make-bool-vector`, but no `#&` reader syntax. `key-description` remains unimplemented; representation availability does not establish formatter parity. The pinned 232-observation scope includes this object kind. | Missing `key-description` implementation remains the boundary, not permission to drop that observation or substitute an ordinary vector. Evidence: `key-description-scope/substrate-boundary.json` in task scratch; no key-description parity claimed. | <!-- symbols: key-description, make-bool-vector -->
