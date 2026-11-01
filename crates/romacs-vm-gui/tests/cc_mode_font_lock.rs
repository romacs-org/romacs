//! c-mode on a real C file, through jit-lock as redisplay drives it, against
//! pinned GNU: every face run, the keyword list, and no `*Compile-Log*`.
//! cc-fonts.el byte-compiles its matchers while it loads, where GNU loads
//! the .elc it compiled them into; that compilation must succeed silently,
//! or C buffers show `(void-function t)' and a compile log GNU never shows.
//! Both sessions run in one private HOME and directory; the test compares
//! exit status and stdout.
use std::process::Command;

const SCRIPT: &str = r#";;; -*- lexical-binding: t -*-
(with-current-buffer (find-file-noselect "sample.c")
  (prin1 (list major-mode font-lock-maximum-decoration (length (cddr font-lock-keywords))
               (seq-count (lambda (k) (eq (car-safe k) t)) (cddr font-lock-keywords))))
  (terpri)
  (jit-lock-register #'font-lock-fontify-region)
  (put-text-property (point-min) (point-max) 'fontified nil)
  (jit-lock-fontify-now (point-min) (point-max))
  (let ((pos (point-min)))
    (while (< pos (point-max))
      (let ((next (next-single-property-change pos 'face nil (point-max))))
        (when (get-text-property pos 'face)
          (prin1 (list pos next (buffer-substring-no-properties pos next) (get-text-property pos 'face))) (terpri))
        (setq pos next))))
  (prin1 (list 'keywords (length (cddr font-lock-keywords))
               (seq-count (lambda (k) (eq (car-safe k) t)) (cddr font-lock-keywords))))
  (terpri)
  (prin1 (list 'compile-log (and (get-buffer "*Compile-Log*") t) (mapcar (lambda (w) (buffer-name (window-buffer w))) (window-list nil t))))
  (terpri))
"#;
const SAMPLE: &str = r#"/* A small C file. */
#include <stdio.h>
#define MAX(a, b) ((a) > (b) ? (a) : (b))

typedef struct point {
    int x, y;
} point_t;

enum color { RED, GREEN = 2, BLUE };

static const char *names[] = { "red", "green", "blue" };

static int
add (int a, int b)
{
    return a + b; // sum
}

int main(int argc, char **argv)
{
    point_t p = { 1, 2 };
    unsigned long total = 0;
    for (int i = 0; i < argc; i++) {
        if (argv[i] == NULL)
            continue;
        total += add(p.x, MAX(i, 3));
    }
    printf("%lu %s\n", total, names[RED]);
    return 0;
}
"#;

fn run(binary: &str, dir: &std::path::Path) -> (Option<i32>, String, String) {
    let output = Command::new(binary).args(["-Q", "--batch", "-l", "probe.el"]).current_dir(dir)
        .env("HOME", dir).env("XDG_CONFIG_HOME", dir.join("config")).env("XDG_CACHE_HOME", dir.join("cache"))
        .env("XDG_DATA_HOME", dir.join("data")).env("XDG_STATE_HOME", dir.join("state"))
        // A batch world is per HOME (src/batch.rs); this one boots from source.
        .env("ROMACS_WORLD", "off")
        .output().unwrap();
    let text = |bytes: Vec<u8>| String::from_utf8_lossy(&bytes).into_owned();
    (output.status.code(), text(output.stdout), text(output.stderr))
}

#[test]
fn c_mode_fontifies_through_jit_lock_as_pinned_gnu() {
    let gnu = std::env::var("ROMACS_GNU_ORACLE").expect("pinned GNU oracle");
    let root = std::env::temp_dir().join(format!("romacs-cc-mode-{}", std::process::id()));
    let run_in = |binary: &str, name: &str| {
        let dir = root.join(name);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("probe.el"), SCRIPT).unwrap();
        std::fs::write(dir.join("sample.c"), SAMPLE).unwrap();
        run(binary, &dir)
    };
    let expected = run_in(&gnu, "gnu");
    let actual = run_in(env!("CARGO_BIN_EXE_romacs-vm-gui"), "romacs");
    let _ = std::fs::remove_dir_all(&root);
    assert_eq!(expected.0, Some(0), "GNU: {}", expected.2);
    assert!(expected.1.contains("(compile-log nil"), "GNU output: {}", expected.1);
    assert_eq!((actual.0, &actual.1), (expected.0, &expected.1), "Romacs stderr: {}", actual.2);
}
