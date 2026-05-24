#!/usr/bin/env python3
"""Build ESP32-C3 test app images for the OTA demo (v1 = factory, v2 = ota_0)."""

import struct
import subprocess
import sys
import os

ASM_SRC  = "test_app.S"
C_SRC    = "test_app.c"

LOAD_ADDR = 0x40380000  # IRAM after bootloader region


def compile_variant(extra_defines: list[str], elf_out: str, bin_out: str) -> bytes:
    cmd = [
        "riscv64-unknown-elf-gcc",
        "-nostdlib", "-nostartfiles", "-nodefaultlibs",
        "-march=rv32imc", "-mabi=ilp32",
        "-Os", "-ffreestanding", "-fno-builtin",
        "-fno-asynchronous-unwind-tables", "-fno-unwind-tables",
        "-T", "test_app.ld",
        "-Wl,--build-id=none",
        *extra_defines,
        "-o", elf_out, ASM_SRC, C_SRC,
    ]
    subprocess.run(cmd, check=True)
    subprocess.run(["riscv64-unknown-elf-objcopy", "-O", "binary", elf_out, bin_out],
                   check=True)
    with open(bin_out, "rb") as f:
        return f.read()


def build_esp_image(payload: bytes, img_out: str) -> None:
    """Wrap raw binary in an ESP image (header + segment header + checksum)."""
    magic         = 0xE9
    segment_count = 1
    spi_mode      = 3       # DOUT
    spi_speed     = 2       # 20 MHz
    spi_size      = 2       # 4 MB
    entry_addr    = LOAD_ADDR

    header = struct.pack("<BBBBIB3BHBHHB4s",
        magic, segment_count, spi_mode, (spi_size << 4) | spi_speed,
        entry_addr,
        0xEE,             # wp_pin
        0, 0, 0,          # spi_pin_drv[3]
        5,                # chip_id (ESP32-C3)
        0,                # legacy min_chip_rev
        0,                # min_chip_rev_full
        0xFFFF,           # max_chip_rev_full
        0,                # hash_appended
        b"\x00\x00\x00\x00",
    )
    seg_header = struct.pack("<II", LOAD_ADDR, len(payload))

    checksum = 0xEF
    for b in payload:
        checksum ^= b

    image = header + seg_header + payload
    while len(image) % 16 != 15:
        image += b'\x00'
    image += bytes([checksum & 0xFF])

    with open(img_out, "wb") as f:
        f.write(image)

    print(f"  {img_out}: {len(image)} bytes, entry=0x{entry_addr:08X}, "
          f"checksum=0x{checksum & 0xFF:02X}")


print("Building test_app v1 (factory — has 'ota' command)...")
v1_payload = compile_variant([], "test_app_v1.elf", "test_app_v1_payload.bin")
build_esp_image(v1_payload, "test_app_v1.img")

print("Building test_app v2 (OTA slot — auto-marks valid, shows v2 banner)...")
v2_payload = compile_variant(["-DV2"], "test_app_v2.elf", "test_app_v2_payload.bin")
build_esp_image(v2_payload, "test_app_v2.img")

# Keep backward-compatible test_app.img pointing at v1
import shutil
shutil.copy("test_app_v1.img", "test_app.img")
print("Copied test_app_v1.img → test_app.img (backward compat)")
