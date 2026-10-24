//! Test-only path aliases so the test modules that moved here from
//! romacs-core compile with their bodies byte-identical: they name
//! `crate::emacs_core::regex_emacs::X`, `crate::emacs_core::syntax::X`,
//! `crate::test_utils::init_test_tracing`.

pub(crate) mod emacs_core {
    pub(crate) use crate::engine as regex_emacs;

    pub(crate) mod syntax {
        pub(crate) use crate::SyntaxClass;
    }
}

pub(crate) mod test_utils {
    /// romacs-core's `init_test_tracing`: a test subscriber honouring
    /// `RUST_LOG`, defaulting to `warn`.
    pub(crate) fn init_test_tracing() {
        let _ = tracing_subscriber::fmt()
            .with_env_filter(
                tracing_subscriber::EnvFilter::try_from_default_env()
                    .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("warn")),
            )
            .with_test_writer()
            .try_init();
    }
}
