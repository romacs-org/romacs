## Observable by a package

| # | mechanism | GNU | romacs | why |
|---|---|---|---|---|
| VM2-G1A | VM2 one-window display subset | `window.c`/`frame.c` return window/frame objects, store per-window point/start; `xfaces.c:merge_face_ref` merges face lists | stable single-instance symbol tokens, start at 1 and buffer point, full-buffer snapshot despite narrowing; named/singleton faces (longer lists use the first), only foreground/background/family/absolute size projected, limited colour spellings | package-observable staged G1a surface, not general window/face compatibility; [details](crates/romacs-vm/GNU-DIVERGENCES.md#vm2-g1a--one-window-display-subset) | <!-- symbols: selected-window, selected-frame, window-buffer, set-window-buffer, window-start, window-point, frame-pixel-width, frame-pixel-height, defface -->

## VM2-G1A — One-window display subset

G1a implements one VM-owned window and frame, not GNU window/frame objects.
The selected identities are stable VM-interned symbol tokens; GNU returns
window/frame objects, so `symbolp` can observe this limited representation.
There is no splitting, scrolling, minibuffer or per-window point storage yet.
`window-start` is 1 and the snapshot starts at byte zero; narrowing does not
restrict the G1a display view. `window-point` follows the displayed buffer point.
The default frame is Liberation Mono at 20px, white on black, for R102.

`defface` delegates condition collapse/inheritance to `romacs-face::Theme`.
GNU `custom.el`, `cus-face.el`, and `faces.el` are excluded from the native
bootstrap: Romacs owns `defface`, `defcustom`, and `defgroup`. `:type` is
retained only as debugger/documentation/agent metadata; there is no Customize
UI, Custom group machinery, or theme priority stack. `defgroup` records its
documentation, members and keyword metadata, not GNU group registration.
Literal `:initialize #'custom-initialize-delay` (also quoted symbol syntax)
leaves an uninitialized variable unbound and queues its name in
`custom-delayed-init-variables` while that value is a list. With a non-list
value it uses ordinary native variable initialization. Other initialization
functions and Custom setters remain metadata rather than callbacks; native
bootstrap does not yet drain the delayed queue. Seam L exports
foreground/background/family/absolute pixel size only. G1a face properties
accept a named face or a singleton list of a named face; a longer list currently
uses its first element, unlike GNU face-list merging. Only `#rrggbb`, white,
black and red colour spellings are converted by this producer.
These limits are package-observable and are not claims of general GNU display
compatibility. The acceptance scene uses only the supported subset.

### Observable by a package — native declaration boundary

| Surface | Intentional difference from GNU |
|---|---|
| `custom.el`, `cus-face.el`, `faces.el` | Excluded from the native bootstrap; declarations use native lowering, not Customize or GNU's mutable face registry. |
| `defgroup` | Documentation, members and keywords are stored as metadata only; no group registration, membership updates, hooks or Customize UI. |
| `defcustom` | Type/set/get/initialize keywords are metadata, not Custom callbacks. Literal `custom-initialize-delay` defers into a native queue, leaving an uninitialized symbol unbound; no automatic startup drain is implemented yet. A non-list queue uses ordinary `defvar` initialization rather than GNU's reset/set machinery. |


GNU 32.0.50 is the behavioural oracle except for decisions explicitly recorded here. An entry changes an exact expected answer, not whether a test runs or how strictly it compares values.
