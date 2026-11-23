//! The structural query compiler and matcher for tree-sitter grammars.
mod analysis;
mod compile;
mod execute;
mod ffi;
mod model;

use super::native::{Language, Node, Tree};
use crate::quit::NativeStop;
use ffi::{CCursor, CGrammar};
use model::{ErrorKind, Predicate, Program, query_index};

/// A grammar's immutable structural query and predicate operands.
pub(crate) struct Query(Program);

/// One operand of a pattern's predicates, or the end of a predicate clause.
pub(crate) enum PredicateStep<'a> {
    Capture(u32),
    String(&'a [u8]),
    Done,
}

/// A matched pattern and its ordered captures.
pub(crate) type Match = execute::Match<Node>;

#[derive(Debug)]
pub(crate) struct InvalidQueryNode;

impl Query {
    /// Compile grammar query SOURCE and keep its error's byte offset and kind.
    pub(crate) fn new(language: &Language, source: &[u8]) -> Result<Self, (u32, &'static str)> {
        let expected =
            crate::treesit::native::MIN_ABI_VERSION..=crate::treesit::native::ABI_VERSION;
        if !expected.contains(&language.abi_version()) {
            return Err((0, "Unknown error"));
        }
        let grammar = CGrammar::new(language.binding());
        compile::compile(source, &grammar)
            .map(Self)
            .map_err(|error| {
                let message = match error.kind {
                    ErrorKind::Syntax => "Syntax error at",
                    ErrorKind::NodeType => "Node type error at",
                    ErrorKind::Field => "Field error at",
                    ErrorKind::Capture => "Capture error at",
                    ErrorKind::Structure => "Structure error at",
                };
                (query_index(error.offset), message)
            })
    }

    pub(crate) fn pattern_count(&self) -> u32 {
        query_index(self.0.patterns.len())
    }

    #[cfg(test)]
    pub(crate) fn capture_count(&self) -> u32 {
        query_index(self.0.names.len())
    }

    pub(crate) fn capture_name(&self, index: u32) -> &[u8] {
        self.0.names.get(index as usize).map_or(&[], Box::as_ref)
    }

    pub(crate) fn predicates(&self, pattern: u32) -> impl Iterator<Item = PredicateStep<'_>> {
        let range = self.0.patterns[pattern as usize].predicates.clone();
        self.0.predicates[range]
            .iter()
            .map(|predicate| match *predicate {
                Predicate::Capture(index) => PredicateStep::Capture(index),
                Predicate::String(index) => PredicateStep::String(&self.0.operands[index as usize]),
                Predicate::Done => PredicateStep::Done,
            })
    }

    /// Run this query at NODE in TREE, discarding partial matches on STOP.
    pub(crate) fn matches(
        &self,
        tree: &Tree,
        node: Node,
        range: (u32, u32),
        stop: &NativeStop,
    ) -> Result<Option<Vec<Match>>, InvalidQueryNode> {
        self.try_matches_in(&tree.binding(), node, range, stop)
    }

    #[cfg(test)]
    pub(in crate::treesit) fn matches_in(
        &self,
        tree: &tree_sitter::Tree,
        node: Node,
        range: (u32, u32),
        stop: &NativeStop,
    ) -> Option<Vec<Match>> {
        self.try_matches_in(tree, node, range, stop)
            .expect("node belongs to the oracle tree")
    }

    fn try_matches_in(
        &self,
        tree: &tree_sitter::Tree,
        node: Node,
        range: (u32, u32),
        stop: &NativeStop,
    ) -> Result<Option<Vec<Match>>, InvalidQueryNode> {
        let cursor = CCursor::for_node(tree, node)?;
        let mut due = || stop.due();
        let poll = stop.armed().then_some(&mut due as &mut dyn FnMut() -> bool);
        Ok(execute::matches(&self.0, cursor, range, poll, Node::from))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn json_tree(source: &str) -> (tree_sitter::Language, tree_sitter::Tree) {
        let json = romacs_grammars::BUNDLED
            .iter()
            .find(|grammar| grammar.name == "json")
            .expect("bundled JSON");
        let language = tree_sitter::Language::new(json.language);
        let mut parser = tree_sitter::Parser::new();
        parser.set_language(&language).expect("JSON grammar");
        let tree = parser.parse(source, None).expect("JSON tree");
        (language, tree)
    }

    fn matches(source: &str, query: &str) -> Vec<(u32, Vec<u32>)> {
        let (language, tree) = json_tree(source);
        let grammar = CGrammar::new(&language);
        let query = compile::compile(query.as_bytes(), &grammar).expect("valid query");
        let cursor = CCursor::for_node(&tree, Node::from(tree.root_node())).expect("root cursor");
        execute::matches(&query, cursor, (0, 0), None, std::convert::identity)
            .expect("unstopped scan")
            .into_iter()
            .map(|found| {
                (
                    found.pattern,
                    found.captures.into_iter().map(|(_, id)| id).collect(),
                )
            })
            .collect()
    }

    #[test]
    fn query_rejects_a_node_from_another_tree() {
        let (language, tree) = json_tree("1");
        let (_, other) = json_tree("2");
        let node = Node::from(other.root_node());
        let query =
            Query(compile::compile(b"(number) @n", &CGrammar::new(&language)).expect("query"));
        assert!(
            query
                .try_matches_in(&tree, node, (0, 0), &NativeStop::never())
                .is_err()
        );
    }

    #[test]
    fn wildcard_root_respects_negated_fields() {
        let source = r#"{"k": 1}"#;
        assert!(matches(source, "(pair !key (number) @number) @parent").is_empty());
        assert!(matches(source, "(_ !key (number) @number) @parent").is_empty());
        assert_eq!(
            matches("[1]", "(_ !key (number) @number) @parent"),
            [(0, vec![1, 0])]
        );
    }

    #[test]
    fn all_four_captures_survive_plain_grouped_alternating_and_indexed_roots() {
        let source = r#"{"k": 1}"#;
        assert_eq!(
            matches(source, "(number) @a @b @c @d"),
            [(0, vec![0, 1, 2, 3])]
        );
        assert_eq!(
            matches(source, "((number) @a @b @c @d)"),
            [(0, vec![0, 1, 2, 3])]
        );
        assert_eq!(
            matches("[1]", "[(number) (string)] @a @b @c @d"),
            [(0, vec![0, 1, 2, 3])]
        );
        assert_eq!(
            matches(source, "(_ (number)) @a @b @c @d"),
            [(0, vec![0, 1, 2, 3])]
        );
    }

    #[test]
    fn pattern_start_beyond_65535_steps_is_matched() {
        let query = "(number) @n\n".repeat(32_768) + "(true) @end";
        assert_eq!(matches("true", &query), [(32768, vec![1])]);
    }

    #[test]
    fn a_query_that_finds_nothing_can_stop_and_start_again() {
        let source = format!("[{}0]", "1,".repeat(4096));
        let (language, tree) = json_tree(&source);
        let root = Node::from(tree.root_node());
        let query = compile::compile(b"(string) @s", &CGrammar::new(&language)).expect("query");
        let mut polls = 0;
        let mut stop = || {
            polls += 1;
            polls == 2
        };
        let cursor = || CCursor::for_node(&tree, root).expect("root cursor");
        assert!(
            execute::matches(
                &query,
                cursor(),
                (0, u32::MAX),
                Some(&mut stop),
                std::convert::identity
            )
            .is_none()
        );
        assert_eq!(polls, 2);
        assert!(
            execute::matches(
                &query,
                cursor(),
                (0, u32::MAX),
                None,
                std::convert::identity
            )
            .expect("unstopped query")
            .is_empty()
        );
    }
}
