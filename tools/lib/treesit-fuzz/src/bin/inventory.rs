//! inventory: the grammars Romacs ships, one name a line, from
//! `romacs_grammars::BUNDLED`. tools/treesit-fuzz reads its inventory here,
//! built from the tree under test, never from the worker it checks.
fn main() {
    for grammar in romacs_grammars::BUNDLED {
        println!("{}", grammar.name);
    }
}
