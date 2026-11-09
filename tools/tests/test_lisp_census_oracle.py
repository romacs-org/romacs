"""Integration tests for the real GNU reader, not a Python Lisp parser.

Run: python3 -m unittest discover -s scripts -p test_lisp_census_oracle.py -v
Set EMACS to select a GNU executable.  Every invocation uses -Q and a private
HOME.  Load-mode tests execute only these disposable test fixtures.
"""

import json
import os
from pathlib import Path
import shutil
import subprocess
import tempfile
import unittest


ORACLE = Path(__file__).with_name("lisp-census-oracle.el").resolve()
EMACS = shutil.which(os.environ.get("EMACS", "emacs"))


@unittest.skipUnless(EMACS, "GNU Emacs executable unavailable (set EMACS)")
class CensusOracleTest(unittest.TestCase):
    def setUp(self):
        self.directory = tempfile.TemporaryDirectory(prefix="lisp-census-")
        self.addCleanup(self.directory.cleanup)
        self.root = Path(self.directory.name)
        self.env = dict(os.environ, HOME=str(self.root), XDG_CONFIG_HOME=str(self.root))

    def source(self, text, name="fixture.el"):
        path = self.root / name
        path.write_bytes(text.encode("utf-8"))
        return path

    def invoke(self, *args):
        assert EMACS is not None
        return subprocess.run(
            [EMACS, "-Q", "--batch", "--script", str(ORACLE), *map(str, args)],
            env=self.env,
            capture_output=True,
            text=True,
            timeout=20,
            check=False,
        )

    def split(self, paths):
        manifest = self.root / "manifest.json"
        manifest.write_text(json.dumps([str(path) for path in paths]), encoding="utf-8")
        result = self.invoke("--manifest", manifest)
        self.assertEqual(result.returncode, 0, result.stderr + result.stdout)
        report = json.loads(result.stdout)
        self.assertEqual(report["schema_version"], 1)
        self.assertEqual(report["mode"], "split")
        self.assertEqual(len(report["files"]), len(paths))
        self.assertEqual([item["path"] for item in report["files"]], list(map(str, paths)))
        return report["files"]

    def test_trivial_spans_are_exact_python_slices(self):
        text = "; comment\n  (setq answer 42)\n\n(provide 'fixture) ; tail\n"
        report, = self.split([self.source(text)])
        self.assertEqual(report["status"], "ok")
        self.assertIsNone(report["error"])
        self.assertEqual(report["character_count"], len(text))
        self.assertEqual(report["form_count"], 2)
        self.assertEqual(
            report["forms"],
            [
                {"start": text.index("(setq"), "end": text.index(")") + 1,
                 "line": 2, "head": "setq"},
                {"start": text.index("(provide"), "end": text.rindex(")") + 1,
                 "line": 4, "head": "provide"},
            ],
        )
        self.assertEqual(
            [text[form["start"]:form["end"]] for form in report["forms"]],
            ["(setq answer 42)", "(provide 'fixture)"],
        )

    def test_unicode_and_crlf_preserve_character_offsets(self):
        text = '; λ😀\r\n(message "κόσμε😀")\r\n\u00a0(quote τέλος)\r\n'
        report, = self.split([self.source(text)])
        self.assertEqual(report["status"], "ok")
        self.assertEqual(report["character_count"], len(text))
        self.assertEqual([form["line"] for form in report["forms"]], [2, 3])
        self.assertEqual(
            [text[form["start"]:form["end"]] for form in report["forms"]],
            ['(message "κόσμε😀")', "(quote τέλος)"],
        )

    def test_empty_and_comment_only_are_clean_eof(self):
        for text in ("", " \t\r\n\f\u00a0", "; last comment", "; first\n; last\n"):
            with self.subTest(text=text):
                report, = self.split([self.source(text)])
                self.assertEqual(report["status"], "ok")
                self.assertEqual(report["form_count"], 0)
                self.assertEqual(report["forms"], [])
                self.assertIsNone(report["error"])

    def test_premature_eof_is_not_clean_eof(self):
        for tail in ("(", '(message "unfinished', "'", "[1 2", "(a ; final"):
            with self.subTest(tail=tail):
                report, = self.split([self.source("(ok)\n" + tail)])
                self.assertEqual(report["status"], "parse-error")
                self.assertEqual(report["form_count"], 1)
                self.assertEqual(report["error"]["condition"], "end-of-file")
                self.assertTrue(report["error"]["premature_eof"])
                self.assertEqual(report["error"]["start"], 5)
                self.assertEqual(report["error"]["line"], 2)

    def test_reader_error_stops_without_resynchronization(self):
        report, = self.split([self.source("(ok)\n)\n(after)")])
        self.assertEqual(report["status"], "parse-error")
        self.assertEqual(report["form_count"], 1)
        self.assertEqual(report["error"]["condition"], "invalid-read-syntax")
        self.assertFalse(report["error"]["premature_eof"])
        self.assertEqual(report["error"]["line"], 2)

    def test_split_never_evaluates_even_errors_or_writes(self):
        marker = self.root / "must-not-exist"
        text = f'(write-region "evaluated" nil {json.dumps(str(marker))})\n(error "stop")\n(after)'
        report, = self.split([self.source(text)])
        self.assertEqual(report["status"], "ok")
        self.assertEqual(report["form_count"], 3)
        self.assertFalse(marker.exists())

    def test_many_files_duplicates_and_errors_stay_in_manifest_order(self):
        good = self.source("(good)", 'space λ "quote".el')
        bad = self.source("(bad", "bad.el")
        missing = self.root / "missing.el"
        reports = self.split([good, bad, missing, good])
        self.assertEqual([item["status"] for item in reports],
                         ["ok", "parse-error", "io-error", "ok"])
        self.assertEqual([item["form_count"] for item in reports], [1, 0, 0, 1])
        self.assertEqual(reports[0], reports[3])

    def test_atoms_quoted_and_circular_forms_are_bounded(self):
        text = 'nil 42 "hi" \'name [x] #1=(node . #1#)'
        report, = self.split([self.source(text)])
        self.assertEqual(report["status"], "ok")
        self.assertEqual(report["form_count"], 6)
        self.assertEqual([form["head"] for form in report["forms"]],
                         [None, None, None, "quote", None, "node"])
        self.assertEqual([text[form["start"]:form["end"]] for form in report["forms"]],
                         ['nil', '42', '"hi"', "'name", '[x]', '#1=(node . #1#)'])

    def test_invalid_manifest_and_cli_are_infrastructure_errors(self):
        for text in ('{}', '[1]', 'null', '{broken'):
            with self.subTest(text=text):
                manifest = self.root / "bad.json"
                manifest.write_text(text, encoding="utf-8")
                result = self.invoke("--manifest", manifest)
                self.assertEqual(result.returncode, 2, result.stderr)
                self.assertIn("error", json.loads(result.stdout))
        self.assertEqual(self.invoke().returncode, 2)
        self.assertEqual(self.split([]), [])

    def test_optional_load_reports_first_error_and_does_not_run_later_forms(self):
        before = self.root / "before"
        after = self.root / "after"
        text = (
            f'(write-region "yes" nil {json.dumps(str(before))})\n'
            '(princ "not JSON")\n(error "first failure")\n'
            f'(write-region "no" nil {json.dumps(str(after))})\n'
        )
        source = self.source(text)
        result = self.invoke("--load-file", source)
        self.assertEqual(result.returncode, 0, result.stderr + result.stdout)
        report = json.loads(result.stdout.splitlines()[-1])
        self.assertEqual(report["mode"], "load")
        verdict, = report["files"]
        self.assertEqual(verdict["status"], "load-error")
        self.assertEqual(verdict["error"]["condition"], "error")
        self.assertIn("first failure", verdict["error"]["message"])
        self.assertTrue(before.exists())
        self.assertFalse(after.exists())

    def test_optional_load_success_and_premature_eof(self):
        for text, expected in (("(setq x 1)", "ok"), ("(setq x 1)\n(", "load-error")):
            with self.subTest(text=text):
                result = self.invoke("--load-file", self.source(text))
                self.assertEqual(result.returncode, 0, result.stderr + result.stdout)
                verdict, = json.loads(result.stdout)["files"]
                self.assertEqual(verdict["status"], expected)
                if expected == "load-error":
                    self.assertTrue(verdict["error"]["premature_eof"])


if __name__ == "__main__":
    unittest.main()
