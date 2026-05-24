//! ESP32-C3 bootloader initialization.
//!
//! Ported from ESP-IDF bootloader (C) to idiomatic no_std Rust.
//! Performs all hardware init for the second-stage bootloader.

use core::ptr::{read_volatile, write_volatile};

// ---------------------------------------------------------------------------
// Register access helpers
// ---------------------------------------------------------------------------

unsafe fn reg_write(addr: u32, val: u32) {
    write_volatile(addr as *mut u32, val);
}

unsafe fn reg_read(addr: u32) -> u32 {
    read_volatile(addr as *const u32)
}

unsafe fn reg_set_bit(addr: u32, mask: u32) {
    reg_write(addr, reg_read(addr) | mask);
}

unsafe fn reg_clr_bit(addr: u32, mask: u32) {
    reg_write(addr, reg_read(addr) & !mask);
}

unsafe fn reg_set_bits(addr: u32, val: u32, mask: u32, shift: u32) {
    let r = reg_read(addr) & !(mask << shift);
    reg_write(addr, r | ((val & mask) << shift));
}

// ---------------------------------------------------------------------------
// Additional register addresses not in soc.rs
// ---------------------------------------------------------------------------

// UART0 (console)
const UART0_BASE: u32 = 0x6000_0000;
const UART_FIFO_REG: u32 = UART0_BASE + 0x00;
const UART_INT_CLR_REG: u32 = UART0_BASE + 0x10;
const UART_CLKDIV_REG: u32 = UART0_BASE + 0x14;
const UART_STATUS_REG: u32 = UART0_BASE + 0x1C;
const UART_CONF0_REG: u32 = UART0_BASE + 0x20;
const UART_CONF1_REG: u32 = UART0_BASE + 0x24;
// UART_CLKDIV register fields (ESP32-C3)
const UART_CLKDIV_FRAG_S: u32 = 12;
const UART_CLKDIV_FRAG_V: u32 = 0x0F;
const UART_CLKDIV_S: u32 = 0;
const UART_CLKDIV_V: u32 = 0x0FFF;
// UART_CONF0 register fields
const UART_BIT_NUM_S: u32 = 0;
const UART_BIT_NUM_V: u32 = 0x03;
const UART_PARITY_EN: u32 = 1 << 2;
const UART_PARITY: u32 = 1 << 3;
const UART_STOP_BIT_NUM_S: u32 = 4;
const UART_STOP_BIT_NUM_V: u32 = 0x03;
const UART_TXFIFO_RST: u32 = 1 << 16;
const UART_RXFIFO_RST: u32 = 1 << 17;
// UART_STATUS register fields
const UART_TXFIFO_CNT_S: u32 = 16;
const UART_TXFIFO_CNT_V: u32 = 0xFF;

const USB_DEVICE_BASE: u32 = 0x6004_3000;
const USB_DEVICE_EP1_REG: u32 = USB_DEVICE_BASE + 0x00;
const USB_DEVICE_EP1_CONF_REG: u32 = USB_DEVICE_BASE + 0x04;
const USB_DEVICE_SERIAL_IN_EP_DATA_FREE: u32 = 1 << 1;
const USB_DEVICE_SERIAL_OUT_EP_DATA_AVAIL: u32 = 1 << 2;
const USB_DEVICE_WR_DONE: u32 = 1 << 0;

// UART RX FIFO count: bits [7:0] of STATUS register
const UART_RXFIFO_CNT_MASK: u32 = 0xFF;

// Timer Group 0 (used for flash-boot watchdog)
const TIMG0_BASE: u32 = 0x6001_F000;
const TIMG_WDTCONFIG0_REG: u32 = TIMG0_BASE + 0x48;
const TIMG_WDTCONFIG1_REG: u32 = TIMG0_BASE + 0x4C;
const TIMG_WDTCONFIG2_REG: u32 = TIMG0_BASE + 0x50;
const TIMG_WDTFEED_REG: u32 = TIMG0_BASE + 0x60;
const TIMG_WDTWPROTECT_REG: u32 = TIMG0_BASE + 0x64;
const TIMG_WDT_KEY: u32 = 0x50D8_3AA1;
const TIMG_WDT_FLASHBOOT_MOD_EN: u32 = 1 << 14;
const TIMG_WDT_EN: u32 = 1 << 31;

// Brown-out detection (same physical register as FIB_SEL_REG in soc.rs,
// but different bit fields)
const RTC_CNTL_BOD_RST_ENA: u32 = 1 << 2;
const RTC_CNTL_BOD_ENA: u32 = 1 << 1;

// I2C master (for analog calibration access)
const RTC_CNTL_I2C_MST_REG: u32 = 0x6000_80BC;
const RTC_CNTL_I2C_MST_CLK_EN: u32 = 1 << 16;
const RTC_CNTL_I2C_MST_BYTE_TRANS: u32 = 1 << 17;
const RTC_CNTL_I2C_CMD_START: u32 = 1 << 15;
const RTC_CNTL_I2C_ADDR_S: u32 = 8;
const RTC_CNTL_I2C_ADDR_V: u32 = 0x7F;
const RTC_CNTL_I2C_DATA_S: u32 = 0;
const RTC_CNTL_I2C_DATA_V: u32 = 0xFF;
const RTC_CNTL_ANA_CONF_I2C_MST_BB: u32 = 1 << 18;

// APB clock is CPU freq / 2 = 40 MHz after rtc_clk_init with 80 MHz CPU
const APB_CLK_HZ: u32 = 40_000_000;

// ---------------------------------------------------------------------------
// Internal helpers
// ---------------------------------------------------------------------------

/// Write a byte to the I2C analog master bus (used for analog calibration).
///
/// Both spin loops have a timeout so this returns cleanly on QEMU (which does
/// not emulate the RTC analog I2C master — CMD_START never auto-clears there).
fn reg_i2c_write(slave_id: u32, reg_addr: u32, value: u32) {
    unsafe {
        // Enable I2C master clock and byte-transfer mode
        reg_set_bit(RTC_CNTL_I2C_MST_REG, RTC_CNTL_I2C_MST_CLK_EN);
        reg_set_bit(RTC_CNTL_I2C_MST_REG, RTC_CNTL_I2C_MST_BYTE_TRANS);

        // Wait for any previous command to finish
        let mut t = 100_000u32;
        while (reg_read(RTC_CNTL_I2C_MST_REG) & RTC_CNTL_I2C_CMD_START) != 0 {
            t = t.wrapping_sub(1);
            if t == 0 { return; }
        }

        // Place combined address (slave_id << 3 | reg_addr) into ADDR field and
        // value into DATA field.
        let addr = (slave_id << 3) | reg_addr;
        reg_set_bits(
            RTC_CNTL_I2C_MST_REG,
            addr,
            RTC_CNTL_I2C_ADDR_V,
            RTC_CNTL_I2C_ADDR_S,
        );
        reg_set_bits(
            RTC_CNTL_I2C_MST_REG,
            value,
            RTC_CNTL_I2C_DATA_V,
            RTC_CNTL_I2C_DATA_S,
        );

        // Trigger the write
        reg_set_bit(RTC_CNTL_I2C_MST_REG, RTC_CNTL_I2C_CMD_START);

        // Wait for completion
        t = 100_000u32;
        while (reg_read(RTC_CNTL_I2C_MST_REG) & RTC_CNTL_I2C_CMD_START) != 0 {
            t = t.wrapping_sub(1);
            if t == 0 { return; }
        }
    }
}

/// Read chip major revision from eFuse.
fn chip_revision() -> u32 {
    unsafe {
        // EFUSE_RD_MAC_SPI_SYS_3_REG at 0x6000_8980
        // ESP32-C3 stores the wafer version in bits [27:24].
        let ver = reg_read(crate::soc::EFUSE_RD_CHIP_VER_REG);
        (ver >> 24) & 0x0F
    }
}

/// Read one byte from UART0 RX FIFO, or `None` if empty.
fn uart_rx_byte() -> Option<u8> {
    unsafe {
        if reg_read(UART_STATUS_REG) & UART_RXFIFO_CNT_MASK > 0 {
            Some((reg_read(UART_FIFO_REG) & 0xFF) as u8)
        } else {
            None
        }
    }
}

/// Read one byte from the USB-Serial/JTAG OUT endpoint, or `None` if empty.
fn usb_serial_jtag_rx_byte() -> Option<u8> {
    unsafe {
        if reg_read(USB_DEVICE_EP1_CONF_REG) & USB_DEVICE_SERIAL_OUT_EP_DATA_AVAIL != 0 {
            Some((reg_read(USB_DEVICE_EP1_REG) & 0xFF) as u8)
        } else {
            None
        }
    }
}

/// Read one byte from either UART0 or USB-Serial/JTAG (UART checked first).
fn console_rx_byte() -> Option<u8> {
    uart_rx_byte().or_else(usb_serial_jtag_rx_byte)
}

/// Transmit a single byte on UART0 (blocking, polling).
fn uart_tx_byte(c: u8) {
    unsafe {
        // Wait until TX FIFO count < 128 (FIFO depth is 128 on ESP32-C3)
        while ((reg_read(UART_STATUS_REG) >> UART_TXFIFO_CNT_S) & UART_TXFIFO_CNT_V) >= 128
        {}
        reg_write(UART_FIFO_REG, c as u32);
    }
}

/// Transmit a single byte on the USB-Serial/JTAG CDC console.
fn usb_serial_jtag_tx_byte(c: u8) {
    unsafe {
        let mut timeout = 100_000u32;
        while reg_read(USB_DEVICE_EP1_CONF_REG) & USB_DEVICE_SERIAL_IN_EP_DATA_FREE == 0 {
            timeout = timeout.wrapping_sub(1);
            if timeout == 0 {
                return;
            }
        }
        reg_write(USB_DEVICE_EP1_REG, c as u32);
        reg_write(USB_DEVICE_EP1_CONF_REG, USB_DEVICE_WR_DONE);
    }
}

/// Transmit a null-terminated byte string on UART0.
fn uart_tx_str(s: &[u8]) {
    for &b in s {
        if b == 0 {
            break;
        }
        if b == b'\n' {
            uart_tx_byte(b'\r');
        }
        uart_tx_byte(b);
    }
}

pub fn debug_tx_byte(b: u8) {
    usb_serial_jtag_tx_byte(b);
    uart_tx_byte(b);
}

pub fn debug_tx_str(s: &[u8]) {
    for &b in s {
        if b == 0 {
            break;
        }
        usb_serial_jtag_tx_byte(b);
    }
    uart_tx_str(s);
}

// ---------------------------------------------------------------------------
// Public API
// ---------------------------------------------------------------------------

/// Main bootloader initialisation.
///
/// Returns `true` on success.  This follows the exact flow of the ESP-IDF
/// `bootloader_init()` for ESP32-C3.
pub fn bootloader_init() -> bool {
    // 1. Chip-specific hardware init (I2C analog bias for old revisions)
    bootloader_hardware_init();

    // 2. Configure analog reset (super WDT, BOD, glitch)
    bootloader_ana_reset_config();

    // 3. Enable super-WDT auto-feed so it does not reset us during flash erase
    bootloader_super_wdt_auto_feed();

    // 4. Validate memory layout
    if !bootloader_init_mem() {
        return false;
    }

    // 5. .bss was already cleared by _start before any Rust stack locals.

    // 6. Switch CPU to 80 MHz
    bootloader_clock_configure();

    // 7. Initialise UART console at 115200 baud
    bootloader_console_init();

    // 8. Print boot banner
    bootloader_print_banner();

    // 9. Initialize SPI flash subsystem.
    crate::flash::bootloader_init_spi_flash();

    // 10. Check whether the previous reset was caused by a watchdog
    bootloader_check_wdt_reset();

    // 11. Disable flash-boot WDT, optionally enable RWDT
    bootloader_config_wdt();

    // 12. Enable RNG entropy source
    bootloader_enable_random();

    true
}

/// Configure CPU clock to 80 MHz via the RTC clock controller.
pub fn bootloader_clock_configure() {
    unsafe {
        crate::rom::esp_rom_output_tx_wait_idle(0);
    }

    let cpu_freq_mhz = crate::rom::CPU_CLK_FREQ_MHZ_BTLD;
    let reset_reason = unsafe { crate::rom::esp_rom_get_reset_reason(0) };

    // Only re-init the clock if the reset was NOT a software reset, or if the
    // APB frequency is too low.
    if reset_reason != crate::rom::RESET_REASON_CPU0_SW
        || unsafe { crate::rom::rtc_clk_apb_freq_get() } < crate::rom::APB_CLK_FREQ
    {
        let mut clk_cfg = crate::rom::RtcClkConfig::default_config();
        clk_cfg.cpu_freq_mhz = cpu_freq_mhz;

        clk_cfg.slow_clk_src = unsafe { crate::rom::rtc_clk_slow_src_get() };
        if clk_cfg.slow_clk_src == crate::rom::SOC_RTC_SLOW_CLK_SRC_INVALID {
            clk_cfg.slow_clk_src = crate::rom::SOC_RTC_SLOW_CLK_SRC_RC_SLOW;
        }

        clk_cfg.fast_clk_src = unsafe { crate::rom::rtc_clk_fast_src_get() };
        if clk_cfg.fast_clk_src == crate::rom::SOC_RTC_FAST_CLK_SRC_INVALID {
            clk_cfg.fast_clk_src = crate::rom::SOC_RTC_FAST_CLK_SRC_DEFAULT;
        }

        unsafe {
            crate::rom::rtc_clk_init(clk_cfg);
        }
    }
}

/// Initialise UART0 console at 115200 baud, 8N1.
pub fn bootloader_console_init() {
    unsafe {
        // Enable UART0 peripheral clock
        reg_set_bit(
            crate::soc::SYSTEM_CPU_PERI_CLK_EN_REG,
            1 << 2, // SYSTEM_UART0_CLK_EN
        );
        // Release UART0 from reset
        reg_clr_bit(
            crate::soc::SYSTEM_CPU_PERI_RST_EN_REG,
            1 << 2, // SYSTEM_UART0_RST_EN
        );
    }

    // Compute baud-rate divider for 115200 with 40 MHz APB clock:
    //   full  = floor((APB_CLK << 4) / 115200)
    //         = floor(640_000_000 / 115200)
    //         = 5555
    //   int   = full >> 4  = 347
    //   frag  = full & 0xF =    3
    const CLKDIV_FULL: u64 = ((APB_CLK_HZ as u64) << 4) / 115200u64;
    const CLKDIV_INT: u32 = (CLKDIV_FULL >> 4) as u32;
    const CLKDIV_FRAG: u32 = (CLKDIV_FULL & 0xF) as u32;

    unsafe {
        // Set baud rate
        reg_write(
            UART_CLKDIV_REG,
            (CLKDIV_INT & UART_CLKDIV_V) << UART_CLKDIV_S
                | (CLKDIV_FRAG & UART_CLKDIV_FRAG_V) << UART_CLKDIV_FRAG_S,
        );

        // Configure 8N1: 8 data bits, no parity, 1 stop bit
        reg_write(
            UART_CONF0_REG,
            (3 << UART_BIT_NUM_S) | (1 << UART_STOP_BIT_NUM_S),
        );

        // Reset TX / RX FIFOs
        reg_set_bit(UART_CONF0_REG, UART_TXFIFO_RST | UART_RXFIFO_RST);
        reg_clr_bit(UART_CONF0_REG, UART_TXFIFO_RST | UART_RXFIFO_RST);

        // Clear any pending interrupts
        reg_write(UART_INT_CLR_REG, 0xFFFF_FFFF);
    }
}

/// Print the bootloader banner on UART0 and USB-Serial/JTAG.
pub fn bootloader_print_banner() {
    debug_tx_str(b"\r\n");
    debug_tx_str(b"ESP-IDF v5.5.1 2nd stage bootloader (Rust)\r\n");
    debug_tx_str(b"SPI Speed      : 80MHz\r\n");
    debug_tx_str(b"SPI Mode       : QIO\r\n");
    debug_tx_str(b"SPI Flash Size : 2MB\r\n");
    debug_tx_str(b"\r\n");
}

/// Enable the RNG entropy source.
///
/// On ESP32-C3 the hardware RNG is always powered; we just need to ensure its
/// clock gate is open.
pub fn bootloader_enable_random() {
    unsafe {
        // SYSTEM_RNG_CLK_EN = bit 6 of SYSTEM_CPU_PERI_CLK_EN_REG
        reg_set_bit(crate::soc::SYSTEM_CPU_PERI_CLK_EN_REG, 1 << 6);
    }
}

/// Configure watchdogs for the bootloader stage.
///
/// Disables the Timer-Group-0 flash-boot WDT protection.  Optionally the
/// RTC WDT (RWDT / super-WDT) could be enabled here with a chosen timeout;
/// the default bootloader keeps RWDT disabled at this point.
pub fn bootloader_config_wdt() {
    unsafe {
        // Unlock TIMG0 WDT protection
        reg_write(TIMG_WDTWPROTECT_REG, TIMG_WDT_KEY);
        // Disable the flash-boot WDT and the MWDT itself before jumping to app.
        reg_clr_bit(TIMG_WDTCONFIG0_REG, TIMG_WDT_FLASHBOOT_MOD_EN | TIMG_WDT_EN);
        reg_write(TIMG_WDTFEED_REG, 1);
        // Re-lock
        reg_write(TIMG_WDTWPROTECT_REG, 0);
    }
}

/// Enable or disable the super-WDT (RTC WDT) reset output.
pub fn bootloader_ana_super_wdt_reset_config(enable: bool) {
    unsafe {
        if enable {
            reg_set_bit(
                crate::soc::RTC_CNTL_FIB_SEL_REG,
                crate::soc::RTC_CNTL_FIB_SUPER_WDT_RST,
            );
        } else {
            reg_clr_bit(
                crate::soc::RTC_CNTL_FIB_SEL_REG,
                crate::soc::RTC_CNTL_FIB_SUPER_WDT_RST,
            );
        }
    }
}

/// Enable or disable the clock-glitch-detection reset.
pub fn bootloader_ana_clock_glitch_reset_config(enable: bool) {
    unsafe {
        if enable {
            reg_set_bit(
                crate::soc::RTC_CNTL_FIB_SEL_REG,
                crate::soc::RTC_CNTL_FIB_GLITCH_RST,
            );
            reg_set_bit(
                crate::soc::RTC_CNTL_ANA_CONF_REG,
                crate::soc::RTC_CNTL_GLITCH_RST_EN,
            );
        } else {
            reg_clr_bit(
                crate::soc::RTC_CNTL_FIB_SEL_REG,
                crate::soc::RTC_CNTL_FIB_GLITCH_RST,
            );
            reg_clr_bit(
                crate::soc::RTC_CNTL_ANA_CONF_REG,
                crate::soc::RTC_CNTL_GLITCH_RST_EN,
            );
        }
    }
}

// ---------------------------------------------------------------------------
// Internal (private) functions
// ---------------------------------------------------------------------------

/// ESP32-C3 chip-specific hardware initialisation.
///
/// Checks the chip revision and, for revisions older than 3, programs the
/// internal I2C analog bias to work around a hardware calibration issue.
fn bootloader_hardware_init() {
    let rev = chip_revision();

    if rev < 3 {
        // Enable I2C master bypass in the analog config register
        unsafe {
            reg_set_bit(
                crate::soc::RTC_CNTL_ANA_CONF_REG,
                RTC_CNTL_ANA_CONF_I2C_MST_BB,
            );
        }

        // Configure I2C_BIAS slave: set register DREG_1P1_PVT = 0
        // to I2C_ULP_IR_FORCE_XPD_IPH = 1.
        reg_i2c_write(1, 0, 1);

        // Disable I2C master bypass
        unsafe {
            reg_clr_bit(
                crate::soc::RTC_CNTL_ANA_CONF_REG,
                RTC_CNTL_ANA_CONF_I2C_MST_BB,
            );
        }
    }
}

/// Configure analog reset options (super WDT, BOD, glitch) based on chip
/// revision.
fn bootloader_ana_reset_config() {
    let rev = chip_revision();

    // Super-WDT reset is always enabled
    bootloader_ana_super_wdt_reset_config(true);
    // Clock-glitch reset is always disabled (it is too sensitive on C3)
    bootloader_ana_clock_glitch_reset_config(false);

    // BOD (brown-out detection) varies with chip version
    if rev >= 3 {
        // Enable brown-out reset for rev >= 3
        unsafe {
            reg_set_bit(crate::soc::RTC_CNTL_FIB_SEL_REG, RTC_CNTL_BOD_RST_ENA);
            reg_set_bit(crate::soc::RTC_CNTL_FIB_SEL_REG, RTC_CNTL_BOD_ENA);
        }
    } else {
        // Disable brown-out reset for older revs (unreliable on those dies)
        unsafe {
            reg_clr_bit(crate::soc::RTC_CNTL_FIB_SEL_REG, RTC_CNTL_BOD_RST_ENA);
            reg_clr_bit(crate::soc::RTC_CNTL_FIB_SEL_REG, RTC_CNTL_BOD_ENA);
        }
    }
}

/// Enable super-WDT auto-feed so the bootloader (which is relatively slow)
/// does not get reset while erasing / writing flash.
fn bootloader_super_wdt_auto_feed() {
    unsafe {
        // Unlock SWD protection
        reg_write(
            crate::soc::RTC_CNTL_SWD_WPROTECT_REG,
            crate::soc::RTC_CNTL_SWD_WKEY_VALUE,
        );
        // Enable auto-feed and bypass the reset
        reg_set_bit(
            crate::soc::RTC_CNTL_SWD_CONF_REG,
            crate::soc::RTC_CNTL_SWD_AUTO_FEED_EN,
        );
        reg_set_bit(
            crate::soc::RTC_CNTL_SWD_CONF_REG,
            crate::soc::RTC_CNTL_SWD_BYPASS_RST,
        );
        // Re-lock
        reg_write(crate::soc::RTC_CNTL_SWD_WPROTECT_REG, 0);
    }
}

/// Validate memory region layout from linker symbols.
///
/// Returns `true` if _bss_start <= _bss_end and _data_start <= _data_end.
fn bootloader_init_mem() -> bool {
    extern "C" {
        static _bss_start: u32;
        static _bss_end: u32;
        static _data_start: u32;
        static _data_end: u32;
    }

    unsafe {
        let bss_start = &_bss_start as *const u32 as u32;
        let bss_end = &_bss_end as *const u32 as u32;
        let data_start = &_data_start as *const u32 as u32;
        let data_end = &_data_end as *const u32 as u32;

        if bss_start > bss_end {
            return false;
        }
        if data_start > data_end {
            return false;
        }
    }
    true
}

/// Clear the .bss section (write zeros).
fn bootloader_clear_bss_section() {
    extern "C" {
        static _bss_start: u32;
        static _bss_end: u32;
    }

    unsafe {
        let start = &_bss_start as *const u32 as *mut u8;
        let end = &_bss_end as *const u32 as *mut u8;
        let len = (end as usize).wrapping_sub(start as usize);
        if len > 0 {
            core::ptr::write_bytes(start, 0u8, len);
        }
    }
}

/// Check whether the previous reset was caused by a watchdog timer.
///
/// In a real bootloader this would log a warning; here we just read the reason
/// and clear the WDT interrupt flags if the reason was WDT-related.
fn bootloader_check_wdt_reset() {
    let reason = unsafe { crate::rom::esp_rom_get_reset_reason(0) };

    let is_wdt = reason == crate::rom::RESET_REASON_CPU0_MWDT0
        || reason == crate::rom::RESET_REASON_CPU0_MWDT1
        || reason == crate::rom::RESET_REASON_CPU0_RTC_WDT
        || reason == crate::rom::RESET_REASON_CORE_MWDT0
        || reason == crate::rom::RESET_REASON_CORE_MWDT1
        || reason == crate::rom::RESET_REASON_CORE_RTC_WDT;

    if is_wdt {
        // Clear any pending RTC WDT interrupt flags so the bootloader can
        // proceed normally.
        unsafe {
            reg_write(crate::soc::RTC_CNTL_INT_CLR_REG, 0xFFFF_FFFF);
        }
    }
}

/// Enable CPU0 watchdog reset recording in the Assist Debug block.
///
/// This must be called early so that a subsequent WDT reset leaves
/// breadcrumbs for the next boot stage.
fn wdt_reset_cpu0_info_enable() {
    unsafe {
        reg_set_bit(
            crate::soc::ASSIST_DEBUG_CORE_0_RCD_EN_REG,
            crate::soc::ASSIST_DEBUG_CORE_0_RCD_PDEBUGEN
                | crate::soc::ASSIST_DEBUG_CORE_0_RCD_RECORDEN,
        );
    }
}

// ---------------------------------------------------------------------------
// Interactive serial shell
// ---------------------------------------------------------------------------

pub enum ShellAction {
    Boot,
    Reset,
}

/// Run a minimal interactive shell on UART0 / USB-Serial-JTAG.
///
/// Prints a 5-second auto-boot countdown; if no key arrives the function
/// returns `ShellAction::Boot` immediately.  Recognised commands:
///   help   – print command list
///   boot   – proceed with normal boot
///   reboot – software-reset the chip
pub fn run_shell() -> ShellAction {
    debug_tx_str(b"Press any key for shell, or auto-booting in 5s...\r\n");

    // 5-second window: 50 × 100 ms
    let mut interactive = false;
    for _ in 0..50u32 {
        unsafe { crate::rom::esp_rom_delay_us(100_000) };
        if console_rx_byte().is_some() {
            interactive = true;
            break;
        }
    }

    if !interactive {
        debug_tx_str(b"Auto-booting.\r\n");
        return ShellAction::Boot;
    }

    debug_tx_str(b"\r\nShell active. Commands: help, boot, reboot\r\n> ");

    let mut buf = [0u8; 64];
    let mut len: usize = 0;

    loop {
        // Spin until a byte arrives on either interface.
        let b = loop {
            if let Some(b) = console_rx_byte() {
                break b;
            }
        };

        match b {
            b'\r' | b'\n' => {
                debug_tx_str(b"\r\n");
                let cmd = &buf[..len];
                len = 0;

                if cmd == b"help" {
                    debug_tx_str(b"  help   - show this list\r\n");
                    debug_tx_str(b"  boot   - proceed with normal boot\r\n");
                    debug_tx_str(b"  reboot - reset the chip\r\n");
                } else if cmd == b"boot" {
                    return ShellAction::Boot;
                } else if cmd == b"reboot" || cmd == b"reset" {
                    return ShellAction::Reset;
                } else if !cmd.is_empty() {
                    debug_tx_str(b"Unknown command. Type 'help'.\r\n");
                }

                debug_tx_str(b"> ");
            }
            0x08 | 0x7F => {
                // Backspace / DEL
                if len > 0 {
                    len -= 1;
                    debug_tx_str(b"\x08 \x08");
                }
            }
            b if b >= 0x20 && b < 0x7F => {
                if len < buf.len() - 1 {
                    buf[len] = b;
                    len += 1;
                    debug_tx_byte(b);
                }
            }
            _ => {}
        }
    }
}
