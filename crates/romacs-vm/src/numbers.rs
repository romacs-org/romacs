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
    // gnulib dtoastr starts at DBL_DIG significant digits (1 below DBL_MIN,
    // where subnormals have fewer), increasing until the decimal
    // round-trips. %g's fixed/scientific threshold follows that precision.
    let first = if n.abs() < f64::MIN_POSITIVE { 1 } else { 15 };
    for precision in first..=17 {
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

/// I against F, exactly (data.c arithcompare): F meets the double nearest I,
/// and a tie is settled between the integers, as that double may not be I.
/// None for a NaN.
fn fixnum_float_cmp(i: i64, f: f64) -> Option<Ordering> {
    match (i as f64).partial_cmp(&f)? {
        Ordering::Equal => Some(i.cmp(&(f as i64))),
        order => Some(order),
    }
}

pub(super) fn integer_float(n: &BigInt) -> f64 {
    n.to_string().parse().expect("decimal integer is a float")
}

enum Number {
    Integer(BigInt),
    Float(f64),
}

/// A number the byte-op kernels compute on in place. A bignum or a marker
/// is neither, and keeps its primitive.
#[derive(Clone, Copy)]
enum Quick {
    Fixnum(i64),
    Float(f64),
}

/// The operations of `Vm::float_kernel', which is how a kernel names itself.
pub(super) mod op {
    pub const ADD: u8 = 0;
    pub const SUB: u8 = 1;
    pub const MUL: u8 = 2;
    pub const DIV: u8 = 3;
    pub const LESS: u8 = 4;
    pub const GREATER: u8 = 5;
    pub const LESS_EQUAL: u8 = 6;
    pub const GREATER_EQUAL: u8 = 7;
    pub const EQUAL: u8 = 8;
}
impl Number {
    fn float(&self) -> f64 {
        match self {
            Self::Integer(n) => integer_float(n),
            Self::Float(n) => *n,
        }
    }
}

/// A float byte op. GNU's C compiler puts the later operand first in a sum
/// and a product, so of two NaNs it answers with that one, sign included.
/// Writing `b + a' does not reach the machine: LLVM may commute it.
fn float_arithmetic(kernel: u8, a: f64, b: f64) -> f64 {
    match kernel {
        op::ADD | op::MUL if a.is_nan() && b.is_nan() => b,
        op::ADD => a + b,
        op::SUB => a - b,
        op::MUL => a * b,
        _ => a / b,
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
    #[inline]
    fn quick_number(&self, v: Value) -> Option<Quick> {
        match v.integer_value() {
            Some(n) => Some(Quick::Fixnum(n)),
            None => match self.object_if(v) {
                Some(data::Object::Float(f)) => Some(Quick::Float(*f)),
                _ => None,
            },
        }
    }
    /// X against Y when both are `Quick': data.c arithcompare with no bignum
    /// built. The inner None is a NaN.
    pub(super) fn quick_cmp(&self, x: Value, y: Value) -> Option<Option<Ordering>> {
        Some(match (self.quick_number(x)?, self.quick_number(y)?) {
            (Quick::Fixnum(a), Quick::Fixnum(b)) => Some(a.cmp(&b)),
            (Quick::Float(a), Quick::Float(b)) => a.partial_cmp(&b),
            (Quick::Fixnum(a), Quick::Float(b)) => fixnum_float_cmp(a, b),
            (Quick::Float(a), Quick::Fixnum(b)) => fixnum_float_cmp(b, a).map(Ordering::reverse),
        })
    }
    /// The byte op OP on X and Y, one of them not a fixnum. UNBOUND when
    /// either is not `Quick', or for two fixnums (an overflow or a zero
    /// divisor), which the primitive answers.
    #[inline(never)]
    pub(super) fn float_kernel(&mut self, kernel: u8, x: Value, y: Value) -> Value {
        if kernel >= op::LESS {
            let Some(order) = self.quick_cmp(x, y) else { return Value::UNBOUND };
            let holds = match kernel {
                op::LESS => order == Some(Ordering::Less),
                op::GREATER => order == Some(Ordering::Greater),
                op::LESS_EQUAL => matches!(order, Some(Ordering::Less | Ordering::Equal)),
                op::GREATER_EQUAL => matches!(order, Some(Ordering::Greater | Ordering::Equal)),
                _ => order == Some(Ordering::Equal),
            };
            return if holds { Value::TRUE } else { Value::NIL };
        }
        let (a, b) = match (self.quick_number(x), self.quick_number(y)) {
            (Some(Quick::Float(a)), Some(Quick::Float(b))) => (a, b),
            (Some(Quick::Float(a)), Some(Quick::Fixnum(b))) => (a, b as f64),
            (Some(Quick::Fixnum(a)), Some(Quick::Float(b))) => (a as f64, b),
            _ => return Value::UNBOUND,
        };
        self.allocate_object(data::Object::Float(float_arithmetic(kernel, a, b)))
    }
    /// GNU Frem: integers or markers, of any size.
    pub(super) fn remainder(&mut self, x: Value, y: Value) -> Result<Value> {
        let (x, y) = (self.integer_operand(x)?, self.integer_operand(y)?);
        if y.sign() == Sign::NoSign {
            return Err("arithmetic error".into());
        }
        Ok(self.bigint(x % y))
    }
    fn integer_operand(&self, v: Value) -> Result<BigInt> {
        let v = if matches!(self.object_if(v), Some(data::Object::Marker(_))) { self.coerce_number(v, true)? } else { v };
        self.bigint_value(v).ok_or_else(|| "integer or marker required".into())
    }
    pub(super) fn integer_operand_ok(&self, v: Value) -> bool {
        self.integer_operand(v).is_ok()
    }
    /// V, a marker read as its position (GNU coerce_marker).
    fn coerce_marker(&self, v: Value) -> Result<Value> {
        if matches!(self.object_if(v), Some(data::Object::Marker(_))) { self.coerce_number(v, true) } else { Ok(v) }
    }
    pub(super) fn numeric_cmp(&self, x: Value, y: Value) -> Result<Option<Ordering>> {
        if let Some(order) = self.quick_cmp(x, y) {
            return Ok(order);
        }
        // data.c arithcompare coerces both before it type-checks either.
        let (x, y) = (self.coerce_marker(x)?, self.coerce_marker(y)?);
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
                        let kernel = match p {
                            Add => op::ADD,
                            Sub => op::SUB,
                            Mul => op::MUL,
                            Div => op::DIV,
                            _ => unreachable!(),
                        };
                        Number::Float(float_arithmetic(kernel, x.float(), y.float()))
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
