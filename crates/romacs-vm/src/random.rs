//! GNU's pseudo-random integers (fns.c `random', sysdep.c, bignum.c) over
//! glibc's `srandom'/`random' TYPE_3 generator, reproduced exactly so a
//! seeded sequence equals GNU's on glibc hosts.

/// glibc random_r.c TYPE_3: degree 31, separation 3.
const DEGREE: usize = 31;
const SEPARATION: usize = 3;
/// random () yields 31 bits (sysdep.c RAND_BITS).
const RAND_BITS: u32 = 31;
/// lisp.h on a 64-bit build: 62 fixnum bits, INTMASK = 2^62 - 1.
const FIXNUM_BITS: u32 = 62;
const INTMASK: u64 = (1 << FIXNUM_BITS) - 1;

pub(crate) struct Random {
    state: [i32; DEGREE],
    front: usize,
    rear: usize,
}

impl Random {
    /// srandom (SEED).
    pub(crate) fn seeded(seed: u32) -> Self {
        let seed = if seed == 0 { 1 } else { seed };
        let mut state = [0i32; DEGREE];
        state[0] = seed as i32;
        let mut word = seed as i32;
        for slot in state.iter_mut().skip(1) {
            // state[i] = 16807 * state[i - 1] % 2147483647, without overflow.
            let (hi, lo) = (i64::from(word) / 127_773, i64::from(word) % 127_773);
            let mut next = 16_807 * lo - 2_836 * hi;
            if next < 0 { next += 2_147_483_647; }
            word = next as i32;
            *slot = word;
        }
        let mut random = Random { state, front: SEPARATION, rear: 0 };
        for _ in 0..DEGREE * 10 { random.next(); }
        random
    }
    /// init_random: a seed from the system's entropy, else the time and pid.
    pub(crate) fn from_entropy() -> Self {
        use std::io::Read;
        let mut bytes = [0u8; 4];
        let seed = match std::fs::File::open("/dev/urandom").and_then(|mut f| f.read_exact(&mut bytes)) {
            Ok(()) => u32::from_ne_bytes(bytes),
            Err(_) => {
                let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap_or_default();
                (now.as_secs() as u32) ^ now.subsec_nanos() ^ std::process::id()
            }
        };
        Random::seeded(seed)
    }
    /// seed_random: SEED's bytes folded into an `unsigned int' by xor.
    pub(crate) fn seed_bytes(&mut self, seed: &[u8]) {
        let mut arg = [0u8; 4];
        for (i, &byte) in seed.iter().enumerate() { arg[i % 4] ^= byte; }
        *self = Random::seeded(u32::from_ne_bytes(arg));
    }
    /// random (): 31 bits.
    fn next(&mut self) -> u64 {
        let value = (self.state[self.front] as u32).wrapping_add(self.state[self.rear] as u32);
        self.state[self.front] = value as i32;
        self.front += 1;
        if self.front >= DEGREE {
            self.front = 0;
            self.rear += 1;
        } else {
            self.rear += 1;
            if self.rear >= DEGREE { self.rear = 0; }
        }
        u64::from(value >> 1)
    }
    /// get_random: FIXNUM_BITS random bits.
    fn bits(&mut self) -> u64 {
        let mut value = 0u64;
        for _ in 0..(FIXNUM_BITS + RAND_BITS - 1) / RAND_BITS {
            value = self.next() ^ (value << RAND_BITS) ^ (value >> (64 - RAND_BITS));
        }
        value ^= value >> (64 - FIXNUM_BITS);
        value & INTMASK
    }
    /// get_random_ulong.
    fn ulong(&mut self) -> u64 {
        let mut value = 0u64;
        for _ in 0..(64 + RAND_BITS - 1) / RAND_BITS {
            value = self.next() ^ (value << RAND_BITS) ^ (value >> (64 - RAND_BITS));
        }
        value
    }
    /// (random): every fixnum equally likely (make_ufixnum wraps the bits).
    pub(crate) fn fixnum(&mut self) -> i64 {
        let bits = self.bits();
        if bits >> (FIXNUM_BITS - 1) == 1 { bits as i64 - (1 << FIXNUM_BITS) } else { bits as i64 }
    }
    /// get_random_fixnum: [0, LIMIT) for a positive fixnum LIMIT.
    pub(crate) fn below(&mut self, limit: i64) -> i64 {
        let difflim = INTMASK as i64 - limit + 1;
        loop {
            let r = self.bits() as i64;
            let remainder = r % limit;
            if r - remainder <= difflim { return remainder; }
        }
    }
    /// get_random_limb_lim: [0, LIMIT), or any limb when LIMIT is 0.
    fn limb_below(&mut self, limit: u64) -> u64 {
        let difflim = limit.wrapping_neg();
        loop {
            let r = self.ulong();
            if limit == 0 { return r; }
            let remainder = r % limit;
            if r - remainder <= difflim { return remainder; }
        }
    }
    /// get_random_bignum: [0, LIMIT) for a positive bignum LIMIT, chosen one
    /// 64-bit limb at a time from the most significant.
    pub(crate) fn below_bignum(&mut self, limit: &num_bigint::BigUint) -> num_bigint::BigUint {
        let limbs = limit.to_u64_digits();
        let n = limbs.len();
        let high = limbs[n - 1];
        let mut result = vec![0u64; n];
        loop {
            let top = self.limb_below(high.wrapping_add(u64::from(n > 1)));
            let mut edgy = top == high;
            result[n - 1] = top;
            for i in (0..n - 1).rev() {
                let limb = self.ulong();
                if edgy {
                    if limbs[i] < limb { break; }
                    edgy = limbs[i] == limb;
                }
                result[i] = limb;
            }
            if !edgy { break; }
        }
        num_bigint::BigUint::from_slice(&result.iter().flat_map(|&l| [l as u32, (l >> 32) as u32]).collect::<Vec<_>>())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// glibc 2.44's srandom then random, recorded through ctypes.
    #[test]
    fn matches_glibc_random() {
        let mut random = Random::seeded(1);
        let got: Vec<u64> = (0..5).map(|_| random.next()).collect();
        assert_eq!(got, [1804289383, 846930886, 1681692777, 1714636915, 1957747793]);
        let mut random = Random::seeded(0xdead_beef);
        let got: Vec<u64> = (0..3).map(|_| random.next()).collect();
        assert_eq!(got, [352217057, 918588210, 499345174]);
    }
}
