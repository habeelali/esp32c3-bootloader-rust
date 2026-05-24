#!/usr/bin/env python3
"""Create the ESP32-C3 second-stage bootloader image from the Rust ELF.

Set SIGNING_KEY to the path of a PKCS#8 PEM private key to also produce
bootloader_signed.bin with a Secure Boot V2 signature block appended:

    SIGNING_KEY=../keys/bootloader.pem python3 build_image.py
"""

import os
import subprocess

ELF = "target/riscv32imc-unknown-none-elf/release/esp32c3-bootloader"
BIN = "bootloader.bin"

subprocess.run(
    [
        "esptool.py",
        "--chip",
        "esp32c3",
        "elf2image",
        "--version",
        "2",
        "--flash-mode",
        "dout",
        "--flash-freq",
        "20m",
        "--flash-size",
        "4MB",
        "-o",
        BIN,
        ELF,
    ],
    check=True,
)

print(f"Written to {BIN}")

key = os.environ.get("SIGNING_KEY")
if key:
    signed = "bootloader_signed.bin"
    subprocess.run(
        [
            "cargo", "run",
            "--manifest-path", "../tools/sign_image/Cargo.toml",
            "--release", "--",
            "sign", "--key", key, "--in", BIN, "--out", signed,
        ],
        check=True,
    )
    print(f"Signed image written to {signed}")
