//! Bounded root-pass GPU queries, independent of the optional video crate.
//!
//! GPU ticks are NOT CLOCK_MONOTONIC. Emit a duration and the raw ticks in a
//! separately labelled structured log; never forge local-clock stage stamps.
use romacs_display_protocol::{PresentationId, present_trace};
use std::sync::mpsc::{self, Receiver, SyncSender};

const SLOT_COUNT: usize = 8;
const QUERY_BYTES: u64 = 16;

pub(crate) struct Pending {
    queries: wgpu::QuerySet,
    resolved: wgpu::Buffer,
    readback: wgpu::Buffer,
}

impl Pending {
    pub(crate) fn writes(&self) -> wgpu::RenderPassTimestampWrites<'_> {
        wgpu::RenderPassTimestampWrites {
            query_set: &self.queries,
            beginning_of_pass_write_index: Some(0),
            end_of_pass_write_index: Some(1),
        }
    }

    pub(crate) fn resolve(&self, encoder: &mut wgpu::CommandEncoder) {
        encoder.resolve_query_set(&self.queries, 0..2, &self.resolved, 0);
        encoder.copy_buffer_to_buffer(&self.resolved, 0, &self.readback, 0, QUERY_BYTES);
    }
}

pub(crate) struct RootGpuTimer {
    available: Vec<Pending>,
    returned: Receiver<Pending>,
    return_to: SyncSender<Pending>,
    active: bool,
}

impl RootGpuTimer {
    pub(crate) fn new(device: &wgpu::Device) -> Self {
        let requested = present_trace::enabled()
            && std::env::var_os("ROMACS_GPU_FRAME_TIMING").as_deref()
                == Some(std::ffi::OsStr::new("1"));
        let active = requested && device.features().contains(wgpu::Features::TIMESTAMP_QUERY);
        if requested {
            tracing::warn!(active, "root GPU timestamp diagnostic availability");
        }
        let (return_to, returned) = mpsc::sync_channel(SLOT_COUNT);
        let available = if active {
            (0..SLOT_COUNT)
                .map(|_| Pending {
                    queries: device.create_query_set(&wgpu::QuerySetDescriptor {
                        label: Some("S0 root glyph timestamps"),
                        ty: wgpu::QueryType::Timestamp,
                        count: 2,
                    }),
                    resolved: device.create_buffer(&wgpu::BufferDescriptor {
                        label: Some("S0 root query resolve"),
                        size: QUERY_BYTES,
                        usage: wgpu::BufferUsages::QUERY_RESOLVE | wgpu::BufferUsages::COPY_SRC,
                        mapped_at_creation: false,
                    }),
                    readback: device.create_buffer(&wgpu::BufferDescriptor {
                        label: Some("S0 root query readback"),
                        size: QUERY_BYTES,
                        usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
                        mapped_at_creation: false,
                    }),
                })
                .collect()
        } else {
            Vec::new()
        };
        Self {
            available,
            returned,
            return_to,
            active,
        }
    }

    pub(crate) fn begin(&mut self, device: &wgpu::Device) -> Option<Pending> {
        if !self.active {
            return None;
        }
        // Non-blocking progress only. Never wait for a query or grow the pool.
        let _ = device.poll(wgpu::PollType::Poll);
        self.available.extend(self.returned.try_iter());
        let slot = self.available.pop();
        if slot.is_none() {
            tracing::warn!("root GPU timestamp sample skipped: pool exhausted");
        }
        slot
    }

    /// Call after submitting the encoder containing `pending.resolve()`.
    pub(crate) fn submitted(
        &self,
        pending: Pending,
        queue: &wgpu::Queue,
        frame: u64,
        presentation: PresentationId,
    ) {
        let return_to = self.return_to.clone();
        let period_ns = queue.get_timestamp_period();
        let readback = pending.readback.clone();
        readback
            .slice(..)
            .map_async(wgpu::MapMode::Read, move |result| {
                if result.is_ok() {
                    if let Ok(mapped) = pending.readback.slice(..).get_mapped_range() {
                        let ticks: &[u64] = bytemuck::cast_slice(&mapped);
                        if let Some(duration_ns) = duration_ns(ticks[0], ticks[1], period_ns) {
                            tracing::warn!(
                                target: "romacs_gpu_frame_timing",
                                frame,
                                presentation = presentation.get(),
                                gpu_start_ticks = ticks[0],
                                gpu_end_ticks = ticks[1],
                                timestamp_period_ns = period_ns,
                                duration_ns,
                                "root glyph GPU duration (GPU clock, not CLOCK_MONOTONIC)"
                            );
                        } else {
                            tracing::warn!(
                                frame,
                                presentation = presentation.get(),
                                "invalid root GPU timestamp sample"
                            );
                        }
                    }
                    pending.readback.unmap();
                    let _ = return_to.try_send(pending);
                } else {
                    // Do not recycle a failed mapping into a future query.
                    tracing::warn!(
                        frame,
                        presentation = presentation.get(),
                        "root GPU timestamp readback failed"
                    );
                }
            });
    }
}

fn duration_ns(start: u64, end: u64, period_ns: f32) -> Option<u64> {
    let duration = end.checked_sub(start)? as f64 * f64::from(period_ns);
    (period_ns > 0.0 && duration.is_finite() && duration < u64::MAX as f64)
        .then(|| duration.round() as u64)
}

#[cfg(test)]
mod tests {
    use super::duration_ns;

    #[test]
    fn converts_only_valid_gpu_clock_deltas() {
        assert_eq!(duration_ns(100, 125, 2.0), Some(50));
        assert_eq!(duration_ns(125, 100, 2.0), None);
        assert_eq!(duration_ns(0, 1, f32::NAN), None);
        assert_eq!(duration_ns(0, 1, 0.0), None);
    }
}
