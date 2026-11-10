## Heights: use the C merge for redisplay

`Theme::realize`, `View::effective`, `View::realize` and `View::realize_ref`
replay GNU's C merge,
parents cdr-first and own attributes last, truncating each relative-to-absolute
height operation. They must not pre-multiply all scales. The review's
`:height 1.5 :inherit (relative absolute)` example gives **235**, not Lisp's
236. Over default 113, two successive 1.5 scales give **253**, not 254.
`face_attribute` retains the separate Lisp height-query grouping.

`tests/gnu/redisplay.el` produced `tests/gnu/redisplay.eld` using GNU's actual
`face-attributes-as-vector`, not `face-attribute` as a stand-in for redisplay.
Its 16 rows cover default heights 100, 113, 97 and 120, with separate C
unspecified/absolute-base results and Lisp query results. Regenerate only
under `-Q` and an isolated HOME; review the fixture diff.

The older 25-theme plus unthemed fixtures remain useful for selection and
attributes, but mostly compare unspecified/default values. Their default
height is 1, so most face-specific heights truncate to 0. They are **not**
a substitute for the non-unit C-path oracle. `remap_parity.rs` retains the
old 24 Lisp-query height values explicitly as Lisp queries.
