# Shared settings for the ci/*.sh scripts (sourced, not run).
set -euo pipefail
shopt -s nullglob
cd "$(dirname "${BASH_SOURCE[0]}")/.."
SRC=$PWD
LLVM=${LLVM:-1}
CI_OUT=$(realpath -m "${CI_OUT:-.ci-out}")
JOBS=${JOBS:-$(nproc)}
KOUT=$CI_OUT/kernel
MAKE=(make -s LLVM="$LLVM" O="$KOUT" -j"$JOBS")
# ccache: cache C compilation across runs so warm builds (especially the slow
# cross builds) recompile far less. The kernel Makefile sets CC to clang
# (clang-<N> for a versioned LLVM), so override it on the make command line
# -- an env var or export would be clobbered. Only enabled when a `ccache`
# binary is in PATH, so local runs without ccache are byte-for-byte unchanged
# (ci/*.sh still work with it absent). ccache is applied to the kernel C
# compiler only, never to HOSTCC (host tools), the linker or the archiver;
# Rust objects are not cached (the C part dominates, per issue #30).
clang_from_llvm() {
	case $1 in
		-*) printf 'clang%s' "$1" ;;   # -18 -> clang-18
		*/) printf '%sclang' "$1" ;;   # /prefix/ -> /prefix/clang
		*)  printf 'clang' ;;          # 1 -> bare clang
	esac
}
if command -v ccache >/dev/null 2>&1; then
	export CCACHE_DIR=${CCACHE_DIR:-"$CI_OUT/ccache"}
	# BASEDIR/NOHASHDIR/SLOPPINESS make cache hits survive different checkout
	# paths and timestamps (fresh runner checkout on every run).
	export CCACHE_BASEDIR=${CCACHE_BASEDIR:-"$SRC"}
	export CCACHE_NOHASHDIR=${CCACHE_NOHASHDIR:-1}
	export CCACHE_SLOPPINESS=${CCACHE_SLOPPINESS:-"file_stat_matches,sysroot,include_file_ctime,system_headers"}
	export CCACHE_MAXSIZE=${CCACHE_MAXSIZE:-"2G"}   # 3 jobs share the 10 GB repo cache limit
	mkdir -p "$CCACHE_DIR"
	MAKE+=("CC=ccache $(clang_from_llvm "$LLVM")")
fi
step() { printf '\n=== %s\n' "$*"; }
die() { printf 'ci: FAIL: %s\n' "$*" >&2; exit 1; }
mkdir -p "$CI_OUT"
