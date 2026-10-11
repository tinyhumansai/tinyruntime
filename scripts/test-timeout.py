"""Portable command deadlines preserve failures without requiring GNU timeout."""
from pathlib import Path
import subprocess
import sys
import unittest

RUNNER = Path(__file__).with_name("run-with-timeout.py")

class TimeoutTests(unittest.TestCase):
    def run_command(self, seconds, code):
        return subprocess.run(
            [sys.executable, str(RUNNER), str(seconds),
             sys.executable, "-c", code],
            capture_output=True, text=True, timeout=10)

    def test_success_preserves_output(self):
        result = self.run_command(5, "print('completed')")
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(result.stdout.strip(), "completed")

    def test_child_failure_preserves_exit_code(self):
        result = self.run_command(5, "raise SystemExit(7)")
        self.assertEqual(result.returncode, 7, result.stderr)

    def test_stalled_child_fails_at_deadline(self):
        result = self.run_command(0.1, "import time; time.sleep(30)")
        self.assertEqual(result.returncode, 124, result.stderr)
        self.assertIn("command timed out", result.stderr)

if __name__ == "__main__":
    unittest.main()
