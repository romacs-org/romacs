use romacs_display_protocol::WebViewId;

use crate::{LoadPhase, WebViewEvent, WebViewGeneration};

/// The strings are GNU's, verbatim: `xwidget-webkit-callback' compares
/// `(nth 3 last-input-event)' against "load-finished".
#[test]
fn load_phases_spell_gnus_event_strings() {
    assert_eq!(LoadPhase::Started.gnu_name(), "load-started");
    assert_eq!(LoadPhase::Redirected.gnu_name(), "load-redirected");
    assert_eq!(LoadPhase::Committed.gnu_name(), "load-committed");
    assert_eq!(LoadPhase::Finished.gnu_name(), "load-finished");
}

/// Every event names the browser instance and generation it belongs to; the
/// Linux reactor keys per-view delivery on it, and the match lives in shared
/// code so it is compiled -- and kept exhaustive -- on every platform.
#[test]
fn every_event_reports_its_view_and_generation() {
    let id = WebViewId::new(7);
    let generation = WebViewGeneration::new(11);
    assert_eq!(
        WebViewEvent::LoadChanged {
            id,
            generation,
            phase: LoadPhase::Committed,
        }
        .identity(),
        (id, generation)
    );
    assert_eq!(
        WebViewEvent::ScriptFinished {
            view: id,
            generation,
            request: crate::ScriptRequestId::new(3),
            result: Ok(crate::WebValue::Null),
        }
        .identity(),
        (id, generation)
    );
}
