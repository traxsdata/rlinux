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
- Safe abstractions for several subsystems (PCI, platform, I2C, auxiliary,
  serdev, misc/char devices, DMA, configfs, debugfs, HID transport, etc.)
- A handful of real in-tree Rust drivers (`drivers/net/phy/*_rust.rs`,
  `drivers/nvmem/`, parts of `drivers/tee/`, `drivers/android/binder`, etc.)
  and sample drivers under `samples/rust/` demonstrating each abstraction.

So "convert to Rust" mostly means: **replace one existing C driver/module
with a Rust implementation using an existing kernel Rust abstraction**, not
inventing new unsafe bindings from scratch. When no abstraction exists yet
for a subsystem, that is its own, separately-reviewed task (adding a safe
abstraction in `rust/kernel/`), never bundled with a driver port.

## Supported architectures

Supported: **x86, arm, arm64, riscv, loongarch, powerpc, s390, um** —
the eight architectures where Rust builds (they select `HAVE_RUST`).

The other thirteen upstream architectures (alpha, arc, csky, hexagon,
m68k, microblaze, mips, nios2, openrisc, parisc, sh, sparc, xtensa),
together with their arch-only drivers, sound boards and documentation,
have been pruned from the tree (issue #5, "Diet"). This is a deliberate,
documented narrowing of scope that falls under the exception of ground
rule 3: a driver converted to Rust can never build on an architecture
that does not select `HAVE_RUST`, so those ports are dead weight for this
project. The exact list of removed paths is recorded in
`scripts/rlinux/pruned-paths.txt`.

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

## Candidate queue (easiest first)

Picked for: small size, an existing Rust abstraction already covers the
subsystem, and no exotic hardware required to test.

| # | Target | Lines (C) | Rust abstraction available | Notes |
|---|--------|-----------|------------------------------|-------|
| 1 | `drivers/char/hw_random/timeriomem-rng.c` | ~200 | platform driver (`rust/kernel/platform.rs`), needs a new thin `hwrng` abstraction | Good first "add an abstraction + port a driver" pair; no `hwrng` bindings exist yet in `rust/kernel/`, so step 1 is a small, separately-reviewed abstraction PR |
| 2 | A `samples/rust/rust_driver_platform.rs`-style simple platform driver already in `drivers/` with no complex DMA/IRQ | varies | platform | pick one during triage |
| 3 | A simple `drivers/nvmem/` provider not yet in Rust | small | nvmem already has Rust examples in-tree | follow existing Rust nvmem provider as a template |
| 4 | A simple `drivers/hwmon/` sensor driver (I2C, no complex state machine) | small-medium | I2C abstraction (`rust/kernel/i2c.rs`-equivalent, see `samples/rust/rust_i2c_client.rs`) | |
| 5 | A `drivers/watchdog/` driver (subsystem already has some Rust usage) | small-medium | watchdog Kconfig already references rust | |

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
