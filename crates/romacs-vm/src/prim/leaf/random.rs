//! fns.c `random' over the VM's generator (`crate::random').
use super::*;

impl Leaf<'_> {
    pub(crate) fn random(&mut self, limit: Value) -> super::super::Out {
        if limit == Value::TRUE {
            self.vm.random = crate::random::Random::from_entropy();
        } else if let Some(view) = self.vm.str_view(limit) {
            let seed = view.bytes.to_vec();
            self.vm.random.seed_bytes(&seed);
        } else if let Some(n) = limit.integer_value() {
            if n <= 0 { return Err(Fault::Range1(limit)); }
            return Ok(Value::integer(self.vm.random.below(n)).map_err(Fault::Error)?);
        } else if self.kind(limit) == Kind::Bignum {
            let n = self.bigint_value(limit).expect("bignum");
            let Some(n) = n.to_biguint().filter(|n| *n > num_bigint::BigUint::ZERO) else { return Err(Fault::Range1(limit)) };
            let r = self.vm.random.below_bignum(&n);
            return Ok(self.bigint(r.into()));
        }
        Value::integer(self.vm.random.fixnum()).map_err(Fault::Error)
    }
}
