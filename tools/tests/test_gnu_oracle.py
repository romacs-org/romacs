"""Re-pinning ledger/parity-reference.toml through tools/lib/gnu_oracle.py.

Run: python3 -m unittest discover -s tools/tests -p test_gnu_oracle.py -v

Every GNU here is a fake: a shell script that answers the two batch probes
re-pinning asks, beside a dump file that has GNU's header. No test runs a real
GNU or touches the checked-in reference; each pins a copy.
"""

import contextlib
import io
import pathlib
import subprocess
import sys
import tempfile
import unittest

ROOT = pathlib.Path(__file__).resolve().parents[2]
sys.path.insert(0, str(ROOT / "tools/lib"))
import gnu_oracle  # noqa: E402

TODAY = "2026-11-17"
OLD_ENTRY = "#   2026-01-01  an older entry"


def fake_gnu(directory, version, build_time, fingerprint):
    """A GNU at DIRECTORY/emacs, its dump beside it with FINGERPRINT's byte 32 times."""
    directory.mkdir(parents=True, exist_ok=True)
    script = directory / "emacs"
    script.write_text("#!/bin/sh\n"
                      'case "$4" in\n'
                      f'  "(princ emacs-version)") printf %s "{version}" ;;\n'
                      f'  *emacs-build-time*) printf %s "{build_time}" ;;\n'
                      "  *) exit 3 ;;\n"
                      "esac\n")
    script.chmod(0o755)
    write_dump(directory / "emacs.pdmp", fingerprint)
    return script


def write_dump(path, fingerprint):
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_bytes(gnu_oracle.DUMP_MAGIC + fingerprint * 32 + b"the rest of the dump")


def keys(**changes):
    fields = dict(schema="1", emacs_version="31.0.90", mirror_commit="a" * 40, build_time="t",
                  fingerprint="b" * 64, executable_sha256="c" * 64, executable_size="1",
                  pdmp_sha256="d" * 64, pdmp_size="2")
    fields.update(changes)
    return gnu_oracle.render_keys(fields)


class RepinTest(unittest.TestCase):
    def setUp(self):
        directory = tempfile.TemporaryDirectory(prefix="gnu-oracle-")
        self.addCleanup(directory.cleanup)
        self.root = pathlib.Path(directory.name)
        self.reference = self.root / "parity-reference.toml"
        self.reference.write_text(f"# a header\n#\n{gnu_oracle.LOG_HEADER}\n{OLD_ENTRY}\n\n{keys()}")

    def repin(self, *argv):
        with contextlib.redirect_stdout(io.StringIO()) as out:
            gnu_oracle.repin(list(argv), reference=self.reference, today=TODAY)
        return out.getvalue()

    def refused(self, *argv):
        before = self.reference.read_bytes()
        with self.assertRaises(gnu_oracle.RepinError) as caught, \
                contextlib.redirect_stdout(io.StringIO()):
            gnu_oracle.repin(list(argv), reference=self.reference, today=TODAY)
        self.assertEqual(self.reference.read_bytes(), before, "a refusal writes nothing")
        return str(caught.exception)

    def mirror(self, fingerprint):
        """A clean git checkout holding a fake GNU, and its commit."""
        mirror = self.root / "mirror"
        emacs = fake_gnu(mirror, "32.0.50", "2026-10-20T21:59:23+0300", fingerprint)
        git = ["git", "-C", str(mirror), "-c", "user.name=t", "-c", "user.email=t@t"]
        subprocess.run([*git, "init", "-q"], check=True)
        subprocess.run([*git, "add", "-A"], check=True)
        subprocess.run([*git, "commit", "-qm", "build"], check=True)
        head = subprocess.run([*git, "rev-parse", "HEAD"], check=True, capture_output=True, text=True)
        return emacs, head.stdout.strip()

    def test_pinning_twice_rewrites_the_keys_and_logs_both_newest_first(self):
        emacs, commit = self.mirror(b"\x01")
        self.repin("--emacs", str(emacs), "--reason", "the mirror was rebuilt")
        rebuilt = fake_gnu(self.root / "rebuilt", "32.0.50", "", b"\x02")
        self.repin("--emacs", str(rebuilt), "--reason", "  rebuilt again for profiling  ",
                   "--mirror-commit", "e" * 40, "--build-time", "2026-11-01T00:00:00+00:00")

        text = self.reference.read_text()
        header = text.index(gnu_oracle.LOG_HEADER + "\n") + len(gnu_oracle.LOG_HEADER) + 1
        self.assertEqual(
            text[header:].split("\n")[:5],
            [f"#   {TODAY}  GNU Emacs 32.0.50 built 2026-11-01T00:00:00+00:00 from mirror commit eeeeeeeeeee.",
             "#               rebuilt again for profiling",
             f"#   {TODAY}  GNU Emacs 32.0.50 built 2026-10-20T21:59:23+03:00 from mirror commit {commit[:11]}.",
             "#               the mirror was rebuilt",
             OLD_ENTRY])
        self.assertTrue(text.startswith("# a header\n#\n"), "comments survive")
        observed = gnu_oracle.observe(rebuilt)
        self.assertEqual(gnu_oracle.parse_reference(text), dict(
            schema="1", emacs_version="32.0.50", mirror_commit="e" * 40,
            build_time="2026-11-01T00:00:00+00:00",
            **{key: observed[key] for key in gnu_oracle.KEYS[4:]}))
        self.assertEqual(observed["fingerprint"], "02" * 32)
        self.assertTrue(text.endswith(gnu_oracle.render_keys(gnu_oracle.parse_reference(text))))

        before = self.reference.read_bytes()
        same = ("--emacs", str(rebuilt), "--mirror-commit", "e" * 40,
                "--build-time", "2026-11-01T00:00:00+00:00", "--reason", "nothing changed")
        self.assertIn("nothing to re-baseline", self.repin(*same))
        self.assertIn("was not written", self.repin("--emacs", str(emacs), "--reason", "r", "--dry-run"))
        self.assertEqual(self.reference.read_bytes(), before, "an unchanged pin and a dry run write nothing")

    def test_a_long_reason_wraps_under_its_entry(self):
        emacs = fake_gnu(self.root / "gnu", "32.0.50", "", b"\x03")
        reason = " ".join(["word"] * 30)
        self.repin("--emacs", str(emacs), "--reason", reason, "--mirror-commit", "f" * 40,
                   "--build-time", "stamp")
        lines = [line for line in self.reference.read_text().split("\n") if line.startswith("#               ")]
        self.assertEqual(" ".join(line.split(None, 1)[1] for line in lines), reason)
        self.assertTrue(all(len(line) <= 78 for line in lines), lines)

    def test_a_reason_is_required_and_may_not_be_blank(self):
        emacs = fake_gnu(self.root / "gnu", "32.0.50", "s", b"\x04")
        self.assertIn("must not be empty", self.refused("--emacs", str(emacs), "--reason", " \t\n"))
        with self.assertRaises(SystemExit), contextlib.redirect_stderr(io.StringIO()):
            gnu_oracle.repin(["--emacs", str(emacs)], reference=self.reference)
        self.assertIn("--build-time must not be blank",
                      self.refused("--emacs", str(emacs), "--reason", "r", "--build-time", " "))

    def test_a_build_without_a_build_time_is_refused_not_given_the_current_time(self):
        emacs = fake_gnu(self.root / "gnu", "32.0.50", "", b"\x05")
        error = self.refused("--emacs", str(emacs), "--reason", "r", "--mirror-commit", "f" * 40)
        self.assertIn("--build-time", error)

    def test_a_dirty_mirror_or_no_checkout_is_refused(self):
        emacs, _ = self.mirror(b"\x06")
        (emacs.parent / "stray").write_text("x")
        self.assertIn("uncommitted changes", self.refused("--emacs", str(emacs), "--reason", "r"))
        outside = fake_gnu(self.root / "outside", "32.0.50", "s", b"\x07")
        if subprocess.run(["git", "-C", str(outside.parent), "rev-parse"], capture_output=True).returncode:
            self.assertIn("--mirror-commit", self.refused("--emacs", str(outside), "--reason", "r"))

    def test_a_reference_without_a_log_or_keys_is_refused(self):
        emacs = fake_gnu(self.root / "gnu", "32.0.50", "s", b"\x08")
        argv = ("--emacs", str(emacs), "--reason", "r", "--mirror-commit", "f" * 40)
        self.reference.write_text(keys())
        self.assertIn(gnu_oracle.LOG_HEADER, self.refused(*argv))
        with self.assertRaises(gnu_oracle.RepinError):
            gnu_oracle.rewrite(f"{gnu_oracle.LOG_HEADER}\n", gnu_oracle.parse_reference(keys()), "r", TODAY)

    def test_an_unreadable_value_is_never_written(self):
        emacs = fake_gnu(self.root / "gnu", "32.0.50", "s", b"\x09")
        self.assertIn("unreadable pin",
                      self.refused("--emacs", str(emacs), "--reason", "r", "--mirror-commit", "short"))

    def test_an_installed_layout_reads_its_libexec_dump(self):
        prefix = self.root / "prefix"
        emacs = fake_gnu(prefix / "bin", "33.1", "s", b"\x0a")
        (prefix / "bin/emacs.pdmp").unlink()
        write_dump(prefix / "libexec/emacs/33.1/x86/emacs-zz.pdmp", b"\x0b")
        write_dump(prefix / "libexec/emacs/33.1/x86/emacs-aa.pdmp", b"\x0c")
        observed = gnu_oracle.observe(emacs)
        self.assertEqual(observed["pdmp"].name, "emacs-aa.pdmp")
        self.assertEqual(observed["fingerprint"], "0c" * 32)

    def test_an_installed_layout_is_found_through_a_symlinked_directory(self):
        prefix = self.root / "prefix"
        emacs = fake_gnu(prefix / "bin", "33.1", "s", b"\x0e")
        (prefix / "bin/emacs.pdmp").unlink()
        write_dump(self.root / "elsewhere/x86/emacs-ff.pdmp", b"\x0f")
        (prefix / "libexec/emacs").mkdir(parents=True)
        (prefix / "libexec/emacs/33.1").symlink_to(self.root / "elsewhere")
        self.assertEqual(gnu_oracle.observe(emacs)["fingerprint"], "0f" * 32)

    def test_a_size_spelt_differently_is_not_a_change(self):
        emacs = fake_gnu(self.root / "gnu", "32.0.50", "s", b"\x10")
        argv = ("--emacs", str(emacs), "--mirror-commit", "f" * 40, "--build-time", "stamp")
        self.repin(*argv, "--reason", "first")
        observed = gnu_oracle.observe(emacs)
        text = self.reference.read_text().replace(
            f'executable_size = "{observed["executable_size"]}"', f'executable_size = "+{observed["executable_size"]}"'
        ).replace(f'pdmp_size = "{observed["pdmp_size"]}"', f'pdmp_size = "00{observed["pdmp_size"]}"')
        self.reference.write_text(text)
        self.assertIn("nothing to re-baseline", self.repin(*argv, "--reason", "same build"))
        self.assertEqual(self.reference.read_text(), text)

    def test_a_file_that_is_not_a_dump_is_named_as_such(self):
        emacs = fake_gnu(self.root / "gnu", "32.0.50", "s", b"\x0d")
        (self.root / "gnu/emacs.pdmp").write_bytes(b"NOTADUMP" * 8)
        with self.assertRaisesRegex(gnu_oracle.RepinError, "not a GNU dump"):
            gnu_oracle.observe(emacs)

    def test_a_zone_offset_is_written_the_way_the_pin_records_it(self):
        self.assertEqual(gnu_oracle.insert_offset_colon("2026-06-10T02:39:56-0400"), "2026-06-10T02:39:56-04:00")
        self.assertEqual(gnu_oracle.insert_offset_colon("2026-06-10T02:39:56+0000"), "2026-06-10T02:39:56+00:00")
        self.assertEqual(gnu_oracle.insert_offset_colon("no offset"), "no offset")


class ParseReferenceTest(unittest.TestCase):
    def test_the_checked_in_reference_parses_and_round_trips(self):
        fields = gnu_oracle.parse_reference(gnu_oracle.REFERENCE.read_text())
        self.assertEqual(fields["schema"], "1")
        self.assertEqual(gnu_oracle.parse_reference(gnu_oracle.render_keys(fields)), fields)

    def test_the_parser_refuses_rather_than_skips(self):
        base = keys()
        for what, text in [
                ("a line the format does not define", base + "[table]\n"),
                ("a duplicate key", base + 'pdmp_size = "3"\n'),
                ("an unknown key", base + 'pdmp_sha512 = "e"\n'),
                ("an unquoted value", base.replace('pdmp_size = "2"', "pdmp_size = 2")),
                ("an escape", base.replace('build_time = "t"', 'build_time = "t\\\\"')),
                ("a non-hex digest", base.replace("d" * 64, "z" * 64)),
                ("an upper-case digest", base.replace("d" * 64, "D" * 64)),
                ("a truncated digest", base.replace("d" * 64, "d" * 63)),
                ("a truncated fingerprint", base.replace("b" * 64, "b" * 63)),
                ("a negative size", base.replace('pdmp_size = "2"', 'pdmp_size = "-2"')),
                ("a size past u64", base.replace('pdmp_size = "2"', f'pdmp_size = "{1 << 64}"')),
                ("an unrecognised schema", base.replace('schema = "1"', 'schema = "2"')),
                ("a missing key", 'schema = "1"\n')]:
            with self.subTest(what), self.assertRaises(ValueError):
                gnu_oracle.parse_reference(text)
        gnu_oracle.parse_reference(f"# a comment\n\n{base}\n   \n# trailing note\n")
        self.assertEqual(gnu_oracle.parse_reference(base.replace('pdmp_size = "2"', 'pdmp_size = "+002"'))
                         ["pdmp_size"], "2", "a size is read as a u64 and kept in one spelling")


if __name__ == "__main__":
    unittest.main()
