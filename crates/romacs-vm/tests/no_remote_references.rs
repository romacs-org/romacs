//! Every place a Value can live outside the four heap tables, named. A later
//! collector relies on the rule that a trace of a table entry reads only that
//! entry's own storage, and that a Value anywhere else is a root or shades at
//! its store (design note gc-smooth.md 4.15). Nothing else enumerates these:
//! a field in no root list is invisible to the collector and to `gc_check`.
//!
//! So each `Vm` field and each `Object` variant has a class here, and a new
//! one fails until it has one, as does a name that is gone. The class is a
//! decision read from the code, not something this test can prove; it makes
//! the decision someone's to take when the field is added.
#[path = "support/rust_tokens.rs"]
mod rust_tokens;
use rust_tokens::{Token, tokens};
// The shared lexer's own list, which this test has no use for.
const _: &[&str] = rust_tokens::CONVERTED;
use std::collections::BTreeSet;
use std::path::Path;

#[derive(Clone, Copy, Debug)]
enum Class {
    /// The collector reads it as a root at every mark.
    Root,
    /// Holds Values a later collector learns of at their stores: the hook runs.
    Barrier,
    /// Holds Values that shade at their stores once C1 adds the shade. Until
    /// then a store here runs nothing: not `Barrier`.
    Owed,
    /// Read for what a mark reached (an own-obarray symbol, a marked region).
    /// An incremental mark will have to read it again at its finish; until C
    /// does, nothing reads it twice.
    Rescanned,
    /// Holds no heap Value.
    Free,
    /// Keeps nothing alive: the collector clears or checks what it holds, or a
    /// stale one is inert by use.
    Cache,
    /// Holds Values only between two points where no collection runs.
    Transient,
}
use Class::{Barrier, Cache, Free, Owed, Rescanned, Root, Transient};

/// Every `Vm` field.
const FIELDS: &[(&str, Class)] = &[
    ("charsets", Root), ("standard_case_tables", Root), ("command_point", Root),
    ("change_prepared", Free),
    // Keeps the last kernel's arguments until the next edit clears it, past
    // any collection; during one it is `stack`, swapped in. Inert: nothing
    // reads it before that clear (insdel.rs).
    ("kernel_stack", Cache),
    ("change_slots", Free), ("case_symbols_slot", Free), ("print_number_index", Free), ("undo_list_slot", Free),
    // `Shown`'s Values are compared only while `collections` still matches, and
    // `previous` and the bar's caches go the same way; faces are hidden slots.
    ("display_state", Cache),
    ("window_state", Free), ("world_version", Free), ("motion_quit", Free),
    ("variable_read_probes", Free), ("speculative_hits", Free), ("inline_hits", Free), ("disable_inline", Free), ("speculative_misses", Free),
    ("regexp", Free),
    // A pattern's syntax table goes at every collection (`clear_regexp_cache`).
    ("regexp_cache", Cache),
    ("mode_line_forms", Cache), ("lambda_lists", Cache),
    ("images", Free),
    // A `Band`: its object constants are `Rooted` handles, and its `rule` keeps
    // a `CharTableView` no root list reads. Inert: those Values reach only
    // `== NIL` and `integer_value()` (width_rule.rs), so a stale one changes
    // nothing.
    ("status_bar_line", Cache),
    ("execution_fatal", Free), ("loader", Free), ("source_cache", Free), ("macros", Free), ("expanding", Free),
    ("compile_depth", Free), ("eval_depth", Free), ("dynamic_scope", Free),
    ("propagated_escape", Root),
    ("site_leaf", Free),
    ("signal_handlers", Root), ("backtrace", Root), ("profiler", Root),
    ("propagated_escape_hooked", Free), ("propagated_eval_depth", Free), ("propagate_next", Free),
    ("host_input", Free), ("layout_host", Free), ("display", Free), ("visual", Free), ("session_host", Free), ("idle_since", Free),
    ("redisplay_since_mark", Free), ("treesit_plain", Free), ("treesit_deferred", Free), ("treesit_parse_owed", Free),
    ("quit", Free), ("force_quit_count", Free), ("quit_slots", Free),
    // Interned symbols, which no collection frees, and indices.
    ("face_keys", Free),
    ("debug_event_slots", Free), ("keymap_generation", Free), ("process_jobs", Free), ("subprocesses", Free),
    ("environment_snapshot", Free), ("trapped", Free), ("retrap", Free), ("doc_file", Free),
    ("terminal_host", Free), ("terminal_bindings", Root),
    ("pending_specials", Free), ("file_specials", Free), ("seed_file_specials", Free), ("file_level_request", Free),
    ("lazy_regime", Free), ("interpreting_source", Free), ("defer_expansion_errors", Free), ("evaluating", Free), ("quit_unwinding", Free),
    // Weak: one whose error a Rust caller discarded goes at the next collection.
    ("thrown_no_catch", Cache),
    ("strict_root", Free), ("reached_head", Free), ("reached_generated", Free), ("loader_order_request", Free),
    ("keep_captures", Free), ("lambda_depth", Free), ("window_change_seen", Free), ("expanded_macros", Free), ("expanded_symbols", Free),
    ("image", Free),
    // A frozen region's symbols, read once it is marked. Written when it is
    // placed and when its hash tables are put in, both before it is returned
    // and before any heap Value names it, with no collection between
    // (frozen.rs), so no store.
    ("regions", Rescanned),
    ("region_free", Free), ("region_collect", Free), ("isolates", Free), ("inotify", Free),
    ("buffer_slot", Free), ("multibyte_slot", Free), ("truncate_lines_slot", Free), ("defun_start_epoch", Free),
    // An alias names a symbol slot, a Value in all but type.
    ("variable_redirects", Owed),
    ("forwarded", Root),
    ("fixnum_slots", Free), ("local_binding_slots", Free), ("free_local_binding_slots", Free),
    ("gc_stats", Free), ("gc_times", Free), ("gc_census", Free), ("collection_log", Free), ("gc_retained", Free), ("gc_payload_bytes", Free),
    ("collect_poll", Free), ("gc_retained_slots", Free), ("gc_code", Free), ("gc_floor", Free), ("job_budget", Free), ("gc_retained_payload", Free),
    ("gc_holds", Free), ("running_engines", Free), ("rooted_write", Free), ("gc_stress", Free), ("allocation", Free),
    ("names", Free), ("initial_obarray", Free), ("symbol_names", Free),
    // Read for the symbols marked, which own-obarray ones are only if reached.
    ("obarray_symbol_names", Rescanned),
    ("permanent_symbol_names", Root),
    // Weak references to nodes the case tables own.
    ("native_case_roots", Cache),
    ("obarray_free_slots", Free), ("generated_symbols", Free), ("generated_slots", Free),
    ("slots", Root),
    ("specials", Free), ("special_changes", Free), ("special_cache", Free),
    // Each function's ops hold constants, traced from the code that is live.
    ("functions", Root),
    ("request_code", Root),
    ("register_width", Free), ("spread_reserve", Free),
    // Lowered code, which adds nothing to its function's ops, and registers.
    ("execution", Root), ("spare_programs", Root),
    ("reader_trampolines", Free),
    ("closures", Barrier), ("cells", Barrier), ("conses", Barrier), ("objects", Barrier),
    // A paired cell's trace pushes the pair's Value, and its writes will shade.
    ("cell_pairs", Owed),
    ("random", Free), ("fence", Free), ("pending", Free), ("mktime_offset", Free), ("tz_rule_dstoff", Free),
    ("local_zone", Free), ("recent_keys", Free), ("lossage_limit", Free),
    ("referrers", Cache), ("local_owners", Cache),
    ("completion_walks", Free), ("completion_case_memo", Free),
    ("caches", Cache),
    ("stack", Root), ("external_roots", Root),
    // Not roots: the collector dooms those it does not reach.
    ("finalizers", Cache),
    ("doomed_finalizers", Root),
    ("post_gc_due", Free), ("unwinding", Free), ("aftermath_runs", Free),
    ("suspended_stacks", Root), ("suspended", Root),
    // Set and cleared around one keymap walk, which never collects (keymap.rs).
    ("keymap_filters", Transient), ("keymap_pending_filter", Transient),
    ("dynamic", Root),
    // Custom symbols' entries are read with their symbol, and will shade at their store.
    ("conditions", Owed),
    ("rooted", Root),
];

/// Every `Object` variant. `Barrier`: a whole-entry store hooks its entry
/// (`Table::entry_mut`), and its trace reads what `visit_values` reads.
const VARIANTS: &[(&str, Class)] = &[
    ("SyntaxTable", Free), ("Float", Free), ("Bignum", Free),
    ("Window", Barrier), ("Frame", Barrier), ("Terminal", Barrier), ("WindowConfiguration", Barrier),
    ("Sqlite", Free), ("Treesit", Barrier), ("Process", Barrier), ("Finalizer", Barrier), ("Thread", Free),
    ("String", Barrier), ("RawString", Barrier), ("Vector", Barrier), ("BoolVector", Free), ("Record", Barrier),
    // Their nodes are shared and locked, so a node's store hooks no one entry:
    // their contents are written only through char_table.rs's funnel (`wrote`),
    // which every write method passes and the collector's barrier will hook.
    // Past it only the collector's own visits write (PAST_THE_BARRIER).
    ("CharTable", Barrier), ("SubCharTable", Barrier), ("MapKeymapCursor", Barrier),
    // Text shared by indirect buffers is traced from the base alone.
    ("Buffer", Barrier),
    ("Marker", Barrier), ("Overlay", Barrier), ("Bytes", Free), ("ByteBuffer", Barrier),
    // Stores go through its setters (`Table::hash_mut`, hash.rs alone), weak ones' too.
    ("HashTable", Barrier),
    ("Obarray", Barrier), ("Free", Free),
];

fn source(name: &str) -> String {
    std::fs::read_to_string(Path::new(env!("CARGO_MANIFEST_DIR")).join("src").join(name)).unwrap()
}

/// The body's tokens of the item `KEYWORD NAME { ... }`, between its braces.
fn body_of(toks: &[Token], keyword: &str, name: &str) -> Vec<String> {
    let start = toks
        .windows(2)
        .position(|w| w[0] == Token::Ident(keyword.into()) && w[1] == Token::Ident(name.into()))
        .unwrap_or_else(|| panic!("no {keyword} {name}"));
    let open = (start..toks.len()).find(|&i| toks[i] == Token::Punct('{')).unwrap();
    let mut depth = 0;
    let mut out = vec![];
    for t in &toks[open..] {
        match t {
            Token::Punct('{') => depth += 1,
            Token::Punct('}') if depth == 1 => return out,
            Token::Punct('}') => depth -= 1,
            _ => {}
        }
        out.push(match t {
            Token::Ident(name) => name.clone(),
            Token::Punct(c) => c.to_string(),
        });
    }
    panic!("{keyword} {name} never closes");
}

/// The members a body declares: an identifier at depth 0 at the start of a
/// member, past its attributes and visibility, up to `,` at depth 0.
fn members(body: &[String]) -> BTreeSet<String> {
    let mut out = BTreeSet::new();
    let (mut depth, mut start, mut i) = (0i32, true, 1);
    while i < body.len() {
        let t = body[i].as_str();
        if start && t == "#" {
            let mut d = 0;
            i += 1;
            loop {
                match body[i].as_str() {
                    "[" => d += 1,
                    "]" => d -= 1,
                    _ => {}
                }
                i += 1;
                if d == 0 { break; }
            }
            continue;
        }
        if start && t == "pub" {
            i += 1;
            if body[i] == "(" {
                while body[i] != ")" { i += 1; }
                i += 1;
            }
            continue;
        }
        if start {
            out.insert(t.to_owned());
            start = false;
        }
        match t {
            "(" | "[" | "{" | "<" => depth += 1,
            // A `->` or a `>` of `=>` closes nothing; neither appears in these bodies.
            ")" | "]" | "}" | ">" => depth -= 1,
            "," if depth == 0 => start = true,
            _ => {}
        }
        i += 1;
    }
    out
}

/// What NAMED and FOUND differ by, as complaints about WHAT.
fn differences(what: &str, named: &[(&str, Class)], found: &BTreeSet<String>) -> Vec<String> {
    let listed: BTreeSet<String> = named.iter().map(|(name, _)| (*name).to_owned()).collect();
    let mut out: Vec<String> = found.difference(&listed).map(|name| format!("{what} {name} has no class here")).collect();
    out.extend(listed.difference(found).map(|name| format!("{what} {name} is gone: drop its row")));
    if listed.len() != named.len() {
        out.push(format!("a {what} is listed twice"));
    }
    out
}

#[test]
fn every_vm_field_and_object_variant_has_a_class() {
    let vm = members(&body_of(&tokens(&source("lib.rs")), "struct", "Vm"));
    let object = members(&body_of(&tokens(&source("data.rs")), "enum", "Object"));
    assert!(vm.len() > 100 && object.len() > 20, "the parse found too little: {} fields, {} variants", vm.len(), object.len());
    let mut problems = differences("Vm field", FIELDS, &vm);
    problems.extend(differences("Object variant", VARIANTS, &object));
    assert!(problems.is_empty(), "{}", problems.join("\n"));
}

/// The check can fire, both ways.
#[test]
fn a_new_name_and_a_stale_name_are_both_refused() {
    let found: BTreeSet<String> = ["kept", "added"].map(str::to_owned).into();
    let problems = differences("Vm field", &[("kept", Root), ("gone", Free)], &found);
    assert_eq!(problems, ["Vm field added has no class here", "Vm field gone is gone: drop its row"]);
    let parsed = members(&body_of(
        &tokens("struct Vm { #[cfg(test)] a: Vec<(u8, u8)>, pub(crate) b: [u8; 2], /// c\n c: std::collections::HashMap<u8, u8>, }"),
        "struct",
        "Vm",
    ));
    assert_eq!(parsed, ["a", "b", "c"].map(str::to_owned).into());
    let parsed = members(&body_of(&tokens("enum Object { A(Box<u8>), B, /// c\n C(Vec<u8>, u8), }"), "enum", "Object"));
    assert_eq!(parsed, ["A", "B", "C"].map(str::to_owned).into());
}

/// The `Table` methods that write past the barrier, and who may name them:
/// the collector's own rewrites, and the hash table's setters, whose `Stores`
/// handle tells each store but whose `&mut HashTable` would let a caller write
/// around it. Each name is the method's alone, so any use of it counts. The
/// rule covers names only: a wrapper in an allowed file that hands a method out
/// is that file's to answer for.
const PAST_THE_BARRIER: &[(&str, MayCall)] = &[
    ("collector_entries_mut", collector), ("collector_compact", collector), ("collector_prepend", collector),
    ("hash_mut", |file| file == "table.rs" || file == "hash.rs"),
    ("collector_visit_node", char_table_collector), ("collector_visit_values", char_table_collector),
];

/// Whether a file under `src` may call a method.
type MayCall = fn(&str) -> bool;

fn collector(file: &str) -> bool {
    ["table.rs", "gc.rs", "gc_mark.rs", "gc_check.rs"].contains(&file)
}

/// The collector, and the char table and keymap cursor files that give it
/// their visits: those write node contents past char_table.rs's funnel.
fn char_table_collector(file: &str) -> bool {
    collector(file) || file == "char_table.rs" || file == "map_keymap.rs"
}

/// Each file under `src` that names a method of PAST_THE_BARRIER it may not,
/// as `FILE: METHOD`: a call, a path to it, a reference to it or a macro's
/// argument, any use of the identifier. A comment or a string never counts.
fn names_past_the_barrier(sources: &[(String, String)]) -> Vec<String> {
    let mut problems = vec![];
    for (file, text) in sources {
        let toks = tokens(text);
        for (method, allowed) in PAST_THE_BARRIER {
            if !allowed(file) && toks.contains(&Token::Ident((*method).into())) {
                problems.push(format!("{file}: {method}"));
            }
        }
    }
    problems
}

#[test]
fn only_the_collector_and_the_hash_setters_write_past_the_barrier() {
    let problems = names_past_the_barrier(&sources());
    assert!(problems.is_empty(), "past the barrier: {}", problems.join(", "));
}

/// The check can fire: each method, named from a file that may not, by a
/// call, a path, a reference and a macro's argument.
#[test]
fn a_name_past_the_barrier_is_refused() {
    let file = |name: &str, text: &str| (name.to_owned(), text.to_owned());
    let sources = [
        file("window.rs", "fn f(vm: &mut Vm) { vm.objects.hash_mut(1); vm.conses.collector_compact(|_| {}); }"),
        file("prim/gc_leaf.rs", "fn f(vm: &mut Vm) { vm.cells.collector_entries_mut(); Table::collector_prepend(&mut vm.cells, 1, || NIL); }"),
        file("gc.rs", "fn f(vm: &mut Vm) { vm.cells.collector_entries_mut(); let g = Table::hash_mut; g(&mut vm.objects, 2); }"),
        file("frame.rs", "fn f(vm: &mut Vm) { through!(vm.objects, hash_mut, 3); }"),
        file("hash.rs", "fn f(vm: &mut Vm) { vm.objects.hash_mut(1); } // collector_compact in a comment"),
        file("handle.rs", "fn f(vm: &mut Vm) { let collector_compact_count = 0; vm.hash_mut_ish(); let s = \"collector_prepend\"; }"),
        file("gcov.rs", "fn f(vm: &mut Vm) { vm.cells.collector_entries_mut(); }"),
        file("prim/leaf/casetab.rs", "fn f(t: &mut CharTable) { t.collector_visit_values(&mut seen, &mut |v| *v = NIL); char_table::collector_visit_node(t.root(), &mut seen, &mut |_| {}); }"),
        file("map_keymap.rs", "fn f(n: &CharTableNodeHandle) { collector_visit_node(n, &mut seen, &mut |_| {}); }"),
    ];
    assert_eq!(names_past_the_barrier(&sources), [
        "window.rs: collector_compact", "window.rs: hash_mut",
        "prim/gc_leaf.rs: collector_entries_mut", "prim/gc_leaf.rs: collector_prepend",
        "gc.rs: hash_mut",
        "frame.rs: hash_mut",
        "gcov.rs: collector_entries_mut",
        "prim/leaf/casetab.rs: collector_visit_node", "prim/leaf/casetab.rs: collector_visit_values",
    ]);
}

// Two ratchets on what a store can write past the barrier. Both are
// censuses, like the field list above: the check finds declarations, and a
// person classifies them, because which hold a Value is read from the code,
// not inferred by this test.
//
// 1. Every declaration that keeps something behind a cell or lock (a field, an
//    enum's body, an alias), anywhere under `src`, has a class here. A shared
//    borrow writes through a cell, so one holding a Value is a store the
//    barrier never sees; those are `Held`, and their number only falls.
// 2. Every table entry type (each `Object` variant's payload, and closures)
//    with a field that isn't private is listed. A `&mut` to an entry comes
//    only from `entry_mut`, which runs the barrier, so this is the funnel each
//    type still lacks: it leaves the list when its fields are all private.

#[derive(Clone, Copy, PartialEq, Debug)]
enum Behind {
    /// Holds no Value.
    Plain,
    /// Holds a Value a shared borrow can write: the barrier never sees it.
    Held,
    /// The collector's roots, read at every mark.
    Rooted,
    /// Holds a Value only while no collection can run.
    Passing,
    /// A cell of the crate's own, which counts as one wherever it is named.
    Wrapper,
}
use Behind::{Held, Passing, Plain, Rooted, Wrapper};

/// The standard cells and locks, which count followed by their argument.
const STD_CELLS: &[&str] = &["Cell", "RefCell", "Mutex", "RwLock", "UnsafeCell", "OnceCell", "OnceLock", "LazyCell", "LazyLock"];

/// Every declaration behind a cell, as `FILE:TYPE.FIELD` (`FILE:TYPE` for an
/// enum's body or an alias). A `Wrapper` struct, and an alias whose row is
/// `Held`, count as cells themselves wherever they are named.
const CELLS: &[(&str, Behind)] = &[
    ("annotations.rs:AnnotatedBuffer.anchors", Plain), ("annotations.rs:AnnotatedBuffer.reversed_pairs", Plain),
    ("bytes.rs:BufferState.modification", Plain),
    // Char table nodes, shared between tables, sub-char-tables, views and
    // keymap cursors: written through their lock (the char-table write funnel).
    ("char_table.rs:CharTableNodeHandle", Held), ("char_table.rs:CharTableSlot", Held),
    ("char_table.rs:CharTableView.root", Held), ("char_table.rs:CharTable.root", Held),
    ("data.rs:Object", Held), ("map_keymap.rs:Frame.node", Held), ("lib.rs:Vm.native_case_roots", Held),
    ("data.rs:LispString.cursors", Plain), ("data.rs:LispString.scanned", Plain), ("data.rs:LispString.flag_scanned", Plain),
    ("display.rs:FaceTable.faces", Plain), ("display.rs:FaceTable.index", Plain), ("display.rs:FaceTable.over", Plain),
    ("display.rs:FaceTable.merged", Plain), ("display.rs:FaceTable.view", Plain), ("display.rs:FaceTable.floor", Plain),
    ("display.rs:DisplayState.diagnostics", Plain),
    // The printer's state for one print, which can call Lisp.
    ("eval_data.rs:Unreadable", Held), ("eval_data.rs:PrintOptions.graph", Held),
    ("gc_mark.rs:Marking.batch", Plain), ("gc_mark.rs:Marking.park", Plain), ("gc_mark.rs:Marking.census", Plain),
    ("gc_mark.rs:Marking.after_take", Plain),
    // The mark's own queue, for one stopped mark.
    ("gc_mark.rs:Marking.serial", Passing),
    // Root registrations, kept as Value bits.
    ("handle.rs:RootSet.slots", Rooted),
    ("handle.rs:Looks.0", Plain),
    ("host.rs:Waiter.armed", Plain), ("host.rs:InspectStore.queue", Plain),
    ("indirect.rs:Shared.0", Wrapper),
    ("isolate.rs:PoolMemory.pids", Plain),
    ("lib.rs:Vm.variable_read_probes", Plain),
    // A reader borrowing its tree, so no collection runs while it holds the run it read.
    ("properties.rs:KeyReader.run", Passing),
    ("regexp.rs:Input.anchors", Plain), ("regexp.rs:CachedPattern.compiled", Plain), ("regexp.rs:StringMatcher.patterns", Plain),
    ("regexp_syntax.rs:VmSyntax.ascii", Plain),
    ("subprocess.rs:Reaper.state", Plain),
    ("text.rs:TextBody.flat", Plain), ("text.rs:TextBody.flat_moved", Plain), ("text.rs:TextBody.flat_internal", Plain),
    ("text.rs:TextBody.coordinate", Plain), ("text.rs:TextBody.chunk", Plain), ("text.rs:TextBody.cold_chunks", Plain),
    ("text.rs:TextBody.cold_order", Plain), ("text.rs:TextBody.scanned_bytes", Plain), ("text.rs:TextBody.coordinate_steps", Plain),
    // Text shared by indirect buffers, its properties among it: `TextBody`'s own write methods.
    ("text.rs:Text.body", Held),
    ("tls.rs:Session.peer", Plain), ("tls.rs:Verifier.peer", Plain),
    ("compiled_lisp/mod.rs:Configured.identity", Plain),
    ("prim/leaf/syntax_scan.rs:View.ascii", Plain), ("prim/leaf/syntax_scan.rs:View.read", Plain), ("prim/leaf/syntax_scan.rs:View.last", Plain),
    // Tree-sitter's native state; the Lisp Values sit beside it, outside the cell.
    ("treesit/mod.rs:Registry.parsers", Plain), ("treesit/mod.rs:Registry.edited", Plain), ("treesit/mod.rs:Early.changes", Plain),
    ("treesit/mod.rs:Parser.native", Plain), ("treesit/mod.rs:Parser.core", Plain), ("treesit/mod.rs:Task.state", Plain),
    ("treesit/native.rs:Tree.1", Plain),
];

/// How many `Held` rows there are. It only falls: a new one fails, and a gone
/// one fails until this is lowered.
const HELD: usize = 10;

/// Each `Object` variant's payload types, as `FILE:TYPE`: none for plain data
/// or a bare Value, several where the payload is an enum of the crate's.
const ENTRIES: &[(&str, &[&str])] = &[
    ("SyntaxTable", &[]), ("Float", &[]), ("Bignum", &[]),
    ("Window", &["window.rs:Window"]), ("Frame", &["frame.rs:Frame"]), ("Terminal", &["frame.rs:Terminal"]),
    ("WindowConfiguration", &["window_config.rs:WindowConfiguration"]), ("Sqlite", &["sqlite.rs:Handle"]),
    ("Treesit", &["treesit/mod.rs:Parser", "treesit/mod.rs:Node", "treesit/mod.rs:Query", "treesit/mod.rs:Task"]),
    ("Process", &["subprocess.rs:Process"]), ("Finalizer", &[]), ("Thread", &[]),
    ("String", &["data.rs:LispString"]), ("RawString", &["data.rs:LispString"]), ("Vector", &[]),
    ("BoolVector", &["bool_vector.rs:BoolVector"]), ("Record", &[]),
    ("CharTable", &["char_table.rs:CharTable"]), ("SubCharTable", &["char_table.rs:CharTableNode"]),
    ("MapKeymapCursor", &["map_keymap.rs:Cursor"]), ("Buffer", &["annotations.rs:AnnotatedBuffer"]),
    ("Marker", &["annotations.rs:Marker"]), ("Overlay", &["annotations.rs:Overlay"]), ("Bytes", &[]),
    ("ByteBuffer", &["bytes.rs:ByteBuffer"]), ("HashTable", &["hash.rs:HashTable"]), ("Obarray", &["obarray.rs:Obarray"]),
    ("Free", &[]),
];

/// Today's table entry types with a field that isn't private, each with what
/// will make them all private.
const PUBLIC_ALLOWED: &[(&str, &str)] = &[
    ("annotations.rs:AnnotatedBuffer", "buffer accessors"), ("annotations.rs:Marker", "marker accessors"),
    ("annotations.rs:Overlay", "overlay accessors"), ("bool_vector.rs:BoolVector", "bool-vector accessors"),
    ("bytes.rs:ByteBuffer", "buffer accessors"),
    ("char_table.rs:CharTable", "the char-table write funnel (perf/gc-chartable)"),
    ("char_table.rs:CharTableNode", "the char-table write funnel (perf/gc-chartable)"),
    ("data.rs:LispString", "string accessors"), ("frame.rs:Frame", "frame accessors"), ("frame.rs:Terminal", "terminal accessors"),
    ("hash.rs:HashTable", "hash table accessors"), ("obarray.rs:Obarray", "obarray accessors"),
    ("sqlite.rs:Handle", "sqlite handle accessors"), ("subprocess.rs:Process", "process accessors"),
    ("treesit/mod.rs:Node", "tree-sitter accessors"), ("treesit/mod.rs:Parser", "tree-sitter accessors"),
    ("treesit/mod.rs:Query", "tree-sitter accessors"), ("treesit/mod.rs:Task", "tree-sitter accessors"),
    ("window.rs:Window", "window accessors"),
];

#[derive(Clone, Copy, PartialEq)]
enum Kind { Struct, Enum, Alias }

/// A type's declaration: a struct's fields, an enum's body as one field, an
/// alias's right side as one field.
struct Shape {
    file: String,
    name: String,
    kind: Kind,
    fields: Vec<(String, bool, Vec<String>)>,
}

/// Associated types, which a `type` inside an `impl` declares.
const ASSOCIATED: &[&str] = &["Output", "Target", "Item", "IntoIter", "Error", "Err", "Owned"];

fn is_type_name(word: &str) -> bool {
    word.starts_with(|c: char| c.is_ascii_uppercase()) && word.chars().all(|c| c.is_alphanumeric() || c == '_')
}

fn words(text: &str) -> Vec<String> {
    tokens(text).into_iter().map(|t| match t { Token::Ident(name) => name, Token::Punct(c) => c.to_string() }).collect()
}

/// The index of the bracket closing the one at I; a `->` closes nothing.
fn close(t: &[String], i: usize) -> usize {
    let mut depth = 0;
    let mut k = i;
    while k < t.len() {
        if t[k] == "-" && t.get(k + 1).is_some_and(|w| w == ">") {
            k += 2;
            continue;
        }
        match t[k].as_str() {
            "(" | "[" | "{" | "<" => depth += 1,
            ")" | "]" | "}" | ">" => depth -= 1,
            _ => {}
        }
        if depth == 0 { return k; }
        k += 1;
    }
    t.len() - 1
}

/// T split at its top-level commas.
fn split_top(t: &[String]) -> Vec<Vec<String>> {
    let mut parts = vec![];
    let mut part = vec![];
    let mut depth = 0;
    let mut k = 0;
    while k < t.len() {
        if t[k] == "-" && t.get(k + 1).is_some_and(|w| w == ">") {
            part.extend_from_slice(&t[k..k + 2]);
            k += 2;
            continue;
        }
        match t[k].as_str() {
            "(" | "[" | "{" | "<" => depth += 1,
            ")" | "]" | "}" | ">" => depth -= 1,
            _ => {}
        }
        if t[k] == "," && depth == 0 { parts.push(std::mem::take(&mut part)); } else { part.push(t[k].clone()); }
        k += 1;
    }
    if !part.is_empty() { parts.push(part); }
    parts
}

/// A member past its attributes, and whether it is public.
fn member(mut p: &[String]) -> (bool, &[String]) {
    while p.first().is_some_and(|w| w == "#") {
        let open = if p.get(1).is_some_and(|w| w == "!") { 2 } else { 1 };
        p = &p[close(p, open) + 1..];
    }
    if p.first().is_some_and(|w| w == "pub") {
        p = &p[1..];
        if p.first().is_some_and(|w| w == "(") { p = &p[close(p, 0) + 1..]; }
        return (true, p);
    }
    (false, p)
}

/// The types a file declares.
fn declarations(file: &str, t: &[String]) -> Vec<Shape> {
    let mut out = vec![];
    let mut i = 0;
    while i + 2 < t.len() {
        let kind = match t[i].as_str() {
            "struct" | "union" => Some(Kind::Struct),
            "enum" => Some(Kind::Enum),
            "type" if !ASSOCIATED.contains(&t[i + 1].as_str()) => Some(Kind::Alias),
            _ => None,
        };
        let Some(kind) = kind.filter(|_| is_type_name(&t[i + 1])) else { i += 1; continue };
        let mut shape = Shape { file: file.to_owned(), name: t[i + 1].clone(), kind, fields: vec![] };
        let mut j = i + 2;
        if t[j] == "<" {
            j = close(t, j) + 1;
        }
        if kind == Kind::Alias {
            if t.get(j).is_some_and(|w| w == "=") {
                let end = (j..t.len()).find(|&k| t[k] == ";").unwrap_or(t.len());
                shape.fields.push(("=".to_owned(), false, t[j + 1..end].to_vec()));
                out.push(shape);
                i = end;
            } else {
                i += 1;
            }
            continue;
        }
        while j < t.len() && !["{", "(", ";"].contains(&t[j].as_str()) { j += 1; }
        if j < t.len() && t[j] != ";" {
            let end = close(t, j);
            let body = &t[j + 1..end];
            if kind == Kind::Enum {
                shape.fields.push((String::new(), false, body.to_vec()));
            } else if t[j] == "{" {
                for part in split_top(body) {
                    let (public, p) = member(&part);
                    if p.len() >= 2 && p[1] == ":" { shape.fields.push((p[0].clone(), public, p[2..].to_vec())); }
                }
            } else {
                for (n, part) in split_top(body).iter().enumerate() {
                    let (public, p) = member(part);
                    shape.fields.push((n.to_string(), public, p.to_vec()));
                }
            }
            j = end;
        }
        out.push(shape);
        i = j + 1;
    }
    out
}


fn sources() -> Vec<(String, String)> {
    let src = Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    let mut stack = vec![src.clone()];
    let mut out = vec![];
    while let Some(dir) = stack.pop() {
        for entry in std::fs::read_dir(&dir).unwrap() {
            let path = entry.unwrap().path();
            if path.is_dir() {
                stack.push(path);
            } else if path.extension().is_some_and(|e| e == "rs") {
                out.push((path.strip_prefix(&src).unwrap().to_string_lossy().into_owned(), std::fs::read_to_string(&path).unwrap()));
            }
        }
    }
    out
}

/// What FOUND and an ALLOWED list differ by: a new offender, or a listed one gone.
fn ratchet(what: &str, found: &BTreeSet<String>, allowed: &[(&str, &str)]) -> Vec<String> {
    let listed: BTreeSet<String> = allowed.iter().map(|(name, _)| (*name).to_owned()).collect();
    let mut out: Vec<String> = found.difference(&listed).map(|name| format!("{name}: {what}")).collect();
    out.extend(listed.difference(found).map(|name| format!("{name}: no longer {what}; drop its line, the list only shrinks")));
    out
}

fn shapes(sources: &[(String, String)]) -> Vec<Shape> {
    sources.iter().flat_map(|(file, text)| declarations(file, &words(text))).collect()
}

/// Whether TY keeps something behind a cell: a standard cell with its
/// argument, or one of NAMED anywhere.
fn has_cell(ty: &[String], named: &BTreeSet<&str>) -> bool {
    ty.iter().enumerate().any(|(k, w)| {
        (STD_CELLS.contains(&w.as_str()) && ty.get(k + 1).is_some_and(|n| n == "<")) || named.contains(w.as_str())
    })
}

/// The types CELLS makes cells themselves: its `Wrapper` structs, and its
/// aliases whose row is `Held`.
fn named_cells<'c>(cells: &[(&'c str, Behind)], shapes: &[Shape]) -> BTreeSet<&'c str> {
    let alias = |key: &str| {
        let (file, name) = key.split_once(':').unwrap_or((key, key));
        shapes.iter().any(|s| s.file == file && s.name == name && s.kind == Kind::Alias)
    };
    cells.iter()
        .filter(|(key, class)| *class == Wrapper || (*class == Held && alias(key)))
        .filter_map(|(key, _)| key.split_once(':').map(|(_, rest)| rest.split('.').next().unwrap_or(rest)))
        .collect()
}

/// Every declaration in SHAPES behind a cell, keyed as `CELLS` is.
fn cell_declarations(shapes: &[Shape], named: &BTreeSet<&str>) -> BTreeSet<String> {
    let mut out = BTreeSet::new();
    for s in shapes {
        for (field, _, ty) in &s.fields {
            if has_cell(ty, named) {
                out.insert(if s.kind == Kind::Struct { format!("{}:{}.{field}", s.file, s.name) } else { format!("{}:{}", s.file, s.name) });
            }
        }
    }
    out
}

/// The entry types ENTRIES names with a field that isn't private, and those it
/// names that no file declares.
fn public_entries(shapes: &[Shape], entries: &[(&str, &[&str])]) -> (BTreeSet<String>, Vec<String>) {
    let mut public = BTreeSet::new();
    let mut missing = vec![];
    for key in entries.iter().flat_map(|(_, types)| types.iter()).collect::<BTreeSet<_>>() {
        let (file, name) = key.split_once(':').unwrap();
        let found: Vec<&Shape> = shapes.iter().filter(|s| s.file == file && s.name == name && s.kind == Kind::Struct).collect();
        if found.is_empty() { missing.push(format!("{key}: no such struct")); }
        if found.iter().any(|s| s.fields.iter().any(|(_, public, _)| *public)) { public.insert((*key).to_owned()); }
    }
    (public, missing)
}

#[test]
fn cells_and_public_entry_fields_only_shrink() {
    let shapes = shapes(&sources());
    let named = named_cells(CELLS, &shapes);
    let found = cell_declarations(&shapes, &named);
    let listed: BTreeSet<String> = CELLS.iter().map(|(key, _)| (*key).to_owned()).collect();
    let mut problems: Vec<String> = found.difference(&listed).map(|key| format!("{key}: behind a cell, and no class here")).collect();
    problems.extend(listed.difference(&found).map(|key| format!("{key}: no longer behind a cell; drop its row")));
    if listed.len() != CELLS.len() { problems.push("a cell is listed twice".to_owned()); }
    let held = CELLS.iter().filter(|(_, class)| *class == Held).count();
    if held != HELD { problems.push(format!("{held} Held rows against HELD = {HELD}: it only falls, so lower it, or take the new one out")); }
    let variants = members(&body_of(&tokens(&source("data.rs")), "enum", "Object"));
    let named_variants: BTreeSet<String> = ENTRIES.iter().map(|(variant, _)| (*variant).to_owned()).collect();
    problems.extend(variants.symmetric_difference(&named_variants).map(|v| format!("Object variant {v}: ENTRIES and the enum differ")));
    let mut entries = ENTRIES.to_vec();
    entries.push(("closures", &["lib.rs:Closure"]));
    let (public, missing) = public_entries(&shapes, &entries);
    problems.extend(missing);
    problems.extend(ratchet("a table entry type with a field that isn't private", &public, PUBLIC_ALLOWED));
    assert!(problems.is_empty(), "{}", problems.join("\n"));
}

/// The checks can fire: a declaration behind a standard cell, behind a
/// wrapper the census names, and behind a named alias; a public field in an
/// entry; an entry type that is gone; and a stale row. Plain data in a cell
/// that is declared is listed too, and a private entry is not.
#[test]
fn a_new_cell_or_public_entry_field_is_refused() {
    let file = |name: &str, text: &str| (name.to_owned(), text.to_owned());
    let shapes = shapes(&[
        file("a.rs", "struct A { w: std::cell::RefCell<Value>, p: Cell<usize>, x: Shared<Inner>, h: Handle, quiet: Value, c: Cell }"),
        file("b.rs", "pub(crate) struct Shared<T>(Rc<UnsafeCell<T>>); type Handle = Arc<Mutex<Inner>>; enum E { V(OnceCell<u8>), W(u8) }"),
        file("e.rs", "pub(crate) struct Open { pub v: Value } pub(crate) struct Closed { v: Value, s: Shared<u8> }"),
    ]);
    let named = named_cells(&[("b.rs:Shared.0", Wrapper), ("b.rs:Handle", Held), ("b.rs:E", Held)], &shapes);
    let found: Vec<String> = cell_declarations(&shapes, &named).into_iter().collect();
    assert_eq!(found, ["a.rs:A.h", "a.rs:A.p", "a.rs:A.w", "a.rs:A.x", "b.rs:E", "b.rs:Handle", "b.rs:Shared.0", "e.rs:Closed.s"]);
    let (public, missing) = public_entries(&shapes, &[("Open", &["e.rs:Open"]), ("Closed", &["e.rs:Closed"]), ("Gone", &["e.rs:Gone"])]);
    assert_eq!(public.into_iter().collect::<Vec<_>>(), ["e.rs:Open"]);
    assert_eq!(missing, ["e.rs:Gone: no such struct"]);
    let found: BTreeSet<String> = ["new.rs:A".to_owned()].into();
    assert_eq!(ratchet("held", &found, &[("gone.rs:B", "why")]), [
        "new.rs:A: held".to_owned(),
        "gone.rs:B: no longer held; drop its line, the list only shrinks".to_owned(),
    ]);
}
