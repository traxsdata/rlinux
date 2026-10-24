#!/usr/bin/env bash
# Boot the kernel from ci/build.sh in QEMU with a busybox initramfs. /init
# checks the system, loads and unloads Rust sample modules, prints one
# "RLINUX-CI:" marker per check, and powers off. Fails on a panic, an oops, a
# timeout, or a missing marker. loglevel=4 keeps routine kernel messages from
# interleaving with the markers on the console; /init prints the full dmesg
# at the end, so warnings and oopses are still caught.
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
for a in sh mount uname cat grep insmod rmmod modprobe lsmod poweroff dmesg ls echo sleep sync; do
  ln -s busybox "$root/bin/$a"
done
"${MAKE[@]}" INSTALL_MOD_PATH="$root" INSTALL_MOD_STRIP=1 modules_install >/dev/null
kver=$(cat "$KOUT/include/config/kernel.release")
cat > "$root/init" <<'INIT'
#!/bin/sh
mount -t proc proc /proc
mount -t sysfs sys /sys
mount -t devtmpfs dev /dev 2>/dev/null
echo "RLINUX-CI: init $(uname -r)"
[ -b /dev/vda ] && echo "RLINUX-CI: virtio-blk ok"
grep -q rust_minimal /lib/modules/$(uname -r)/modules.dep && echo "RLINUX-CI: modules.dep ok"
modprobe rust_minimal && echo "RLINUX-CI: modprobe rust_minimal ok"
modprobe rust_print && echo "RLINUX-CI: modprobe rust_print ok"
rmmod rust_print && rmmod rust_minimal && echo "RLINUX-CI: rmmod ok"
dmesg | grep -q 'Rust minimal sample (exit)' && echo "RLINUX-CI: rust exit message ok"
echo "RLINUX-CI: done"
echo "RLINUX-CI: dmesg-begin"
dmesg
echo "RLINUX-CI: dmesg-end"
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
log=$CI_OUT/boot-serial.log
cpu=max; [ "$QEMU_ACCEL" = kvm ] && cpu=host
rc=0
timeout "${BOOT_TIMEOUT:-300}" qemu-system-x86_64 -machine q35,accel="$QEMU_ACCEL" -cpu "$cpu" \
  -m 1024 -smp 2 -nographic -no-reboot -monitor none -serial "file:$log" \
  -kernel "$BZ" -initrd "$CI_OUT/initramfs.cpio.gz" \
  -drive file="$CI_OUT/disk.img",format=raw,if=virtio \
  -append "console=ttyS0 loglevel=4 panic=-1 oops=panic" || rc=$?
grep 'RLINUX-CI:' "$log" || true
fail=
[ "$rc" = 0 ] || fail="qemu exit $rc (124 = timeout)"
grep -qE 'Kernel panic|BUG:|Oops|WARNING: CPU|Call Trace' "$log" && fail="kernel error in serial log"
for m in "init $kver" "virtio-blk ok" "modules.dep ok" "modprobe rust_minimal ok" \
         "modprobe rust_print ok" "rmmod ok" "rust exit message ok" "done"; do
  grep -qF "RLINUX-CI: $m" "$log" || fail=${fail:-"missing marker: RLINUX-CI: $m"}
done
if [ -n "$fail" ]; then
  echo "--- serial log tail ($log)"; tail -60 "$log"
  die "$fail"
fi
echo "ci: boot OK"
