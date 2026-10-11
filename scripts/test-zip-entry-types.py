"""The Windows verifier rejects special Unix ZIP entries before extraction."""
from pathlib import Path
import subprocess
import tempfile
import unittest
import zipfile

ROOT = Path(__file__).resolve().parent.parent

class ZipEntryTests(unittest.TestCase):
    def test_special_entries_are_rejected_before_extraction(self):
        target = ROOT / "target"
        target.mkdir(exist_ok=True)
        with tempfile.TemporaryDirectory(dir=target) as directory:
            archive = Path(directory) / "special.zip"
            for mode in [0x1000, 0x2000, 0x6000, 0xA000, 0xC000]:
                with self.subTest(mode=hex(mode)):
                    entry = zipfile.ZipInfo("special")
                    entry.create_system = 3
                    entry.external_attr = (mode | 0o600) << 16
                    with zipfile.ZipFile(archive, "w") as package:
                        package.writestr(entry, b"fixture")
                    result = subprocess.run([
                        "pwsh", "-NoProfile", "-File",
                        str(ROOT / "scripts/verify-module.ps1"), "-Archive", str(archive)
                    ], text=True, capture_output=True)
                    self.assertNotEqual(result.returncode, 0)
                    self.assertIn("Archive member is unsafe: special", result.stderr)

if __name__ == "__main__":
    unittest.main()
