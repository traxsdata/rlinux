#!/usr/bin/env bash
# Cross-configure and build a non-host $ARCH (arm64, riscv): vmlinux and
# modules including Rust and the Rust samples, with clippy and warnings as
# errors. No boot test here (needs a different QEMU; see ci/README.md).
#
# Usage: ci/cross-build.sh <arch>
# (arch values are the kernel ARCH names: arm64, riscv)
ARCH=${1:-}
case $ARCH in
  arm64|riscv) ;;
  *) printf 'usage: ci/cross-build.sh <arch>  (arm64 | riscv)\n' >&2; exit 2 ;;
esac
# Per-arch out-of-tree dir (lib.sh then sets KOUT=$CI_OUT/kernel).
CI_OUT=${CI_OUT:-.ci-out/$ARCH}
. "$(dirname "$0")/lib.sh"
export ARCH

step "toolchain"
"${MAKE[@]}" ARCH=$ARCH rustavailable

step "configure: ${ARCH} defconfig + ci/rlinux-ci.config + ci/config.d/*.config"
frags=(ci/rlinux-ci.config ci/config.d/*.config)
"${MAKE[@]}" ARCH=$ARCH defconfig
scripts/kconfig/merge_config.sh -m -O "$KOUT" "$KOUT/.config" "${frags[@]}" >/dev/null
"${MAKE[@]}" ARCH=$ARCH olddefconfig
bad=0
while IFS= read -r line; do
  case $line in CONFIG_*=*) ;; *) continue ;; esac
  grep -qxF "$line" "$KOUT/.config" || { echo "missing from final .config: $line"; bad=1; }
done < <(cat "${frags[@]}")
[ "$bad" = 0 ] || die "config fragment symbols dropped by Kconfig"

step "build (CLIPPY=1, warnings are errors)"
"${MAKE[@]}" ARCH=$ARCH CLIPPY=1

step "artifacts"
ls -l "$KOUT/vmlinux" \
	"$KOUT/samples/rust/rust_minimal.ko" \
	"$KOUT/samples/rust/rust_print.ko" \
	"$KOUT/samples/rust/rust_hwrng.ko"
echo "ci: cross-build $ARCH OK"
