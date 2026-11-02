//! GNU gnutls.c's TLS client sessions, over rustls: a process channel's
//! session, its handshake, its record I/O on the channel's descriptor, and
//! what `gnutls-peer-status' reports of the peer. Verification follows
//! gnutls_verify_boot: the handshake never fails on the peer's certificate;
//! the chain's GnuTLS status bits and the host match are recorded, and
//! `gnutls-boot' refuses them only as its :verify-error asks. Nothing here
//! calls Lisp (prims/gnutls.rs builds the Lisp values).
use rustls::client::danger::{HandshakeSignatureValid, ServerCertVerified, ServerCertVerifier};
use rustls::pki_types::pem::PemObject;
use rustls::pki_types::{CertificateDer, PrivateKeyDer, ServerName, UnixTime};
use rustls::{DigitallySignedStruct, SignatureScheme};
use std::io::{self, Read, Write};
use std::os::fd::RawFd;
use std::sync::{Arc, Mutex};

/// GNUTLS_STAGE_*: how far `gnutls-boot' got (gnutls.h's gnutls_initstage_t).
pub(crate) mod stage {
    pub const EMPTY: u8 = 0;
    pub const CRED_SET: u8 = 6;
    pub const HANDSHAKE_TRIED: u8 = 8;
    pub const READY: u8 = 9;
}

/// The gnutls_certificate_status_t bits a chain verification sets.
pub(crate) mod cert {
    pub const INVALID: u32 = 1 << 1;
    pub const REVOKED: u32 = 1 << 5;
    pub const SIGNER_NOT_FOUND: u32 = 1 << 6;
    pub const SIGNER_NOT_CA: u32 = 1 << 7;
    pub const INSECURE_ALGORITHM: u32 = 1 << 8;
    pub const NOT_ACTIVATED: u32 = 1 << 9;
    pub const EXPIRED: u32 = 1 << 10;
    pub const SIGNATURE_FAILURE: u32 = 1 << 11;
    pub const SIGNER_CONSTRAINTS_FAILURE: u32 = 1 << 16;
    pub const PURPOSE_MISMATCH: u32 = 1 << 18;
}

/// The GnuTLS error codes a session reports (gnutls.h GNUTLS_E_*).
pub(crate) mod code {
    pub const UNSUPPORTED_VERSION_PACKET: i32 = -8;
    pub const UNEXPECTED_PACKET_LENGTH: i32 = -9;
    pub const FATAL_ALERT_RECEIVED: i32 = -12;
    pub const UNEXPECTED_PACKET: i32 = -15;
    pub const DECRYPTION_FAILED: i32 = -24;
    pub const AGAIN: i32 = -28;
    pub const INSUFFICIENT_CREDENTIALS: i32 = -32;
    pub const CERTIFICATE_ERROR: i32 = -43;
    pub const INTERRUPTED: i32 = -52;
    pub const PUSH_ERROR: i32 = -53;
    pub const PULL_ERROR: i32 = -54;
    pub const RECEIVED_ILLEGAL_PARAMETER: i32 = -55;
    pub const INTERNAL_ERROR: i32 = -59;
    pub const FILE_ERROR: i32 = -64;
    pub const NO_CIPHER_SUITES: i32 = -87;
    pub const PREMATURE_TERMINATION: i32 = -110;
    pub const NO_PRIORITIES_WERE_SET: i32 = -326;
}

/// gnutls_strerror for the codes above; None for one GnuTLS does not know.
pub(crate) fn error_string(code: i32) -> Option<&'static str> {
    Some(match code {
        0 => "Success.",
        -8 => "A packet with illegal or unsupported version was received.",
        -9 => "Error decoding the received TLS packet.",
        -10 => "The specified session has been invalidated for some reason.",
        -12 => "A TLS fatal alert has been received.",
        -15 => "An unexpected TLS packet was received.",
        -24 => "Decryption has failed.",
        -28 => "Resource temporarily unavailable, try again.",
        -32 => "Insufficient credentials for that request.",
        -43 => "Error in the certificate.",
        -50 => "The request is invalid.",
        -52 => "Function was interrupted.",
        -53 => "Error in the push function.",
        -54 => "Error in the pull function.",
        -55 => "An illegal parameter has been received.",
        -59 => "GnuTLS internal error.",
        -64 => "Error while reading file.",
        -87 => "No supported cipher suites have been found.",
        -110 => "The TLS connection was non-properly terminated.",
        -326 => "No or insufficient priorities were set.",
        _ => return None,
    })
}

/// gnutls_error_is_fatal: every negative code but the retryable ones.
pub(crate) fn error_fatal(code: i32) -> bool {
    code < 0 && !matches!(code, code::AGAIN | code::INTERRUPTED)
}

/// A boot that could not start: a GnuTLS error code, or the message
/// gnutls.c's boot_error reports.
pub(crate) enum BootError {
    Code(i32),
    Message(String),
}

/// What `gnutls-boot' reads of its PROPLIST.
pub(crate) struct Params {
    pub hostname: String,
    pub trustfiles: Vec<std::path::PathBuf>,
    /// :keylist's (KEYFILE CERTFILE) pairs; the first is the client's.
    pub keylist: Vec<(std::path::PathBuf, std::path::PathBuf)>,
    pub priority: Option<String>,
}

/// What the handshake learned of the peer, for gnutls_verify_boot.
#[derive(Default)]
struct Peer {
    verification: u32,
    host_mismatch: bool,
    certificates: Vec<Vec<u8>>,
}

/// One process's TLS session over its channel's descriptors.
pub(crate) struct Session {
    conn: rustls::ClientConnection,
    peer: Arc<Mutex<Peer>>,
    pub stage: u8,
    /// p->gnutls_p: records go through the session. A blocking client's
    /// becomes active once gnutls_verify_boot accepts the peer; until then
    /// (and after a refusal) the process reads and writes plain bytes.
    pub active: bool,
    /// gnutls_verify_boot's results, kept once the handshake is done.
    pub verification: u32,
    pub host_mismatch: bool,
    pub certificates: Vec<Vec<u8>>,
}

/// The system trust gnutls_certificate_set_x509_system_trust adds to every
/// session: the distribution's bundle, read once.
fn system_trust() -> &'static [CertificateDer<'static>] {
    static TRUST: std::sync::OnceLock<Vec<CertificateDer<'static>>> = std::sync::OnceLock::new();
    TRUST.get_or_init(|| {
        ["/etc/ssl/certs/ca-certificates.crt", "/etc/pki/tls/certs/ca-bundle.crt", "/etc/ssl/ca-bundle.pem", "/etc/ssl/cert.pem"]
            .iter()
            .find_map(|path| {
                let certs: Vec<_> = CertificateDer::pem_file_iter(path).ok()?.filter_map(|c| c.ok()).collect();
                (!certs.is_empty()).then_some(certs)
            })
            .unwrap_or_default()
    })
}

/// gnutls_priority_set_direct's protocol versions: TLS 1.2 and 1.3 (what
/// rustls speaks) less any `-VERS-' the string removes; the rest of the
/// priority string selects nothing rustls offers otherwise.
fn versions(priority: Option<&str>) -> Result<Vec<&'static rustls::SupportedProtocolVersion>, BootError> {
    let (mut tls12, mut tls13) = (true, true);
    for token in priority.unwrap_or("NORMAL").split(':') {
        match token {
            "-VERS-ALL" | "-VERS-TLS-ALL" => (tls12, tls13) = (false, false),
            "+VERS-ALL" | "+VERS-TLS-ALL" => (tls12, tls13) = (true, true),
            "-VERS-TLS1.2" => tls12 = false,
            "+VERS-TLS1.2" => tls12 = true,
            "-VERS-TLS1.3" => tls13 = false,
            "+VERS-TLS1.3" => tls13 = true,
            _ => {}
        }
    }
    let chosen: Vec<_> = [(tls13, &rustls::version::TLS13), (tls12, &rustls::version::TLS12)]
        .into_iter().filter(|(on, _)| *on).map(|(_, v)| v).collect();
    if chosen.is_empty() { Err(BootError::Code(code::NO_PRIORITIES_WERE_SET)) } else { Ok(chosen) }
}

fn trust_store(params: &Params) -> Result<rustls::RootCertStore, BootError> {
    let mut roots = rustls::RootCertStore::empty();
    roots.add_parsable_certificates(system_trust().iter().cloned());
    for file in &params.trustfiles {
        let certs: Vec<_> = CertificateDer::pem_file_iter(file).map_err(|_| BootError::Code(code::FILE_ERROR))?
            .collect::<Result<_, _>>().map_err(|_| BootError::Code(code::FILE_ERROR))?;
        roots.add_parsable_certificates(certs);
    }
    Ok(roots)
}

fn client_identity(params: &Params) -> Result<Option<(Vec<CertificateDer<'static>>, PrivateKeyDer<'static>)>, BootError> {
    let Some((key, cert)) = params.keylist.first() else { return Ok(None) };
    let certs = CertificateDer::pem_file_iter(cert).and_then(|i| i.collect::<Result<Vec<_>, _>>())
        .map_err(|_| BootError::Code(code::FILE_ERROR))?;
    let key = PrivateKeyDer::from_pem_file(key).map_err(|_| BootError::Code(code::FILE_ERROR))?;
    Ok(Some((certs, key)))
}

/// The server name a session offers and checks: an IP address names no
/// SNI host, as gnutls_server_name_set is skipped for one.
fn server_name(hostname: &str) -> Result<ServerName<'static>, BootError> {
    if let Ok(ip) = hostname.parse::<std::net::IpAddr>() {
        return Ok(ServerName::IpAddress(ip.into()));
    }
    ServerName::try_from(hostname.to_owned()).map_err(|_| BootError::Message("gnutls-boot: invalid :hostname parameter (not a string)".into()))
}

impl Session {
    /// gnutls-boot up to GNUTLS_STAGE_CRED_SET: credentials, trust,
    /// priorities and the server name; no I/O yet.
    pub(crate) fn new(params: &Params) -> Result<Session, BootError> {
        let provider = Arc::new(rustls::crypto::ring::default_provider());
        let peer = Arc::new(Mutex::new(Peer::default()));
        let verifier = Arc::new(Verifier { roots: Arc::new(trust_store(params)?), provider: provider.clone(), peer: peer.clone() });
        let builder = rustls::ClientConfig::builder_with_provider(provider)
            .with_protocol_versions(&versions(params.priority.as_deref())?)
            .map_err(|_| BootError::Code(code::NO_PRIORITIES_WERE_SET))?
            .dangerous()
            .with_custom_certificate_verifier(verifier);
        let config = match client_identity(params)? {
            Some((certs, key)) => builder.with_client_auth_cert(certs, key).map_err(|_| BootError::Code(code::CERTIFICATE_ERROR))?,
            None => builder.with_no_client_auth(),
        };
        let conn = rustls::ClientConnection::new(Arc::new(config), server_name(&params.hostname)?)
            .map_err(|_| BootError::Code(code::INTERNAL_ERROR))?;
        Ok(Session { conn, peer, stage: stage::CRED_SET, active: false, verification: 0, host_mismatch: false, certificates: Vec::new() })
    }

    pub(crate) fn wants_write(&self) -> bool {
        self.conn.wants_write()
    }

    /// Records waiting to go out, written as far as FD takes them now.
    pub(crate) fn flush(&mut self, fd: RawFd) -> io::Result<()> {
        while self.conn.wants_write() {
            match self.conn.write_tls(&mut Fd(fd)) {
                Ok(0) => break,
                Ok(_) => {}
                Err(error) if error.kind() == io::ErrorKind::WouldBlock => break,
                Err(error) => return Err(error),
            }
        }
        Ok(())
    }

    /// One non-blocking round of gnutls_handshake: Ok(true) once done.
    pub(crate) fn handshake_step(&mut self, fd: RawFd) -> Result<bool, i32> {
        self.flush(fd).map_err(|_| code::PUSH_ERROR)?;
        if self.conn.is_handshaking() && self.conn.wants_read() {
            match self.conn.read_tls(&mut Fd(fd)) {
                Ok(0) => return Err(code::PREMATURE_TERMINATION),
                Ok(_) => {
                    let processed = self.conn.process_new_packets();
                    // The alert a failure queued goes out before the error.
                    let _ = self.flush(fd);
                    processed.map_err(|error| error_code(&error))?;
                }
                Err(error) if error.kind() == io::ErrorKind::WouldBlock => {}
                Err(_) => return Err(code::PULL_ERROR),
            }
            self.flush(fd).map_err(|_| code::PUSH_ERROR)?;
        }
        let done = !self.conn.is_handshaking();
        self.stage = if done { stage::READY } else { stage::HANDSHAKE_TRIED };
        if done { self.take_peer(); }
        Ok(done)
    }

    /// gnutls_try_handshake for a blocking client: rounds until done,
    /// waiting on FD between them; an interrupt (C-g) ends it with
    /// GNUTLS_E_INTERRUPTED for the engine's safepoint to signal.
    pub(crate) fn handshake(&mut self, fd: RawFd) -> Result<(), i32> {
        loop {
            if self.handshake_step(fd)? {
                return Ok(());
            }
            if crate::host::interrupt_pending() {
                return Err(code::INTERRUPTED);
            }
            let events = libc::POLLIN | if self.conn.wants_write() { libc::POLLOUT } else { 0 };
            crate::subprocess::poll(&[(fd, events)], Some(std::time::Duration::from_millis(100))).map_err(|_| code::PULL_ERROR)?;
        }
    }

    fn take_peer(&mut self) {
        let peer = std::mem::take(&mut *self.peer.lock().expect("peer"));
        (self.verification, self.host_mismatch, self.certificates) = (peer.verification, peer.host_mismatch, peer.certificates);
    }

    /// Decrypted bytes the session holds that no read has taken yet.
    pub(crate) fn plaintext_pending(&mut self) -> bool {
        self.stage == stage::READY && self.conn.process_new_packets().is_ok_and(|s| s.plaintext_bytes_to_read() > 0)
    }

    /// emacs_gnutls_read: plaintext into BUF; Ok(0) at the peer's close
    /// (orderly or not), EAGAIN when no record is complete.
    pub(crate) fn read(&mut self, fd: RawFd, buf: &mut [u8]) -> io::Result<usize> {
        let again = || io::Error::from_raw_os_error(libc::EAGAIN);
        if self.stage != stage::READY {
            return Err(again());
        }
        if let Some(n) = self.take_plaintext(buf)? {
            return Ok(n);
        }
        match self.conn.read_tls(&mut Fd(fd)) {
            Ok(0) => return Ok(0),
            Ok(_) => {}
            Err(error) => return Err(error),
        }
        let processed = self.conn.process_new_packets();
        let _ = self.flush(fd);
        if processed.is_err() {
            // emacs_gnutls_handle_error: a fatal error reads as the end.
            return Ok(0);
        }
        self.take_plaintext(buf)?.map_or_else(|| Err(again()), Ok)
    }

    fn take_plaintext(&mut self, buf: &mut [u8]) -> io::Result<Option<usize>> {
        match self.conn.reader().read(buf) {
            Ok(n) => Ok(Some(n)),
            Err(error) if error.kind() == io::ErrorKind::WouldBlock => Ok(None),
            Err(error) if error.kind() == io::ErrorKind::UnexpectedEof => Ok(Some(0)),
            Err(error) => Err(error),
        }
    }

    /// emacs_gnutls_write: BYTES into records, sent as far as FD takes
    /// them now: how many bytes the session took. It holds at most its
    /// buffer limit of records FD has not taken; the caller keeps the rest
    /// of BYTES until FD is writable again.
    pub(crate) fn write(&mut self, fd: RawFd, bytes: &[u8]) -> io::Result<usize> {
        if self.stage != stage::READY {
            return Err(io::Error::from_raw_os_error(libc::EAGAIN));
        }
        let mut taken = 0;
        while taken < bytes.len() {
            self.flush(fd)?;
            match self.conn.writer().write(&bytes[taken..])? {
                0 => break,
                n => taken += n,
            }
        }
        self.flush(fd)?;
        Ok(taken)
    }

    /// gnutls_bye: the close_notify alert.
    pub(crate) fn bye(&mut self, fd: RawFd) -> io::Result<()> {
        self.conn.send_close_notify();
        self.flush(fd)
    }

    /// The negotiated parameters `gnutls-peer-status' reports:
    /// (KEY-EXCHANGE PROTOCOL CIPHER), with GnuTLS's names.
    pub(crate) fn negotiated(&self) -> Option<(&'static str, &'static str, &'static str)> {
        use rustls::CipherSuite as S;
        let suite = self.conn.negotiated_cipher_suite()?.suite();
        let protocol = match self.conn.protocol_version()? {
            rustls::ProtocolVersion::TLSv1_3 => "TLS1.3",
            rustls::ProtocolVersion::TLSv1_2 => "TLS1.2",
            _ => "UNKNOWN",
        };
        let name = format!("{suite:?}");
        // gnutls_kx_get under TLS 1.3 names an (EC)DHE group's exchange
        // ECDHE-RSA whatever the certificate's key.
        let kx = if protocol == "TLS1.3" || name.contains("ECDHE_RSA") { "ECDHE-RSA" } else { "ECDHE-ECDSA" };
        let cipher = match suite {
            S::TLS13_AES_256_GCM_SHA384 | S::TLS_ECDHE_RSA_WITH_AES_256_GCM_SHA384 | S::TLS_ECDHE_ECDSA_WITH_AES_256_GCM_SHA384 => "AES-256-GCM",
            S::TLS13_AES_128_GCM_SHA256 | S::TLS_ECDHE_RSA_WITH_AES_128_GCM_SHA256 | S::TLS_ECDHE_ECDSA_WITH_AES_128_GCM_SHA256 => "AES-128-GCM",
            _ => "CHACHA20-POLY1305",
        };
        Some((kx, protocol, cipher))
    }
}

/// The GnuTLS error a rustls failure reads as.
fn error_code(error: &rustls::Error) -> i32 {
    use rustls::Error as E;
    match error {
        E::AlertReceived(_) => code::FATAL_ALERT_RECEIVED,
        E::InappropriateMessage { .. } | E::InappropriateHandshakeMessage { .. } => code::UNEXPECTED_PACKET,
        E::InvalidMessage(_) => code::UNEXPECTED_PACKET_LENGTH,
        E::DecryptError => code::DECRYPTION_FAILED,
        E::PeerIncompatible(rustls::PeerIncompatible::Tls12NotOffered | rustls::PeerIncompatible::Tls13RequiredForQuic
            | rustls::PeerIncompatible::SupportedVersionsExtensionRequired) => code::UNSUPPORTED_VERSION_PACKET,
        E::PeerIncompatible(_) => code::NO_CIPHER_SUITES,
        E::PeerMisbehaved(_) => code::RECEIVED_ILLEGAL_PARAMETER,
        E::NoCertificatesPresented | E::InvalidCertificate(_) => code::CERTIFICATE_ERROR,
        E::General(_) | E::FailedToGetCurrentTime | E::FailedToGetRandomBytes => code::INTERNAL_ERROR,
        _ => code::INSUFFICIENT_CREDENTIALS,
    }
}

/// gnutls_certificate_verify_peers2's status for a chain webpki refused.
fn status_bits(error: &webpki::Error) -> u32 {
    use webpki::Error as E;
    cert::INVALID | match error {
        E::UnknownIssuer => cert::SIGNER_NOT_FOUND,
        E::CertExpired { .. } => cert::EXPIRED,
        E::CertNotValidYet { .. } => cert::NOT_ACTIVATED,
        E::CertRevoked => cert::REVOKED,
        E::EndEntityUsedAsCa => cert::SIGNER_NOT_CA,
        E::InvalidSignatureForPublicKey | E::UnsupportedSignatureAlgorithmForPublicKeyContext(_)
        | E::SignatureAlgorithmMismatch => cert::SIGNATURE_FAILURE,
        E::UnsupportedSignatureAlgorithmContext(_) => cert::INSECURE_ALGORITHM,
        E::RequiredEkuNotFoundContext(_) => cert::PURPOSE_MISMATCH,
        E::NameConstraintViolation | E::PathLenConstraintViolated => cert::SIGNER_CONSTRAINTS_FAILURE,
        _ => 0,
    }
}

/// The handshake's certificate check: record, never refuse (see the
/// module comment); the handshake signatures are verified as rustls does.
#[derive(Debug)]
struct Verifier {
    roots: Arc<rustls::RootCertStore>,
    provider: Arc<rustls::crypto::CryptoProvider>,
    peer: Arc<Mutex<Peer>>,
}

impl std::fmt::Debug for Peer {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Peer").field("verification", &self.verification).finish()
    }
}

impl ServerCertVerifier for Verifier {
    fn verify_server_cert(&self, end_entity: &CertificateDer<'_>, intermediates: &[CertificateDer<'_>], server_name: &ServerName<'_>,
                          _ocsp: &[u8], now: UnixTime) -> Result<ServerCertVerified, rustls::Error> {
        let mut peer = Peer {
            certificates: std::iter::once(end_entity).chain(intermediates).map(|c| c.to_vec()).collect(),
            ..Peer::default()
        };
        match webpki::EndEntityCert::try_from(end_entity) {
            Ok(leaf) => {
                let algorithms = self.provider.signature_verification_algorithms.all;
                peer.verification = leaf.verify_for_usage(algorithms, &self.roots.roots, intermediates, now, webpki::KeyUsage::server_auth(), None, None)
                    .map_or_else(|e| status_bits(&e), |_| 0);
                peer.host_mismatch = leaf.verify_is_valid_for_subject_name(server_name).is_err();
            }
            Err(_) => (peer.verification, peer.host_mismatch) = (cert::INVALID, true),
        }
        *self.peer.lock().expect("peer") = peer;
        Ok(ServerCertVerified::assertion())
    }
    fn verify_tls12_signature(&self, message: &[u8], cert: &CertificateDer<'_>, dss: &DigitallySignedStruct) -> Result<HandshakeSignatureValid, rustls::Error> {
        rustls::crypto::verify_tls12_signature(message, cert, dss, &self.provider.signature_verification_algorithms)
    }
    fn verify_tls13_signature(&self, message: &[u8], cert: &CertificateDer<'_>, dss: &DigitallySignedStruct) -> Result<HandshakeSignatureValid, rustls::Error> {
        rustls::crypto::verify_tls13_signature(message, cert, dss, &self.provider.signature_verification_algorithms)
    }
    fn supported_verify_schemes(&self) -> Vec<SignatureScheme> {
        self.provider.signature_verification_algorithms.supported_schemes()
    }
}

/// A descriptor as rustls's transport.
struct Fd(RawFd);
impl Read for Fd {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        crate::subprocess::read(self.0, buf)
    }
}
impl Write for Fd {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        crate::subprocess::write(self.0, buf)
    }
    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

// --- X.509 details (emacs_gnutls_certificate_details) -------------------------

/// One DER element: its tag, its contents, and the whole encoding.
#[derive(Clone, Copy)]
struct Der<'a> {
    tag: u8,
    body: &'a [u8],
    raw: &'a [u8],
}

/// The first element of INPUT and what follows it.
fn der(input: &[u8]) -> Option<(Der<'_>, &[u8])> {
    let (&tag, rest) = input.split_first()?;
    let (&first, rest) = rest.split_first()?;
    let (len, rest) = if first < 0x80 {
        (first as usize, rest)
    } else {
        let n = (first & 0x7f) as usize;
        if n == 0 || n > 4 || rest.len() < n { return None; }
        (rest[..n].iter().fold(0usize, |a, &b| (a << 8) | b as usize), &rest[n..])
    };
    if rest.len() < len { return None; }
    let header = input.len() - rest.len();
    Some((Der { tag, body: &rest[..len], raw: &input[..header + len] }, &rest[len..]))
}

fn elements(mut input: &[u8]) -> Vec<Der<'_>> {
    let mut out = Vec::new();
    while let Some((e, rest)) = der(input) {
        out.push(e);
        input = rest;
    }
    out
}

fn oid(body: &[u8]) -> String {
    let mut arcs: Vec<u64> = Vec::new();
    let mut value = 0u64;
    for &b in body {
        value = (value << 7) | u64::from(b & 0x7f);
        if b & 0x80 == 0 {
            if arcs.is_empty() {
                let first = (value / 40).min(2);
                arcs.extend([first, value - first * 40]);
            } else {
                arcs.push(value);
            }
            value = 0;
        }
    }
    arcs.iter().map(u64::to_string).collect::<Vec<_>>().join(".")
}

/// gnutls_hex_string: lowercase pairs joined by colons after PREFIX.
pub(crate) fn hex(bytes: &[u8], prefix: &str) -> String {
    prefix.to_owned() + &bytes.iter().map(|b| format!("{b:02x}")).collect::<Vec<_>>().join(":")
}

/// The LDAP name GnuTLS gives an attribute type.
fn attribute_name(oid: &str) -> String {
    match oid {
        "2.5.4.3" => "CN", "2.5.4.6" => "C", "2.5.4.7" => "L", "2.5.4.8" => "ST", "2.5.4.10" => "O", "2.5.4.11" => "OU",
        "2.5.4.5" => "serialNumber", "2.5.4.9" => "street", "2.5.4.12" => "title", "2.5.4.4" => "surName",
        "2.5.4.42" => "givenName", "2.5.4.43" => "initials", "2.5.4.44" => "generationQualifier", "2.5.4.46" => "dnQualifier",
        "2.5.4.65" => "pseudonym", "2.5.4.17" => "postalCode", "2.5.4.15" => "businessCategory", "2.5.4.97" => "organizationIdentifier",
        "0.9.2342.19200300.100.1.25" => "DC", "0.9.2342.19200300.100.1.1" => "UID", "1.2.840.113549.1.9.1" => "EMAIL",
        "1.3.6.1.4.1.311.60.2.1.1" => "jurisdictionOfIncorporationLocalityName",
        "1.3.6.1.4.1.311.60.2.1.2" => "jurisdictionOfIncorporationStateOrProvinceName",
        "1.3.6.1.4.1.311.60.2.1.3" => "jurisdictionOfIncorporationCountryName",
        other => return other.to_owned(),
    }.to_owned()
}

/// An attribute's value as RFC 4514 writes it.
fn attribute_value(value: Der<'_>) -> String {
    let text = match value.tag {
        0x0c | 0x13 | 0x16 | 0x1a => String::from_utf8_lossy(value.body).into_owned(),
        0x14 => value.body.iter().map(|&b| b as char).collect(),
        0x1e => char::decode_utf16(value.body.chunks(2).map(|c| u16::from_be_bytes([c[0], *c.get(1).unwrap_or(&0)])))
            .map(|c| c.unwrap_or('\u{fffd}')).collect(),
        _ => return format!("#{}", value.raw.iter().map(|b| format!("{b:02x}")).collect::<String>()),
    };
    let last = text.chars().count().saturating_sub(1);
    text.chars().enumerate().fold(String::new(), |mut out, (i, c)| {
        if matches!(c, ',' | '+' | '"' | '\\' | '<' | '>' | ';') || (i == 0 && matches!(c, '#' | ' ')) || (i == last && c == ' ') {
            out.push('\\');
        }
        out.push(c);
        out
    })
}

/// gnutls_x509_crt_get_dn: a Name's RDNs in their DER order (its
/// compatible form, not RFC 4514's reversed one), each as RFC 4514 writes it.
fn distinguished_name(name: Der<'_>) -> String {
    elements(name.body).iter().map(|rdn| {
        elements(rdn.body).iter().filter_map(|pair| {
            let parts = elements(pair.body);
            Some(format!("{}={}", attribute_name(&oid(parts.first()?.body)), attribute_value(*parts.get(1)?)))
        }).collect::<Vec<_>>().join("+")
    }).collect::<Vec<_>>().join(",")
}

/// A UTCTime or GeneralizedTime as %Y-%m-%d.
fn date(time: Der<'_>) -> Option<String> {
    let text = std::str::from_utf8(time.body).ok()?;
    let (year, rest) = if time.tag == 0x17 {
        let yy: u32 = text.get(..2)?.parse().ok()?;
        (if yy < 50 { 2000 + yy } else { 1900 + yy }, text.get(2..)?)
    } else {
        (text.get(..4)?.parse().ok()?, text.get(4..)?)
    };
    Some(format!("{year:04}-{}-{}", rest.get(..2)?, rest.get(2..4)?))
}

fn significant_bits(integer: &[u8]) -> usize {
    let bytes = integer.iter().skip_while(|&&b| b == 0).collect::<Vec<_>>();
    bytes.first().map_or(0, |&&b| (bytes.len() - 1) * 8 + (8 - b.leading_zeros() as usize))
}

/// gnutls_x509_crt_get_pk_algorithm's name and bits.
fn public_key(spki: Der<'_>) -> Option<(&'static str, usize, bool)> {
    let parts = elements(spki.body);
    let algorithm = elements(parts.first()?.body);
    let key = parts.get(1)?.body.get(1..)?;
    Some(match oid(algorithm.first()?.body).as_str() {
        "1.2.840.113549.1.1.1" => ("RSA", significant_bits(elements(der(key)?.0.body).first()?.body), false),
        "1.2.840.113549.1.1.10" => ("RSA-PSS", significant_bits(elements(der(key)?.0.body).first()?.body), false),
        "1.2.840.10040.4.1" => ("DSA", 0, false),
        "1.2.840.10045.2.1" => ("EC/ECDSA", match oid(algorithm.get(1)?.body).as_str() {
            "1.2.840.10045.3.1.7" => 256, "1.3.132.0.34" => 384, "1.3.132.0.35" => 521, "1.3.132.0.10" => 256, _ => 0,
        }, true),
        "1.3.101.112" => ("EdDSA (Ed25519)", 256, true),
        "1.3.101.113" => ("EdDSA (Ed448)", 456, true),
        _ => return None,
    })
}

/// gnutls_sec_param_get_name (gnutls_pk_bits_to_sec_param): GnuTLS's
/// security parameter table, by modulus or curve bits.
fn security_level(bits: usize, elliptic: bool) -> &'static str {
    const LEVELS: [(&str, usize, usize); 10] = [
        ("Insecure", 0, 0), ("Export", 512, 0), ("Very weak", 767, 0), ("Weak", 1008, 160), ("Low", 1024, 160),
        ("Legacy", 1776, 192), ("Medium", 2048, 224), ("High", 3072, 256), ("Ultra", 8192, 384), ("Future", 15360, 512),
    ];
    LEVELS.iter().take_while(|(_, pk, ecc)| if elliptic { *ecc <= bits } else { *pk <= bits }).last().map_or("Insecure", |l| l.0)
}

fn signature_name(algorithm: Der<'_>) -> Option<&'static str> {
    Some(match oid(elements(algorithm.body).first()?.body).as_str() {
        "1.2.840.113549.1.1.4" => "RSA-MD5", "1.2.840.113549.1.1.5" => "RSA-SHA1", "1.2.840.113549.1.1.14" => "RSA-SHA224",
        "1.2.840.113549.1.1.11" => "RSA-SHA256", "1.2.840.113549.1.1.12" => "RSA-SHA384", "1.2.840.113549.1.1.13" => "RSA-SHA512",
        "1.2.840.113549.1.1.10" => "RSA-PSS-SHA256", "1.2.840.10045.4.1" => "ECDSA-SHA1", "1.2.840.10045.4.3.1" => "ECDSA-SHA224",
        "1.2.840.10045.4.3.2" => "ECDSA-SHA256", "1.2.840.10045.4.3.3" => "ECDSA-SHA384", "1.2.840.10045.4.3.4" => "ECDSA-SHA512",
        "1.3.101.112" => "EdDSA-Ed25519", "1.3.101.113" => "EdDSA-Ed448", "1.2.840.10040.4.3" => "DSA-SHA1",
        _ => return None,
    })
}

/// A certificate detail's value.
pub(crate) enum Detail {
    Int(i64),
    Str(String),
}

/// emacs_gnutls_certificate_details: (KEY VALUE) pairs in GNU's order, for
/// the fields DER parses; a field it cannot read is left out, as GNU leaves
/// out one GnuTLS cannot.
pub(crate) fn certificate_details(der_cert: &[u8]) -> Vec<(&'static str, Detail)> {
    use sha1::Digest;
    let mut out = Vec::new();
    let Some((certificate, _)) = der(der_cert) else { return out };
    let parts = elements(certificate.body);
    let Some(tbs) = parts.first() else { return out };
    let mut fields = elements(tbs.body).into_iter().peekable();
    let version = if fields.peek().is_some_and(|f| f.tag == 0xa0) {
        fields.next().and_then(|v| der(v.body)).and_then(|(i, _)| i.body.last().copied()).map_or(1, |v| i64::from(v) + 1)
    } else { 1 };
    out.push(("version", Detail::Int(version)));
    let rest: Vec<_> = fields.collect();
    let field = |i: usize| rest.get(i).copied();
    if let Some(serial) = field(0) { out.push(("serial-number", Detail::Str(hex(serial.body, "")))); }
    if let Some(issuer) = field(2) { out.push(("issuer", Detail::Str(distinguished_name(issuer)))); }
    if let Some(validity) = field(3) {
        let times = elements(validity.body);
        if let Some(from) = times.first().and_then(|t| date(*t)) { out.push(("valid-from", Detail::Str(from))); }
        if let Some(to) = times.get(1).and_then(|t| date(*t)) { out.push(("valid-to", Detail::Str(to))); }
    }
    if let Some(subject) = field(4) { out.push(("subject", Detail::Str(distinguished_name(subject)))); }
    if let Some((name, bits, elliptic)) = field(5).and_then(public_key) {
        out.push(("public-key-algorithm", Detail::Str(name.into())));
        out.push(("certificate-security-level", Detail::Str(security_level(bits, elliptic).into())));
    }
    if let Some(name) = parts.get(1).and_then(|a| signature_name(*a)) { out.push(("signature-algorithm", Detail::Str(name.into()))); }
    if let Some(spki) = field(5) {
        out.push(("public-key-id", Detail::Str(hex(&sha1::Sha1::digest(spki.raw), "sha1:"))));
        out.push(("public-key-id-sha256", Detail::Str(hex(&sha2::Sha256::digest(spki.raw), "sha256:"))));
    }
    out.push(("certificate-id", Detail::Str(hex(&sha1::Sha1::digest(der_cert), "sha1:"))));
    out.push(("pem", Detail::Str(pem(der_cert))));
    out
}

/// The certificate's issuer and subject are one name (check_issuer of a
/// certificate with itself): GNU's :self-signed warning.
pub(crate) fn self_signed(der_cert: &[u8]) -> bool {
    let Some((certificate, _)) = der(der_cert) else { return false };
    let Some(tbs) = elements(certificate.body).into_iter().next() else { return false };
    let fields: Vec<_> = elements(tbs.body).into_iter().filter(|f| f.tag != 0xa0).collect();
    matches!((fields.get(2), fields.get(4)), (Some(i), Some(s)) if i.raw == s.raw)
}

fn pem(der_cert: &[u8]) -> String {
    const ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut text = String::new();
    for chunk in der_cert.chunks(3) {
        let n = chunk.iter().fold(0u32, |a, &b| (a << 8) | u32::from(b)) << (8 * (3 - chunk.len()));
        for i in 0..4 {
            text.push(if i <= chunk.len() { ALPHABET[(n >> (18 - 6 * i) & 63) as usize] as char } else { '=' });
        }
    }
    let lines: Vec<&str> = text.as_bytes().chunks(64).map(|l| std::str::from_utf8(l).expect("base64")).collect();
    format!("-----BEGIN CERTIFICATE-----\n{}\n-----END CERTIFICATE-----\n", lines.join("\n"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn security_levels_follow_gnutls_table() {
        assert_eq!(security_level(256, true), "High");
        assert_eq!(security_level(384, true), "Ultra");
        assert_eq!(security_level(2048, false), "Medium");
        assert_eq!(security_level(4096, false), "High");
        assert_eq!(security_level(1024, false), "Low");
    }

    #[test]
    fn priority_strings_select_versions() {
        assert_eq!(versions(Some("NORMAL:-VERS-TLS1.0:-VERS-TLS1.1")).ok().map(|v| v.len()), Some(2));
        assert_eq!(versions(Some("NORMAL:-VERS-TLS1.3")).ok().map(|v| v.len()), Some(1));
        assert!(matches!(versions(Some("NORMAL:-VERS-ALL")), Err(BootError::Code(code::NO_PRIORITIES_WERE_SET))));
    }

    #[test]
    fn names_escape_as_rfc4514() {
        let value = [0x0c, 3, b'a', b',', b' '];
        assert_eq!(attribute_value(der(&value).unwrap().0), "a\\,\\ ");
        assert_eq!(oid(&[0x55, 0x04, 0x03]), "2.5.4.3");
    }
}
