## Observable by a package

| # | mechanism | GNU | romacs | why |
|---|---|---|---|---|
| VM2-IMAGE-CANVAS | `image-types`, `init-image-library` | The pinned GNU build lists `canvas` (the Android/HTML canvas image type) in `image-types`, and `init-image-library` accepts it. | `canvas` is not an image type: `image-types` is GNU's other nine, `(init-image-library 'canvas)` and `imagep` on a canvas spec are nil. Every other type's validation, size, mask and metadata follow pinned GNU (`harness/images_live.py`). | No Romacs host draws a canvas; claiming the type would make `image-type-available-p` lie. | <!-- symbols: image-types, init-image-library -->
