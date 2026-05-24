//! SOC (System-on-Chip) register definitions for ESP32-C3 bootloader.
//! These are the key registers used by the bootloader, mapped from ESP-IDF headers.

#![allow(unused)]

// ============================================================
// System & Clock registers
// ============================================================
pub const SYSTEM_CPU_PERI_CLK_EN_REG: u32 = 0x600C0000;
pub const SYSTEM_CPU_PERI_RST_EN_REG: u32 = 0x600C0004;
pub const SYSTEM_CLK_EN_ASSIST_DEBUG: u32 = 1 << 11;
pub const SYSTEM_RST_EN_ASSIST_DEBUG: u32 = 1 << 11;

// ============================================================
// RTC CNTL registers (Low-power management)
// ============================================================
pub const RTC_CNTL_BASE: u32 = 0x60008000;

pub const RTC_CNTL_SWD_WPROTECT_REG: u32 = RTC_CNTL_BASE + 0x00AC;
pub const RTC_CNTL_SWD_WKEY_VALUE: u32 = 0x8F1D312A;
pub const RTC_CNTL_SWD_CONF_REG: u32 = RTC_CNTL_BASE + 0x00B0;
pub const RTC_CNTL_SWD_AUTO_FEED_EN: u32 = 1 << 31;
pub const RTC_CNTL_SWD_BYPASS_RST: u32 = 1 << 30;

pub const RTC_CNTL_FIB_SEL_REG: u32 = RTC_CNTL_BASE + 0x0014;
pub const RTC_CNTL_FIB_SUPER_WDT_RST: u32 = 1 << 31;
pub const RTC_CNTL_FIB_GLITCH_RST: u32 = 1 << 30;

pub const RTC_CNTL_ANA_CONF_REG: u32 = RTC_CNTL_BASE + 0x0034;
pub const RTC_CNTL_GLITCH_RST_EN: u32 = 1 << 18;

pub const RTC_CNTL_INT_ENA_REG: u32 = RTC_CNTL_BASE + 0x000C;
pub const RTC_CNTL_INT_CLR_REG: u32 = RTC_CNTL_BASE + 0x0010;

// ============================================================
// SPI Memory registers (SPI0/SPI1 for flash)
// ============================================================
pub const SPI0_BASE: u32 = 0x60003000;
pub const SPI1_BASE: u32 = 0x60002000;

// SPI_MEM peripheral base (SPI1 for flash memory access)
pub const SPIMEM0_BASE: u32 = SPI0_BASE;
pub const SPIMEM1_BASE: u32 = SPI1_BASE;

// SPI_MEM register offsets
const SPI_MEM_CMD_OFFSET: u32 = 0x00;
const SPI_MEM_ADDR_OFFSET: u32 = 0x04;
const SPI_MEM_CTRL_OFFSET: u32 = 0x08;
const SPI_MEM_CTRL1_OFFSET: u32 = 0x0C;
const SPI_MEM_CTRL2_OFFSET: u32 = 0x10;
const SPI_MEM_USER_OFFSET: u32 = 0x18;
const SPI_MEM_USER1_OFFSET: u32 = 0x1C;
const SPI_MEM_USER2_OFFSET: u32 = 0x20;

// SPI_MEM register helpers
pub const fn spi_mem_reg(base: u32, offset: u32) -> u32 {
    base + offset
}

pub const fn SPI_MEM_CMD_REG(n: u32) -> u32 {
    if n == 0 { SPI0_BASE + SPI_MEM_CMD_OFFSET } else { SPI1_BASE + SPI_MEM_CMD_OFFSET }
}
pub const fn SPI_MEM_CTRL_REG(n: u32) -> u32 {
    if n == 0 { SPI0_BASE + SPI_MEM_CTRL_OFFSET } else { SPI1_BASE + SPI_MEM_CTRL_OFFSET }
}
pub const fn SPI_MEM_CTRL2_REG(n: u32) -> u32 {
    if n == 0 { SPI0_BASE + SPI_MEM_CTRL2_OFFSET } else { SPI1_BASE + SPI_MEM_CTRL2_OFFSET }
}
pub const fn SPI_MEM_USER_REG(n: u32) -> u32 {
    if n == 0 { SPI0_BASE + SPI_MEM_USER_OFFSET } else { SPI1_BASE + SPI_MEM_USER_OFFSET }
}
pub const fn SPI_MEM_ADDR_REG(n: u32) -> u32 {
    if n == 0 { SPI0_BASE + SPI_MEM_ADDR_OFFSET } else { SPI1_BASE + SPI_MEM_ADDR_OFFSET }
}

// SPI_MEM bit definitions
pub const SPI_MEM_FREAD_QIO: u32 = 1 << 24;
pub const SPI_MEM_FREAD_QUAD: u32 = 1 << 20;
pub const SPI_MEM_FREAD_DIO: u32 = 1 << 23;
pub const SPI_MEM_FREAD_DUAL: u32 = 1 << 21;
pub const SPI_MEM_FASTRD_MODE: u32 = 1 << 13;

pub const SPI_MEM_CS_HOLD_M: u32 = 1 << 5;
pub const SPI_MEM_CS_SETUP_M: u32 = 1 << 6;
pub const SPI_MEM_CS_HOLD_TIME_V: u32 = 0x3F;
pub const SPI_MEM_CS_HOLD_TIME_S: u32 = 16;
pub const SPI_MEM_CS_SETUP_TIME_V: u32 = 0x3F;
pub const SPI_MEM_CS_SETUP_TIME_S: u32 = 8;

pub const SPI_MEM_FDUMMY_OUT: u32 = 1 << 3;
pub const SPI_MEM_D_POL: u32 = 1 << 1;
pub const SPI_MEM_Q_POL: u32 = 1 << 4;

pub const SPI_MEM_CACHE_SCT: u32 = 1 << 0;

// ============================================================
// GPIO / IO MUX registers
// ============================================================
pub const MAX_PAD_GPIO_NUM: u8 = 21;

pub const MSPI_IOMUX_PIN_NUM_CLK: u8 = 12;
pub const MSPI_IOMUX_PIN_NUM_MISO: u8 = 13;
pub const MSPI_IOMUX_PIN_NUM_MOSI: u8 = 11;
pub const MSPI_IOMUX_PIN_NUM_CS0: u8 = 10;
pub const MSPI_IOMUX_PIN_NUM_HD: u8 = 15;
pub const MSPI_IOMUX_PIN_NUM_WP: u8 = 14;

pub const ESP_ROM_EFUSE_FLASH_DEFAULT_SPI: u32 = 0;
pub const ESP_ROM_EFUSE_FLASH_DEFAULT_HSPI: u32 = 1;

// ============================================================
// Assist Debug registers
// ============================================================
pub const ASSIST_DEBUG_BASE: u32 = 0x600CE000;
pub const ASSIST_DEBUG_CORE_0_RCD_EN_REG: u32 = ASSIST_DEBUG_BASE + 0x00;
pub const ASSIST_DEBUG_CORE_0_RCD_PDEBUGEN: u32 = 1 << 3;
pub const ASSIST_DEBUG_CORE_0_RCD_RECORDEN: u32 = 1 << 0;

// ============================================================
// SPI Flash geometry
// ============================================================
pub const FLASH_SECTOR_SIZE: u32 = 4096;
pub const FLASH_BLOCK_SIZE: u32 = 65536;
pub const SPI_FLASH_SEC_SIZE: u32 = FLASH_SECTOR_SIZE;
pub const SPI_FLASH_MMU_PAGE_SIZE: u32 = 65536;
pub const CONFIG_MMU_PAGE_SIZE: u32 = 65536;

// ESP32-C3 address map
pub const SOC_DROM_LOW: u32 = 0x3C000000;
pub const SOC_DROM_HIGH: u32 = 0x3E000000;
pub const SOC_IROM_LOW: u32 = 0x42000000;
pub const SOC_IROM_HIGH: u32 = 0x44000000;
pub const SOC_DRAM_FLASH_ADDRESS_LOW: u32 = SOC_DROM_LOW;
pub const SOC_DRAM_FLASH_ADDRESS_HIGH: u32 = SOC_DROM_HIGH;

pub const SOC_DIRAM_DRAM_LOW: u32 = 0x3FCA0000;
pub const SOC_DIRAM_DRAM_HIGH: u32 = 0x3FCE0000;
pub const SOC_DIRAM_IRAM_LOW: u32 = 0x403A0000;
pub const SOC_DIRAM_IRAM_HIGH: u32 = 0x403E0000;

pub const SOC_RTC_IRAM_LOW: u32 = 0x50000000;
pub const SOC_RTC_IRAM_HIGH: u32 = 0x50002000;
pub const SOC_RTC_DRAM_LOW: u32 = 0x50002000;
pub const SOC_RTC_DRAM_HIGH: u32 = 0x50004000;

pub const SOC_ROM_STACK_START: u32 = 0x3FCE2000;

// ============================================================
// MMU / Cache
// ============================================================
pub const MMU_VADDR_DATA: u32 = 1 << 0;
pub const MMU_VADDR_INSTRUCTION: u32 = 1 << 1;
pub const MMU_TARGET_FLASH0: u32 = 0;

pub const MMAP_ALIGNED_MASK: u32 = SPI_FLASH_MMU_PAGE_SIZE - 1;
pub const MMU_FLASH_MASK: u32 = !(SPI_FLASH_MMU_PAGE_SIZE - 1);

pub const fn mmu_flash_mask_from_val(mmu_page_size: u32) -> u32 {
    !(mmu_page_size - 1)
}

// MMU mapping window: ESP32-C3 DROM = 0x3C000000 - 0x3E000000 (32 MB, 512 pages)
pub const MMU_BLOCK0_VADDR: u32 = SOC_DROM_LOW;
pub const MMU_TOTAL_SIZE: u32 = SOC_DRAM_FLASH_ADDRESS_HIGH - SOC_DRAM_FLASH_ADDRESS_LOW;
pub const MMU_END_VADDR: u32 = MMU_BLOCK0_VADDR + MMU_TOTAL_SIZE;
pub const MMU_BLOCKL_VADDR: u32 = MMU_END_VADDR - CONFIG_MMU_PAGE_SIZE;
pub const FLASH_READ_VADDR: u32 = MMU_BLOCKL_VADDR;
pub const FLASH_MMAP_VADDR: u32 = MMU_BLOCK0_VADDR;
pub const MMU_FREE_PAGES: u32 = MMU_TOTAL_SIZE / CONFIG_MMU_PAGE_SIZE;

// ============================================================
// Bootloader offsets
// ============================================================
pub const ESP_BOOTLOADER_OFFSET: u32 = 0x0;
pub const ESP_BOOTLOADER_SIZE: u32 = 0x7000;
pub const ESP_PRIMARY_BOOTLOADER_OFFSET: u32 = 0x0;
pub const ESP_PARTITION_TABLE_OFFSET: u32 = 0x8000;
pub const ESP_PARTITION_TABLE_MAX_LEN: u32 = 0xC00;

// ============================================================
// EFUSE registers for ESP32-C3
// ============================================================
pub const EFUSE_BASE: u32 = 0x60008800;
pub const EFUSE_RD_MAC_SPI_SYS_0_REG: u32 = EFUSE_BASE + 0x044; // CHIP_VER, CHIP_PACKAGE
pub const EFUSE_RD_CHIP_VER_REG: u32 = EFUSE_BASE + 0x044; // Alias for chip version register
pub const EFUSE_RD_MAC_SPI_SYS_3_REG: u32 = EFUSE_BASE + 0x180; // FLASH_GPIO_INFO, FLASH_WP_GPIO, WAFER_VERSION_MAJOR
pub const EFUSE_RD_MAC_SPI_SYS_4_REG: u32 = EFUSE_BASE + 0x184; // BLK_VERSION_MAJOR
pub const EFUSE_RD_REPEAT_DATA0_REG: u32 = EFUSE_BASE + 0x030; // BLK0 bits [63:32]
pub const EFUSE_RD_REPEAT_DATA1_REG: u32 = EFUSE_BASE + 0x034; // BLK0 bits [95:64] — SPI_BOOT_CRYPT_CNT [20:18]
pub const EFUSE_RD_REPEAT_DATA2_REG: u32 = EFUSE_BASE + 0x038; // BLK0 bits [127:96]
pub const EFUSE_RD_REPEAT_DATA3_REG: u32 = EFUSE_BASE + 0x03C; // BLK0 bits [159:128] — SECURE_BOOT_EN bit[20]
pub const EFUSE_RD_REPEAT_DATA4_REG: u32 = EFUSE_BASE + 0x040; // BLK0 bits [191:160] — SECURE_VERSION [29:14]

// CHIP_VER field: bits [20:18] in RD_MAC_SPI_SYS_0_REG (ECO version)
// CHIP_PACKAGE field: bits [17:15]
// CHIP_ID field: bits [14:12]
// BLK_VERSION_MAJOR: bits [1:0] in RD_MAC_SPI_SYS_4_REG
// BLK_VERSION_MINOR: bits [3:2] in RD_MAC_SPI_SYS_4_REG
// SPI_BOOT_CRYPT_CNT: bits [20:18] in RD_REPEAT_DATA1_REG (odd popcount = enabled)
// SECURE_BOOT_EN: bit [20] in RD_REPEAT_DATA3_REG
// SECURE_VERSION (anti-rollback): bits [29:14] in RD_REPEAT_DATA4_REG

pub fn efuse_hal_chip_revision() -> u32 {
    let val = unsafe { core::ptr::read_volatile(EFUSE_RD_MAC_SPI_SYS_3_REG as *const u32) };
    // Also read the minor rev from the ECO version bits
    // For ESP32-C3: WAFER_VERSION_MINOR[2:0] + WAFER_VERSION_MAJOR[1:0]
    let val0 = unsafe { core::ptr::read_volatile(EFUSE_RD_MAC_SPI_SYS_0_REG as *const u32) };
    let minor = (val0 >> 18) & 0x7; // ECO version
    let major = (val >> 0) & 0x3;   // WAFER_VERSION_MAJOR
    // On ESP32-C3, chip_revision = major*100 + minor (IDF convention)
    major * 100 + minor
}

pub fn efuse_hal_blk_version() -> u32 {
    let val = unsafe { core::ptr::read_volatile(EFUSE_RD_MAC_SPI_SYS_4_REG as *const u32) };
    let major = (val >> 0) & 0x3;
    let minor = (val >> 2) & 0x3;
    major * 100 + minor
}

pub fn efuse_hal_get_disable_blk_version_major() -> bool {
    // On ESP32-C3, this is a field in one of the efuse registers
    // WR_DIS_BLK_VERSION_MAJOR bit
    false // safe default for bootloader
}

pub fn efuse_hal_get_disable_wafer_version_major() -> bool {
    let val = unsafe { core::ptr::read_volatile(EFUSE_RD_REPEAT_DATA4_REG as *const u32) };
    (val >> 19) & 1 != 0
}

/// Returns the 16-bit anti-rollback secure version stored in eFuse BLK0 bits[157:142].
pub fn efuse_get_secure_version() -> u32 {
    unsafe { (core::ptr::read_volatile(EFUSE_RD_REPEAT_DATA4_REG as *const u32) >> 14) & 0xFFFF }
}

pub const CONFIG_IDF_FIRMWARE_CHIP_ID: u32 = 5; // ESP32-C3 chip ID

pub fn ESP_CHIP_REV_ABOVE(revision: u32, compare: u32) -> bool {
    revision >= compare
}

pub fn ESP_EFUSE_BLK_REV_ABOVE(revision: u32, compare: u32) -> bool {
    revision >= compare
}

// ============================================================
// I2C register control (for analog bias config)
// ============================================================
pub const I2C_ULP: u32 = 0;
pub const I2C_BIAS: u32 = 1;
pub const I2C_ULP_IR_FORCE_XPD_IPH: u32 = 1 << 0;
pub const I2C_BIAS_DREG_1P1_PVT: u32 = 0;

// ============================================================
// RTC clock configuration helpers
// ============================================================
#[repr(C)]
pub struct RtcClkConfig {
    pub cpu_freq_mhz: u32,
    pub slow_clk_src: u32,
    pub fast_clk_src: u32,
}
