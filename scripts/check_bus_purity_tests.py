"""Dependency identity regressions for the contract purity gate."""
import ast
import pathlib
import unittest

SCRIPT = pathlib.Path(__file__).with_name('check-bus-purity.py')

class DependencyIdentityTests(unittest.TestCase):
    def test_allowed_names_require_the_approved_source(self):
        tree = ast.parse(SCRIPT.read_text())
        functions = [node for node in tree.body if isinstance(node, ast.FunctionDef) and node.name == 'approved_identity']
        self.assertEqual(len(functions), 1, 'the purity gate must validate source-qualified identities')
        namespace = {'ROOT': SCRIPT.parent.parent, 'pathlib': pathlib}
        exec(compile(ast.Module(body=functions, type_ignores=[]), str(SCRIPT), 'exec'), namespace)
        approved = namespace['approved_identity']
        crates_io = 'registry+https://github.com/rust-lang/crates.io-index'
        self.assertTrue(approved({'name': 'serde', 'source': crates_io}))
        for source in [None, 'git+https://example.invalid/serde', 'registry+https://example.invalid/index']:
            self.assertFalse(approved({'name': 'serde', 'source': source}))
        self.assertTrue(approved({'name': 'tinyruntime-bus', 'source': None, 'manifest_path': str(SCRIPT.parent.parent / 'crates/tinyruntime-bus/Cargo.toml')}))
        self.assertFalse(approved({'name': 'tinyruntime-bus', 'source': None, 'manifest_path': '/other/Cargo.toml'}))

if __name__ == '__main__':
    unittest.main()
