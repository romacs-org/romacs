//! GNU list-walking and signal-data semantics for the bulk-verified core
//! primitives (`tools/primh/bulk/core.corpus.json`, `tests/bulk_core.rs`).
//!
//! The legacy primitives return text errors and `data.rs` maps those to a
//! condition on the cold path by re-deriving GNU's data from the arguments
//! still on the stack. Nothing here retains a `Value` past the call: every
//! function reads the argument registers and returns.
//!
//! These primitives stay on the legacy enum ONLY because the threaded engine
//! keys its compiled fast paths on `Primitive::{Car, Cdr, Cons, Eq, Null, Memq,
//! Nth, Length}` (threaded.rs `Op::Direct`) and the generated `defsubr!` path is
//! uncached (PRIMITIVE-DOCTRINE U4, unmeasured). `nreverse` stays because the
//! image contract needs GNU's read-only error behind a raw VM error and a
//! generated `Fault` can only become a Lisp condition. `substring` has neither
//! constraint and lives in `prims/fns_core.rs`.
use super::*;

/// Where GNU `FOR_EACH_TAIL` (lisp.h) stops on LIST.
pub(crate) enum TailEnd {
    /// A proper list of this many conses.
    Proper(usize),
    /// The non-nil, non-cons terminator; GNU `CHECK_LIST_END (tail, tail)`.
    Dotted(Value),
    /// The tail at which GNU's Brent check fired: `circular_list (tail)`.
    Circular(Value),
}

/// Legacy primitives whose arity failure reports GNU's
/// `(wrong-number-of-arguments #<subr NAME> COUNT)` (eval.c funcall_subr).
pub(crate) fn reports_subr_arity(p: Primitive) -> bool {
    use Primitive::*;
    matches!(
        p,
        Car | Cdr | Cons | Length | Nreverse | Null | Stringp | Eq | Memq | Nth | DeepEqual | FormatModeLine
    )
}

impl Vm {
    /// GNU lisp.h `FOR_EACH_TAIL_INTERNAL` at the pin (`FOR_EACH_TAIL_STEP_CYCLEP`,
    /// lisp.h:5899): step, compare with the tortoise, count; the tortoise
    /// starts at LIST and teleports to the current tail only when the count
    /// is a power of two that is a multiple of 4096. Lisp observes which tail
    /// a `circular-list' signal names, so this is reproduced, not approximated.
    pub(crate) fn gnu_tails(&self, list: Value) -> TailEnd {
        let mut tail = list;
        let mut tortoise = list;
        let mut l: u64 = 0;
        let mut count = 0usize;
        while tail.tag() == 3 {
            let Ok((_, cdr)) = self.cons_pair(tail) else { break };
            count += 1;
            tail = cdr;
            if tail == tortoise {
                return TailEnd::Circular(tail);
            }
            l += 1;
            if l & 4095 == 0 && l.is_power_of_two() {
                tortoise = tail;
            }
        }
        if tail == Value::NIL { TailEnd::Proper(count) } else { TailEnd::Dotted(tail) }
    }

    fn is_array(&self, v: Value) -> bool {
        self.str_view(v).is_some() || matches!(self.object_if(v), Some(data::Object::Vector(_) | data::Object::BoolVector(_)))
    }

    /// GNU's condition for a failed call of a bulk-verified core primitive,
    /// recomputed from the arguments still in `stack[start..start + argc]`.
    /// `None` leaves the call to the shared legacy mapping.
    pub(super) fn core_condition(
        &mut self,
        p: Primitive,
        start: usize,
        argc: usize,
        error: &str,
    ) -> Option<(&'static str, Vec<Value>)> {
        use Primitive::*;
        let arg = |vm: &Vm, i: usize| if i < argc { vm.stack[start + i] } else { Value::NIL };
        if error == "wrong primitive arity" && reports_subr_arity(p) {
            return Some((
                "wrong-number-of-arguments",
                vec![prim::legacy_value(p)?, Value::integer(argc as i64).ok()?],
            ));
        }
        let wrong = |vm: &mut Vm, predicate: &str, v: Value| {
            let slot = vm.slot(predicate);
            ("wrong-type-argument", vec![Value::reference(slot + 1, 7), v])
        };
        match p {
            // fns.c Flength / list_length.
            Length => {
                let x = arg(self, 0);
                Some(match self.gnu_tails(x) {
                    TailEnd::Dotted(t) if x.tag() == 3 => wrong(self, "listp", t),
                    TailEnd::Circular(t) => ("circular-list", vec![t]),
                    _ => wrong(self, "sequencep", x),
                })
            }
            // fns.c Fnreverse: SEQ itself (already partly reversed) is the datum.
            // The image refusal is VM2's own contract, not a GNU condition:
            // leave it to the generic path rather than invent a type error.
            Nreverse if error == crate::frozen::READ_ONLY => None,
            Nreverse => {
                let x = arg(self, 0);
                Some(if error.contains("circular") {
                    ("circular-list", vec![x])
                } else if x.tag() == 3 {
                    wrong(self, "listp", x)
                } else {
                    wrong(self, "arrayp", x)
                })
            }
            // fns.c Fmemq: FOR_EACH_TAIL then CHECK_LIST_END (list, list).
            Memq if error.contains("circular") => match self.gnu_tails(arg(self, 1)) {
                TailEnd::Circular(t) => Some(("circular-list", vec![t])),
                _ => None,
            },
            // fns.c concat_to_list: each argument but the last, in order.
            Append => {
                for i in 0..argc.saturating_sub(1) {
                    let v = arg(self, i);
                    if v == Value::NIL || self.is_array(v) {
                        continue;
                    }
                    if v.tag() != 3 {
                        return Some(wrong(self, "sequencep", v));
                    }
                    // concat_to_list copies the car, then walks FOR_EACH_TAIL
                    // from the cdr: that is where its tortoise starts.
                    let rest = self.cons_pair(v).map_or(Value::NIL, |(_, cdr)| cdr);
                    match self.gnu_tails(rest) {
                        TailEnd::Proper(_) => continue,
                        TailEnd::Dotted(t) => return Some(wrong(self, "listp", t)),
                        TailEnd::Circular(t) => return Some(("circular-list", vec![t])),
                    }
                }
                None
            }
            _ => None,
        }
    }

    pub(super) fn core_nreverse(&mut self, seq: Value) -> Result<Value> {
        if seq == Value::NIL {
            return Ok(seq);
        }
        if seq.tag() == 3 {
            // VM2 contract (tests/gc.rs, tests/image_startup.rs): an image
            // cons is never written and the refusal precedes every write.
            // Image edges reach only image objects, so the private prefix ends
            // at the first image cons; Brent's walk finds it, a private end or
            // a private cycle in bounded steps without mutating.
            let (mut slow, mut fast, mut power, mut lam) = (seq, seq, 1usize, 0usize);
            while fast.tag() == 3 {
                if Self::image_value(fast) {
                    return Err(self.refuse_read_only(seq));
                }
                fast = self.conses[fast.index(3)?].1;
                lam += 1;
                if fast == slow {
                    break;
                }
                if lam == power {
                    (slow, power, lam) = (fast, power * 2, 0);
                }
            }
            let mut prev = Value::NIL;
            let mut tail = seq;
            while tail.tag() == 3 {
                let i = tail.index(3)?;
                let next = self.conses[i].1;
                // "If SEQ contains a cycle, attempting to reverse it in-place
                // will inevitably come back to SEQ."
                if next == seq {
                    return Err("circular list".into());
                }
                self.conses.set_cdr(i, prev);
                prev = tail;
                tail = next;
            }
            if tail != Value::NIL {
                return Err("list required".into());
            }
            return Ok(prev);
        }
        if let Some(text) = self.str_view(seq) {
            // Freverse: a fresh string; unibyte exactly when chars == bytes.
            let unibyte = text.len() == text.bytes().len();
            let multibyte = !unibyte && text.is_multibyte();
            let bytes = if unibyte || !text.is_multibyte() {
                text.bytes().iter().rev().copied().collect::<Vec<u8>>()
            } else {
                let mut out = Vec::with_capacity(text.bytes().len());
                let chars: Vec<u32> = text.chars().collect();
                let mut buf = [0u8; 8];
                for c in chars.into_iter().rev() {
                    let n = crate::lstring::char_string(c, &mut buf);
                    out.extend_from_slice(&buf[..n]);
                }
                out
            };
            return Ok(self.make_lisp_string(bytes, multibyte));
        }
        if Self::image_value(seq) {
            return Err(self.refuse_read_only(seq));
        }
        let index = seq.index_if(0).and_then(|i| i.checked_sub(1));
        if let Some(data::Object::Vector(v)) = index.and_then(|i| self.objects.get_entry_mut(i)) {
            v.reverse();
            return Ok(seq);
        }
        if let Some(data::Object::BoolVector(bits)) = index.and_then(|i| self.objects.get_entry_mut(i)) {
            for i in 0..bits.len / 2 {
                let j = bits.len - 1 - i;
                let left = bits.get(i).unwrap();
                let right = bits.get(j).unwrap();
                bits.set(i, right);
                bits.set(j, left);
            }
            return Ok(seq);
        }
        Err("array required".into())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Tag-6 payloads are BOTH an index into `PRIMITIVES` and, at many sites,
    /// `Primitive::X as usize` (threaded.rs `Primitive::Apply as usize`, lib.rs
    /// `Primitive::Null as usize`, ...). A row removed or reordered without its
    /// variant silently re-targets every later primitive: removing the `car`
    /// row alone made the next call site recurse until the native stack died.
    #[test]
    fn legacy_rows_are_aligned_with_their_enum_ordinals() {
        for (index, (name, p)) in PRIMITIVES.iter().enumerate() {
            assert_eq!(*p as usize, index, "PRIMITIVES row {index} ({name}) is not variant ordinal {index}");
        }
    }

    /// The loop above cannot see a variant with NO row: a trailing variant
    /// appended to `enum Primitive` without its `PRIMITIVES` entry leaves every
    /// row aligned, yet `legacy_value` of it finds nothing and
    /// `Value::reference(Variant as usize, 6)` indexes past the table (into
    /// GENERATED). Read the declaration itself and require one row per
    /// variant, in declaration order.
    #[test]
    fn every_legacy_variant_has_its_row() {
        let source = include_str!("lib.rs");
        let start = source.find("\nenum Primitive {\n").expect("enum Primitive declaration") + "\nenum Primitive {\n".len();
        let body = &source[start..start + source[start..].find("\n}\n").expect("end of enum Primitive")];
        let variants: Vec<&str> = body
            .lines()
            .map(str::trim)
            .filter(|line| !line.is_empty() && !line.starts_with("//") && !line.starts_with("#["))
            .map(|line| line.strip_suffix(',').unwrap_or_else(|| panic!("unparsed enum Primitive line {line:?}")))
            .collect();
        assert!(variants.len() > 200, "parsed only {} variants", variants.len());
        assert_eq!(
            variants.len(),
            PRIMITIVES.len(),
            "enum Primitive has {} variants but PRIMITIVES has {} rows; last variant {:?}",
            variants.len(),
            PRIMITIVES.len(),
            variants.last()
        );
        for (index, ((name, p), variant)) in PRIMITIVES.iter().zip(&variants).enumerate() {
            assert_eq!(format!("{p:?}"), *variant, "PRIMITIVES row {index} ({name}) is not declared variant {index}");
        }
    }
}
