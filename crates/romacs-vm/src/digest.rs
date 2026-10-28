//! The digests of fns.c `secure-hash': MD5, SHA-1 and SHA-2 from their
//! crates, and SHA-3 (FIPS 202) over Keccak-f[1600] here, as gnulib's
//! sha3.c computes it.

const RC: [u64; 24] = [
    0x0000000000000001, 0x0000000000008082, 0x800000000000808a, 0x8000000080008000,
    0x000000000000808b, 0x0000000080000001, 0x8000000080008081, 0x8000000000008009,
    0x000000000000008a, 0x0000000000000088, 0x0000000080008009, 0x000000008000000a,
    0x000000008000808b, 0x800000000000008b, 0x8000000000008089, 0x8000000000008003,
    0x8000000000008002, 0x8000000000000080, 0x000000000000800a, 0x800000008000000a,
    0x8000000080008081, 0x8000000000008080, 0x0000000080000001, 0x8000000080008008,
];
/// The rho rotations along the pi lane order.
const RHO: [u32; 24] = [1, 3, 6, 10, 15, 21, 28, 36, 45, 55, 2, 14, 27, 41, 56, 8, 25, 43, 62, 18, 39, 61, 20, 44];
const PI: [usize; 24] = [10, 7, 11, 17, 18, 3, 5, 16, 8, 21, 24, 4, 15, 23, 19, 13, 12, 2, 20, 14, 22, 9, 6, 1];

fn keccak_f(a: &mut [u64; 25]) {
    for rc in RC {
        let c: [u64; 5] = std::array::from_fn(|x| a[x] ^ a[x + 5] ^ a[x + 10] ^ a[x + 15] ^ a[x + 20]);
        for x in 0..5 {
            let d = c[(x + 4) % 5] ^ c[(x + 1) % 5].rotate_left(1);
            for y in 0..5 { a[x + 5 * y] ^= d; }
        }
        let mut last = a[1];
        for (&lane, &rotation) in PI.iter().zip(RHO.iter()) {
            let next = a[lane];
            a[lane] = last.rotate_left(rotation);
            last = next;
        }
        for y in 0..5 {
            let row: [u64; 5] = std::array::from_fn(|x| a[5 * y + x]);
            for x in 0..5 { a[5 * y + x] = row[x] ^ (!row[(x + 1) % 5] & row[(x + 2) % 5]); }
        }
        a[0] ^= rc;
    }
}

/// SHA3 with an OUT-byte digest: rate 200 - 2 OUT, padding 0x06 ... 0x80.
fn sha3(bytes: &[u8], out: usize) -> Vec<u8> {
    let rate = 200 - 2 * out;
    let mut state = [0u64; 25];
    let absorb = |state: &mut [u64; 25], block: &[u8]| {
        for (word, lane) in block.chunks_exact(8).zip(state.iter_mut()) {
            *lane ^= u64::from_le_bytes(word.try_into().expect("eight bytes"));
        }
        keccak_f(state);
    };
    let blocks = bytes.chunks_exact(rate);
    let rest = blocks.remainder();
    for block in blocks { absorb(&mut state, block); }
    let mut last = vec![0u8; rate];
    last[..rest.len()].copy_from_slice(rest);
    last[rest.len()] ^= 0x06;
    last[rate - 1] ^= 0x80;
    absorb(&mut state, &last);
    state.iter().flat_map(|lane| lane.to_le_bytes()).take(out).collect()
}

/// The digest of BYTES under ALGORITHM, a `secure-hash-algorithms' name.
pub(crate) fn digest(algorithm: &str, bytes: &[u8]) -> Option<Vec<u8>> {
    use sha2::Digest as _;
    Some(match algorithm {
        "md5" => md5::Md5::digest(bytes).to_vec(),
        "sha1" => sha1::Sha1::digest(bytes).to_vec(),
        "sha224" => sha2::Sha224::digest(bytes).to_vec(),
        "sha256" => sha2::Sha256::digest(bytes).to_vec(),
        "sha384" => sha2::Sha384::digest(bytes).to_vec(),
        "sha512" => sha2::Sha512::digest(bytes).to_vec(),
        "sha3-224" => sha3(bytes, 28),
        "sha3-256" => sha3(bytes, 32),
        "sha3-384" => sha3(bytes, 48),
        "sha3-512" => sha3(bytes, 64),
        _ => return None,
    })
}
