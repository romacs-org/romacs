//! GNU treesit.c's availability predicate. VM2 has no tree-sitter, so it
//! answers as a GNU built without HAVE_TREE_SITTER does (VM2-TREESIT1).
use super::*;

defsubr! { name = "treesit-available-p", gnu = "treesit.c:5295", effect = Pure,
fn treesit_available_p(cx) {
    let _ = cx;
    Ok(Value::NIL)
}}
