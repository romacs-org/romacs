use crate::thread_comm::Reply;
use super::*;

struct BlockingClipboard {
    started: Sender<()>,
    release: Receiver<()>,
}

struct BlockingDropClipboard {
    drop_started: Sender<()>,
    release_drop: Receiver<()>,
}

struct ExpiringClipboard {
    read_started: Sender<()>,
    release_read: Receiver<()>,
    writes: Sender<Option<String>>,
}

impl ClipboardBackend for BlockingClipboard {
    fn set_text(
        &mut self,
        _selection: ClipboardSelection,
        _text: Option<&str>,
    ) -> Result<(), String> {
        Ok(())
    }

    fn text(&mut self, _selection: ClipboardSelection) -> Result<text_policy::TextRead, String> {
        self.started.send(()).unwrap();
        self.release.recv().unwrap();
        Ok(text_policy::TextRead::Text("released".to_owned()))
    }
}

impl ClipboardBackend for BlockingDropClipboard {
    fn set_text(
        &mut self,
        _selection: ClipboardSelection,
        _text: Option<&str>,
    ) -> Result<(), String> {
        Ok(())
    }

    fn text(&mut self, _selection: ClipboardSelection) -> Result<text_policy::TextRead, String> {
        Ok(text_policy::TextRead::NoSelection)
    }
}

impl Drop for BlockingDropClipboard {
    fn drop(&mut self) {
        self.drop_started.send(()).unwrap();
        self.release_drop.recv().unwrap();
    }
}

impl ClipboardBackend for ExpiringClipboard {
    fn set_text(
        &mut self,
        _selection: ClipboardSelection,
        text: Option<&str>,
    ) -> Result<(), String> {
        self.writes.send(text.map(str::to_owned)).unwrap();
        Ok(())
    }

    fn text(&mut self, _selection: ClipboardSelection) -> Result<text_policy::TextRead, String> {
        self.read_started.send(()).unwrap();
        self.release_read.recv().unwrap();
        Ok(text_policy::TextRead::NoSelection)
    }
}

#[test]
fn submitting_a_slow_native_read_never_blocks_the_caller() {
    let (started, has_started) = crossbeam_channel::bounded(1);
    let (release, may_finish) = crossbeam_channel::bounded(1);
    let (answers, result) = crossbeam_channel::unbounded();
    let service = ClipboardService::with_backend(BlockingClipboard {
        started,
        release: may_finish,
    }, answers);

    service.submit(ClipboardCommand::GetText {
        selection: ClipboardSelection::Clipboard,
        expires_at: std::time::Instant::now() + Duration::from_secs(5),
        request: 1,
    });

    has_started.recv().unwrap();
    assert!(result.try_recv().is_err());
    release.send(()).unwrap();
    assert_eq!(reply(&result, 1), Reply::Text(Ok(Some("released".to_owned()))));
}

#[test]
fn service_shutdown_is_bounded_even_when_native_backend_drop_stalls() {
    let (drop_started, has_started) = crossbeam_channel::bounded(1);
    let (release_drop, may_finish) = crossbeam_channel::bounded(1);
    let service = ClipboardService::with_backend(BlockingDropClipboard {
        drop_started,
        release_drop: may_finish,
    }, crossbeam_channel::unbounded().0);

    let started = std::time::Instant::now();
    drop(service);

    assert!(
        started.elapsed() < Duration::from_secs(2),
        "clipboard shutdown must not hang the display event loop"
    );
    has_started.recv().unwrap();
    release_drop.send(()).unwrap();
}

#[test]
fn mutation_that_expires_behind_a_slow_read_is_never_executed() {
    let (read_started, has_started) = crossbeam_channel::bounded(1);
    let (release_read, may_finish) = crossbeam_channel::bounded(1);
    let (writes, recorded_writes) = crossbeam_channel::unbounded();
    let (answers, result) = crossbeam_channel::unbounded();
    let service = ClipboardService::with_backend(ExpiringClipboard {
        read_started,
        release_read: may_finish,
        writes,
    }, answers);
    service.submit(ClipboardCommand::GetText {
        selection: ClipboardSelection::Clipboard,
        expires_at: std::time::Instant::now() + Duration::from_secs(5),
        request: 1,
    });
    has_started.recv().unwrap();

    service.submit(ClipboardCommand::SetText {
        selection: ClipboardSelection::Clipboard,
        text: Some("must not be published".to_owned()),
        expires_at: std::time::Instant::now(),
        request: 2,
    });
    release_read.send(()).unwrap();

    assert_eq!(reply(&result, 1), Reply::Text(Ok(None)));
    assert_eq!(
        reply(&result, 2),
        Reply::Done(Err("clipboard request expired before execution".to_owned()))
    );
    assert_eq!(
        recorded_writes.try_recv(),
        Err(crossbeam_channel::TryRecvError::Empty)
    );
}

/// The answer to REQUEST, as the worker sent it.
fn reply(answers: &crossbeam_channel::Receiver<crate::thread_comm::Fact>, request: u64) -> Reply {
    match answers.recv_timeout(Duration::from_secs(5)).unwrap() {
        crate::thread_comm::Fact::Reply { request: answered, reply } if answered == request => reply,
        other => panic!("expected the answer to {request}, got {other:?}"),
    }
}
