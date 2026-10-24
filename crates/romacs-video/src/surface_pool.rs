use std::collections::VecDeque;
use std::sync::{Arc, Mutex, MutexGuard};

/// Bounded pool whose checked-out values are affine leases. Values return to
/// the pool only when their lease drops, which lets the renderer tie reuse to
/// GPU submission retirement without exposing platform handles publicly.
pub(crate) struct BoundedSurfacePool<K, V> {
    shared: Arc<Mutex<SurfacePoolState<K, V>>>,
}

struct SurfacePoolState<K, V> {
    capacity: usize,
    /// Slots occupied by live values or by in-progress allocations.
    allocated: usize,
    reservations: usize,
    allocations: u64,
    reuses: u64,
    backpressured_acquires: u64,
    in_flight_high_water: usize,
    /// Idle entries in least-recently-returned order. Decoder surface pools
    /// rotate through several stable identities, so a miss is not evidence
    /// that the other identities are stale.
    idle: VecDeque<(K, V)>,
}

pub(crate) enum SurfacePoolAcquire<K, V> {
    Reused(SurfaceLease<K, V>),
    Allocate(SurfaceReservation<K, V>),
    Backpressured,
}

pub(crate) struct SurfaceReservation<K, V> {
    key: Option<K>,
    shared: Arc<Mutex<SurfacePoolState<K, V>>>,
}

pub(crate) struct SurfaceLease<K, V> {
    entry: Option<(K, V)>,
    shared: Arc<Mutex<SurfacePoolState<K, V>>>,
}

impl<K, V> BoundedSurfacePool<K, V>
where
    K: Eq,
{
    pub(crate) fn new(capacity: usize) -> Self {
        assert!(capacity > 0, "a surface pool must retain at least one slot");
        Self {
            shared: Arc::new(Mutex::new(SurfacePoolState {
                capacity,
                allocated: 0,
                reservations: 0,
                allocations: 0,
                reuses: 0,
                backpressured_acquires: 0,
                in_flight_high_water: 0,
                idle: VecDeque::with_capacity(capacity),
            })),
        }
    }

    pub(crate) fn acquire(&self, key: K) -> SurfacePoolAcquire<K, V> {
        let mut key = Some(key);
        loop {
            let mut state = lock_unpoisoned(&self.shared);
            let wanted = key.as_ref().expect("surface key is consumed once");
            if let Some(index) = state
                .idle
                .iter()
                .position(|(candidate, _)| candidate == wanted)
            {
                let entry = state
                    .idle
                    .remove(index)
                    .expect("the matching idle surface index remains valid");
                state.reuses = state.reuses.saturating_add(1);
                let in_flight = state
                    .allocated
                    .saturating_sub(state.reservations)
                    .saturating_sub(state.idle.len());
                state.in_flight_high_water = state.in_flight_high_water.max(in_flight);
                return SurfacePoolAcquire::Reused(SurfaceLease {
                    entry: Some(entry),
                    shared: Arc::clone(&self.shared),
                });
            }

            if state.allocated < state.capacity {
                state.allocated += 1;
                state.reservations += 1;
                return SurfacePoolAcquire::Allocate(SurfaceReservation {
                    key: key.take(),
                    shared: Arc::clone(&self.shared),
                });
            }

            // The pool is full. Evict exactly one least-recently-returned
            // idle identity outside the lock, then retry. Heterogeneous idle
            // identities remain cached while spare capacity exists.
            if let Some(stale) = state.idle.pop_front() {
                state.allocated -= 1;
                drop(state);
                drop(stale);
                continue;
            }

            state.backpressured_acquires = state.backpressured_acquires.saturating_add(1);
            return SurfacePoolAcquire::Backpressured;
        }
    }

    pub(crate) fn diagnostics(
        &self,
        role: crate::VideoSurfacePoolRole,
    ) -> crate::VideoSurfacePoolDiagnostics {
        let state = lock_unpoisoned(&self.shared);
        let idle = state.idle.len();
        let allocated = state.allocated.saturating_sub(state.reservations);
        crate::VideoSurfacePoolDiagnostics {
            role,
            capacity: state.capacity,
            allocated,
            idle,
            in_flight: allocated.saturating_sub(idle),
            allocations: state.allocations,
            reuses: state.reuses,
            backpressured_acquires: state.backpressured_acquires,
            in_flight_high_water: state.in_flight_high_water,
        }
    }

    /// Start a new observation epoch without invalidating reusable surfaces.
    /// Occupancy is state, so the new high-water mark starts at the number of
    /// leases already checked out at the acknowledged boundary.
    pub(crate) fn begin_measurement_epoch(&self) {
        let mut state = lock_unpoisoned(&self.shared);
        let allocated = state.allocated.saturating_sub(state.reservations);
        let in_flight = allocated.saturating_sub(state.idle.len());
        state.allocations = 0;
        state.reuses = 0;
        state.backpressured_acquires = 0;
        state.in_flight_high_water = in_flight;
    }
}

impl<K, V> SurfaceReservation<K, V> {
    pub(crate) fn fulfill(mut self, value: V) -> SurfaceLease<K, V> {
        let key = self
            .key
            .take()
            .expect("a surface reservation can only be fulfilled once");
        {
            let mut state = lock_unpoisoned(&self.shared);
            state.reservations -= 1;
            state.allocations = state.allocations.saturating_add(1);
            let in_flight = state
                .allocated
                .saturating_sub(state.reservations)
                .saturating_sub(state.idle.len());
            state.in_flight_high_water = state.in_flight_high_water.max(in_flight);
        }
        SurfaceLease {
            entry: Some((key, value)),
            shared: Arc::clone(&self.shared),
        }
    }
}

impl<K, V> Drop for SurfaceReservation<K, V> {
    fn drop(&mut self) {
        if self.key.is_some() {
            let mut state = lock_unpoisoned(&self.shared);
            state.allocated -= 1;
            state.reservations -= 1;
        }
    }
}

impl<K, V> SurfaceLease<K, V> {
    pub(crate) fn value(&self) -> &V {
        &self
            .entry
            .as_ref()
            .expect("a live surface lease owns one entry")
            .1
    }
}

impl<K, V> Drop for SurfaceLease<K, V> {
    fn drop(&mut self) {
        let entry = self
            .entry
            .take()
            .expect("a surface lease returns its entry exactly once");
        lock_unpoisoned(&self.shared).idle.push_back(entry);
    }
}

fn lock_unpoisoned<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    match mutex.lock() {
        Ok(guard) => guard,
        Err(poisoned) => poisoned.into_inner(),
    }
}
