#!/usr/bin/env python3
"""Distributed benchmark policy never implicitly authorizes a machine."""
import unittest
from unittest.mock import patch

import bench_runner_policy as policy


class RunnerPolicyTests(unittest.TestCase):
    def test_distributed_policy_has_no_machine_defaults(self):
        self.assertEqual(policy.RUNNER_ELIGIBILITY, {})
        with self.assertRaisesRegex(ValueError, 'unknown benchmark runner'):
            policy.check_runner('benchmark.example.org', [])

    def test_explicit_disallow_is_not_overridden_by_unknown_opt_in(self):
        local_policy = {'benchmark.example.org': {
            'eligible_rows': (), 'reason': 'not approved for measurement',
        }}
        with patch.object(policy, 'RUNNER_ELIGIBILITY', local_policy):
            with self.assertRaisesRegex(ValueError, 'no eligible rows'):
                policy.check_runner('benchmark.example.org', [], allow_unknown=True)

    def test_unknown_opt_in_does_not_change_distributed_policy(self):
        policy.check_runner('benchmark.example.org', [], allow_unknown=True)
        self.assertEqual(policy.RUNNER_ELIGIBILITY, {})


if __name__ == '__main__':
    unittest.main()
