//! ESP32-C3 partition-table module.
//!
//! Parses and validates the flash partition table located at offset 0x8000.
//! Populates a `BootloaderState` with the available factory, test, OTA, and
//! OTA-info partitions.
//!
//! The table is an array of up to 95 entries, each 32 bytes.  An entry whose
//! `magic` field is not `0x50AA` marks the end of the table.

#[allow(unused_imports)]
use core::{cmp, mem, ptr, slice};

use crate::flash;
#[allow(unused_imports)]
use crate::image;
#[allow(unused_imports)]
use crate::rom::*;
#[allow(unused_imports)]
use crate::sha256;
use crate::soc::*;
use crate::{BootloaderState, PartitionPos};

// ---------------------------------------------------------------------------
// Partition-type constants
// ---------------------------------------------------------------------------

pub const PART_TYPE_APP: u8 = 0x00;
pub const PART_TYPE_DATA: u8 = 0x01;
pub const PART_TYPE_BOOTLOADER: u8 = 0x02;
pub const PART_TYPE_PARTITION_TABLE: u8 = 0x03;

// ---------------------------------------------------------------------------
// Partition-subtype constants
// ---------------------------------------------------------------------------

pub const PART_SUBTYPE_FACTORY: u8 = 0x00;
pub const PART_SUBTYPE_TEST: u8 = 0x01;
pub const PART_SUBTYPE_OTA_FLAG: u8 = 0x10;
pub const PART_SUBTYPE_OTA_MASK: u8 = 0x0F;

pub const PART_SUBTYPE_DATA_OTA: u8 = 0x00;
pub const PART_SUBTYPE_DATA_RF: u8 = 0x01;
pub const PART_SUBTYPE_DATA_WIFI: u8 = 0x02;
pub const PART_SUBTYPE_DATA_NVS_KEYS: u8 = 0x04;
pub const PART_SUBTYPE_DATA_EFUSE_EM: u8 = 0x05;

pub const PART_SUBTYPE_BOOTLOADER_PRIMARY: u8 = 0x00;
pub const PART_SUBTYPE_BOOTLOADER_OTA: u8 = 0x01;
pub const PART_SUBTYPE_BOOTLOADER_RECOVERY: u8 = 0x02;

pub const PART_SUBTYPE_PARTITION_TABLE_PRIMARY: u8 = 0x00;
pub const PART_SUBTYPE_PARTITION_TABLE_OTA: u8 = 0x01;

// ---------------------------------------------------------------------------
// Magic
// ---------------------------------------------------------------------------

pub const ESP_PARTITION_MAGIC: u16 = 0x50AA;

// ---------------------------------------------------------------------------
// Limits
// ---------------------------------------------------------------------------

pub const MAX_PARTITIONS: usize = 95;

// ---------------------------------------------------------------------------
// Structures
// ---------------------------------------------------------------------------

/// A single partition-table entry (32 bytes on flash).
#[repr(C, packed)]
pub struct EspPartitionInfo {
    pub magic: u16,        // 0x50AA
    pub type_: u8,
    pub subtype: u8,
    pub pos: PartitionPos, // offset (u32) + size (u32)
    pub label: [u8; 16],
    pub flags: u32,
}

const _: () = assert!(mem::size_of::<EspPartitionInfo>() == 32);

// ---------------------------------------------------------------------------
// Constants for validation
// ---------------------------------------------------------------------------

/// Minimum valid partition offset (after partition table itself).
const MIN_PART_OFFSET: u32 = ESP_PARTITION_TABLE_OFFSET + ESP_PARTITION_TABLE_MAX_LEN;

/// A generous upper bound for flash size (16 MB, the maximum ESP32-C3
/// supports).  Individual entries are also checked to not wrap around.
const MAX_FLASH_SIZE: u32 = 16 * 1024 * 1024;

// ---------------------------------------------------------------------------
// Helper: check that a label (up to 16 bytes) contains only printable ASCII
// or NUL bytes (i.e. it is a valid null-terminated ASCII string).
// ---------------------------------------------------------------------------

fn label_is_valid(label: &[u8; 16]) -> bool {
    let mut seen_nul = false;
    for &b in label.iter() {
        if b == 0 {
            seen_nul = true;
        } else if seen_nul {
            // Non-nul byte after a nul byte is invalid.
            return false;
        } else if !(0x20..=0x7E).contains(&b) {
            // Non-printable character.
            return false;
        }
    }
    true
}

// ---------------------------------------------------------------------------
// Public API
// ---------------------------------------------------------------------------

/// Verify a partition table loaded from flash.
///
/// Iterates through entries until magic != `ESP_PARTITION_MAGIC` or
/// `MAX_PARTITIONS` is reached, validating:
///   - no two partitions overlap,
///   - no partition extends outside a reasonable flash boundary,
///   - labels are valid printable-ASCII / NUL-terminated.
///
/// The total number of valid entries is written to `out_num`.
pub fn esp_partition_table_verify(
    partitions: &[EspPartitionInfo],
    log_errors: bool,
    out_num: &mut i32,
) -> bool {
    let mut count: i32 = 0;
    let mut valid = true;
    let max = cmp::min(partitions.len(), MAX_PARTITIONS);

    for i in 0..max {
        let part = &partitions[i];
        if part.magic != ESP_PARTITION_MAGIC {
            break;
        }
        count += 1;

        // --- basic range checks ---------------------------------------------
        let start = part.pos.offset;
        let size = part.pos.size;
        let end = start.wrapping_add(size);

        if size == 0 || start < MIN_PART_OFFSET || end > MAX_FLASH_SIZE || end <= start {
            if log_errors {
                // We cannot print in no_std easily; the caller is responsible.
            }
            valid = false;
            continue;
        }

        // --- overlap check --------------------------------------------------
        for j in 0..i {
            let prev = &partitions[j];
            if prev.magic != ESP_PARTITION_MAGIC {
                break;
            }
            let p_start = start;
            let p_end = end;
            let prev_start = prev.pos.offset;
            let prev_end = prev.pos.offset.wrapping_add(prev.pos.size);

            // Two regions overlap if each starts before the other ends.
            if p_start < prev_end && p_end > prev_start {
                if log_errors {}
                valid = false;
                break;
            }
        }

        // --- label validation -----------------------------------------------
        if !label_is_valid(&part.label) {
            if log_errors {}
            valid = false;
        }
    }

    *out_num = count;
    valid
}

/// Load the partition table from flash and populate `BootloaderState`.
///
/// Returns `true` on success, `false` on any error (mmap failure, verification
/// failure).
pub fn bootloader_utility_load_partition_table(bs: &mut BootloaderState) -> bool {
    // Map the partition table region from flash.
    let addr = flash::bootloader_mmap(ESP_PARTITION_TABLE_OFFSET, ESP_PARTITION_TABLE_MAX_LEN);
    if addr.is_null() {
        return false;
    }


    let partitions: &[EspPartitionInfo] =
        unsafe { slice::from_raw_parts(addr as *const EspPartitionInfo, MAX_PARTITIONS) };

    let mut num_partitions: i32 = 0;
    if !esp_partition_table_verify(partitions, true, &mut num_partitions) {
        flash::bootloader_munmap(addr);
        return false;
    }

    // Reset bootloader state before populating.
    *bs = BootloaderState::default();

    for i in 0..(num_partitions as usize) {
        let part = &partitions[i];
        match (part.type_, part.subtype) {
            (PART_TYPE_APP, PART_SUBTYPE_FACTORY) => {
                bs.factory = part.pos;
            }
            (PART_TYPE_APP, PART_SUBTYPE_TEST) => {
                bs.test = part.pos;
            }
            (PART_TYPE_APP, st) if st & PART_SUBTYPE_OTA_FLAG == PART_SUBTYPE_OTA_FLAG => {
                let slot = (st & PART_SUBTYPE_OTA_MASK) as usize;
                if slot < MAX_PARTITIONS.min(16) {
                    bs.ota[slot] = part.pos;
                    if (slot as u32) >= bs.app_count {
                        bs.app_count = (slot as u32) + 1;
                    }
                }
            }
            (PART_TYPE_DATA, PART_SUBTYPE_DATA_OTA) => {
                bs.ota_info = part.pos;
            }
            _ => {}
        }
    }

    flash::bootloader_munmap(addr);

    // (Optional) print a summary – omitted here since we have no formatted
    // output, but a real bootloader would log the table.

    true
}
