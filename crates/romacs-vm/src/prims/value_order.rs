//! GNU standard value ordering, following src/fns.c value_cmp.
use super::*;
use crate::prim::Kind;
use std::cmp::Ordering;

defsubr! { name = "value<", gnu = "fns.c:3291", effect = Reads,
fn value_less(cx, a: Value, b: Value) {
    Ok(if value_cmp(cx, a, b, 200)? == Ordering::Less { Value::TRUE } else { Value::NIL })
}}

fn basic_kind(kind: Kind) -> Kind {
    match kind {
        Kind::Bytes => Kind::String,
        Kind::ByteBuffer => Kind::Buffer,
        Kind::SyntaxTable => Kind::CharTable,
        kind => kind,
    }
}

fn vectorlike(kind: Kind) -> bool {
    matches!(
        kind,
        Kind::Bignum
            | Kind::Vector
            | Kind::BoolVector
            | Kind::Record
            | Kind::Marker
            | Kind::Buffer
            | Kind::Closure
            | Kind::Subr
            | Kind::HashTable
            | Kind::Obarray
            | Kind::CharTable
            | Kind::SubCharTable
            | Kind::Overlay
            | Kind::Window
            | Kind::Frame
            | Kind::Terminal
            | Kind::Thread
    )
}

fn mismatch(cx: &mut Leaf<'_>, a: Value, b: Value) -> Fault {
    let condition = cx.intern("type-mismatch");
    let tail = cx.cons(b, Value::NIL);
    Fault::Signal(condition, cx.cons(a, tail))
}

fn string_cmp(a: &crate::lstring::OwnedStr, b: &crate::lstring::OwnedStr) -> Ordering {
    a.chars().cmp(b.chars())
}

fn value_cmp(
    cx: &mut Leaf<'_>,
    mut a: Value,
    mut b: Value,
    depth: i32,
) -> std::result::Result<Ordering, Fault> {
    if depth < 0 {
        return Err(Fault::Error("Maximum depth exceeded in comparison".into()));
    }
    loop {
        if a == b {
            return Ok(Ordering::Equal);
        }
        let ka = basic_kind(cx.kind(a));
        let kb = basic_kind(cx.kind(b));
        match ka {
            Kind::Integer | Kind::Float | Kind::Bignum => {
                if let (Some(x), Some(y)) = (cx.bigint_value(a), cx.bigint_value(b)) {
                    return Ok(x.cmp(&y));
                }
                if ka == Kind::Float && kb == Kind::Float {
                    return Ok(cx
                        .float_value(a)?
                        .partial_cmp(&cx.float_value(b)?)
                        .unwrap_or(Ordering::Equal));
                }
                if let Some(x) = cx.bigint_value(a) {
                    if kb == Kind::Float {
                        return Ok(crate::numbers::int_float_cmp(&x, cx.float_value(b)?)
                            .unwrap_or(Ordering::Equal));
                    }
                }
                if ka == Kind::Float {
                    if let Some(y) = cx.bigint_value(b) {
                        return Ok(crate::numbers::int_float_cmp(&y, cx.float_value(a)?)
                            .unwrap_or(Ordering::Equal)
                            .reverse());
                    }
                }
                if ka == Kind::Bignum && !vectorlike(kb) {
                    return value_cmp(cx, b, a, depth).map(Ordering::reverse);
                }
            }
            Kind::Symbol => {
                if kb == Kind::Symbol {
                    return Ok(string_cmp(
                        &cx.value_order_symbol_name(a)?,
                        &cx.value_order_symbol_name(b)?,
                    ));
                }
                if a == Value::NIL && kb == Kind::Cons {
                    return Ok(Ordering::Less);
                }
            }
            Kind::String => {
                if kb == Kind::String {
                    return Ok(string_cmp(&cx.string_view(a)?, &cx.string_view(b)?));
                }
            }
            Kind::Cons => {
                let (mut tortoise, mut steps) = (b, 0);
                while let Some((bcar, bcdr)) = cx.pair(b) {
                    let (acar, acdr) = cx
                        .pair(a)
                        .expect("cons ordering keeps the left cons until its tail step");
                    let order = value_cmp(cx, acar, bcar, depth - 1)?;
                    if order != Ordering::Equal {
                        return Ok(order);
                    }
                    a = acdr;
                    if cx.pair(a).is_none() {
                        b = bcdr;
                        break;
                    }
                    (b, tortoise, steps) =
                        super::fns_equal::for_each_tail_step(cx, b, tortoise, steps)?;
                }
                if cx.pair(a).is_none() {
                    continue;
                }
                if b == Value::NIL {
                    return Ok(Ordering::Greater);
                }
            }
            Kind::Vector | Kind::Record if ka == kb => {
                let (x, y) = if ka == Kind::Vector {
                    (cx.vector_values(a)?, cx.vector_values(b)?)
                } else {
                    (cx.value_order_record(a), cx.value_order_record(b))
                };
                for (&x, &y) in x.iter().zip(&y) {
                    let order = value_cmp(cx, x, y, depth - 1)?;
                    if order != Ordering::Equal {
                        return Ok(order);
                    }
                }
                return Ok(x.len().cmp(&y.len()));
            }
            Kind::BoolVector if kb == Kind::BoolVector => {
                let (x, y) = (cx.bool_vector(a)?, cx.bool_vector(b)?);
                for i in 0..x.len.min(y.len) {
                    let order = x.get(i).cmp(&y.get(i));
                    if order != Ordering::Equal {
                        return Ok(order);
                    }
                }
                return Ok(x.len.cmp(&y.len));
            }
            Kind::Marker if kb == Kind::Marker => {
                let (x, y) = (
                    cx.value_order_marker_buffer(a),
                    cx.value_order_marker_buffer(b),
                );
                if x == Value::NIL || y == Value::NIL {
                    return Ok((x != Value::NIL).cmp(&(y != Value::NIL)));
                }
                let order = value_cmp(cx, x, y, depth - 1)?;
                if order != Ordering::Equal {
                    return Ok(order);
                }
                return Ok(cx.marker_position(a).cmp(&cx.marker_position(b)));
            }
            Kind::Buffer if kb == Kind::Buffer => {
                let x = cx
                    .buffer_state(a)
                    .expect("buffer kind has state")
                    .name
                    .as_deref();
                let y = cx
                    .buffer_state(b)
                    .expect("buffer kind has state")
                    .name
                    .as_deref();
                return Ok(x.cmp(&y));
            }
            Kind::Closure
            | Kind::Subr
            | Kind::HashTable
            | Kind::Obarray
            | Kind::CharTable
            | Kind::SubCharTable
            | Kind::Overlay
            | Kind::Window
            | Kind::Frame
            | Kind::Terminal
            | Kind::Thread
                if ka == kb =>
            {
                return Ok(Ordering::Equal);
            }
            Kind::Cell | Kind::Unbound | Kind::Invalid => {
                return Err(Fault::Error(
                    "Internal value has no standard value order".into(),
                ));
            }
            _ => {}
        }
        return Err(mismatch(cx, a, b));
    }
}
