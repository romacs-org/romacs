//! GNU's special forms (UNEVALLED DEFUNs) as the subrs in their function
//! cells, so `fboundp', `symbol-function', `subrp', `subr-arity' and
//! `special-form-p' see what GNU does. The compiler handles the forms by
//! name; calling one as a function signals `invalid-function' with the subr,
//! as funcall_subr does for UNEVALLED.
use super::*;

/// Each special form and its minimum argument count (`subr-arity').
pub(crate) const SPECIAL_FORMS: &[(&str, usize)] = &[
    ("and", 0), ("catch", 1), ("cond", 0), ("condition-case", 2), ("defconst", 2), ("defvar", 1), ("function", 1), ("if", 2), ("interactive", 0), ("let", 1), ("let*", 1), ("or", 0), ("prog1", 1), ("progn", 0), ("quote", 1), ("save-current-buffer", 0), ("save-excursion", 0), ("save-restriction", 0), ("setq", 0), ("unwind-protect", 1), ("while", 1),
];

/// The `(MIN . unevalled)' minimum of special form SUBR's name, if it is one.
pub(crate) fn special_form_min(name: &str) -> Option<usize> {
    SPECIAL_FORMS.iter().find(|(form, _)| *form == name).map(|&(_, min)| min)
}

fn called(cx: &mut Leaf<'_>, name: &str) -> Out {
    let subr = crate::prim::subr_named(name).expect("declared special form");
    let condition = cx.intern("invalid-function");
    let data = cx.cons(subr, Value::NIL);
    Err(Fault::Signal(condition, data))
}

defsubr! { name = "and", gnu = "eval.c:365", effect = Pure,
fn special_and(cx; rest _args) { called(cx, "and") }}

defsubr! { name = "catch", gnu = "eval.c:1351", effect = Pure,
fn special_catch(cx; rest _args) { called(cx, "catch") }}

defsubr! { name = "cond", gnu = "eval.c:403", effect = Pure,
fn special_cond(cx; rest _args) { called(cx, "cond") }}

defsubr! { name = "condition-case", gnu = "eval.c:1500", effect = Pure,
fn special_condition_case(cx; rest _args) { called(cx, "condition-case") }}

defsubr! { name = "defconst", gnu = "eval.c:1018", effect = Pure,
fn special_defconst(cx; rest _args) { called(cx, "defconst") }}

defsubr! { name = "defvar", gnu = "eval.c:944", effect = Pure,
fn special_defvar(cx; rest _args) { called(cx, "defvar") }}

defsubr! { name = "function", gnu = "eval.c:560", effect = Pure,
fn special_function(cx; rest _args) { called(cx, "function") }}

defsubr! { name = "if", gnu = "eval.c:386", effect = Pure,
fn special_if(cx; rest _args) { called(cx, "if") }}

defsubr! { name = "interactive", gnu = "callint.c:37", effect = Pure,
fn special_interactive(cx; rest _args) { called(cx, "interactive") }}

defsubr! { name = "let", gnu = "eval.c:1133", effect = Pure,
fn special_let(cx; rest _args) { called(cx, "let") }}

defsubr! { name = "let*", gnu = "eval.c:1076", effect = Pure,
fn special_let_star(cx; rest _args) { called(cx, "let*") }}

defsubr! { name = "or", gnu = "eval.c:344", effect = Pure,
fn special_or(cx; rest _args) { called(cx, "or") }}

defsubr! { name = "prog1", gnu = "eval.c:458", effect = Pure,
fn special_prog1(cx; rest _args) { called(cx, "prog1") }}

defsubr! { name = "progn", gnu = "eval.c:433", effect = Pure,
fn special_progn(cx; rest _args) { called(cx, "progn") }}

defsubr! { name = "quote", gnu = "eval.c:507", effect = Pure,
fn special_quote(cx; rest _args) { called(cx, "quote") }}

defsubr! { name = "save-current-buffer", gnu = "editfns.c:843", effect = Pure,
fn special_save_current_buffer(cx; rest _args) { called(cx, "save-current-buffer") }}

defsubr! { name = "save-excursion", gnu = "editfns.c:818", effect = Pure,
fn special_save_excursion(cx; rest _args) { called(cx, "save-excursion") }}

defsubr! { name = "save-restriction", gnu = "editfns.c:3150", effect = Pure,
fn special_save_restriction(cx; rest _args) { called(cx, "save-restriction") }}

defsubr! { name = "setq", gnu = "eval.c:470", effect = Pure,
fn special_setq(cx; rest _args) { called(cx, "setq") }}

defsubr! { name = "unwind-protect", gnu = "eval.c:1484", effect = Pure,
fn special_unwind_protect(cx; rest _args) { called(cx, "unwind-protect") }}

defsubr! { name = "while", gnu = "eval.c:1200", effect = Pure,
fn special_while(cx; rest _args) { called(cx, "while") }}
