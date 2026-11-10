//! Native models (rlisp/romacs-model.el, `src/display_host/models.rs`): the primitives
//! check Lisp's arguments and send the display exactly the protocol values,
//! a `(surface :id ...)` display spec lays out as a surface glyph only while
//! its surface lives, and the renderer's events reach Lisp as the special
//! event `(romacs-model ID TYPE PLIST)`, between commands and inside a read.
use romacs_display_protocol::model::{ModelChange, ModelEvent, ModelHit, ModelTarget, ModelView};
use romacs_vm::{CommandEvent, HostEvent, HostFrame, HostInput, Vm};
use std::cell::RefCell;
use std::collections::VecDeque;
use std::rc::Rc;

#[derive(Default)]
struct Log {
    available: bool,
    created: Vec<(u32, romacs_vm::ModelCreate)>,
    updates: Vec<(u32, u64, ModelChange)>,
    freed: Vec<u32>,
}

struct Display(Rc<RefCell<Log>>);

impl romacs_vm::DisplayHost for Display {
    fn window_system(&self) -> &'static str { "pgtk" }
    fn display_name(&self) -> String { String::new() }
    fn list_fonts(&mut self, _: &romacs_vm::FontQuery) -> Vec<romacs_vm::FontEntity> { vec![] }
    fn font_families(&mut self) -> Vec<String> { vec![] }
    fn open_font(&mut self, _: &romacs_vm::FontEntity, _: i64) -> Option<romacs_vm::OpenedFont> { None }
    fn char_font(&mut self, _: &romacs_vm::FaceFont, _: char) -> Option<(romacs_vm::OpenedFont, u32)> { None }
    fn model_available(&self) -> bool { self.0.borrow().available }
    fn create_model(&mut self, id: u32, model: romacs_vm::ModelCreate) -> Result<(), String> {
        self.0.borrow_mut().created.push((id, model));
        Ok(())
    }
    fn update_model(&mut self, id: u32, req: u64, change: ModelChange) -> Result<(), String> {
        self.0.borrow_mut().updates.push((id, req, change));
        Ok(())
    }
    fn free_surface(&mut self, id: u32) { self.0.borrow_mut().freed.push(id); }
}

fn session(available: bool) -> (Vm, Rc<RefCell<Log>>) {
    let log = Rc::new(RefCell::new(Log { available, ..Log::default() }));
    let mut vm = Vm::new();
    vm.install_lisp_prelude().unwrap();
    vm.set_frame_geometry(800, 600, 20, 10).unwrap();
    vm.set_display_host(Some(Box::new(Display(log.clone()))));
    assert!(vm.open_display());
    (vm, log)
}

fn eval(vm: &mut Vm, source: &str) -> String {
    match vm.eval_source(source) {
        Ok(value) => value.display().unwrap(),
        Err(e) => format!("ERROR {e}"),
    }
}

const CREATE: &str = r#"(romacs-model--create '(("/m/a.obj" nil) ("/m/b.obj" nil)) '(10 -20 1.5) '(0 0.5 1) 64 48)"#;

#[test]
fn create_and_requests_send_the_protocol_values() {
    let (mut vm, log) = session(true);
    assert_eq!(eval(&mut vm, "(romacs-model--available-p)"), "t");
    assert_eq!(eval(&mut vm, CREATE), "1");
    {
        let log = log.borrow();
        let (id, model) = &log.created[0];
        assert_eq!(*id, 1);
        assert_eq!(model.meshes.len(), 2);
        assert_eq!(model.meshes[1].path, std::path::PathBuf::from("/m/b.obj"));
        assert_eq!(model.view, ModelView { yaw: 10.0, pitch: -20.0, zoom: 1.5 });
        assert_eq!(model.background.map(f32::to_bits), [0.0f32, 0.5, 1.0, 1.0].map(f32::to_bits));
        assert_eq!(model.size, (64, 48));
    }
    eval(&mut vm, "(romacs-model--set-view 1 1 30 0 2)");
    eval(&mut vm, "(romacs-model--set-highlight 1 2 '(:mesh 1 :kind point :face 3 :barycentric [0.2 0.3 0.5] :tolerance 0.1) nil)");
    eval(&mut vm, "(romacs-model--set-highlight 1 3 nil 0)");
    eval(&mut vm, "(romacs-model--set-highlight-layers 1 4 '(((:mesh 0 :kind region :faces (1 2)) 1 0 0)))");
    eval(&mut vm, "(romacs-model--resize 1 5 128 128)");
    eval(&mut vm, "(romacs-model--freeze 1 6)");
    let updates = &log.borrow().updates;
    assert_eq!(updates[0], (1, 1, ModelChange::View(ModelView { yaw: 30.0, pitch: 0.0, zoom: 2.0 })));
    assert_eq!(updates[1].2, ModelChange::Highlight {
        target: Some(ModelTarget::Point { mesh: 1, face: 3, barycentric: [0.2, 0.3, 0.5], tolerance: 0.1 }),
        selected: None,
    });
    assert_eq!(updates[2].2, ModelChange::Highlight { target: None, selected: Some(0) });
    assert!(matches!(&updates[3].2, ModelChange::HighlightLayers(layers)
        if layers.len() == 1 && layers[0].target == ModelTarget::Region { mesh: 0, faces: vec![1, 2] }));
    assert_eq!(updates[4].2, ModelChange::Resize { width: 128, height: 128 });
    assert_eq!(updates[5].2, ModelChange::Freeze);
}

#[test]
fn bad_arguments_and_freed_models_are_refused_and_send_nothing() {
    let (mut vm, log) = session(true);
    for (form, error) in [
        (r#"(romacs-model--create '(("rel.obj" nil)) '(0 0 1) '(0 0 0) 64 64)"#, "absolute"),
        (r#"(romacs-model--create '(("/a.obj" "XYZ")) '(0 0 1) '(0 0 0) 64 64)"#, "64 lowercase hex"),
        (r#"(romacs-model--create '(("/a.obj" nil)) '(0 0 9) '(0 0 0) 64 64)"#, "zoom 0.25..4"),
        (r#"(romacs-model--create '(("/a.obj" nil)) '(0 0 1) '(0 0 0) 8 64)"#, "32..2048"),
        (r"(romacs-model--create nil '(0 0 1) '(0 0 0) 64 64)", "1..255 meshes"),
        // Fixed arities and limits refuse before converting anything, a
        // circular list too.
        (r#"(romacs-model--create '(("/a.obj" nil)) (let ((v (list 0 0 1))) (setcdr (cdr (cdr v)) v) v) '(0 0 0) 64 64)"#, "view must be a list of at most 3"),
        (r#"(romacs-model--create '(("/a.obj" nil)) '(0 0 1) '(0 0 0 0) 64 64)"#, "background must be a list of at most 3"),
        (r#"(romacs-model--create (make-list 256 '("/a.obj" nil)) '(0 0 1) '(0 0 0) 64 64)"#, "meshes must be a list of at most 255"),
    ] {
        let got = eval(&mut vm, form);
        assert!(got.contains(error), "{form}: {got}");
    }
    assert!(log.borrow().created.is_empty());
    assert_eq!(eval(&mut vm, CREATE), "1");
    for (form, error) in [
        ("(romacs-model--set-highlight 1 1 '(:mesh 2 :kind object) nil)", "mesh index out of range"),
        ("(romacs-model--set-highlight 1 1 '(:mesh 0 :kind point :face 0 :barycentric [1 1 1] :tolerance 1) nil)", "summing to 1"),
        ("(romacs-model--set-view 9 1 0 0 1)", "Model is destroyed"),
    ] {
        let got = eval(&mut vm, form);
        assert!(got.contains(error), "{form}: {got}");
    }
    assert!(log.borrow().updates.is_empty());
    assert_eq!(eval(&mut vm, "(list (romacs-surface-destroy 1) (romacs-surface-destroy 1))"), "(t nil)");
    assert_eq!(log.borrow().freed, vec![1]);
    assert!(eval(&mut vm, "(romacs-model--set-view 1 1 0 0 1)").contains("Model is destroyed"));
}

#[test]
fn without_a_live_renderer_nothing_is_created() {
    let (mut vm, log) = session(false);
    assert_eq!(eval(&mut vm, "(romacs-model--available-p)"), "nil");
    assert!(eval(&mut vm, CREATE).contains("No GPU renderer"));
    assert!(log.borrow().created.is_empty());
    let mut batch = Vm::new();
    batch.install_lisp_prelude().unwrap();
    assert_eq!(eval(&mut batch, "(romacs-model--available-p)"), "nil");
    assert!(eval(&mut batch, CREATE).contains("graphical display"));
}

#[test]
fn a_surface_spec_shows_its_live_surface_and_then_its_text() {
    let (mut vm, _) = session(true);
    assert_eq!(eval(&mut vm, CREATE), "1");
    eval(&mut vm, r#"(with-current-buffer (window-buffer (selected-window))
                       (insert "a" (propertize "M" 'display '(surface :id 1 :width 64 :height 48)) "b"))"#);
    let snapshot = vm.frame_snapshot(800, 600).unwrap().windows[0].snapshot.clone();
    let glyph = snapshot.images[0];
    assert_eq!((glyph.surface, glyph.image, glyph.width, glyph.ascent, glyph.descent), (1, 0, 64, 48, 0));
    assert_eq!(glyph.byte, 1);
    eval(&mut vm, "(romacs-surface-destroy 1)");
    let snapshot = vm.frame_snapshot(800, 600).unwrap().windows[0].snapshot.clone();
    assert!(snapshot.images.is_empty(), "a freed surface shows its text");
}

const BINDING: &str = r#"(progn (setq seen nil) (current-global-map)
  (define-key special-event-map [romacs-model]
    (lambda (event) (interactive "e") (push event seen))) t)"#;

fn pick() -> ModelEvent {
    ModelEvent::Pick {
        hit: Some(ModelHit { mesh: 1, face: 7, barycentric: [0.25, 0.25, 0.5], point: [1.0, -2.0, 0.5] }),
        view: ModelView { yaw: 0.0, pitch: 90.0, zoom: 1.0 },
        rev: 3,
        req: 2,
    }
}

#[test]
fn events_run_their_special_event_binding_between_commands() {
    let (mut vm, _) = session(true);
    eval(&mut vm, BINDING);
    vm.run_model_event(4, &pick()).unwrap();
    vm.run_model_event(4, &ModelEvent::Readiness { current: true, rev: 3, req: 2 }).unwrap();
    vm.run_model_event(4, &ModelEvent::Failed { error: "λ missing".into() }).unwrap();
    assert_eq!(eval(&mut vm, "(reverse seen)"),
               "((romacs-model 4 pick (:hit (:mesh 1 :face 7 :barycentric [0.25 0.25 0.5] :point [1.0 -2.0 0.5]) \
:view (0.0 90.0 1.0) :rev 3 :req 2)) (romacs-model 4 readiness (:current t :rev 3 :req 2)) \
(romacs-model 4 failed (:error \"λ missing\")))");
}

/// A binding that signals between commands answers the embedder with the
/// error and leaves nothing behind: the next evaluation runs as usual.
#[test]
fn a_failing_event_binding_leaves_no_escape_behind() {
    let (mut vm, _) = session(true);
    eval(&mut vm, "(progn (current-global-map) (define-key special-event-map [romacs-model] \
        (lambda (_event) (interactive \"e\") (error \"model handler failed\"))) t)");
    let failed = vm.run_model_event(4, &pick());
    assert!(failed.is_err(), "the error comes back: {failed:?}");
    assert_eq!(eval(&mut vm, "(condition-case err (+ 1 2) (error (list 'caught err)))"), "3");
}

struct Script(VecDeque<HostEvent>);

impl HostInput for Script {
    fn input_pending(&mut self) -> bool { true }
    fn redisplay(&mut self, _: HostFrame) -> Result<Vec<romacs_vm::WindowEnd>, String> { Ok(vec![]) }
    fn next_event(&mut self, _: Option<std::time::Duration>) -> Result<HostEvent, String> {
        self.0.pop_front().ok_or_else(|| "script exhausted".to_string())
    }
}

/// The read hands the event to Lisp as its special event; rlisp/vm/keyboard.el
/// runs its binding and reads on, as for focus events.
#[test]
fn a_read_answers_an_event_as_its_special_event() {
    let (mut vm, _) = session(true);
    eval(&mut vm, BINDING);
    vm.set_host_input(Some(Box::new(Script(VecDeque::from([
        HostEvent::Model { id: 2, event: ModelEvent::Ready { triangles: vec![12, 4] } },
        HostEvent::Key(CommandEvent::Character('x' as i64)),
    ])))));
    assert_eq!(eval(&mut vm, "(list (romacs--read-host-event) (romacs--read-host-event))"),
               "((romacs-model 2 ready (:triangles (12 4))) 120)");
}
