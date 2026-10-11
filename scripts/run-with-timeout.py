"""Run a command with a deadline on every supported CI platform."""
import subprocess
import sys

try:
    result = subprocess.run(sys.argv[2:], timeout=float(sys.argv[1]), check=False)
except subprocess.TimeoutExpired:
    print("command timed out", file=sys.stderr)
    raise SystemExit(124)
raise SystemExit(result.returncode)
