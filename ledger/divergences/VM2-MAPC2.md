## Observable by a package

| # | mechanism | GNU | romacs | why |
|---|---|---|---|---|
| VM2-MAPC2 | `mapc` closure printing/regime | GNU's compiler represents lambda values as bytecode closures, with an interpreted alternate recorded by primh. | `mapc/0081`, `/0088`, `/0089` cannot print the returned closure/lambda under the VM2 CLI printer, and are pinned `PRINTER`. | Package-observable inherited callable representation/printer gap, not claimed closure-vector sequence parity. List/vector/string mutation, return identity and callback faults are tested separately with printable results. | <!-- symbols: mapc, prin1-to-string -->
