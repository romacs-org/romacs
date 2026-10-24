"""Pinned-GNU controls for the strict ERT bootstrap measurement tools."""
import json
import os
from pathlib import Path
import subprocess
import tempfile
import unittest

ROOT = Path(__file__).resolve().parents[1]
ORACLE_ROOT = Path(os.environ.get("GNU_ORACLE_ROOT", Path.home() / ".cache/gnu-emacs-pin"))
PIN = __import__("tomllib").loads((ROOT / "parity-reference.toml").read_text())["mirror_commit"]
GNU = os.environ.get("ROMACS_GNU_ORACLE", str(ORACLE_ROOT / PIN / "bin/emacs"))
SOURCE = Path(os.environ.get("ROMACS_GNU_SOURCE", ORACLE_ROOT / "src"))


def normalize_graph(rows):
    return [dict(row, file="lisp/" + str(Path(row["file"]).relative_to(SOURCE / "lisp"))) for row in rows]


def run(*args):
    return subprocess.run([GNU, "--batch", "-Q", *map(str, args)], text=True,
                          capture_output=True, timeout=60, check=False)


class ClosureControls(unittest.TestCase):
    def test_recorded_require_graphs_reproduce(self):
        for mode in ("load", "all"):
            with self.subTest(mode=mode):
                result = run("-l", ROOT / "scripts/ert-require-closure.el", SOURCE / "lisp", mode)
                self.assertEqual(result.returncode, 0, result.stderr)
                expected = json.loads((ROOT / f"crates/romacs-vm/evidence/ert-closure/require-{mode}.json").read_text())
                self.assertEqual(normalize_graph(json.loads(result.stdout)), expected)

    def test_missing_require_is_fatal(self):
        result = run("-l", ROOT / "scripts/ert-require-closure.el", SOURCE / "lisp", "load", "ert-closure-nonexistent-library")
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("Unresolved dependency", result.stderr + result.stdout)

    def test_malformed_sources_are_fatal(self):
        scratch = tempfile.gettempdir()
        with tempfile.TemporaryDirectory(prefix="ert-closure-control-", dir=scratch) as directory:
            for suffix in ("(", ")"):
                with self.subTest(suffix=suffix):
                    (Path(directory) / "ert.el").write_text("(provide 'ert)\n" + suffix)
                    result = run("-l", ROOT / "scripts/ert-require-closure.el", directory, "load")
                    self.assertNotEqual(result.returncode, 0)
                    self.assertIn("Cannot read", result.stderr + result.stdout)

    def test_real_ert_success_and_failure_controls(self):
        for body, succeeds in (("(should t)", True), ("(should nil)", False), ("(ert-skip \"control\")", False)):
            with self.subTest(body=body):
                result = run("-l", SOURCE / "lisp/emacs-lisp/ert.el", "--eval",
                             f"(ert-deftest ert-closure-control () {body})",
                             "-l", ROOT / "scripts/ert-closure-run.el")
                self.assertEqual(result.returncode == 0, succeeds, result.stderr)
                if not succeeds:
                    self.assertIn("ERT run incomplete or unsuccessful", result.stderr + result.stdout)

    def test_zero_tests_is_not_execution(self):
        result = run("-l", SOURCE / "lisp/emacs-lisp/ert.el", "-l", ROOT / "scripts/ert-closure-run.el")
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("0 executed, 0 passed", result.stderr + result.stdout)


if __name__ == "__main__":
    unittest.main()
