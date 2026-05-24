//! Boot-selection and application-loading module for the ESP32-C3 bootloader.
//!
//! Orchestrates the following high-level flow:
//! 1. Read the OTA data partition to discover the last / preferred OTA slot.
//! 2. Honour GPIO-held override pins (factory-reset, test-app).
//! 3. Iterate candidate partitions (backward then forward) trying to load an
//!    image.
//! 4. On success, configure the MMU / cache for DROM and IROM segments and
//!    jump to the application entry point.

use core::{mem, ptr, slice};

use crate::flash;
use crate::image;
use crate::rom::*;
use crate::soc::*;
use crate::{BootloaderState, PartitionPos};

// ---------------------------------------------------------------------------
// Partition-index constants
// ---------------------------------------------------------------------------

pub const FACTORY_INDEX: i32 = -1;
pub const TEST_APP_INDEX: i32 = -2;
pub const INVALID_INDEX: i32 = -99;

/// Maximum number of OTA slots defined in `BootloaderState`.
pub const MAX_OTA_SLOTS: usize = 16;

// ---------------------------------------------------------------------------
// OTA-state constants
// ---------------------------------------------------------------------------

pub const ESP_OTA_IMG_NEW: u32 = 0;
pub const ESP_OTA_IMG_PENDING_VERIFY: u32 = 1;
pub const ESP_OTA_IMG_VALID: u32 = 2;
pub const ESP_OTA_IMG_INVALID: u32 = 3;
pub const ESP_OTA_IMG_ABORTED: u32 = 4;

// ---------------------------------------------------------------------------
// OTA-select entry (first few bytes of each 4 KiB OTA-data sector)
// ---------------------------------------------------------------------------

/// Single OTA-select entry stored at the beginning of a 4 KiB sector.
#[derive(Clone, Copy)]
#[repr(C)]
pub struct EspOtaSelectEntry {
    pub ota_seq: u32,
    pub ota_state: u32,
    pub crc: u32,
    // The remainder of the sector (4096 - 12 bytes) is padding / unused.
}

// ---------------------------------------------------------------------------
// GPIO valid mask for ESP32-C3 (GPIO 0-21)
// ---------------------------------------------------------------------------

const SOC_GPIO_VALID_GPIO_MASK: u64 = 0x3FFFFF;

// ---------------------------------------------------------------------------
// Long-hold GPIO detection for factory reset / test app
// ---------------------------------------------------------------------------

#[derive(PartialEq)]
enum GpioHold {
    NotHold = 0,
    ShortHold = 1,
    LongHold = 2,
}

/// Check whether a GPIO pin is held at a given level for a sustained period.
///
/// First configures the pin as an input with pull-up, then verifies the
/// initial level.  If the pin stays at `level` for `delay_sec` seconds the
/// function returns `LongHold`.  If the pin is released earlier it returns
/// `ShortHold`.  If the pin never matches the requested level, `NotHold` is
/// returned.
fn bootloader_common_check_long_hold_gpio_level(pin: u32, delay_sec: u32, level: bool) -> GpioHold {
    if pin > 21 || ((1u64 << pin) & SOC_GPIO_VALID_GPIO_MASK) == 0 {
        return GpioHold::NotHold;
    }
    // Configure pin as GPIO input with pullup
    unsafe {
        esp_rom_gpio_pad_select_gpio(pin);
        gpio_ll_input_enable(pin);
        esp_rom_gpio_pad_pullup_only(pin);
    }
    // Check initial level - must match
    if gpio_ll_get_level(pin) != level {
        return GpioHold::NotHold;
    }
    // Poll the pin for the requested duration.
    for _ in 0..(delay_sec * 1000) {
        unsafe { esp_rom_delay_us(1000); }
        if gpio_ll_get_level(pin) != level {
            return GpioHold::ShortHold;
        }
    }
    GpioHold::LongHold
}

// ---------------------------------------------------------------------------
// Factory-reset helper: erase data partitions by label
// ---------------------------------------------------------------------------

/// Erase all data partitions whose label matches one of the comma-separated
/// `labels` string.  When `do_erase` is false only the probe is performed.
/// Returns `true` if at least one matching partition was found (and erased).
fn bootloader_common_erase_part_type_data(labels: &str, do_erase: bool) -> bool {
    // Re-map the partition table from flash.
    let addr = flash::bootloader_mmap(
        crate::soc::ESP_PARTITION_TABLE_OFFSET,
        crate::soc::ESP_PARTITION_TABLE_MAX_LEN,
    );
    if addr.is_null() {
        return false;
    }

    /// On-flash partition entry (32 bytes), matching the hardware layout.
    #[repr(C, packed)]
    struct PartEntry {
        magic: u16,
        type_: u8,
        _subtype: u8,
        offset: u32,
        size: u32,
        label: [u8; 16],
        _flags: u32,
    }

    // Maximum partition table entries is 95.
    let entries =
        unsafe { slice::from_raw_parts(addr as *const PartEntry, 95) };

    let mut erased = false;

    for entry in entries {
        if entry.magic != 0x50AA {
            break;
        }
        // Only erase data-type partitions.
        if entry.type_ != 0x01 {
            // PART_TYPE_DATA = 0x01
            continue;
        }

        // Determine the actual string length (NUL-terminated label).
        let label_len = entry
            .label
            .iter()
            .position(|&b| b == 0)
            .unwrap_or(entry.label.len());
        let label_slice = &entry.label[..label_len];

        // Compare against each label in the comma-separated list.
        for list_label in labels.split(',') {
            let list_label = list_label.trim();
            if list_label.is_empty() {
                continue;
            }
            if label_slice.len() != list_label.len() {
                continue;
            }
            if label_slice.iter().zip(list_label.bytes()).all(|(&a, b)| a == b) {
                if do_erase {
                    flash::bootloader_flash_erase_range(entry.offset, entry.size);
                }
                erased = true;
                break;
            }
        }
    }

    flash::bootloader_munmap(addr);
    erased
}

// ---------------------------------------------------------------------------
// Page / sector geometry (duplicated here for convenience)
// ---------------------------------------------------------------------------

const PAGE_SIZE: u32 = SPI_FLASH_MMU_PAGE_SIZE; // 64 KiB
const SECTOR_SIZE: u32 = FLASH_SECTOR_SIZE; // 4 KiB

// ---------------------------------------------------------------------------
// ROM cache / MMU functions are in crate::rom (function pointers to known ROM addresses).

// ===========================================================================
// Helper: read GPIO input level
// ===========================================================================

/// Read the logic level of an GPIO pin.
///
/// Returns `true` for high (1), `false` for low (0).  Pins >= 22 always
/// return `true` (not asserted).
fn gpio_read(pin: u8) -> bool {
    if pin >= 22 {
        // Beyond the valid GPIO range for ESP32-C3.
        return true;
    }
    // GPIO_IN_REG on ESP32-C3.
    const GPIO_IN_REG: u32 = 0x6000_403C;
    let reg = GPIO_IN_REG as *const u32;
    unsafe { (ptr::read_volatile(reg) >> pin) & 1 != 0 }
}

// ===========================================================================
// OTA-data helpers
// ===========================================================================

/// Read the two OTA-select entries from the OTA-info partition.
///
/// Uses a single MMU mapping covering both 4 KiB sectors.  Returns `true`
/// on success.
fn bootloader_common_read_otadata(
    ota_info: &PartitionPos,
    two_otadata: &mut [EspOtaSelectEntry; 2],
) -> bool {
    if ota_info.offset == 0 {
        return false;
    }
    if ota_info.size < 2 * SECTOR_SIZE {
        return false;
    }

    // Map each sector separately — bootloader_mmap only handles up to 4093 bytes,
    // and the two sectors together (8 KB) would exceed that limit.
    let entry_size = mem::size_of::<EspOtaSelectEntry>() as u32;

    let map0 = flash::bootloader_mmap(ota_info.offset, entry_size);
    if map0.is_null() {
        return false;
    }
    unsafe {
        ptr::copy_nonoverlapping(
            map0,
            &mut two_otadata[0] as *mut EspOtaSelectEntry as *mut u8,
            entry_size as usize,
        );
    }
    flash::bootloader_munmap(map0);

    let map1 = flash::bootloader_mmap(ota_info.offset + SECTOR_SIZE, entry_size);
    if map1.is_null() {
        return false;
    }
    unsafe {
        ptr::copy_nonoverlapping(
            map1,
            &mut two_otadata[1] as *mut EspOtaSelectEntry as *mut u8,
            entry_size as usize,
        );
    }
    flash::bootloader_munmap(map1);
    true
}

/// Calculate the CRC of an OTA-select entry.
///
/// CRC is computed over the first 4 bytes (`ota_seq`) using an initial
/// value of `0xFFFFFFFF` and the standard ESP-ROM CRC32-LE polynomial.
fn bootloader_common_ota_select_crc(entry: &EspOtaSelectEntry) -> u32 {
    esp_rom_crc32_le(u32::MAX, &entry.ota_seq as *const u32 as *const u8, 4)
}

/// Check whether an OTA-select entry is valid (valid sequence, non-invalid
/// state, and matching CRC).
fn bootloader_common_ota_select_valid(entry: &EspOtaSelectEntry) -> bool {
    entry.ota_seq != u32::MAX
        && entry.ota_state != ESP_OTA_IMG_INVALID
        && entry.ota_state != ESP_OTA_IMG_ABORTED
        && entry.crc == bootloader_common_ota_select_crc(entry)
}

/// Quick check whether an OTA-select entry is obviously invalid (all-ones
/// sequence, INVALID state, or ABORTED state).  Does not verify CRC.
fn bootloader_common_ota_select_invalid(entry: &EspOtaSelectEntry) -> bool {
    entry.ota_seq == u32::MAX
        || entry.ota_state == ESP_OTA_IMG_INVALID
        || entry.ota_state == ESP_OTA_IMG_ABORTED
}

/// Select an OTA-data sector index based on validity flags.
///
/// When both are valid and `max` is `true` the higher sequence number wins;
/// when `max` is `false` the lower sequence number wins.  Ties favour
/// index 0.  Returns the selected index (0 or 1) or -1 if neither is valid.
fn bootloader_common_select_otadata(
    otadata: &[EspOtaSelectEntry; 2],
    valid: &[bool; 2],
    max: bool,
) -> i32 {
    if valid[0] && valid[1] {
        let condition = if max {
            otadata[0].ota_seq.max(otadata[1].ota_seq)
        } else {
            otadata[0].ota_seq.min(otadata[1].ota_seq)
        };
        if condition == otadata[0].ota_seq {
            0
        } else {
            1
        }
    } else if valid[0] {
        0
    } else if valid[1] {
        1
    } else {
        -1
    }
}

/// Return the index (0 or 1) of the active OTA-data sector, or -1 if none
/// is valid.
///
/// When both are valid the one with the higher `ota_seq` wins; ties favour
/// index 0.
fn bootloader_common_get_active_otadata(otadata: &[EspOtaSelectEntry; 2]) -> i32 {
    let mut valid = [false; 2];
    valid[0] = bootloader_common_ota_select_valid(&otadata[0]);
    valid[1] = bootloader_common_ota_select_valid(&otadata[1]);
    bootloader_common_select_otadata(otadata, &valid, true)
}

// ===========================================================================
// OTA-data update helper
// ===========================================================================

/// Write a new (inactive) OTA-select entry to record that we are about to
/// boot from the given OTA slot.
///
/// This implements `set_actual_ota_seq` -- it erases the inactive sector and
/// writes a fresh entry with the sequence number `start_index + 1` and state
/// `PENDING_VERIFY`.
fn set_actual_ota_seq(bs: &BootloaderState, start_index: i32) {
    // Only meaningful for OTA slots (non-negative).
    if bs.ota_info.size < 2 * SECTOR_SIZE || start_index < 0 {
        return;
    }

    let mut otadata = [EspOtaSelectEntry {
        ota_seq: 0xFFFF_FFFF,
        ota_state: ESP_OTA_IMG_NEW,
        crc: 0,
    }; 2];

    if !bootloader_common_read_otadata(&bs.ota_info, &mut otadata) {
        return;
    }

    let active = bootloader_common_get_active_otadata(&otadata);
    // Write to the inactive sector (the one we are NOT currently reading).
    let write_slot = if active != 1 { 1u32 } else { 0u32 };

    let new_entry = EspOtaSelectEntry {
        ota_seq: (start_index as u32) + 1,
        ota_state: ESP_OTA_IMG_PENDING_VERIFY,
        crc: 0, // filled below
    };

    let mut entry = new_entry;
    entry.crc = bootloader_common_ota_select_crc(&entry);

    let addr = bs.ota_info.offset + write_slot * SECTOR_SIZE;
    let sector_num = addr / SECTOR_SIZE;

    unsafe {
        esp_rom_spiflash_wait_idle(g_rom_flashchip());
        if esp_rom_spiflash_erase_sector(sector_num) != SpiFlashResult::Ok {
            return;
        }
        esp_rom_spiflash_wait_idle(g_rom_flashchip());
        esp_rom_spiflash_write(
            addr,
            &entry as *const EspOtaSelectEntry as *const u8,
            mem::size_of::<EspOtaSelectEntry>() as u32,
        );
    }
}

// ===========================================================================
// Boot-partition selection
// ===========================================================================

/// High-level entry: load the partition table, consult OTA data and GPIO
/// overrides, and return the preferred partition index.
pub fn select_partition_number(bs: &mut BootloaderState) -> i32 {
    crate::init::debug_tx_str(b"[P]");
    if !crate::partition::bootloader_utility_load_partition_table(bs) {
        crate::init::debug_tx_str(b"[p]");
        return INVALID_INDEX;
    }
    crate::init::debug_tx_str(b"[Q]");
    selected_boot_partition(bs)
}

/// Determine which partition to boot from, considering GPIO overrides and
/// long-hold detection for factory reset and test app.
fn selected_boot_partition(bs: &BootloaderState) -> i32 {
    let idx = bootloader_utility_get_selected_boot_partition(bs);

    let reset_reason = unsafe { esp_rom_get_reset_reason(0) };

    // On deep-sleep wake we never honour GPIO overrides (the app is
    // expected to resume where it left off).
    if reset_reason != RESET_REASON_CORE_DEEP_SLEEP {
        // Factory-reset GPIO long-hold detection.
        #[cfg(feature = "factory_reset")]
        {
            let reset_pin: u32 = 9; // GPIO9 is common factory reset pin on C3
            if bootloader_common_check_long_hold_gpio_level(reset_pin, 5, false) == GpioHold::LongHold
            {
                // Erase data partitions listed in CONFIG_BOOTLOADER_DATA_FACTORY_RESET.
                bootloader_common_erase_part_type_data("nvs, phy, otadata", true);
                // Update RTC retain mem for factory reset state.
                bootloader_common_set_rtc_retain_mem_factory_reset_state();
                return bootloader_utility_get_selected_boot_partition(bs);
            }
        }

        // Test-app GPIO long-hold detection.
        #[cfg(feature = "app_test")]
        {
            let test_pin: u32 = 8; // GPIO8 is common test app pin on C3
            if bootloader_common_check_long_hold_gpio_level(test_pin, 5, false) == GpioHold::LongHold {
                if bs.test.offset != 0 {
                    return TEST_APP_INDEX;
                }
            }
        }
    }

    idx
}

/// Read OTA data and determine which OTA slot (or factory) should boot.
///
/// Returns `FACTORY_INDEX` when no valid OTA data exists or the selected
/// partition is unavailable.
///
/// Before choosing the active OTA, any entry in `PENDING_VERIFY` state is
/// marked `ABORTED` (rollback of an unverified update).
pub fn bootloader_utility_get_selected_boot_partition(bs: &BootloaderState) -> i32 {
    // No OTA-info partition -> must boot factory.
    if bs.ota_info.size == 0 {
        return FACTORY_INDEX;
    }

    let mut otadata = [EspOtaSelectEntry {
        ota_seq: 0xFFFF_FFFF,
        ota_state: ESP_OTA_IMG_NEW,
        crc: 0,
    }; 2];

    if !bootloader_common_read_otadata(&bs.ota_info, &mut otadata) {
        return FACTORY_INDEX;
    }

    // ---- OTA rollback: mark PENDING_VERIFY as ABORTED ---------------------
    for i in 0..2 {
        if otadata[i].ota_state == ESP_OTA_IMG_PENDING_VERIFY {
            otadata[i].ota_state = ESP_OTA_IMG_ABORTED;
            // Recompute CRC after changing the state.
            otadata[i].crc = bootloader_common_ota_select_crc(&otadata[i]);

            let offset = bs.ota_info.offset + SECTOR_SIZE * i as u32;
            let sector = offset / SECTOR_SIZE;

            // Erase the sector and write the updated entry.
            if flash::bootloader_flash_erase_sector(sector) {
                unsafe {
                    flash::bootloader_flash_write(
                        offset,
                        &otadata[i] as *const EspOtaSelectEntry as *const u8,
                        mem::size_of::<EspOtaSelectEntry>() as u32,
                        false,
                    );
                }
            }
        }
    }

    let active = bootloader_common_get_active_otadata(&otadata);
    if active < 0 {
        return FACTORY_INDEX;
    }

    let entry = &otadata[active as usize];

    // An aborted entry forces factory boot.
    if entry.ota_state == ESP_OTA_IMG_ABORTED {
        return FACTORY_INDEX;
    }

    // Sequence numbers are 1-based in the OTA data.
    let slot = entry.ota_seq.wrapping_sub(1);
    if slot >= MAX_OTA_SLOTS as u32 {
        return FACTORY_INDEX;
    }

    let slot = slot as usize;

    // Verify the slot has a non-zero partition.
    if bs.ota[slot].size == 0 {
        return FACTORY_INDEX;
    }

    slot as i32
}

// ===========================================================================
// Image loading and boot
// ===========================================================================

/// Resolve an index to a `PartitionPos` reference.
fn get_partition(bs: &BootloaderState, index: i32) -> Option<&PartitionPos> {
    match index {
        FACTORY_INDEX => Some(&bs.factory),
        TEST_APP_INDEX => Some(&bs.test),
        i if i >= 0 && (i as usize) < MAX_OTA_SLOTS && bs.ota[i as usize].size > 0 => {
            Some(&bs.ota[i as usize])
        }
        _ => None,
    }
}

/// Anti-rollback check: verify the partition's secure version against the eFuse counter.
///
/// Reads `esp_app_desc_t` (magic 0xABCD5432, secure_version at offset +4) located at
/// `partition.offset + 32` (after EspImageHeader(24) + EspImageSegmentHeader(8)).
/// Returns true if the app version is >= the eFuse secure version, or if the partition
/// has no valid app descriptor.
fn check_anti_rollback(partition: &PartitionPos) -> bool {
    let desc_addr = partition.offset.wrapping_add(32);
    let ptr = flash::bootloader_mmap(desc_addr, 8);
    if ptr.is_null() {
        return true; // can't read — fail open
    }
    let magic   = unsafe { ptr::read_unaligned(ptr.cast::<u32>()) };
    let app_ver = unsafe { ptr::read_unaligned((ptr as usize + 4) as *const u32) };
    flash::bootloader_munmap(ptr);

    if magic != 0xABCD_5432 {
        return true; // no esp_app_desc_t (test app, custom image) — allow boot
    }

    let efuse_ver = crate::soc::efuse_get_secure_version();
    if app_ver < efuse_ver {
        crate::init::debug_tx_str(b"[R]");
        return false;
    }
    true
}

/// Attempt to load an image from a single partition.
///
/// Returns `true` if the image was loaded successfully.
fn try_load_partition(part: &PartitionPos, data: &mut image::EspImageMetadata) -> bool {
    if part.size == 0 {
        crate::init::debug_tx_str(b"[z]");
        return false;
    }
    if !check_anti_rollback(part) {
        crate::init::debug_tx_str(b"[a]");
        return false;
    }
    crate::init::debug_tx_str(b"[L]");
    let ok = image::bootloader_load_image(part, data);
    if ok {
        crate::init::debug_tx_str(b"[l]");
    } else {
        crate::init::debug_tx_str(b"[x]");
    }
    ok
}

/// Main boot-image loading routine.
///
/// Tries partitions starting from `start_index`:
///   - backward to `FACTORY_INDEX` (and test-app if applicable),
///   - then forward from `start_index + 1` to the end of the OTA slot list.
///
/// On success it updates the OTA sequence (if applicable) and jumps to the
/// application.  If no partition is bootable it resets the chip.
// A `start_index` of `TEST_APP_INDEX` is handled by trying the test-app
// partition first, then falling through to the normal flow with
// `FACTORY_INDEX`.
pub fn bootloader_utility_load_boot_image(bs: &BootloaderState, start_index: i32) -> ! {
    let mut data = image::EspImageMetadata::default();
    crate::init::debug_tx_str(b"[I]");

    // -- Special handling for TEST_APP_INDEX --------------------------------
    let actual_start = if start_index == TEST_APP_INDEX {
        if try_load_partition(&bs.test, &mut data) {
            load_image(&data);
        }
        FACTORY_INDEX
    } else {
        start_index
    };

    // -- Backward sweep: actual_start .. FACTORY_INDEX ----------------------
    let mut i = actual_start;
    while i >= FACTORY_INDEX {
        if let Some(part) = get_partition(bs, i) {
            if try_load_partition(part, &mut data) {
                set_actual_ota_seq(bs, i);
                crate::init::debug_tx_str(b"[J]");
                load_image(&data);
            }
        }
        i -= 1;
    }

    // -- Forward sweep: actual_start+1 .. app_count-1 -----------------------
    let mut i = actual_start + 1;
    while i < bs.app_count as i32 {
        if let Some(part) = get_partition(bs, i) {
            if try_load_partition(part, &mut data) {
                set_actual_ota_seq(bs, i);
                crate::init::debug_tx_str(b"[K]");
                load_image(&data);
            }
        }
        i += 1;
    }

    bootloader_reset()
}

/// Try a fast boot from deep sleep using the partition saved in RTC retain
/// memory.  This skips verification to minimise wake latency.
///
/// If the fast boot succeeds this function never returns.  Otherwise it
/// returns normally and the caller should proceed with the normal boot flow.
pub fn bootloader_utility_load_boot_image_from_deep_sleep() {
    let reset_reason = unsafe { esp_rom_get_reset_reason(0) };
    if reset_reason == RESET_REASON_CORE_DEEP_SLEEP {
        if is_rtc_retain_mem_valid() {
            let partition = unsafe { &(*rtc_retain_mem()).partition };
            if partition.offset != 0 {
                let mut image_data = image::EspImageMetadata::default();
                if image::bootloader_load_image_no_verify(partition, &mut image_data) {
                    unsafe {
                        Cache_Disable_ICache();
                    }
                    load_image(&image_data);
                }
            }
        }
    }
}

// ===========================================================================
// Anti-rollback: done above in check_anti_rollback.
// ===========================================================================

// ===========================================================================
// RTC retain memory for deep sleep fast boot
// ===========================================================================

/// Structure stored at the base of RTC DRAM to persist boot-selection state
/// across deep sleep cycles.
#[repr(C)]
struct RtcRetainMem {
    crc: u32,
    reboot_counter: u32,
    partition: PartitionPos,
    flags: RtcRetainFlags,
    custom: [u8; 64], // reserved
}

#[repr(C)]
struct RtcRetainFlags {
    factory_reset_state: bool,
}

/// The RTC retain memory area starts at the beginning of RTC IRAM/DRAM on
/// ESP32-C3 (SOC_RTC_IRAM_LOW = 0x50000000).
const RTC_RETAIN_MEM_ADDR: u32 = SOC_RTC_IRAM_LOW;

fn rtc_retain_mem() -> *mut RtcRetainMem {
    RTC_RETAIN_MEM_ADDR as *mut RtcRetainMem
}

fn is_rtc_retain_mem_valid() -> bool {
    let mem = unsafe { &*rtc_retain_mem() };
    if mem.crc == u32::MAX {
        return false;
    }
    let computed = crc32_retain_mem(rtc_retain_mem());
    computed == mem.crc
}

fn update_rtc_retain_mem_crc() {
    let crc = crc32_retain_mem(rtc_retain_mem());
    unsafe {
        (*rtc_retain_mem()).crc = crc;
    }
}

/// Compute CRC32-LE over the retain memory content, skipping the CRC field
/// itself (first 4 bytes).
fn crc32_retain_mem(mem: *mut RtcRetainMem) -> u32 {
    // Cast to byte pointer first, then skip 4 bytes (the CRC field).
    let base = unsafe { (mem as *const u8).add(4) };
    esp_rom_crc32_le(u32::MAX, base, mem::size_of::<RtcRetainMem>() - 4)
}

fn bootloader_common_set_rtc_retain_mem_factory_reset_state() {
    if !is_rtc_retain_mem_valid() {
        unsafe {
            // Zero out the entire structure byte by byte.
            ptr::write_bytes(rtc_retain_mem() as *mut u8, 0u8, mem::size_of::<RtcRetainMem>());
        }
    }
    unsafe {
        (*rtc_retain_mem()).flags.factory_reset_state = true;
    }
    update_rtc_retain_mem_crc();
}

// ===========================================================================
// Final boot steps
// ===========================================================================

/// Extract DROM and IROM mapping parameters from loaded image metadata.
///
/// Returns `(drom_addr, drom_load, drom_size, irom_addr, irom_load, irom_size)`.
fn extract_segment_info(data: &image::EspImageMetadata) -> (u32, u32, u32, u32, u32, u32) {
    let mut drom_addr: u32 = 0;
    let mut drom_load: u32 = 0;
    let mut drom_size: u32 = 0;
    let mut irom_addr: u32 = 0;
    let mut irom_load: u32 = 0;
    let mut irom_size: u32 = 0;

    for i in 0..data.image.segment_count as usize {
        let load_addr = data.segments[i].load_addr;
        let data_addr = data.segment_data[i];
        let data_len = data.segments[i].data_len;

        if data_len == 0 {
            continue;
        }

        if load_addr >= SOC_DROM_LOW && load_addr < SOC_DROM_HIGH {
            // DROM segment -- first occurrence wins.
            if drom_size == 0 {
                drom_addr = data_addr;
                drom_load = load_addr;
                drom_size = data_len;
            }
        } else if load_addr >= SOC_IROM_LOW && load_addr < SOC_IROM_HIGH {
            // IROM segment -- first occurrence wins.
            if irom_size == 0 {
                irom_addr = data_addr;
                irom_load = load_addr;
                irom_size = data_len;
            }
        }
    }

    (drom_addr, drom_load, drom_size, irom_addr, irom_load, irom_size)
}

/// Disable RNG entropy source and glitch-detection reset, then configure the
/// MMU / cache and jump to the application.
fn load_image(image_data: &image::EspImageMetadata) -> ! {
    // Disable the RNG entropy source.
    unsafe {
        let rng_clk_en_reg = SYSTEM_CPU_PERI_CLK_EN_REG as *mut u32;
        // Bit 0 in SYSTEM_CPU_PERI_CLK_EN_REG gates the RNG clock.
        ptr::write_volatile(rng_clk_en_reg, ptr::read_volatile(rng_clk_en_reg) & !(1 << 0));
    }

    // Disable clock-glitch detection reset.
    unsafe {
        let ana_conf = RTC_CNTL_ANA_CONF_REG as *mut u32;
        ptr::write_volatile(ana_conf, ptr::read_volatile(ana_conf) & !RTC_CNTL_GLITCH_RST_EN);
    }

    let entry_addr = image_data.image.entry_addr;

    // Extract DROM / IROM mapping parameters from segment arrays.
    let (drom_addr, drom_load, drom_size, irom_addr, irom_load, irom_size) =
        extract_segment_info(image_data);

    set_cache_and_start_app(
        drom_addr,
        drom_load,
        drom_size,
        irom_addr,
        irom_load,
        irom_size,
        entry_addr,
    )
}

/// Configure the MMU / cache for DROM and IROM flash segments and jump to
/// the application entry point.
///
/// This function does **not** return.
fn set_cache_and_start_app(
    drom_addr: u32,
    drom_load_addr: u32,
    drom_size: u32,
    irom_addr: u32,
    irom_load_addr: u32,
    irom_size: u32,
    entry_addr: u32,
) -> ! {
    // -- 1. Disable and invalidate caches -----------------------------------
    unsafe {
        Cache_Disable_ICache();
        Cache_Invalidate_ICache_All();
    }

    // -- 2. Initialise MMU (unmap all entries) --------------------------------
    unsafe {
        Cache_MMU_Init();
    }

    // -- 3. Map DROM pages ---------------------------------------------------
    if drom_size > 0 && drom_addr != 0 {
        let drom_paddr = drom_addr & !(PAGE_SIZE - 1);
        let drom_vaddr = drom_load_addr & !(PAGE_SIZE - 1);
        let vaddr_offset = drom_load_addr - drom_vaddr;
        let drom_pages = ((drom_size + vaddr_offset + PAGE_SIZE - 1) / PAGE_SIZE) as i32;

        unsafe {
            // psize=64 is the ROM's constant meaning "64 KB pages"
            Cache_Dbus_MMU_Set(0, drom_vaddr, drom_paddr, 64, drom_pages, 0);
        }
    }

    // -- 4. Map IROM pages ---------------------------------------------------
    if irom_size > 0 && irom_addr != 0 {
        let irom_paddr = irom_addr & !(PAGE_SIZE - 1);
        let irom_vaddr = irom_load_addr & !(PAGE_SIZE - 1);
        let vaddr_offset = irom_load_addr - irom_vaddr;
        let irom_pages = ((irom_size + vaddr_offset + PAGE_SIZE - 1) / PAGE_SIZE) as i32;

        unsafe {
            Cache_Ibus_MMU_Set(0, irom_vaddr, irom_paddr, 64, irom_pages, 0);
        }
    }

    // -- 5. Resume caches and set mode --------------------------------------
    unsafe {
        Cache_Enable_ICache(0); // 0 = no autoload
    }

    // -- 6. De-init console (flush UART) ------------------------------------
    bootloader_atexit();

    // -- 7. Jump to the application entry point -----------------------------
    unsafe {
        core::arch::asm!("jr {addr}", addr = in(reg) entry_addr, options(noreturn));
    }
}

// ===========================================================================
// Reset / cleanup
// ===========================================================================

/// De-initialise the bootloader console (flush UART FIFO).
pub fn bootloader_atexit() {
    unsafe {
        esp_rom_output_tx_wait_idle(0);
    }
}

/// Reset the chip.
pub fn bootloader_reset() -> ! {
    crate::init::debug_tx_str(b"[R]");
    bootloader_atexit();
    unsafe {
        esp_rom_delay_us(1000);
        esp_rom_software_reset_system();
    }
}
