## VM2 host-name bootstrap subset

| # | mechanism | GNU | romacs | why |
|---|---|---|---|---|
| VM2-SYSNAME3 | when the host name is acquired | `init_editfns` sets host state at startup, after a dump reload too, and `init_system_name` returns nil when build details are disabled | Romacs acquires the host name on first access, not when the VM is made, so a change of host name before that access shows in the initial value; a restored image acquires it again on first access. There is no `--no-build-details` host-name policy, so a package can see a name in a reproducible-build regime | Keeps the VM's constructor allocation-free; the build-details policy is deferred | <!-- symbols: system-name -->
