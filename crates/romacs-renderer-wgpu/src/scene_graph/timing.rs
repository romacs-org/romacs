//! GPU timestamps around each target's render pass.
//!
//! Active only when the device has `TIMESTAMP_QUERY` (the renderer requests
//! it under `ROMACS_GPU_FRAME_TIMING=1`). One readback is in flight at a
//! time; a render while it is still mapping goes untimed rather than
//! waiting. GPU ticks are not a host clock: only durations leave here.

use super::draw::TargetId;
use std::sync::{Arc, Mutex};

/// Targets one render can time; a longer list goes untimed.
const MAX_TARGETS: u32 = 32;

/// GPU time one target's pass took (MSAA resolve included).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TargetTiming {
    pub target: TargetId,
    pub gpu_ns: u64,
}

enum State {
    Idle,
    Recorded(Vec<TargetId>),
    Mapping,
    Ready(Vec<TargetTiming>),
}

pub(super) struct SceneTimer {
    queries: wgpu::QuerySet,
    resolved: wgpu::Buffer,
    readback: wgpu::Buffer,
    period_ns: f32,
    state: Arc<Mutex<State>>,
}

impl SceneTimer {
    pub fn new(device: &wgpu::Device, queue: &wgpu::Queue) -> Option<Self> {
        if !device.features().contains(wgpu::Features::TIMESTAMP_QUERY) {
            return None;
        }
        let bytes = u64::from(MAX_TARGETS) * 2 * 8;
        Some(Self {
            queries: device.create_query_set(&wgpu::QuerySetDescriptor {
                label: Some("scene pass timestamps"),
                ty: wgpu::QueryType::Timestamp,
                count: MAX_TARGETS * 2,
            }),
            resolved: device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("scene timestamp resolve"),
                size: bytes,
                usage: wgpu::BufferUsages::QUERY_RESOLVE | wgpu::BufferUsages::COPY_SRC,
                mapped_at_creation: false,
            }),
            readback: device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("scene timestamp readback"),
                size: bytes,
                usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
                mapped_at_creation: false,
            }),
            period_ns: queue.get_timestamp_period(),
            state: Arc::new(Mutex::new(State::Idle)),
        })
    }

    /// Whether this render can be timed; true claims the query slots.
    pub fn begin(&self, targets: usize) -> bool {
        let free = matches!(*self.lock(), State::Idle | State::Ready(_));
        free && targets as u32 <= MAX_TARGETS
    }

    pub fn writes(&self, index: usize) -> wgpu::RenderPassTimestampWrites<'_> {
        let base = index as u32 * 2;
        wgpu::RenderPassTimestampWrites {
            query_set: &self.queries,
            beginning_of_pass_write_index: Some(base),
            end_of_pass_write_index: Some(base + 1),
        }
    }

    pub fn resolve(&self, encoder: &mut wgpu::CommandEncoder, targets: Vec<TargetId>) {
        let count = targets.len() as u32 * 2;
        encoder.resolve_query_set(&self.queries, 0..count, &self.resolved, 0);
        encoder.copy_buffer_to_buffer(&self.resolved, 0, &self.readback, 0, u64::from(count) * 8);
        *self.lock() = State::Recorded(targets);
    }

    /// Call after the encoder holding [`Self::resolve`] was submitted.
    pub fn submitted(&self) {
        let targets = {
            let mut state = self.lock();
            match std::mem::replace(&mut *state, State::Mapping) {
                State::Recorded(targets) => targets,
                other => {
                    *state = other;
                    return;
                }
            }
        };
        let state = self.state.clone();
        let readback = self.readback.clone();
        let period_ns = self.period_ns;
        let bytes = targets.len() as u64 * 16;
        self.readback
            .slice(..bytes)
            .map_async(wgpu::MapMode::Read, move |result| {
                let timings = result.ok().and_then(|()| {
                    let mapped = readback.slice(..bytes).get_mapped_range().ok()?;
                    let ticks: &[u64] = bytemuck::cast_slice(&mapped);
                    Some(durations(&targets, ticks, period_ns))
                });
                readback.unmap();
                let mut state = state.lock().unwrap_or_else(|e| e.into_inner());
                *state = timings.map_or(State::Idle, State::Ready);
            });
    }

    /// The latest completed timings, once.
    pub fn take(&self) -> Option<Vec<TargetTiming>> {
        let mut state = self.lock();
        match std::mem::replace(&mut *state, State::Idle) {
            State::Ready(timings) => Some(timings),
            other => {
                *state = other;
                None
            }
        }
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, State> {
        self.state.lock().unwrap_or_else(|e| e.into_inner())
    }
}

fn durations(targets: &[TargetId], ticks: &[u64], period_ns: f32) -> Vec<TargetTiming> {
    targets
        .iter()
        .zip(ticks.chunks_exact(2))
        .filter_map(|(target, pair)| {
            let ns = pair[1].checked_sub(pair[0])? as f64 * f64::from(period_ns);
            (period_ns > 0.0 && ns.is_finite()).then(|| TargetTiming {
                target: *target,
                gpu_ns: ns.round() as u64,
            })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn durations_scale_ticks_and_drop_inverted_pairs() {
        let targets = [TargetId(1), TargetId(2)];
        assert_eq!(
            durations(&targets, &[10, 20, 50, 40], 2.0),
            vec![TargetTiming {
                target: TargetId(1),
                gpu_ns: 20
            }]
        );
        assert!(durations(&targets, &[0, 1, 0, 1], 0.0).is_empty());
    }
}
