## Raw plist order after multi-key inheritance

`insert-and-inherit` from a neighbour carrying two or more value keys can leave
the inherited character's raw property list in a different ORDER from the one
GNU produces for the same history. Values are identical; only the plist
traversal order differs, and `equal-including-properties` returns t.

The investigation that settled this is worth keeping, because the obvious
reading -- "VM2 is wrong, GNU is right" -- is not what the oracle says.

GNU is deterministic for a FIXED history: padding, no-op edits and copying do
not change its order. But EQUIVALENT histories produce every possible order.
For two of the three measured rows GNU produced six of six possible orders, and
for the third two of two, depending only on how the identical final string was
built. Five public routes to the same string give three different orders, and
`equal-including-properties` returns t across all of them. VM2's output for
each row is an order GNU itself produces for an equivalent history.

GNU's actual rule is that an inherited character copies its SOURCE NEIGHBOUR's
plist order. Matching it exactly is a cheap optional follow-up, not a
correctness fix.

Exposure, measured rather than assumed:
  - GNU's own Lisp contains no order-sensitive comparison of raw plists.
  - Neither do the seventeen installed ELPA packages.
  - GNU's test suites do compare raw plists in five places (term, ansi-color,
    visual-wrap, and fns `object-intervals`), but none reaches them through
    multi-key inheritance.
  - The one idiom that could observe it is
    `(equal (text-properties-at (1- p)) (text-properties-at p))`,
    which is t in GNU and can be nil here. No such code was found.

Detection: a 6,000-case matrix over seeds 53 and 54 found nothing, because it
only ever placed ONE value key on a neighbour. Adding neighbours with two or
more value keys produced 8 order-only differences in 2,000 cases, all from
`insert-and-inherit`, none differing in value. A sweep that cannot produce the
shape cannot clear it.
