use std::time::{Duration, Instant};
use romacs_display_runtime::thread_comm::InputEvent;

pub fn take_pending(
    pending: &mut std::collections::VecDeque<InputEvent>,
) -> std::collections::vec_deque::IntoIter<InputEvent> {
    std::mem::take(pending).into_iter()
}

pub fn take_batch(
    pending: &mut std::collections::VecDeque<InputEvent>,
    incoming: impl Iterator<Item = InputEvent>,
    available: usize,
) -> std::collections::vec_deque::IntoIter<InputEvent> {
    pending.extend(incoming.take(available));
    take_pending(pending)
}

pub struct Presentation {
    dirty: bool,
    in_flight: Option<(u64, Instant)>,
}

impl Presentation {
    pub fn new() -> Self { Self { dirty: false, in_flight: None } }

    pub fn changed(&mut self) { self.dirty = true; }

    pub fn ready(&self) -> bool { self.dirty && self.in_flight.is_none() }

    pub fn after_batch<T>(&self, submit: impl FnOnce() -> T) -> Option<T> {
        self.ready().then(submit)
    }

    pub fn command(
        &mut self,
        vm: &mut romacs_vm::Vm,
        commands: &mut crate::command::Commands,
        event: romacs_vm::CommandEvent,
    ) -> Result<Option<romacs_vm::CommandOutcome>, String> {
        self.changed();
        vm.eval_source("(set-buffer (window-buffer (selected-window)))")?;
        // A confirmation belongs only to the next answer in its original buffer.
        // Unlike GNU's reprompt, other input cancels before ordinary dispatch;
        // clearing the echo area must never leave invisible discard consent.
        let answer = matches!(&event, romacs_vm::CommandEvent::Character(121 | 110 | 7))
            && !commands.in_prefix();
        vm.eval_source(&format!(
            "(when (and (boundp 'romacs-gui-pending) romacs-gui-pending
                        (not (and {} (eq (current-buffer) (car (cdr romacs-gui-pending))))))
               (romacs-gui-answer-no))",
            if answer { "t" } else { "nil" },
        ))?;
        // GNU's command loop clears the previous message when a new command
        // starts, not on each key of a prefix sequence.
        if !commands.in_prefix() { crate::echo::clear(vm)?; }
        let outcome = commands.dispatch(vm, event);
        if let Ok(Some(romacs_vm::CommandOutcome { signal: Some(signal), .. })) = &outcome {
            let message = crate::echo::signal_message(vm, signal)?;
            crate::echo::show(vm, &message)?;
        } else {
            crate::echo::show_current_message(vm)?;
        }
        crate::eval_last_sexp::after_command(vm)?;
        outcome
    }

    pub fn window_close(&mut self, vm: &mut romacs_vm::Vm) -> Result<bool, String> {
        if vm.eval_source("(fboundp 'romacs-gui-quit)")?.is_nil() { return Ok(true); }
        self.changed();
        vm.eval_source("(set-buffer (window-buffer (selected-window))) (romacs-gui-quit)")?;
        Ok(vm.eval_source("romacs-gui-quit-requested")? != romacs_vm::Fixed::NIL)
    }

    pub fn submitted(&mut self, presentation: u64, now: Instant) {
        self.dirty = false;
        self.in_flight = Some((presentation, now));
    }

    pub fn in_flight(&self) -> Option<(u64, Instant)> { self.in_flight }

    pub fn completed(&mut self, presentation: u64) {
        if self.in_flight.is_some_and(|(actual, _)| actual == presentation) { self.in_flight = None; }
    }

    pub fn wait(&self) -> Duration {
        if self.in_flight.is_some() { Duration::from_millis(1) }
        else { Duration::from_millis(100) }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn workspace() -> romacs_vm::Vm {
        let mut vm = romacs_vm::Vm::new();
        vm.set_collection_floor(1);
        vm.set_frame_geometry(1200, 800, 20, 10).unwrap();
        crate::startup::initialize(&mut vm).unwrap();
        crate::eval_last_sexp::install(&mut vm).unwrap();
        vm.eval_source("(erase-buffer)").unwrap();
        vm
    }

    struct PromptKeys(std::collections::VecDeque<i64>);

    impl romacs_vm::HostInput for PromptKeys {
        fn input_pending(&mut self) -> bool { false }

        fn redisplay(&mut self, _: romacs_vm::HostFrame) -> Result<Vec<(u64, usize)>, String> {
            Ok(vec![])
        }

        fn next_event(&mut self) -> Result<romacs_vm::HostEvent, String> {
            self.0.pop_front()
                .map(|code| romacs_vm::HostEvent::Key(key(code)))
                .ok_or_else(|| "prompt input exhausted".into())
        }
    }

    #[test]
    fn prompt_quit_shows_gnus_message_and_the_command_loop_continues() {
        let mut vm = workspace();
        vm.set_host_input(Some(Box::new(PromptKeys([7, 122, 10].into()))));
        vm.eval_source(r#"
            (defun presentation-prompt () (interactive)
              (setq presentation-answer (read-string "P: ")))
            (global-set-key [presentation-prompt] 'presentation-prompt)
        "#).unwrap();
        let mut commands = crate::command::Commands::default();
        let mut presentation = Presentation::new();
        let event = || romacs_vm::CommandEvent::Named("presentation-prompt".into());
        let quit = presentation.command(&mut vm, &mut commands, event());
        assert_eq!(crate::echo::text(&mut vm).unwrap(), "Quit", "{quit:?}");
        let quit = quit.unwrap().unwrap();
        assert_eq!(quit.signal.as_ref().unwrap().condition, "minibuffer-quit");
        assert_eq!(quit.signal.as_ref().unwrap().data, "nil");
        assert!(quit.bell && presentation.ready());
        assert_eq!(vm.eval_source("(eq (selected-window) (minibuffer-window))").unwrap().display().unwrap(), "nil");
        let next = presentation.command(&mut vm, &mut commands, event()).unwrap().unwrap();
        assert!(next.signal.is_none(), "{next:?}");
        assert_eq!(vm.eval_source("presentation-answer").unwrap().display().unwrap(), "\"z\"");
        assert_eq!(crate::echo::text(&mut vm).unwrap(), "");
        presentation.command(&mut vm, &mut commands, key(97)).unwrap();
        assert_eq!(vm.eval_source("(buffer-string)").unwrap().display().unwrap(), "\"a\"");
    }

    fn native_key(code: i64) -> InputEvent {
        use romacs_display_runtime::host_types::FrontendKey;
        InputEvent::Key {
            key: FrontendKey::Character(char::from_u32(code as u32).unwrap()),
            modifiers: 0,
            pressed: true,
            emacs_frame_id: 1,
        }
    }

    fn key(code: i64) -> romacs_vm::CommandEvent {
        crate::native::translate(native_key(code)).unwrap()
    }

    #[test]
    fn finite_input_batch_does_not_starve_dirty_presentation_under_continuous_arrivals() {
        let mut pending = std::collections::VecDeque::new();
        let mut presentation = Presentation::new();
        let mut arrivals = std::iter::repeat_with(|| native_key(97));
        let batch = take_batch(&mut pending, &mut arrivals, 3);
        assert_eq!(batch.count(), 3);
        presentation.changed();
        pending.push_back(arrivals.next().unwrap());
        let mut submissions = 0;
        let submitted = presentation.after_batch(|| {
            submissions += 1;
            assert_eq!(pending.len(), 1);
            2
        }).expect("new arrivals cannot postpone submission after the finite batch");
        presentation.submitted(submitted, Instant::now());
        assert_eq!(submissions, 1);
        assert_eq!(presentation.in_flight().unwrap().0, 2);
    }

    #[test]
    fn busy_renderer_does_not_lose_or_reorder_native_commands() {
        let mut vm = workspace();
        let mut commands = crate::command::Commands::default();
        let mut presentation = Presentation::new();
        presentation.submitted(1, Instant::now());
        let collections = vm.heap_stats().collections;
        let mut pending = ['a' as i64, 'Δ' as i64, 2, 'λ' as i64].into_iter().map(native_key).collect();
        for event in take_pending(&mut pending) {
            let event = crate::native::translate(event).unwrap();
            let outcome = presentation.command(&mut vm, &mut commands, event).unwrap().unwrap();
            assert!(!outcome.bell, "{outcome:?}");
            assert!(!presentation.ready());
            vm.collect_garbage();
        }
        let value = vm.eval_source("(list (buffer-string) (point))").unwrap();
        assert_eq!(value.display().unwrap(), "(\"aλΔ\" 3)");
        assert!(vm.heap_stats().collections > collections);
        presentation.completed(1);
        assert!(presentation.ready());
        let snapshot = crate::snapshot(&mut vm, false, 1200, 800).unwrap();
        assert_eq!(snapshot.windows[0].snapshot.text.utf8, "aλΔ");
        assert_eq!(snapshot.windows[0].snapshot.point_byte, 3);
    }

    #[test]
    fn owned_presentation_keeps_text_point_and_layout_consistent_across_gc_and_later_input() {
        let mut vm = workspace();
        let mut commands = crate::command::Commands::default();
        let mut presentation = Presentation::new();
        for code in ['a' as i64, 'Δ' as i64, 2, 'λ' as i64] {
            presentation.command(&mut vm, &mut commands, key(code)).unwrap();
        }
        let snapshot = crate::snapshot(&mut vm, false, 1200, 800).unwrap();
        let mut engine = romacs_layout2::LayoutEngine::new().unwrap();
        let (before, _) = engine.layout_frame(&snapshot, 2, None).unwrap();
        presentation.submitted(2, Instant::now());
        presentation.command(&mut vm, &mut commands, key('!' as i64)).unwrap();
        vm.collect_garbage();
        assert_eq!(snapshot.windows[0].snapshot.text.utf8, "aλΔ");
        assert_eq!(snapshot.windows[0].snapshot.point_byte, 3);
        let (after, _) = engine.layout_frame(&snapshot, 2, None).unwrap();
        assert_eq!(format!("{:?}", before.window_cursors), format!("{:?}", after.window_cursors));
        assert_eq!(format!("{:?}", before.glyphs), format!("{:?}", after.glyphs));
        let next = crate::snapshot(&mut vm, false, 1200, 800).unwrap();
        assert_eq!(next.windows[0].snapshot.text.utf8, "aλ!Δ");
        assert_eq!(next.windows[0].snapshot.point_byte, 4);
        assert_ne!(snapshot, next);
        presentation.completed(2);
        assert!(presentation.ready());
    }

    #[test]
    fn signals_after_mutation_still_schedule_a_presentation() {
        let mut vm = workspace();
        vm.eval_source("(defun presentation-fail () (interactive) (insert \"!\") (error \"failed\")) (global-set-key [102] 'presentation-fail)").unwrap();
        let mut commands = crate::command::Commands::default();
        let mut presentation = Presentation::new();
        let outcome = presentation.command(&mut vm, &mut commands, key(102)).unwrap().unwrap();
        assert_eq!(outcome.signal.unwrap().condition, "error");
        assert!(presentation.ready());
        let snapshot = crate::snapshot(&mut vm, false, 1200, 800).unwrap();
        assert_eq!(snapshot.windows[0].snapshot.text.utf8, "!");
        assert_eq!(snapshot.windows[0].snapshot.point_byte, 1);
    }

    #[test]
    fn commands_coalesce_before_snapshot_and_wait_for_matching_completion() {
        let now = Instant::now();
        let mut presentation = Presentation::new();
        presentation.submitted(1, now);
        presentation.changed();
        presentation.changed();
        assert!(!presentation.ready(), "input must not trigger per-command layout while rendering");
        presentation.completed(0);
        assert!(!presentation.ready(), "stale completion cannot release a frame");
        presentation.completed(1);
        assert!(presentation.ready(), "dirty commands must survive renderer completion");
        presentation.submitted(2, now);
        presentation.completed(2);
        assert!(!presentation.ready());
    }
}
