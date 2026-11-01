//! GNU gnutls.c's client primitives over rustls (crate::tls, VM2-TLS1):
//! X.509 sessions on network processes, their stage, verification and
//! peer status. The crypto primitives (`gnutls-hash-*', `gnutls-symmetric-*')
//! are absent, and `gnutls-available-p' does not claim them.
use super::*;

defsubr! { name = "gnutls-available-p", gnu = "gnutls.c:2949", effect = Pure,
fn gnutls_available_p(cx) {
    let names = [cx.intern("gnutls"), cx.intern("gnutls3")];
    Ok(cx.coding_list(&names))
}}

defsubr! { name = "gnutls-boot", gnu = "gnutls.c:1915", effect = Global,
fn gnutls_boot(cx, proc: Value, kind: Value, proplist: Value) { cx.gnutls_boot(proc, kind, proplist) }}

defsubr! { name = "gnutls-get-initstage", gnu = "gnutls.c:992", effect = Reads,
fn gnutls_get_initstage(cx, proc: Value) { cx.gnutls_initstage(proc) }}

defsubr! { name = "gnutls-deinit", gnu = "gnutls.c:1076", effect = Global,
fn gnutls_deinit(cx, proc: Value) { cx.gnutls_deinit(proc) }}

defsubr! { name = "gnutls-bye", gnu = "gnutls.c:2305", effect = Global,
fn gnutls_bye(cx, proc: Value, _cont: Value) { cx.gnutls_bye(proc) }}

defsubr! { name = "gnutls-peer-status", gnu = "gnutls.c:1385", effect = Reads,
fn gnutls_peer_status(cx, proc: Value) { cx.gnutls_peer_status(proc) }}

// The wait's gnutls_verify_boot for a non-blocking client whose handshake
// it finished (process.c:5466): PROC's :tls-parameters' verification.
defsubr! { name = "romacs--gnutls-verify", gnu = "gnutls.c:1681", effect = Global,
fn romacs_gnutls_verify(cx, proc: Value) { cx.gnutls_verify_boot(proc, Value::NIL) }}

defsubr! { name = "gnutls-errorp", gnu = "gnutls.c:1002", effect = Pure,
fn gnutls_errorp(cx, error: Value) {
    let again = cx.intern("gnutls-e-again");
    Ok(if error == Value::TRUE || error == again { Value::NIL } else { Value::TRUE })
}}

defsubr! { name = "gnutls-error-fatalp", gnu = "gnutls.c:1015", effect = Pure,
fn gnutls_error_fatalp(cx, error: Value) {
    if error == Value::TRUE { return Ok(Value::NIL); }
    let fatal = match cx.gnutls_code(error)? { Some(code) => crate::tls::error_fatal(code), None => true };
    Ok(if fatal { Value::TRUE } else { Value::NIL })
}}

defsubr! { name = "gnutls-error-string", gnu = "gnutls.c:1047", effect = Pure,
fn gnutls_error_string(cx, error: Value) {
    let text = if error == Value::TRUE { "Not an error" } else {
        cx.gnutls_code(error)?.and_then(crate::tls::error_string).unwrap_or("(unknown error code)")
    };
    Ok(cx.make_string(text.as_bytes().to_vec(), false))
}}

defsubr! { name = "gnutls-peer-status-warning-describe", gnu = "gnutls.c:1325", effect = Pure,
fn gnutls_peer_status_warning_describe(cx, warning: Value) {
    if cx.kind(warning) != Kind::Symbol { return Err(Fault::Wrong("symbolp", warning)); }
    let text = match cx.symbol_name(warning)?.as_str() {
        ":invalid" => "certificate could not be verified",
        ":revoked" => "certificate was revoked (CRL)",
        ":self-signed" => "certificate signer was not found (self-signed)",
        ":unknown-ca" => "the certificate was signed by an unknown and therefore untrusted authority",
        ":not-ca" => "certificate signer is not a CA",
        ":insecure" => "certificate was signed with an insecure algorithm",
        ":not-activated" => "certificate is not yet activated",
        ":expired" => "certificate has expired",
        ":no-host-match" => "certificate host does not match hostname",
        ":signature-failure" => "certificate signature could not be verified",
        ":revocation-data-superseded" => "certificate revocation data are old and have been superseded",
        ":revocation-data-issued-in-future" => "certificate revocation data have a future issue date",
        ":signer-constraints-failure" => "certificate signer constraints were violated",
        ":purpose-mismatch" => "certificate does not match the intended purpose",
        ":missing-ocsp-status" => "certificate requires the server to send a OCSP certificate status, but no status was received",
        ":invalid-ocsp-status" => "the received OCSP certificate status is invalid",
        _ => return Ok(Value::NIL),
    };
    Ok(cx.make_string(text.as_bytes().to_vec(), false))
}}
