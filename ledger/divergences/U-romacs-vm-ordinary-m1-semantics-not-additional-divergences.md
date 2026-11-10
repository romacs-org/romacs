## Ordinary M1 semantics, not additional divergences

Replay the checked-in GNU answers with a private Emacs home:

```
python3 crates/romacs-vm/tools/annotation_m1_oracle.py --oracle emacs
```

The runner creates a private HOME/XDG tree even when invoked from a normal
shell. `tests/annotations.rs` replays the same forms in the VM, including raw
property lists and callback parameter identity. GNU observations include:

```
resolve-nil=(default default nil)
resolve-(p nil)=(nil nil (p nil))
resolve-(q alias)=(alias alias (q alias))
resolve-(category cat)=(category category (category cat))
category-nil=alias
alias-not-recursive=nil
kill-ring=#("xy" 0 2 (secret t p value))
yank-props=(p value)
handler-param="ALT"
noexclude=(rear-nonsticky t p v yank-handler (nil nil t))
exclude-all=nil
```

A directly present nil masks category/aliases/defaults; nil from a category or
an alias does not. Aliases are not recursively expanded. Defaults are resolved
per key without being stored as raw properties. Plain `insert` grafts string
properties but does not inherit neighbours. `insert-and-inherit` keeps inherited
keys, even nil, over conflicting string values; missing string keys are prepended
in a plist traversal order GNU also produces, though not necessarily the one GNU
produces for this history (see "Raw plist order after multi-key inheritance"
below). Rear/front stickiness is per property, including
`text-property-default-nonsticky` overrides.

Kill-ring strings retain the original properties. Exclusion happens in the
destination after each yank-handler invocation; NOEXCLUDE bypasses it. FUNCTION,
PARAM, UNDO and COMMAND fields are honoured; the function's return is ignored.
Handler runs are processed incrementally so a callback can mutate the unprocessed
suffix. Calls use ordinary VM function/closure dispatch, not reentrant native
execution or macro replacement, and observe function redefinition and GC.

Default and custom `yank-handled-properties` callbacks run over property-value
runs before exclusion. The defaults materialize `font-lock-face` into `face`
when font locking is absent, and materialize category plists before `category`
is removed. NOEXCLUDE skips both handling and exclusion. Temporary buffers start
without a buffer-local font-lock configuration, rather than inheriting another
buffer's dynamic value.

This does not claim a complete interactive command loop, clipboard integration,
undo engine, mark handling or yank-pop. Those require their owning editor layers.
`get-char-property` consults the overlay index before resolving text properties
through category, alias and default fallbacks. None of the remaining boundaries
is an additional licensed GNU divergence.
