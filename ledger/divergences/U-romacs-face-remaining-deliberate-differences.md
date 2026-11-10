## Remaining deliberate differences

| mechanism | GNU | romacs | test |
|---|---|---|---|
| `reset` in realized queries | Lisp query can return `reset`; C substitutes default | realized values substitute default, never paint a sentinel | `parity.rs` reset classification |
| anonymous plist `:inherit` | positional; inherited value can override an earlier own attr | own attrs always win | `remap_parity.rs` anonymous-attribute-before-inherit |  <!-- symbols: face-attribute, defface -->
| function heights | invokes Lisp during C merge | rejected: code is not immutable face data | `readiness.rs` removed_font_mechanisms_have_explicit_tests |  <!-- symbols: set-face-attribute, face-attribute -->
| `:font` | window-frame font backend interprets it | ignored here; font objects belong to romacs-font | same test, height remains base 113 |  <!-- symbols: set-face-attribute, face-font -->
| dashed family | setter can split foundry/family | family stays literal `Foo-Bar`, foundry unspecified | same test |  <!-- symbols: set-face-attribute, face-family -->
| undefined parent | C skips; Lisp query can stop at the first missing face | skipped, including Lisp-style query merge | `readiness.rs` missing_parent_does_not_hide_later_parent |  <!-- symbols: face-attribute, defface -->
| `:bold` / `:italic` | C applies keys in order (contrary to docstring) | applies in order, matching C | `parity.rs` corpus |  <!-- symbols: set-face-attribute -->
| window filters | matches using window context | never matches; this View has no window | `remap_parity.rs` filtered fixture |  <!-- symbols: face-remapping-alist -->

The default-face underline reset is independently tested with a base that
has underline **true**; it cannot pass merely because the base supplies nil.
`tests/mutants.json` and `tests/mutants.py` commit exact preimages, replacement
text, target tests and expected assertion markers. Replay receipts belong
outside the source tree and include restored-green runs; compile failures are
not accepted as assertion-red evidence.
