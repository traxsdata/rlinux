# rlinux: incremental Rust conversion of the Linux kernel

## What this is

A fork of mainline Linux (`torvalds/linux`) with an ongoing effort to convert
subsystems and drivers from C to Rust, one self-contained piece at a time.
This is not a rewrite-from-scratch project — a full kernel rewrite is not a
credible goal for any team of our size. Instead we follow the same model as
the upstream "Rust for Linux" effort, which mainline already merged: C and
Rust coexist indefinitely, Rust is adopted where it adds real safety value
and has working abstractions, and every change must build and boot.

## Why this is tractable

Mainline Linux (the base we forked) already ships:

- A working Rust toolchain integration (`rust/`, `Documentation/rust/`)
- Safe abstractions for several subsystems — see the verified
  "Available abstractions" list in the candidate-queue section below, which
  names the file in `rust/kernel/` for each one.
- A handful of real in-tree Rust drivers (`drivers/net/phy/ax88796b_rust.rs`,
  `drivers/android/binder/rust_binder_main.rs` and friends) and sample
  drivers under `samples/rust/` demonstrating each abstraction.

So "convert to Rust" mostly means: **replace one existing C driver/module
with a Rust implementation using an existing kernel Rust abstraction**, not
inventing new unsafe bindings from scratch. When no abstraction exists yet
for a subsystem, that is its own, separately-reviewed task (adding a safe
abstraction in `rust/kernel/`), never bundled with a driver port.

## Supported architectures

Supported: **x86, arm64, riscv, um** — the only architectures this
project builds.

The other upstream architectures have been pruned from the tree:

- alpha, arc, csky, hexagon, m68k, microblaze, mips, nios2, openrisc,
  parisc, sh, sparc, xtensa (issue #5, "Diet"): they cannot build Rust
  (they do not select `HAVE_RUST`), so any Rust port for them would be
  dead weight for this project.
- arm (32-bit), loongarch, powerpc, s390 (issue #37, "Diet 2"): they
  build Rust but are not targets. The only 32-bit ARM content kept is
  a small closure of device-tree files under `arch/arm/boot/dts/`
  that arm64's device trees `#include` through
  `scripts/dtc/include-prefixes`.

This is a deliberate, documented narrowing of scope that falls under the
exception of ground rule 3. The exact list of removed paths is recorded
in `scripts/rlinux/pruned-paths.txt`.

## Ground rules for every conversion PR

1. **One driver/module per PR.** No multi-subsystem sweeps.
2. **Must build.** `make LLVM=1 rustavailable` passes, and the kernel builds
   with `CONFIG_RUST=y` plus the relevant subsystem enabled
   (`allmodconfig` or a targeted `.config` for that driver).
3. **Must not remove hardware support.** The Rust version must support the
   same device IDs / `of_match_table` / `MODULE_DEVICE_TABLE` entries as the
   C version it replaces, unless the PR explicitly documents a narrowed
   scope and why.
4. **The C file is deleted in the same PR** that adds its Rust replacement
   (no parallel C+Rust implementations of the same driver left behind),
   and `Kconfig`/`Makefile` are updated accordingly.
5. **Tested, not just compiled**, wherever testing is feasible: QEMU boot
   for core subsystems, `modprobe`/`rmmod` for loadable drivers, or the
   relevant kselftest. If hardware-in-the-loop testing isn't available,
   the PR says so explicitly instead of claiming it.
6. Follow `Documentation/rust/coding-guidelines.rst` and run
   `rustfmt`/`clippy` via `make LLVM=1 rustfmtcheck rust-analyzer`-style
   checks before opening the PR.
7. **Candidate claims are checked against the tree.** Before a driver enters
   the candidate queue below, its "abstraction available" claim must be
   verified against the current tree — cite the file in `rust/kernel/` or
   the `grep` that proves the claim, never rely on memory or on how the tree
   looked when the entry was first written. (This rule exists because the
   queue once claimed nvmem and watchdog Rust support that does not exist.)

## Candidate queue (easiest first)

Picked for: small size, no exotic hardware required to test, and — unless
the row says **needs abstraction** — an existing Rust abstraction already
covers the subsystem (rows marked "needs abstraction" first need a separate,
reviewed step-0 abstraction PR in `rust/kernel/`, never bundled with the
driver port).

### Available abstractions (verified against the tree)

Driver-facing abstractions that exist in `rust/kernel/` (generated with
`ls rust/kernel/*.rs` and `ls rust/kernel/net/`, each entry checked by hand
on `main`, 2026-10-24 — per ground rule 7):

- **Bus/driver**: `platform.rs`, `pci.rs`, `i2c.rs`, `usb.rs`,
  `auxiliary.rs`, `serdev.rs`, `faux.rs`
- **Device class**: `miscdevice.rs`, `block.rs`, `net/phy.rs`, `pwm.rs`,
  `regulator.rs`, `clk.rs`, `cpufreq.rs`, `opp.rs`, `hwrng.rs`
- **Firmware & config**: `firmware.rs`, `configfs.rs`, `debugfs.rs`
- **Discovery**: `of.rs`, `acpi.rs`

Directory-based abstractions also exist: `drm/` (DRM device, driver, file,
GEM, GPUVM, ioctl; used by `drivers/gpu/drm/nova` and `tyr`), `block/`,
`io/`, `irq/`, `fs/`, `mm/`, `net/` and others; `ls -d rust/kernel/*/`
lists them all. `rust/kernel/gpu.rs` only holds the GPU buddy allocator.

Verified **absent**: there is no nvmem or watchdog abstraction anywhere in
`rust/kernel/`. `grep -ril rust drivers/nvmem/` matches only the
Kconfig substring "t**rust**ed application"; `drivers/watchdog/Kconfig`
matches only "ARM-T**rust**ed-Firmware".

| # | Target | Lines (C) | Rust abstraction available | Notes |
|---|--------|-----------|------------------------------|-------|
| 1 | `drivers/char/hw_random/timeriomem-rng.c` | ~200 | platform driver (`rust/kernel/platform.rs`) + hwrng (`rust/kernel/hwrng.rs`) | the `hwrng` abstraction has landed (`rust/kernel/hwrng.rs`; issue #1 closed), so this is now a plain port — issue #2 |
| 2 | A `samples/rust/rust_driver_platform.rs`-style simple platform driver already in `drivers/` with no complex DMA/IRQ | varies | platform (`rust/kernel/platform.rs`) | pick one during triage |
| 3 | A simple `drivers/nvmem/` provider not yet in Rust | small | **needs abstraction** — no nvmem bindings exist in `rust/kernel/`, and no Rust nvmem provider in-tree (the old "already has Rust examples" claim was a false `grep` match on "t**rust**ed") | step 0: add an nvmem abstraction to `rust/kernel/` (separate PR), then port a simple provider |
| 4 | A simple `drivers/hwmon/` sensor driver (I2C, no complex state machine) | small-medium | I2C (`rust/kernel/i2c.rs`, see `samples/rust/rust_i2c_client.rs`) | |
| 5 | A `drivers/watchdog/` driver | small-medium | **needs abstraction** — no watchdog bindings exist in `rust/kernel/` (the old "Kconfig references rust" claim was a false `grep` match on "ARM-T**rust**ed-Firmware") | step 0: issue #11 (watchdog abstraction + sample), then port a simple driver |
| 6 | `drivers/misc/pvpanic/pvpanic-pci.c` | 49 | pci (`rust/kernel/pci.rs`) | **blocked on #9** (`dev_groups` support in driver-core): without it the port would drop the `events`/`capability` sysfs files (`pvpanic_dev_groups` in `drivers/misc/pvpanic/pvpanic.c:178`), breaking rule 3. QEMU-testable (`-device pvpanic-pci`, vendor 0x1b36 / device 0x0011). Issue #10 |
| 7 | `drivers/misc/pvpanic/pvpanic-mmio.c` | 74 | platform (`rust/kernel/platform.rs`) | **blocked on #9** (`dev_groups` support, `pvpanic_dev_groups`); QEMU-testable with the ISA device `-device pvpanic` (described in ACPI as `QEMU0001`, which this driver matches; there is no `pvpanic-mmio` device on x86 QEMU). File its GitHub issue at triage time (labeled `needs-abstraction` until #9 lands) |

Issues are filed on GitHub per candidate once triaged, labeled
`difficulty:N`, `subsystem:<name>`, and `needs-abstraction` when step 0
(writing the safe abstraction) isn't done yet.

## Non-goals (for now)

- Rewriting core subsystems (scheduler, mm, VFS core, networking core) —
  these are explicitly out of scope until the driver-level work has proven
  out the process, has real test coverage, and upstream Rust-for-Linux has
  more of the required abstractions merged.
- Any change that drops hardware support silently.
- Any "converted" code that hasn't actually been built with
  `LLVM=1 CONFIG_RUST=y`.

## Build quickstart

```sh
rustup toolchain install $(./scripts/min-tool-version.sh rustc) \
    --component rust-src rustfmt clippy
cargo install --locked bindgen-cli \
    --version $(./scripts/min-tool-version.sh bindgen)

make LLVM=1 defconfig
scripts/config --enable CONFIG_RUST
make LLVM=1 olddefconfig
make LLVM=1 rustavailable   # should print "Rust is available!"
make LLVM=1 -j$(nproc)
```

If your distro's `clang`/`lld`/`llvm-*` binaries are version-suffixed
(e.g. `clang-21`), symlink them to unsuffixed names on a directory you put
early in `PATH`, or pass `CC=`, `LD=`, `AR=`, etc. explicitly to `make`.

## Syncing with upstream

rlinux is a squashed import with no upstream history, so a sync against
`torvalds/linux` is an overlay onto a pruned tree. The expected conflict
shape is modify/delete: upstream keeps modifying paths that this tree has
deliberately removed. Resolution is mechanical — anything under a path
listed in `scripts/rlinux/pruned-paths.txt` stays deleted:

```sh
grep -v -e '^#' -e '^$' scripts/rlinux/pruned-paths.txt | xargs git rm -r -q --ignore-unmatch
```

If upstream has since started referencing removed paths from files that
remain (e.g. new entries in `drivers/Makefile` or
`Documentation/arch/index.rst`), fix those references in the same way as
done in issue #5.
