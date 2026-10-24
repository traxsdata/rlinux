# rlinux CI

Checks run on every pull request and push to `main` (`.github/workflows/ci.yml`),
and locally by the same scripts. All of them build out of tree, in `$CI_OUT`
(default `.ci-out/`, gitignored), so the source tree stays clean.

| Script | What it proves |
|---|---|
| `ci/build.sh` | `rustavailable`; x86_64 defconfig + `ci/rlinux-ci.config` (Rust, the Rust samples, the QEMU boot devices) builds with clippy and warnings as errors; every fragment symbol survived Kconfig |
| `ci/rustfmt.sh` | `make rustfmtcheck`: all Rust code is formatted |
| `ci/cross-build.sh <arch>` | cross-builds a non-host arch (`arm64`, `riscv`) from `$ARCH` defconfig + a config fragment (`ci/rlinux-ci-<arch>.config` if present, else `ci/rlinux-ci.config`), clippy on and warnings as errors; verifies `vmlinux` and the Rust sample `.ko`s. No boot test (would need a non-x86 QEMU) |
| `ci/kunit.sh` | KUnit in QEMU, including the Rust doctests and `#[kunit_tests]` (`CONFIG_RUST_KERNEL_DOCTESTS`) |
| `ci/boot.sh` | the kernel from `ci/build.sh` boots in QEMU with a busybox initramfs whose `/init` checks the virtio disk, loads/unloads the Rust sample modules, and reads from the `rust_hwrng` sample through `/dev/hwrng` (no panic, oops or warning allowed) |
| `ci/all.sh` | all of the above in order |

Requirements: the rustc and bindgen versions from `scripts/min-tool-version.sh`
(with the `rust-src`, `rustfmt` and `clippy` components), clang/lld/llvm >= the
`llvm` minimum, `qemu-system-x86_64`, and a statically linked `busybox`.

Environment: `LLVM` (default `1`; e.g. `LLVM=-21` for versioned binaries),
`CI_OUT`, `JOBS` (default `nproc`), `BUSYBOX` (default `busybox` in `PATH`),
`QEMU_ACCEL` (default `kvm` when `/dev/kvm` is usable, else `tcg`).

Locally: `ci/all.sh` (about 10 minutes with 16 cores).
