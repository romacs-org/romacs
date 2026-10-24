//! GNU data.c arithmetic driver (3199) and bitwise DEFUNs (3494ff).
use crate::prim::{Fault, Leaf};
use crate::Value;

fn integer_marker(cx: &Leaf<'_>, value: Value, first: bool) -> Result<num_bigint::BigInt, Fault> {
    if let Some(n) = cx.bigint_value(value) {
        return Ok(n);
    }
    if let Some(position) = cx.marker_position(value) {
        return position
            .map(Into::into)
            .ok_or_else(|| Fault::Error("Marker does not point anywhere".into()));
    }
    let predicate = if first || cx.float_value(value).is_ok() {
        "integer-or-marker-p"
    } else {
        "number-or-marker-p"
    };
    Err(Fault::Wrong(predicate, value))
}

defsubr! { name = "logior", gnu = "data.c:3506", effect = Reads,
    fn logior(cx; rest args) {
        if args.len() == 1 && cx.bigint_value(args[0]).is_some() { return Ok(args[0]); }
        let mut result = num_bigint::BigInt::from(0);
        for (i, value) in args.iter().copied().enumerate() {
            result |= integer_marker(cx, value, i == 0)?;
        }
        Ok(cx.bigint(result))
    }
}

defsubr! { name = "logand", gnu = "data.c:3494", effect = Reads,
    fn logand(cx; rest args) {
        if args.len() == 1 && cx.bigint_value(args[0]).is_some() { return Ok(args[0]); }
        let mut result = num_bigint::BigInt::from(-1);
        for (i, value) in args.iter().copied().enumerate() {
            result &= integer_marker(cx, value, i == 0)?;
        }
        Ok(cx.bigint(result))
    }
}
