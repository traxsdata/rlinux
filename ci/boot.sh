#!/usr/bin/env bash
# Boot the kernel from ci/build.sh in QEMU with a busybox initramfs. /init
# checks the system, loads and unloads Rust sample modules, prints one
# "RLINUX-CI:" marker per check, and powers off. Fails on a panic, an oops, a
# timeout, or a missing marker. Markers are written to a second serial port
# (ttyS1, not a console) so kernel output cannot split them; /init prints the
# full dmesg on the console (ttyS0) at the end, so warnings and oopses are
# still caught.
. "$(dirname "$0")/lib.sh"
BZ=$KOUT/arch/x86/boot/bzImage
[ -f "$BZ" ] || die "no $BZ; run ci/build.sh first"
BUSYBOX=$(command -v "${BUSYBOX:-busybox}") || die "busybox not found"
file -L "$BUSYBOX" | grep -q 'statically linked' || die "$BUSYBOX is not statically linked"
if [ -z "${QEMU_ACCEL:-}" ]; then
  if [ -r /dev/kvm ] && [ -w /dev/kvm ]; then QEMU_ACCEL=kvm; else QEMU_ACCEL=tcg; fi
fi

step "initramfs"
root=$CI_OUT/initramfs
rm -rf "$root"; mkdir -p "$root"/{bin,dev,proc,sys,tmp}
cp "$BUSYBOX" "$root/bin/busybox"
for a in sh mount uname cat grep insmod rmmod modprobe lsmod poweroff dmesg ls echo sleep sync dd wc; do
  ln -s busybox "$root/bin/$a"
done
"${MAKE[@]}" INSTALL_MOD_PATH="$root" INSTALL_MOD_STRIP=1 modules_install >/dev/null
kver=$(cat "$KOUT/include/config/kernel.release")
cat > "$root/init" <<'INIT'
#!/bin/sh
# Markers go to ttyS1, which is not a kernel console, so kernel messages
# (even emergency-level ones, which ignore loglevel) cannot split them.
m() { echo "RLINUX-CI: $*" > /dev/ttyS1; }
mount -t proc proc /proc
mount -t sysfs sys /sys
mount -t devtmpfs dev /dev 2>/dev/null
m "init $(uname -r)"
[ -b /dev/vda ] && m "virtio-blk ok"
grep -q rust_minimal /lib/modules/$(uname -r)/modules.dep && m "modules.dep ok"
modprobe rust_minimal && m "modprobe rust_minimal ok"
modprobe rust_print && m "modprobe rust_print ok"
rmmod rust_print && rmmod rust_minimal && m "rmmod ok"
dmesg | grep -q 'Rust minimal sample (exit)' && m "rust exit message ok"
hw=/sys/class/misc/hw_random
modprobe rust_hwrng && grep -qw rust_hwrng $hw/rng_available && m "hwrng registered"
echo rust_hwrng > $hw/rng_current && [ "$(dd if=/dev/hwrng bs=16 count=1 2>/dev/null | wc -c)" = 16 ] && m "hwrng read ok"
rmmod rust_hwrng && ! grep -qw rust_hwrng $hw/rng_available && m "hwrng unregistered"
dmesg | grep -q 'rust_hwrng: init' && dmesg | grep -q 'rust_hwrng: cleanup' && m "hwrng callbacks ok"
m "done"
m "dmesg-begin"
dmesg
m "dmesg-end"
sync
poweroff -f
INIT
chmod +x "$root/init"
# The kernel's own cpio writer (built with the kernel), so the host needs no
# cpio; files are mapped to root:root.
(cd "$KOUT" && "$SRC/scripts/gen_initramfs.sh" -o "$CI_OUT/initramfs.cpio" \
  -u "$(id -u)" -g "$(id -g)" "$root")
gzip -1 -f "$CI_OUT/initramfs.cpio"
truncate -s 16M "$CI_OUT/disk.img"

step "boot ($QEMU_ACCEL)"
log=$CI_OUT/boot-serial.log markers=$CI_OUT/boot-markers.log
cpu=max; [ "$QEMU_ACCEL" = kvm ] && cpu=host
rc=0
timeout "${BOOT_TIMEOUT:-300}" qemu-system-x86_64 -machine q35,accel="$QEMU_ACCEL" -cpu "$cpu" \
  -m 1024 -smp 2 -nographic -no-reboot -monitor none -serial "file:$log" -serial "file:$markers" \
  -kernel "$BZ" -initrd "$CI_OUT/initramfs.cpio.gz" \
  -drive file="$CI_OUT/disk.img",format=raw,if=virtio \
  -append "console=ttyS0 loglevel=4 panic=-1 oops=panic" || rc=$?
tr -d '\r' < "$markers" || true
fail=
[ "$rc" = 0 ] || fail="qemu exit $rc (124 = timeout)"
grep -qE 'Kernel panic|BUG:|Oops|WARNING: CPU|Call Trace' "$log" && fail="kernel error in serial log"
for m in "init $kver" "virtio-blk ok" "modules.dep ok" "modprobe rust_minimal ok" \
         "modprobe rust_print ok" "rmmod ok" "rust exit message ok" \
         "hwrng registered" "hwrng read ok" "hwrng unregistered" "hwrng callbacks ok" "done"; do
  grep -qxF "RLINUX-CI: $m" <(tr -d '\r' < "$markers") || fail=${fail:-"missing marker: RLINUX-CI: $m"}
done
if [ -n "$fail" ]; then
  echo "--- console log tail ($log)"; tail -60 "$log"
  die "$fail"
fi
echo "ci: boot OK"
