## Observable by a package

| # | mechanism | GNU | romacs | why |
|---|---|---|---|---|
| VM2-LABELED-WIDEN-POINT | `widen` inside `with-restriction :label` after text was inserted at the labeled end | sets BEGV and ZV to the labeled bounds and leaves point where it was, past ZV: `(point point-min point-max)` is `(6 2 4)` after inserting "XY" at 4 within 2..4 | the same bounds, with point moved to ZV, as every narrowing moves it: `(4 2 4)` | a buffer's point stays within its accessible region, which the text and the display rely on | <!-- symbols: widen, with-restriction -->

## VM2-LABELED-WIDEN-POINT — Labeled widening keeps point accessible

editfns.c `Fwiden`, in a labeled restriction, sets the bounds from the
innermost restriction's markers without `SET_PT`. Those markers have
insertion type nil, so text inserted at the labeled end lies past the bounds
`widen` restores, and point, which followed the insertion, ends outside the
accessible region until something moves it. Romacs's buffers hold point
within their restriction; `widen` narrows to the labeled bounds as
`narrow-to-region` does, so point ends at ZV.
