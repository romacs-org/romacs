## Observable by a package

| # | mechanism | GNU | romacs | why |
|---|---|---|---|---|
| VM2-SHORTDOC-LOAD | Loading `shortdoc-doc.el` directly | Pinned GNU autoloads `define-short-documentation-group` from `shortdoc.el`, which requires `shortdoc-doc` while its first form expands: an eager macro-expansion cycle error | `shortdoc-doc.el` loads the macro explicitly, binding `shortdoc--loading-doc` so `shortdoc.el` leaves the groups to its caller; both load orders succeed | Romacs's warm census loads the documentation file directly; matching GNU would make that load fail | <!-- symbols: define-short-documentation-group, shortdoc--loading-doc -->

## Detail

Loading `shortdoc-doc.el` directly in pinned GNU first autoloads its group
definition macro from `shortdoc.el`. The latter requires `shortdoc-doc`
while its first form is still expanding, and GNU signals an eager expansion
cycle. Romacs's native expander used to complete both loads.

The documentation file now loads the macro explicitly when it needs it.
It binds `shortdoc--loading-doc` during that load so `shortdoc.el` can
leave the group definitions to its caller. Loading `shortdoc.el` first
still loads the built-in groups after defining the macro.
