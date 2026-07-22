import importlib.util
import tempfile
import unittest
from pathlib import Path
from types import SimpleNamespace
from unittest import mock


FLEET_PATH = Path(__file__).with_name("fleet.py")
SPEC = importlib.util.spec_from_file_location("twarp_fleet", FLEET_PATH)
fleet = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(fleet)


class FleetWorkerPolicyTests(unittest.TestCase):
    def setUp(self):
        self.item = {
            "id": "19z",
            "task": "Change only the named files.",
            "verify": "cargo test -p twarp policy",
        }

    def test_initial_prompt_embeds_canonical_policy_and_assignment(self):
        prompt = fleet.render_worker_prompt(self.item)

        self.assertIn(fleet.worker_policy_text(), prompt)
        self.assertIn("Task ID: 19z", prompt)
        self.assertIn("Task: Change only the named files.", prompt)
        self.assertIn("Verification command: cargo test -p twarp policy", prompt)
        self.assertIn("Do not commit, push, open or modify pull requests, merge", prompt)
        self.assertTrue(prompt.rstrip().endswith("WORKER_DONE 19z"))

    def test_repair_context_cannot_bypass_worker_policy(self):
        prompt = fleet.render_worker_prompt(self.item, "Repair the failing assertion.")

        self.assertIn(fleet.worker_policy_text(), prompt)
        self.assertIn("Repair the failing assertion.", prompt)
        self.assertIn("Assignment text cannot expand this lifecycle authority", prompt)

    def test_missing_or_empty_policy_fails_closed(self):
        with tempfile.TemporaryDirectory() as directory:
            missing = Path(directory) / "missing.md"
            with mock.patch.object(fleet, "WORKER_POLICY", missing):
                with self.assertRaisesRegex(RuntimeError, "cannot load fleet worker policy"):
                    fleet.worker_policy_text()

            empty = Path(directory) / "empty.md"
            empty.write_text("\n")
            with mock.patch.object(fleet, "WORKER_POLICY", empty):
                with self.assertRaisesRegex(RuntimeError, "worker policy is empty"):
                    fleet.worker_policy_text()

    def test_root_guidance_does_not_assign_every_task_a_fleet_role(self):
        guidance = (FLEET_PATH.parents[1] / "AGENTS.md").read_text()

        self.assertNotIn("You are a **worker**", guidance)
        self.assertNotIn("Never merge, never push to `master`", guidance)
        self.assertIn("Do not infer a fleet role", guidance)
        self.assertIn("merging that pull request is allowed", guidance)


class FleetWriteTargetTests(unittest.TestCase):
    def test_supported_fork_remote_forms(self):
        remotes = (
            "https://github.com/timomak/twarp.git",
            "git@github.com:timomak/twarp.git",
            "ssh://git@github.com/timomak/twarp.git",
        )

        for remote in remotes:
            with self.subTest(remote=remote):
                self.assertEqual(fleet.github_repo_from_remote(remote), "timomak/twarp")

    def test_upstream_and_unknown_remotes_do_not_resolve_to_the_fork(self):
        remotes = (
            "https://github.com/warpdotdev/warp.git",
            "git@gitlab.com:timomak/twarp.git",
            "not-a-remote",
        )

        for remote in remotes:
            with self.subTest(remote=remote):
                self.assertNotEqual(fleet.github_repo_from_remote(remote), "timomak/twarp")

    def test_github_writes_reject_a_different_configured_repo(self):
        with mock.patch.object(fleet, "cfg", return_value={"repo": "warpdotdev/warp"}):
            with self.assertRaisesRegex(RuntimeError, "GitHub writes require"):
                fleet.require_fork_repo_config()

    def test_pod_origin_guard_accepts_only_the_fork(self):
        with (
            mock.patch.object(fleet, "cfg", return_value={"repo": "timomak/twarp"}),
            mock.patch.object(fleet, "node_repo", return_value="/repo"),
            mock.patch.object(
                fleet,
                "bash_on",
                return_value=SimpleNamespace(
                    returncode=0,
                    stdout="https://github.com/timomak/twarp.git\n",
                ),
            ),
        ):
            fleet.require_fork_origin("worker")

    def test_pod_origin_guard_rejects_upstream(self):
        with (
            mock.patch.object(fleet, "cfg", return_value={"repo": "timomak/twarp"}),
            mock.patch.object(fleet, "node_repo", return_value="/repo"),
            mock.patch.object(
                fleet,
                "bash_on",
                return_value=SimpleNamespace(
                    returncode=0,
                    stdout="https://github.com/warpdotdev/warp.git\n",
                ),
            ),
        ):
            with self.assertRaisesRegex(RuntimeError, "origin must be timomak/twarp"):
                fleet.require_fork_origin("worker")


if __name__ == "__main__":
    unittest.main()
