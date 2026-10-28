//! timefns.c `encode-time' against pinned GNU: decoded-time round trips,
//! date arithmetic, DST gaps and overlaps with each DST hint, fixed, POSIX
//! and TZif zones, fractional seconds, the obsolescent argument convention
//! and the errors. Each line runs in a fresh process, so mktime's cached
//! offset starts as GNU's does; zones glibc cannot use are UTC.
use romacs_vm::Vm;

/// One expression per line.
const CASES: &str = r###"
(list (encode-time '(0 0 0 1 1 2024 nil nil t)) (encode-time (decode-time 1700000000 "Europe/Athens")) (encode-time (decode-time 1700000000 "America/New_York")) (encode-time '(30 61 25 32 13 2023 nil -1 "Europe/Athens")) (encode-time '(-1 -1 -1 0 0 2024 nil nil t)))
(list (encode-time '(0 30 3 31 3 2024 nil -1 "Europe/Athens")) (encode-time '(0 30 3 31 3 2024 nil nil "Europe/Athens")) (encode-time '(0 30 3 31 3 2024 nil t "Europe/Athens")) (encode-time '(0 30 3 27 10 2024 nil -1 "Europe/Athens")) (encode-time '(0 30 3 27 10 2024 nil t "Europe/Athens")) (encode-time '(0 30 3 27 10 2024 nil nil "Europe/Athens")))
(list (encode-time '(0 0 12 15 1 2024 nil t "America/New_York")) (encode-time '(0 0 12 15 7 2024 nil nil "America/New_York")) (encode-time '(0 0 12 15 7 2024 nil -1 "America/New_York")))
(list (encode-time '(0 0 0 1 1 2024 nil nil 3600)) (encode-time '(0 0 0 1 1 2024 nil t 3600)) (encode-time '(0 0 0 1 1 2024 nil nil (3600 "X"))) (encode-time '(0 0 0 1 1 2024 nil nil "UTC0")) (encode-time '(0 0 0 1 1 2024 nil nil "EST5")) (encode-time '(0 0 0 1 1 2024 nil nil "<+0330>-3:30")) (encode-time '(0 0 0 1 1 2024)))
(list (encode-time '(1.5 0 0 1 1 2024 nil nil t)) (encode-time '((3 . 2) 0 0 1 1 2024 nil nil t)) (encode-time '((7 . 1) 0 0 1 1 2024 nil nil t)) (encode-time '((0 1 2 3) 0 0 1 1 2024 nil nil t)) (let ((current-time-list t)) (encode-time '(0 0 0 1 1 2024 nil nil t))) (let ((current-time-list t)) (encode-time '(0.25 0 0 1 1 1960 nil nil t))))
(list (encode-time 0 0 0 1 1 2024) (encode-time 0 0 0 1 1 2024 t) (encode-time 0 0 0 1 1 2024 'ignored 3600) (encode-time 0 0 0 1 1 2024 'ignored nil "Europe/Athens"))
(list (condition-case e (encode-time '(0 0 0)) (error e)) (condition-case e (encode-time 1 2 3) (error e)) (condition-case e (encode-time '(0 0 0 1 1 a nil nil t)) (error e)) (condition-case e (encode-time '(0 0 0 1 1 3000000000 nil nil t)) (error e)) (condition-case e (encode-time '(0 0 0 1 1 2024 nil nil "No/Such_Zone")) (error e)) (condition-case e (encode-time '(0 0 0 1 1 2024 . 5)) (error e)) (condition-case e (encode-time '(0 0 0 1 1 2024 nil)) (error e)) (condition-case e (encode-time '(0.5 0 0 1 1 2024 nil nil t 1)) (error e)))
(list (encode-time '(0 30 3 27 10 2024 nil -1 "Europe/Athens")) (encode-time '(0 30 1 3 11 2024 nil -1 "America/New_York")) (current-time-zone 0 '(3600 "X")) (current-time-zone 0 "No/Such_Zone") (current-time-zone 0 '(3600 "ABC")) (format-time-string "%Z %z" 0 "No/Such_Zone"))
(list (encode-time '(0 0 12 15 1 2024 nil t "Asia/Kolkata")) (encode-time '(0 0 12 15 1 2024 nil t "UTC0")) (encode-time '(0 0 12 15 7 2024 nil t "EST5")) (encode-time '(0 0 12 15 1 2024 nil nil "Asia/Kolkata")) (encode-time '(0 0 12 15 7 2024 nil nil "Europe/Athens")))
(list (encode-time '(0 0 12 15 7 2024 nil -1 "EST5EDT,M3.2.0,M11.1.0")) (encode-time '(0 0 12 15 1 2024 nil -1 "EST5EDT,M3.2.0,M11.1.0")) (decode-time 1721044800 "EST5EDT,M3.2.0,M11.1.0") (format-time-string "%Z %z" 1721044800 "EST5EDT,M3.2.0,M11.1.0") (current-time-zone 1704067200 "EST5EDT,M3.2.0,M11.1.0") (current-time-zone 0 "ABC") (current-time-zone 1704067200 "AEST-10AEDT,M10.1.0,M4.1.0/3") (current-time-zone 1719792000 "AEST-10AEDT,M10.1.0,M4.1.0/3") (current-time-zone 1719792000 "XYZ3ABC,J60/1,300/-1"))
(list (encode-time '(0 0 12 20 3 2000 nil -1 "FOO5BAR")) (encode-time '(0 0 12 1 7 2024 nil -1 "FOO3BAR")) (encode-time '(0 0 12 1 7 2040 nil -1 "FOO3BAR")) (current-time-zone 1710054000 "FOO3BAR") (current-time-zone 1710061200 "FOO3BAR") (current-time-zone 2224771200 "FOO3BAR") (current-time-zone 1719835200 "FOO3BAR4") (encode-time '(0 30 3 10 3 2024 nil -1 "FOO5BAR,M3.2.0/+3,M11.1.0/2")) (current-time-zone 0 ":") (current-time-zone 0 "") (current-time-zone 0 "ABC 5") (current-time-zone 1719835200 "XYZ3ABC 2,M3.2.0,M11.1.0"))
(list (encode-time '(0 0 4 3 11 2024 nil -1 "FOO3BAR")) (current-time-zone 1730602800 "FOO3BAR") (current-time-zone 1730606400 "FOO3BAR") (current-time-zone 1730613600 "FOO5BAR4") (current-time-zone 0 "FOO5BAR,M+3.2.0,M11.1.0") (current-time-zone 1719835200 "FOO5BAR,M 3.2.0,M11.1.0"))
(list (current-time-zone 1730602800 "UTC") (encode-time '(0 0 4 3 11 2024 nil -1 "FOO3BAR")) (current-time-zone 1730602800 "FOO3BAR") (encode-time '(0 0 4 3 11 2024 nil -1 "FOO3BAR")))
"###;

#[test]
fn encode_time_matches_pinned_gnu() {
    let binary = std::env::var("ROMACS_GNU_ORACLE").expect("pinned GNU oracle");
    let mut failures = Vec::new();
    for expression in CASES.lines().filter(|line| !line.is_empty()) {
        // As character codes, as tests/json.rs compares.
        let source = format!("(append (prin1-to-string (condition-case e {expression} (error (list 'signal e)))) nil)");
        let output = std::process::Command::new(&binary).args(["-Q", "--batch", "--eval", &format!("(prin1 {source})")]).output().unwrap();
        assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
        let expected = String::from_utf8(output.stdout).unwrap();
        let actual = Vm::new().eval_source(&source).and_then(|v| v.display());
        if actual.as_deref() != Ok(expected.as_str()) {
            let text = |codes: &str| codes.trim_matches(|c| c == '(' || c == ')').split_whitespace()
                .filter_map(|n| n.parse::<u32>().ok().and_then(char::from_u32)).collect::<String>();
            failures.push(format!("{expression}\n  GNU    {}\n  Romacs {:?}", text(&expected), actual.as_deref().map(text)));
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}
