# Shared settings for the ci/*.sh scripts (sourced, not run).
set -euo pipefail
cd "$(dirname "${BASH_SOURCE[0]}")/.."
SRC=$PWD
LLVM=${LLVM:-1}
CI_OUT=$(realpath -m "${CI_OUT:-.ci-out}")
JOBS=${JOBS:-$(nproc)}
KOUT=$CI_OUT/kernel
MAKE=(make -s LLVM="$LLVM" O="$KOUT" -j"$JOBS")
step() { printf '\n=== %s\n' "$*"; }
die() { printf 'ci: FAIL: %s\n' "$*" >&2; exit 1; }
mkdir -p "$CI_OUT"
