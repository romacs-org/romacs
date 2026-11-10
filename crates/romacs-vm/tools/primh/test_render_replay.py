"""Provenance stays byte-exact while generated Rust follows the current API."""
import argparse
import hashlib
from pathlib import Path
import tempfile
import unittest
from unittest import mock

import render_replay


class ReplayRenderingTests(unittest.TestCase):
    def test_rendering_changes_only_the_import_and_display_api(self):
        for family in ("primh", "varh"):
            with self.subTest(family=family), tempfile.TemporaryDirectory() as directory:
                harness = render_replay.load_harness(family)
                specs = [str(render_replay.HERE / "specs" / "assoc.json")] if family == "primh" else []
                out = Path(directory) / "replay.rs"
                args = argparse.Namespace(spec=specs, out=str(out))
                harness.rust(args)
                original = out.read_text()
                render_replay.render(family, specs, str(out))
                expected = original.replace("use romacs_vm2::Vm;", "use romacs_vm::Vm;", 1)
                expected = expected.replace("vm.display(v)", "v.display()")
                self.assertEqual(out.read_text(), expected)
                self.assertIn("use romacs_vm::Vm;", out.read_text())

    def test_original_template_hash_and_unrelated_edit_refusal(self):
        for family in ("primh", "varh"):
            with self.subTest(family=family):
                harness = render_replay.load_harness(family)
                template = render_replay.HERE.parent / family / "replay.rs.in"
                read_bytes = Path.read_bytes
                if family == "primh":
                    spec = harness.load_spec(render_replay.HERE / "specs" / "assoc.json")
                    evidence = lambda: harness.evidence(spec)
                else:
                    evidence = harness.evidence
                recorded = evidence()
                self.assertEqual(recorded["replay_sha256"], hashlib.sha256(template.read_bytes()).hexdigest())

                def changed(path):
                    data = read_bytes(path)
                    return data + b"\n" if path == template else data

                with mock.patch.object(Path, "read_bytes", changed):
                    with self.assertRaisesRegex(ValueError, "replay_sha256 changed"):
                        evidence()
                    with tempfile.TemporaryDirectory() as directory:
                        out = Path(directory) / "refused.rs"
                        specs = [str(render_replay.HERE / "specs" / "assoc.json")] if family == "primh" else []
                        with self.assertRaisesRegex(ValueError, "replay_sha256 changed"):
                            render_replay.render(family, specs, str(out))
                        self.assertFalse(out.exists())


    def test_into_adds_each_missing_replay_once_under_the_same_template(self):
        with tempfile.TemporaryDirectory() as directory:
            target = Path(directory) / "primh.rs"
            assoc, cdr = (str(render_replay.HERE / "specs" / name) for name in ("assoc.json", "cdr-safe.json"))
            render_replay.render("primh", [assoc], str(target))
            first = target.read_text()
            self.assertEqual(len(render_replay.into([cdr, assoc], str(target))), 1)
            self.assertTrue(target.read_text().startswith(first))
            self.assertEqual(len(render_replay.REPLAY.findall(target.read_text())), 2)
            self.assertEqual(render_replay.into([cdr], str(target)), [])
            target.write_text(first.replace("use romacs_vm::Vm;", "use other::Vm;"))
            with self.assertRaisesRegex(ValueError, "does not begin with the template"):
                render_replay.into([cdr], str(target))

if __name__ == "__main__":
    unittest.main()
