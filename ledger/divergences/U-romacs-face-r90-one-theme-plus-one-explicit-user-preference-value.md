## R90: one theme plus one explicit user preference value

GNU replays enabled themes and its always-enabled `user` theme,
`custom-set-faces` and `theme-face` records. We have **none** of that stack.
`Theme::build_with_overrides` composes one `ThemeSpec` with one
`Arc<UserOverrides>`. Unspecified user slots retain the theme's values;
explicit user inheritance replaces the inherited-name list. Changing the
builder cannot mutate an already-built theme. Theme replacement retains
the same user preferences; transient runtime setter edits are discarded.

`examples/user-overrides.eld` is the port of one user's eight face
preferences from custom.el: Iosevka 120 default, black fringe/dividers,
Iosevka italic and variable-pitch, and Iosevka Aile org-modern-symbol.
It is **plain data**, loaded with `BoundedDatum::parse` and
`UserOverrides::from_alist`, not an Elisp Custom form. The old `t` theme
flag on org-modern-symbol disappears: a new explicit preference face may
be added even before its authoring package loads. No user config is edited
by this crate. `readiness.rs::owner_preferences_compose_over_bundled_modus`
builds the bundled modus-vivendi with this exact file and checks all eight
outcomes while preserving the theme's default background.

A built theme swap is an Arc replacement; a rebuild is not. All old numeric
speed comparisons between a prebuilt swap and GNU's full enable/disable
rebuild are withdrawn. No speed advantage is claimed here. Runtime setters
and late defface still rebuild the theme and mint a new generation; cached
realization misses replay the C merge iteratively. Batch mutations before
adding pulse-style per-tick callers; do not infer O(1) setters from O(1) swaps.
