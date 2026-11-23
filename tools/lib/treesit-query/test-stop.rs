    #[cfg(test)]
    pub(crate) fn with_test_signal(quit: std::sync::Arc<QuitSignal>) -> Self {
        Self { quit: Some(quit), latched: false, debug_events: 0 }
    }
