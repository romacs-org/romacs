## Aliases

Alias chains are registered through `Deffaces::alias`, rejected if cyclic,
and resolved before inheritance topology, theme/user spec composition,
remap-key binding and named face-reference lookup. Thus the obsolete smerge
alias names remain usable. Ambiguous duplicate canonical keys within one
explicit theme/user value are rejected rather than choosing hash-map order.
The user alist reader likewise rejects duplicate face entries; the builder's
`set_face` explicitly replaces a previous entry. No Custom priority stack
is implied by alias resolution.

Tests: `readiness.rs` inheritance/lookup/remap/ref aliases, alias cycles,
and alias-named theme/user/runtime attribute edits.
