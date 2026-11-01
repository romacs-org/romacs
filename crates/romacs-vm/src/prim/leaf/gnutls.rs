//! GNU gnutls.c's process state for prims/gnutls.rs: `gnutls-boot' and
//! gnutls_verify_boot over a channel's session (crate::tls), the session's
//! stage, and the Lisp values `gnutls-peer-status' reports.
use super::*;
use crate::prim::Out;
use crate::subprocess::Slot;
use crate::tls::{self, BootError, Session, stage};
use std::os::fd::AsRawFd;

type Result<T> = std::result::Result<T, Fault>;

/// What the wait learned of a session still in its handshake.
pub(crate) enum Handshake {
    Pending,
    Ready,
    Failed(i32),
}

impl Leaf<'_> {
    fn plist_value(&self, mut plist: Value, key: Value) -> Value {
        for _ in 0..self.list_limit() {
            let Some((k, rest)) = self.pair(plist) else { break };
            let Some((v, rest)) = self.pair(rest) else { break };
            if k == key { return v; }
            plist = rest;
        }
        Value::NIL
    }
    fn keyword(&mut self, name: &str) -> Value {
        self.intern(name)
    }
    /// The session the process's records go through (p->gnutls_p).
    fn active_session(&mut self, proc: Value) -> Option<&mut Session> {
        self.session_mut(proc).filter(|s| s.active)
    }
    fn session(&self, proc: Value) -> Option<&Session> {
        self.channel(proc)?.tls.as_deref()
    }
    fn session_mut(&mut self, proc: Value) -> Option<&mut Session> {
        self.channel_mut(proc)?.tls.as_deref_mut()
    }
    /// p->is_non_blocking_client: the process was made with :nowait.
    fn non_blocking_client(&mut self, proc: Value) -> Result<bool> {
        let contact = self.process(proc)?.get(Slot::Childp);
        let nowait = self.keyword(":nowait");
        Ok(self.plist_value(contact, nowait) != Value::NIL)
    }
    /// boot_error: a non-blocking client's status becomes (failed
    /// MESSAGE); any other caller gets the error.
    fn boot_error(&mut self, proc: Value, message: String) -> Out {
        if self.non_blocking_client(proc)? {
            let failed = self.sym("failed");
            let text = self.make_string(message.into_bytes(), true);
            let status = self.list(&[failed, text]);
            self.process_mut(proc).set(Slot::Status, status);
            Ok(Value::NIL)
        } else {
            Err(Fault::Error(message))
        }
    }
    /// gnutls_make_error: t for success, the retryable codes' symbols,
    /// else the code.
    pub(crate) fn gnutls_make_error(&mut self, code: i32) -> Value {
        match code {
            0 => Value::TRUE,
            tls::code::AGAIN => self.intern("gnutls-e-again"),
            tls::code::INTERRUPTED => self.intern("gnutls-e-interrupted"),
            -10 => self.intern("gnutls-e-invalid-session"),
            _ => Value::integer(i64::from(code)).expect("gnutls code"),
        }
    }
    /// An error value's GnuTLS code: t is success; a symbol's is its
    /// gnutls-code (the two retryable ones and invalid-session).
    pub(crate) fn gnutls_code(&mut self, err: Value) -> Result<Option<i32>> {
        if err == Value::TRUE { return Ok(Some(0)); }
        if let Some(n) = err.integer_value() { return Ok(i32::try_from(n).ok()); }
        if self.kind(err) == Kind::Symbol {
            return match self.symbol_name(err)?.as_str() {
                "gnutls-e-again" => Ok(Some(tls::code::AGAIN)),
                "gnutls-e-interrupted" => Ok(Some(tls::code::INTERRUPTED)),
                "gnutls-e-invalid-session" => Ok(Some(-10)),
                "gnutls-e-not-ready-for-handshake" => Ok(None),
                _ => Err(Fault::Error("Symbol has no numeric gnutls-code property".into())),
            };
        }
        Err(Fault::Wrong("symbolp", err))
    }

    /// Fgnutls_boot: credentials, the handshake (all of it unless PROC is a
    /// non-blocking client not asked to :complete-negotiation, or still
    /// connecting), then gnutls_verify_boot.
    pub(crate) fn gnutls_boot(&mut self, proc: Value, kind: Value, proplist: Value) -> Out {
        self.process(proc)?;
        if self.kind(kind) != Kind::Symbol { return Err(Fault::Wrong("symbolp", kind)); }
        if !matches!(self.symbol_name(kind)?.as_str(), "gnutls-x509pki" | "gnutls-anon") {
            return self.boot_error(proc, "Invalid GnuTLS credential type".into());
        }
        if self.symbol_name(kind)? == "gnutls-anon" {
            // VM2-TLS1: rustls has no anonymous key exchange.
            return self.boot_error(proc, "VM2 TLS has no anonymous (gnutls-anon) credentials".into());
        }
        let key = self.keyword(":hostname");
        let hostname = self.plist_value(proplist, key);
        let Ok(hostname) = self.string_text(hostname) else {
            return self.boot_error(proc, "gnutls-boot: invalid :hostname parameter (not a string)".into());
        };
        let Some(params) = self.boot_params(proc, hostname, proplist)? else { return Ok(Value::NIL) };
        let mut session = match Session::new(&params) {
            Ok(session) => session,
            Err(BootError::Code(code)) => return Ok(self.gnutls_make_error(code)),
            Err(BootError::Message(message)) => return self.boot_error(proc, message),
        };
        let key = self.keyword(":complete-negotiation");
        let complete = self.plist_value(proplist, key) != Value::NIL;
        let non_blocking = self.non_blocking_client(proc)? && !complete;
        // gnutls_try_handshake: a non-blocking client's records go through
        // the session from its first round.
        session.active = non_blocking;
        let Some(channel) = self.channel_mut(proc) else { return Ok(self.gnutls_make_error(tls::code::PUSH_ERROR)) };
        let Some(fd) = channel.infd.as_ref().map(|f| f.as_raw_fd()) else { return Ok(self.gnutls_make_error(tls::code::PUSH_ERROR)) };
        if channel.connecting.is_some() {
            // The handshake starts once the connection is made (the wait's
            // gnutls_try_handshake).
            session.stage = stage::HANDSHAKE_TRIED;
            session.active = true;
            channel.tls = Some(Box::new(session));
            return Ok(self.gnutls_make_error(tls::code::AGAIN));
        }
        let result = if non_blocking { session.handshake_step(fd) } else { session.handshake(fd).map(|()| true) };
        if result.is_err() { session.stage = stage::HANDSHAKE_TRIED; }
        channel.tls = Some(Box::new(session));
        match result {
            Ok(true) => self.gnutls_verify_boot(proc, proplist),
            Ok(false) => Ok(self.gnutls_make_error(tls::code::AGAIN)),
            Err(code) => Ok(self.gnutls_make_error(code)),
        }
    }
    /// PROPLIST's files and priority, or None after boot_error refused one.
    fn boot_params(&mut self, proc: Value, hostname: String, proplist: Value) -> Result<Option<tls::Params>> {
        let path = |leaf: &Self, v: Value| leaf.string_view(v).ok().map(|s| std::path::PathBuf::from(<std::ffi::OsStr as std::os::unix::ffi::OsStrExt>::from_bytes(s.bytes())));
        let key = self.keyword(":trustfiles");
        let mut trustfiles = Vec::new();
        let mut tail = self.plist_value(proplist, key);
        while let Some((file, rest)) = self.pair(tail) {
            match path(self, file) {
                Some(p) => trustfiles.push(p),
                None => { self.boot_error(proc, "Invalid trustfile".into())?; return Ok(None) }
            }
            tail = rest;
        }
        let key = self.keyword(":crlfiles");
        let mut tail = self.plist_value(proplist, key);
        while let Some((file, rest)) = self.pair(tail) {
            if path(self, file).is_none() { self.boot_error(proc, "Invalid CRL file".into())?; return Ok(None) }
            tail = rest;
        }
        let key = self.keyword(":keylist");
        let mut keylist = Vec::new();
        let mut tail = self.plist_value(proplist, key);
        while let Some((pair, rest)) = self.pair(tail) {
            let (keyfile, more) = self.pair(pair).unwrap_or((Value::NIL, Value::NIL));
            let certfile = self.pair(more).map_or(Value::NIL, |p| p.0);
            match (path(self, keyfile), path(self, certfile)) {
                (Some(k), Some(c)) => keylist.push((k, c)),
                (k, _) => {
                    let message = if k.is_some() { "Invalid client cert file" } else { "Invalid client key file" };
                    self.boot_error(proc, message.into())?;
                    return Ok(None);
                }
            }
            tail = rest;
        }
        let key = self.keyword(":priority");
        let priority = self.plist_value(proplist, key);
        let priority = self.string_text(priority).ok();
        Ok(Some(tls::Params { hostname, trustfiles, keylist, priority }))
    }

    /// gnutls_verify_boot: refuse the peer as :verify-error asks (t, or a
    /// list naming :trustfiles and :hostname), else t. PROPLIST nil means
    /// the process's :tls-parameters.
    pub(crate) fn gnutls_verify_boot(&mut self, proc: Value, proplist: Value) -> Out {
        let proplist = if proplist == Value::NIL {
            let contact = self.process(proc)?.get(Slot::Childp);
            let key = self.keyword(":tls-parameters");
            let params = self.plist_value(contact, key);
            self.pair(params).map_or(Value::NIL, |p| p.1)
        } else { proplist };
        let key = self.keyword(":verify-error");
        let verify_error = self.plist_value(proplist, key);
        let all = verify_error == Value::TRUE;
        if !all && verify_error != Value::NIL && self.pair(verify_error).is_none() {
            return self.boot_error(proc, "gnutls-boot: invalid :verify_error parameter (not a list)".into());
        }
        let key = self.keyword(":hostname");
        let hostname = self.plist_value(proplist, key);
        let Ok(hostname) = self.string_text(hostname) else {
            return self.boot_error(proc, "gnutls-boot: invalid :hostname parameter (not a string)".into());
        };
        let asks = |leaf: &mut Self, name: &str| -> bool {
            let key = leaf.keyword(name);
            let mut tail = verify_error;
            while let Some((item, rest)) = leaf.pair(tail) {
                if item == key { return true; }
                tail = rest;
            }
            false
        };
        let Some(session) = self.session(proc) else { return Ok(self.gnutls_make_error(-10)) };
        let (verification, mismatch, none) = (session.verification, session.host_mismatch, session.certificates.is_empty());
        if verification != 0 && (all || asks(self, ":trustfiles")) {
            self.gnutls_deinit(proc)?;
            return self.boot_error(proc, format!("Certificate validation failed {hostname}, verification code {verification:x}"));
        }
        if none {
            self.gnutls_deinit(proc)?;
            return self.boot_error(proc, "No x509 certificate was found\n".into());
        }
        if mismatch && (all || asks(self, ":hostname")) {
            self.gnutls_deinit(proc)?;
            return self.boot_error(proc, format!("The x509 certificate does not match \"{hostname}\""));
        }
        if let Some(session) = self.session_mut(proc) { session.active = true; }
        Ok(Value::TRUE)
    }

    /// emacs_gnutls_deinit: an active session ends and the process speaks
    /// plain bytes again; nil when none was active.
    pub(crate) fn gnutls_deinit(&mut self, proc: Value) -> Out {
        self.process(proc)?;
        if self.active_session(proc).is_none() { return Ok(Value::NIL); }
        let channel = self.channel_mut(proc).expect("session channel");
        let stage = channel.tls.take().map_or(stage::EMPTY, |s| s.stage);
        channel.tls_stage = if stage >= 4 { 3 } else { stage };
        Ok(Value::TRUE)
    }

    pub(crate) fn gnutls_initstage(&mut self, proc: Value) -> Out {
        self.process(proc)?;
        let stage = match self.channel(proc) {
            Some(channel) => channel.tls.as_deref().map_or(channel.tls_stage, |s| s.stage),
            None => stage::EMPTY,
        };
        Ok(Value::integer(i64::from(stage)).expect("stage"))
    }

    /// Fgnutls_bye: the close_notify; CONT is the same to rustls.
    pub(crate) fn gnutls_bye(&mut self, proc: Value) -> Out {
        self.process(proc)?;
        let Some(channel) = self.channel_mut(proc) else { return Ok(self.gnutls_make_error(-10)) };
        let Some(fd) = channel.outfd.as_ref().map(|f| f.as_raw_fd()) else { return Ok(self.gnutls_make_error(tls::code::PUSH_ERROR)) };
        let Some(session) = channel.tls.as_deref_mut() else { return Ok(self.gnutls_make_error(-10)) };
        let code = if session.bye(fd).is_ok() { 0 } else { tls::code::PUSH_ERROR };
        Ok(self.gnutls_make_error(code))
    }

    /// The wait's gnutls_try_handshake for a non-blocking client's
    /// session once its connection is made.
    pub(crate) fn gnutls_handshake_poll(&mut self, proc: Value) -> Handshake {
        let Some(channel) = self.channel_mut(proc) else { return Handshake::Failed(-10) };
        let Some(fd) = channel.infd.as_ref().map(|f| f.as_raw_fd()) else { return Handshake::Failed(tls::code::PULL_ERROR) };
        let Some(session) = channel.tls.as_deref_mut() else { return Handshake::Failed(-10) };
        match session.handshake_step(fd) {
            Ok(true) => Handshake::Ready,
            Ok(false) => Handshake::Pending,
            Err(code) => Handshake::Failed(code),
        }
    }

    fn certificate_value(&mut self, der: &[u8]) -> Value {
        let mut items = Vec::new();
        for (key, detail) in tls::certificate_details(der) {
            items.push(self.intern(&format!(":{key}")));
            items.push(match detail {
                tls::Detail::Int(n) => Value::integer(n).expect("detail"),
                tls::Detail::Str(s) => self.make_string(s.into_bytes(), true),
            });
        }
        self.list(&items)
    }

    /// Fgnutls_peer_status: nil until the session is READY.
    pub(crate) fn gnutls_peer_status(&mut self, proc: Value) -> Out {
        self.process(proc)?;
        let Some(session) = self.session(proc).filter(|s| s.stage == stage::READY) else { return Ok(Value::NIL) };
        let (verification, mismatch, certificates) = (session.verification, session.host_mismatch, session.certificates.clone());
        let negotiated = session.negotiated();
        // Fgnutls_peer_status conses each warning onto the front.
        let mut warnings = Vec::new();
        for (bit, name) in [(tls::cert::INVALID, ":invalid"), (tls::cert::REVOKED, ":revoked"), (tls::cert::SIGNER_NOT_FOUND, ":unknown-ca"),
                            (tls::cert::SIGNER_NOT_CA, ":not-ca"), (tls::cert::INSECURE_ALGORITHM, ":insecure"),
                            (tls::cert::NOT_ACTIVATED, ":not-activated"), (tls::cert::EXPIRED, ":expired"),
                            (tls::cert::SIGNATURE_FAILURE, ":signature-failure"),
                            (tls::cert::SIGNER_CONSTRAINTS_FAILURE, ":signer-constraints-failure"),
                            (tls::cert::PURPOSE_MISMATCH, ":purpose-mismatch")] {
            if verification & bit != 0 { warnings.insert(0, self.intern(name)); }
        }
        if mismatch { warnings.insert(0, self.intern(":no-host-match")); }
        if certificates.first().is_some_and(|c| tls::self_signed(c)) { warnings.insert(0, self.intern(":self-signed")); }
        let mut result = Vec::new();
        if !warnings.is_empty() {
            result.push(self.intern(":warnings"));
            result.push(self.list(&warnings));
        }
        if !certificates.is_empty() {
            let certs: Vec<Value> = certificates.iter().map(|c| self.certificate_value(c)).collect();
            result.push(self.intern(":certificates"));
            result.push(self.list(&certs));
            result.push(self.intern(":certificate"));
            result.push(certs[0]);
        }
        if let Some((kx, protocol, cipher)) = negotiated {
            for (key, value) in [(":key-exchange", kx), (":protocol", protocol), (":cipher", cipher), (":mac", "AEAD")] {
                result.push(self.intern(key));
                result.push(self.make_string(value.as_bytes().to_vec(), false));
            }
            result.push(self.intern(":encrypt-then-mac"));
            result.push(Value::NIL);
            if protocol == "TLS1.2" {
                result.push(self.intern(":safe-renegotiation"));
                result.push(Value::TRUE);
            }
        }
        Ok(self.list(&result))
    }
}
