    #[cfg(test)]
    fn capture_count(&self) -> u32 {
        // UNSAFE(the reference query exposes its capture table through C)
        // SAFETY: the wrapper owns a live TSQuery for this borrow.
        unsafe { ffi::ts_query_capture_count(self.0) }
    }
