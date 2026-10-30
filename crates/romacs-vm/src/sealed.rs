//! GNU's byte-op primitive set: the calls `bytecomp.el` compiles to a byte
//! code instead of a function call. Byte code never consults the function
//! cell (`src/bytecode.c`: `Bcar` reads the cons, `Bplus` adds), so compiled
//! GNU code ignores a redefinition or advice of these names, and so does
//! every Romacs call of them (DIVERGENCES.md VM2-SEALED1).
//!
//! The rows are `bytecomp.el`'s `byte-defop-compiler` forms with an opcode,
//! in file order, with the arities GNU's handlers emit the opcode for: a call
//! with any other number of arguments is an ordinary function call there and
//! here. `sealed_set_matches_gnu_bytecomp` pins the table to the pinned file.
use super::{prim, PRIMITIVES, Value};
use std::sync::OnceLock;

/// One `byte-defop-compiler` row: `name` compiles to the primitive `target`
/// when called with `min..=max` arguments.
struct Row {
    name: &'static str,
    target: &'static str,
    min: usize,
    max: usize,
}
const fn row(name: &'static str, min: usize, max: usize) -> Row {
    Row { name, target: name, min, max }
}
const fn alias(name: &'static str, target: &'static str, min: usize, max: usize) -> Row {
    Row { name, target, min, max }
}
/// `byte-compile-list`, `-concat` and `-insert` take up to 255 operands.
/// (`insert` with more still gets opcodes when no later operand is a call;
/// such a call is an ordinary call here.)
const OPERANDS: usize = 255;
const ROWS: &[Row] = &[
    row("point", 0, 0),
    row("point-max", 0, 0),
    row("point-min", 0, 0),
    row("following-char", 0, 0),
    row("preceding-char", 0, 0),
    row("current-column", 0, 0),
    row("eolp", 0, 0),
    row("eobp", 0, 0),
    row("bolp", 0, 0),
    row("bobp", 0, 0),
    row("current-buffer", 0, 0),
    row("widen", 0, 0),
    row("end-of-line", 0, 1),
    row("forward-char", 0, 1),
    row("forward-line", 0, 1),
    row("symbolp", 1, 1),
    row("consp", 1, 1),
    row("stringp", 1, 1),
    row("listp", 1, 1),
    alias("not", "null", 1, 1),
    row("null", 1, 1),
    row("car", 1, 1),
    row("cdr", 1, 1),
    row("length", 1, 1),
    row("symbol-value", 1, 1),
    row("symbol-function", 1, 1),
    row("1+", 1, 1),
    row("1-", 1, 1),
    row("goto-char", 1, 1),
    row("char-after", 0, 1),
    row("set-buffer", 1, 1),
    row("forward-word", 0, 1),
    row("char-syntax", 1, 1),
    row("nreverse", 1, 1),
    row("car-safe", 1, 1),
    row("cdr-safe", 1, 1),
    row("numberp", 1, 1),
    row("integerp", 1, 1),
    row("skip-chars-forward", 1, 2),
    row("skip-chars-backward", 1, 2),
    row("eq", 2, 2),
    row("memq", 2, 2),
    row("cons", 2, 2),
    row("aref", 2, 2),
    row("set", 2, 2),
    row("fset", 2, 2),
    // byte-compile-cmp: two operands take the opcode; one compiles to `t`
    // without a call, which Romacs does not reproduce (an ordinary call).
    row("=", 2, 2),
    row("<", 2, 2),
    row(">", 2, 2),
    row("<=", 2, 2),
    row(">=", 2, 2),
    row("get", 2, 2),
    row("nth", 2, 2),
    row("substring", 1, 3),
    alias("move-marker", "set-marker", 2, 3),
    row("set-marker", 2, 3),
    row("match-beginning", 1, 1),
    row("match-end", 1, 1),
    row("upcase", 1, 1),
    row("downcase", 1, 1),
    alias("string=", "string-equal", 2, 2),
    alias("string<", "string-lessp", 2, 2),
    row("string-equal", 2, 2),
    row("string-lessp", 2, 2),
    row("equal", 2, 2),
    row("nthcdr", 2, 2),
    row("elt", 2, 2),
    row("member", 2, 2),
    row("assq", 2, 2),
    alias("rplaca", "setcar", 2, 2),
    alias("rplacd", "setcdr", 2, 2),
    row("setcar", 2, 2),
    row("setcdr", 2, 2),
    row("buffer-substring", 2, 2),
    row("delete-region", 2, 2),
    row("narrow-to-region", 2, 2),
    row("%", 2, 2),
    row("aset", 3, 3),
    // byte-compile-min-max: no operands is an ordinary (failing) call.
    row("max", 1, 2),
    row("min", 1, 2),
    // byte-compile-variadic-numeric: none folds to the identity, one is
    // `(* x 1)`, two take the opcode, more are an ordinary call.
    row("+", 0, 2),
    row("*", 0, 2),
    row("list", 0, OPERANDS),
    row("concat", 0, OPERANDS),
    alias("indent-to-column", "indent-to", 1, 1),
    row("indent-to", 1, 1),
    row("insert", 0, OPERANDS),
    // byte-compile-minus: one operand negates, otherwise variadic-numeric.
    row("-", 0, 2),
    row("/", 2, 2),
    // byte-compile-nconc: none is nil, one is the operand, more chain.
    row("nconc", 0, usize::MAX),
];

#[derive(Clone, Copy)]
struct Sealed {
    min: usize,
    max: usize,
    target: Value,
}
/// The primitive declared under NAME: a legacy row or a generated defsubr.
fn builtin(name: &str) -> Option<Value> {
    PRIMITIVES
        .iter()
        .position(|(n, _)| *n == name)
        .map(|i| Value::reference(i, 6))
        .or_else(|| prim::subr_named(name))
}
fn table() -> &'static rustc_hash::FxHashMap<&'static str, Sealed> {
    static TABLE: OnceLock<rustc_hash::FxHashMap<&'static str, Sealed>> = OnceLock::new();
    TABLE.get_or_init(|| {
        ROWS.iter()
            .filter_map(|r| {
                let target = builtin(r.target)?;
                Some((r.name, Sealed { min: r.min, max: r.max, target }))
            })
            .collect()
    })
}
/// The primitive a call of NAME with ARGC arguments compiles to, when GNU
/// compiles that call to a byte code and Romacs has the primitive.
/// Out of line: the compiler's recursion runs on a bounded native stack.
#[inline(never)]
pub(crate) fn target(name: &str, argc: usize) -> Option<Value> {
    let s = table().get(name)?;
    (s.min..=s.max).contains(&argc).then_some(s.target)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Opcode name -> primitive, for the rows whose opcode is not `byte-NAME`
    /// (`src/bytecode.c` implements each with the named primitive's semantics).
    const OPCODES: &[(&str, &str)] = &[
        ("byte-not", "null"),
        ("byte-add1", "1+"),
        ("byte-sub1", "1-"),
        ("byte-eqlsign", "="),
        ("byte-lss", "<"),
        ("byte-gtr", ">"),
        ("byte-leq", "<="),
        ("byte-geq", ">="),
        ("byte-set-marker", "set-marker"),
        ("byte-string=", "string-equal"),
        ("byte-string<", "string-lessp"),
        ("byte-setcar", "setcar"),
        ("byte-setcdr", "setcdr"),
        ("byte-rem", "%"),
        ("byte-plus", "+"),
        ("byte-mult", "*"),
        ("byte-indent-to", "indent-to"),
        ("byte-diff", "-"),
        ("byte-quo", "/"),
    ];
    /// The arities each compile handler emits its opcode for.
    fn arities(handler: &str, name: &str) -> (usize, usize) {
        match handler {
            "0" => (0, 0),
            "1" => (1, 1),
            "2" | "2-cmp" => (2, 2),
            "3" => (3, 3),
            "0-1" => (0, 1),
            "1-2" => (1, 2),
            "2-3" => (2, 3),
            "1-3" => (1, 3),
            "byte-compile-min-max" => (1, 2),
            "byte-compile-variadic-numeric" | "byte-compile-minus" => (0, 2),
            "byte-compile-quo" => (2, 2),
            "byte-compile-indent-to" => (1, 1),
            "" => match name {
                "list" | "concat" | "insert" => (0, OPERANDS),
                "nconc" => (0, usize::MAX),
                "indent-to" => (1, 1),
                _ => panic!("no arity rule for byte-compile-{name}"),
            },
            _ => panic!("no arity rule for handler {handler} of {name}"),
        }
    }
    /// `(byte-defop-compiler SPEC [HANDLER])` rows of the pinned bytecomp.el
    /// that name an opcode: (function, primitive, min, max).
    fn gnu_rows() -> Vec<(String, String, usize, usize)> {
        let path = concat!(env!("CARGO_MANIFEST_DIR"), "/../../gnu-lisp/emacs-lisp/bytecomp.el");
        let text = std::fs::read_to_string(path).unwrap();
        let mut rows = vec![];
        for line in text.lines() {
            let Some(rest) = line.strip_prefix("(byte-defop-compiler ") else { continue };
            let rest = rest.trim_end().strip_suffix(')').unwrap();
            let (spec, handler) = if let Some(body) = rest.strip_prefix('(') {
                let (spec, handler) = body.split_once(')').unwrap();
                (spec.to_owned(), handler.trim().to_owned())
            } else {
                let mut parts = rest.split_whitespace();
                (parts.next().unwrap().to_owned(), parts.next().unwrap_or("").to_owned())
            };
            let (name, opcode) = match spec.split_once(' ') {
                Some((name, opcode)) => (name.to_owned(), opcode.trim().to_owned()),
                None => (spec.clone(), format!("byte-{spec}")),
            };
            let target = OPCODES
                .iter()
                .find(|(o, _)| *o == opcode)
                .map(|(_, p)| (*p).to_owned())
                .unwrap_or_else(|| opcode.strip_prefix("byte-").unwrap().to_owned());
            let (min, max) = arities(&handler, &name);
            rows.push((name, target, min, max));
        }
        rows
    }

    #[test]
    fn sealed_set_matches_gnu_bytecomp() {
        let gnu = gnu_rows();
        assert!(gnu.len() > 80, "bytecomp.el rows not parsed: {}", gnu.len());
        let ours: Vec<_> = ROWS
            .iter()
            .map(|r| (r.name.to_owned(), r.target.to_owned(), r.min, r.max))
            .collect();
        assert_eq!(ours, gnu, "ROWS must be bytecomp.el's byte-defop-compiler forms in order");
    }

    #[test]
    fn every_sealed_name_has_its_primitive_and_calls_what_the_cell_holds() {
        let mut vm = crate::Vm::new();
        vm.eval_source("nil").unwrap();
        let mut missing = vec![];
        for r in ROWS {
            let Some(primitive) = builtin(r.target) else {
                missing.push(r.name);
                continue;
            };
            assert_eq!(target(r.name, r.min), Some(primitive), "{}", r.name);
            // GNU's aliases are its own `defalias` rows in subr.el; Romacs
            // defines the targets, and the names when it has them.
            let slot = vm.slot(r.name);
            let cell = vm.slots[slot].function;
            if cell != Value::UNBOUND && r.name == r.target {
                assert_eq!(cell, primitive, "the cell of {} does not hold the sealed primitive", r.name);
            }
        }
        assert_eq!(missing, Vec::<&str>::new(), "byte-op names without a Romacs primitive");
        assert_eq!(target("car", 2), None);
        assert_eq!(target("list", 256), None);
        assert_eq!(target("nconc", 300), builtin("nconc"));
        assert_eq!(target("plist-get", 2), None);
    }
}
