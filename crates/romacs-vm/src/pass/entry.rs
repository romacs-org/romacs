//! An entry's text as GNU's auth-source-pass reads it
//! (`auth-source-pass--parse-secret`, `auth-source-pass--parse-data`).

/// The secret: the first line that is not empty, as it stands.
pub(crate) fn secret(text: &str) -> Option<&str> {
    text.split('\n').find(|line| !line.is_empty())
}

/// The fields: each later line, trimmed of spaces and tabs, that holds a
/// colon, split at its first one, both sides trimmed as `string-trim`
/// trims.  The first line counted is the first that is not blank once
/// trimmed, which is the secret's line unless blank lines precede it.
pub(crate) fn fields(text: &str) -> Vec<(&str, &str)> {
    text.split('\n')
        .map(|line| line.trim_matches([' ', '\t']))
        .filter(|line| !line.is_empty())
        .skip(1)
        .filter_map(|line| line.split_once(':'))
        .map(|(key, value)| (trim(key), trim(value)))
        .collect()
}

/// The first `otpauth://` URI the entry holds on a line of its own, as
/// pass-otp stores it, else a field's value that is one.
pub(crate) fn otp_uri(text: &str) -> Option<&str> {
    let lines = || text.split('\n').map(trim);
    lines().find(|line| line.starts_with("otpauth://"))
        .or_else(|| lines().filter_map(|line| line.split_once(':')).map(|(_, value)| trim(value)).find(|value| value.starts_with("otpauth://")))
}

/// The default of `string-trim`: spaces, tabs, newlines and returns.
fn trim(text: &str) -> &str {
    text.trim_matches([' ', '\t', '\n', '\r'])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_first_line_is_the_secret_and_the_rest_are_fields() {
        let text = "hunter2\nuser: thanos\n url :  https://example.org/x \nnotes without colon\nport:\n";
        assert_eq!(secret(text), Some("hunter2"));
        assert_eq!(fields(text), [("user", "thanos"), ("url", "https://example.org/x"), ("port", "")]);
    }

    #[test]
    fn blank_lines_before_the_secret_are_skipped_as_gnu_skips_them() {
        assert_eq!(secret("\n\npw\nk: v"), Some("pw"));
        assert_eq!(fields("\n\npw\nk: v"), [("k", "v")]);
        // A line of spaces is a secret to `split-string' without trimming, and blank to the fields' split.
        assert_eq!(secret("  \npw\nk: v"), Some("  "));
        assert_eq!(fields("  \npw\nk: v"), [("k", "v")]);
        assert_eq!(secret(""), None);
        assert!(fields("").is_empty());
    }

    #[test]
    fn an_otpauth_line_is_found_whole() {
        let uri = "otpauth://totp/Example:me?secret=JBSWY3DPEHPK3PXP&issuer=Example";
        assert_eq!(otp_uri(&format!("pw\nuser: me\n{uri}\n")), Some(uri));
        assert_eq!(otp_uri(&format!("pw\notp: {uri}\n")), Some(uri));
        assert_eq!(otp_uri("pw\nuser: me\n"), None);
    }
}
