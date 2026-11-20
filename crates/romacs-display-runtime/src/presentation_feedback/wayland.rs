use romacs_display_protocol::frame_time::{EventTime, observe_platform_now};
use std::{collections::VecDeque, fs, path::PathBuf, time::Duration};

use wayland_client::globals::{GlobalListContents, registry_queue_init};
use wayland_client::protocol::wl_registry;
use wayland_client::{Connection, Dispatch, EventQueue, QueueHandle};
use wayland_protocols::wp::presentation_time::client::{wp_presentation, wp_presentation_feedback};
use winit::event_loop::OwnedDisplayHandle;
use winit::window::Window;

use crate::wayland_guest::Guest;

#[derive(Clone, Copy, Debug)]
struct Submission {
    serial: u64,
    frame: u64,
    layout: romacs_display_protocol::PresentationId,
    draw_seq: Option<u64>,
    width: u32,
    height: u32,
    scale: f64,
}

#[derive(Clone, Copy, Debug)]
struct CompositorTimestamp {
    // Deliberately not EventTime: this clock belongs to the compositor and
    // must not be substituted for a scheduler observation or predicted tick.
    clock_id: u32,
    seconds: u64,
    nanoseconds: u32,
}

#[derive(Clone, Copy, Debug)]
struct ConfirmedPresentation {
    submission: Submission,
    timestamp: CompositorTimestamp,
}

#[derive(Clone, Copy, Debug)]
enum NativeFeedback {
    Presented(ConfirmedPresentation),
    Discarded(Submission),
}

#[derive(Default)]
struct PendingReceipts(VecDeque<(u64, EventTime)>);
impl PendingReceipts {
    fn requested(&mut self, serial: u64, now: EventTime) {
        if self.0.len() == 256 {
            self.0.pop_front();
        }
        self.0.push_back((serial, now));
    }
    fn received(&mut self, serial: u64) {
        self.0.retain(|&(id, _)| id != serial);
    }
    fn deadline(&mut self, now: EventTime) -> Option<EventTime> {
        // Missing feedback from an occluded/destroyed surface must not keep
        // diagnostics waking forever. This never asks for another frame.
        self.0
            .retain(|&(_, requested)| now.saturating_since(requested) < Duration::from_secs(1));
        (!self.0.is_empty()).then(|| now.plus(Duration::from_millis(4)))
    }
}

struct Receipts {
    path: Option<PathBuf>,
    latest_presented: u64,
    clock_id: Option<u32>,
    pending: PendingReceipts,
}

impl Receipts {
    fn observe(&mut self, feedback: NativeFeedback) {
        let serial = match feedback {
            NativeFeedback::Presented(confirmed) => confirmed.submission.serial,
            NativeFeedback::Discarded(submission) => submission.serial,
        };
        self.pending.received(serial);
        let confirmed = match feedback {
            NativeFeedback::Presented(confirmed) => confirmed,
            NativeFeedback::Discarded(submission) => {
                romacs_display_protocol::present_trace::record_draw(
                    romacs_display_protocol::present_trace::Stage::Discarded,
                    submission.frame,
                    submission.layout,
                    submission.draw_seq,
                );
                tracing::debug!(?submission, "native presentation discarded");
                return;
            }
        };
        self.publish(confirmed);
    }

    // Raw submissions cannot cross this interface. Only the native Presented
    // event constructs a confirmation with the compositor's timestamp.
    fn publish(&mut self, confirmed: ConfirmedPresentation) {
        tracing::debug!(?confirmed, "native presentation confirmed");
        let ConfirmedPresentation {
            submission,
            timestamp,
        } = confirmed;
        if let Some(nanoseconds) = timestamp
            .seconds
            .checked_mul(1_000_000_000)
            .and_then(|seconds| seconds.checked_add(u64::from(timestamp.nanoseconds)))
        {
            romacs_display_protocol::present_trace::presented_draw(
                submission.frame,
                submission.layout,
                timestamp.clock_id,
                nanoseconds,
                submission.draw_seq,
            );
        }
        let Some(path) = &self.path else { return };
        if submission.serial <= self.latest_presented {
            return;
        }
        // Atomic replacement prevents the independent Lisp test reader from
        // seeing half a receipt. Discarded submissions never advance readiness.
        let receipt = format!(
            "(:submission {} :frame {} :presentation {} :width {} :height {} :scale {} :outcome presented :clock-id {} :seconds {} :nanoseconds {})\n",
            submission.serial,
            submission.frame,
            submission.layout.get(),
            submission.width,
            submission.height,
            submission.scale,
            timestamp.clock_id,
            timestamp.seconds,
            timestamp.nanoseconds
        );
        let temporary = path.with_extension("pending");
        match fs::write(&temporary, receipt).and_then(|()| fs::rename(&temporary, path)) {
            Ok(()) => self.latest_presented = submission.serial,
            Err(error) => tracing::warn!(%error, "cannot write native presentation receipt"),
        }
    }
}

impl Dispatch<wl_registry::WlRegistry, GlobalListContents> for Receipts {
    fn event(
        _: &mut Self,
        _: &wl_registry::WlRegistry,
        _: wl_registry::Event,
        _: &GlobalListContents,
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
    }
}
impl Dispatch<wp_presentation::WpPresentation, ()> for Receipts {
    fn event(
        state: &mut Self,
        _: &wp_presentation::WpPresentation,
        event: wp_presentation::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        if let wp_presentation::Event::ClockId { clk_id } = event {
            state.clock_id = Some(clk_id);
        }
    }
}

impl Dispatch<wp_presentation_feedback::WpPresentationFeedback, Submission> for Receipts {
    fn event(
        state: &mut Self,
        _: &wp_presentation_feedback::WpPresentationFeedback,
        event: wp_presentation_feedback::Event,
        submission: &Submission,
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        match event {
            wp_presentation_feedback::Event::Presented {
                tv_sec_hi,
                tv_sec_lo,
                tv_nsec,
                ..
            } => {
                let Some(clock_id) = state.clock_id else {
                    tracing::warn!("presentation timestamp arrived without its clock domain");
                    return;
                };
                if tv_nsec >= 1_000_000_000 {
                    tracing::warn!(tv_nsec, "invalid compositor presentation timestamp");
                    return;
                }
                state.observe(NativeFeedback::Presented(ConfirmedPresentation {
                    submission: *submission,
                    timestamp: CompositorTimestamp {
                        clock_id,
                        seconds: (u64::from(tv_sec_hi) << 32) | u64::from(tv_sec_lo),
                        nanoseconds: tv_nsec,
                    },
                }));
            }
            wp_presentation_feedback::Event::Discarded => {
                state.observe(NativeFeedback::Discarded(*submission))
            }
            _ => {}
        }
    }
}

struct Session {
    guest: Guest,
    queue: EventQueue<Receipts>,
    receipts: Receipts,
    presentation: wp_presentation::WpPresentation,
    next_submission: u64,
}

impl Session {
    fn connect(display: OwnedDisplayHandle, path: Option<PathBuf>) -> Result<Self, String> {
        let guest =
            Guest::new(display).ok_or("presentation receipts require a Wayland display")?;
        let (globals, queue) =
            registry_queue_init::<Receipts>(guest.connection()).map_err(|error| error.to_string())?;
        let presentation = globals
            .bind(&queue.handle(), 1..=1, ())
            .map_err(|error| error.to_string())?;
        Ok(Self {
            guest,
            queue,
            receipts: Receipts {
                path,
                latest_presented: 0,
                clock_id: None,
                pending: PendingReceipts::default(),
            },
            presentation,
            next_submission: 0,
        })
    }

    fn request(
        &mut self,
        window: &dyn Window,
        frame: u64,
        layout: romacs_display_protocol::PresentationId,
        size: (u32, u32),
        scale: f64,
    ) -> Result<(), String> {
        let surface = self.guest.surface(window)?;
        self.next_submission += 1;
        let submission = Submission {
            serial: self.next_submission,
            draw_seq: romacs_display_protocol::present_trace::current_draw(),
            frame,
            layout,
            width: size.0,
            height: size.1,
            scale,
        };
        self.receipts
            .pending
            .requested(submission.serial, observe_platform_now());
        self.presentation
            .feedback(&surface, &self.queue.handle(), submission);
        tracing::debug!(?submission, "requested native presentation feedback");
        Ok(())
    }
}

enum ObserverState {
    Disabled,
    Uninitialized(Option<PathBuf>),
    Active(Session),
}

pub(crate) struct PresentationObserver {
    state: ObserverState,
    /// Winit's display, once the event loop gives it (`attach`).
    display: Option<OwnedDisplayHandle>,
}

fn initial_state(path: Option<PathBuf>, trace_enabled: impl FnOnce() -> bool) -> ObserverState {
    match path {
        Some(path) => ObserverState::Uninitialized(Some(path)),
        None if trace_enabled() => ObserverState::Uninitialized(None),
        None => ObserverState::Disabled,
    }
}

impl PresentationObserver {
    pub(crate) fn new() -> Self {
        Self {
            state: initial_state(
                std::env::var_os("ROMACS_GUI_PRESENTATION_RECEIPT").map(PathBuf::from),
                romacs_display_protocol::present_trace::enabled,
            ),
            display: None,
        }
    }

    /// Where the session connects when the first frame presents.
    pub(crate) fn attach(&mut self, display: OwnedDisplayHandle) {
        self.display.get_or_insert(display);
    }

    pub(crate) fn before_present(
        &mut self,
        window: &dyn Window,
        frame: u64,
        layout: romacs_display_protocol::PresentationId,
        size: (u32, u32),
        scale: f64,
    ) {
        if matches!(self.state, ObserverState::Uninitialized(_))
            && let Some(display) = self.display.clone()
        {
            let ObserverState::Uninitialized(path) =
                std::mem::replace(&mut self.state, ObserverState::Disabled)
            else {
                unreachable!()
            };
            match Session::connect(display, path) {
                Ok(session) => self.state = ObserverState::Active(session),
                Err(error) => tracing::warn!(%error, "native presentation feedback unavailable"),
            }
        }
        if let ObserverState::Active(session) = &mut self.state
            && let Err(error) = session.request(window, frame, layout, size, scale)
        {
            tracing::warn!(%error, "cannot request native presentation feedback");
        }
    }

    pub(crate) fn dispatch_deadline(&mut self, now: EventTime) -> Option<EventTime> {
        match &mut self.state {
            ObserverState::Active(session) => session.receipts.pending.deadline(now),
            _ => None,
        }
    }

    pub(crate) fn dispatch_pending(&mut self) {
        if let ObserverState::Active(session) = &mut self.state {
            // Winit reads the shared display. Dispatch only our own queue; no
            // roundtrip, blocking read, redraw request, or parent commit here.
            if let Err(error) = session.queue.dispatch_pending(&mut session.receipts) {
                tracing::warn!(%error, "native presentation feedback dispatch failed");
            }
        }
    }

    pub(crate) fn shutdown(&mut self) {
        if let ObserverState::Active(mut session) =
            std::mem::replace(&mut self.state, ObserverState::Disabled)
        {
            let _ = session.queue.dispatch_pending(&mut session.receipts);
            session.presentation.destroy();
            let _ = session.guest.connection().flush();
        }
    }
}

#[cfg(test)]
mod tests;
