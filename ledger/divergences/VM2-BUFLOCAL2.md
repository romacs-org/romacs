## VM2 buffer-local substrate subset

| # | Mechanism | GNU | Romacs | Why |
|---|---|---|---|---|
| VM2-BUFLOCAL2 | Major-mode reset | Resets all native mode state and invokes variable watchers. | Resets represented fields and preserves modeled visit state; absent subsystems and watchers are not simulated, details below. | Missing case/abbrev/redisplay/watcher substrates are not supplied by this lane. | <!-- symbols: kill-all-local-variables, kill-local-variable -->
