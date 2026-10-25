//! Numeric slow path for promoted integers and mixed floating arithmetic;
//! leaf allocation does not introduce a GC safepoint.
use super::*;
use num_bigint::{BigInt, Sign, ToBigInt};
use std::cmp::Ordering;

pub(super) fn print_float(n: f64) -> String {
    if n.is_nan() {
        return format!("{}0.0e+NaN", if n.is_sign_negative() { "-" } else { "" });
    }
    if n.is_infinite() {
        return format!("{}1.0e+INF", if n.is_sign_negative() { "-" } else { "" });
    }
    // GNU starts at DBL_DIG significant digits, increasing until the decimal
    // round-trips. %g's fixed/scientific threshold follows that precision.
    for precision in 15..=17 {
        let scientific = format!("{:.*e}", precision - 1, n);
        if scientific
            .parse::<f64>()
            .is_ok_and(|v| v.to_bits() == n.to_bits())
        {
            let (mantissa, exponent) = scientific.split_once('e').unwrap();
            let exponent: i32 = exponent.parse().unwrap();
            if exponent >= -4 && exponent < precision as i32 {
                let digits = (precision as i32 - 1 - exponent).max(0) as usize;
                let mut out = format!("{n:.digits$}");
                if out.contains('.') {
                    while out.ends_with('0') {
                        out.pop();
                    }
                    if out.ends_with('.') {
                        out.push('0');
                    }
                } else {
                    out.push_str(".0");
                }
                return out;
            }
            let mantissa = mantissa.trim_end_matches('0').trim_end_matches('.');
            return format!("{mantissa}e{exponent:+03}");
        }
    }
    unreachable!("17 significant digits round-trip binary64")
}

pub(super) fn int_float_cmp(i: &BigInt, f: f64) -> Option<Ordering> {
    if f.is_nan() {
        return None;
    }
    if f.is_infinite() {
        return Some(if f.is_sign_positive() {
            Ordering::Less
        } else {
            Ordering::Greater
        });
    }
    let truncated = f.to_bigint().expect("finite float is an integer magnitude");
    let order = i.cmp(&truncated);
    if order != Ordering::Equal {
        return Some(order);
    }
    0.0f64.partial_cmp(&f.fract())
}

pub(super) fn integer_float(n: &BigInt) -> f64 {
    n.to_string().parse().expect("decimal integer is a float")
}

enum Number {
    Integer(BigInt),
    Float(f64),
}
impl Number {
    fn float(&self) -> f64 {
        match self {
            Self::Integer(n) => integer_float(n),
            Self::Float(n) => *n,
        }
    }
}

impl Vm {
    pub(super) fn bigint_value(&self, v: Value) -> Option<BigInt> {
        if let Some(n) = v.integer_value() {
            return Some(n.into());
        }
        match self.object_if(v) {
            Some(data::Object::Bignum(n)) => Some((**n).clone()),
            _ => None,
        }
    }
    pub(super) fn bigint(&mut self, n: BigInt) -> Value {
        if let Ok(n) = i64::try_from(&n) {
            if let Ok(v) = Value::integer(n) {
                return v;
            }
        }
        self.allocate_object(data::Object::Bignum(Box::new(n)))
    }
    pub(super) fn coerce_number(&self, v: Value, markers: bool) -> Result<Value> {
        if v.integer_value().is_some()
            || matches!(
                self.object_if(v),
                Some(data::Object::Bignum(_) | data::Object::Float(_))
            )
        {
            return Ok(v);
        }
        if markers && matches!(self.object_if(v), Some(data::Object::Marker(_))) {
            return self.marker_position_value(v).and_then(|position| {
                if position == Value::NIL {
                    Err("Marker does not point anywhere".into())
                } else {
                    Ok(position)
                }
            });
        }
        Err("number required".into())
    }
    fn arithmetic_number(&self, v: Value, markers: bool) -> Result<Number> {
        let v = self.coerce_number(v, markers)?;
        if let Some(n) = self.bigint_value(v) {
            return Ok(Number::Integer(n));
        }
        match self.object_if(v) {
            Some(data::Object::Float(n)) => Ok(Number::Float(*n)),
            _ => unreachable!(),
        }
    }
    pub(super) fn number(&self, v: Value) -> Result<f64> {
        Ok(self.arithmetic_number(v, false)?.float())
    }
    pub(super) fn numeric_cmp(&self, x: Value, y: Value) -> Result<Option<Ordering>> {
        Ok(
            match (
                self.arithmetic_number(x, true)?,
                self.arithmetic_number(y, true)?,
            ) {
                (Number::Integer(x), Number::Integer(y)) => Some(x.cmp(&y)),
                (Number::Integer(x), Number::Float(y)) => int_float_cmp(&x, y),
                (Number::Float(x), Number::Integer(y)) => {
                    int_float_cmp(&y, x).map(Ordering::reverse)
                }
                (Number::Float(x), Number::Float(y)) => x.partial_cmp(&y),
            },
        )
    }
    pub(super) fn add_primitive(&mut self, p: Primitive, start: usize, n: usize) -> Result<Value> {
        let mut sum = 0i64;
        for value in &self.stack[start..start + n] {
            let Some(next) = value.integer_value().and_then(|v| sum.checked_add(v)) else {
                return self.numeric_primitive(p, start, n);
            };
            sum = next;
        }
        match Value::integer(sum) {
            Ok(value) => Ok(value),
            Err(_) => self.numeric_primitive(p, start, n),
        }
    }
    pub(super) fn numeric_primitive(
        &mut self,
        p: Primitive,
        start: usize,
        n: usize,
    ) -> Result<Value> {
        use Primitive::*;
        if matches!(p, Abs | Inc | Dec) && n != 1 || matches!(p, Div | Less | Equal) && n == 0 {
            return Err("wrong primitive arity".into());
        }
        if matches!(p, Less | Equal) {
            for pair in self.stack[start..start + n].windows(2) {
                if self.numeric_cmp(pair[0], pair[1])?
                    != Some(if p == Less {
                        Ordering::Less
                    } else {
                        Ordering::Equal
                    })
                {
                    return Ok(Value::NIL);
                }
            }
            return Ok(Value::TRUE);
        }
        if n == 0 {
            return Value::integer(if p == Mul { 1 } else { 0 });
        }
        let first = self.coerce_number(self.stack[start], p != Abs)?;
        if n == 1 && matches!(p, Add | Mul) {
            return Ok(first);
        }
        if p == Abs
            && match self.object_if(first) {
                Some(data::Object::Bignum(n)) => n.sign() != Sign::Minus,
                Some(data::Object::Float(n)) => !n.is_sign_negative(),
                _ => first.integer_value().is_some_and(|n| n >= 0),
            }
        {
            return Ok(first);
        }
        let mut accumulator = self.arithmetic_number(first, false)?;
        if p == Div
            && self.stack[start + 2.min(n)..start + n]
                .iter()
                .any(|v| matches!(self.object_if(*v), Some(data::Object::Float(_))))
        {
            accumulator = Number::Float(accumulator.float());
        }
        if matches!(p, Abs | Inc | Dec) || n == 1 {
            accumulator = match accumulator {
                Number::Integer(x) => Number::Integer(match p {
                    Abs => {
                        if x.sign() == Sign::Minus {
                            -x
                        } else {
                            x
                        }
                    }
                    Inc => x + 1,
                    Dec => x - 1,
                    Sub => -x,
                    Div if x.sign() == Sign::NoSign => return Err("arithmetic error".into()),
                    Div => BigInt::from(1) / x,
                    _ => unreachable!(),
                }),
                Number::Float(x) => Number::Float(match p {
                    Abs => x.abs(),
                    Inc => x + 1.0,
                    Dec => x - 1.0,
                    Sub => -x,
                    Div => 1.0 / x,
                    _ => unreachable!(),
                }),
            };
        } else {
            for index in start + 1..start + n {
                let next = self.arithmetic_number(self.stack[index], true)?;
                accumulator = match (accumulator, next) {
                    (Number::Integer(x), Number::Integer(y)) => Number::Integer(match p {
                        Add => x + y,
                        Sub => x - y,
                        Mul => x * y,
                        Div if y.sign() == Sign::NoSign => return Err("arithmetic error".into()),
                        Div => x / y,
                        _ => unreachable!(),
                    }),
                    (x, y) => {
                        let (x, y) = (x.float(), y.float());
                        Number::Float(match p {
                            Add => x + y,
                            Sub => x - y,
                            Mul => x * y,
                            Div => x / y,
                            _ => unreachable!(),
                        })
                    }
                };
            }
        }
        Ok(match accumulator {
            Number::Integer(n) => self.bigint(n),
            Number::Float(n) => self.allocate_object(data::Object::Float(n)),
        })
    }
}
