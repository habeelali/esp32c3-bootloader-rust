# ESP32-C3 Second-Stage Bootloader — Rust Implementation

## Overview

This is a second-stage bootloader for the **ESP32-C3** RISC-V microcontroller, ported from the ESP-IDF C implementation to `#![no_std]` Rust. The bootloader runs from IRAM at `0x403CE000` and is loaded by the ESP32-C3 ROM first-stage bootloader from flash offset `0x0`.

### Capabilities

| Feature | Status |
|---|---|
| Hardware initialization (clock, UART, WDT, RNG) | Implemented |
| SPI flash subsystem (read, write, erase, QIO, mmap) | Implemented |
| Partition table parsing (factory, test, OTA) | Implemented |
| OTA boot selection (sequence numbers, rollback) | Implemented |
| GPIO long-hold override (factory reset on GPIO9, test app on GPIO8) | Implemented |
| ESP image format parsing & validation (checksum, SHA-256) | Implemented |
| MMU/cache configuration for app launch | Implemented |
| Deep sleep fast boot via RTC retain memory | Implemented |
| Flash encryption (XTS-AES) | Implemented (eFuse-gated, routes via hardware cache) |
| Secure Boot V2 (ECDSA P-256 signature verification) | Implemented (`--features secure_boot`) |
| Anti-rollback (`secure_version` vs eFuse counter) | Implemented |
| Image verification on boot (CONFIG_BOOTLOADER_APP_VERIFY) | Implemented |

---

## Project Structure

```
├── Cargo.toml                  # Crate manifest
├── Cargo.lock                  # Dependency lock
├── build.rs                    # Build script (linker configuration)
├── linker.ld                   # Primary linker script
├── memory.x                    # Alternative linker script (riscv-rt format)
├── rom.ld                      # ROM symbol includes
├── build_image.py              # Generate bootloader.bin from Rust ELF
├── build_partition_table.py    # Generate partition-table.bin (factory + ota_0 + otadata)
├── build_test_app.py           # Build v1 and v2 test app images
├── build_qemu_flash.py         # Assemble 4 MB qemu_flash.bin for QEMU demo
├── ota_push.py                 # Host-side OTA push tool (TCP UART)
├── test_app.S                  # Test app assembly entry point
├── test_app.c                  # Test app interactive shell (OTA receive in v1, auto-valid in v2)
├── test_app.ld                 # Test app linker script
├── tools/
│   └── sign_image/             # CLI: keygen, sign, verify for Secure Boot V2
│       ├── Cargo.toml
│       └── src/
│           ├── main.rs
│           ├── keygen.rs
│           ├── sign.rs
│           ├── verify.rs
│           └── sig_block.rs
└── src/
    ├── main.rs                 # Entry point, boot sequence orchestration
    ├── init.rs                 # Hardware initialization
    ├── flash.rs                # SPI flash subsystem (encryption-aware)
    ├── image.rs                # ESP image format parsing & loading
    ├── partition.rs            # Partition table parsing
    ├── utility.rs              # OTA selection, rollback, MMU config, app launch
    ├── soc.rs                  # SoC register definitions (inc. eFuse registers)
    ├── rom.rs                  # ROM function wrappers
    ├── sha256.rs               # Pure-software SHA-256 & CRC32-LE
    └── secure_boot_key.rs      # Compile-time P-256 trust anchor
```

---

## Build System

### Cargo Configuration

The crate is named `esp32c3-bootloader`, targets `riscv32imc-unknown-none-elf`, and builds as a `no_std` binary.

```toml
[profile.release]
opt-level = "z"       # Optimize for size
lto = true            # Link-time optimization
codegen-units = 1     # Single codegen unit for better LTO
panic = "abort"       # Abort on panic (no unwinding)
debug = false         # No debug info in release
```

**Conditional features:**

| Feature | Description |
|---|---|
| `factory_reset` | Enable GPIO9 long-hold to trigger factory partition boot |
| `app_test` | Enable GPIO8 long-hold to trigger test app partition boot |
| `secure_boot` | Enable Secure Boot V2 (ECDSA P-256 signature verification); pulls in `p256` crate |

### Dependencies

| Crate | Version | Purpose |
|---|---|---|
| `esp32c3` | 0.22 | PAC (Peripheral Access Crate) for ESP32-C3 memory-mapped registers |
| `riscv` | 0.11 | RISC-V CSR access, critical section support |
| `critical-section` | 1.1 | Critical section implementation for single-hart systems |
| `portable-atomic` | 1.5 | Atomic operations backed by critical sections |
| `p256` | 0.13 | ECDSA P-256 signature verification (optional, `secure_boot` feature only) |

### Build Script (`build.rs`)

Passes `-Tlinker.ld` to the linker and tracks changes to `linker.ld` and `build.rs` for rebuilds.

### Image Generation (`build_image.py`)

Invokes `esptool.py elf2image` to produce the final `bootloader.bin`:

```
esptool.py --chip esp32c3 elf2image --version 2 \
    --flash-mode dout --flash-freq 20m --flash-size 4MB \
    -o bootloader.bin target/riscv32imc-unknown-none-elf/release/esp32c3-bootloader
```

---

## Memory Layout

### Linker Script (`linker.ld`)

| Region | Origin | Size | Purpose |
|---|---|---|---|
| IRAM | `0x403CE000` | 32 KB (`0x8000`) | Code, read-only data (executed in-place from IRAM) |
| DRAM | `0x3FC80000` | 32 KB (`0x8000`) | Mutable data (.data, .bss, stack, heap) |

- `.text` and `.rodata` are placed in IRAM.
- `.data` is stored in IRAM (in flash) and copied to DRAM at boot.
- `.bss` is zero-initialized in DRAM at boot.
- The stack grows downward from the top of DRAM.
- The global pointer is set to `_data_start + 0x800`.
- The heap occupies the space between `_bss_end` and `_stack_start`.

### Alternative Memory Layout (`memory.x`)

Uses IRAM origin `0x4037C000` (32 KB). This is compatible with the `riscv-rt` runtime crate if used.

### ROM Symbols (`rom.ld`)

Includes ESP-IDF ROM linker scripts to resolve symbols for ROM functions used by the bootloader:
- `esp32c3.rom.ld` — core ROM function addresses
- `esp32c3.rom.api.ld` — ROM API function addresses
- `esp32c3.rom.newlib.ld` — newlib functions in ROM
- `esp32c3.rom.libc.ld` — libc functions in ROM

---

## Boot Sequence

### Entry Point (`src/main.rs`)

```
CPU Reset
  → _start (assembly)
    → Set global pointer (gp)
    → Set stack pointer (sp)
    → call rust_start()
      → Zero .bss section
      → call call_start_cpu0()
```

### `call_start_cpu0()`

1. **`init::bootloader_init()`** — Initialize hardware
2. **`utility::select_partition_number()`** — Load partition table and select boot partition
3. **`utility::bootloader_utility_load_boot_image()`** — Load and boot the application image

### `BootloaderState` Structure

```rust
struct BootloaderState {
    factory: Option<PartitionPos>,
    test: Option<PartitionPos>,
    ota: [Option<PartitionPos>; 16],
    ota_info: Option<PartitionPos>,
}

struct PartitionPos {
    offset: u32,
    size: u32,
}
```

---

## Module Details

### 1. Hardware Initialization (`src/init.rs`)

Orchestrated by `bootloader_init()`, which calls sub-functions in sequence:

| Function | Purpose |
|---|---|
| `bootloader_hardware_init()` | Check chip revision; apply I2C analog bias calibration for rev < 3 |
| `bootloader_ana_reset_config()` | Configure brown-out detector, super WDT, glitch reset |
| `bootloader_super_wdt_auto_feed()` | Enable super-WDT auto-feed to prevent reset during flash operations |
| `bootloader_clock_configure()` | Set CPU clock to 80 MHz via ROM RTC clock APIs |
| `bootloader_console_init()` | Initialize UART0 at 115200 baud (8N1, source clock = APB at 40 MHz) |
| `bootloader_config_wdt()` | Disable flash-boot watchdog timer |
| `bootloader_enable_random()` | Enable RNG clock for random number generation |
| `bootloader_print_banner()` | Output bootloader banner via UART |

**Register peripherals used:**

- `UART0` — serial console output
- `USB_DEVICE` (`USB_SERIAL_JTAG`) — dual-output for console
- `TIMG0` — timer group for WDT
- `RTC_CNTL` — RTC control registers
- `I2C_MASTER` — I2C for analog bias calibration

**Chip revision handling:**

Reads EFUSE registers to determine chip version. For `rev < 3` (ECO0-ECO2), applies I2C master analog bias calibration workaround. The `efuse_hal_get_disable_wafer_version_major()` function checks if wafer version major is disabled in EFUSE.

---

### 2. SPI Flash Subsystem (`src/flash.rs`)

Provides a complete SPI flash driver for the ESP32-C3.

#### Initialization Flow

```
bootloader_init_spi_flash()
  → Configure SPI pins (CS0=10, MOSI=11, CLK=12, MISO=13, WP=14, HD=15)
  → bootloader_flash_resume()           # Resume from deep sleep
  → bootloader_flash_unlock()           # Clear block protect bits
  → bootloader_enable_qio_mode()        # Enable Quad I/O if supported
  → Detect flash size via SFDP or RDID
  → Configure SPI clock divider
```

#### Flash Commands

| Function | Description |
|---|---|
| `bootloader_execute_flash_command()` | Execute generic SPI1 user command (cmd, addr, dummy, MOSI/MISO widths) |
| `bootloader_read_flash_id()` | Read JEDEC manufacturer/device ID (RDID, 0x9F) |
| `bootloader_flash_read_sfdp()` | Read SFDP table (RDSFDP, 0x5A) for flash parameter discovery |
| `bootloader_flash_read()` | Read data from flash (raw SPI or cache-window with optional AES-XTS decryption) |
| `bootloader_flash_write()` | Program pages via ROM `esp_rom_spiflash_write()` |
| `bootloader_flash_erase_sector()` | Erase 4 KB sector via ROM `esp_rom_spiflash_erase_sector()` |
| `bootloader_flash_erase_range()` | Erase arbitrary range (aligned to sectors) |
| `bootloader_flash_unlock()` | Clear block protect bits, per-vendor (ISSI, MXIC, GD, Winbond, XMC, TH) |
| `bootloader_enable_qio_mode()` | Set QE bit on flash chip, configure SPI controller for QIO |
| `bootloader_mmap()` / `bootloader_munmap()` | Map/unmap flash regions via 4096-byte bounce buffer |

#### SPI Clock Configuration

| Frequency | Divider |
|---|---|
| 80 MHz | 1 |
| 40 MHz | 2 |
| 26.7 MHz | 3 |
| 20 MHz | 4 |

#### Flash Memory Mapping

Uses a reserved MMU page as a 64 KB sliding window (`flash_read_via_cache()`) for efficient large reads. This is also used for AES-XTS decrypted reads when flash encryption is enabled.

---

### 3. ESP Image Format (`src/image.rs`)

Parses and validates ESP32 application images.

#### Image Header (24 bytes, packed)

```rust
#[repr(C, packed)]
struct EspImageHeader {
    magic: u8,              // 0xE9
    segment_count: u8,
    spi_mode: u8,           // 0=QIO, 1=QOUT, 2=DIO, 3=DOUT, 4=FAST_READ, 5=SLOW_READ
    spi_speed: u8,          // 0=40MHz, 1=26.7MHz, 2=20MHz, 0xF=80MHz
    spi_size: u8,           // 0=1MB, 1=2MB, 2=4MB, 3=8MB, 4=16MB
    entry_addr: u32,        // Application entry point
    wp_pin: u8,             // Write protect pin
    spi_pin_drv: [u8; 3],   // SPI pin drive strength
    chip_id: u16,           // 5 for ESP32-C3
    min_chip_rev: u8,
    reserved: [u8; 8],      // Future use
    hash_appended: u8,      // 1 if SHA-256 digest appended
}
```

#### Segment Header (8 bytes, packed)

```rust
#[repr(C, packed)]
struct EspImageSegmentHeader {
    load_addr: u32,    // Target address in memory
    data_len: u32,     // Length of segment data in bytes
}
```

#### Segment Classification

| Type | Address Range | Action |
|---|---|---|
| DRAM | `0x3FC80000` – `0x3FCE0000` | Copy from flash to RAM (`should_load()`) |
| IRAM | `0x40370000` – `0x403E0000` | Copy from flash to RAM (`should_load()`) |
| RTC_FAST | `0x50000000` – `0x50002000` | Copy from flash to RTC memory (`should_load()`) |
| DROM | `0x3C000000` – `0x3C800000` | MMU-mapped from flash (`should_map()`) |
| IROM | `0x42000000` – `0x42800000` | MMU-mapped from flash (`should_map()`) |

#### Image Processing Pipeline (`process_image()`)

1. Read 24-byte image header from flash
2. Hash the header with SHA-256
3. Iterate over segments:
   - Read 8-byte segment header
   - Hash the segment header
   - Read segment data, XOR-checksum each byte (accumulator starts at `0xEF`)
   - Hash segment data
   - Classify segment (`should_load` / `should_map`)
4. Read final checksum byte, verify against computed XOR checksum
5. Read appended SHA-256 digest (32 bytes), verify against computed hash

#### Public API

| Function | Description |
|---|---|
| `bootloader_load_image()` | Full load: parse, verify, copy segments, configure MMU |
| `bootloader_load_image_no_verify()` | Load without SHA-256 verification |
| `esp_image_verify()` | Verify image checksum and SHA-256 only |
| `esp_image_get_metadata()` | Parse headers into `EspImageMetadata` structure |
| `bootloader_common_check_chip_validity()` | Check that image chip_id matches ESP32-C3 (5) |
| `esp_image_get_flash_size()` | Return configured flash size in bytes |

---

### 4. Partition Table (`src/partition.rs`)

Parses the ESP32 partition table located at flash offset `0x8000`.

#### Partition Entry (32 bytes, packed)

```rust
#[repr(C, packed)]
struct EspPartitionInfo {
    magic: u16,          // 0x50AA
    type_: u8,           // Partition type
    subtype: u8,         // Partition subtype
    offset: u32,         // Offset from start of flash
    size: u32,           // Size in bytes
    label: [u8; 16],     // ASCII label
    flags: u32,          // Flags
}
```

#### Partition Type Constants

| Constant | Value | Description |
|---|---|---|
| `PART_TYPE_APP` | `0x00` | Application partition |
| `PART_TYPE_DATA` | `0x01` | Data partition |
| `PART_SUBTYPE_FACTORY` | `0x00` | Factory app |
| `PART_SUBTYPE_TEST` | `0x01` | Test app |
| `PART_SUBTYPE_OTA_FLAG` | `0x10` | OTA slot bitmask |
| `PART_SUBTYPE_OTA_0` – `PART_SUBTYPE_OTA_15` | `0x10` – `0x1F` | OTA slots 0–15 |

#### Validation (`esp_partition_table_verify()`)

- Checks magic byte `0x50AA` on each entry
- Validates offsets and sizes are within flash bounds
- Detects overlapping partitions
- Validates labels contain printable ASCII only

#### Loading (`bootloader_utility_load_partition_table()`)

1. Memory-map the partition table region at flash offset `0x8000` (0xC00 bytes, up to 96 entries)
2. Parse up to 96 entries
3. Classify into `BootloaderState`:
   - `type=APP, subtype=FACTORY` → `state.factory`
   - `type=APP, subtype=TEST` → `state.test`
   - `type=APP, subtype=OTA_0..OTA_15` → `state.ota[n]`
   - `type=DATA, subtype=OTA` → `state.ota_info`

---

### 5. Boot Selection & App Loading (`src/utility.rs`)

#### Boot Partition Selection (`select_partition_number()`)

```
select_partition_number()
  → bootloader_utility_load_partition_table()   # Parse partitions
  → selected_boot_partition()                    # Select boot partition
```

#### GPIO Override (`selected_boot_partition()`)

| GPIO | Feature Gate | Action |
|---|---|---|
| GPIO9 (held low at boot) | `factory_reset` | Boot from factory partition |
| GPIO8 (held low at boot) | `app_test` | Boot from test app partition |

GPIO levels are read via `esp_rom_gpio_pad_select_gpio()` and `gpio_ll_get_level()`.

#### OTA Boot Selection (`bootloader_utility_get_selected_boot_partition()`)

1. Read OTA data partition (`state.ota_info`)
2. Parse two OTA select entries (`EspOtaSelectEntry`):
   ```rust
   struct EspOtaSelectEntry {
       ota_seq: u32,      // Sequence counter
       ota_state: u32,    // State (0=new, 1=pending_verify, 2=valid, 3=invalid, 4=aborted)
       crc: u32,          // CRC of the entry
   }
   ```
3. Handle `PENDING_VERIFY` → roll back to previous boot slot
4. Select active OTA slot based on highest sequence number
5. Fall back to factory partition if no valid OTA slot found
6. If boot slot changed, write a new OTA data entry via `set_actual_ota_seq()`

#### App Image Loading (`bootloader_utility_load_boot_image()`)

1. Sweep partitions **backwards** then **forwards**, attempting to load each
2. For each partition, call `try_load_partition()`:
   - Anti-rollback check (stub)
   - Call `bootloader_load_image()` to parse and verify the image
   - Call `load_image()` to configure MMU and jump to app

#### MMU Configuration & App Launch (`set_cache_and_start_app()`)

1. Sort segments by type (DROM/IROM)
2. For each DROM segment → `Cache_Dbus_MMU_Set()` (data bus MMU)
3. For each IROM segment → `Cache_Ibus_MMU_Set()` (instruction bus MMU)
4. Invalidate ICache
5. Disable RNG, disable glitch reset
6. Jump to app entry point via `jalr x0, <entry>, 0` (inline assembly `jr`)

#### RTC Retain Memory (Deep Sleep Fast Boot)

```rust
struct RtcRetainMem {
    crc: u32,
    ota_seq: [u32; 2],
    active_ota_seq: u32,
}
```

Stored at `SOC_RTC_IRAM_LOW` (`0x50000000`), CRC-protected. On fast wake from deep sleep, the bootloader reads the previously used OTA slot from RTC memory, skipping partition table and OTA data reads.

---

### 6. SOC Register Definitions (`src/soc.rs`)

#### System & Peripheral Registers

| Register | Address | Purpose |
|---|---|---|
| `SYSTEM_CPU_PERI_CLK_EN_REG` | `0x600C0000` | Peripheral clock enable |
| `SYSTEM_CPU_PERI_RST_EN_REG` | `0x600C0004` | Peripheral reset enable |
| `SYSTEM_PERIP_CLK_EN1_REG` | `0x600C0040` | Additional clock enables (crypto, RNG) |
| `SYSTEM_PERIP_RST_EN1_REG` | `0x600C0044` | Additional reset enables |

#### RTC Control Registers

| Register | Offset | Purpose |
|---|---|---|
| `RTC_CNTL_OPTIONS0_REG` | `0x00` | RTC options |
| `RTC_CNTL_SWD_CONF_REG` | `0x68` | Super WDT configuration |
| `RTC_CNTL_SWD_WPROTECT_REG` | `0x6C` | Super WDT write protect |
| `RTC_CNTL_BROWN_OUT_REG` | `0x88` | Brown-out detector |
| `RTC_CNTL_FIB_SEL_REG` | `0xD0` | Glitch filter select |
| `RTC_CNTL_ANA_CONF_REG` | `0x34` | Analog configuration |

#### SPI Memory Registers

Helper constants generate register addresses for SPI0 and SPI1:

```rust
SPI_MEM_CMD_REG(n)      // command register
SPI_MEM_CTRL_REG(n)     // control register
SPI_MEM_USER_REG(n)     // user register
SPI_MEM_ADDR_REG(n)     // address register
SPI_MEM_CTRL1_REG(n)    // control register 1
SPI_MEM_CTRL2_REG(n)    // control register 2
SPI_MEM_MOSI_DLEN_REG(n)// MOSI data length
SPI_MEM_MISO_DLEN_REG(n)// MISO data length
```

#### MSPI Pin Numbers (GPIO Matrix)

| Signal | GPIO |
|---|---|
| CS0 | 10 |
| MOSI | 11 |
| CLK | 12 |
| MISO | 13 |
| WP | 14 |
| HD | 15 |

#### Flash Geometry

| Constant | Value |
|---|---|
| `FLASH_SECTOR_SIZE` | 4096 |
| `FLASH_BLOCK_SIZE` | 65536 |
| `SPI_FLASH_MMU_PAGE_SIZE` | 65536 |

#### Memory Map Addresses

| Region | Address Range |
|---|---|
| DROM (data) | `0x3C00_0000` – `0x3C80_0000` |
| IROM (instruction) | `0x4200_0000` – `0x4280_0000` |
| DRAM | `0x3FC8_0000` – `0x3FCE_0000` |
| IRAM | `0x4037_0000` – `0x403E_0000` |
| RTC IRAM | `0x5000_0000` – `0x5000_2000` |
| ROM stack start | `0x3FCD_F7F0` |

#### EFUSE Registers

Base address `0x60008800`. Key register offsets:

| Register | Offset | Purpose |
|---|---|---|
| `EFUSE_RD_MAC_SPI_SYS_0_REG` | `0x044` | MAC address, SPI pad configuration |
| `EFUSE_RD_MAC_SPI_SYS_1_REG` | `0x048` | SPI pad drive strength |
| `EFUSE_RD_MAC_SPI_SYS_3_REG` | `0x050` | Chip version info |
| `EFUSE_RD_MAC_SPI_SYS_4_REG` | `0x054` | PKG version, WAFER version |
| `EFUSE_RD_MAC_SPI_SYS_5_REG` | `0x058` | Flash capacity, temperature sensor, disable WAFER version |

#### Chip Revision

```rust
fn efuse_hal_chip_revision() -> u32  // major * 100 + minor
fn efuse_hal_get_disable_wafer_version_major() -> bool
```

---

### 7. ROM Function Wrappers (`src/rom.rs`)

All ROM function calls use `core::mem::transmute` to convert known ROM addresses to function pointers.

#### General ROM Functions

| Function | ROM Address | Description |
|---|---|---|
| `ets_delay_us()` | `0x4000019C` | Microsecond delay |
| `uart_tx_wait_idle()` | `0x400002C0` | Wait for UART TX to complete |
| `software_reset()` | `0x4000028C` | Software CPU reset |
| `rtc_get_reset_reason()` | `0x40000270` | Get reset reason from RTC |

#### SPI Flash ROM Functions

| Function | ROM Address | Description |
|---|---|---|
| `esp_rom_spiflash_read()` | `0x40000458` | Read data from flash |
| `esp_rom_spiflash_write()` | `0x40000494` | Program flash page |
| `esp_rom_spiflash_write_encrypted()` | `0x400004A8` | Program encrypted flash page |
| `esp_rom_spiflash_erase_sector()` | `0x40000444` | Erase 4 KB sector |
| `esp_rom_spiflash_erase_block()` | `0x4000044C` | Erase 64 KB block |
| `esp_rom_spiflash_config_clk()` | `0x4000040C` | Configure SPI clock divider |
| `esp_rom_spiflash_config_readmode()` | `0x400004BC` | Configure SPI read mode |
| `esp_rom_spiflash_config_param()` | `0x400003C4` | Configure flash chip parameters |
| `esp_rom_spiflash_select_qio_pins()` | `0x400003F0` | Configure pins for QIO mode |
| `esp_rom_spiflash_unlock()` | `0x4000055C` | Unlock flash chip |

#### Cache/MMU ROM Functions

| Function | ROM Address | Description |
|---|---|---|
| `Cache_Disable_ICache()` | `0x40000304` | Disable instruction cache |
| `Cache_Enable_ICache()` | `0x4000030C` | Enable instruction cache |
| `Cache_Suspend_ICache()` | `0x40000310` | Suspend instruction cache |
| `Cache_Resume_ICache()` | `0x40000314` | Resume instruction cache |
| `Cache_Invalidate_ICache_All()` | `0x4000031C` | Invalidate entire ICache |
| `Cache_MMU_Init()` | `0x400002E0` | Initialize MMU |
| `Cache_Dbus_MMU_Set()` | `0x40000320` | Map flash page to data bus MMU |
| `Cache_Ibus_MMU_Set()` | `0x40000660` | Map flash page to instruction bus MMU |

#### Flash Chip State Accessors

| Function | Description |
|---|---|
| `g_rom_flashchip()` | Read pointer to `spiflash_legacy_data` structure in ROM |
| `g_rom_flashchip_mut()` | Mutable pointer to `spiflash_legacy_data` structure |
| `g_rom_spiflash_dummy_len_plus()` | Read dummy cycle adjustment value at offset 28 |

#### GPIO Functions (Inline)

| Function | Description |
|---|---|
| `esp_rom_gpio_pad_select_gpio()` | Select GPIO function for a pad (IO MUX) |
| `esp_rom_gpio_pad_pullup_only()` | Enable pull-up resistor on a pad |
| `esp_rom_gpio_pad_set_drv()` | Set pad drive strength |
| `gpio_ll_input_enable()` | Enable input on a GPIO pin |
| `gpio_ll_get_level()` | Read GPIO input level |

#### CRC32

```rust
fn esp_rom_crc32_le(crc: u32, buf: &[u8]) -> u32
```

Implements CRC32-LE (polynomial `0xEDB88320`, same as zlib/gzip).

#### EFUSE Functions

| Function | Description |
|---|---|
| `esp_rom_efuse_get_flash_gpio_info()` | Get flash GPIO configuration from EFUSE |
| `esp_rom_efuse_get_flash_wp_gpio()` | Get flash WP GPIO from EFUSE |
| `efuse_hal_flash_encryption_enabled()` | Check `SPI_BOOT_CRYPT_CNT` EFUSE field |

#### SPI Flash Constants

| Constant | Value | Description |
|---|---|---|
| `CMD_RDID` | `0x9F` | Read JEDEC ID |
| `CMD_RDSR` | `0x05` | Read Status Register |
| `CMD_WREN` | `0x06` | Write Enable |
| `CMD_RDSFDP` | `0x5A` | Read SFDP |
| `CMD_FAST_READ` | `0x0B` | Fast Read |
| `CMD_PP` | `0x02` | Page Program |
| `CMD_SE` | `0x20` | Sector Erase (4KB) |
| `CMD_BE` | `0xD8` | Block Erase (64KB) |

#### Reset Reasons

| Constant | Value | Description |
|---|---|---|
| `RESET_REASON_CHIP_POWER_ON` | `0x01` | Power-on reset |
| `RESET_REASON_CHIP_BROWN_OUT` | `0x06` | Brown-out reset |
| `RESET_REASON_CHIP_SUPER_WDT` | `0x07` | Super WDT reset |
| `RESET_REASON_CHIP_GLITCH_RTC` | `0x08` | RTC glitch reset |
| `RESET_REASON_CORE_DEEP_SLEEP` | `0x0A` | Wake from deep sleep |
| `RESET_REASON_CORE_SW` | `0x0B` | Software CPU reset |

#### `SpiFlashReadMode` Enum

| Variant | Value |
|---|---|
| `FastRead` | 0 |
| `ReadStatus` | 1 |
| `ReadSFDP` | 2 |

#### `SpiFlashResult` Enum

| Variant | Value |
|---|---|
| `Ok` | 0 |
| `Err` | 1 |

#### `RtcClkConfig` Structure

```rust
struct RtcClkConfig {
    cpu_freq_mhz: u32,
    slow_clk_src: u32,
    fast_clk_src: u32,
}
```

- `default_config()`: Returns config with `cpu_freq_mhz = 80`
- `rtc_clk_init()`: Stub that trusts the ROM's default clock configuration

---

### 8. SHA-256 & CRC32 (`src/sha256.rs`)

#### CRC32-LE

```rust
fn bootloader_crc32_le(crc: u32, buf: &[u8]) -> u32
```

Standard CRC32-LE using polynomial `0xEDB88320`.

#### SHA-256

Pure-software implementation with a 2-context static pool:

```rust
static mut CTX_POOL: [Sha256Context; 2]
static mut CTX_USED: [bool; 2]
```

**`Sha256Context` Fields:**

| Field | Size | Description |
|---|---|---|
| `state` | 8 × u32 | Hash state (H0–H7) |
| `buffer` | 64 × u8 | Input data buffer |
| `buffer_len` | usize | Bytes buffered |
| `total_bits` | u64 | Total bits processed |

**SHA-256 Constants:**

- Initial hash values (H0–H7): `[0x6a09e667, 0xbb67ae85, 0x3c6ef372, 0xa54ff53a, 0x510e527f, 0x9b05688c, 0x1f83d9ab, 0x5be0cd19]`
- 64 round constants K[0..63]: Standard SHA-256 specification values

**Helper Functions:**

| Function | Operation |
|---|---|
| `ch(x, y, z)` | `(x & y) ^ (~x & z)` |
| `maj(x, y, z)` | `(x & y) ^ (x & z) ^ (y & z)` |
| `sigma0(x)` | `rotr(x, 2) ^ rotr(x, 13) ^ rotr(x, 22)` |
| `sigma1(x)` | `rotr(x, 6) ^ rotr(x, 11) ^ rotr(x, 25)` |
| `gamma0(x)` | `rotr(x, 7) ^ rotr(x, 18) ^ shr(x, 3)` |
| `gamma1(x)` | `rotr(x, 17) ^ rotr(x, 19) ^ shr(x, 10)` |

**Public API (ESP-IDF compatible):**

```rust
type Sha256Handle = u32;

fn bootloader_sha256_start() -> Sha256Handle
fn bootloader_sha256_data(handle: Sha256Handle, data: &[u8])
fn bootloader_sha256_finish(handle: Sha256Handle, digest: &mut [u8; 32])
```

Each call to `bootloader_sha256_start()` allocates a context from the 2-element pool. `Sha256Handle` is the pool index (0 or 1). The pool wraps once both contexts are allocated.

---

---

### 9. Secure Boot V2 (`src/image.rs`, `src/secure_boot_key.rs`, `tools/sign_image/`)

Enabled with `--features secure_boot`. Verifies an ECDSA P-256 signature block appended to the bootloader image before the ROM hands off control.

#### Signature Block Layout (4096 bytes, at image end aligned to 4096)

| Offset | Size | Field |
|---|---|---|
| 0 | 1 | Magic (`0xE7`) |
| 1 | 2 | Version |
| 3 | 1 | Signature count |
| 4 | 64 | ECDSA P-256 signature (r ‖ s, big-endian) |
| 68 | 64 | Public key (X ‖ Y, big-endian) |
| 132 | 32 | Image SHA-256 digest |
| 164 | 3932 | Padding |

#### Trust Anchor

The verifying public key is embedded at compile time in `src/secure_boot_key.rs`:

```rust
pub const SECURE_BOOT_PUBLIC_KEY: [u8; 64] = [ /* X ‖ Y */ ];
```

Verification succeeds only if the signature block's public key matches the compile-time constant **and** the ECDSA signature over the image SHA-256 digest is valid.

#### `sign-image` CLI Tool (`tools/sign_image/`)

```bash
# Generate key pair
cargo run --manifest-path tools/sign_image/Cargo.toml -- keygen \
  --out signing_key.pem [--out-rust src/secure_boot_key.rs]

# Sign image (appends 4096-byte signature block)
cargo run --manifest-path tools/sign_image/Cargo.toml -- sign \
  --key signing_key.pem --in bootloader.bin --out bootloader_signed.bin

# Verify
cargo run --manifest-path tools/sign_image/Cargo.toml -- verify \
  --key signing_key.pem --in bootloader_signed.bin
```

---

### 10. Anti-Rollback (`src/utility.rs`, `src/soc.rs`)

Prevents downgrading to an older firmware version after a security fix.

#### Mechanism

Each ESP image carries a `secure_version` field in `esp_app_desc_t` (located 32 bytes into the first DRAM segment). The bootloader reads the eFuse anti-rollback counter from `EFUSE_RD_REPEAT_DATA4_REG[29:14]` and rejects any image whose `secure_version` is below that counter.

#### eFuse Register

```
EFUSE_RD_REPEAT_DATA4_REG = 0x60008840
bits [29:14] = SECURE_VERSION (16-bit counter)
```

#### Behavior

- If `image.secure_version < efuse_counter` → partition is rejected, bootloader tries next slot
- Counter is a hardware fuse — once burned it cannot decrease
- In QEMU (all eFuse bits = 0) the counter is 0, so all images pass

---

### 11. Flash Encryption (`src/flash.rs`, `src/rom.rs`, `src/soc.rs`)

Transparent XTS-AES hardware encryption for flash reads.

#### Detection

`efuse_hal_flash_encryption_enabled()` reads `EFUSE_RD_REPEAT_DATA1_REG[20:18]` (`SPI_BOOT_CRYPT_CNT`). Odd popcount = encryption enabled.

```
EFUSE_RD_REPEAT_DATA1_REG = 0x60008834
bits [20:18] = SPI_BOOT_CRYPT_CNT (3-bit field)
```

#### Routing

`bootloader_mmap()` checks the eFuse at first call (result cached in a `static mut` flag) and routes accordingly:

| Condition | Path |
|---|---|
| Encryption disabled (QEMU, unprovisioned hardware) | `flash_read_raw()` — direct SPI read |
| Encryption enabled | `flash_read_via_cache()` — reads through MMU cache window, hardware decrypts transparently |

All higher-level callers (`bootloader_load_image`, partition table parsing, OTA data reads) are unaffected — they go through `bootloader_mmap` and get plaintext regardless.

---

## Test Application

Located in the project root, the test application provides an interactive shell for verifying the bootloader works correctly.

### Build Process (`build_test_app.py`)

Builds two variants in one invocation:

1. Compiles `test_app.S` + `test_app.c` with `riscv64-unknown-elf-gcc`:
   - Architecture: `rv32imc`, ABI: `ilp32`, no stdlib (`-nostdlib`)
   - Link script: `test_app.ld`, entry point: `0x40380000`
   - v1: no extra flags → `test_app_v1.elf` / `test_app_v1.img`
   - v2: `-DV2` → `test_app_v2.elf` / `test_app_v2.img`
2. Extracts raw binary via `objcopy -O binary`
3. Constructs ESP image:
   - Header: magic=0xE9, 1 segment, DOUT, 20MHz, 4MB, entry=0x40380000, chip_id=5
   - Segment: load_addr=0x40380000, data=raw binary
   - XOR checksum (accumulator starts at 0xEF)
   - 16-byte alignment, checksum byte appended
4. Copies `test_app_v1.img` → `test_app.img` for backward compatibility

### Test App Variants

Two variants are built from the same source with a `V2` preprocessor flag:

| Variant | File | Description |
|---|---|---|
| v1 (factory) | `test_app_v1.img` | Has `ota` UART command; loads at `0x10000` |
| v2 (OTA slot) | `test_app_v2.img` | Auto-marks itself valid on startup; loads at `0x20000` |

### Shell Commands (`test_app.c`)

**v1 commands:**

| Command | Description |
|---|---|
| `help` | Show available commands |
| `info` | Chip info, hart ID, SYSTIMER ticks since boot |
| `mem` | `_image_end`, stack pointer, free bytes above image |
| `regs` | Dump `mstatus`, `mtvec`, `sp`, SYSTIMER, UART status |
| `echo <text>` | Echo text back |
| `ota <size>` | Receive `<size>` bytes of firmware over UART, write to `ota_0` flash partition, update OTA data entry, reset |

**v2 additions:**
- Calls `mark_ota_valid()` on startup (writes `{seq=1, state=VALID}` to sector 0 of otadata)
- Displays `[OTA v2]` in the welcome banner; `ota` command is not present

### OTA Protocol (v1 `cmd_ota`)

1. Drain UART FIFO (spin + flush residual bytes after command line)
2. Send `READY\r\n`
3. Erase enough 4 KB sectors in `ota_0` to hold the firmware
4. Receive firmware in 256-byte chunks (blocking `getc_uart_blocking`), write each chunk to flash via `ROM_WRITE`, ACK with `.`
5. Write OTA data entry `{seq=1, state=NEW, crc}` to sector 0 of otadata
6. Send `OK\r\n`, spin briefly, call `ROM_RESET`

ROM function addresses used by the test app:

| Symbol | Address | Purpose |
|---|---|---|
| `ROM_UNLOCK` | `0x40000140` | `esp_rom_spiflash_unlock` |
| `ROM_ERASE` | `0x40000128` | `esp_rom_spiflash_erase_sector` |
| `ROM_WRITE` | `0x4000012C` | `esp_rom_spiflash_write` |
| `ROM_RESET` | `0x40000090` | Software reset |

**WDT Handling:**
The assembly entry point (`test_app.S`) disables Timer Group 0 WDT before jumping to C code:
1. Write `0x50D83AA1` → `TIMG0_WDTWRITEPROTECT` (unlock)
2. Write `0` → `TIMG0_WDTCONFIG0` (disable)
3. Write `1` → `TIMG0_WDTFEED` (feed/confirm)
4. Write `0` → `TIMG0_WDTWRITEPROTECT` (re-lock)

---

## Reference Disassembly

A full disassembly of the official ESP-IDF `bootloader_esp32c3.bin` (v5.4, from `esp-rs/espflash`) is provided in `../disasm_full.txt` (7839 lines) for comparison and validation purposes. The binary was disassembled with `riscv64-unknown-elf-objdump -D`.

Key offset ranges in the reference binary:

| Offset Range | Content |
|---|---|
| `0x0000` – `0x10F0` | Main bootloader code (IRAM) |
| `0x10F0` – `0x3000` | Support functions (SHA, flash, etc.) |
| `0x3000` – `0x6000` | Additional handlers |
| `0x6000` – `0x7000` | Data sections |

---

## Build & Flash Instructions

### Prerequisites

- Rust toolchain with `riscv32imc-unknown-none-elf` target:
  ```bash
  rustup target add riscv32imc-unknown-none-elf
  ```
- Python 3 with `esptool`:
  ```bash
  pip install esptool
  ```
- For the test app: `riscv64-unknown-elf-gcc` cross-compiler toolchain

### Building

```bash
# Bootloader ELF + bin
cargo build --release
python3 build_image.py          # → bootloader.bin

# Partition table
python3 build_partition_table.py  # → partition-table.bin

# Test app (v1 factory + v2 OTA)
python3 build_test_app.py       # → test_app_v1.img, test_app_v2.img

# QEMU flash image (4 MB)
python3 build_qemu_flash.py     # → qemu_flash.bin
```

### Flashing to ESP32-C3

```bash
esptool.py --chip esp32c3 write_flash 0x0     bootloader.bin
esptool.py --chip esp32c3 write_flash 0x8000  partition-table.bin
esptool.py --chip esp32c3 write_flash 0x10000 test_app_v1.img
```

Connect at 115200 baud to see bootloader output and interact with the test app shell.

### QEMU OTA Demo

```bash
# Terminal 1 — start QEMU with TCP serial
qemu-system-riscv32 -machine esp32c3 -nographic \
  -drive file=qemu_flash.bin,if=mtd,format=raw \
  -serial tcp::5555,server,nowait -monitor none 2>/dev/null

# Terminal 2 — push OTA firmware
python3 ota_push.py --port 5555 --firmware test_app_v2.img
```

Expected output: factory v1 shell → `READY` → `.` ACKs per chunk → device resets → bootloader selects ota_0 → v2 banner with `[OTA v2]`.

---

## Porting Notes

### Key Differences from ESP-IDF C Implementation

1. **No RTOS dependencies**: The ESP-IDF bootloader relies on some FreeRTOS primitives. This port replaces them with direct register access and spin loops.

2. **ROM function calls**: C uses linker-provided symbols for ROM functions. Rust uses `core::mem::transmute` with hardcoded addresses from the ROM linker script.

3. **Static state management**: C uses global variables. Rust uses `static mut` with careful management of mutable static state (e.g., `BootloaderState`, SHA-256 context pool).

4. **Inline assembly**: Entry point (`_start`) and MMU jump (`set_cache_and_start_app`) use `global_asm!` and `asm!` macros.

5. **Packed structs**: ESP image headers and partition entries use `#[repr(C, packed)]` for exact binary layout matching.

6. **No alloc**: The bootloader does not use the Rust allocator. All memory is statically allocated or stack-allocated.

7. **Feature gates**: GPIO long-hold triggers (`factory_reset`, `app_test`) are behind Cargo features to avoid pulling in unnecessary GPIO code when not needed.