#![allow(unused_imports)]

//! ESP32-C3 bootloader image format parsing and loading.
//!
//! Ported from the ESP-IDF bootloader image handling (`bootloader_flash`).
//!
//! Handles reading, parsing, verification, and loading of ESP32-C3 application
//! and bootloader images from flash.  The on-flash image layout is:
//!
//! ```text
//!   [EspImageHeader]               24 bytes
//!   [SegmentHeader 0,  8 bytes] [SegmentData 0,  variable]
//!   [SegmentHeader 1,  8 bytes] [SegmentData 1,  variable]
//!   ...
//!   [Checksum byte]
//!   [SHA-256 hash, 32 bytes]               (optional, hash_appended == 1)
//! ```

use crate::flash;
use crate::rom::*;
use crate::soc::*;
use crate::sha256;
use crate::{BootloaderState, PartitionPos};
use core::{mem, ptr, slice};

// ===========================================================================
// Image header (packed – matches the on-flash binary format)
// ===========================================================================

/// ESP32-C3 image header – the first 24 bytes of every bootable image.
#[repr(C, packed)]
pub struct EspImageHeader {
    pub magic: u8,              // 0xE9
    pub segment_count: u8,      // number of segments (max 16)
    pub spi_mode: u8,           // SPI flash mode constant
    pub spi_speed_size: u8,     // high nibble: size, low nibble: speed
    pub entry_addr: u32,        // entry-point address after loading
    pub wp_pin: u8,             // write-protect pin (unused on ESP32-C3)
    pub spi_pin_drv: [u8; 3],   // drive-strength (unused on ESP32-C3)
    pub chip_id: u16,           // target chip ID (ESP32-C3 = 5)
    pub min_chip_rev: u8,       // legacy minimum chip revision
    pub min_chip_rev_full: u16, // minimum chip revision required
    pub max_chip_rev_full: u16, // maximum chip revision allowed
    pub hash_appended: u8,      // 1 when a SHA-256 digest is appended
    pub reserved: [u8; 4],      // padding
}

// ===========================================================================
// Segment header (8 bytes)
// ===========================================================================

/// On-flash segment header preceding each segment's data.
#[repr(C, packed)]
#[derive(Clone, Copy)]
pub struct EspImageSegmentHeader {
    pub load_addr: u32,
    pub data_len: u32,
}

// ===========================================================================
// Constants
// ===========================================================================

/// Magic byte that identifies a valid ESP32 image.
pub const ESP_IMAGE_HEADER_MAGIC: u8 = 0xE9;

/// Length (in bytes) of a SHA-256 digest.
pub const HASH_LEN: usize = 32;

/// Maximum number of segments allowed in a single image.
pub const ESP_IMAGE_MAX_SEGMENTS: u8 = 16;

// --- Image type identifiers ------------------------------------------------
pub const ESP_IMAGE_BOOTLOADER: u32 = 0;
pub const ESP_IMAGE_APPLICATION: u32 = 1;

// --- SPI speed constants ---------------------------------------------------
pub const ESP_IMAGE_SPI_SPEED_DIV_1: u8 = 0; // 80 MHz
pub const ESP_IMAGE_SPI_SPEED_DIV_2: u8 = 1; // 40 MHz
pub const ESP_IMAGE_SPI_SPEED_DIV_3: u8 = 2; // 26.7 MHz
pub const ESP_IMAGE_SPI_SPEED_DIV_4: u8 = 3; // 20 MHz

// --- SPI mode constants ----------------------------------------------------
pub const ESP_IMAGE_SPI_MODE_QIO: u8 = 0;
pub const ESP_IMAGE_SPI_MODE_QOUT: u8 = 1;
pub const ESP_IMAGE_SPI_MODE_DIO: u8 = 2;
pub const ESP_IMAGE_SPI_MODE_FAST_READ: u8 = 3;
pub const ESP_IMAGE_SPI_MODE_SLOW_READ: u8 = 4;

// --- Flash size constants --------------------------------------------------
pub const ESP_IMAGE_FLASH_SIZE_1MB: u8 = 0;
pub const ESP_IMAGE_FLASH_SIZE_2MB: u8 = 1;
pub const ESP_IMAGE_FLASH_SIZE_4MB: u8 = 2;
pub const ESP_IMAGE_FLASH_SIZE_8MB: u8 = 3;
pub const ESP_IMAGE_FLASH_SIZE_16MB: u8 = 4;

// ===========================================================================
// Image metadata – populated during load / verify
// ===========================================================================

/// Full metadata describing a loaded or verified ESP32 image.
///
/// This is the main structure that the bootloader uses to track image state,
/// segment locations, checksum, and hash results.
#[repr(C)]
pub struct EspImageMetadata {
    /// Flash offset where this image starts (the partition offset).
    pub start_addr: u32,
    /// Parsed image header.
    pub image: EspImageHeader,
    /// Segment headers for all segments (indexed by segment number).
    pub segments: [EspImageSegmentHeader; 16],
    /// Flash offset of each segment's data payload.
    pub segment_data: [u32; 16],
    /// Total image length in bytes (header + segments + checksum + optional hash).
    pub image_len: u32,
    /// Verified or computed SHA-256 digest.
    pub image_digest: [u8; 32],
    /// MMU page size for this chip (used for flash-mapping decisions).
    pub mmu_page_size: u32,
    /// Secure version for anti-rollback (not yet enforced).
    pub secure_version: u32,
}

impl Default for EspImageMetadata {
    fn default() -> Self {
        Self::zeroed()
    }
}

impl EspImageMetadata {
    /// Create a zeroed-out placeholder (used by the partition loader).
    pub const fn zeroed() -> Self {
        EspImageMetadata {
            start_addr: 0,
            image: EspImageHeader {
                magic: 0,
                segment_count: 0,
                spi_mode: 0,
                spi_speed_size: 0,
                entry_addr: 0,
                wp_pin: 0,
                spi_pin_drv: [0; 3],
                chip_id: 0,
                min_chip_rev: 0,
                min_chip_rev_full: 0,
                max_chip_rev_full: 0,
                hash_appended: 0,
                reserved: [0; 4],
            },
            segments: [EspImageSegmentHeader {
                load_addr: 0,
                data_len: 0,
            }; 16],
            segment_data: [0u32; 16],
            image_len: 0,
            image_digest: [0u8; 32],
            mmu_page_size: SPI_FLASH_MMU_PAGE_SIZE,
            secure_version: 0,
        }
    }
}

// ===========================================================================
// Compile-time size assertions
// ===========================================================================

const _: () = assert!(mem::size_of::<EspImageHeader>() == 24);
const _: () = assert!(mem::size_of::<EspImageSegmentHeader>() == 8);

extern "C" {
    static _stext: u32;
    static _etext: u32;
    static _data_start: u32;
    static _bss_end: u32;
}

// ===========================================================================
// Helpers: region classification
// ===========================================================================

/// Returns `true` if the target address is in DRAM, IRAM, or RTC memory,
/// i.e. the segment data must be **copied** from flash to RAM.
fn should_load(load_addr: u32) -> bool {
    // DRAM: 0x3FC8_0000 .. 0x3FCE_0000   (384 KB)
    if load_addr >= 0x3FC80000 && load_addr < 0x3FCE0000 {
        return true;
    }
    // IRAM: 0x4037_C000 .. 0x403E_0000   (400 KB)
    if load_addr >= 0x4037C000 && load_addr < 0x403E0000 {
        return true;
    }
    // RTC IRAM (fast RTC memory)
    if load_addr >= SOC_RTC_IRAM_LOW && load_addr < SOC_RTC_IRAM_HIGH {
        return true;
    }
    // RTC DRAM (slow RTC memory)
    if load_addr >= SOC_RTC_DRAM_LOW && load_addr < SOC_RTC_DRAM_HIGH {
        return true;
    }
    false
}

/// Returns `true` if the target address is in IROM or DROM,
/// i.e. the segment is accessed via the flash MMU without copying.
fn should_map(load_addr: u32) -> bool {
    // IROM: 0x4200_0000 .. 0x4400_0000
    if load_addr >= SOC_IROM_LOW && load_addr < SOC_IROM_HIGH {
        return true;
    }
    // DROM: 0x3C00_0000 .. 0x3E00_0000
    if load_addr >= SOC_DROM_LOW && load_addr < SOC_DROM_HIGH {
        return true;
    }
    false
}

// ===========================================================================
// Image-header verification
// ===========================================================================

/// Check the image header for basic validity.
///
/// Fails when:
/// - the magic byte is not `0xE9`,
/// - `segment_count` is zero or exceeds `ESP_IMAGE_MAX_SEGMENTS`,
/// - the chip validity check rejects the image.
fn verify_image_header(_src_addr: u32, header: &EspImageHeader, silent: bool) -> bool {
    if header.magic != ESP_IMAGE_HEADER_MAGIC {
        if !silent {
            // "image at 0x{:08x} has invalid magic byte 0x{:02x}"
        }
        return false;
    }
    if header.segment_count == 0 || header.segment_count > ESP_IMAGE_MAX_SEGMENTS {
        if !silent {
            // "image has invalid segment count {}"
        }
        return false;
    }
    if !bootloader_common_check_chip_validity(header, ESP_IMAGE_APPLICATION) {
        if !silent {
            // "image is not valid for this chip revision"
        }
        return false;
    }
    true
}

// ===========================================================================
// Load-address validation
// ===========================================================================

/// Ensure a loaded segment does not overlap the running bootloader's
/// memory regions or the ROM stack area.
fn verify_load_addresses(
    _segment_index: i32,
    load_addr: u32,
    load_end: u32,
    print_error: bool,
) -> bool {
    let boot_iram_start = unsafe { &_stext as *const u32 as u32 };
    let boot_iram_end = unsafe { &_etext as *const u32 as u32 };

    if load_addr < boot_iram_end && load_end > boot_iram_start {
        if print_error {
            // "segment {} at 0x{:08x} overlaps bootloader IRAM"
        }
        return false;
    }

    let boot_dram_start = unsafe { &_data_start as *const u32 as u32 };
    let boot_dram_end = unsafe { &_bss_end as *const u32 as u32 };

    if load_addr < boot_dram_end && load_end > boot_dram_start {
        if print_error {
            // "segment {} at 0x{:08x} overlaps bootloader DRAM"
        }
        return false;
    }

    // Stack headroom: keep at least 32 KB away from SOC_ROM_STACK_START.
    // The stack grows downward from SOC_ROM_STACK_START.
    let headroom = 0x8000u32;
    if load_addr + headroom > SOC_ROM_STACK_START
        && load_addr < SOC_ROM_STACK_START + headroom
    {
        if print_error {
            // "segment {} at 0x{:08x} is too close to the stack"
        }
        return false;
    }

    true
}

// ===========================================================================
// SHA-256 helper: release a context (free pool slot) on error
// ===========================================================================

/// Abort a SHA-256 computation and return the context to the pool.
///
/// Safe to call with `None`.
fn sha_abort(handle: sha256::Sha256Handle) {
    if let Some(ctx) = handle {
        // Finalize with a dummy buffer; `bootloader_sha256_finish` also
        // returns the context to the static pool.
        let mut dummy = [0u8; 32];
        sha256::bootloader_sha256_finish(Some(ctx), dummy.as_mut_ptr());
    }
}

// ===========================================================================
// Segment-data processing
// ===========================================================================

/// Map segment data from flash, update the running XOR checksum, optionally
/// copy the data to its load address, and feed the data to the SHA-256
/// engine when verification is required.
///
/// # Returns
///
/// `true` on success.
fn process_segment_data(
    _segment: i32,
    load_addr: u32,
    data_addr: u32,
    data_len: u32,
    do_load: bool,
    sha_handle: sha256::Sha256Handle,
    checksum: &mut u8,
    _metadata: &mut EspImageMetadata,
) -> bool {
    if data_len == 0 {
        return true;
    }

    // Map the segment data from flash via the bounce-buffer.
    let data_ptr = flash::bootloader_mmap(data_addr, data_len);
    if data_ptr.is_null() {
        return false;
    }

    let data = unsafe { slice::from_raw_parts(data_ptr, data_len as usize) };

    // ESP image checksums are byte-wise XORs over segment payload bytes.
    for &byte in data {
        *checksum ^= byte;
    }

    // --- SHA-256 feeding ---------------------------------------------------
    if sha_handle.is_some() {
        sha256::bootloader_sha256_data(sha_handle, data.as_ptr(), data.len());
    }

    // --- Copy to target RAM (if do_load) ----------------------------------
    if do_load {
        unsafe {
            ptr::copy_nonoverlapping(data.as_ptr(), load_addr as *mut u8, data.len());
        }
    }

    flash::bootloader_munmap(data_ptr);
    true
}

// ===========================================================================
// Internal image processing (shared by all public entry points)
// ===========================================================================

/// Flags controlling the behaviour of `process_image`.
struct ProcessFlags {
    verify: bool, // perform checksum + SHA-256 verification
    load: bool,   // copy / map segment data to target addresses
    silent: bool, // suppress diagnostic output
}

/// Core image-processing loop.
///
/// Steps:
/// 1. Read and validate the image header.
/// 2. If `hash_appended` and `verify`, initialise SHA-256 and hash the header.
/// 3. Walk each segment: read header, determine load-vs-map, process data.
/// 4. Read the trailing checksum byte, verify it.
/// 5. If SHA-256 was appended, finalise and compare.
fn process_image(
    part: &PartitionPos,
    data: &mut EspImageMetadata,
    flags: ProcessFlags,
) -> bool {
    let header_size = mem::size_of::<EspImageHeader>() as u32;
    let seg_header_size = mem::size_of::<EspImageSegmentHeader>() as u32;
    let mut checksum: u8 = 0xEF;
    let silent = flags.silent;

    // ---- Step 1: read and verify the image header --------------------------
    let hdr_ptr = flash::bootloader_mmap(part.offset, header_size);
    if hdr_ptr.is_null() {
        crate::init::debug_tx_str(b"[h]");
        if !silent {
            // "failed to mmap image header at 0x{:08x}"
        }
        return false;
    }
    // Copy header bytes for SHA before moving the struct.
    let hdr_raw = unsafe { slice::from_raw_parts(hdr_ptr, header_size as usize) };
    data.image = unsafe { ptr::read_unaligned(hdr_ptr.cast()) };
    flash::bootloader_munmap(hdr_ptr);

    if !verify_image_header(part.offset, &data.image, silent) {
        crate::init::debug_tx_str(b"[H]");
        return false;
    }

    // ---- Step 2: initialise SHA-256 if needed ------------------------------
    let sha_needed = flags.verify && data.image.hash_appended != 0;
    let mut sha_handle: sha256::Sha256Handle = None;
    if sha_needed {
        sha_handle = sha256::bootloader_sha256_start();
        if sha_handle.is_some() {
            sha256::bootloader_sha256_data(sha_handle, hdr_raw.as_ptr(), hdr_raw.len());
        }
    }

    data.start_addr = part.offset;
    data.image_len = header_size;
    let mut next_addr = part.offset + header_size;

    // ---- Step 3: process each segment --------------------------------------
    for i in 0..data.image.segment_count as usize {
        // Read segment header.
        let sh_ptr = flash::bootloader_mmap(next_addr, seg_header_size);
        if sh_ptr.is_null() {
            crate::init::debug_tx_str(b"[s]");
            if sha_needed {
                sha_abort(sha_handle);
            }
            if !silent {
                // "failed to mmap segment header {}"
            }
            return false;
        }
        let seg_raw = unsafe { slice::from_raw_parts(sh_ptr, seg_header_size as usize) };
        data.segments[i] = unsafe { ptr::read_unaligned(sh_ptr.cast()) };
        flash::bootloader_munmap(sh_ptr);

        // Hash the segment header.
        if sha_needed && sha_handle.is_some() {
            sha256::bootloader_sha256_data(sha_handle, seg_raw.as_ptr(), seg_raw.len());
        }

        let load_addr = data.segments[i].load_addr;
        let seg_data_len = data.segments[i].data_len;

        // Record the flash offset of this segment's data payload.
        data.segment_data[i] = next_addr + seg_header_size;

        let do_load = flags.load && should_load(load_addr);
        let _do_map = flags.load && should_map(load_addr);

        // Verify load addresses for segments that will be copied to RAM.
        if do_load {
            let load_end = load_addr.wrapping_add(seg_data_len);
            if !verify_load_addresses(i as i32, load_addr, load_end, !silent) {
                crate::init::debug_tx_str(b"[v]");
                if sha_needed {
                    sha_abort(sha_handle);
                }
                return false;
            }
        }

        // Process (and optionally copy) the segment data.
        if !process_segment_data(
            i as i32,
            load_addr,
            data.segment_data[i],
            seg_data_len,
            do_load,
            sha_handle,
            &mut checksum,
            data,
        ) {
            crate::init::debug_tx_str(b"[d]");
            if sha_needed {
                sha_abort(sha_handle);
            }
            if !silent {
                // "failed to process segment {} data"
            }
            return false;
        }

        // Advance past this segment.
        let seg_total = seg_header_size + seg_data_len;
        next_addr = next_addr.wrapping_add(seg_total);
        data.image_len = data.image_len.wrapping_add(seg_total);
    }

    // ---- Step 4: read and verify the checksum byte -------------------------
    // ESP images place the checksum as the last byte of the 16-byte-aligned
    // image, padding with zeroes after the final segment data as needed.
    let checksum_addr = ((next_addr + 16) & !0x0f).wrapping_sub(1);
    let ck_ptr = flash::bootloader_mmap(checksum_addr, 1);
    if ck_ptr.is_null() {
        crate::init::debug_tx_str(b"[c]");
        if sha_needed {
            sha_abort(sha_handle);
        }
        if !silent {
            // "failed to mmap checksum byte"
        }
        return false;
    }
    let checksum_byte = unsafe { ptr::read_unaligned(ck_ptr.cast::<u8>()) };
    flash::bootloader_munmap(ck_ptr);

    // Feed the checksum byte into the hash.
    if sha_needed && sha_handle.is_some() {
        sha256::bootloader_sha256_data(sha_handle, &checksum_byte as *const u8, 1);
    }

    data.image_len = checksum_addr.wrapping_sub(part.offset).wrapping_add(1);
    next_addr = checksum_addr.wrapping_add(1);

    if flags.verify && checksum != checksum_byte {
        crate::init::debug_tx_str(b"[C]");
        if sha_needed {
            sha_abort(sha_handle);
        }
        if !silent {
            // "checksum mismatch: computed 0x{:02x}, stored 0x{:02x}"
        }
        return false;
    }

    // ---- Step 5: verify SHA-256 hash if appended ---------------------------
    if sha_needed {
        // Read the appended hash.
        let hash_ptr = flash::bootloader_mmap(next_addr, HASH_LEN as u32);
        if hash_ptr.is_null() {
            crate::init::debug_tx_str(b"[g]");
            sha_abort(sha_handle);
            if !silent {
                // "failed to mmap appended SHA-256 hash"
            }
            return false;
        }
        let stored_hash = unsafe { slice::from_raw_parts(hash_ptr, HASH_LEN) };
        flash::bootloader_munmap(hash_ptr);

        // Finalise the computation.
        let mut computed_hash = [0u8; HASH_LEN];
        sha256::bootloader_sha256_finish(sha_handle, computed_hash.as_mut_ptr());

        // Compare.
        if computed_hash != stored_hash {
            crate::init::debug_tx_str(b"[G]");
            if !silent {
                // "SHA-256 hash mismatch"
            }
            return false;
        }

        data.image_digest.copy_from_slice(&computed_hash);
        data.image_len = data.image_len.wrapping_add(HASH_LEN as u32);

        #[cfg(feature = "secure_boot")]
        if flags.verify {
            // sig block immediately follows the SHA-256 hash in flash
            let sig_block_addr = next_addr.wrapping_add(HASH_LEN as u32);
            if unsafe { crate::rom::ets_efuse_secure_boot_enabled() }
                && !verify_secure_boot_sig(&computed_hash, sig_block_addr)
            {
                crate::init::debug_tx_str(b"[S]");
                return false;
            }
        }
    }

    true
}

// ===========================================================================
// Secure Boot V2 — ECDSA P-256 signature verification
// ===========================================================================

#[cfg(feature = "secure_boot")]
fn verify_secure_boot_sig(image_digest: &[u8; HASH_LEN], sig_flash_addr: u32) -> bool {
    use p256::ecdsa::{signature::hazmat::PrehashVerifier, Signature, VerifyingKey};

    let ptr = flash::bootloader_mmap(sig_flash_addr, 4096);
    if ptr.is_null() {
        return false;
    }
    let block = unsafe { slice::from_raw_parts(ptr, 4096) };

    // magic + version
    let hdr_ok = block[0] == 0xE7 && block[1] == 0x02;
    // CRC32 over [0..164) matches crc32fast::hash (ISO-HDLC with final XOR)
    let stored_crc = u32::from_le_bytes([block[164], block[165], block[166], block[167]]);
    let computed_crc = sha256::crc32_bytes(&block[..164]);
    let crc_ok = stored_crc == computed_crc;
    // image_digest in block must match our computed hash
    let digest_ok = &block[4..36] == image_digest;

    // r || s (64 bytes at offset 36)
    let mut sig_bytes = [0u8; 64];
    sig_bytes.copy_from_slice(&block[36..100]);

    flash::bootloader_munmap(ptr);

    if !hdr_ok || !crc_ok || !digest_ok {
        return false;
    }

    // Build uncompressed SEC1 point: 0x04 || x (32 B) || y (32 B)
    let mut sec1 = [0u8; 65];
    sec1[0] = 0x04;
    sec1[1..33].copy_from_slice(&crate::secure_boot_key::PUB_KEY_X);
    sec1[33..65].copy_from_slice(&crate::secure_boot_key::PUB_KEY_Y);

    let vk = match VerifyingKey::from_sec1_bytes(&sec1) {
        Ok(k) => k,
        Err(_) => return false,
    };
    let sig = match Signature::from_bytes((&sig_bytes).into()) {
        Ok(s) => s,
        Err(_) => return false,
    };
    vk.verify_prehash(image_digest, &sig).is_ok()
}

// ===========================================================================
// Public API
// ===========================================================================

/// Load **and** verify an image from flash.
///
/// Validates the image header, segment layout, XOR checksum, and optional
/// SHA-256 hash.  Segments targeting DRAM / IRAM / RTC memory are copied
/// into place.
///
/// Returns `true` on success.
pub fn bootloader_load_image(part: &PartitionPos, data: &mut EspImageMetadata) -> bool {
    process_image(
        part,
        data,
        ProcessFlags {
            verify: true,
            load: true,
            silent: false,
        },
    )
}

/// Load an image without verification.
///
/// Segments are still copied into their target regions, but checksum and
/// SHA-256 checking are skipped.
pub fn bootloader_load_image_no_verify(
    part: &PartitionPos,
    data: &mut EspImageMetadata,
) -> bool {
    process_image(
        part,
        data,
        ProcessFlags {
            verify: false,
            load: true,
            silent: false,
        },
    )
}

/// Verify an image without loading it into RAM.
///
/// `mode` controls error output:
/// - `0` (`ESP_IMAGE_VERIFY`): print errors
/// - non-zero (`ESP_IMAGE_VERIFY_SILENT`): suppress errors
pub fn esp_image_verify(
    mode: u32,
    part: &PartitionPos,
    data: &mut EspImageMetadata,
) -> bool {
    let silent = mode != 0;
    process_image(
        part,
        data,
        ProcessFlags {
            verify: true,
            load: false,
            silent,
        },
    )
}

/// Read image metadata without loading or verifying.
///
/// Populates the header and segment header arrays.  No checksum or
/// SHA-256 verification is performed, and no data is copied to RAM.
pub fn esp_image_get_metadata(
    part: &PartitionPos,
    data: &mut EspImageMetadata,
) -> bool {
    process_image(
        part,
        data,
        ProcessFlags {
            verify: false,
            load: false,
            silent: true,
        },
    )
}

/// Convert a flash-size enum value to bytes.
///
/// | Enum | Size   |
/// |------|--------|
/// | 0    |  1 MB  |
/// | 1    |  2 MB  |
/// | 2    |  4 MB  |
/// | 3    |  8 MB  |
/// | 4    | 16 MB  |
///
/// Unrecognised values default to 2 MB.
pub fn esp_image_get_flash_size(app_flash_size: u8) -> u32 {
    match app_flash_size {
        ESP_IMAGE_FLASH_SIZE_1MB => 1 * 1024 * 1024,
        ESP_IMAGE_FLASH_SIZE_2MB => 2 * 1024 * 1024,
        ESP_IMAGE_FLASH_SIZE_4MB => 4 * 1024 * 1024,
        ESP_IMAGE_FLASH_SIZE_8MB => 8 * 1024 * 1024,
        ESP_IMAGE_FLASH_SIZE_16MB => 16 * 1024 * 1024,
        _ => 2 * 1024 * 1024,
    }
}

/// Check that the image is valid for this chip.
///
/// Verifies the chip ID from eFuse against the expected ESP32-C3 chip ID (5),
/// then checks that the chip revision is within the range specified in the
/// image header (min_rev .. max_rev).  For application images the max revision
/// check is enforced; for bootloader images it is skipped.
pub fn bootloader_common_check_chip_validity(header: &EspImageHeader, image_type: u32) -> bool {
    if header.chip_id != CONFIG_IDF_FIRMWARE_CHIP_ID as u16 {
        return false;
    }

    let revision = efuse_hal_chip_revision();
    let min_rev = header.min_chip_rev_full;

    if !ESP_CHIP_REV_ABOVE(revision, min_rev as u32) {
        return false;
    }

    // Max revision check only for app images (not bootloader)
    if image_type == ESP_IMAGE_APPLICATION {
        let max_rev = header.max_chip_rev_full as u32;
        let is_field_set = max_rev != 65535 && max_rev != 0;
        if is_field_set && revision > max_rev && !efuse_hal_get_disable_wafer_version_major() {
            return false;
        }
    }

    true
}
