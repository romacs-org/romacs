use crate::Result;
use num_bigint::{BigInt, Sign};
use std::{iter::{Copied, Peekable}, slice::Iter};

type Codes<'a> = Peekable<Copied<Iter<'a, u32>>>;

#[derive(Clone, Default)]
pub(super) struct FormatSpec {
    pub argument: Option<usize>,
    pub conversion: char,
    left: bool,
    plus: bool,
    space: bool,
    alternate: bool,
    zero: bool,
    width: usize,
    precision: Option<usize>,
}

fn digits(chars: &mut Codes<'_>) -> Result<usize> {
    let mut number = 0usize;
    while let Some(code @ 48..=57) = chars.peek().copied() {
        chars.next();
        number = number.checked_mul(10).and_then(|n| n.checked_add((code - 48) as usize))
            .filter(|&n| n <= 1 << 20).ok_or("format width or precision exceeds supported limit")?;
    }
    Ok(number)
}

impl FormatSpec {
    pub fn parse(chars: &mut Codes<'_>) -> Result<Self> {
        let mut result = Self::default();
        let mut field = chars.clone();
        let argument = digits(&mut field)?;
        if field.next_if_eq(&36).is_some() {
            result.argument = Some(argument.checked_sub(1).ok_or("Invalid format field number")?);
            *chars = field;
        }
        loop {
            match chars.peek() {
                Some(45) => result.left = true,
                Some(43) => result.plus = true,
                Some(32) => result.space = true,
                Some(35) => result.alternate = true,
                Some(48) => result.zero = true,
                _ => break,
            }
            chars.next();
        }
        result.width = digits(chars)?;
        if chars.next_if_eq(&46).is_some() { result.precision = Some(digits(chars)?); }
        result.conversion = chars.next().and_then(char::from_u32).ok_or("incomplete format specifier")?;
        if !matches!(result.conversion, '%' | 's' | 'S' | 'c' | 'd' | 'i' | 'b' | 'B' | 'o' | 'x' | 'X' | 'e' | 'f' | 'g') {
            return Err("unsupported format specifier".into());
        }
        Ok(result)
    }

    fn sign(&self, negative: bool) -> &'static str {
        if negative { "-" } else if self.plus { "+" } else if self.space { " " } else { "" }
    }

    fn pad_number(&self, prefix: &str, body: &str, integer: bool) -> String {
        let padding = self.width.saturating_sub(prefix.len() + body.len());
        let zeros = self.zero && !self.left && (!integer || self.precision.is_none())
            && body.as_bytes().first().is_some_and(u8::is_ascii_hexdigit);
        if self.left { format!("{prefix}{body}{}", " ".repeat(padding)) }
        else if zeros { format!("{prefix}{}{body}", "0".repeat(padding)) }
        else { format!("{}{prefix}{body}", " ".repeat(padding)) }
    }

    pub fn integer(&self, number: &BigInt) -> String {
        let radix = match self.conversion { 'b' | 'B' => 2, 'o' => 8, 'x' | 'X' => 16, _ => 10 };
        let nonzero = number.sign() != Sign::NoSign;
        let raw = number.to_str_radix(radix);
        let raw = raw.strip_prefix('-').unwrap_or(&raw);
        let mut body = if self.precision == Some(0) && !nonzero { String::new() } else { raw.to_owned() };
        let zeros = self.precision.unwrap_or(0).saturating_sub(body.len());
        body.insert_str(0, &"0".repeat(zeros));
        if self.conversion == 'X' { body.make_ascii_uppercase(); }
        let mut prefix = self.sign(number.sign() == Sign::Minus).to_owned();
        if self.alternate {
            match self.conversion {
                'b' | 'B' | 'x' | 'X' if nonzero => { prefix.push('0'); prefix.push(self.conversion); }
                'o' if !body.starts_with('0') => body.insert(0, '0'),
                _ => {}
            }
        }
        self.pad_number(&prefix, &body, true)
    }

    pub fn truncated_float(&self, number: &BigInt) -> String {
        let mut spec = self.clone();
        if matches!(self.conversion, 'd' | 'i') && number.sign() == Sign::NoSign && self.precision == Some(0) {
            spec.precision = Some(1);
        }
        spec.integer(number)
    }

    pub fn float(&self, number: f64) -> String {
        let precision = self.precision.unwrap_or(6);
        let body = if number.is_nan() { "nan".into() }
            else if number.is_infinite() { "inf".into() }
            else {
                let magnitude = number.abs();
                match self.conversion {
                    'f' => decimal_point(format!("{magnitude:.precision$}"), self.alternate),
                    'e' => self.scientific(&format!("{magnitude:.precision$e}")),
                    'g' => {
                        let precision = precision.max(1);
                        let original = format!("{magnitude:e}").split_once('e').unwrap().1.parse::<i32>().unwrap();
                        self.general(&format!("{magnitude:.*e}", precision - 1), precision, original)
                    }
                    _ => unreachable!(),
                }
            };
        self.pad_number(self.sign(number.is_sign_negative()), &body, false)
    }

    pub fn integer_float(&self, number: &BigInt) -> String {
        let raw = number.to_str_radix(10);
        let negative = number.sign() == Sign::Minus;
        let digits = raw.strip_prefix('-').unwrap_or(&raw);
        let precision = self.precision.unwrap_or(6);
        let body = if self.conversion == 'f' {
            let point = if precision != 0 || self.alternate { "." } else { "" };
            format!("{digits}{point}{}", "0".repeat(precision))
        } else {
            let significant = if self.conversion == 'g' { precision.max(1) } else { precision + 1 };
            let original = digits.len() as i32 - 1;
            let (digits, exponent) = rounded_integer(digits, significant);
            let raw = format!("{}.{}e{exponent}", &digits[..1], &digits[1..]);
            if self.conversion == 'g' { self.general(&raw, significant, original) } else { self.scientific(&raw) }
        };
        self.pad_number(self.sign(negative), &body, false)
    }

    fn scientific(&self, raw: &str) -> String {
        let (mantissa, exponent) = raw.split_once('e').unwrap();
        let exponent = exponent.parse::<i32>().unwrap();
        let mantissa = decimal_point(mantissa.to_owned(), self.alternate);
        format!("{mantissa}e{exponent:+03}")
    }

    fn general(&self, raw: &str, precision: usize, original: i32) -> String {
        let (mantissa, exponent) = raw.split_once('e').unwrap();
        let exponent = exponent.parse::<i32>().unwrap();
        if exponent < -4 || exponent >= precision as i32 {
            let carry_boundary = exponent == precision as i32 && original + 1 == exponent;
            let mantissa = if self.alternate && !carry_boundary { decimal_point(mantissa.to_owned(), true) }
                else if self.alternate { decimal_point(mantissa.trim_end_matches('0').trim_end_matches('.').to_owned(), true) }
                else { mantissa.trim_end_matches('0').trim_end_matches('.').to_owned() };
            return format!("{mantissa}e{exponent:+03}");
        }
        let digits = mantissa.replace('.', "");
        let point = exponent + 1;
        let mut fixed = if point <= 0 {
            format!("0.{}{digits}", "0".repeat((-point) as usize))
        } else if point as usize >= digits.len() {
            format!("{digits}{}", "0".repeat(point as usize - digits.len()))
        } else {
            format!("{}.{}", &digits[..point as usize], &digits[point as usize..])
        };
        if !self.alternate && fixed.contains('.') { fixed = fixed.trim_end_matches('0').trim_end_matches('.').to_owned(); }
        decimal_point(fixed, self.alternate)
    }

    pub(crate) fn text(&self, text: Vec<u8>, multibyte: bool, rule: Option<&crate::width_rule::WidthRule>) -> Vec<u8> {
        self.text_field(text, multibyte, rule).0
    }

    /// Whether a field width or precision makes the text's columns count:
    /// only then is a width rule needed.
    pub(crate) fn counts_columns(&self) -> bool { self.width > 0 || self.precision.is_some() }

    /// `text', and how many bytes of padding come before the text: where
    /// GNU's field starts for the text's properties (editfns.c:3787, 3797).
    /// RULE counts the columns (lisp_string_width), given when
    /// `counts_columns'.
    pub(crate) fn text_field(&self, mut text: Vec<u8>, multibyte: bool, rule: Option<&crate::width_rule::WidthRule>) -> (Vec<u8>, usize) {
        let Some(rule) = rule.filter(|_| self.counts_columns()) else { return (text, 0) };
        if self.precision == Some(0) { text.clear(); }
        let mut at = 0;
        let mut columns = 0;
        while at < text.len() {
            let (code, bytes) = if multibyte { crate::lstring::string_char(&text[at..]) } else { (u32::from(text[at]), 1) };
            let width = rule.width(code);
            if self.precision.is_some_and(|limit| columns + width > limit) { break; }
            columns += width;
            at += bytes;
        }
        text.truncate(at);
        let leading = if self.left { 0 } else { self.width.saturating_sub(columns) };
        (self.pad_text(text, columns), leading)
    }

    pub(crate) fn character(&self, text: Vec<u8>, code: u32, rule: Option<&crate::width_rule::WidthRule>) -> Vec<u8> {
        if code >= 128 { return self.text(text, true, rule); }
        let text = if self.precision == Some(0) { Vec::new() } else { text };
        let columns = text.len();
        self.pad_text(text, columns)
    }

    fn pad_text(&self, mut text: Vec<u8>, columns: usize) -> Vec<u8> {
        let padding = self.width.saturating_sub(columns);
        if self.left { text.extend(std::iter::repeat_n(b' ', padding)); }
        else { text.splice(0..0, std::iter::repeat_n(b' ', padding)); }
        text
    }
}

fn decimal_point(mut number: String, alternate: bool) -> String {
    if alternate && !number.contains('.') { number.push('.'); }
    number
}

fn rounded_integer(raw: &str, precision: usize) -> (String, i32) {
    let mut exponent = raw.len() as i32 - 1;
    let mut digits = raw.as_bytes()[..raw.len().min(precision)].to_vec();
    if raw.len() > precision {
        let tail = &raw.as_bytes()[precision..];
        let round = tail[0] > b'5' || (tail[0] == b'5'
            && (tail[1..].iter().any(|&c| c != b'0') || (digits[precision - 1] - b'0') % 2 != 0));
        if round {
            let mut at = digits.len();
            while at != 0 && digits[at - 1] == b'9' { digits[at - 1] = b'0'; at -= 1; }
            if at == 0 { digits[0] = b'1'; exponent += 1; }
            else { digits[at - 1] += 1; }
        }
    }
    digits.resize(precision, b'0');
    (String::from_utf8(digits).unwrap(), exponent)
}
