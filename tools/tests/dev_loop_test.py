#!/usr/bin/env python3
"""Focused target selection, environment preservation and failure propagation."""
import contextlib
import importlib.machinery
import importlib.util
import io
import json
from pathlib import Path
import subprocess
import tempfile
import unittest
from unittest.mock import patch

loader = importlib.machinery.SourceFileLoader("dev_loop", str(Path(__file__).resolve().parents[1] / "dev-loop"))
spec = importlib.util.spec_from_loader(loader.name, loader)
loop = importlib.util.module_from_spec(spec)
loader.exec_module(loop)


def package(name="small", targets=None):
    return {"name": name, "id": name,
            "targets": targets if targets is not None else [
                {"name": name, "kind": ["lib"], "test": True},
                {"name": "regression", "kind": ["test"]},
                {"name": "other", "kind": ["test"]}]}


class SelectionTests(unittest.TestCase):
    def test_default_only_selects_the_library(self):
        self.assertEqual(loop.select(package()), ["--lib"])

    def test_explicit_targets_never_expand_to_other_targets(self):
        self.assertEqual(loop.select(package(), ["regression", "regression"]),
                         ["--test", "regression"])

    def test_explicit_library_and_integration_tests_can_be_combined(self):
        self.assertEqual(loop.select(package(), ["regression", "other"], True),
                         ["--lib", "--test", "regression", "--test", "other"])

    def test_expensive_packages_accept_a_focused_selection(self):
        self.assertEqual(loop.select(package("romacs-gnu-tests"), ["regression"]),
                         ["--test", "regression"])

    def test_unknown_target_and_unavailable_library_fail(self):
        with self.assertRaisesRegex(ValueError, "unknown test targets"):
            loop.select(package(), ["absent"])
        with self.assertRaisesRegex(ValueError, "select --test"):
            loop.select(package(targets=[{"name": "small", "kind": ["bin"]}]))


class RunnerTests(unittest.TestCase):
    def setUp(self):
        self.cache = tempfile.TemporaryDirectory(prefix="dev-loop-test-")
        self.addCleanup(self.cache.cleanup)

    def invoke(self, arguments=(), fail=False, extra_env=None):
        calls = []
        metadata = {"packages": [package()], "workspace_members": ["small"]}

        def run(command, **kwargs):
            tmpdir = Path(kwargs["env"]["TMPDIR"])
            self.assertTrue(tmpdir.is_dir() and not any(tmpdir.iterdir()), "a fresh private TMPDIR")
            calls.append((command, kwargs["env"]))
            if fail:
                raise subprocess.CalledProcessError(100, command)

        environment = {"PATH": "/bin", "HOME": "/owner", "CARGO_TARGET_DIR": "/chosen-target", "TMPDIR": "/shared-tmp",
                       "GNUPGHOME": "/signing", "XDG_CACHE_HOME": self.cache.name, **(extra_env or {})}
        environment = {key: value for key, value in environment.items() if value is not None}
        stream = io.StringIO()
        with patch.object(loop.sys, "argv", ["dev-loop", "small", *arguments]), \
             patch.dict(loop.os.environ, environment, clear=True), \
             patch.object(loop.subprocess, "check_output", return_value=json.dumps(metadata).encode()), \
             patch.object(loop.subprocess, "run", side_effect=run), \
             contextlib.redirect_stdout(stream), contextlib.redirect_stderr(stream):
            result = loop.main()
            self.assertEqual(dict(loop.os.environ), environment)
        return result, calls, stream.getvalue(), environment

    def test_one_nextest_build_preserves_the_callers_environment(self):
        result, calls, text, environment = self.invoke()
        self.assertEqual(result, 0)
        self.assertEqual(len(calls), 1)
        command, child_env = calls[0]
        self.assertEqual(command[:4], ["cargo", "nextest", "run", "--config"])
        # The caller's environment, but a TMPDIR of the run's own, gone after it.
        tmpdir = Path(child_env["TMPDIR"])
        self.assertEqual(tmpdir.parent, Path(self.cache.name))
        self.assertFalse(tmpdir.exists())
        self.assertEqual({**child_env, "TMPDIR": environment["TMPDIR"]}, environment)
        self.assertIn("--no-tests=fail", command)
        self.assertEqual(command[command.index("--retries") + 1], "0")
        self.assertIn("PASS (focused tests only)", text)

    def test_target_filter_and_resources_reach_nextest(self):
        result, calls, _, _ = self.invoke(["--test", "regression", "-E", "test(exact_case)",
                                          "--jobs", "2", "--threads", "3"])
        self.assertEqual(result, 0)
        command = calls[0][0]
        self.assertEqual(command[command.index("--test") + 1], "regression")
        self.assertNotIn("--lib", command)
        for option, value in (("-E", "test(exact_case)"), ("--build-jobs", "2"),
                              ("--test-threads", "3")):
            self.assertEqual(command[command.index(option) + 1], value)
        self.assertNotIn("--tests", command)
        self.assertNotIn("--workspace", command)

    def test_test_failure_is_not_success(self):
        result, calls, text, _ = self.invoke(fail=True)
        self.assertEqual(result, 1)
        self.assertEqual(len(calls), 1)
        self.assertIn("FAIL", text)
        self.assertNotIn("PASS", text)

    def test_gate_profile_reuses_optimized_builds_without_the_dev_linker_override(self):
        result, calls, _, _ = self.invoke(["--profile", "gate", "--test", "regression"])
        self.assertEqual(result, 0)
        command = calls[0][0]
        self.assertNotIn("--config", command)
        self.assertEqual(command[command.index("--cargo-profile") + 1], "gate")
        self.assertEqual(command[command.index("--test") + 1], "regression")

    def test_list_and_invalid_target_never_build(self):
        result, calls, text, _ = self.invoke(["--test", "regression", "--list"])
        self.assertEqual((result, calls), (0, []))
        self.assertIn("--test regression", text)
        self.assertIn("PASS (selection only)", text)
        result, calls, text, _ = self.invoke(["--test", "absent"])
        self.assertEqual((result, calls), (1, []))

    def test_rustflag_override_is_refused(self):
        result, calls, text, _ = self.invoke(extra_env={"RUSTFLAGS": "-C opt-level=3"})
        self.assertEqual((result, calls), (1, []))
        self.assertIn("unset rustflag overrides", text)

    def test_default_target_directory_is_left_to_cargo(self):
        result, calls, _, _ = self.invoke(extra_env={"CARGO_TARGET_DIR": None})
        self.assertEqual(result, 0)
        self.assertNotIn("CARGO_TARGET_DIR", calls[0][1])

    def test_invalid_worker_counts_are_rejected_before_subprocesses(self):
        for option in ("--threads", "--jobs"):
            with self.subTest(option=option), \
                 patch.object(loop.sys, "argv", ["dev-loop", "small", option, "0"]), \
                 patch.object(loop.subprocess, "check_output") as metadata, \
                 contextlib.redirect_stderr(io.StringIO()), self.assertRaises(SystemExit):
                loop.main()
            metadata.assert_not_called()


if __name__ == "__main__":
    unittest.main()
