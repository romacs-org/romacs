use super::round_six::{delete_last_process_form, initial_call, initial_call_events};
use super::{Master, REAP_BEFORE_DISPATCH, SETUP_TIMEOUT, record_owner_events};
use std::fmt::Write;
use std::time::Instant;

const FIRST_SEED: u64 = 0xa2_0600;
const TRANSITION_SEQUENCES: u64 = 1024;
const WORKER_SEQUENCES: u64 = 24;

struct Random(u64);

impl Random {
    fn pick(&mut self, limit: u64) -> u64 {
        self.0 = self.0.wrapping_add(0x9e37_79b9_7f4a_7c15);
        let mut n = self.0;
        n = (n ^ (n >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
        n = (n ^ (n >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
        (n ^ (n >> 31)) % limit
    }
}

#[expect(
    clippy::cast_possible_truncation,
    reason = "a pick below the event count fits usize"
)]
fn actions(seed: u64) -> String {
    let mut random = Random(seed);
    let mut actions = String::new();
    let events = [
        "kill",
        "end",
        "retire",
        "deadline",
        "answered",
        "memory",
        "orphaned",
        "refused",
        "refuse",
        "hello-refused",
        "world-refused",
        "death",
        "child-gone",
        "shutdown",
    ];
    let length = 4 + random.pick(36);
    for _ in 0..length {
        let target = random.pick(8);
        let event = events[random.pick(events.len() as u64) as usize];
        write!(actions, "({target} {event}) ").unwrap();
    }
    actions.push_str("(0 shutdown)");
    actions
}

#[test]
fn seeded_transition_walk_checks_settlement_shutdown_slots_and_effect_order() {
    let now = Instant::now();
    let mut master = Master::bare();
    master.run(include_str!("owner_lifecycle_random.el"));
    let mut failed = 0;
    let mut first = None;
    let end = FIRST_SEED + TRANSITION_SEQUENCES;
    eprintln!("transition walk seeds {FIRST_SEED:#x}..{end:#x} (exclusive)");
    for seed in FIRST_SEED..end {
        let result = master.eval(&format!("(rw-episode {seed} '({}))", actions(seed)));
        if result != "nil" {
            failed += 1;
            first.get_or_insert((seed, result));
        }
    }
    let covered = master.eval("rw-covered");
    eprintln!(
        "transition walk {failed}/{TRANSITION_SEQUENCES} failed in {:?}; events {covered}",
        now.elapsed()
    );
    assert_eq!(
        covered.split_whitespace().count(),
        14,
        "reachable event coverage: {covered}"
    );
    assert_eq!(
        failed, 0,
        "first failing seed and transition trace: {first:#x?}"
    );
}

struct Round {
    children: u64,
    batch: u64,
    quit: &'static str,
    late: bool,
    early: bool,
}

impl Round {
    #[expect(
        clippy::cast_possible_truncation,
        reason = "a pick below three fits usize"
    )]
    fn from_seed(seed: u64) -> Self {
        let mut random = Random(seed);
        Self {
            children: 2 + random.pick(3),
            batch: 1 + random.pick(6),
            quit: ["nil", "filter", "sentinel"][random.pick(3) as usize],
            late: random.pick(2) == 1,
            early: random.pick(4) == 0,
        }
    }

    fn start(&self) -> String {
        let Self {
            children,
            batch,
            quit,
            late,
            ..
        } = self;
        let launch = if *late {
            "(isolate-send 'parent 'a-enter) (isolate-receive 10)"
        } else {
            ""
        };
        let ready = if *late {
            ""
        } else {
            "(isolate-send 'parent 'a-enter) (isolate-receive 10)"
        };
        format!(
            r#"(setq a (owner-call owner '(lambda ()
                   (setq owner-output-batch {batch})
                   {launch}
                   (let ((children nil))
                     (dotimes (i {children})
                       (push (make-process :name (format "walk-%d" i)
                              :command '("/bin/sh" "-c" "printf x") :noquery t
                              :connection-type 'pipe) children))
                     {REAP_BEFORE_DISPATCH}
                     (let ((first (car (process-list))))
                       (dolist (p children)
                         (set-process-filter p (lambda (p _)
                           (isolate-send 'parent (list 'filter (process-name p)))
                           (when (and (eq p first) (eq '{quit} 'filter)) (signal 'quit nil))))
                         (set-process-sentinel p (lambda (p _)
                           (isolate-send 'parent (list 'sentinel (process-name p)))
                           (when (and (eq p first) (eq '{quit} 'sentinel)) (signal 'quit nil)))))))
                   {ready}
                   (isolate-send 'parent 'a-end))))"#
        )
    }

    fn check(&self, master: &mut Master) -> String {
        let expected = 3 + 2 * self.children;
        master.eval(&format!(
            r#"(let ((end (+ (float-time) 1.0)))
              (while (and (< (length owner-test-events) {expected}) (< (float-time) end))
                (romacs--isolate-pump 0.02))
              (let ((ordered (reverse owner-test-events))
                    (ok t) (output-before-b nil) (past-b nil))
                (dotimes (i {})
                  (let ((seen 0) (name (format "walk-%d" i)))
                    (dolist (e ordered)
                      (when (and (eq (car-safe e) 'sentinel) (equal (nth 1 e) name))
                        (setq seen (1+ seen))))
                    (unless (= seen 1) (setq ok nil))))
                (dolist (e ordered)
                  (when (eq e 'b) (setq past-b t))
                  (when (and (not past-b) (eq (car-safe e) 'filter))
                    (setq output-before-b t)))
                (if (and ok output-before-b past-b) nil
                  (list ok output-before-b ordered))))"#,
            self.children
        ))
    }
}

fn walk_round(master: &mut Master, round: &Round) -> String {
    record_owner_events(master);
    let early = if round.early {
        initial_call(master);
        assert_eq!(master.eval("(isolate-await first 15)"), "armed");
        let events = initial_call_events(master);
        record_owner_events(master);
        events
    } else {
        master.run("(setq owner (owner--spawn)) (isolate-await (owner--start-future owner) 10)");
        "(t t)".into()
    };
    master.until(SETUP_TIMEOUT, "owner parked in its idle wait", |m| {
        m.eval("(cdr (assq 'state (process-attributes (plist-get (owner-describe owner) :pid))))")
            == r#""S""#
    });
    master.run(&round.start());
    master.until(SETUP_TIMEOUT, "worker entered A", |m| {
        m.eval("(memq 'a-enter owner-test-events)") != "nil"
    });
    master.run(
        "(setq b (owner-call owner '(lambda () (isolate-send 'parent 'b))))
                (isolate-send (owner--isolate owner) 'release)
                (isolate-await a 10)
                (isolate-await b 10)",
    );
    let output = round.check(master);
    let deleted = master.eval(&format!(
        "(isolate-await (owner-call owner '(lambda () {})) 15)",
        delete_last_process_form()
    ));
    master.run("(owner-kill owner)");
    master.until(SETUP_TIMEOUT, "walk owner reaped", |m| {
        m.eval("(owner-state owner)") == "dead"
    });
    if early == "(t t)" && output == "nil" && deleted == "(t nil nil nil)" {
        "nil".into()
    } else {
        format!("early callbacks {early}, ordering {output}, last process {deleted}")
    }
}

#[test]
fn seeded_worker_walk_checks_sentinels_and_output_between_calls() {
    let now = Instant::now();
    let mut master = Master::bare();
    master.run("(setq owner-wedge-delay 300.0)");
    let start = FIRST_SEED + TRANSITION_SEQUENCES;
    let end = start + WORKER_SEQUENCES;
    eprintln!("worker walk seeds {start:#x}..{end:#x} (exclusive)");
    let mut failed = 0;
    let mut first = None;
    for seed in start..end {
        eprintln!("worker seed {seed:#x}");
        let round = Round::from_seed(seed);
        let result = if round.early {
            // An already-forked spare does not inherit the zygote's
            // current round when the owner takes it.
            let mut cold = Master::bare();
            cold.run("(setq owner-wedge-delay 300.0)");
            walk_round(&mut cold, &round)
        } else {
            walk_round(&mut master, &round)
        };
        if result != "nil" {
            failed += 1;
            first.get_or_insert((seed, result));
        }
    }
    eprintln!(
        "worker walk {failed}/{WORKER_SEQUENCES} failed in {:?}",
        now.elapsed()
    );
    assert_eq!(
        failed, 0,
        "first failing seed and observations: {first:#x?}"
    );
}
