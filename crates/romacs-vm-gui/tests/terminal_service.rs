use romacs_vm_gui::{terminal_service, terminal_wait};

use romacs_vm::{TerminalHost, TerminalSnapshot};
use std::time::{Duration, Instant};
use terminal_service::Service;

fn wait(
    service: &mut Service,
    id: u64,
    predicate: impl Fn(&TerminalSnapshot) -> bool,
) -> TerminalSnapshot {
    let deadline = Instant::now() + Duration::from_secs(8);
    loop {
        let snapshot = service.snapshot(id).unwrap();
        if predicate(&snapshot) {
            return snapshot;
        }
        assert!(Instant::now() < deadline, "PTY timeout: {snapshot:?}");
        std::thread::sleep(Duration::from_millis(10));
    }
}

#[test]
fn real_shell_cwd_dimensions_resize_interrupt_and_final_drain() {
    let mut service = Service::new();
    let id = service.create("/bin/sh", "/", 40, 8).unwrap();
    service
        .send(id, b"stty -echo; printf 'READY\\n'; pwd; stty size\n")
        .unwrap();
    let initial = wait(&mut service, id, |s| {
        s.text.contains("8 40") && s.text.contains("READY\n/\n")
    });
    assert!(initial.running);
    service.resize(id, 52, 11).unwrap();
    service
        .send(
            id,
            b"stty size; printf '\\033[?1h\\033[?2004h'; printf 'MODES\\n'\n",
        )
        .unwrap();
    let resized = wait(&mut service, id, |s| {
        s.text.contains("11 52") && s.application_cursor && s.bracketed_paste
    });
    assert_eq!((resized.columns, resized.rows), (52, 11));
    assert!(resized.revision > initial.revision);
    assert_eq!(resized, service.snapshot(id).unwrap());
    service.send(id, b"sleep 30\n").unwrap();
    std::thread::sleep(Duration::from_millis(100));
    service.send(id, &[3]).unwrap();
    service
        .send(
            id,
            b"printf 'INTERRUPTED\\n'; printf 'e\\314\\201X\\n'; printf 'LAST-OUTPUT\\n'; exit\n",
        )
        .unwrap();
    let final_snapshot = wait(&mut service, id, |s| !s.running);
    assert!(
        final_snapshot.text.contains("INTERRUPTED"),
        "{final_snapshot:?}"
    );
    assert!(
        final_snapshot.text.contains("e\u{301}X"),
        "{final_snapshot:?}"
    );
    assert!(
        final_snapshot.text.contains("LAST-OUTPUT"),
        "{final_snapshot:?}"
    );
    assert!(
        final_snapshot
            .text
            .is_char_boundary(final_snapshot.cursor_byte)
    );
    assert!(
        final_snapshot
            .text
            .is_char_boundary(final_snapshot.screen_start_byte)
    );
    assert_eq!(final_snapshot, service.snapshot(id).unwrap());
    assert!(service.send(id, b"x").is_err());
    service.close(id).unwrap();
    assert!(service.snapshot(id).is_err());
}

#[test]
fn unchanged_publications_share_text_and_changed_output_preserves_old_values() {
    let mut service = Service::new();
    let id = service.create("/bin/sh", "/", 80, 8).unwrap();
    service.send(id, b"stty -echo; printf '\\033[2J\\033[HIMMUTABLE_READY\\n'; read first; printf 'NEXT_PUBLICATION\\n'; read second\n").unwrap();
    wait(&mut service, id, |s| s.text.contains("IMMUTABLE_READY\n"));
    let before = service.snapshot(id).unwrap();
    let retained = before.text.to_string();
    for _ in 0..100 {
        let unchanged = service.snapshot(id).unwrap();
        assert_eq!(unchanged.revision, before.revision);
        assert_eq!(unchanged.text.as_ptr(), before.text.as_ptr(), "IDLE_TERMINAL_MUST_NOT_REBUILD_OR_COPY_HISTORY");
    }
    service.send(id, b"go\n").unwrap();
    let after = wait(&mut service, id, |s| s.text.contains("NEXT_PUBLICATION\n"));
    assert!(after.revision > before.revision);
    assert!(!retained.contains("NEXT_PUBLICATION\n"), "PUBLISHED_TERMINAL_VALUES_ARE_IMMUTABLE: {retained:?}");
    assert_eq!(&*before.text, retained);
    service.close(id).unwrap();
}

#[test]
fn native_readiness_delivers_final_output_without_timer_polling() {
    let mut service = Service::new();
    let ready = service.ready();
    let id = service.create("/bin/sh", "/", 80, 8).unwrap();
    service.send(id, b"stty -echo; printf 'EVENT_READY\\n'; read answer; printf 'EVENT_FINAL\\n'; exit\n").unwrap();
    let deadline = Instant::now() + Duration::from_secs(8);
    let mut resumed = false;
    loop {
        ready.recv_timeout(deadline.saturating_duration_since(Instant::now())).expect("NATIVE_OUTPUT_MUST_WAKE_WAITER");
        let owners = service.changed_sessions();
        assert_eq!(owners, vec![id], "WAKE_MUST_ADDRESS_ITS_TERMINAL");
        let snapshot = service.snapshot(id).unwrap();
        if !resumed && snapshot.text.contains("EVENT_READY\n") {
            service.send(id, b"go\n").unwrap();
            resumed = true;
        }
        if !snapshot.running {
            assert!(resumed);
            assert!(snapshot.text.contains("EVENT_FINAL\n"), "EXIT_WAKE_MUST_INCLUDE_DRAINED_OUTPUT");
            break;
        }
    }
    service.close(id).unwrap();
}

#[test]
fn closing_the_last_real_terminal_returns_editor_waits_to_idle() {
    let service = Service::new();
    let ready = service.ready();
    let mut vm = romacs_vm::Vm::new();
    vm.set_frame_geometry(800, 600, 20, 10).unwrap();
    vm.set_terminal_host(Some(Box::new(service))).unwrap();
    vm.eval_source("(set-buffer (get-buffer-create \"closing-terminal\")) (setq closing-id (romacs--term-create \"/bin/sh\" \"/\" 80 8))").unwrap();
    vm.poll_terminal_output().unwrap();
    vm.eval_source("(romacs--term-close closing-id)").unwrap();
    let (_keys, input) = crossbeam_channel::bounded(1);
    let deadline = Instant::now() + Duration::from_secs(2);
    loop {
        assert!(Instant::now() < deadline, "LAST_TERMINAL_CLOSE_MUST_RETURN_TO_IDLE");
        assert!(!vm.poll_terminal_output().unwrap());
        match terminal_wait::wait(&input, &ready, Some(Duration::from_millis(50)), &[]) {
            terminal_wait::Wake::Terminal => {},
            terminal_wait::Wake::Timeout => break,
            _ => panic!("no keyboard or close event was sent"),
        }
    }
    assert!(!vm.has_terminals());
    assert!(ready.is_empty());
}

fn face_at(view: &romacs_layout_input::WindowSnapshot, byte: usize) -> &romacs_layout_input::Face {
    let run = view.text.face_runs.iter().find(|run| run.bytes.contains(&byte)).unwrap();
    &view.faces[run.face.0 as usize]
}

#[test]
fn native_shell_colors_reach_buffer_faces_and_repaint_without_text_changes() {
    let mut vm = romacs_vm::Vm::new();
    vm.set_frame_geometry(1000, 600, 20, 10).unwrap();
    vm.set_terminal_host(Some(Box::new(Service::new()))).unwrap();
    vm.eval_source("(setq color-buffer (get-buffer-create \"terminal-colors\")) (set-buffer color-buffer) (set-window-buffer (selected-window) color-buffer) (setq color-id (romacs--term-create \"/bin/sh\" \"/\" 100 12))").unwrap();
    vm.terminal_send(1, b"stty -echo; printf '\\033[2J\\033[H\\033[31mRED\\033[0m \\033[38;5;196mINDEXED\\033[0m \\033[38;2;12;34;56mTRUE\\033[48;2;65;43;21mBG\\033[0m RESET\\nCOLOR_DONE\\n'; read repaint; printf '\\033[H\\033[32mRED\\033[0m\\033[3;1H'; read stop\n").unwrap();
    let deadline = Instant::now() + Duration::from_secs(8);
    let initial = loop {
        vm.poll_terminal_output().unwrap();
        let frame = vm.frame_snapshot(1000, 600).unwrap();
        let view = frame.windows.into_iter().next().unwrap().snapshot;
        if view.text.utf8.contains("RED INDEXED TRUEBG RESET\nCOLOR_DONE\n") { break view; }
        assert!(Instant::now() < deadline, "shell color fixture did not reach output");
        std::thread::sleep(Duration::from_millis(5));
    };
    let red = initial.text.utf8.rfind("RED INDEXED").unwrap();
    assert_eq!(face_at(&initial, red).foreground, [172, 66, 66], "SHELL_ANSI_COLORS_MUST_REACH_RENDERED_FACES");
    assert_eq!(face_at(&initial, red + 4).foreground, [255, 0, 0], "ANSI_256_COLOR");
    assert_eq!(face_at(&initial, red + 12).foreground, [12, 34, 56], "ANSI_TRUE_COLOR");
    assert_eq!(face_at(&initial, red + 16).background, [65, 43, 21], "ANSI_BACKGROUND_COLOR");
    assert_eq!(face_at(&initial, red + 19).foreground, initial.faces[0].foreground, "SGR_RESET_USES_EDITOR_DEFAULT");
    let retained = initial.clone();
    vm.terminal_send(1, b"go\n").unwrap();
    let deadline = Instant::now() + Duration::from_secs(8);
    let changed = loop {
        vm.poll_terminal_output().unwrap();
        let frame = vm.frame_snapshot(1000, 600).unwrap();
        let view = frame.windows.into_iter().next().unwrap().snapshot;
        if face_at(&view, red).foreground == [144, 169, 89] { break view; }
        assert!(Instant::now() < deadline, "STYLE_ONLY_UPDATE_MUST_REPAINT");
        std::thread::sleep(Duration::from_millis(5));
    };
    assert_eq!(changed.text.utf8, initial.text.utf8, "COLOR_UPDATE_DOES_NOT_EDIT_TRANSCRIPT");
    assert_eq!(initial, retained, "OLD_FRAME_VALUES_STAY_IMMUTABLE");
    vm.eval_source("(romacs--term-close color-id)").unwrap();
}

#[test]
fn terminal_styles_retain_unicode_colored_spaces_and_palette_overrides() {
    use romacs_vm::{TerminalColor, TerminalStyle};
    let mut service = Service::new();
    let id = service.create("/bin/sh", "/", 100, 12).unwrap();
    service.send(id, "stty -echo; printf '\\033[2J\\033[H\\033[31m界é\\033[0m \\033[44m  \\033[0m\\nUNICODE_STYLE_DONE\\n'; read first; printf '\\033]4;1;rgb:01/02/03\\007\\033]10;rgb:04/05/06\\007\\033]11;rgb:07/08/09\\007\\033[2J\\033[H\\033[31mOVERRIDE\\033[0m \\033[2;31mDIMSET\\033[0m \\033[2;32mDIMTHEME\\033[0m\\nOSC_DONE\\n'; read last\n".as_bytes()).unwrap();
    let initial = wait(&mut service, id, |s| s.text.contains("界é   \nUNICODE_STYLE_DONE\n"));
    let start = initial.text.rfind("界é   \n").unwrap();
    let style = |at| initial.styles.iter().find(|run| run.bytes.contains(&at)).map(|run| run.style).unwrap_or_default();
    assert_eq!(style(start).foreground, TerminalColor::Ansi(1), "THEME_BASE_COLOURS_STAY_SYMBOLIC");
    assert_eq!(style(start + "界é".len() + 1).background, TerminalColor::Ansi(4), "COLORED_TRAILING_SPACES_MUST_NOT_BE_TRIMMED");
    for run in initial.styles.iter() {
        assert!(initial.text.is_char_boundary(run.bytes.start) && initial.text.is_char_boundary(run.bytes.end));
    }
    assert_eq!(style(start + "界é".len()), TerminalStyle::default(), "RESET_MUST_END_STYLE_AT_UNICODE_BOUNDARY");
    service.send(id, b"go\n").unwrap();
    let changed = wait(&mut service, id, |s| s.text.contains("OSC_DONE\n"));
    assert_eq!(changed.default_colors, [Some([4, 5, 6]), Some([7, 8, 9])], "SHELL_DEFAULT_PALETTE_OVERRIDES");
    let start = changed.text.rfind("OVERRIDE").unwrap();
    assert_eq!(changed.styles.iter().find(|r| r.bytes.contains(&start)).unwrap().style.foreground, TerminalColor::Rgb([1, 2, 3]), "SHELL_INDEXED_PALETTE_OVERRIDES");
    let fg = |word: &str| changed.styles.iter().find(|r| r.bytes.contains(&changed.text.rfind(word).unwrap())).unwrap().style.foreground;
    assert_eq!(fg("DIMSET"), TerminalColor::Rgb([0, 1, 1]), "DIM_OF_A_SESSION_COLOUR_IS_LITERAL");
    assert_eq!(fg("DIMTHEME"), TerminalColor::DimAnsi(2), "DIM_OF_A_THEME_COLOUR_STAYS_SYMBOLIC");
    service.close(id).unwrap();
}

#[test]
fn real_raw_input_and_bounded_close() {
    let mut service = Service::new();
    let id = service.create("/bin/sh", "/", 32, 5).unwrap();
    service.send(id, b"stty raw -echo; printf 'RAW_READY\\r\\n'; dd bs=1 count=3 2>/dev/null | od -An -tx1; stty sane; exit\n").unwrap();
    wait(&mut service, id, |s| s.text.contains("RAW_READY\n"));
    service.send(id, &[0, 3, 255]).unwrap();
    let final_snapshot = wait(&mut service, id, |s| !s.running);
    assert!(
        final_snapshot.text.contains("00 03 ff"),
        "{final_snapshot:?}"
    );
    service.close(id).unwrap();
    let id = service.create("/bin/sh", "/", 32, 5).unwrap();
    service
        .send(
            id,
            b"stty -echo; trap '' HUP; printf 'CLOSE_READY:%s\\n' \"$$\"\n",
        )
        .unwrap();
    let snapshot = wait(&mut service, id, |s| {
        s.text.lines().any(|l| l.starts_with("CLOSE_READY:"))
    });
    let pid: u32 = snapshot
        .text
        .lines()
        .find_map(|l| l.strip_prefix("CLOSE_READY:"))
        .unwrap()
        .parse()
        .unwrap();
    let start = Instant::now();
    service.close(id).unwrap();
    assert!(start.elapsed() < Duration::from_millis(500));
    #[cfg(target_os = "linux")]
    {
        let deadline = Instant::now() + Duration::from_secs(2);
        while std::path::Path::new(&format!("/proc/{pid}")).exists() {
            assert!(
                Instant::now() < deadline,
                "closed shell {pid} was not reaped"
            );
            std::thread::sleep(Duration::from_millis(10));
        }
    }
    assert!(service.close(id).is_err());
}

#[test]
fn rejects_invalid_dimensions_and_unknown_sessions() {
    let mut service = Service::new();
    assert!(service.create("/bin/sh", "/", 0, 0).is_err());
    assert!(service.create("/bin/sh", "/", 1, 4).is_err());
    assert!(service.snapshot(0).is_err());
    assert!(service.send(0, b"").is_err());
    assert!(service.resize(0, 20, 4).is_err());
    assert!(service.close(0).is_err());
}

#[test]
fn real_queries_reply_without_term_callback_deadlock() {
    let mut service = Service::new();
    let id = service.create("/bin/sh", "/", 40, 8).unwrap();
    service.send(id, b"stty raw -echo; printf 'QUERY_READY\\r\\n'; printf '\\033[18t'; dd bs=1 count=9 2>/dev/null | od -An -tx1; printf '\\033]4;1;rgb:12/34/56\\007\\033]4;1;?\\007'; dd bs=1 count=25 2>/dev/null | od -An -tx1; stty sane; exit\n").unwrap();
    let snapshot = wait(&mut service, id, |s| !s.running);
    assert!(
        snapshot.text.contains("1b 5b 38 3b 38 3b 34 30 74"),
        "{snapshot:?}"
    );
    assert!(
        snapshot
            .text
            .contains("1b 5d 34 3b 31 3b 72 67 62 3a 31 32 31 32 2f 33"),
        "{snapshot:?}"
    );
    assert!(
        snapshot.text.contains("34 33 34 2f 35 36 35 36 07"),
        "{snapshot:?}"
    );
    service.close(id).unwrap();
}

#[test]
fn resize_after_final_output_is_a_safe_noop() {
    let mut service=Service::new();
    let id=service.create("/bin/sh","/",40,8).unwrap();
    service.send(id,b"printf 'FINAL_BEFORE_RESIZE\\n'; exit\n").unwrap();
    let final_snapshot=wait(&mut service,id,|s|!s.running);
    assert!(final_snapshot.text.contains("FINAL_BEFORE_RESIZE"));
    assert!(service.resize(id,60,12).is_ok(),"EXITED_TERMINAL_RESIZE_MUST_NOT_STOP_EDITOR");
    assert_eq!(final_snapshot,service.snapshot(id).unwrap(),"EXITED_TRANSCRIPT_IS_IMMUTABLE");
    service.close(id).unwrap();
}

#[cfg(target_os="linux")]
#[test]
fn confirmed_close_hangs_up_shell_background_jobs() {
    struct Background(u32);
    fn running(pid:u32)->bool {
        std::fs::read_to_string(format!("/proc/{pid}/stat")).ok()
            .and_then(|s|s.rsplit_once(')').map(|(_,tail)|tail.trim().starts_with('Z')))
            .is_some_and(|zombie|!zombie)
    }
    impl Drop for Background {
        fn drop(&mut self) {
            if running(self.0) { let _=std::process::Command::new("kill").args(["-KILL",&self.0.to_string()]).status(); }
        }
    }
    let mut service=Service::new();
    let id=service.create("/bin/sh","/",80,8).unwrap();
    service.send(id,b"stty -echo; sleep 7777 & printf '\\nBG_PID:%s\\n' \"$!\"\n").unwrap();
    let s=wait(&mut service,id,|s|s.text.lines().any(|l|l.starts_with("BG_PID:")));
    let pid=s.text.lines().find_map(|l|l.strip_prefix("BG_PID:")).unwrap().parse().unwrap();
    let job=Background(pid);
    assert!(running(job.0),"BACKGROUND_JOB_STARTED");
    service.close(id).unwrap();
    let deadline=Instant::now()+Duration::from_secs(2);
    while running(job.0) && Instant::now()<deadline { std::thread::sleep(Duration::from_millis(10)); }
    assert!(!running(job.0),"CONFIRMED_CLOSE_MUST_TERMINATE_BACKGROUND_JOB");
}

#[test]
fn empty_shell_selects_the_configured_user_shell() {
    let expected=std::env::var("SHELL").ok().filter(|s|!s.is_empty()).unwrap_or_else(||"/bin/sh".into());
    let mut service=Service::new();
    let session=service.create("", "/", 120, 8);
    assert!(session.is_ok(),"HOST_DEFAULT_SHELL_MUST_USE_USER_SHELL: {session:?}");
    let id=session.unwrap();
    service.send(id,b"printf 'SELECTED-SHELL:%s\\n' \"$0\"\n").unwrap();
    let snapshot=wait(&mut service,id,|s|s.text.lines().any(|l|l==format!("SELECTED-SHELL:{expected}")));
    assert!(snapshot.running,"USER_SHELL_STAYS_INTERACTIVE");
    service.close(id).unwrap();
}

#[test]
fn terminal_tab_cells_project_as_visible_columns_not_raw_controls() {
    let mut service = Service::new();
    let id = service.create("/bin/sh", "/", 40, 8).unwrap();
    service.send(id, b"stty -echo; printf '\\033[2J\\033[Ha\\tb\\nTAB_CELL_DONE\\n'\n").unwrap();
    let snapshot = wait(&mut service, id, |s| s.text.contains("TAB_CELL_DONE\n"));
    assert!(!snapshot.text.chars().any(|c|c.is_control() && c!='\n'), "TERMINAL_TAB_CELLS_MUST_BE_DISPLAYABLE: {:?}",snapshot.text);
    assert!(snapshot.text.contains("a       b\nTAB_CELL_DONE\n"), "TERMINAL_TAB_CELL_COLUMNS: {:?}",snapshot.text);
    service.close(id).unwrap();
}

#[test]
fn real_soft_wrap_unicode_and_cursor_padding() {
    let mut service = Service::new();
    let id = service.create("/bin/sh", "/", 8, 4).unwrap();
    service
        .send(
            id,
            "stty -echo; printf '\\033[2J\\033[Habc界éXYZ\\r\\nHARD\\033[4;7H'; sleep 30\n"
                .as_bytes(),
        )
        .unwrap();
    let snapshot = wait(&mut service, id, |s| {
        s.text[s.screen_start_byte..].starts_with("abc界éXYZ\nHARD\n") && s.cursor_byte >= 10
    });
    assert!(!snapshot.text.contains("abc界é\nXYZ"), "{snapshot:?}");
    assert_eq!(snapshot.text[snapshot.cursor_byte..].chars().next(), None);
    assert!(snapshot.text.ends_with("\n      "), "{snapshot:?}");
    assert_eq!(snapshot, service.snapshot(id).unwrap());
    service.close(id).unwrap();
}

/// Past the history limit every push scrolls by the same count and can
/// look like the one before: the session must still be told, or its
/// history falls behind the terminal's.
#[test]
fn repeated_output_past_the_history_limit_keeps_the_newest_lines() {
    let mut service = Service::new();
    let id = service.create("/bin/sh", "/", 40, 5).unwrap();
    service.send(id, b"i=0; while [ $i -lt 10400 ]; do echo same; i=$((i+1)); done; echo L1; echo same; echo L2; echo same; echo L3\n").unwrap();
    let deadline = std::time::Instant::now() + Duration::from_secs(30);
    let text = loop {
        let _ = service.ready().recv_timeout(Duration::from_millis(100));
        service.changed_sessions();
        let snapshot = service.snapshot(id).unwrap();
        if snapshot.text.contains("\nL3\n") { break snapshot.text; }
        assert!(std::time::Instant::now() < deadline, "the last lines never arrived");
    };
    // Whether or not the next prompt has been printed yet.
    let lines: Vec<&str> = text.lines().collect();
    let last = lines.iter().rposition(|line| *line == "L3").unwrap();
    assert_eq!(lines[last - 4..=last], ["L1", "same", "L2", "same", "L3"], "the history ends as the terminal's does");
    assert_eq!(lines[last - 6..last - 4], ["same", "same"]);
    let (fulls, deltas) = service.transport_counts(id).unwrap();
    assert!(deltas > 0 && fulls < 10, "{fulls} full pushes, {deltas} deltas");
    service.close(id).unwrap();
}

/// A shell that cannot start is an ended terminal, not a blank running one.
#[test]
fn a_shell_that_cannot_start_ends_the_terminal() {
    let mut service = Service::new();
    assert!(service.create("/bin/sh", "/", 1, 5).is_err(), "no helper is spawned for a size no terminal has");
    let id = service.create("/bin/sh", "/nonexistent-directory-for-a-terminal", 40, 5);
    if let Ok(id) = id {
        let deadline = std::time::Instant::now() + Duration::from_secs(10);
        while service.input_state(id).unwrap().running {
            assert!(std::time::Instant::now() < deadline, "the terminal still claims to run");
            let _ = service.ready().recv_timeout(Duration::from_millis(50));
        }
    }
}
