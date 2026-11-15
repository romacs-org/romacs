//! Time-based one-time passwords (RFC 6238) from an `otpauth://totp/`
//! URI, the Key Uri Format that pass-otp and authenticator apps share:
//! `secret` in base32 (RFC 4648), and `algorithm` (SHA1, SHA256 or
//! SHA512), `digits` (6 to 10) and `period` (seconds) with their usual
//! defaults.  Counter-based HOTP is refused: its counter lives in the
//! entry and moves at every use, which a read cannot do.

/// What a URI asks for.
#[derive(Debug, PartialEq, Eq)]
pub(crate) struct Totp {
    pub key: Vec<u8>,
    pub algorithm: &'static str,
    pub digits: u32,
    pub period: u64,
}

/// URI's TOTP parameters, or why it has none.
pub(crate) fn parse(uri: &str) -> Result<Totp, String> {
    let rest = uri.strip_prefix("otpauth://").ok_or("not an otpauth:// URI")?;
    let (kind, rest) = rest.split_once('/').ok_or("an otpauth URI without a label")?;
    match kind.to_ascii_lowercase().as_str() {
        "totp" => {}
        "hotp" => return Err("counter-based (HOTP) codes are not supported".into()),
        other => return Err(format!("unknown otpauth type: {other}")),
    }
    let query = rest.split_once('?').map_or("", |(_, query)| query);
    let mut totp = Totp { key: Vec::new(), algorithm: "sha1", digits: 6, period: 30 };
    let mut secret = None;
    for (name, value) in query.split('&').filter_map(|pair| pair.split_once('=')) {
        let value = percent_decoded(value);
        match name.to_ascii_lowercase().as_str() {
            "secret" => secret = Some(base32(&value).ok_or("the secret is not base32")?),
            "algorithm" => totp.algorithm = match value.to_ascii_uppercase().as_str() {
                "SHA1" => "sha1",
                "SHA256" => "sha256",
                "SHA512" => "sha512",
                _ => return Err(format!("unknown algorithm: {value}")),
            },
            "digits" => totp.digits = value.parse().ok().filter(|d| (6..=10).contains(d)).ok_or("digits is not 6 to 10")?,
            "period" => totp.period = value.parse().ok().filter(|&p| p > 0).ok_or("period is not a positive number of seconds")?,
            _ => {}
        }
    }
    totp.key = secret.filter(|key| !key.is_empty()).ok_or("the URI has no secret")?;
    Ok(totp)
}

/// The code TOTP gives at UNIX-SECONDS, zero-padded to its digits, and the
/// seconds it stays valid.
pub(crate) fn code(totp: &Totp, unix_seconds: u64) -> (String, u64) {
    let counter = unix_seconds / totp.period;
    let mac = crate::digest::hmac(totp.algorithm, &totp.key, &counter.to_be_bytes()).expect("a parsed algorithm");
    let offset = usize::from(mac[mac.len() - 1] & 0x0f);
    let word = u32::from_be_bytes(mac[offset..offset + 4].try_into().expect("four bytes")) & 0x7fff_ffff;
    let code = u64::from(word) % 10u64.pow(totp.digits);
    (format!("{code:0width$}", width = totp.digits as usize), totp.period - unix_seconds % totp.period)
}

/// RFC 4648 base32, any case, padding and spaces ignored.
fn base32(text: &str) -> Option<Vec<u8>> {
    let (mut out, mut buffer, mut bits) = (Vec::new(), 0u64, 0u32);
    for c in text.chars().filter(|c| !matches!(c, '=' | ' ' | '-')) {
        let value = match c.to_ascii_uppercase() {
            c @ 'A'..='Z' => c as u64 - 'A' as u64,
            c @ '2'..='7' => c as u64 - '2' as u64 + 26,
            _ => return None,
        };
        buffer = (buffer << 5) | value;
        bits += 5;
        if bits >= 8 {
            bits -= 8;
            out.push(u8::try_from(buffer >> bits).ok()?);
            buffer &= (1 << bits) - 1;
        }
    }
    Some(out)
}

/// TEXT with its `%XX` escapes decoded, and `+` as a space.
fn percent_decoded(text: &str) -> String {
    let bytes = text.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        let hex = bytes.get(i + 1..i + 3).and_then(|h| std::str::from_utf8(h).ok()).and_then(|h| u8::from_str_radix(h, 16).ok());
        match (bytes[i], hex) {
            (b'%', Some(byte)) => { out.push(byte); i += 3; }
            (b'+', _) => { out.push(b' '); i += 1; }
            (byte, _) => { out.push(byte); i += 1; }
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// RFC 6238 Appendix B: the seeds are ASCII "1234567890" repeated to
    /// the hash's size, and the codes have eight digits.
    #[test]
    fn rfc6238_vectors() {
        let seed = |n: usize| b"1234567890".iter().cycle().take(n).copied().collect::<Vec<u8>>();
        let cases: [(u64, &str, &str, &str); 6] = [
            (59, "94287082", "46119246", "90693936"),
            (1_111_111_109, "07081804", "68084774", "25091201"),
            (1_111_111_111, "14050471", "67062674", "99943326"),
            (1_234_567_890, "89005924", "91819424", "93441116"),
            (2_000_000_000, "69279037", "90698825", "38618901"),
            (20_000_000_000, "65353130", "77737706", "47863826"),
        ];
        for (time, sha1, sha256, sha512) in cases {
            for (algorithm, size, expected) in [("sha1", 20, sha1), ("sha256", 32, sha256), ("sha512", 64, sha512)] {
                let totp = Totp { key: seed(size), algorithm, digits: 8, period: 30 };
                assert_eq!(code(&totp, time).0, expected, "{algorithm} at {time}");
            }
        }
    }

    #[test]
    fn a_uri_gives_its_parameters_and_defaults() {
        let totp = parse("otpauth://totp/Example:alice%40example.org?secret=JBSWY3DPEHPK3PXP&issuer=Example").unwrap();
        assert_eq!(totp, Totp { key: b"Hello!\xde\xad\xbe\xef".to_vec(), algorithm: "sha1", digits: 6, period: 30 });
        let totp = parse("otpauth://totp/x?secret=jbsw%20y3dp&algorithm=SHA512&digits=8&period=60").unwrap();
        assert_eq!((totp.key.as_slice(), totp.algorithm, totp.digits, totp.period), (&b"Hello"[..], "sha512", 8, 60));
        assert_eq!(code(&totp, 61).1, 59);
    }

    #[test]
    fn a_bad_uri_says_why() {
        assert!(parse("otpauth://hotp/x?secret=JBSWY3DP&counter=1").unwrap_err().contains("HOTP"));
        assert!(parse("otpauth://totp/x?issuer=nobody").unwrap_err().contains("no secret"));
        assert!(parse("otpauth://totp/x?secret=not*base32").unwrap_err().contains("base32"));
        assert!(parse("otpauth://totp/x?secret=JBSWY3DP&digits=4").unwrap_err().contains("digits"));
        assert!(parse("https://example.org").is_err());
    }
}
