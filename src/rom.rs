//! ESP32-C3 ROM function wrappers.
//! All calls go through raw function pointers cast from known ROM addresses,
//! so no linker symbol resolution is needed for ROM symbols.

#![allow(dead_code)]

// ============================================================
// ROM function addresses (from esp32c3.rom.ld)
// ============================================================
const ROM_ETS_DELAY_US: usize = 0x40000050;
const ROM_UART_TX_WAIT_IDLE: usize = 0x40000084;
const ROM_SOFTWARE_RESET: usize = 0x40000090;
const ROM_RTC_GET_RESET_REASON: usize = 0x40000018;

const ROM_SPIFLASH_WAIT_IDLE: usize = 0x4000010c;
const ROM_SPIFLASH_READ: usize = 0x40000130;
const ROM_SPIFLASH_WRITE: usize = 0x4000012c;
const ROM_SPIFLASH_WRITE_ENCRYPTED: usize = 0x40000110;
const ROM_SPIFLASH_ERASE_SECTOR: usize = 0x40000128;
const ROM_SPIFLASH_ERASE_BLOCK: usize = 0x40000124;
const ROM_SPIFLASH_CONFIG_CLK: usize = 0x40000150;
const ROM_SPIFLASH_CONFIG_READMODE: usize = 0x40000154;
const ROM_SPIFLASH_CONFIG_PARAM: usize = 0x40000134;
const ROM_SPIFLASH_SELECT_QIO_PINS: usize = 0x4000013c;
const ROM_SPIFLASH_UNLOCK: usize = 0x40000140;

const ROM_CACHE_DISABLE_ICACHE: usize = 0x4000051c;
const ROM_CACHE_ENABLE_ICACHE: usize = 0x40000520;
const ROM_CACHE_SUSPEND_ICACHE: usize = 0x40000524;
const ROM_CACHE_RESUME_ICACHE: usize = 0x40000528;
const ROM_CACHE_INVALIDATE_ICACHE_ALL: usize = 0x400004d8;
const ROM_CACHE_MMU_INIT: usize = 0x4000055c;
const ROM_CACHE_DBUS_MMU_SET: usize = 0x40000564;
const ROM_CACHE_IBUS_MMU_SET: usize = 0x40000560;

// ROM data pointers
const ROM_SPIFLASH_LEGACY_DATA: usize = 0x3fcdfff0;

const ROM_ETS_EFUSE_SECURE_BOOT_ENABLED: usize = 0x400006F8;

// ============================================================
// Flash command constants
// ============================================================
pub const CMD_RDID: u8 = 0x9F;
pub const CMD_RDSR: u8 = 0x05;
pub const CMD_RDSR2: u8 = 0x35;
pub const CMD_RDSR3: u8 = 0x15;
pub const CMD_WRSR: u8 = 0x01;
pub const CMD_WRSR2: u8 = 0x31;
pub const CMD_WRSR3: u8 = 0x11;
pub const CMD_WREN: u8 = 0x06;
pub const CMD_WRDI: u8 = 0x04;
pub const CMD_RDSFDP: u8 = 0x5A;
pub const CMD_RESUME: u8 = 0xAB;
pub const CMD_RESETEN: u8 = 0x66;
pub const CMD_RESET: u8 = 0x99;
pub const CMD_OTPEN: u8 = 0x3A;

// ============================================================
// Reset reason constants
// ============================================================
pub const RESET_REASON_CHIP_POWER_ON: u32 = 0x01;
pub const RESET_REASON_CORE_DEEP_SLEEP: u32 = 0x05;
pub const RESET_REASON_CORE_RTC_WDT: u32 = 0x06;
pub const RESET_REASON_CORE_MWDT0: u32 = 0x08;
pub const RESET_REASON_CORE_MWDT1: u32 = 0x09;
pub const RESET_REASON_CPU0_MWDT0: u32 = 0x0B;
pub const RESET_REASON_CPU0_MWDT1: u32 = 0x0C;
pub const RESET_REASON_CPU0_RTC_WDT: u32 = 0x0D;
pub const RESET_REASON_CPU0_SW: u32 = 0x0C;
pub const RESET_REASON_CORE_EFUSE_CRC: u32 = 0x0E;

// ============================================================
// RTC / Clock constants
// ============================================================
pub const CPU_CLK_FREQ_MHZ_BTLD: u32 = 80;
pub const APB_CLK_FREQ: u32 = 80_000_000;
pub const SOC_RTC_SLOW_CLK_SRC_INVALID: u32 = 0;
pub const SOC_RTC_SLOW_CLK_SRC_RC_SLOW: u32 = 1;
pub const SOC_RTC_FAST_CLK_SRC_INVALID: u32 = 0;
pub const SOC_RTC_FAST_CLK_SRC_DEFAULT: u32 = 1;

// ============================================================
// Public wrappers
// ============================================================

#[inline]
pub unsafe fn ets_efuse_secure_boot_enabled() -> bool {
    let f: extern "C" fn() -> bool = core::mem::transmute(ROM_ETS_EFUSE_SECURE_BOOT_ENABLED);
    f()
}

#[inline]
pub unsafe fn esp_rom_delay_us(us: u32) {
    let f: extern "C" fn(u32) = core::mem::transmute(ROM_ETS_DELAY_US);
    f(us);
}

#[inline]
pub unsafe fn esp_rom_output_tx_wait_idle(channel: u8) {
    let f: extern "C" fn(u8) = core::mem::transmute(ROM_UART_TX_WAIT_IDLE);
    f(channel);
}

#[inline]
pub unsafe fn esp_rom_software_reset_system() -> ! {
    let f: extern "C" fn() -> ! = core::mem::transmute(ROM_SOFTWARE_RESET);
    f()
}

#[inline]
pub unsafe fn esp_rom_get_reset_reason(cpu: u32) -> u32 {
    let f: extern "C" fn(u32) -> u32 = core::mem::transmute(ROM_RTC_GET_RESET_REASON);
    f(cpu)
}

// --- SPI Flash ----------------------------------------------------------------

#[repr(C)]
pub struct RomSpiFlashChip {
    pub device_id: u32,
    pub chip_size: u32,
    pub block_size: u32,
    pub sector_size: u32,
    pub page_size: u32,
    pub status_mask: u32,
}

#[derive(Clone, Copy, PartialEq, Eq)]
#[repr(u32)]
pub enum SpiFlashReadMode {
    Qio = 0,
    Qout = 1,
    Dio = 2,
    Dout = 3,
    FastRd = 4,
    SlowRd = 5,
}

#[derive(Clone, Copy, PartialEq, Eq)]
#[repr(i32)]
pub enum SpiFlashResult {
    Ok = 0,
    Err = 1,
    Timeout = 2,
}

pub unsafe fn g_rom_flashchip() -> *const RomSpiFlashChip {
    let legacy_data: *const usize = core::mem::transmute(ROM_SPIFLASH_LEGACY_DATA as *const u32);
    let ptr_addr: usize = core::ptr::read_volatile(legacy_data);
    ptr_addr as *const RomSpiFlashChip
}

pub unsafe fn g_rom_flashchip_mut() -> *mut RomSpiFlashChip {
    g_rom_flashchip() as *mut RomSpiFlashChip
}

pub unsafe fn g_rom_spiflash_dummy_len_plus() -> *const u32 {
    let legacy_data: *const usize = core::mem::transmute(ROM_SPIFLASH_LEGACY_DATA as *const u32);
    let ptr_addr: usize = core::ptr::read_volatile(legacy_data);
    // dummy_len_plus is at offset 28 in the legacy data struct (after chip: 24 bytes)
    (ptr_addr + 28) as *const u32
}

pub unsafe fn esp_rom_spiflash_wait_idle(chip: *const RomSpiFlashChip) {
    let f: extern "C" fn(*const RomSpiFlashChip) = core::mem::transmute(ROM_SPIFLASH_WAIT_IDLE);
    f(chip);
}

pub unsafe fn esp_rom_spiflash_read(src_addr: u32, dest: *mut u8, size: u32) -> SpiFlashResult {
    let f: extern "C" fn(u32, *mut u8, u32) -> SpiFlashResult = core::mem::transmute(ROM_SPIFLASH_READ);
    f(src_addr, dest, size)
}

pub unsafe fn esp_rom_spiflash_write(dest_addr: u32, src: *const u8, size: u32) -> SpiFlashResult {
    let f: extern "C" fn(u32, *const u8, u32) -> SpiFlashResult = core::mem::transmute(ROM_SPIFLASH_WRITE);
    f(dest_addr, src, size)
}

pub unsafe fn esp_rom_spiflash_write_encrypted(dest_addr: u32, src: *const u8, size: u32) -> SpiFlashResult {
    let f: extern "C" fn(u32, *const u8, u32) -> SpiFlashResult = core::mem::transmute(ROM_SPIFLASH_WRITE_ENCRYPTED);
    f(dest_addr, src, size)
}

pub unsafe fn esp_rom_spiflash_erase_sector(sector: u32) -> SpiFlashResult {
    let f: extern "C" fn(u32) -> SpiFlashResult = core::mem::transmute(ROM_SPIFLASH_ERASE_SECTOR);
    f(sector)
}

pub unsafe fn esp_rom_spiflash_erase_block(block: u32) -> SpiFlashResult {
    let f: extern "C" fn(u32) -> SpiFlashResult = core::mem::transmute(ROM_SPIFLASH_ERASE_BLOCK);
    f(block)
}

pub unsafe fn esp_rom_spiflash_config_clk(freqdiv: u32, spi: u32) {
    let f: extern "C" fn(u32, u32) = core::mem::transmute(ROM_SPIFLASH_CONFIG_CLK);
    f(freqdiv, spi);
}

pub unsafe fn esp_rom_spiflash_config_readmode(mode: SpiFlashReadMode) {
    let f: extern "C" fn(SpiFlashReadMode) = core::mem::transmute(ROM_SPIFLASH_CONFIG_READMODE);
    f(mode);
}

pub unsafe fn esp_rom_spiflash_config_param(device_id: u32, chip_size: u32, block_size: u32, sector_size: u32, page_size: u32, status_mask: u32) {
    let f: extern "C" fn(u32, u32, u32, u32, u32, u32) = core::mem::transmute(ROM_SPIFLASH_CONFIG_PARAM);
    f(device_id, chip_size, block_size, sector_size, page_size, status_mask);
}

pub unsafe fn esp_rom_spiflash_select_qio_pins(wp_gpio: u8, spiconfig: u32) {
    let f: extern "C" fn(u8, u32) = core::mem::transmute(ROM_SPIFLASH_SELECT_QIO_PINS);
    f(wp_gpio, spiconfig);
}

pub unsafe fn esp_rom_spiflash_unlock() -> SpiFlashResult {
    let f: extern "C" fn() -> SpiFlashResult = core::mem::transmute(ROM_SPIFLASH_UNLOCK);
    f()
}

// --- Cache / MMU (ESP32-C3 has unified cache, no separate I/D) ------------------

pub unsafe fn Cache_Disable_ICache() {
    let f: extern "C" fn() = core::mem::transmute(ROM_CACHE_DISABLE_ICACHE);
    f();
}

pub unsafe fn Cache_Enable_ICache(autoload: u32) {
    let f: extern "C" fn(u32) = core::mem::transmute(ROM_CACHE_ENABLE_ICACHE);
    f(autoload);
}

// Returns the autoload state that must be passed to Cache_Resume_ICache.
pub unsafe fn Cache_Suspend_ICache() -> u32 {
    let f: extern "C" fn() -> u32 = core::mem::transmute(ROM_CACHE_SUSPEND_ICACHE);
    f()
}

pub unsafe fn Cache_Resume_ICache(autoload: u32) {
    let f: extern "C" fn(u32) = core::mem::transmute(ROM_CACHE_RESUME_ICACHE);
    f(autoload);
}

pub unsafe fn Cache_Invalidate_ICache_All() {
    let f: extern "C" fn() = core::mem::transmute(ROM_CACHE_INVALIDATE_ICACHE_ALL);
    f();
}

pub unsafe fn Cache_MMU_Init() {
    let f: extern "C" fn() = core::mem::transmute(ROM_CACHE_MMU_INIT);
    f();
}

/// Map flash pages into the data bus (DROM).
/// psize must be 64 (ROM's token for 64 KB pages); num is the page count.
pub unsafe fn Cache_Dbus_MMU_Set(ext_ram: u32, vaddr: u32, paddr: u32, psize: u32, num: i32, trigger: u32) -> i32 {
    let f: extern "C" fn(u32, u32, u32, u32, i32, u32) -> i32 = core::mem::transmute(ROM_CACHE_DBUS_MMU_SET);
    f(ext_ram, vaddr, paddr, psize, num, trigger)
}

/// Map flash pages into the instruction bus (IROM).
/// psize must be 64 (ROM's token for 64 KB pages); num is the page count.
pub unsafe fn Cache_Ibus_MMU_Set(ext_ram: u32, vaddr: u32, paddr: u32, psize: u32, num: i32, trigger: u32) -> i32 {
    let f: extern "C" fn(u32, u32, u32, u32, i32, u32) -> i32 = core::mem::transmute(ROM_CACHE_IBUS_MMU_SET);
    f(ext_ram, vaddr, paddr, psize, num, trigger)
}

// --- GPIO / EFUSE (not in ROM - inline implementations) -------------------------

pub unsafe fn esp_rom_gpio_pad_select_gpio(pin: u32) {
    // On ESP32-C3, this sets the pin function to GPIO
    // IO_MUX register for each pin
    if pin > 21 { return; }
    let iomux_reg = 0x60009000u32 + pin * 4;
    let val = core::ptr::read_volatile(iomux_reg as *const u32);
    // MCU_SEL = 0 for GPIO function
    core::ptr::write_volatile(iomux_reg as *mut u32, val & !0x1);
}

pub unsafe fn esp_rom_gpio_pad_pullup_only(pin: u32) {
    if pin > 21 { return; }
    let iomux_reg = 0x60009000u32 + pin * 4;
    let val = core::ptr::read_volatile(iomux_reg as *const u32);
    // Set FUN_WPU (bit 8) and FUN_WPD (bit 7) appropriately for pullup
    core::ptr::write_volatile(iomux_reg as *mut u32, (val & !0x80) | 0x100);
}

// GPIO input level read
pub fn gpio_ll_get_level(pin: u32) -> bool {
    if pin > 21 { return false; }
    const GPIO_IN_REG: u32 = 0x6000403C;
    let val = unsafe { core::ptr::read_volatile(GPIO_IN_REG as *const u32) };
    (val >> pin) & 1 != 0
}

pub unsafe fn gpio_ll_input_enable(pin: u32) {
    if pin > 21 { return; }
    // GPIO_FUNCx_IN_SEL_CFG_REG at 0x60004000 + pin*4 sets input enable (bit 13)
    let reg = 0x60004000u32 + pin * 4;
    let val = core::ptr::read_volatile(reg as *const u32);
    core::ptr::write_volatile(reg as *mut u32, val | (1 << 13));
}

// efuse flash encryption check
pub fn efuse_hal_flash_encryption_enabled() -> bool {
    // SPI_BOOT_CRYPT_CNT: BLK0 bits [84:82] → RD_REPEAT_DATA1[20:18]
    // Odd popcount = encryption enabled (0=off, 1=on, 3=off, 7=permanently on)
    let val = unsafe {
        core::ptr::read_volatile(super::soc::EFUSE_RD_REPEAT_DATA1_REG as *const u32)
    };
    let crypt_cnt = (val >> 18) & 0x7;
    crypt_cnt.count_ones() & 1 != 0
}

// CRC32 LE (matches esp_rom_crc32_le)
pub fn esp_rom_crc32_le(mut crc: u32, data: *const u8, len: usize) -> u32 {
    for i in 0..len {
        let byte = unsafe { *data.add(i) };
        crc ^= byte as u32;
        for _ in 0..8 {
            if crc & 1 != 0 {
                crc = (crc >> 1) ^ 0xEDB88320;
            } else {
                crc >>= 1;
            }
        }
    }
    crc
}

pub unsafe fn esp_rom_efuse_get_flash_gpio_info() -> u32 {
    // Read efuse register for SPI flash GPIO config (EFUSE_RD_MAC_SPI_SYS_3_REG)
    let val: u32 = core::ptr::read_volatile(0x60008800u32 as *const u32);
    // Bits [25:20] are FLASH_GPIO_INFO on ESP32-C3
    (val >> 20) & 0x3F
}

pub unsafe fn esp_rom_efuse_get_flash_wp_gpio() -> u8 {
    // Bits [19:14] are FLASH_WP_GPIO
    let val: u32 = core::ptr::read_volatile(0x60008800u32 as *const u32);
    ((val >> 14) & 0x3F) as u8
}

pub unsafe fn esp_rom_gpio_pad_set_drv(gpio_num: u8, drv: u32) {
    // GPIO pad driver strength register
    let base = if gpio_num > 21 { return } else { 0x60004000u32 };
    let reg = base + 0x08 + (gpio_num as u32 * 4);
    let val = core::ptr::read_volatile(reg as *const u32);
    // DRV is bits [1:0] in the pad register
    let new_val = (val & !0x3) | (drv & 0x3);
    core::ptr::write_volatile(reg as *mut u32, new_val);
}

// ============================================================
// RTC / Clock functions
// These are NOT in ROM; they're in the esp_hw_support component.
// We implement minimal versions here.
// ============================================================

pub unsafe fn rtc_clk_apb_freq_get() -> u32 {
    // APB clock is 40MHz on ESP32-C3 unless PLL is used
    // After boot, ROM sets APB to 40MHz
    40_000_000
}

pub unsafe fn rtc_clk_slow_src_get() -> u32 {
    SOC_RTC_SLOW_CLK_SRC_RC_SLOW
}

pub unsafe fn rtc_clk_fast_src_get() -> u32 {
    SOC_RTC_FAST_CLK_SRC_DEFAULT
}

pub unsafe fn rtc_clk_slow_freq_get_hz() -> u32 {
    150_000 // RC_SLOW is ~150kHz
}

pub unsafe fn rtc_clk_32k_enabled() -> bool {
    false
}

pub unsafe fn rtc_clk_32k_bootstrap(_cycles: u32) {}

// RTC clock config struct
#[repr(C)]
#[derive(Clone, Copy)]
pub struct RtcClkConfig {
    pub cpu_freq_mhz: u32,
    pub slow_clk_src: u32,
    pub fast_clk_src: u32,
}

impl RtcClkConfig {
    pub fn default_config() -> Self {
        RtcClkConfig {
            cpu_freq_mhz: CPU_CLK_FREQ_MHZ_BTLD,
            slow_clk_src: SOC_RTC_SLOW_CLK_SRC_RC_SLOW,
            fast_clk_src: SOC_RTC_FAST_CLK_SRC_DEFAULT,
        }
    }
}

pub unsafe fn rtc_clk_init(cfg: RtcClkConfig) {
    // On ESP32-C3, the ROM bootloader already configured the CPU clock.
    // For the 2nd stage bootloader, we mainly need to ensure the clock
    // is at the desired frequency. The ROM already set it to 80MHz PLL.
    // We trust the ROM configuration and only reconfigure if needed.
    let _ = cfg;
}
