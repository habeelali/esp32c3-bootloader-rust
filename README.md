# ESP32-C3 Second-Stage Bootloader in Rust

A complete second-stage bootloader for the **ESP32-C3** (RISC-V) microcontroller, ported from the ESP-IDF C implementation to `#![no_std]` Rust. Runs from IRAM at `0x403CE000`, loaded by the ROM first-stage bootloader from flash offset `0x0`.

## Features

- Ported from C to Rust with `#![no_std]`, no allocator — fits in 32 KB IRAM
- Partition table parser, ESP image header verification, and MMU configuration
- Pure-software SHA-256 image integrity verification
- OTA dual-slot boot selection with automatic rollback protection (PENDING_VERIFY state machine)
- Secure Boot V2: ECDSA P-256 signature verification with compile-time trust anchor
- Anti-rollback: `secure_version` field checked against hardware eFuse counter
- Flash encryption: transparent XTS-AES detection and routing via hardware cache
- GPIO long-hold boot overrides (GPIO9 → factory reset, GPIO8 → test app)
- Custom `sign-image` CLI tool: key generation, signing, and verification
- Full OTA demo in QEMU: push firmware over UART → erase/write flash → reboot → new slot boots → app marks itself valid

## Repository Layout

```
├── src/                    # Bootloader Rust source
│   ├── main.rs             # Entry point, _start, boot orchestration
│   ├── init.rs             # Clock (80 MHz), UART0 (115200), WDT, RNG, SPI init
│   ├── flash.rs            # SPI flash driver (read/write/erase, QIO, mmap)
│   ├── image.rs            # ESP image parsing, checksum, SHA-256, segment loading
│   ├── partition.rs        # Partition table parsing
│   ├── utility.rs          # OTA selection, GPIO override, MMU config, app launch
│   ├── rom.rs              # ROM function wrappers
│   ├── soc.rs              # Register address constants
│   ├── sha256.rs           # Pure-software SHA-256 and CRC32-LE
│   └── secure_boot_key.rs  # Compile-time trust anchor (P-256 public key)
├── tools/
│   └── sign_image/         # CLI: keygen, sign, verify for Secure Boot V2
├── test_app.c              # Test application (v1 factory + v2 OTA variant)
├── test_app.S              # Test app entry point assembly
├── test_app.ld             # Test app linker script
├── build_image.py          # Wrap ELF in ESP image format
├── build_partition_table.py# Generate partition-table.bin
├── build_test_app.py       # Build v1 and v2 test app images
├── build_qemu_flash.py     # Assemble qemu_flash.bin (4 MB)
├── ota_push.py             # Host-side OTA push tool (TCP UART)
├── linker.ld               # Bootloader linker script
├── rom.ld                  # ROM symbol addresses
└── memory.x                # Alternative riscv-rt memory layout
```

## Prerequisites

```bash
rustup target add riscv32imc-unknown-none-elf
pip install esptool
# For test app only:
# riscv64-unknown-elf-gcc
```

Optional Cargo features:
- `--features factory_reset` — GPIO9 long-hold → boot factory partition
- `--features app_test` — GPIO8 long-hold → boot test app partition
- `--features secure_boot` — enable ECDSA P-256 image signature verification

## Build

```bash
# Build bootloader ELF
cargo build --release

# Generate bootloader.bin
python3 build_image.py

# Build partition table
python3 build_partition_table.py

# Build test app (produces test_app_v1.img and test_app_v2.img)
python3 build_test_app.py

# Assemble full 4 MB QEMU flash image
python3 build_qemu_flash.py
```

## Flash to Hardware

```bash
esptool.py --chip esp32c3 write_flash 0x0     bootloader.bin
esptool.py --chip esp32c3 write_flash 0x8000  partition-table.bin
esptool.py --chip esp32c3 write_flash 0x10000 test_app_v1.img
```

## QEMU OTA Demo

Run QEMU with TCP serial (requires [Espressif QEMU](https://github.com/espressif/qemu)):

```bash
# Terminal 1 — start QEMU
qemu-system-riscv32 -machine esp32c3 -nographic \
  -drive file=qemu_flash.bin,if=mtd,format=raw \
  -serial tcp::5555,server,nowait -monitor none 2>/dev/null

# Terminal 2 — push OTA firmware
python3 ota_push.py --port 5555 --firmware test_app_v2.img
```

Expected flow:
1. QEMU boots → factory v1 shell (`esp32c3>`)
2. `ota_push.py` sends `ota <size>`, device responds `READY`
3. Firmware streamed in 256-byte chunks (`.` ACK per chunk)
4. Device writes to `ota_0` partition, updates OTA data, resets
5. Bootloader selects `ota_0` on next boot
6. v2 app starts, marks itself valid (cancels rollback)

For interactive use, replace `-serial tcp::5555,...` with `-serial mon:stdio`.

## Secure Boot V2

```bash
# Generate key pair
cargo run --manifest-path tools/sign_image/Cargo.toml -- keygen --out signing_key.pem

# Sign a firmware image
cargo run --manifest-path tools/sign_image/Cargo.toml -- sign \
  --key signing_key.pem --in bootloader.bin --out bootloader_signed.bin

# Verify signature
cargo run --manifest-path tools/sign_image/Cargo.toml -- verify \
  --key signing_key.pem --in bootloader_signed.bin
```

The tool outputs a `secure_boot_key.rs` snippet with the public key as a Rust constant for embedding in the bootloader at compile time.

## Flash Layout

| Offset   | Size   | Contents              |
|----------|--------|-----------------------|
| `0x00000`| 32 KB  | Bootloader            |
| `0x08000`| 4 KB   | Partition table       |
| `0x10000`| 64 KB  | Factory app (v1)      |
| `0x20000`| 120 KB | OTA slot 0 (v2)       |
| `0x3E000`| 8 KB   | OTA data (2× sectors) |

## Architecture

### Boot Sequence

```
ROM → _start (asm) → rust_start() → call_start_cpu0()
  → bootloader_init()            hardware init
  → select_partition_number()    parse partition table, OTA selection
  → bootloader_utility_load_boot_image()  verify image, configure MMU, jump
```

### Memory

| Region | Address      | Size  | Use                          |
|--------|-------------|-------|------------------------------|
| IRAM   | `0x403CE000` | 32 KB | Code + rodata                |
| DRAM   | `0x3FC80000` | 32 KB | `.data`, `.bss`, stack       |

## License

MIT
