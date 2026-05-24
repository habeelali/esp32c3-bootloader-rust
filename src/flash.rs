//! ESP32-C3 bootloader flash subsystem
//! Ported from ESP-IDF bootloader_flash.c and bootloader_flash_qio.c
//! SPI flash read/write/erase, QIO mode, vendor-specific operations.

use crate::rom::*;
use crate::soc::*;
use core::ptr;

// ---------------------------------------------------------------------------
// Register helpers
// ---------------------------------------------------------------------------
unsafe fn reg_write(addr: u32, val: u32) {
    ptr::write_volatile(addr as *mut u32, val);
}
unsafe fn reg_read(addr: u32) -> u32 {
    ptr::read_volatile(addr as *const u32)
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
// Register offset constants not exported from soc.rs
// ---------------------------------------------------------------------------
const SPI_MEM_USER1_OFF: u32 = 0x1C;
const SPI_MEM_USER2_OFF: u32 = 0x20;
const SPI_MEM_CLOCK_OFF: u32 = 0x14;
const SPI_MEM_W0_OFF: u32 = 0x80;
const SPI_MEM_MOSI_DLEN_OFF: u32 = 0x24;
const SPI_MEM_MISO_DLEN_OFF: u32 = 0x28;

// ---------------------------------------------------------------------------
// Bit definitions for SPI_MEM_USER_REG (offset 0x18)
// ---------------------------------------------------------------------------
const SPI_MEM_USR_COMMAND: u32 = 1 << 31;
const SPI_MEM_USR_ADDR: u32 = 1 << 30;
const SPI_MEM_USR_DUMMY: u32 = 1 << 29;
const SPI_MEM_USR_MISO: u32 = 1 << 28;
const SPI_MEM_USR_MOSI: u32 = 1 << 27;

// Bit definitions for SPI_MEM_CMD_REG (offset 0x00)
const SPI_MEM_USR: u32 = 1 << 18;

// Bit fields for SPI_MEM_USER1_REG (offset 0x1C)
const SPI_MEM_USR_ADDR_BITLEN_S: u32 = 26;
const SPI_MEM_USR_ADDR_BITLEN_V: u32 = 0x3F;
const SPI_MEM_USR_ADDR_BITLEN_M: u32 =
    SPI_MEM_USR_ADDR_BITLEN_V << SPI_MEM_USR_ADDR_BITLEN_S;
const SPI_MEM_USR_DUMMY_CYCLELEN_S: u32 = 0;
const SPI_MEM_USR_DUMMY_CYCLELEN_V: u32 = 0xFF;
const SPI_MEM_USR_DUMMY_CYCLELEN_M: u32 =
    SPI_MEM_USR_DUMMY_CYCLELEN_V << SPI_MEM_USR_DUMMY_CYCLELEN_S;

// Bit fields for SPI_MEM_USER2_REG (offset 0x20)
const SPI_MEM_USR_COMMAND_BITLEN_S: u32 = 28;
const SPI_MEM_USR_COMMAND_BITLEN_V: u32 = 0xF;
const SPI_MEM_USR_COMMAND_BITLEN_M: u32 =
    SPI_MEM_USR_COMMAND_BITLEN_V << SPI_MEM_USR_COMMAND_BITLEN_S;
const SPI_MEM_USR_COMMAND_VALUE_S: u32 = 0;
const SPI_MEM_USR_COMMAND_VALUE_V: u32 = 0xFFFF;
const SPI_MEM_USR_COMMAND_VALUE_M: u32 =
    SPI_MEM_USR_COMMAND_VALUE_V << SPI_MEM_USR_COMMAND_VALUE_S;

// Bit fields for SPI_MEM_MOSI_DLEN_REG / MISO_DLEN_REG
const SPI_MEM_USR_MOSI_DLEN_S: u32 = 0;
const SPI_MEM_USR_MOSI_DLEN_V: u32 = 0xFFFF;
const SPI_MEM_USR_MISO_DLEN_S: u32 = 0;
const SPI_MEM_USR_MISO_DLEN_V: u32 = 0xFFFF;

// Bit fields for SPI_MEM_CLOCK_REG (offset 0x14)
const SPI_MEM_CLK_EQU_SYSCLK: u32 = 1 << 31;
const SPI_MEM_CLKCNT_N_S: u32 = 18;
const SPI_MEM_CLKCNT_N_V: u32 = 0x3F;
const SPI_MEM_CLKCNT_N_M: u32 = SPI_MEM_CLKCNT_N_V << SPI_MEM_CLKCNT_N_S;
const SPI_MEM_CLKCNT_H_S: u32 = 12;
const SPI_MEM_CLKCNT_H_V: u32 = 0x3F;
const SPI_MEM_CLKCNT_H_M: u32 = SPI_MEM_CLKCNT_H_V << SPI_MEM_CLKCNT_H_S;
const SPI_MEM_CLKCNT_L_S: u32 = 6;
const SPI_MEM_CLKCNT_L_V: u32 = 0x3F;
const SPI_MEM_CLKCNT_L_M: u32 = SPI_MEM_CLKCNT_L_V << SPI_MEM_CLKCNT_L_S;

// Extra bits for SPI_MEM_CTRL_REG
const SPI_MEM_SYNC_RESET: u32 = 1 << 27;
const SPI_MEM_WP: u32 = 1 << 14;

// Flash vendor IDs
const FLASH_VENDOR_ID_MXIC: u8 = 0xC2;
const FLASH_VENDOR_ID_ISSI: u8 = 0x9D;
const FLASH_VENDOR_ID_WINBOND: u8 = 0xEF;
const FLASH_VENDOR_ID_GD: u8 = 0xC8;
const FLASH_VENDOR_ID_XMC: u8 = 0x20;
const FLASH_VENDOR_ID_TH: u8 = 0x4B;

// ---------------------------------------------------------------------------
// Helper: compute register addresses for SPI1 / SPI0
// ---------------------------------------------------------------------------
#[inline(always)]
fn spi1_reg(offset: u32) -> u32 {
    SPIMEM1_BASE + offset
}
#[inline(always)]
fn spi0_reg(offset: u32) -> u32 {
    SPIMEM0_BASE + offset
}

// Wait until SPI1 finishes any ongoing user command.
unsafe fn spi1_wait_ready() {
    while reg_read(spi1_reg(0)) & SPI_MEM_USR != 0 {}
}

// Send CMD_RESUME (0xAB) to wake flash from deep power-down.
fn spi_flash_resume() {
    bootloader_execute_flash_command(CMD_RESUME, 0, 0, 0);
    unsafe {
        esp_rom_delay_us(20);
    }
}

// Read 8 bytes of the image header at flash offset 0.
fn read_flash_config_from_header() -> (u8, u8, u8) {
    // ROM function requires 4-byte-aligned destination.
    let mut hdr: [u32; 2] = [0; 2];
    unsafe {
        let autoload = crate::rom::Cache_Suspend_ICache();
        esp_rom_spiflash_read(0, hdr.as_mut_ptr().cast::<u8>(), 8);
        crate::rom::Cache_Resume_ICache(autoload);
    }
    let b = hdr[0].to_le_bytes();
    (b[0], b[2], b[3])
}

// ---------------------------------------------------------------------------
// 1. bootloader_execute_flash_command
// ---------------------------------------------------------------------------
pub fn bootloader_execute_flash_command(
    command: u8,
    mosi_data: u32,
    mosi_len: u8,
    miso_len: u8,
) -> u32 {
    unsafe {
        spi1_wait_ready();

        let old_ctrl = reg_read(SPI_MEM_CTRL_REG(1));
        let old_user = reg_read(SPI_MEM_USER_REG(1));
        let old_user1 = reg_read(spi1_reg(SPI_MEM_USER1_OFF));
        let old_user2 = reg_read(spi1_reg(SPI_MEM_USER2_OFF));
        let old_mosi_dlen = reg_read(spi1_reg(SPI_MEM_MOSI_DLEN_OFF));
        let old_miso_dlen = reg_read(spi1_reg(SPI_MEM_MISO_DLEN_OFF));

        // --- USER2: command value and bit length (always 8 bits) ---
        reg_write(
            spi1_reg(SPI_MEM_USER2_OFF),
            ((command as u32) << SPI_MEM_USR_COMMAND_VALUE_S)
                | ((8 - 1) << SPI_MEM_USR_COMMAND_BITLEN_S),
        );

        // --- USER1: address length = 0, dummy length ---
        let mut user1_val = reg_read(spi1_reg(SPI_MEM_USER1_OFF));
        user1_val &= !(SPI_MEM_USR_ADDR_BITLEN_M | SPI_MEM_USR_DUMMY_CYCLELEN_M);
        let dummy_len = if miso_len > 0 {
            unsafe { *crate::rom::g_rom_spiflash_dummy_len_plus().add(1) }
        } else {
            0
        };
        if dummy_len > 0 {
            user1_val |= ((dummy_len - 1) << SPI_MEM_USR_DUMMY_CYCLELEN_S)
                & SPI_MEM_USR_DUMMY_CYCLELEN_M;
        }
        reg_write(spi1_reg(SPI_MEM_USER1_OFF), user1_val);

        // --- USER: command + optional dummy / mosi / miso ---
        let mut user_val = SPI_MEM_USR_COMMAND;
        if dummy_len > 0 {
            user_val |= SPI_MEM_USR_DUMMY;
        }
        if mosi_len > 0 {
            user_val |= SPI_MEM_USR_MOSI;
            reg_write(
                spi1_reg(SPI_MEM_MOSI_DLEN_OFF),
                ((mosi_len - 1) as u32) << SPI_MEM_USR_MOSI_DLEN_S,
            );
        }
        if miso_len > 0 {
            user_val |= SPI_MEM_USR_MISO;
            reg_write(
                spi1_reg(SPI_MEM_MISO_DLEN_OFF),
                ((miso_len - 1) as u32) << SPI_MEM_USR_MISO_DLEN_S,
            );
        }
        reg_write(SPI_MEM_USER_REG(1), user_val);

        // --- write MOSI data (padded to nearest byte) ---
        if mosi_len > 0 {
            let num_bytes = ((mosi_len + 7) / 8) as usize;
            if num_bytes >= 4 {
                reg_write(spi1_reg(SPI_MEM_W0_OFF), mosi_data);
            } else {
                reg_write(
                    spi1_reg(SPI_MEM_W0_OFF),
                    mosi_data & ((1u32 << (num_bytes * 8)).wrapping_sub(1)),
                );
            }
        }

        // --- trigger ---
        reg_set_bit(SPI_MEM_CMD_REG(1), SPI_MEM_USR);

        // wait for completion
        spi1_wait_ready();

        // --- read MISO data ---
        let out = if miso_len > 0 {
            reg_read(spi1_reg(SPI_MEM_W0_OFF))
        } else {
            0
        };

        // --- restore ---
        reg_write(SPI_MEM_CTRL_REG(1), old_ctrl);
        reg_write(SPI_MEM_USER_REG(1), old_user);
        reg_write(spi1_reg(SPI_MEM_USER1_OFF), old_user1);
        reg_write(spi1_reg(SPI_MEM_USER2_OFF), old_user2);
        reg_write(spi1_reg(SPI_MEM_MOSI_DLEN_OFF), old_mosi_dlen);
        reg_write(spi1_reg(SPI_MEM_MISO_DLEN_OFF), old_miso_dlen);

        out
    }
}

// ---------------------------------------------------------------------------
// 2. bootloader_read_flash_id
// ---------------------------------------------------------------------------
pub fn bootloader_read_flash_id() -> u32 {
    let raw = bootloader_execute_flash_command(CMD_RDID, 0, 0, 24);
    ((raw & 0xff) << 16) | ((raw >> 16) & 0xff) | (raw & 0xff00)
}

// ---------------------------------------------------------------------------
// 3. bootloader_flash_read_sfdp
// ---------------------------------------------------------------------------
pub fn bootloader_flash_read_sfdp(sfdp_addr: u32, miso_byte_num: u32) -> u32 {
    unsafe {
        spi1_wait_ready();

        let old_ctrl = reg_read(SPI_MEM_CTRL_REG(1));
        let old_user = reg_read(SPI_MEM_USER_REG(1));
        let old_user1 = reg_read(spi1_reg(SPI_MEM_USER1_OFF));
        let old_user2 = reg_read(spi1_reg(SPI_MEM_USER2_OFF));

        let miso_bits = miso_byte_num * 8;

        // Command byte
        reg_write(
            spi1_reg(SPI_MEM_USER2_OFF),
            ((CMD_RDSFDP as u32) << SPI_MEM_USR_COMMAND_VALUE_S)
                | ((8 - 1) << SPI_MEM_USR_COMMAND_BITLEN_S),
        );

        // Address (24 bits)
        reg_write(SPI_MEM_ADDR_REG(1), sfdp_addr & 0x00FF_FFFF);

        // Dummy: 8 cycles, address: 24 bits
        let mut user1_val = reg_read(spi1_reg(SPI_MEM_USER1_OFF));
        user1_val &= !(SPI_MEM_USR_ADDR_BITLEN_M | SPI_MEM_USR_DUMMY_CYCLELEN_M);
        user1_val |=
            ((24 - 1) << SPI_MEM_USR_ADDR_BITLEN_S) & SPI_MEM_USR_ADDR_BITLEN_M;
        user1_val |=
            ((8 - 1) << SPI_MEM_USR_DUMMY_CYCLELEN_S) & SPI_MEM_USR_DUMMY_CYCLELEN_M;
        reg_write(spi1_reg(SPI_MEM_USER1_OFF), user1_val);

        // USER: command + addr + dummy + miso
        let mut user_val =
            SPI_MEM_USR_COMMAND | SPI_MEM_USR_ADDR | SPI_MEM_USR_DUMMY;
        user_val |= SPI_MEM_USR_MISO;
        reg_write(
            spi1_reg(SPI_MEM_MISO_DLEN_OFF),
            (miso_bits - 1) << SPI_MEM_USR_MISO_DLEN_S,
        );
        reg_write(SPI_MEM_USER_REG(1), user_val);

        // trigger
        reg_set_bit(SPI_MEM_CMD_REG(1), SPI_MEM_USR);
        spi1_wait_ready();

        let out = reg_read(spi1_reg(SPI_MEM_W0_OFF));

        // restore
        reg_write(SPI_MEM_CTRL_REG(1), old_ctrl);
        reg_write(SPI_MEM_USER_REG(1), old_user);
        reg_write(spi1_reg(SPI_MEM_USER1_OFF), old_user1);
        reg_write(spi1_reg(SPI_MEM_USER2_OFF), old_user2);

        out
    }
}

// ---------------------------------------------------------------------------
// 4. bootloader_enable_wp
// ---------------------------------------------------------------------------
pub fn bootloader_enable_wp() {
    bootloader_execute_flash_command(CMD_WRDI, 0, 0, 0);
}

// ---------------------------------------------------------------------------
// 5. bootloader_spi_flash_reset
// ---------------------------------------------------------------------------
pub fn bootloader_spi_flash_reset() {
    bootloader_execute_flash_command(CMD_RESETEN, 0, 0, 0);
    bootloader_execute_flash_command(CMD_RESET, 0, 0, 0);
}

// ---------------------------------------------------------------------------
// 6. bootloader_flash_get_spi_mode
// ---------------------------------------------------------------------------
pub fn bootloader_flash_get_spi_mode() -> SpiFlashReadMode {
    let ctrl = unsafe { reg_read(SPI_MEM_CTRL_REG(0)) };
    if ctrl & SPI_MEM_FREAD_QIO != 0 {
        SpiFlashReadMode::Qio
    } else if ctrl & SPI_MEM_FREAD_DIO != 0 {
        SpiFlashReadMode::Dio
    } else if ctrl & SPI_MEM_FREAD_QUAD != 0 {
        SpiFlashReadMode::Qout
    } else if ctrl & SPI_MEM_FREAD_DUAL != 0 {
        SpiFlashReadMode::Dout
    } else if ctrl & SPI_MEM_FASTRD_MODE != 0 {
        SpiFlashReadMode::FastRd
    } else {
        SpiFlashReadMode::SlowRd
    }
}

// ---------------------------------------------------------------------------
// 7. bootloader_flash_reset_chip
// ---------------------------------------------------------------------------
pub fn bootloader_flash_reset_chip() -> bool {
    unsafe {
        reg_set_bit(SPI_MEM_CTRL_REG(1), SPI_MEM_SYNC_RESET);
        spi1_wait_ready();
    }
    bootloader_execute_flash_command(CMD_RESETEN, 0, 0, 0);
    bootloader_execute_flash_command(CMD_RESET, 0, 0, 0);
    true
}

// ---------------------------------------------------------------------------
// 8. bootloader_flash_is_octal_mode_enabled
// ---------------------------------------------------------------------------
pub fn bootloader_flash_is_octal_mode_enabled() -> bool {
    false
}

// ---------------------------------------------------------------------------
// 10. bootloader_flash_write
// ---------------------------------------------------------------------------
pub unsafe fn bootloader_flash_write(
    dest_addr: u32,
    src: *const u8,
    size: u32,
    write_encrypted: bool,
) -> bool {
    bootloader_flash_unlock();
    let res = if write_encrypted {
        esp_rom_spiflash_write_encrypted(dest_addr, src, size)
    } else {
        esp_rom_spiflash_write(dest_addr, src, size)
    };
    res == SpiFlashResult::Ok
}

// ---------------------------------------------------------------------------
// 11. bootloader_flash_erase_sector
// ---------------------------------------------------------------------------
pub fn bootloader_flash_erase_sector(sector: u32) -> bool {
    unsafe { esp_rom_spiflash_erase_sector(sector) == SpiFlashResult::Ok }
}

// ---------------------------------------------------------------------------
// 12. bootloader_flash_erase_range
// ---------------------------------------------------------------------------
pub fn bootloader_flash_erase_range(start_addr: u32, size: u32) -> bool {
    let start_sector = start_addr / FLASH_SECTOR_SIZE;
    let num_sectors = (size + FLASH_SECTOR_SIZE - 1) / FLASH_SECTOR_SIZE;
    for i in 0..num_sectors {
        if !bootloader_flash_erase_sector(start_sector + i) {
            return false;
        }
    }
    true
}

// ---------------------------------------------------------------------------
// 13. bootloader_flash_unlock
// ---------------------------------------------------------------------------
pub fn bootloader_flash_unlock() -> bool {
    let id = bootloader_read_flash_id();
    let mf_id = (id & 0xff) as u8;

    let sr1 =
        (bootloader_execute_flash_command(CMD_RDSR, 0, 0, 8) & 0xff) as u8;
    let sr2 =
        (bootloader_execute_flash_command(CMD_RDSR2, 0, 0, 8) & 0xff) as u8;

    let (new_sr1, new_sr2) = match mf_id {
        FLASH_VENDOR_ID_ISSI => {
            // ISSI: BP bits in SR1 bits 6:3, QE in SR2 bit 1
            let n1 = sr1 & 0x87;
            let n2 = sr2 & 0x02;
            (n1, n2)
        }
        FLASH_VENDOR_ID_MXIC => {
            // MXIC: QE may be in SR1 bit 6; BP typically bits 5:2
            let qe = sr1 & (1 << 6);
            let n1 = (sr1 & 0x83) | qe;
            let n2 = sr2 & 0x02;
            (n1, n2)
        }
        FLASH_VENDOR_ID_GD
        | FLASH_VENDOR_ID_WINBOND
        | FLASH_VENDOR_ID_XMC
        | FLASH_VENDOR_ID_TH => {
            let n1 = sr1 & 0x83;
            let n2 = sr2 & 0x02;
            (n1, n2)
        }
        _ => {
            let n1 = sr1 & 0x83;
            let n2 = sr2 & 0x02;
            (n1, n2)
        }
    };

    let changed = (new_sr1 != sr1) || (new_sr2 != sr2);

    if changed {
        bootloader_execute_flash_command(CMD_WREN, 0, 0, 0);
        if new_sr2 != sr2 {
            // Write both SR1 and SR2 (16 bits: SR2<<8 | SR1)
            bootloader_execute_flash_command(
                CMD_WRSR,
                ((new_sr2 as u32) << 8) | (new_sr1 as u32),
                16,
                0,
            );
        } else {
            bootloader_execute_flash_command(CMD_WRSR, new_sr1 as u32, 8, 0);
        }
        while bootloader_execute_flash_command(CMD_RDSR, 0, 0, 8) & 1 != 0 {}
    }

    bootloader_execute_flash_command(CMD_WRDI, 0, 0, 0);
    true
}

// ---------------------------------------------------------------------------
// 14. bootloader_flash_xmc_startup
// ---------------------------------------------------------------------------
pub fn bootloader_flash_xmc_startup() -> bool {
    let id = bootloader_read_flash_id();
    let mf_id = (id & 0xff) as u8;
    if mf_id != FLASH_VENDOR_ID_XMC {
        return true;
    }
    let sr2 =
        (bootloader_execute_flash_command(CMD_RDSR2, 0, 0, 8) & 0xff) as u8;
    let qe_set = (sr2 & 0x02) != 0;
    if !qe_set {
        bootloader_execute_flash_command(CMD_WREN, 0, 0, 0);
        bootloader_execute_flash_command(CMD_WRSR2, (sr2 | 0x02) as u32, 8, 0);
        while bootloader_execute_flash_command(CMD_RDSR, 0, 0, 8) & 1 != 0 {}
    }
    bootloader_execute_flash_command(CMD_WRDI, 0, 0, 0);
    true
}

// ---------------------------------------------------------------------------
// 15. bootloader_configure_spi_pins
// ---------------------------------------------------------------------------
pub fn bootloader_configure_spi_pins(drv: i32) {
    let drv_u32 = drv as u32;
    unsafe {
        let gpio = esp_rom_efuse_get_flash_gpio_info();
        if gpio == ESP_ROM_EFUSE_FLASH_DEFAULT_SPI {
            esp_rom_gpio_pad_set_drv(MSPI_IOMUX_PIN_NUM_CLK, drv_u32);
            esp_rom_gpio_pad_set_drv(MSPI_IOMUX_PIN_NUM_MISO, drv_u32);
            esp_rom_gpio_pad_set_drv(MSPI_IOMUX_PIN_NUM_MOSI, drv_u32);
            esp_rom_gpio_pad_set_drv(MSPI_IOMUX_PIN_NUM_CS0, drv_u32);
            esp_rom_gpio_pad_set_drv(MSPI_IOMUX_PIN_NUM_HD, drv_u32);
            esp_rom_gpio_pad_set_drv(MSPI_IOMUX_PIN_NUM_WP, drv_u32);
        }
    }
}

// ---------------------------------------------------------------------------
// 16. bootloader_flash_set_dummy_out
// ---------------------------------------------------------------------------
pub fn bootloader_flash_set_dummy_out() {
    let mask = SPI_MEM_FDUMMY_OUT | SPI_MEM_D_POL | SPI_MEM_Q_POL;
    unsafe {
        reg_set_bit(SPI_MEM_CTRL_REG(0), mask);
        reg_set_bit(SPI_MEM_CTRL_REG(1), mask);
    }
}

// ---------------------------------------------------------------------------
// 17. bootloader_flash_cs_timing_config
// ---------------------------------------------------------------------------
pub fn bootloader_flash_cs_timing_config() {
    unsafe {
        let cs_hold_mask = SPI_MEM_CS_HOLD_TIME_V << SPI_MEM_CS_HOLD_TIME_S;
        let cs_setup_mask =
            SPI_MEM_CS_SETUP_TIME_V << SPI_MEM_CS_SETUP_TIME_S;
        let both_mask = cs_hold_mask | cs_setup_mask;
        let val = (1u32 << SPI_MEM_CS_HOLD_TIME_S) | (0u32 << SPI_MEM_CS_SETUP_TIME_S);

        let mut r = reg_read(SPI_MEM_CTRL2_REG(0));
        r = (r & !both_mask) | val;
        reg_write(SPI_MEM_CTRL2_REG(0), r);

        let mut r = reg_read(SPI_MEM_CTRL2_REG(1));
        r = (r & !both_mask) | val;
        reg_write(SPI_MEM_CTRL2_REG(1), r);
    }
}

// ---------------------------------------------------------------------------
// 18. bootloader_flash_clock_config
// ---------------------------------------------------------------------------
pub fn bootloader_flash_clock_config(spi_speed: u8) {
    let clk_reg = spi1_reg(SPI_MEM_CLOCK_OFF);
    unsafe {
        match spi_speed {
            0 => {}
            1 => {
                // 80 MHz: equal to system clock
                reg_set_bit(clk_reg, SPI_MEM_CLK_EQU_SYSCLK);
            }
            2 => {
                // 40 MHz: CLKCNT_N = 1 (APB / 2)
                reg_clr_bit(clk_reg, SPI_MEM_CLK_EQU_SYSCLK);
                reg_set_bits(clk_reg, 1, SPI_MEM_CLKCNT_N_V, SPI_MEM_CLKCNT_N_S);
                reg_set_bits(clk_reg, 0, SPI_MEM_CLKCNT_H_V, SPI_MEM_CLKCNT_H_S);
                reg_set_bits(clk_reg, 0, SPI_MEM_CLKCNT_L_V, SPI_MEM_CLKCNT_L_S);
            }
            3 => {
                // 26.7 MHz: CLKCNT_N = 2 (APB / 3)
                reg_clr_bit(clk_reg, SPI_MEM_CLK_EQU_SYSCLK);
                reg_set_bits(clk_reg, 2, SPI_MEM_CLKCNT_N_V, SPI_MEM_CLKCNT_N_S);
                reg_set_bits(clk_reg, 1, SPI_MEM_CLKCNT_H_V, SPI_MEM_CLKCNT_H_S);
                reg_set_bits(clk_reg, 1, SPI_MEM_CLKCNT_L_V, SPI_MEM_CLKCNT_L_S);
            }
            4 => {
                // 20 MHz: CLKCNT_N = 3 (APB / 4)
                reg_clr_bit(clk_reg, SPI_MEM_CLK_EQU_SYSCLK);
                reg_set_bits(clk_reg, 3, SPI_MEM_CLKCNT_N_V, SPI_MEM_CLKCNT_N_S);
                reg_set_bits(clk_reg, 1, SPI_MEM_CLKCNT_H_V, SPI_MEM_CLKCNT_H_S);
                reg_set_bits(clk_reg, 1, SPI_MEM_CLKCNT_L_V, SPI_MEM_CLKCNT_L_S);
            }
            _ => {}
        }
    }
}

// ---------------------------------------------------------------------------
// 19. bootloader_flash_update_id
// ---------------------------------------------------------------------------
pub fn bootloader_flash_update_id() {
    let id = bootloader_read_flash_id();
    unsafe {
        let chip = crate::rom::g_rom_flashchip_mut();
        (*chip).device_id = id;
    }
}

// ---------------------------------------------------------------------------
// 20. bootloader_flash_update_size
// ---------------------------------------------------------------------------
pub fn bootloader_flash_update_size(size: u32) {
    unsafe {
        let chip = crate::rom::g_rom_flashchip_mut();
        (*chip).chip_size = size;
    }
}

// ---------------------------------------------------------------------------
// QIO chip information table for bootloader_enable_qio_mode
// ---------------------------------------------------------------------------
#[derive(Clone, Copy)]
struct QioChipData {
    mf_id: u8,
    qe_sr: u8,
    qe_bit: u8,
    wrsr_cmd: u8,
}

static QIO_CHIPS: &[QioChipData] = &[
    QioChipData {
        mf_id: 0xC2,
        qe_sr: 1,
        qe_bit: 6,
        wrsr_cmd: 0x01,
    }, // MXIC
    QioChipData {
        mf_id: 0x9D,
        qe_sr: 2,
        qe_bit: 1,
        wrsr_cmd: 0x31,
    }, // ISSI
    QioChipData {
        mf_id: 0xEF,
        qe_sr: 2,
        qe_bit: 1,
        wrsr_cmd: 0x31,
    }, // WinBond
    QioChipData {
        mf_id: 0xC8,
        qe_sr: 2,
        qe_bit: 1,
        wrsr_cmd: 0x31,
    }, // GD
    QioChipData {
        mf_id: 0x20,
        qe_sr: 2,
        qe_bit: 1,
        wrsr_cmd: 0x31,
    }, // XMC XM25QU64A
    QioChipData {
        mf_id: 0x4B,
        qe_sr: 2,
        qe_bit: 1,
        wrsr_cmd: 0x31,
    }, // TH
    QioChipData {
        mf_id: 0x00,
        qe_sr: 2,
        qe_bit: 1,
        wrsr_cmd: 0x31,
    }, // default probe
];

// ---------------------------------------------------------------------------
// 22. bootloader_enable_qio_mode
// ---------------------------------------------------------------------------
pub fn bootloader_enable_qio_mode() {
    let id = bootloader_read_flash_id();
    let mf_id = (id & 0xff) as u8;

    let chip = QIO_CHIPS
        .iter()
        .find(|c| c.mf_id == mf_id)
        .unwrap_or(&QIO_CHIPS[QIO_CHIPS.len() - 1]);

    let sr_cmd = if chip.qe_sr == 1 {
        CMD_RDSR
    } else {
        CMD_RDSR2
    };
    let sr = (bootloader_execute_flash_command(sr_cmd, 0, 0, 8) & 0xff) as u8;
    let qe_mask = 1u8 << chip.qe_bit;

    if sr & qe_mask == 0 {
        bootloader_execute_flash_command(CMD_WREN, 0, 0, 0);
        let new_sr = sr | qe_mask;
        bootloader_execute_flash_command(chip.wrsr_cmd, new_sr as u32, 8, 0);
        while bootloader_execute_flash_command(CMD_RDSR, 0, 0, 8) & 1 != 0 {}
    }

    // Configure SPI controller for quad I/O mode on both SPI0 (cache) and SPI1
    unsafe {
        reg_set_bit(SPI_MEM_CTRL_REG(0), SPI_MEM_FREAD_QIO);
        reg_set_bit(SPI_MEM_CTRL_REG(1), SPI_MEM_FREAD_QIO);
        reg_set_bit(SPI_MEM_CTRL_REG(0), SPI_MEM_FREAD_QUAD);
        reg_set_bit(SPI_MEM_CTRL_REG(1), SPI_MEM_FREAD_QUAD);
    }

    bootloader_execute_flash_command(CMD_WRDI, 0, 0, 0);
}

// ---------------------------------------------------------------------------
// 21. bootloader_init_spi_flash
// ---------------------------------------------------------------------------
pub fn bootloader_init_spi_flash() -> bool {
    // a. Configure SPI pins
    bootloader_configure_spi_pins(2);

    // b. Resume flash from deep power-down
    spi_flash_resume();

    // c. Unlock flash
    if !bootloader_flash_unlock() {
        return false;
    }

    // d. Enable QIO mode if configured for QIO/QOUT
    let mode = bootloader_flash_get_spi_mode();
    if mode == SpiFlashReadMode::Qio || mode == SpiFlashReadMode::Qout {
        bootloader_enable_qio_mode();
    }

    // e. Print flash info (no-op in no_std; flash id/size available
    //    via rom_spiflash_legacy_data)

    // f. Update flash config from image header (magic, SPI mode, speed/size)
    let (_magic, _spi_mode, spi_speed_size) = read_flash_config_from_header();
    let flash_size = (spi_speed_size >> 4) as u32;
    if flash_size <= 4 {
        let chip_size = 1_048_576u32 << flash_size; // 1MB << n
        bootloader_flash_update_size(chip_size);
        unsafe {
            let chip = crate::rom::g_rom_flashchip();
            esp_rom_spiflash_config_param(
                (*chip).device_id,
                chip_size,
                FLASH_BLOCK_SIZE,
                FLASH_SECTOR_SIZE,
                256,
                0xffff,
            );
        }
    }

    // g. Enable write protect
    bootloader_enable_wp();

    true
}

// ---------------------------------------------------------------------------
// Legacy mmap / munmap helpers (used by partition, image, utility modules)
// ---------------------------------------------------------------------------
//
// During early bring-up, use a ROM-read bounce buffer instead of programming
// cache/MMU mappings. Flash encryption is checked by callers before requesting
// decrypted reads, and this target currently has flash encryption disabled.
// This keeps partition-table and image parsing independent from the MMU setup.

static mut FLASH_ENC_CHECKED: bool = false;
static mut FLASH_ENCRYPTED: bool = false;

fn flash_encryption_enabled() -> bool {
    unsafe {
        if !FLASH_ENC_CHECKED {
            FLASH_ENCRYPTED = crate::rom::efuse_hal_flash_encryption_enabled();
            FLASH_ENC_CHECKED = true;
        }
        FLASH_ENCRYPTED
    }
}

static mut MAPPED: bool = false;
// ROM esp_rom_spiflash_read requires uint32_t* (4-byte-aligned) destination.
// Wrap in align(4) to prevent a 1-byte offset when bool MAPPED precedes this.
#[repr(C, align(4))]
struct AlignedBuf([u8; 4096]);
static mut MMAP_BUF: AlignedBuf = AlignedBuf([0; 4096]);
static mut CURRENT_MAPPED_SIZE: u32 = 0;
static mut CURRENT_READ_MAPPING: u32 = u32::MAX;

/// Number of MMU pages available for `bootloader_mmap`.
pub fn bootloader_mmap_get_free_pages() -> u32 {
    MMU_FREE_PAGES
}

/// Map a region of flash by reading it into a small static RAM buffer.
///
/// Returns a pointer to the copied flash content, or null on failure. Only one
/// mapping may be active at a time; call `bootloader_munmap` before creating
/// another.
pub fn bootloader_mmap(src_paddr: u32, size: u32) -> *const u8 {
    // Leave 3 bytes of headroom so rounding to 4 bytes can't overflow the buffer.
    if size == 0 || size > 4093 {
        return core::ptr::null();
    }
    unsafe {
        if MAPPED {
            return core::ptr::null();
        }

        let ok = if flash_encryption_enabled() {
            // Cache reads auto-decrypt via XTS-AES hardware.
            // Round up to 4-byte boundary: flash_read_via_cache iterates in u32 steps.
            let aligned = (size + 3) & !3;
            flash_read_via_cache(src_paddr, MMAP_BUF.0.as_mut_ptr() as *mut u32, aligned, true)
        } else {
            flash_read_raw(src_paddr, MMAP_BUF.0.as_mut_ptr(), size)
        };

        if !ok {
            return core::ptr::null();
        }

        MAPPED = true;
        CURRENT_MAPPED_SIZE = size;
        MMAP_BUF.0.as_ptr()
    }
}

/// Release the mapping created by `bootloader_mmap`.
pub fn bootloader_munmap(_ptr: *const u8) {
    unsafe {
        MAPPED = false;
        CURRENT_MAPPED_SIZE = 0;
    }
}

/// Read raw flash bytes (no decryption) using the ROM SPI read function.
/// Disables cache during the read to avoid stale data.
unsafe fn flash_read_raw(src_addr: u32, dest: *mut u8, size: u32) -> bool {
    // QEMU esp_rom_spiflash_read adds 1 to the source address before reading
    // (diagnosed by reading addr 0 → data starts at flash[1], addr 0x7FFF →
    // data starts at flash[0x8000]).  Subtract 1 to compensate.  wrapping_sub
    // is safe: addr-1 for addr=0 wraps to 0xFFFFFFFF which is beyond any real
    // flash and makes the header parse return no-op defaults.
    let autoload = crate::rom::Cache_Suspend_ICache();
    let r = esp_rom_spiflash_read(src_addr.wrapping_sub(1), dest, size);
    crate::rom::Cache_Resume_ICache(autoload);
    r == SpiFlashResult::Ok
}

/// Read flash with optional on-the-fly decryption via cache.
///
/// Uses the reserved last MMU page (`FLASH_READ_VADDR`) as a 64 KB sliding
/// window.  When `allow_decrypt` is true, reads go through the cache which
/// transparently handles AES-XTS decryption.
unsafe fn flash_read_via_cache(src_addr: u32, dest: *mut u32, size: u32, _allow_decrypt: bool) -> bool {
    for word in 0..(size / 4) {
        let word_src: u32 = src_addr + word * 4;
        let map_at: u32 = word_src & MMU_FLASH_MASK; // 64 KB-aligned block

        if map_at != CURRENT_READ_MAPPING {
            // Suspend cache, remap the single-page window, resume.
            let autoload = crate::rom::Cache_Suspend_ICache();
            crate::rom::Cache_Invalidate_ICache_All();

            crate::rom::Cache_Dbus_MMU_Set(0, FLASH_READ_VADDR, map_at, 64, 1, 0);
            crate::rom::Cache_Invalidate_ICache_All();
            crate::rom::Cache_Resume_ICache(autoload);

            CURRENT_READ_MAPPING = map_at;
        }

        // Read one word through the cache window.
        let map_ptr: *const u32 = (FLASH_READ_VADDR + (word_src - map_at)) as *const u32;
        *dest.add(word as usize) = core::ptr::read_volatile(map_ptr);
    }
    // Ensure the next read re-maps even if the address happens to match.
    CURRENT_READ_MAPPING = u32::MAX;
    true
}

/// Public flash read entry point.
///
/// Uses ROM direct reads when `allow_decrypt` is false; uses the MMU cache
/// window when `allow_decrypt` is true (for transparent AES-XTS).
pub unsafe fn bootloader_flash_read(
    src_addr: u32,
    dest: *mut u8,
    size: u32,
    allow_decrypt: bool,
) -> bool {
    if src_addr & 3 != 0 || size & 3 != 0 || (dest as usize) & 3 != 0 {
        return false;
    }
    if allow_decrypt {
        flash_read_via_cache(src_addr, dest as *mut u32, size, allow_decrypt)
    } else {
        flash_read_raw(src_addr, dest, size)
    }
}
