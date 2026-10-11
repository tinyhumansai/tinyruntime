#!/usr/bin/env bash
set -euo pipefail
cd "$(dirname "${BASH_SOURCE[0]}")/.."
python3 scripts/test-e2e-version.py
python3 scripts/test-process-lints.py
python3 scripts/check-bus-purity.py
# Keep panic diagnostics visible even if another test stalls; a stalled suite fails CI.
timeout 600 cargo test --locked --workspace -- --nocapture
timeout 600 cargo test --locked --workspace --all-features -- --nocapture

cargo run -p tinyruntime --example basic

set -euo pipefail
forbidden="$(cargo tree -p tinyruntime-bus -e normal,build --prefix none \
  | grep -Ei 'tinybus|tokio|reqwest|ureq|hyper|rusqlite|git2' || true)"
if [ -n "$forbidden" ]; then
  echo "tinyruntime-bus pulled in a dependency its manifest forbids:" >&2
  echo "$forbidden" >&2
  echo >&2
  echo "The contract is what a host compiles against. It must stay free" >&2
  echo "of transports, async runtimes, HTTP clients and native libraries." >&2
  exit 1
fi
case "${RUNNER_OS:-$(uname -s)}" in
  Windows|MINGW*|MSYS*|CYGWIN*) python3 scripts/test-zip-entry-types.py ;;
  *) python3 scripts/test-package-layout.py ;;
esac
