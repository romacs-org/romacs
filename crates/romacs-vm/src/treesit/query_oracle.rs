//! The same probe runs in separate test binaries on the C and Rust revisions.
//! Its records use tree-local preorder indices, so raw addresses and equal
//! spans on different nodes cannot hide changed captures.
use super::{Language, Node, PredicateStep, Query};
use crate::quit::NativeStop;
use romacs_grammars::BUNDLED;
use serde_json::{Value, json};
use std::collections::HashMap;
use std::hash::{DefaultHasher, Hash, Hasher};
use std::io::{BufWriter, Write};
use std::path::PathBuf;
use tree_sitter::{InputEdit, Point};

struct Case {
    language: String,
    text: String,
    queries: Vec<String>,
}

impl Case {
    fn from_json(value: &Value) -> Self {
        let string = |value: &Value| value.as_str().expect("query corpus string").to_owned();
        Self {
            language: string(&value["language"]),
            text: string(&value["text"]),
            queries: value["queries"]
                .as_array()
                .expect("query corpus array")
                .iter()
                .map(string)
                .collect(),
        }
    }
}

fn language(name: &str) -> Language {
    let grammar = BUNDLED
        .iter()
        .find(|grammar| grammar.name == name)
        .unwrap_or_else(|| panic!("unknown bundled grammar {name}"));
    Language::from_function(grammar.language)
}

type Identity = (usize, usize);

fn tree_index(root: Node) -> (u64, HashMap<Identity, usize>) {
    let mut hash = DefaultHasher::new();
    let mut ids = HashMap::new();
    let mut cursor = root.ts().walk();
    loop {
        let node = cursor.node();
        (
            node.kind_id(),
            node.byte_range(),
            node.start_position(),
            node.end_position(),
            node.is_named(),
            node.is_missing(),
            node.is_extra(),
            node.is_error(),
            node.child_count(),
            cursor.field_id(),
        )
            .hash(&mut hash);
        assert!(ids.insert(Node::from(node).identity(), ids.len()).is_none());
        if cursor.goto_first_child() {
            continue;
        }
        while !cursor.goto_next_sibling() {
            if !cursor.goto_parent() {
                return (hash.finish(), ids);
            }
        }
    }
}

fn metadata(query: &Query) -> Value {
    let captures: Vec<_> = (0..query.capture_count())
        .map(|id| query.capture_name(id))
        .collect();
    let predicates: Vec<Vec<_>> = (0..query.pattern_count())
        .map(|pattern| {
            query
                .predicates(pattern)
                .into_iter()
                .map(|step| match step {
                    PredicateStep::Capture(id) => json!({"capture": id}),
                    PredicateStep::String(bytes) => json!({"string": bytes}),
                    PredicateStep::Done => Value::Null,
                })
                .collect()
        })
        .collect();
    json!({"patterns": query.pattern_count(), "captures": captures, "predicates": predicates})
}

fn matches(
    query: &Query,
    node: Node,
    range: (u32, u32),
    ids: &HashMap<Identity, usize>,
) -> Vec<(u32, Vec<(u32, usize)>)> {
    query
        .matches(node, range, &NativeStop::never())
        .expect("unstopped walk")
        .into_iter()
        .map(|matched| {
            let captures = matched
                .captures
                .into_iter()
                .map(|(node, id)| (id, ids[&node.identity()]))
                .collect();
            (matched.pattern, captures)
        })
        .collect()
}

fn point(text: &[u8], offset: usize) -> Point {
    let before = &text[..offset];
    Point {
        row: before.iter().filter(|&&byte| byte == b'\n').count(),
        column: before
            .iter()
            .rposition(|&byte| byte == b'\n')
            .map_or(offset, |line| offset - line - 1),
    }
}

fn edit(text: &mut Vec<u8>, seed: &mut u64, near: usize) -> InputEdit {
    *seed ^= *seed >> 12;
    *seed ^= *seed << 25;
    *seed ^= *seed >> 27;
    let start_byte = near
        .saturating_sub(8)
        .saturating_add((*seed % 17) as usize)
        .min(text.len());
    let (delete, insert): (usize, &[u8]) = match *seed % 7 {
        0 => (1, b""),
        1 => (2, b"}"),
        2 => (0, b"x"),
        3 => (0, b"\n"),
        4 => (0, b"\""),
        5 => (0, b","),
        _ => (0, b"("),
    };
    let old_end_byte = (start_byte + delete).min(text.len());
    let start_position = point(text, start_byte);
    let old_end_position = point(text, old_end_byte);
    assert!(start_byte <= old_end_byte);
    text.splice(start_byte..old_end_byte, insert.iter().copied());
    let new_end_byte = start_byte + insert.len();
    InputEdit {
        start_byte,
        old_end_byte,
        new_end_byte,
        start_position,
        old_end_position,
        new_end_position: point(text, new_end_byte),
    }
}

fn write_matches(
    output: &mut impl Write,
    tree: &tree_sitter::Tree,
    queries: &[Option<Query>],
    near: u32,
) {
    let root = Node::from(tree.root_node());
    let (digest, ids) = tree_index(root);
    writeln!(output, "tree {digest:016x}").expect("write tree");
    let ranges = [
        (0, u32::MAX),
        (near.saturating_sub(7), near.saturating_add(8)),
        (near, near),
    ];
    for (index, query) in queries.iter().enumerate() {
        let Some(query) = query else { continue };
        for range in ranges {
            let found = matches(query, root, range, &ids);
            writeln!(output, "{index} {} {} {}", range.0, range.1, json!(found))
                .expect("write matches");
        }
    }
}

fn run_case(case: &Case, output: &mut impl Write, edits: u32) {
    assert!(
        !case.text.is_empty() && !case.queries.is_empty(),
        "empty case for {}",
        case.language
    );
    let language = language(&case.language);
    writeln!(
        output,
        "grammar {} {} {}",
        case.language,
        language.abi_version(),
        case.queries.len()
    )
    .expect("write grammar");
    let queries: Vec<_> = case
        .queries
        .iter()
        .enumerate()
        .map(
            |(index, source)| match Query::new(&language, source.as_bytes()) {
                Ok(query) => {
                    writeln!(output, "query {index} {}", metadata(&query)).expect("write query");
                    Some(query)
                }
                Err((offset, kind)) => {
                    writeln!(output, "error {index} {offset} {kind}").expect("write error");
                    None
                }
            },
        )
        .collect();
    assert!(
        queries.iter().any(Option::is_some),
        "all queries failed for {}",
        case.language
    );
    for seed in [1, 2, 3] {
        run_edits(case, output, &language, &queries, edits, seed);
    }
}

fn run_edits(
    case: &Case,
    output: &mut impl Write,
    language: &Language,
    queries: &[Option<Query>],
    edits: u32,
    seed: u64,
) {
    let mut parser = tree_sitter::Parser::new();
    parser
        .set_language(&language.0)
        .expect("compatible grammar");
    let mut text = case.text.as_bytes().to_vec();
    let mut tree = parser.parse(&text, None).expect("fresh tree");
    let mut random = seed;
    let mut near = text.len() / 2;
    writeln!(output, "seed {seed} edit 0").expect("write initial state");
    write_matches(output, &tree, queries, near as u32);
    for index in 1..=edits {
        let edit = edit(&mut text, &mut random, near);
        near = edit.new_end_byte;
        tree.edit(&edit);
        tree = parser.parse(&text, Some(&tree)).expect("incremental tree");
        writeln!(
            output,
            "seed {seed} edit {index} {} {} {}",
            edit.start_byte, edit.old_end_byte, edit.new_end_byte
        )
        .expect("write edit");
        write_matches(output, &tree, queries, near as u32);
    }
}

#[test]
fn query_oracle_preserves_match_and_capture_order() {
    let language = language("json");
    let bytes = b"[1, 2]";
    let mut parser = tree_sitter::Parser::new();
    parser
        .set_language(&language.0)
        .expect("compatible grammar");
    let tree = parser.parse(bytes, None).expect("JSON tree");
    let root = Node::from(tree.root_node());
    let (_, ids) = tree_index(root);
    let query = Query::new(
        &language,
        b"(number) @item (array (number) @first . (number) @second) @parent",
    )
    .expect("query");
    let found = matches(&query, root, (0, u32::MAX), &ids);
    assert_eq!(
        found,
        [
            (0, vec![(0, 3)]),
            (1, vec![(3, 1), (1, 3), (2, 5)]),
            (0, vec![(0, 5)])
        ]
    );
}

/// The manifest supplies one or more cases for every bundled grammar.
/// Run this test by name in each executable and compare the output files.
#[test]
#[ignore = "requires ROMACS_QUERY_CORPUS and ROMACS_QUERY_RECORDS"]
fn query_oracle_record() {
    let path = |name| PathBuf::from(std::env::var_os(name).expect(name));
    let manifest = std::fs::read(path("ROMACS_QUERY_CORPUS")).expect("query corpus");
    let manifest: Vec<Value> = serde_json::from_slice(&manifest).expect("corpus JSON");
    let cases: Vec<_> = manifest.iter().map(Case::from_json).collect();
    let languages: std::collections::BTreeSet<_> =
        cases.iter().map(|case| case.language.as_str()).collect();
    assert_eq!(
        languages,
        BUNDLED.iter().map(|grammar| grammar.name).collect()
    );
    let mut output =
        BufWriter::new(std::fs::File::create(path("ROMACS_QUERY_RECORDS")).expect("records"));
    for case in &cases {
        run_case(case, &mut output, 16);
    }
    output.flush().expect("flush records");
}
