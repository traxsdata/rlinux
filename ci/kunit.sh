#!/usr/bin/env bash
# Run KUnit (including the Rust doctests and #[kunit_tests]) under QEMU.
. "$(dirname "$0")/lib.sh"
step "kunit (x86_64 QEMU, Rust doctests)"
./tools/testing/kunit/kunit.py run --arch x86_64 --build_dir "$CI_OUT/kunit" \
  --make_options LLVM="$LLVM" --jobs "$JOBS" --timeout 900 \
  --kconfig_add CONFIG_RUST=y --kconfig_add CONFIG_RUST_KERNEL_DOCTESTS=y
echo "ci: kunit OK"
