//! GNU gnutls.c as a build without GnuTLS has it (VM2-TLS1): VM2 has no
//! TLS transport, so the capability query answers that none is available.
use super::*;

defsubr! { name = "gnutls-available-p", gnu = "gnutls.c:2949", effect = Pure,
fn gnutls_available_p(cx) {
    let _ = cx;
    Ok(Value::NIL)
}}
