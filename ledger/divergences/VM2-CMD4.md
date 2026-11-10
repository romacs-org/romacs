## Observable by a package

| # | mechanism | GNU | romacs | why |
|---|---|---|---|---|
| VM2-CMD4 | VM2 control-space representation | `keyboard.c:make_lispy_event` and live GNU 32.0.50 `(kbd "C-SPC")` yield `[67108896]`, while C-@ is zero | Existing VM2 `key_modifier`/`kbd` encode C-SPC as zero; the native adapter deliberately uses that same zero so bindings installed through `(kbd "C-SPC")` work. Already encoded control bytes 0..31/127 from the renderer are preserved, including with Ctrl set | **Package-observable inherited key-encoding difference**, not a GNU representation claim. The GUI oracle binds the same symbolic C-SPC in both editors and drives the real native keyboard | <!-- symbols: kbd, key-binding, last-command-event, this-command-keys -->
