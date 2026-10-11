"""Keep the process boundary under the complete shared lint policy."""
from pathlib import Path
import tomllib
import unittest

ROOT = Path(__file__).resolve().parent.parent

class LintPolicyTests(unittest.TestCase):
    def test_only_the_documented_unsafe_boundary_differs_from_workspace_lints(self):
        shared = tomllib.loads((ROOT / "Cargo.toml").read_text())["workspace"]["lints"]
        local = tomllib.loads((ROOT / "crates/tinyruntime-process/Cargo.toml").read_text())["lints"]
        self.assertEqual(local["rust"]["unsafe_code"], "deny")
        shared["rust"]["unsafe_code"] = "deny"
        self.assertEqual(local, shared)

if __name__ == "__main__":
    unittest.main()
