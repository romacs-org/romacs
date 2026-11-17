//! New passwords: characters drawn uniformly from an alphabet with the
//! kernel's entropy, as pass(1) draws them from /dev/urandom.  Lisp's
//! `random` is not a cryptographic source; this is.

/// A password of LENGTH characters from ALPHABET, each drawn uniformly by
/// rejection sampling, so no character is likelier than another.
pub(crate) fn password(length: usize, alphabet: &[char]) -> std::io::Result<zeroize::Zeroizing<String>> {
    if alphabet.is_empty() || alphabet.len() > 256 {
        return Err(std::io::Error::new(std::io::ErrorKind::InvalidInput, "the alphabet holds 1 to 256 characters"));
    }
    let mut source = crate::host::random_source()?;
    // The largest multiple of the alphabet's size a byte reaches; bytes at or over it are redrawn.
    let limit = 256 - 256 % alphabet.len();
    // Sized for the widest characters, so the text never moves and leaves a copy behind.
    let mut out = zeroize::Zeroizing::new(String::with_capacity(length * 4));
    let mut bytes = zeroize::Zeroizing::new([0u8; 64]);
    let mut drawn = 0;
    while drawn < length {
        source.read_exact(&mut bytes[..])?;
        for &byte in bytes.iter().filter(|&&byte| usize::from(byte) < limit).take(length - drawn) {
            out.push(alphabet[usize::from(byte) % alphabet.len()]);
            drawn += 1;
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_password_has_its_length_and_only_its_alphabet() {
        let alphabet: Vec<char> = "ab€".chars().collect();
        let pw = password(200, &alphabet).unwrap();
        assert_eq!(pw.chars().count(), 200);
        assert!(pw.chars().all(|c| alphabet.contains(&c)));
        assert!(alphabet.iter().all(|c| pw.contains(*c)), "200 draws over 3 characters reach each");
        assert_eq!(password(0, &alphabet).unwrap().as_str(), "");
        assert!(password(4, &[]).is_err());
    }
}
