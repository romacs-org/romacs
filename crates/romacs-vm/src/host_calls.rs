//! The calls the evaluator makes out of the VM, counted by name and by
//! side (`romacs--host-calls`): each method of the four host traits, each
//! host.rs result the flight recorder's funnels see, and each call the
//! evaluator's thread makes on the paint sender (romacs-display-runtime
//! counts those, in romacs-display-protocol's `call_count`).
//!
//! A call's side says where it would land if the evaluator ran in its own
//! process with layout and fonts beside it: `Local` when the GUI's
//! implementation touches only state the evaluator's thread owns, `Cross`
//! when it reaches another thread or a descriptor the display side would
//! own, and `Through` for a trait method whose crossings are the paint
//! sender's calls it makes, counted under their own names, so a sum of
//! `Cross` counts each crossing once. The sides are read from
//! romacs-vm-gui's implementations (`SIDES`); a name the table lacks is a
//! host.rs result, which crosses unless it is the process's own clock,
//! identity or environment (`OWN`).
//!
//! The counting hosts wrap the embedder's at attachment, so every caller
//! is counted, present or future. `every_trait_method_is_counted` holds
//! the wrappers to the traits: a method they do not forward would run the
//! trait's default instead of the embedder's.
use crate::{DisplayHost, FaceFont, FontEntity, FontQuery, GlyphMetrics, HostEvent, HostFrame, HostInput, ImageFacts, ImageQuery,
            LayoutHost, NativeWindow, OpenedFont, TerminalHost, TerminalInputState, TerminalSnapshot, WindowEnd};
use romacs_layout_input::{FontsetState, FrameSnapshot, MeasureInput, Measured, OpenedDefFont};
use std::time::Duration;

/// Where a call would land with the evaluator in its own process.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Side { Local, Cross, Through }

use Side::{Cross, Local, Through};

/// Each counted trait method and loop call with its side, by what
/// romacs-vm-gui's implementation touches.
const SIDES: &[(&str, Side)] = &[
    // measure.rs: the layout engine, an Rc the evaluator's thread owns.
    ("layout.measure", Local), ("layout.lay_out", Local),
    // fonts.rs: a constant, the process environment, and the display's own
    // font resolver and tables.
    ("display.window_system", Local), ("display.display_name", Local), ("display.list_fonts", Local),
    ("display.family_fonts", Local), ("display.font_families", Local), ("display.open_font", Local),
    ("display.char_font", Local), ("display.glyph_metrics", Local), ("display.clear_font_cache", Local),
    ("display.set_fontsets", Local), ("display.opened_def_font", Local),
    // main.rs: hooks that call the paint sender.
    ("display.set_frame_alpha", Through), ("display.set_visual_config", Through), ("display.lose_device", Through),
    // fonts.rs: the decoder and channels shared with the decode worker
    // (`finish_decode` blocks on its results), and image files read
    // outside host.rs.
    ("display.decode_image", Cross), ("display.release_image", Cross), ("display.begin_decode", Cross),
    ("display.finish_decode", Cross), ("display.finished_decodes", Cross), ("display.image_header_size", Cross),
    // prompt.rs: each calls the paint sender for the render thread's
    // input channel, paint channel, rendered state, clipboard, adapter,
    // window or pointer.
    ("input.input_pending", Through), ("input.key_pending", Through), ("input.redisplay", Through), ("input.next_event", Through),
    ("input.clipboard_text", Through), ("input.set_clipboard_text", Through), ("input.display_animates", Through),
    ("input.native_window", Through), ("input.pointer", Through),
    // prompt.rs: a list kept for the next wait, and the images' own flag.
    ("input.watch_processes", Local), ("input.images_behind", Local),
    // terminal_service.rs: each session's terminal is locked against its
    // pty thread, and the mailbox against every session's.
    ("terminal.create", Cross), ("terminal.send", Cross), ("terminal.resize", Cross), ("terminal.input_state", Cross),
    ("terminal.changed_sessions", Cross), ("terminal.snapshot", Cross), ("terminal.close", Cross),
    // paint_host.rs, one-way commands queued for the render thread:
    ("sender.submit_scene", Cross), ("sender.load_image", Cross), ("sender.retire_image", Cross),
    ("sender.debug_lose_device", Cross), ("sender.set_frame_alpha", Cross), ("sender.set_title", Cross),
    ("sender.hide_primary", Cross), ("sender.set_visual_config", Cross), ("sender.raise_primary", Cross),
    ("sender.shutdown", Cross),
    // requests that block for the render thread's answer (a rendered
    // presentation, the clipboard, a window made):
    ("sender.wait_for_render", Cross), ("sender.clipboard_text", Cross), ("sender.set_clipboard_text", Cross),
    ("sender.show_primary", Cross),
    // and reads of state the render thread writes: its input channel,
    // the latest rendered presentation, the lost images, the adapter and
    // the pointer.
    ("sender.input", Cross), ("sender.render_poll", Cross), ("sender.take_lost_images", Cross),
    ("sender.images_reset_pending", Cross), ("sender.software_adapter", Cross), ("sender.pointer", Cross),
];

/// The host.rs results that are the process's own: its clocks, counters,
/// identity, entropy and environment.
const OWN: &[&str] = &["monotonic", "clock", "boot-clock", "run-time", "instructions", "tid", "pid", "ppid", "processors",
                       "entropy", "getenv", "environment", "cwd", "umask"];

/// NAME's side.
#[must_use]
pub fn side(name: &str) -> Side {
    match SIDES.iter().find(|(known, _)| *known == name) {
        Some(&(_, side)) => side,
        None if OWN.contains(&name) => Local,
        None => Cross,
    }
}

pub(crate) use romacs_display_protocol::call_count::count;

/// This thread's counts so far, by name.
#[must_use]
pub fn counts() -> Vec<(&'static str, Side, u64)> {
    romacs_display_protocol::call_count::counts().into_iter().map(|(name, n)| (name, side(name), n)).collect()
}

impl crate::Vm {
    /// `romacs--host-calls`: ((NAME SIDE COUNT)...), SIDE `local`, `cross` or `through`.
    pub(crate) fn host_calls(&mut self) -> crate::Result<crate::Value> {
        let mut list = crate::Value::NIL;
        for (name, side, n) in counts().into_iter().rev() {
            let n = crate::Value::integer(i64::try_from(n).map_err(|e| e.to_string())?)?;
            let side = self.intern(match side { Local => "local", Cross => "cross", Through => "through" });
            let name = self.intern(name);
            let mut row = self.keymap_cons(n, crate::Value::NIL);
            row = self.keymap_cons(side, row);
            row = self.keymap_cons(name, row);
            list = self.keymap_cons(row, list);
        }
        Ok(list)
    }
}

/// A host that counts each call before the embedder's own answers it.
pub(crate) struct Counted<H>(pub H);

/// The `&mut self` methods of a trait, each counted as PREFIX.NAME and
/// forwarded.
macro_rules! counted {
    ($prefix:literal; $(fn $name:ident(&mut self $(, $arg:ident: $ty:ty)*) $(-> $ret:ty)?;)*) => {
        $(fn $name(&mut self $(, $arg: $ty)*) $(-> $ret)? {
            count(concat!($prefix, ".", stringify!($name)));
            self.0.$name($($arg),*)
        })*
    };
}

impl LayoutHost for Counted<Box<dyn LayoutHost>> {
    counted! { "layout";
        fn measure(&mut self, input: &MeasureInput) -> Result<Measured, String>;
        fn lay_out(&mut self, frame: &FrameSnapshot) -> Result<Vec<WindowEnd>, String>;
    }
}

impl DisplayHost for Counted<Box<dyn DisplayHost>> {
    fn window_system(&self) -> &str { count("display.window_system"); self.0.window_system() }
    fn display_name(&self) -> String { count("display.display_name"); self.0.display_name() }
    counted! { "display";
        fn list_fonts(&mut self, query: &FontQuery) -> Vec<FontEntity>;
        fn family_fonts(&mut self, family: Option<&str>) -> std::rc::Rc<Vec<FontEntity>>;
        fn font_families(&mut self) -> Vec<String>;
        fn open_font(&mut self, entity: &FontEntity, pixel_size: i64) -> Option<OpenedFont>;
        fn char_font(&mut self, face: &FaceFont, ch: char) -> Option<(OpenedFont, u32)>;
        fn glyph_metrics(&mut self, font: &OpenedFont, ch: char) -> Option<GlyphMetrics>;
        fn clear_font_cache(&mut self);
        fn set_fontsets(&mut self, state: std::sync::Arc<FontsetState>);
        fn opened_def_font(&mut self, font: &OpenedDefFont) -> Option<OpenedFont>;
        fn set_frame_alpha(&mut self, frame: u64, alpha: [f32; 2], limit: f32);
        fn set_visual_config(&mut self, config: &romacs_display_protocol::VisualConfig);
        fn lose_device(&mut self) -> bool;
        fn decode_image(&mut self, query: ImageQuery) -> Result<ImageFacts, String>;
        fn release_image(&mut self, id: u64);
        fn begin_decode(&mut self, query: ImageQuery) -> Result<ImageFacts, String>;
        fn finish_decode(&mut self, id: u64, quit: &dyn Fn() -> bool) -> Result<ImageFacts, String>;
        fn finished_decodes(&mut self) -> Vec<(u64, Result<ImageFacts, String>)>;
        fn image_header_size(&mut self, path: &str) -> Option<(u32, u32)>;
    }
}

impl HostInput for Counted<Box<dyn HostInput>> {
    counted! { "input";
        fn input_pending(&mut self) -> bool;
        fn key_pending(&mut self) -> bool;
        fn redisplay(&mut self, frame: HostFrame) -> Result<Vec<WindowEnd>, String>;
        fn next_event(&mut self, timeout: Option<Duration>) -> Result<HostEvent, String>;
        fn watch_processes(&mut self, fds: &[(std::os::fd::RawFd, i16)]);
        fn images_behind(&mut self) -> bool;
        fn clipboard_text(&mut self) -> Result<Option<String>, String>;
        fn set_clipboard_text(&mut self, text: &str) -> Result<(), String>;
        fn display_animates(&mut self) -> Option<bool>;
        fn native_window(&mut self, action: NativeWindow) -> Option<bool>;
        fn pointer(&mut self) -> Option<(i32, i32)>;
    }
}

impl TerminalHost for Counted<Box<dyn TerminalHost>> {
    counted! { "terminal";
        fn create(&mut self, shell: &str, directory: &str, columns: u16, rows: u16) -> Result<u64, String>;
        fn send(&mut self, session: u64, bytes: &[u8]) -> Result<(), String>;
        fn resize(&mut self, session: u64, columns: u16, rows: u16) -> Result<(), String>;
        fn input_state(&mut self, session: u64) -> Result<TerminalInputState, String>;
        fn changed_sessions(&mut self) -> Vec<u64>;
        fn snapshot(&mut self, session: u64) -> Result<TerminalSnapshot, String>;
        fn close(&mut self, session: u64) -> Result<(), String>;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The methods of `pub trait NAME` in SOURCE.
    fn trait_methods(source: &str, name: &str) -> Vec<String> {
        let body = source.split_once(&format!("pub trait {name} {{")).expect("the trait").1;
        let body = body.split_once("\n}").expect("the trait's end").0;
        body.lines().filter_map(|line| line.strip_prefix("    fn ")).map(|rest| rest.split('(').next().unwrap().to_owned()).collect()
    }

    #[test]
    fn every_trait_method_is_counted() {
        let traits = [("LayoutHost", "layout", include_str!("layout_host.rs")), ("DisplayHost", "display", include_str!("display_host.rs")),
                      ("HostInput", "input", include_str!("host_input.rs")), ("TerminalHost", "terminal", include_str!("terminal_host.rs"))];
        let wrappers = include_str!("host_calls.rs");
        for (name, prefix, source) in traits {
            let methods = trait_methods(source, name);
            assert!(methods.len() >= 2, "{name}: {methods:?}");
            for method in methods {
                assert!(wrappers.contains(&format!("fn {method}(&")), "{name}::{method} is not forwarded");
                assert!(SIDES.iter().any(|(known, _)| *known == format!("{prefix}.{method}")), "{name}::{method} has no side");
            }
        }
    }

    #[test]
    fn a_counted_host_counts_and_forwards() {
        struct Terminals;
        impl TerminalHost for Terminals {
            fn create(&mut self, _: &str, _: &str, _: u16, _: u16) -> Result<u64, String> { Err("none".into()) }
            fn send(&mut self, _: u64, _: &[u8]) -> Result<(), String> { Err("none".into()) }
            fn resize(&mut self, _: u64, _: u16, _: u16) -> Result<(), String> { Err("none".into()) }
            fn input_state(&mut self, _: u64) -> Result<TerminalInputState, String> { Err("none".into()) }
            fn changed_sessions(&mut self) -> Vec<u64> { vec![7] }
            fn snapshot(&mut self, _: u64) -> Result<TerminalSnapshot, String> { Err("none".into()) }
            fn close(&mut self, _: u64) -> Result<(), String> { Err("none".into()) }
        }
        let counted = |name| counts().into_iter().find(|row| row.0 == name).map_or(0, |row| row.2);
        let was = counted("terminal.changed_sessions");
        let mut host = Counted(Box::new(Terminals) as Box<dyn TerminalHost>);
        assert_eq!((host.changed_sessions(), host.changed_sessions()), (vec![7], vec![7]));
        assert_eq!(counted("terminal.changed_sessions"), was + 2);
        assert_eq!((side("layout.lay_out"), side("clock"), side("read-file"), side("input.redisplay"), side("sender.wait_for_render")), (Local, Local, Cross, Through, Cross));
    }
}
