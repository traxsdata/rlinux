#!/usr/bin/env bash
# Every CI check, in order. Stops at the first failure.
set -euo pipefail
d=$(dirname "$0")
"$d/build.sh"
"$d/rustfmt.sh"
"$d/boot.sh"
"$d/kunit.sh"
echo; echo "ci: ALL OK"
