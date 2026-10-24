#!/usr/bin/env python3
"""Disposable Git integration tests; optional copied public-lander exercise.

MASTER_GUARD_LANDER_DIR and MASTER_GUARD_SIGNING_KEY enable that exercise.
Its attestation verifier is explicitly a fixture stub: this tests the real
lander's signature/ancestry/lock/CAS/sync path, NOT gate/attestation validity.
"""
import os
from pathlib import Path
import shlex
import shutil
import subprocess
import sys
import tempfile
import unittest

HERE = Path(__file__).resolve().parent
GUARD = Path(os.environ.get("MASTER_GUARD_SOURCE", HERE / "master-reference-guard.py"))
DENIED = "MASTER GUARD REFUSED: refs/heads/master may move only through romacs-land.sh"


class GuardTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory(prefix="master-guard-", dir=os.environ.get("TMPDIR"))
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name)
        self.env = {k: v for k, v in os.environ.items() if not k.startswith("GIT_")}
        self.env.update(GIT_CONFIG_GLOBAL=os.devnull, GIT_CONFIG_NOSYSTEM="1")
        self.repo = self.root / "repo"
        self.init(self.repo)
        self.old = self.git("rev-parse", "HEAD").stdout.strip()
        self.git("checkout", "-b", "lane")
        self.git("commit", "--allow-empty", "-m", "Lane candidate")
        self.new = self.git("rev-parse", "HEAD").stdout.strip()
        self.git("checkout", "master")
        self.common = self.repo / ".git"
        self.install(self.repo, self.common)

    def run_cmd(self, argv, *, check=True, **kwargs):
        result = subprocess.run([str(a) for a in argv], env=self.env,
                                capture_output=True, text=True, **kwargs)
        if check:
            self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
        return result

    def git(self, *args, repo=None, **kwargs):
        return self.run_cmd(["git", "-C", repo or self.repo, *args], **kwargs)

    def init(self, repo):
        self.run_cmd(["git", "init", "--initial-branch=master", repo])
        for key, value in (("user.name", "Guard Fixture"), ("user.email", "fixture@example.org"),
                           ("commit.gpgsign", "false")):
            self.git("config", key, value, repo=repo)
        self.git("commit", "--allow-empty", "-m", "Initial fixture", repo=repo)

    def install(self, repo, pinned):
        command = shlex.join([sys.executable, str(GUARD), "hook", "--common-dir", str(pinned)])
        self.git("config", "hook.master-guard.event", "reference-transaction", repo=repo)
        self.git("config", "hook.master-guard.command", command, repo=repo)

    def refused(self, result):
        self.assertNotEqual(result.returncode, 0, "master move unexpectedly succeeded")
        self.assertIn(DENIED, result.stderr)
        self.assertEqual(self.git("rev-parse", "master").stdout.strip(), self.old)
        print(result.stderr.strip(), flush=True)

    def test_direct_commit_refused(self):
        self.refused(self.git("commit", "--allow-empty", "-m", "Direct master", check=False))

    def test_direct_merge_refused(self):
        self.refused(self.git("merge", "--ff-only", "lane", check=False))

    def test_direct_merge_commit_refused(self):
        self.refused(self.git("merge", "--no-ff", "lane", "-m", "Direct merge", check=False))

    def test_linked_worktree_master_move_refused(self):
        lane = self.root / "lane"
        self.git("worktree", "add", lane, "lane")
        self.refused(self.git("update-ref", "refs/heads/master", self.new, self.old,
                             repo=lane, check=False))

    def test_symbolic_master_refused(self):
        self.refused(self.git("symbolic-ref", "refs/heads/master", "refs/heads/lane", check=False))

    def test_regular_file_is_not_capability(self):
        path = self.root / "forged-token"
        path.write_bytes(self.token())
        with path.open("rb") as token:
            self.env["ROMACS_MASTER_AUTH_FD"] = str(token.fileno())
            self.refused(self.git("update-ref", "refs/heads/master", self.new, self.old,
                                 check=False, pass_fds=(token.fileno(),)))

    def test_maintenance_policy(self):
        # Git's files backend reports physical packing as master mutations.
        # Fail closed on direct packing; supported deployment disables only
        # ref packing in gc/automatic maintenance, not object maintenance.
        self.refused(self.git("pack-refs", "--all", check=False))
        self.git("config", "gc.packRefs", "false")
        self.git("config", "maintenance.pack-refs.enabled", "false")
        result = self.git("gc")
        self.assertNotIn(DENIED, result.stderr)
        result = self.git("maintenance", "run")
        self.assertNotIn(DENIED, result.stderr)
        self.assertEqual(self.git("rev-parse", "master").stdout.strip(), self.old)
        self.refused(self.git("update-ref", "-d", "refs/heads/master", self.old, check=False))

    def test_plumbing_and_delete_refused(self):
        self.refused(self.git("update-ref", "refs/heads/master", self.new, self.old, check=False))
        self.refused(self.git("update-ref", "-d", "refs/heads/master", self.old, check=False))

    def test_environment_flag_does_not_authorize(self):
        self.env["ROMACS_MASTER_AUTH_FD"] = "1"
        self.refused(self.git("update-ref", "refs/heads/master", self.new, self.old, check=False))

    def test_lane_commit_succeeds(self):
        lane = self.root / "lane"
        self.git("worktree", "add", lane, "lane")
        result = self.git("commit", "--allow-empty", "-m", "Lane remains writable", repo=lane)
        print(result.stdout.strip(), flush=True)
        self.assertNotEqual(self.git("rev-parse", "HEAD", repo=lane).stdout.strip(), self.new)
        self.assertEqual(self.git("rev-parse", "master").stdout.strip(), self.old)

    def test_other_repo_unaffected_even_with_shared_hook(self):
        other = self.root / "other"
        self.init(other)
        self.install(other, self.common)
        before = self.git("rev-parse", "HEAD", repo=other).stdout.strip()
        result = self.git("commit", "--allow-empty", "-m", "Other master", repo=other)
        print(result.stdout.strip(), flush=True)
        self.assertNotEqual(self.git("rev-parse", "HEAD", repo=other).stdout.strip(), before)

    def token_git(self, token, *args, **kwargs):
        reader, writer = os.pipe()
        try:
            os.write(writer, token)
            os.close(writer)
            writer = None
            self.env["ROMACS_MASTER_AUTH_FD"] = str(reader)
            return self.git(*args, check=False, pass_fds=(reader,), **kwargs)
        finally:
            os.close(reader)
            if writer is not None:
                os.close(writer)
            self.env.pop("ROMACS_MASTER_AUTH_FD", None)

    def token(self, old=None, new=None, common=None):
        return (str(common or self.common) + "\n" +
                (old or self.old) + " " + (new or self.new) + " refs/heads/master\n").encode()

    def test_wrong_old_new_repository_refused(self):
        for token in (self.token(old=self.new), self.token(new=self.old),
                      self.token(common=self.root / "elsewhere")):
            self.refused(self.token_git(token, "update-ref", "refs/heads/master", self.new, self.old))

    def test_capability_single_use_and_exact_transaction(self):
        reader, writer = os.pipe()
        try:
            os.write(writer, self.token())
            os.close(writer)
            self.env["ROMACS_MASTER_AUTH_FD"] = str(reader)
            self.git("update-ref", "refs/heads/master", self.new, self.old, pass_fds=(reader,))
            # Restore master through a distinct authorized operation, then replay
            # exactly the original transaction with the now-empty capability.
            self.run_cmd([sys.executable, GUARD, "publish", "--repo", self.repo,
                          "--old", self.new, "--new", self.old, "--message", "fixture reset"])
            result = self.git("update-ref", "refs/heads/master", self.new, self.old,
                              check=False, pass_fds=(reader,))
            self.assertNotEqual(result.returncode, 0)
            self.assertIn(DENIED, result.stderr)
            self.assertEqual(self.git("rev-parse", "master").stdout.strip(), self.old)
        finally:
            os.close(reader)
            self.env.pop("ROMACS_MASTER_AUTH_FD", None)

    def test_multiref_transaction_refused(self):
        self.refused(self.token_git(self.token(), "update-ref", "--stdin",
                                   input=f"update refs/heads/master {self.new} {self.old}\n"
                                         f"update refs/heads/lane {self.old} {self.new}\n"))
        self.assertEqual(self.git("rev-parse", "lane").stdout.strip(), self.new)

    def test_publisher_compare_and_swap(self):
        args = [sys.executable, GUARD, "publish", "--repo", self.repo,
                "--old", self.old, "--new", self.new, "--message", "romacs-land: fixture"]
        self.run_cmd(args)
        self.assertEqual(self.git("rev-parse", "master").stdout.strip(), self.new)
        stale = self.run_cmd(args, check=False)
        self.assertNotEqual(stale.returncode, 0)
        self.assertIn("cannot lock ref", stale.stderr)

    @unittest.skipUnless(os.environ.get("MASTER_GUARD_LANDER_DIR"), "copied public lander not requested")
    def test_public_lander_fast_forward(self):
        source = Path(os.environ["MASTER_GUARD_LANDER_DIR"])
        copied = self.root / "lander"
        copied.mkdir()
        for name in ("romacs-land.sh", "romacs-land-fast.sh"):
            shutil.copy2(source / name, copied / name)
        shutil.copy2(GUARD, copied / GUARD.name)
        self.run_cmd(["patch", "--batch", "--fuzz=0", "-p1", "-i",
                      HERE / "master-reference-guard-lander.patch"], cwd=copied)
        # Deliberately isolated seam; do not mistake this for attestation proof.
        (copied / "romacs-attestation.py").write_text(
            'print("FIXTURE ONLY: attestation verifier stub; no gate claim")\n')
        key = os.environ["MASTER_GUARD_SIGNING_KEY"]
        self.git("config", "user.signingkey", key)
        self.git("checkout", "lane")
        self.git("reset", "--hard", self.old)
        self.git("-c", "commit.gpgsign=true", "commit", "--allow-empty", "-m", "Signed lane fixture")
        tip = self.git("rev-parse", "HEAD").stdout.strip()
        self.git("checkout", "master")
        lane = self.root / "signed-lane"
        self.git("worktree", "add", lane, "lane")
        self.env.update(ROMACS_REPO=str(self.repo), ROMACS_LAND_TEST="1",
                        ROMACS_LAND_TEST_RUNS_ROOT=str(self.root / "runs"),
                        ROMACS_LAND_TEST_FREEZE=str(self.root / "freeze"))
        result = self.run_cmd(["bash", copied / "romacs-land.sh", "lane", lane, "test",
                               "--attestation", self.root / "fixture.json"])
        self.assertIn("LANDED: lane -> master " + tip, result.stdout)
        self.assertEqual(self.git("rev-parse", "master").stdout.strip(), tip)
        self.assertEqual(self.git("status", "--porcelain").stdout, "")
        print(result.stdout.strip(), flush=True)


if __name__ == "__main__":
    unittest.main(verbosity=2)
