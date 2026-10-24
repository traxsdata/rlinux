# rlinux-port

Does the mechanical part of porting a small C driver to Rust, so the porter
(human or the local LLM worker) can spend its time on the driver logic.
Plain Python 3, no dependencies. Run it from the top of an rlinux tree.

| Command | What it does |
|---|---|
| `rlinux-port analyze <file.c \| REV:file.c>` | Facts (bus, driver/module name, OF/ACPI/PHY IDs, hwrng/PHY callbacks, resources, Kconfig symbol and dependencies, Makefile line) and a check of every C API used against what `rust/kernel` provides. Ends with `verdict: portable`, `needs abstraction` (with the gaps listed) or `no template`. Exit code 0 only when portable. |
| `rlinux-port scaffold <file.c>` | Generates `<file>.rs` from a template (platform + hwrng, or PHY): register `#define`s become `const`s, plus the module macro, ID tables, registration, drop-order skeleton and each callback as a stub with **the C body pasted in as comments**. Adds the Kconfig `depends on RUST…`, writes `ci/config.d/<sym>.config` and `ci/modules.d/<sym>.txt`, and `git rm`s the C file (kbuild would otherwise keep building it; use `--keep-c` to keep it). Refuses when `analyze` finds gaps (`--force` to override). |
| `rlinux-port compare <REV:file.c> <file.rs>` | Lists each function's register accesses (operation + offset) in the C original and the Rust port and diffs them, compares the OF ID tables, and lists any `TODO(port)` left. Exit code 0 only when the accesses and IDs match and no TODO is left. |
| `rlinux-port api-index [--out FILE]` | A one-line-per-item index of the public `rust/kernel` API (path:line, signature, `impl` context). The worker's copy is `/opt/aether-shared/rust-kernel-api.txt`: grep it instead of reading whole source files. |

## Typical port

```sh
P=scripts/rlinux/port/rlinux-port
$P analyze drivers/char/hw_random/foo-rng.c        # must say "portable"
$P scaffold drivers/char/hw_random/foo-rng.c       # writes foo-rng.rs, Kconfig/CI edits, git rm foo-rng.c
# translate every TODO(port) in foo-rng.rs (the C code is in the comments, or
# `git show HEAD:drivers/char/hw_random/foo-rng.c`), then:
make LLVM=-21 O=.ci-out/kernel drivers/char/hw_random/foo-rng.o   # fast inner loop
LLVM=-21 ci/build.sh && LLVM=-21 ci/rustfmt.sh && LLVM=-21 ci/boot.sh
$P compare HEAD:drivers/char/hw_random/foo-rng.c drivers/char/hw_random/foo-rng.rs
```

## What it does not do

- It never translates function bodies; that is the porter's job.
- The C parser is regex- and brace-based, written for small single-file
  drivers. Check its facts against the source when something looks odd, and
  improve the tool when you find a gap.
- Support checks are greps in `rust/kernel` (e.g. "is there `fn config_init`
  in `phy.rs`?"), so verdicts follow the tree as abstractions land.
- `compare` checks the set of accesses and lists the sequence per function;
  confirming the order and the values written is still part of the review.

## Adding a template

Add a `scaffold_<kind>()` function next to `scaffold_platform_hwrng()` and
`scaffold_phy()`, and pick it in `cmd_scaffold()`. Add new C API knowledge to
`RULES` (with a `present` grep when the Rust side may or may not exist yet).
Test a template by scaffolding an already-ported driver from its pre-port
revision and building it with `ci/build.sh`.
