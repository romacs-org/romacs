## VM2 buffer-local substrate subset

| # | Mechanism | GNU | Romacs | Why |
|---|---|---|---|---|
| VM2-BUFLOCAL2 | Major-mode reset | Resets all native mode state and invokes variable watchers. | Resets represented fields and preserves modeled visit state; absent subsystems and watchers are not simulated, details below. | Missing case/abbrev/redisplay/watcher substrates are not supplied by this lane. | <!-- symbols: kill-all-local-variables, kill-local-variable -->

GNU
`src/buffer.c:1137,3031` resets case tables, abbrev tables, invisibility state,
redisplay flags and (for `reset`) the entire native buffer state; it also
notifies variable watchers except for the documented suppressing arguments.
VM2 runs `change-major-mode-hook`, removes eligible ordinary/forwarded
locals, preserves permanent hooks and modeled visit metadata, and resets
major-mode/mode-name, local keymap and syntax table. `reset` additionally
clears modeled file name/read-only/format and visit timestamp/identity/
truename metadata. Missing case/abbrev/redisplay state and variable watchers
are not reset/notified. Packages can observe this. Why: the missing native subsystems are outside
the buffer-local storage slice, rather than silently claimed as implemented.
