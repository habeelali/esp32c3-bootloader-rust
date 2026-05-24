/// ESP32-C3 Secure Boot V2 — ECDSA-P256 signature block (4096 bytes total).
///
/// Layout (byte offsets):
///   0     magic        0xE7
///   1     version      0x02 (ECDSA-256)
///   2–3   reserved     0x00 0x00
///   4–35  image_digest SHA-256 of all image bytes before this block
///  36–99  signature    r (32 B) || s (32 B), big-endian
/// 100–163 pub_key      x (32 B) || y (32 B), uncompressed P-256 point (no 0x04 prefix)
/// 164–167 crc32        CRC32-LE over bytes [0..164)
/// 168–4095 padding     0xFF

pub const MAGIC: u8 = 0xE7;
pub const VERSION: u8 = 0x02;
pub const BLOCK_SIZE: usize = 4096;
#[allow(dead_code)]
pub const HEADER_SIZE: usize = 168; // bytes before padding (sig + pubkey + crc)

pub struct SigBlock {
    pub image_digest: [u8; 32],
    pub signature: [u8; 64], // r || s
    pub pub_key: [u8; 64],   // x || y
}

impl SigBlock {
    /// Serialise to a 4096-byte block ready to append to the image.
    pub fn to_bytes(&self) -> [u8; BLOCK_SIZE] {
        let mut block = [0xFFu8; BLOCK_SIZE];

        block[0] = MAGIC;
        block[1] = VERSION;
        block[2] = 0x00;
        block[3] = 0x00;
        block[4..36].copy_from_slice(&self.image_digest);
        block[36..100].copy_from_slice(&self.signature);
        block[100..164].copy_from_slice(&self.pub_key);

        let crc = crc32fast::hash(&block[..164]);
        block[164..168].copy_from_slice(&crc.to_le_bytes());

        block
    }

    /// Parse a 4096-byte block. Returns None if magic, version, or CRC are wrong.
    pub fn from_bytes(raw: &[u8; BLOCK_SIZE]) -> Option<Self> {
        if raw[0] != MAGIC || raw[1] != VERSION {
            return None;
        }

        let stored_crc = u32::from_le_bytes(raw[164..168].try_into().unwrap());
        let computed_crc = crc32fast::hash(&raw[..164]);
        if stored_crc != computed_crc {
            eprintln!(
                "CRC mismatch: stored {:#010x}, computed {:#010x}",
                stored_crc, computed_crc
            );
            return None;
        }

        let mut block = SigBlock {
            image_digest: [0u8; 32],
            signature: [0u8; 64],
            pub_key: [0u8; 64],
        };
        block.image_digest.copy_from_slice(&raw[4..36]);
        block.signature.copy_from_slice(&raw[36..100]);
        block.pub_key.copy_from_slice(&raw[100..164]);
        Some(block)
    }
}

/// Returns the number of image bytes that should be hashed (everything before the sig block).
/// If the binary has a sig block already appended (len % 4096 == 0 last 4096 bytes start with
/// 0xE7), we treat only the prefix. Otherwise hash the whole thing.
pub fn image_content_len(image: &[u8]) -> usize {
    if image.len() >= BLOCK_SIZE {
        let tail_offset = image.len() - BLOCK_SIZE;
        if image[tail_offset] == MAGIC && image[tail_offset + 1] == VERSION {
            return tail_offset;
        }
    }
    image.len()
}
