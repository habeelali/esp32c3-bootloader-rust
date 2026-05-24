#!/usr/bin/env python3
"""Build a minimal ESP32-C3 partition table for the OTA demo."""

import struct

OUT = "partition-table.bin"
TABLE_SIZE = 0xC00  # 3 KB (max 95 entries × 32 bytes = 3040 bytes)

# Flash layout:
#   0x00000  bootloader
#   0x08000  partition table
#   0x10000  factory    (64 KB)  — test app v1
#   0x20000  ota_0     (120 KB)  — receives v2 firmware
#   0x3E000  otadata     (8 KB)  — 2× 4 KB OTA-select sectors


def label_bytes(label: str) -> bytes:
    raw = label.encode("ascii")
    if len(raw) > 15:
        raise ValueError("partition label must fit in 15 bytes plus NUL")
    return raw + b"\x00" * (16 - len(raw))


def part(type_, subtype, offset, size, label):
    return struct.pack("<HBBII16sI",
                       0x50AA, type_, subtype, offset, size,
                       label_bytes(label), 0)


factory = part(0x00, 0x00, 0x10000, 0x10000, "factory")
ota_0   = part(0x00, 0x10, 0x20000, 0x1E000, "ota_0")    # subtype 0x10 = OTA slot 0
otadata = part(0x01, 0x00, 0x3E000, 0x02000, "otadata")  # data / ota-info

image = factory + ota_0 + otadata + b"\xff" * (TABLE_SIZE - len(factory) - len(ota_0) - len(otadata))

with open(OUT, "wb") as f:
    f.write(image)

print(f"Written to {OUT}")
print(f"  factory : offset=0x{0x10000:05x}, size=0x{0x10000:x}")
print(f"  ota_0   : offset=0x{0x20000:05x}, size=0x{0x1E000:x}")
print(f"  otadata : offset=0x{0x3E000:05x}, size=0x{0x02000:x}")
