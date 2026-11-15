//! The password store's pure parts, for lisp/romacs/romacs-pass.el:
//! entry text, one-time codes, the store's entry names and new
//! passwords.  Decryption is gpg's, run from Lisp through gpg-agent; what
//! it yields is read here once, so the plaintext is split into no more
//! Lisp strings than the fields asked for.
//!
//! The store is pass(1)'s: a directory of `NAME.gpg` files, each one an
//! encrypted text whose first line is the password and whose other
//! lines are `key: value` fields, one of them possibly an `otpauth://`
//! URI (pass-otp's convention).

pub(crate) mod entry;
pub(crate) mod generate;
pub(crate) mod otp;
pub(crate) mod store;
