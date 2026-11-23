## Observable by a package

| # | mechanism | GNU | romacs | why |
|---|---|---|---|---|
| VM2-TREESIT-QUERY | query roots, capture lists and step indices | can ignore a wildcard root's negated field, silently discard the fourth capture on one step, or drop matches beyond the 16-bit step index | checks the wildcard root's negated fields, emits all captures in query order, and keeps full-width step indices | observable through query results. Each correction follows the query language and has an isolated reproducer in the differential. | <!-- symbols: treesit-query-capture treesit-query-compile treesit-font-lock-fontify-region -->

### Reproductions

GNU's `src/treesit.c` passes compiled queries to the C tree-sitter cursor.
The first two cases use the pinned JSON grammar and input `{"k": 1}`.

- `(pair !key (number) @number) @parent` has no matches in either engine.
  The C cursor's indexed child shortcut also matches the wildcard form
  `(_ !key (number) @number) @parent`. Romacs checks the negated field in
  both forms, so neither matches this input.
- `(number) @a @b @c @d` yields `a`, `b`, `c` through the C engine.
  Romacs yields `a`, `b`, `c`, `d`, in that order, on the same node.
- With input `true`, append `(true) @end` after 32,768 repetitions of
  `(number) @n`. The C compiler accepts it and the cursor drops the final
  match because the starting step index wraps. Romacs yields `end` on
  the `true` node.

A package can observe each through `treesit-query-capture`. The
differential checks the expected difference on each reproducer and
compares every shipped query without exceptions.
