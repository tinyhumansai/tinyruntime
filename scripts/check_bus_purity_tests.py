"""Dependency identity regressions for the contract purity gate."""
import importlib.util
import pathlib
import unittest

SCRIPT = pathlib.Path(__file__).with_name('check-bus-purity.py')

class DependencyIdentityTests(unittest.TestCase):
    def test_allowed_names_require_the_approved_source(self):
        spec = importlib.util.spec_from_file_location('bus_purity', SCRIPT)
        module = importlib.util.module_from_spec(spec)
        spec.loader.exec_module(module)
        approved = module.approved_identity
        crates_io = 'registry+https://github.com/rust-lang/crates.io-index'
        self.assertTrue(approved({'name': 'serde', 'source': crates_io}))
        for source in [None, 'git+https://example.invalid/serde', 'registry+https://example.invalid/index']:
            self.assertFalse(approved({'name': 'serde', 'source': source}))
        self.assertTrue(approved({'name': 'tinyruntime-bus', 'source': None, 'manifest_path': str(SCRIPT.parent.parent / 'crates/tinyruntime-bus/Cargo.toml')}))
        self.assertFalse(approved({'name': 'tinyruntime-bus', 'source': None, 'manifest_path': '/other/Cargo.toml'}))

if __name__ == '__main__':
    unittest.main()
