#!/usr/bin/env python3
"""Assemble qemu_flash.bin from all components for the OTA demo.

Flash layout:
  0x00000  bootloader.bin
  0x08000  partition-table.bin
  0x10000  test_app_v1.img   (factory)
  0x20000  test_app_v2.img   (ota_0 — pre-loaded so rollback demo works too)
  0x3E000  (all 0xFF — erased otadata; OTA written here at runtime by app)
"""

import sys
import os

FLASH_SIZE = 0x400000  # 4 MB

LAYOUT = [
    (0x00000, "bootloader.bin"),
    (0x08000, "partition-table.bin"),
    (0x10000, "test_app_v1.img"),
    (0x20000, "test_app_v2.img"),
]


def main():
    flash = bytearray(b"\xff" * FLASH_SIZE)

    for offset, path in LAYOUT:
        if not os.path.exists(path):
            print(f"ERROR: {path} not found — run build scripts first", file=sys.stderr)
            sys.exit(1)
        with open(path, "rb") as f:
            data = f.read()
        end = offset + len(data)
        if end > FLASH_SIZE:
            print(f"ERROR: {path} at 0x{offset:05x} overflows flash", file=sys.stderr)
            sys.exit(1)
        flash[offset:end] = data
        print(f"  0x{offset:05x}  {path}  ({len(data)} bytes)")

    out = "qemu_flash.bin"
    with open(out, "wb") as f:
        f.write(flash)
    print(f"\nWritten to {out}  ({FLASH_SIZE // 1024} KB)")
    print("\nTo run QEMU (TCP serial for ota_push.py):")
    qemu = "/home/x/.espressif/tools/esp-qemu/qemu/bin/qemu-system-riscv32"
    print(f"  {qemu} \\")
    print(f"    -machine esp32c3 -nographic \\")
    print(f"    -drive file=qemu_flash.bin,if=mtd,format=raw \\")
    print(f"    -serial tcp::5555,server,nowait -monitor none 2>/dev/null")
    print("\nOr interactive (stdio serial):")
    print(f"  {qemu} \\")
    print(f"    -machine esp32c3 -nographic \\")
    print(f"    -drive file=qemu_flash.bin,if=mtd,format=raw \\")
    print(f"    -serial mon:stdio 2>/dev/null")


if __name__ == "__main__":
    main()
