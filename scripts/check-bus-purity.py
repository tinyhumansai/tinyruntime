#!/usr/bin/env python3
"""Audit independent default/all contract dependency closures, excluding dev edges."""
import argparse
import json
import pathlib
import subprocess
import tomllib

ROOT = pathlib.Path(__file__).resolve().parents[1]
ALLOWED = {
    'tinyruntime-bus', 'serde', 'serde_core', 'serde_derive', 'serde_json',
    'proc-macro2', 'quote', 'syn', 'unicode-ident', 'itoa', 'memchr', 'zmij',
}
FORBIDDEN = ('async fn ', 'async_trait', 'tokio::', 'std::sync', 'std::thread',
             'std::fs', 'std::env', 'std::time', 'std::process', 'parking_lot::',
             'sha2::', 'hmac::', 'rand::', 'uuid::')
for source in (ROOT / 'crates/tinyruntime-bus/src').rglob('*.rs'):
    if source.name.endswith('_tests.rs'):
        continue
    content = source.read_text()
    for forbidden in FORBIDDEN:
        if forbidden in content:
            raise SystemExit(f'{source.relative_to(ROOT)}: implementation token {forbidden!r}')
parser = argparse.ArgumentParser(description=__doc__)
parser.add_argument('--offline', action='store_true', help='Use only cached Cargo registry data')
args = parser.parse_args()
features = list(tomllib.loads((ROOT / 'crates/tinyruntime-bus/Cargo.toml').read_text()).get('features', {}))
for mode in ('default', 'all'):
    probe = ROOT / 'target/bus-purity' / mode
    probe.mkdir(parents=True, exist_ok=True)
    (probe / 'src').mkdir(exist_ok=True)
    (probe / 'src/lib.rs').write_text('pub use tinyruntime_bus::*;\n')
    (probe / 'Cargo.toml').write_text(
        '[package]\nname="contract-purity-probe"\nversion="0.0.0"\nedition="2024"\n'
        '[workspace]\n[dependencies]\ntinyruntime-bus = { path = ' + json.dumps(str(ROOT / 'crates/tinyruntime-bus'))
        + (', features = ' + json.dumps(features) if mode == 'all' else '') + ' }\n')
    graph = json.loads(subprocess.check_output([
        'cargo', 'metadata', *(['--offline'] if args.offline else []), '--format-version=1', '--manifest-path', str(probe / 'Cargo.toml')
    ], cwd=ROOT))
    packages = {p['id']: p['name'] for p in graph['packages']}
    nodes = {n['id']: n for n in graph['resolve']['nodes']}
    pending = [next(p['id'] for p in graph['packages'] if p['name'] == 'tinyruntime-bus')]
    seen = set()
    while pending:
        package = pending.pop()
        if package in seen:
            continue
        seen.add(package)
        for edge in nodes[package]['deps']:
            if any(kind['kind'] != 'dev' for kind in edge['dep_kinds']):
                pending.append(edge['pkg'])
    unsafe = sorted({packages[p] for p in seen} - ALLOWED)
    if unsafe:
        raise SystemExit(f'{mode}: unsafe contract dependencies: {unsafe}')
    closure = sorted(({'name': packages[p], 'id': p} for p in seen), key=lambda item: item['name'])
    (probe / 'closure.json').write_text(json.dumps(closure, indent=2) + '\n')
    print(f'{mode}: pure contract closure ({len(seen)} packages)')
