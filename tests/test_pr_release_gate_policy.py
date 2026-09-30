from pathlib import Path
import unittest

ROOT = Path(__file__).resolve().parents[1]
WORKFLOW = ROOT / ".github" / "workflows" / "pr-release-gate.yml"


class PrReleaseGatePolicyTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.workflow = WORKFLOW.read_text(encoding="utf-8")

    def test_gate_is_report_only_and_read_only(self):
        self.assertIn('name: "PR Gate Bericht (report-only)"', self.workflow)
        self.assertIn("  contents: read", self.workflow)
        self.assertIn("  pull-requests: read", self.workflow)
        self.assertNotIn("  contents: write", self.workflow)
        self.assertNotIn("  pull-requests: write", self.workflow)

    def test_no_automatic_mutation_or_merge_path_remains(self):
        self.assertNotIn("github.rest.pulls.merge", self.workflow)
        self.assertNotIn("github.rest.pulls.updateBranch", self.workflow)
        self.assertIn("Merge remains manual", self.workflow)
        self.assertIn("Branch update and renewed gates are required manually", self.workflow)


if __name__ == "__main__":
    unittest.main()
