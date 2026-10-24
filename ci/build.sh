#!/usr/bin/env bash
# Configure and build the CI kernel (bzImage + modules) in $CI_OUT/kernel.
. "$(dirname "$0")/lib.sh"

step "toolchain"
"${MAKE[@]}" rustavailable

step "configure: x86_64_defconfig + ci/rlinux-ci.config + ci/config.d/*.config"
"${MAKE[@]}" x86_64_defconfig
frags=(ci/rlinux-ci.config ci/config.d/*.config)
scripts/kconfig/merge_config.sh -m -O "$KOUT" "$KOUT/.config" "${frags[@]}" >/dev/null
"${MAKE[@]}" olddefconfig
bad=0
while IFS= read -r line; do
  case $line in CONFIG_*=*) ;; *) continue ;; esac
  grep -qxF "$line" "$KOUT/.config" || { echo "missing from final .config: $line"; bad=1; }
done < <(cat "${frags[@]}")
[ "$bad" = 0 ] || die "config fragment symbols dropped by Kconfig"

step "build (CLIPPY=1, warnings are errors)"
# Default target: vmlinux, bzImage and modules. (Naming "bzImage modules"
# together races on vmlinux.lds with -j.)
"${MAKE[@]}" CLIPPY=1
ls -l "$KOUT/arch/x86/boot/bzImage"
echo "ci: build OK"
