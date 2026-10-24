//! Native GNU 32.0.50 -Q catalog, before loading json.el.
//! See evidence/conditions-catalog.json and tools/control_oracle.py.
pub(super) const BUILTINS: &[(&str, &[&str], &str)] = &[
    (
        "args-out-of-range",
        &["args-out-of-range", "error"],
        "Args out of range",
    ),
    ("arith-error", &["arith-error", "error"], "Arithmetic error"),
    (
        "beginning-of-buffer",
        &["beginning-of-buffer", "error"],
        "Beginning of buffer",
    ),
    (
        "buffer-read-only",
        &["buffer-read-only", "error"],
        "Buffer is read-only",
    ),
    (
        "circular-list",
        &["circular-list", "error"],
        "List contains a loop",
    ),
    (
        "cl-assertion-failed",
        &["cl-assertion-failed", "error"],
        "Assertion failed",
    ),
    (
        "cl-no-applicable-method",
        &["cl-no-applicable-method", "cl-no-method", "error"],
        "No applicable method",
    ),
    ("cl-no-method", &["cl-no-method", "error"], "No method"),
    (
        "cl-no-next-method",
        &["cl-no-next-method", "cl-no-method", "error"],
        "No next method",
    ),
    (
        "cl-no-primary-method",
        &["cl-no-primary-method", "cl-no-method", "error"],
        "No primary method",
    ),
    (
        "coding-system-error",
        &["coding-system-error", "error"],
        "Invalid coding system",
    ),
    (
        "comp-sanitizer-error",
        &["comp-sanitizer-error", "error"],
        "Native code sanitizer runtime error",
    ),
    (
        "cyclic-function-indirection",
        &["cyclic-function-indirection", "error"],
        "Symbol's chain of function indirections contains a loop",
    ),
    (
        "cyclic-variable-indirection",
        &["cyclic-variable-indirection", "error"],
        "Symbol's chain of variable indirections contains a loop",
    ),
    ("dbus-error", &["dbus-error", "error"], "D-Bus error"),
    (
        "domain-error",
        &["domain-error", "arith-error", "error"],
        "Arithmetic domain error",
    ),
    (
        "end-of-buffer",
        &["end-of-buffer", "error"],
        "End of buffer",
    ),
    (
        "end-of-file",
        &["end-of-file", "error"],
        "End of file during parsing",
    ),
    ("error", &["error"], "error"),
    (
        "excessive-lisp-nesting",
        &["excessive-lisp-nesting", "recursion-error", "error"],
        "Lisp nesting exceeds `max-lisp-eval-depth'",
    ),
    (
        "excessive-variable-binding",
        &["excessive-variable-binding", "recursion-error", "error"],
        "Variable binding depth exceeds max-specpdl-size",
    ),
    (
        "file-already-exists",
        &["file-already-exists", "file-error", "error"],
        "File already exists",
    ),
    (
        "file-date-error",
        &["file-date-error", "file-error", "error"],
        "Cannot set file date",
    ),
    ("file-error", &["file-error", "error"], "File error"),
    (
        "file-missing",
        &["file-missing", "file-error", "error"],
        "File is missing",
    ),
    (
        "file-notify-error",
        &["file-notify-error", "file-error", "error"],
        "File notification error",
    ),
    (
        "inhibited-interaction",
        &["inhibited-interaction", "error"],
        "User interaction while inhibited",
    ),
    (
        "invalid-arity",
        &["invalid-arity", "error"],
        "Invalid function arity",
    ),
    (
        "invalid-function",
        &["invalid-function", "error"],
        "Invalid function",
    ),
    (
        "invalid-read-syntax",
        &["invalid-read-syntax", "error"],
        "Invalid read syntax",
    ),
    (
        "invalid-regexp",
        &["invalid-regexp", "error"],
        "Invalid regexp",
    ),
    (
        "json-end-of-file",
        &[
            "json-end-of-file",
            "json-parse-error",
            "json-error",
            "error",
        ],
        "end of JSON stream",
    ),
    ("json-error", &["json-error", "error"], "generic JSON error"),
    (
        "json-escape-sequence-error",
        &[
            "json-escape-sequence-error",
            "json-parse-error",
            "json-error",
            "error",
        ],
        "invalid escape sequence",
    ),
    (
        "json-invalid-surrogate-error",
        &["json-invalid-surrogate-error", "json-error", "error"],
        "invalid surrogate pair",
    ),
    (
        "json-number-out-of-range-error",
        &["json-number-out-of-range-error", "json-error", "error"],
        "number out of range",
    ),
    (
        "json-object-too-deep",
        &["json-object-too-deep", "json-error", "error"],
        "object cyclic or Lisp evaluation too deep",
    ),
    (
        "json-out-of-memory",
        &["json-out-of-memory", "json-error", "error"],
        "not enough memory for creating JSON object",
    ),
    (
        "json-parse-error",
        &["json-parse-error", "json-error", "error"],
        "could not parse JSON stream",
    ),
    (
        "json-trailing-content",
        &[
            "json-trailing-content",
            "json-parse-error",
            "json-error",
            "error",
        ],
        "trailing content after JSON stream",
    ),
    (
        "json-utf8-decode-error",
        &["json-utf8-decode-error", "json-error", "error"],
        "invalid utf-8 encoding",
    ),
    (
        "malformed-keyword-arg-list",
        &["malformed-keyword-arg-list", "error"],
        "Keyword lacks a corresponding value",
    ),
    (
        "mark-inactive",
        &["mark-inactive", "error"],
        "The mark is not active now",
    ),
    (
        "memory-buffer-too-small",
        &["memory-buffer-too-small", "error"],
        "Memory buffer too small",
    ),
    ("minibuffer-quit", &["minibuffer-quit", "quit"], "Quit"),
    (
        "missing-module-init-function",
        &[
            "missing-module-init-function",
            "module-load-failed",
            "error",
        ],
        "Module does not export an initialization function",
    ),
    (
        "module-init-failed",
        &["module-init-failed", "module-load-failed", "error"],
        "Module initialization failed",
    ),
    (
        "module-load-failed",
        &["module-load-failed", "error"],
        "Module load failed",
    ),
    (
        "module-not-gpl-compatible",
        &["module-not-gpl-compatible", "module-load-failed", "error"],
        "Module is not GPL compatible",
    ),
    (
        "module-open-failed",
        &["module-open-failed", "module-load-failed", "error"],
        "Module could not be opened",
    ),
    (
        "module-out-of-memory",
        &["module-out-of-memory", "error"],
        "Module out of memory",
    ),
    (
        "native-compiler-error",
        &["native-compiler-error", "error"],
        "Native compiler error",
    ),
    (
        "native-ice",
        &["native-ice", "native-compiler-error", "error"],
        "Internal native compiler error",
    ),
    (
        "native-lisp-file-inconsistent",
        &[
            "native-lisp-file-inconsistent",
            "native-lisp-load-failed",
            "error",
        ],
        "eln file inconsistent with current runtime configuration, please recompile",
    ),
    (
        "native-lisp-load-failed",
        &["native-lisp-load-failed", "error"],
        "Native elisp load failed",
    ),
    (
        "native-lisp-wrong-reloc",
        &[
            "native-lisp-wrong-reloc",
            "native-lisp-load-failed",
            "error",
        ],
        "Primitive redefined or wrong relocation",
    ),
    ("no-catch", &["no-catch", "error"], "No catch for tag"),
    (
        "overflow-error",
        &["overflow-error", "range-error", "arith-error", "error"],
        "Arithmetic overflow error",
    ),
    (
        "permission-denied",
        &["permission-denied", "file-error", "error"],
        "Cannot access file or directory",
    ),
    (
        "protected-field",
        &["protected-field", "error"],
        "Attempt to modify a protected field",
    ),
    ("quit", &["quit"], "Quit"),
    (
        "range-error",
        &["range-error", "arith-error", "error"],
        "Arithmetic range error",
    ),
    (
        "recursion-error",
        &["recursion-error", "error"],
        "Excessive recursive calling error",
    ),
    (
        "remote-file-error",
        &["remote-file-error", "file-error", "error"],
        "Remote file error",
    ),
    ("scan-error", &["scan-error", "error"], "Scan error"),
    (
        "search-failed",
        &["search-failed", "error"],
        "Search failed",
    ),
    (
        "setting-constant",
        &["setting-constant", "error"],
        "Attempt to set a constant symbol",
    ),
    (
        "singularity-error",
        &["singularity-error", "domain-error", "arith-error", "error"],
        "Arithmetic singularity error",
    ),
    ("sqlite-error", &["sqlite-error", "error"], "Database error"),
    (
        "sqlite-locked-error",
        &["sqlite-locked-error", "sqlite-error", "error"],
        "Database locked",
    ),
    (
        "text-read-only",
        &["text-read-only", "buffer-read-only", "error"],
        "Text is read-only",
    ),
    (
        "thread-buffer-killed",
        &["thread-buffer-killed", "error"],
        "Thread's current buffer killed",
    ),
    (
        "trapping-constant",
        &["trapping-constant", "error"],
        "Attempt to trap writes to a constant symbol",
    ),
    (
        "treesit-buffer-too-large",
        &["treesit-buffer-too-large", "treesit-error", "error"],
        "Buffer too large (> 4GiB)",
    ),
    (
        "treesit-buffer_changed",
        &["treesit-buffer_changed", "treesit-error", "error"],
        "Buffer content changed, please don't edit buffer in predicate function, etc",
    ),
    (
        "treesit-error",
        &["treesit-error", "error"],
        "Generic tree-sitter error",
    ),
    (
        "treesit-invalid-predicate",
        &["treesit-invalid-predicate", "treesit-error", "error"],
        "Invalid predicate, see `treesit-thing-settings' for valid forms for a predicate",
    ),
    (
        "treesit-load-language-error",
        &["treesit-load-language-error", "treesit-error", "error"],
        "Cannot load language definition",
    ),
    (
        "treesit-node-buffer-killed",
        &["treesit-node-buffer-killed", "treesit-error", "error"],
        "The buffer associated with this node is killed",
    ),
    (
        "treesit-node-outdated",
        &["treesit-node-outdated", "treesit-error", "error"],
        "This node is outdated, please retrieve a new one",
    ),
    (
        "treesit-parse-error",
        &["treesit-parse-error", "treesit-error", "error"],
        "Parse failed",
    ),
    (
        "treesit-parser-deleted",
        &["treesit-parser-deleted", "treesit-error", "error"],
        "This parser is deleted and cannot be used",
    ),
    (
        "treesit-query-error",
        &["treesit-query-error", "treesit-error", "error"],
        "Query pattern is malformed",
    ),
    (
        "treesit-range-invalid",
        &["treesit-range-invalid", "treesit-error", "error"],
        "RANGES are invalid: they have to be ordered and should not overlap",
    ),
    (
        "type-mismatch",
        &["type-mismatch", "error"],
        "Types do not match",
    ),
    (
        "underflow-error",
        &["underflow-error", "range-error", "arith-error", "error"],
        "Arithmetic underflow error",
    ),
    (
        "unknown-image-type",
        &["unknown-image-type", "error"],
        "Unknown image type",
    ),
    ("user-error", &["user-error", "error"], ""),
    (
        "user-search-failed",
        &["user-search-failed", "user-error", "search-failed", "error"],
        "Search failed",
    ),
    (
        "vc-not-supported",
        &["vc-not-supported", "error"],
        "VC method not implemented for backend",
    ),
    (
        "void-function",
        &["void-function", "error"],
        "Symbol's function definition is void",
    ),
    (
        "void-variable",
        &["void-variable", "error"],
        "Symbol's value as variable is void",
    ),
    (
        "wrong-length-argument",
        &["wrong-length-argument", "error"],
        "Wrong length argument",
    ),
    (
        "wrong-number-of-arguments",
        &["wrong-number-of-arguments", "error"],
        "Wrong number of arguments",
    ),
    (
        "wrong-register-subr-call",
        &[
            "wrong-register-subr-call",
            "native-lisp-load-failed",
            "error",
        ],
        "comp--register-subr can only be called during native lisp load phase.",
    ),
    (
        "wrong-type-argument",
        &["wrong-type-argument", "error"],
        "Wrong type argument",
    ),
];
