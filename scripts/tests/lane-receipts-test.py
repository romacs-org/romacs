#!/usr/bin/env python3
"""Disposable CLI proofs; no product builds and no real evidence record writes."""
import hashlib
import json
import os
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest

SCRIPT = Path(__file__).resolve().parents[1] / "lane-receipts.py"


class ReceiptPruning(unittest.TestCase):
    def setUp(self):
        self.scratch = tempfile.TemporaryDirectory(dir=os.environ.get("TMPDIR", SCRIPT.parent))
        self.addCleanup(self.scratch.cleanup)
        self.base = Path(self.scratch.name).resolve()
        self.record = self.base / "record"
        (self.record / "reference").mkdir(parents=True)
        self.index = self.record / "reference/references.json"
        self.index.write_text(json.dumps({"other": ["preserved"], "receipts": []}))
        self.build = self.base / "build"
        self.lane = self.build / "example-target"
        self.lane.mkdir(parents=True)
        self.source = self.lane / "result/receipt.json"
        self.source.parent.mkdir()
        self.data = b'{"tree":"abc123","verdict":"RED","complete":false}\n'
        self.source.write_bytes(self.data)

    def run_cli(self, action, candidate=None, success=True):
        run = subprocess.run([sys.executable, str(SCRIPT), action,
                              str(candidate or self.lane), "--record", str(self.record),
                              "--build-root", str(self.build)], text=True, capture_output=True)
        self.assertEqual(run.returncode, 0 if success else 1, run.stdout + run.stderr)
        return run

    def preserve(self):
        self.run_cli("preserve")
        return json.loads(self.index.read_text())["receipts"][0]

    def test_orphan_refused_then_durable_copy_allows_real_build_prune(self):
        self.run_cli("prune-build", success=False)
        self.assertTrue(self.source.exists())
        entry = self.preserve()
        durable = self.record / entry["path"]
        self.assertEqual(durable.read_bytes(), self.data)
        self.assertEqual(hashlib.sha256(durable.read_bytes()).hexdigest(), entry["sha256"])
        self.assertEqual(entry["role"], "historical-evidence-not-acceptance")
        self.assertEqual(entry["tree"], "abc123")
        self.assertEqual(entry["verdict"], "RED")
        self.assertIs(entry["complete"], False)
        self.run_cli("prune-build")
        self.assertFalse(self.lane.exists())
        self.assertEqual(durable.read_bytes(), self.data)
        self.assertEqual(json.loads(self.index.read_text())["other"], ["preserved"])

    def test_cited_missing_source_still_requires_durable_evidence(self):
        entry = self.preserve()
        self.source.unlink()
        (self.record / entry["path"]).unlink()
        self.run_cli("prune-build", success=False)
        self.assertTrue(self.lane.exists())

    def test_corrupt_durable_copy_refuses(self):
        entry = self.preserve()
        (self.record / entry["path"]).write_bytes(b"corrupt")
        self.run_cli("prune-build", success=False)
        self.assertTrue(self.source.exists())

    def test_changed_source_refuses(self):
        self.preserve()
        self.source.write_bytes(self.data + b" ")
        self.run_cli("prune-build", success=False)

    def test_preservation_idempotent(self):
        self.preserve()
        self.preserve()
        self.assertEqual(len(json.loads(self.index.read_text())["receipts"]), 1)

    def test_gate_runs_never_deleted_even_after_preservation(self):
        runs = self.lane / ".cache/emax-gate/runs/example"
        runs.mkdir(parents=True)
        receipt = runs / "receipt.json"
        receipt.write_bytes(self.data)
        self.preserve()
        self.run_cli("check", success=False)
        self.run_cli("prune-build", success=False)
        self.run_cli("prune-build", candidate=runs, success=False)
        self.assertEqual(receipt.read_bytes(), self.data)
        self.assertEqual(len(json.loads(self.index.read_text())["receipts"]), 2)

    def test_symlink_directory_refuses(self):
        (self.lane / "alias").symlink_to(self.source.parent, target_is_directory=True)
        self.run_cli("preserve", success=False)
        self.run_cli("prune-build", success=False)

    def test_symlink_receipt_refuses(self):
        self.source.unlink()
        target = self.base / "receipt.json"
        target.write_bytes(self.data)
        self.source.symlink_to(target)
        self.run_cli("preserve", success=False)
        self.assertTrue(target.exists())

    def test_missing_registration_refuses(self):
        self.preserve()
        self.index.write_text('{"receipts":[]}')
        self.run_cli("prune-build", success=False)

    def test_schema_extensions_do_not_invalidate_existing_receipt(self):
        self.preserve()
        index = json.loads(self.index.read_text())
        index["receipts"][0]["provenance"] = "fixture"
        self.index.write_text(json.dumps(index))
        self.run_cli("check")
        self.preserve()
        self.assertEqual(len(json.loads(self.index.read_text())["receipts"]), 1)

    def test_check_leaves_versioned_record_unchanged(self):
        self.preserve()
        before = {str(p): p.read_bytes() for p in self.record.rglob("*") if p.is_file()}
        self.run_cli("check")
        after = {str(p): p.read_bytes() for p in self.record.rglob("*") if p.is_file()}
        self.assertEqual(before, after)

    def test_preserve_retains_index_mode(self):
        self.index.chmod(0o640)
        self.preserve()
        self.assertEqual(self.index.stat().st_mode & 0o777, 0o640)

    def test_nonobject_record_refuses_cleanly(self):
        self.index.write_text("[]")
        run = self.run_cli("prune-build", success=False)
        self.assertIn("REFUSED", run.stderr)
        self.assertNotIn("Traceback", run.stderr)
        self.assertTrue(self.source.exists())

    def test_bad_record_refuses_without_deletion(self):
        self.index.write_text("not json")
        self.run_cli("prune-build", success=False)
        self.assertTrue(self.source.exists())

    def test_arbitrary_directory_not_a_build_target(self):
        self.preserve()
        self.run_cli("prune-build", candidate=self.build, success=False)
        self.assertTrue(self.lane.exists())

    def test_record_nested_in_candidate_refuses(self):
        nested = self.lane / "record"
        (nested / "reference").mkdir(parents=True)
        (nested / "reference/references.json").write_text('{"receipts":[]}')
        self.record = nested
        self.run_cli("preserve", success=False)


if __name__ == "__main__":
    unittest.main(verbosity=2)
