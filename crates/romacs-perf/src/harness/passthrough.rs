//! Which operator environment variables a benchmark editor inherits:
//! harness engine so new runtime knobs do not grow `harness.rs`.

const BENCHMARK_PASSTHROUGH_ENVIRONMENT: &[&str] = &[
    // Paired binaries must be able to load the same frozen Lisp runtime.
    "ROMACS_RUNTIME_ROOT",
    // Independent redisplay controls; preserve upstream policy forwarding.
    "ROMACS_PERF_SUSTAINED_VISIBLE",
    "ROMACS_PERF_SUSTAINED_EDIT_CASE",
    "ROMACS_EDIT_SYNC_STILL",
    "ROMACS_EDIT_SYNC_LAZY_PROOF",
    "ROMACS_RETAINED_FACE_GATHER",
    "ROMACS_LAYOUT_PROPERTY_KEYS_INLINE",
    "ROMACS_EDIT_SYNC_PROVE_FIRST",
    "ROMACS_EDIT_SYNC_DENSE_INDEX",
    "ROMACS_EDIT_SYNC_FONTIFY_COVERAGE",
    "ROMACS_EDIT_SYNC_SHIFT_SKIP",
    "ROMACS_EDIT_SYNC_SOURCE_BUDGET",
    "ROMACS_MODE_LINE_PROP_BORROW",
    "ROMACS_MODE_LINE_PROP_SLICE",
    "ROMACS_MODE_LINE_PLAIN_FIELD",
    "ROMACS_MODE_LINE_NUMERIC_PADDING",
    "ROMACS_REDISPLAY_GNU_HOOKS",
    "ROMACS_WATCHED_PROP_DEMAND",
    "ROMACS_POSN_OBJECT_EXTENT",
    "PATH",
    "LD_LIBRARY_PATH",
    "DYLD_LIBRARY_PATH",
    "DYLD_FALLBACK_LIBRARY_PATH",
    // Preserve Vulkan loader discovery on hosts whose drivers live outside
    // the default search paths (e.g. NixOS). Dropping these can silently
    // benchmark software rendering instead of the session's GPU driver.
    "VK_DRIVER_FILES",
    "VK_ICD_FILENAMES",
    "VK_ADD_DRIVER_FILES",
    "GST_PLUGIN_SYSTEM_PATH_1_0",
    "GST_PLUGIN_SCANNER_1_0",
    "SYSTEMROOT",
    "WINDIR",
    "ROMACS_PERF_RELEASE_STARTUP_GC_CEILING",
    // Per-frame layout telemetry (relaid rows, fast-path classification), for
    // attributing an input-latency tail to relayout rather than guessing.
    "ROMACS_LAYOUT_STATS_FILE",
    // Per-keystroke latency decomposition: wall, CPU, and collection work.
    // Percentiles cannot say whether a slow keystroke was computing or
    // waiting, and those call for opposite work.
    "ROMACS_PERF_LATENCY_TRACE_FILE",
    // Request-owned GC sidecar; the controller still owns the gate port.
    // Capture this exact path in input provenance when diagnostics are enabled.
    "ROMACS_PERF_GC_WINDOW_FILE",
    // GC pacing sweep: the live-proportional term as a percentage of the live
    // heap. 0 leaves GNU's `gc-cons-threshold`/`gc-cons-percentage` contract
    // exactly; the built-in default is 50.
    "ROMACS_GC_LIVE_GROWTH_PERCENT",
    "ROMACS_TEXT_SNAPSHOT",
    // TTY silent frames: `off` or `on` (default).
    "ROMACS_TTY_SILENT",
    // TTY damage-proportional repaint: `off`, `verify` or `on` (default), and
    // the per-frame verify report.
    "ROMACS_TTY_DAMAGE",
    "ROMACS_TTY_DAMAGE_REPORT_FILE",
    // What names a window row in a TTY painter key: `address` or
    // `appearance` (default; position-only copies keep their row).
    "ROMACS_TTY_ROW_IDENTITY",
    // Chrome string positions from chrome rows: `frame` or `rows` (default).
    "ROMACS_PRESENT_CHROME_POS",
    // Text hit positions built while composing (`eager`) or on the
    // first pointer query (`lazy`, default).
    "ROMACS_PRESENT_HIT",
    // C6 compact row point storage and per-window row hits: off, on (default), verify.
    "ROMACS_PRESENT_POINT_ROWS",
    // Certified row-stream concatenation: off, on (default); overlaps keep heap merge.
    "ROMACS_POINT_ROW_ITER",
    // Deferred numeric presentation positions: on (default), off; same-binary arms.
    "ROMACS_PRESENT_GEOMETRY_LAZY",
    // Source newline counting: off, on (default), verify; indexed counts unchanged.
    "ROMACS_LAYOUT_LINE_COUNT",
    // The mini-window stands still when what it shows is unchanged: `on`.
    "ROMACS_LAYOUT_MINI_STILL",
    // The visible automatic-composition scan's ASCII fast path and memo: `on`.
    "ROMACS_COMPOSITION_FASTPATH",
    // P3.5 U3.7. An edit frame's mode line by GNU's optimization-1 guard:
    // `legacy` or `gnu` (default); on/1/true/yes select GNU.
    "ROMACS_MODE_LINE_GATE",
    // Edit replays synchronize with the rows below the edit (GNU
    // try_window_id): `prove` or `sync` (default); on/1/true/yes select Sync.
    "ROMACS_LAYOUT_EDIT_SYNC",
    // A window whose start moved back reuses its old rows: `on`.
    "ROMACS_LAYOUT_SCROLL_BACK",
    // posn-at-point & co. read only the text the window shows: `on`.
    "ROMACS_POSN_BOUNDED_TEXT",
    // `(redisplay t)` skips the layout when nothing visible changed: `on`.
    "ROMACS_REDISPLAY_IDLE_SKIP",
    // An evaluated mode line that renders the same reuses its row: `on|verify`.
    "ROMACS_CHROME_MEMO",
    // U2.8 builtin front-end diets: `=off` restores the general search,
    // syntax and text-property paths for a same-binary board A/B.
    "ROMACS_BUILTIN_FRONTEND",
    // P1.4 Stage A cached variable tiers: `=0` restores the general
    // read/set/bind/unbind paths.
    "ROMACS_VAR_CACHE",
    // CL2 performance-cliff controls; record the exact editor environment in
    // input-provenance.json for same-binary comparisons.
    "ROMACS_COMPARE_STRINGS_POS_CACHE",
    "ROMACS_REGEX_SHORT_LITERAL",
    "ROMACS_EMACS_MULE_PREPARED",
    "ROMACS_OVERLAY_LOCAL_MOVE",
    // P1.2 / P1.0 §3.10: Tier-0 `Bcall` of leaf builtins (`=on`).
    "ROMACS_VM_LEAF",
    // P3.3 regex knobs: alternation anchors (`=on`) and the existence DFA
    // (`=on`/`=verify`), plus its exit statistics.
    "ROMACS_REGEX_ANCHOR_ALT",
    "ROMACS_REGEX_DFA",
    "ROMACS_REGEX_DFA_STATS",
    "ROMACS_REGEX_CACHE_DIAGNOSTIC",
    // Folded two-character suffix search.
    "ROMACS_REGEX_SUFFIX_LITERAL",
    "ROMACS_CALLBACK_CACHE",
    // P4.1 Stage 0 cconv memo (`off`/`stats`/`on`/`verify`) and the native
    // no-lexvars closure path (`on`).
    "ROMACS_CCONV_MEMO",
    "ROMACS_CCONV_FAST",
    // U0.7: `parse-partial-sexp` runs `syntax-propertize` like GNU (on);
    // `=0` never propertizes, to attribute the parity fix's cost.
    "ROMACS_PPS_PROPERTIZE",
    // P3.4 syntax cache: L1 (`1` default, `0`, `verify`), L2 (`0` default,
    // `1`, `verify`), the shared geometry and counters file.
    "ROMACS_SYNTAX_PARSE_CACHE",
    "ROMACS_SYNTAX_PARSE_CACHE_L2",
    "ROMACS_SYNTAX_PARSE_CACHE_CHUNK",
    "ROMACS_SYNTAX_PARSE_CACHE_MIN_SPAN",
    "ROMACS_SYNTAX_PARSE_CACHE_STATS",
    // Collector knobs (same-binary A/B; each defaults to the old path): the
    // chunk map (`=1`), falsifier F-G's vector deferral (`=defer`), and the
    // generation census (`=1`), whose records go to the census file.
    "ROMACS_GC_CHUNK_MAP",
    "ROMACS_GC_CENSUS",
    "ROMACS_GC_CENSUS_REMSET",
    "ROMACS_GC_CENSUS_FILE",
    // Collector controls and trace evidence for mutator/marker attribution.
    "ROMACS_GC_STRESS",
    "ROMACS_GC_TRACE",
    // Callback and interpreter-pool experiments use the same editor binary.
    "ROMACS_ASSOC_RESOLVED",
    "ROMACS_HASH_TEST_PARITY",
    "ROMACS_COMPARE_STRINGS_PARITY",
    "ROMACS_MODE_LINE_FLOW",
    "ROMACS_MAPHASH_BYTECODE",
    "ROMACS_VM_STACK_RETURN",
];

/// Filter an explicitly supplied historical benchmark environment.
pub(crate) fn passthrough_from(
    vars: impl IntoIterator<Item = (std::ffi::OsString, std::ffi::OsString)>,
) -> Vec<(String, std::ffi::OsString)> {
    vars.into_iter()
        .filter_map(|(name, value)| {
            let name = name.into_string().ok()?;
            let forwarded = BENCHMARK_PASSTHROUGH_ENVIRONMENT.contains(&name.as_str());
            forwarded.then_some((name, value))
        })
        .collect()
}
