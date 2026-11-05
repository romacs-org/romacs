//! GNU treesit.c: tree-sitter parsers, nodes and queries
//! (`prim/leaf/treesit.rs`). Parsing is lazy, at GNU's two forcing calls
//! (`treesit-parser-root-node', `treesit-parser-changed-regions') and what
//! reaches them; a parse's notifiers, `#pred?' and function predicates
//! cross the traced T2 boundary. The Lisp half (rlisp/vm/treesit.el) is
//! installed the first time a primitive needs it.
use super::*;
use crate::prim::leaf::treesit::Next as TsNext;

const LISP_HALF: &str = include_str!("../../../../rlisp/vm/treesit.el");

fn flag(b: bool) -> Value {
    if b { Value::TRUE } else { Value::NIL }
}

/// Install the Lisp half when HELPER is missing, then continue with THEN.
fn with_half(cx: &mut Leaf<'_>, helper: &str, then: Then, saved: Vec<Value>) -> Next {
    let helper = cx.intern(helper);
    if cx.symbol_function(helper)? != Value::NIL {
        return then(cx, Value::NIL, &saved);
    }
    let source = cx.string(LISP_HALF);
    let form = cx.read_from_string(source, Value::NIL, Value::NIL)?;
    let (form, _) = cx.pair(form).expect("read-from-string returns (OBJECT . POSITION)");
    Ok(Step::Call { f: cx.intern("eval"), args: vec![form, Value::TRUE], then, saved })
}

fn ts_step(next: TsNext, then: Then) -> Next {
    Ok(match next {
        TsNext::Done(value) => Step::Done(value),
        TsNext::Call { f, args, task } => Step::Call { f, args, then, saved: vec![task] },
    })
}

// A forced parse: SAVED is [PARSER, RANGES, AFTER, ARGS...], AFTER naming
// the continuation once the parse's notifiers ran.
const AFTER_ROOT: i64 = 0;
const AFTER_CHANGED: i64 = 1;
const AFTER_CAPTURE: i64 = 2;

fn after(saved: &[Value]) -> Then {
    match saved[2].integer_value() {
        Some(AFTER_ROOT) => root_after,
        Some(AFTER_CHANGED) => changed_after,
        _ => capture_after,
    }
}

/// treesit_ensure_parsed for PARSER, then AFTER with ARGS.
fn forced(cx: &mut Leaf<'_>, parser: Value, kind: i64, args: &[Value]) -> Next {
    let ranges = cx.ts_reparse(parser)?;
    let mut saved = vec![parser, ranges.unwrap_or(Value::NIL), Value::integer(kind).unwrap()];
    saved.extend_from_slice(args);
    if ranges.is_some() && cx.ts_parser(parser)?.notifiers != Value::NIL {
        return with_half(cx, "romacs--treesit-notify", notify, saved);
    }
    if ranges.is_some() {
        cx.ts_reparse_done(parser);
    }
    after(&saved)(cx, Value::NIL, &saved)
}
fn notify(cx: &mut Leaf<'_>, _: Value, saved: &[Value]) -> Next {
    let functions = cx.ts_parser(saved[0])?.notifiers;
    Ok(Step::Call {
        f: cx.intern("romacs--treesit-notify"),
        args: vec![functions, saved[1], saved[0]],
        then: notified,
        saved: saved.to_vec(),
    })
}
fn notified(cx: &mut Leaf<'_>, _: Value, saved: &[Value]) -> Next {
    cx.ts_reparse_done(saved[0]);
    after(saved)(cx, Value::NIL, saved)
}
fn root_after(cx: &mut Leaf<'_>, _: Value, saved: &[Value]) -> Next {
    Ok(Step::Done(cx.ts_root(saved[0])?))
}
fn changed_after(_: &mut Leaf<'_>, _: Value, saved: &[Value]) -> Next {
    Ok(Step::Done(saved[1]))
}
fn capture_after(cx: &mut Leaf<'_>, _: Value, saved: &[Value]) -> Next {
    let root = cx.ts_root(saved[0])?;
    let next = cx.ts_capture(root, saved[3], saved[4], saved[5], saved[6], saved[7])?;
    ts_step(next, capture_resume)
}
fn capture_resume(cx: &mut Leaf<'_>, value: Value, saved: &[Value]) -> Next {
    let next = cx.ts_capture_resume(saved[0], value)?;
    ts_step(next, capture_resume)
}
fn walk_resume(cx: &mut Leaf<'_>, value: Value, saved: &[Value]) -> Next {
    let (guarded, timestamp) = cx.ts_walk_pending_of(saved[0]);
    let next = cx.ts_walk_resume(saved[0], value, guarded, timestamp)?;
    ts_step(next, walk_resume)
}

// Languages.

defsubr! { name = "treesit-available-p", gnu = "treesit.c:5295", effect = Pure,
fn treesit_available_p(cx) {
    let _ = cx;
    Ok(Value::TRUE)
}}

defsubr! { name = "treesit-language-available-p", gnu = "treesit.c:948", effect = Global,
fn treesit_language_available_p(cx, language: Value; optional detail: Value) {
    cx.ts_language_available_p(language, detail)
}}

defsubr! { name = "treesit-library-abi-version", gnu = "treesit.c:980", effect = Pure,
fn treesit_library_abi_version(cx; optional min_compatible: Value) {
    let _ = cx;
    let version = if min_compatible == Value::NIL { crate::treesit::native::ABI_VERSION } else { crate::treesit::native::MIN_ABI_VERSION };
    Ok(Value::integer(i64::from(version)).unwrap())
}}

defsubr! { name = "treesit-language-abi-version", gnu = "treesit.c:997", effect = Global,
fn treesit_language_abi_version(cx; optional language: Value) {
    cx.ts_language_abi_version(language)
}}

defsubr! { name = "treesit-grammar-location", gnu = "treesit.c:1022", effect = Global,
fn treesit_grammar_location(cx, language: Value) {
    cx.ts_grammar_location(language)
}}

defsubr! { name = "romacs-treesit-bundled-grammars", gnu = "romacs:treesit", effect = Reads,
fn romacs_treesit_bundled_grammars(cx) {
    cx.ts_bundled_grammars()
}}

// Line and column tracking.

defsubr! { name = "treesit-tracking-line-column-p", gnu = "treesit.c:1250", effect = Reads,
fn treesit_tracking_line_column_p(cx; optional buffer: Value) {
    cx.ts_tracking(buffer)
}}

defsubr! { name = "treesit-parser-tracking-line-column-p", gnu = "treesit.c:1268", effect = Reads,
fn treesit_parser_tracking_line_column_p(cx, parser: Value) {
    cx.ts_parser_tracking(parser)
}}

defsubr! { name = "treesit--linecol-at", gnu = "treesit.c:5234", effect = Reads,
fn treesit_linecol_at(cx, pos: Value) {
    cx.ts_linecol_at(pos)
}}

defsubr! { name = "treesit--linecol-cache-set", gnu = "treesit.c:5253", effect = Global,
fn treesit_linecol_cache_set(cx, line: Value, col: Value, bytepos: Value) {
    cx.ts_linecol_cache_set(line, col, bytepos)
}}

defsubr! { name = "treesit--linecol-cache", gnu = "treesit.c:5274", effect = Reads,
fn treesit_linecol_cache(cx) {
    cx.ts_linecol_cache()
}}

// Types.

defsubr! { name = "treesit-parser-p", gnu = "treesit.c:2299", effect = Pure,
fn treesit_parser_p(cx, object: Value) {
    Ok(flag(matches!(cx.ts(object), Some(crate::treesit::Object::Parser(_)))))
}}

defsubr! { name = "treesit-node-p", gnu = "treesit.c:2310", effect = Pure,
fn treesit_node_p(cx, object: Value) {
    Ok(flag(matches!(cx.ts(object), Some(crate::treesit::Object::Node(_)))))
}}

defsubr! { name = "treesit-compiled-query-p", gnu = "treesit.c:2321", effect = Pure,
fn treesit_compiled_query_p(cx, object: Value) {
    Ok(flag(matches!(cx.ts(object), Some(crate::treesit::Object::Query(_)))))
}}

defsubr! { name = "treesit-query-p", gnu = "treesit.c:2332", effect = Pure,
fn treesit_query_p(cx, object: Value) {
    Ok(flag(cx.ts_query_p(object)))
}}

defsubr! { name = "treesit-query-eagerly-compiled-p", gnu = "treesit.c:2344", effect = Reads,
fn treesit_query_eagerly_compiled_p(cx, query: Value) {
    cx.ts_query_eager(query)
}}

defsubr! { name = "treesit-query-language", gnu = "treesit.c:2359", effect = Reads,
fn treesit_query_language(cx, query: Value) {
    cx.ts_query_field(query, true)
}}

defsubr! { name = "treesit-query-source", gnu = "treesit.c:2369", effect = Reads,
fn treesit_query_source(cx, query: Value) {
    cx.ts_query_field(query, false)
}}

defsubr! { name = "treesit-node-parser", gnu = "treesit.c:2379", effect = Reads,
fn treesit_node_parser(cx, node: Value) {
    cx.ts_node_parser(node)
}}

// Parsers.

defsubr! { name = "treesit-parser-create", gnu = "treesit.c:2389", effect = Global,
fn treesit_parser_create(cx, language: Value; optional buffer: Value, no_reuse: Value, tag: Value) {
    cx.ts_parser_create(language, buffer, no_reuse, tag)
}}

defsubr! { name = "romacs--treesit-parse-early", gnu = "romacs:treesit", effect = Global,
fn romacs_treesit_parse_early(cx, language: Value) {
    cx.ts_parse_early(language)
}}

defsubr! { name = "treesit-parser-delete", gnu = "treesit.c:2491", effect = Global,
fn treesit_parser_delete(cx, parser: Value) {
    cx.ts_parser_delete(parser)
}}

defsubr! { name = "treesit-parser-list", gnu = "treesit.c:2510", effect = Reads,
fn treesit_parser_list(cx; optional buffer: Value, language: Value, tag: Value) {
    cx.ts_parser_list(buffer, language, tag)
}}

defsubr! { name = "treesit-parser-buffer", gnu = "treesit.c:2563", effect = Reads,
fn treesit_parser_buffer(cx, parser: Value) {
    cx.ts_parser_field(parser, |p| p.buffer)
}}

defsubr! { name = "treesit-parser-language", gnu = "treesit.c:2575", effect = Reads,
fn treesit_parser_language(cx, parser: Value) {
    cx.ts_parser_field(parser, |p| p.language)
}}

defsubr! { name = "treesit-parser-tag", gnu = "treesit.c:2586", effect = Reads,
fn treesit_parser_tag(cx, parser: Value) {
    cx.ts_parser_field(parser, |p| p.tag)
}}

defsubr! { name = "treesit-parser-embed-level", gnu = "treesit.c:2596", effect = Reads,
fn treesit_parser_embed_level(cx, parser: Value) {
    cx.ts_parser_field(parser, |p| p.embed_level)
}}

defsubr! { name = "treesit-parser-set-embed-level", gnu = "treesit.c:2611", effect = Global,
fn treesit_parser_set_embed_level(cx, parser: Value, level: Value) {
    cx.ts_parser_set_embed_level(parser, level)
}}

defsubr! { name = "treesit-parser-root-node", gnu = "treesit.c:2643", effect = Global,
fn treesit_parser_root_node(cx, parser: Value) -> Step {
    cx.ts_check_parser(parser)?;
    forced(cx, parser, AFTER_ROOT, &[])
}}

defsubr! { name = "treesit-parser-changed-regions", gnu = "treesit.c:2935", effect = Global,
fn treesit_parser_changed_regions(cx, parser: Value) -> Step {
    cx.ts_check_parser(parser)?;
    forced(cx, parser, AFTER_CHANGED, &[])
}}

defsubr! { name = "treesit-parser-set-included-ranges", gnu = "treesit.c:2759", effect = Global,
fn treesit_parser_set_included_ranges(cx, parser: Value, ranges: Value) {
    cx.ts_set_included_ranges(parser, ranges)
}}

defsubr! { name = "treesit-parser-included-ranges", gnu = "treesit.c:2817", effect = Global,
fn treesit_parser_included_ranges(cx, parser: Value) {
    cx.ts_included_ranges(parser)
}}

defsubr! { name = "treesit-parser-notifiers", gnu = "treesit.c:2834", effect = Reads,
fn treesit_parser_notifiers(cx, parser: Value) {
    cx.ts_notifiers(parser)
}}

defsubr! { name = "treesit-parser-add-notifier", gnu = "treesit.c:2850", effect = Global,
fn treesit_parser_add_notifier(cx, parser: Value, function: Value) {
    cx.ts_add_notifier(parser, function)
}}

defsubr! { name = "treesit-parser-remove-notifier", gnu = "treesit.c:2870", effect = Global,
fn treesit_parser_remove_notifier(cx, parser: Value, function: Value) {
    cx.ts_remove_notifier(parser, function)
}}

defsubr! { name = "treesit-parse-string", gnu = "treesit.c:2900", effect = Global,
fn treesit_parse_string(cx, string: Value, language: Value) -> Step {
    if cx.kind(language) != Kind::Symbol { return Err(Fault::Wrong("symbolp", language)); }
    if !cx.ts_is_string(string) { return Err(Fault::Wrong("stringp", string)); }
    with_half(cx, "romacs--treesit-parse-string", parse_string, vec![string, language])
}}
fn parse_string(cx: &mut Leaf<'_>, _: Value, saved: &[Value]) -> Next {
    Ok(Step::Call { f: cx.intern("romacs--treesit-parse-string"), args: saved.to_vec(), then: |_, value, _| Ok(Step::Done(value)), saved: vec![] })
}

defsubr! { name = "romacs--treesit-detach", gnu = "treesit.c:2925", effect = Global,
fn romacs_treesit_detach(cx, parser: Value) {
    cx.ts_detach(parser)
}}

// Nodes.

defsubr! { name = "treesit-node-type", gnu = "treesit.c:2994", effect = Reads,
fn treesit_node_type(cx, node: Value) {
    cx.ts_node_type(node)
}}

defsubr! { name = "treesit-node-start", gnu = "treesit.c:3012", effect = Reads,
fn treesit_node_start(cx, node: Value) {
    cx.ts_node_bound(node, false)
}}

defsubr! { name = "treesit-node-end", gnu = "treesit.c:3033", effect = Reads,
fn treesit_node_end(cx, node: Value) {
    cx.ts_node_bound(node, true)
}}

defsubr! { name = "treesit-node-string", gnu = "treesit.c:3053", effect = Reads,
fn treesit_node_string(cx, node: Value) {
    cx.ts_node_string(node)
}}

defsubr! { name = "treesit-node-parent", gnu = "treesit.c:3070", effect = Reads,
fn treesit_node_parent(cx, node: Value) {
    cx.ts_node_parent(node)
}}

defsubr! { name = "treesit-node-child", gnu = "treesit.c:3101", effect = Reads,
fn treesit_node_child(cx, node: Value, n: Value; optional named: Value) {
    cx.ts_node_child(node, n, named)
}}

defsubr! { name = "treesit-node-check", gnu = "treesit.c:3146", effect = Reads,
fn treesit_node_check(cx, node: Value, property: Value) {
    cx.ts_node_check(node, property)
}}

defsubr! { name = "treesit-node-field-name-for-child", gnu = "treesit.c:3202", effect = Reads,
fn treesit_node_field_name_for_child(cx, node: Value, n: Value) {
    cx.ts_node_field_name_for_child(node, n)
}}

defsubr! { name = "treesit-node-child-count", gnu = "treesit.c:3241", effect = Reads,
fn treesit_node_child_count(cx, node: Value; optional named: Value) {
    cx.ts_node_child_count(node, named)
}}

defsubr! { name = "treesit-node-child-by-field-name", gnu = "treesit.c:3264", effect = Reads,
fn treesit_node_child_by_field_name(cx, node: Value, field_name: Value) {
    cx.ts_node_child_by_field_name(node, field_name)
}}

defsubr! { name = "treesit-node-next-sibling", gnu = "treesit.c:3288", effect = Reads,
fn treesit_node_next_sibling(cx, node: Value; optional named: Value) {
    cx.ts_node_sibling(node, named, true)
}}

defsubr! { name = "treesit-node-prev-sibling", gnu = "treesit.c:3314", effect = Reads,
fn treesit_node_prev_sibling(cx, node: Value; optional named: Value) {
    cx.ts_node_sibling(node, named, false)
}}

defsubr! { name = "treesit-node-first-child-for-pos", gnu = "treesit.c:3380", effect = Reads,
fn treesit_node_first_child_for_pos(cx, node: Value, pos: Value; optional named: Value) {
    cx.ts_node_first_child_for_pos(node, pos, named)
}}

defsubr! { name = "treesit-node-descendant-for-range", gnu = "treesit.c:3418", effect = Reads,
fn treesit_node_descendant_for_range(cx, node: Value, beg: Value, end: Value; optional named: Value) {
    cx.ts_node_descendant_for_range(node, beg, end, named)
}}

defsubr! { name = "treesit-node-eq", gnu = "treesit.c:3468", effect = Reads,
fn treesit_node_eq(cx, node1: Value, node2: Value) {
    cx.ts_node_eq(node1, node2)
}}

defsubr! { name = "treesit-subtree-stat", gnu = "treesit.c:5172", effect = Reads,
fn treesit_subtree_stat(cx, node: Value) {
    cx.ts_subtree_stat(node)
}}

// Queries.

defsubr! { name = "treesit-pattern-expand", gnu = "treesit.c:3533", effect = Global,
fn treesit_pattern_expand(cx, pattern: Value) {
    cx.ts_pattern_expand(pattern)
}}

defsubr! { name = "treesit-query-expand", gnu = "treesit.c:3589", effect = Global,
fn treesit_query_expand(cx, query: Value) {
    cx.ts_query_expand(query)
}}

defsubr! { name = "treesit-query-compile", gnu = "treesit.c:3904", effect = Global,
fn treesit_query_compile(cx, language: Value, query: Value; optional eager: Value) {
    cx.ts_query_compile(language, query, eager)
}}

defsubr! { name = "treesit-query-capture", gnu = "treesit.c:4052", effect = Global,
fn treesit_query_capture(cx, node: Value, query: Value; optional beg: Value, end: Value, node_only: Value, grouped: Value) -> Step {
    if !cx.ts_query_p(query) {
        let predicate = cx.intern("treesit-query-p");
        return Err(cx.ts_signal("wrong-type-argument", &[predicate, query]));
    }
    let args = [query, beg, end, node_only, grouped];
    let parser = match cx.ts(node) {
        Some(crate::treesit::Object::Node(_)) => {
            let next = cx.ts_capture(node, query, beg, end, node_only, grouped)?;
            return ts_step(next, capture_resume);
        }
        Some(crate::treesit::Object::Parser(_)) => {
            cx.ts_check_parser(node)?;
            node
        }
        _ if cx.kind(node) == Kind::Symbol => {
            let buffer = cx.current_buffer()?;
            cx.ts_parser_create(node, buffer, Value::NIL, Value::NIL)?
        }
        _ => {
            let predicates = ["or", "treesit-node-p", "treesit-parser-p", "symbolp"].map(|p| cx.intern(p));
            let predicate = cx.ts_list(&predicates);
            return Err(cx.ts_signal("wrong-type-argument", &[predicate, node]));
        }
    };
    forced(cx, parser, AFTER_CAPTURE, &args)
}}

// Search and things.

defsubr! { name = "treesit-search-subtree", gnu = "treesit.c:4831", effect = Global,
fn treesit_search_subtree(cx, node: Value, predicate: Value; optional backward: Value, all: Value, depth: Value) -> Step {
    let next = cx.ts_search_subtree(node, predicate, backward, all, depth)?;
    ts_step(next, walk_resume)
}}

defsubr! { name = "treesit-search-forward", gnu = "treesit.c:4901", effect = Global,
fn treesit_search_forward(cx, start: Value, predicate: Value; optional backward: Value, all: Value) -> Step {
    let next = cx.ts_search_forward(start, predicate, backward, all)?;
    ts_step(next, walk_resume)
}}

defsubr! { name = "treesit-induce-sparse-tree", gnu = "treesit.c:5025", effect = Global,
fn treesit_induce_sparse_tree(cx, root: Value, predicate: Value; optional process_fn: Value, depth: Value) -> Step {
    let next = cx.ts_induce_sparse_tree(root, predicate, process_fn, depth)?;
    ts_step(next, walk_resume)
}}

defsubr! { name = "treesit-node-match-p", gnu = "treesit.c:5119", effect = Global,
fn treesit_node_match_p(cx, node: Value, predicate: Value; optional ignore_missing: Value) -> Step {
    let next = cx.ts_node_match_p(node, predicate, ignore_missing)?;
    ts_step(next, walk_resume)
}}
