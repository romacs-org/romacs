//! Keep obsolete frames out of the interactive presentation queue.
//! The frame coordinator still paces work; mailbox replaces pending images
//! instead of making the newest input wait behind them. FIFO is universal.
pub(crate) const MAXIMUM_FRAME_LATENCY: u32 = 1;

pub(crate) fn present_mode(supported: &[wgpu::PresentMode]) -> wgpu::PresentMode {
    let requested = std::env::var("ROMACS_BENCH_PRESENT_MODE").ok();
    select_present_mode(supported, requested.as_deref())
}

fn select_present_mode(
    supported: &[wgpu::PresentMode],
    requested: Option<&str>,
) -> wgpu::PresentMode {
    if let Some(requested) = requested {
        let mode = match requested {
            "fifo" => wgpu::PresentMode::Fifo,
            "mailbox" => wgpu::PresentMode::Mailbox,
            _ => panic!("invalid ROMACS_BENCH_PRESENT_MODE: {requested}"),
        };
        assert!(
            supported.contains(&mode),
            "unsupported benchmark present mode: {mode:?}"
        );
        tracing::info!(?mode, "benchmark present mode selected");
        return mode;
    }
    if supported.contains(&wgpu::PresentMode::Mailbox) {
        wgpu::PresentMode::Mailbox
    } else {
        wgpu::PresentMode::Fifo
    }
}

#[cfg(test)]
mod tests;
