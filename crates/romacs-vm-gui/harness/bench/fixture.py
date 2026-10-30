"""A generic project for the stock profile: mixed files and a fixed history.

Every file and commit is written here, with fixed authors and dates, so
both engines and every replay see the same tree and the same hashes:
Emacs Lisp whose functions call each other (xref), C and Rust, Markdown,
an Org file with an outline, TODO items and a table, and a Makefile with
cheap targets.  Nothing is copied from anyone's files.
"""
import os
from pathlib import Path
import subprocess

FILES = {
    'README.md': '''# Tally

Tally counts words and lines in text files.  It is a small project used
to exercise an editor: Lisp, C, Rust, Markdown, Org and a Makefile.

## Building

Run `make` to build the C tool and `make check` to run its tests.

## Layout

- `lisp/tally.el` is the Emacs front end.
- `src/tally.c` is the command-line counter.
- `src/lib.rs` is a Rust port of the counting rules.
- `notes.org` tracks the work.
''',
    'Makefile': '''CC ?= cc
CFLAGS ?= -O2 -Wall

all: tally

tally: src/tally.c
\t$(CC) $(CFLAGS) -o tally src/tally.c

check:
\t@echo "tally: 3 checks passed"

clean:
\trm -f tally

.PHONY: all check clean
''',
    'lisp/tally.el': ''';;; tally.el --- Count words and lines  -*- lexical-binding: t; -*-

;;; Commentary:
;; The Emacs front end of Tally.

;;; Code:

(defgroup tally nil
  "Count words and lines."
  :group 'tools)

(defcustom tally-ignore-blank-lines t
  "Non-nil means blank lines are not counted."
  :type 'boolean)

(defun tally-count-lines (start end)
  "Return the number of lines between START and END."
  (let ((count 0))
    (save-excursion
      (goto-char start)
      (while (< (point) end)
        (unless (and tally-ignore-blank-lines (looking-at-p "^$"))
          (setq count (1+ count)))
        (forward-line 1)))
    count))

(defun tally-count-words (start end)
  "Return the number of words between START and END."
  (let ((count 0))
    (save-excursion
      (goto-char start)
      (while (and (< (point) end) (forward-word 1))
        (setq count (1+ count))))
    count))

(defun tally-region (start end)
  "Report the lines and words between START and END."
  (interactive "r")
  (message "%d lines, %d words"
           (tally-count-lines start end)
           (tally-count-words start end)))

(defun tally-buffer ()
  "Report the lines and words of the buffer."
  (interactive)
  (tally-region (point-min) (point-max)))

(provide 'tally)
;;; tally.el ends here
''',
    'src/tally.c': '''#include <ctype.h>
#include <stdio.h>

struct counts {
\tlong lines;
\tlong words;
};

static struct counts
count(FILE *in)
{
\tstruct counts c = {0, 0};
\tint ch, inword = 0;

\twhile ((ch = getc(in)) != EOF) {
\t\tif (ch == '\\n')
\t\t\tc.lines++;
\t\tif (isspace(ch))
\t\t\tinword = 0;
\t\telse if (!inword) {
\t\t\tinword = 1;
\t\t\tc.words++;
\t\t}
\t}
\treturn c;
}

int
main(void)
{
\tstruct counts c = count(stdin);
\tprintf("%ld lines, %ld words\\n", c.lines, c.words);
\treturn 0;
}
''',
    'src/lib.rs': '''//! Tally's counting rules.

/// Lines and words in a text.
#[derive(Debug, Default, PartialEq)]
pub struct Counts {
    pub lines: usize,
    pub words: usize,
}

/// Count the lines and words of TEXT.
pub fn count(text: &str) -> Counts {
    Counts {
        lines: text.lines().count(),
        words: text.split_whitespace().count(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn counts_words_and_lines() {
        assert_eq!(count("a b\\nc\\n"), Counts { lines: 2, words: 3 });
    }
}
''',
    'notes.org': '''#+title: Tally notes

* Plan
** TODO Count words in the region
   The Emacs front end counts words with =forward-word=.
** DONE Count lines
   Blank lines are skipped when =tally-ignore-blank-lines= is set.
* Measurements
| file        | lines | words |
|-------------+-------+-------|
| README.md   |    17 |    80 |
| src/tally.c |    35 |    90 |
* Ideas
- Count characters too.
- Report the longest line.
''',
    'docs/guide.md': '''# Guide

Call `tally-buffer` to count the current buffer, or `tally-region` for
the region.  The C tool reads standard input:

    ./tally < README.md
''',
}

# (message, files written in this commit, day)
HISTORY = [
    ('Add the C counter and its Makefile', ['src/tally.c', 'Makefile'], 1),
    ('Add the Emacs front end', ['lisp/tally.el'], 2),
    ('Document Tally', ['README.md', 'docs/guide.md'], 3),
    ('Port the counting rules to Rust', ['src/lib.rs'], 4),
    ('Track the work in Org', ['notes.org'], 5),
]


def make_repo(target):
    """Write the project at TARGET, a new directory, with its history."""
    target = Path(target)
    target.mkdir(parents=True)
    env = {'PATH': os.environ['PATH'], 'HOME': str(target), 'GIT_CONFIG_NOSYSTEM': '1',
           'GIT_CONFIG_GLOBAL': os.devnull, 'GIT_AUTHOR_NAME': 'Tally Author',
           'GIT_AUTHOR_EMAIL': 'author@example.org', 'GIT_COMMITTER_NAME': 'Tally Author',
           'GIT_COMMITTER_EMAIL': 'author@example.org'}
    git = lambda *args, **extra: subprocess.run(['git', '-C', str(target), *args], env={**env, **extra},
                                                check=True, capture_output=True, timeout=60)
    git('init', '-q', '-b', 'main')
    for message, names, day in HISTORY:
        for name in names:
            path = target / name
            path.parent.mkdir(parents=True, exist_ok=True)
            path.write_text(FILES[name])
        git('add', *names)
        date = f'2026-01-0{day}T12:00:00+0000'
        git('commit', '-q', '-m', message, GIT_AUTHOR_DATE=date, GIT_COMMITTER_DATE=date)
    return target
