//! GNU numeric identity exposed through the existing equality substrate.
use super::*;

defsubr! { name = "eql", gnu = "fns.c:2800", effect = Pure,
fn eql(cx, first: Value, second: Value) {
    Ok(if cx.eql(first, second) { Value::TRUE } else { Value::NIL })
}}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn native_bignum_eql_is_exact() {
        let mut vm = Vm::new();
        let mut cx = Leaf::new(&mut vm);
        let n = num_bigint::BigInt::from(1u64) << 100usize;
        let a = cx.bigint(n.clone());
        let b = cx.bigint(n.clone());
        let c = cx.bigint(n + 1);
        assert_eq!(eql(&mut cx, a, b).ok(), Some(Value::TRUE));
        assert_eq!(eql(&mut cx, a, c).ok(), Some(Value::NIL));
    }
}
