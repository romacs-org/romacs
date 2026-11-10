## Detail

| crate | file | entries |
|---|---|---|
| VM2-SPECIAL-ALIAS1 | A symbol whose function is a special form | `eval_sub` (`src/eval.c`) dispatches on the head's function when the form is evaluated: `(defalias 'my-progn (symbol-function 'progn))` makes `(my-progn ...)` the special form, as byte-run.el's `(defalias 'inline 'progn)` makes `inline` | A call whose head names, through aliases, a special form is compiled as that form, when the form is compiled (`Vm::special_form_alias`); an alias made later, or by the same top-level form that calls it, is called as a function and signals `(invalid-function #<subr progn>)` | **Package-observable** only for a special-form alias made after the code that calls it is compiled; bytecomp's `inline` and every alias GNU's Lisp makes before use behave as in GNU (`tests/bytecomp_function_value.rs`) | <!-- symbols: inline, defalias, special-form-p -->
