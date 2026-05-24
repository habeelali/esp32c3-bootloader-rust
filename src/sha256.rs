//! Pure-software SHA-256 implementation for bootloader image verification.
//! Also provides CRC32-Little-Endian for OTA data validation.

/// Software CRC32 (little-endian, polynomial 0xEDB88320).
/// Matches the esp_rom_crc32_le used in ESP-IDF for OTA select entry validation.
/// CRC is accumulated over 8-bit bytes starting from the supplied `crc` value.
/// No final XOR is applied.
pub fn bootloader_crc32_le(mut crc: u32, data: &[u8]) -> u32 {
    for &byte in data {
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

/// CRC32 (ISO-HDLC / zlib) over `data`, matching `crc32fast::hash`.
/// Initial value 0xFFFF_FFFF, polynomial 0xEDB8_8320, final XOR 0xFFFF_FFFF.
pub fn crc32_bytes(data: &[u8]) -> u32 {
    bootloader_crc32_le(0xFFFF_FFFF, data) ^ 0xFFFF_FFFF
}
//
// The remainder of this file provides a pure-software SHA-256 implementation
// with a static pool of two contexts, mirroring the ESP-IDF ROM
// `bootloader_sha256_*` API.

// ---------------------------------------------------------------------------
// SHA-256 algorithm constants
// ---------------------------------------------------------------------------

/// Initial hash values (H0 .. H7).
const H: [u32; 8] = [
    0x6A09_E667,
    0xBB67_AE85,
    0x3C6E_F372,
    0xA54F_F53A,
    0x510E_527F,
    0x9B05_688C,
    0x1F83_D9AB,
    0x5BE0_CD19,
];

/// Round constants (K0 .. K63).
const K: [u32; 64] = [
    0x428A_2F98, 0x7137_4491, 0xB5C0_FBCF, 0xE9B5_DBA5, 0x3956_C25B, 0x59F1_11F1,
    0x923F_82A4, 0xAB1C_5ED5, 0xD807_AA98, 0x1283_5B01, 0x2431_85BE, 0x550C_7DC3,
    0x72BE_5D74, 0x80DE_B1FE, 0x9BDC_06A7, 0xC19B_F174, 0xE49B_69C1, 0xEFBE_4786,
    0x0FC1_9DC6, 0x240C_A1CC, 0x2DE9_2C6F, 0x4A74_84AA, 0x5CB0_A9DC, 0x76F9_88DA,
    0x983E_5152, 0xA831_C66D, 0xB003_27C8, 0xBF59_7FC7, 0xC6E0_0BF3, 0xD5A7_9147,
    0x06CA_6351, 0x1429_2967, 0x27B7_0A85, 0x2E1B_2138, 0x4D2C_6DFC, 0x5338_0D13,
    0x650A_7354, 0x766A_0ABB, 0x81C2_C92E, 0x9272_2C85, 0xA2BF_E8A1, 0xA81A_664B,
    0xC24B_8B70, 0xC76C_51A3, 0xD192_E819, 0xD699_0624, 0xF40E_3585, 0x106A_A070,
    0x19A4_C116, 0x1E37_6C08, 0x2748_774C, 0x34B0_BCB5, 0x391C_0CB3, 0x4ED8_AA4A,
    0x5B9C_CA4F, 0x682E_6FF3, 0x748F_82EE, 0x78A5_636F, 0x84C8_7814, 0x8CC7_0208,
    0x90BE_FFFA, 0xA450_6CEB, 0xBEF9_A3F7, 0xC671_78F2,
];

// ---------------------------------------------------------------------------
// Elementary SHA-256 operations
// ---------------------------------------------------------------------------

#[inline(always)]
fn ch(x: u32, y: u32, z: u32) -> u32 {
    (x & y) ^ (!x & z)
}

#[inline(always)]
fn maj(x: u32, y: u32, z: u32) -> u32 {
    (x & y) ^ (x & z) ^ (y & z)
}

#[inline(always)]
fn sigma0(x: u32) -> u32 {
    x.rotate_right(2) ^ x.rotate_right(13) ^ x.rotate_right(22)
}

#[inline(always)]
fn sigma1(x: u32) -> u32 {
    x.rotate_right(6) ^ x.rotate_right(11) ^ x.rotate_right(25)
}

#[inline(always)]
fn gamma0(x: u32) -> u32 {
    x.rotate_right(7) ^ x.rotate_right(18) ^ (x >> 3)
}

#[inline(always)]
fn gamma1(x: u32) -> u32 {
    x.rotate_right(17) ^ x.rotate_right(19) ^ (x >> 10)
}

// ---------------------------------------------------------------------------
// Context type and pool
// ---------------------------------------------------------------------------

/// SHA-256 computation context.
#[repr(C)]
pub struct Sha256Context {
    state: [u32; 8],   // current hash value H0..H7
    buffer: [u8; 64],  // 512-bit block buffer
    count: u64,        // total bits hashed so far
    index: usize,      // number of bytes currently in buffer
}

impl Sha256Context {
    /// Create a fresh context initialised to the SHA-256 start state.
    const fn new() -> Self {
        Sha256Context {
            state: H,
            buffer: [0u8; 64],
            count: 0,
            index: 0,
        }
    }

    /// Feed one 512-bit block (64 bytes) into the compressor.
    fn compress(&mut self, block: &[u8; 64]) {
        // Message schedule
        let mut w = [0u32; 64];

        for t in 0..16 {
            let i = t * 4;
            w[t] = u32::from_be_bytes([block[i], block[i + 1], block[i + 2], block[i + 3]]);
        }
        for t in 16..64 {
            w[t] = gamma1(w[t - 2])
                .wrapping_add(w[t - 7])
                .wrapping_add(gamma0(w[t - 15]))
                .wrapping_add(w[t - 16]);
        }

        // Working variables
        let mut a = self.state[0];
        let mut b = self.state[1];
        let mut c = self.state[2];
        let mut d = self.state[3];
        let mut e = self.state[4];
        let mut f = self.state[5];
        let mut g = self.state[6];
        let mut h = self.state[7];

        // Compression rounds
        for t in 0..64 {
            let t1 = h
                .wrapping_add(sigma1(e))
                .wrapping_add(ch(e, f, g))
                .wrapping_add(K[t])
                .wrapping_add(w[t]);
            let t2 = sigma0(a).wrapping_add(maj(a, b, c));

            h = g;
            g = f;
            f = e;
            e = d.wrapping_add(t1);
            d = c;
            c = b;
            b = a;
            a = t1.wrapping_add(t2);
        }

        // Add compressed result to state
        self.state[0] = self.state[0].wrapping_add(a);
        self.state[1] = self.state[1].wrapping_add(b);
        self.state[2] = self.state[2].wrapping_add(c);
        self.state[3] = self.state[3].wrapping_add(d);
        self.state[4] = self.state[4].wrapping_add(e);
        self.state[5] = self.state[5].wrapping_add(f);
        self.state[6] = self.state[6].wrapping_add(g);
        self.state[7] = self.state[7].wrapping_add(h);
    }

    /// Absorb input bytes, buffering and compressing full blocks.
    fn update(&mut self, data: *const u8, len: usize) {
        if data.is_null() || len == 0 {
            return;
        }

        self.count = self.count.wrapping_add((len as u64) * 8);

        let mut offset = 0usize;
        let mut remaining = len;

        // If there are bytes already buffered, fill the buffer first
        if self.index > 0 && self.index + remaining >= 64 {
            let copy = 64 - self.index;
            unsafe {
                core::ptr::copy_nonoverlapping(data, self.buffer.as_mut_ptr().add(self.index), copy);
            }
            let block = self.buffer;
            self.compress(&block);
            offset += copy;
            remaining -= copy;
            self.index = 0;
        }

        // Compress full 64-byte blocks directly from the input
        while remaining >= 64 {
            let block = unsafe { &*(data.add(offset) as *const [u8; 64]) };
            self.compress(block);
            offset += 64;
            remaining -= 64;
        }

        // Buffer remaining bytes
        if remaining > 0 {
            unsafe {
                core::ptr::copy_nonoverlapping(data.add(offset), self.buffer.as_mut_ptr().add(self.index), remaining);
            }
            self.index += remaining;
        }
    }

    /// Finalise the hash and write the 32-byte digest into `digest`.
    fn finalize(&mut self, digest: *mut u8) {
        // Append the 0x80 padding byte
        self.buffer[self.index] = 0x80;
        self.index += 1;

        // If the remaining space in buffer is less than 8 bytes (for the
        // 64-bit bit count), pad with zeros and compress the current block.
        if self.index > 56 {
            // Zero out the rest of the buffer
            for i in self.index..64 {
                self.buffer[i] = 0;
            }
            let block = self.buffer;
            self.compress(&block);
            self.index = 0;
        }

        // Zero out remaining buffer up to byte 56
        for i in self.index..56 {
            self.buffer[i] = 0;
        }

        // Write the 64-bit bit count as big-endian in the last 8 bytes
        let bits = self.count;
        self.buffer[56..64].copy_from_slice(&bits.to_be_bytes());

        // Compress the final (padded) block
        let block = self.buffer;
        self.compress(&block);

        // Extract the digest in big-endian byte order
        if !digest.is_null() {
            for i in 0..8 {
                let bytes = self.state[i].to_be_bytes();
                unsafe {
                    core::ptr::copy_nonoverlapping(
                        bytes.as_ptr(),
                        digest.add(i * 4),
                        4,
                    );
                }
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Static context pool
// ---------------------------------------------------------------------------

/// Number of available SHA-256 contexts.
const POOL_SIZE: usize = 2;

static mut CTX_POOL: [Sha256Context; POOL_SIZE] =
    [Sha256Context::new(), Sha256Context::new()];

static mut CTX_USED: [bool; POOL_SIZE] = [false, false];

// ---------------------------------------------------------------------------
// Handle type and public API
// ---------------------------------------------------------------------------

/// Opaque handle to a SHA-256 context.
///
/// `None` indicates that no context was available.
pub type Sha256Handle = Option<*mut Sha256Context>;

/// Start a SHA-256 computation.
///
/// Returns a handle to a context from the static pool, or `None` if all
/// contexts are in use.
pub fn bootloader_sha256_start() -> Sha256Handle {
    unsafe {
        for i in 0..POOL_SIZE {
            if !CTX_USED[i] {
                CTX_USED[i] = true;
                CTX_POOL[i] = Sha256Context::new();
                return Some(&mut CTX_POOL[i] as *mut Sha256Context);
            }
        }
    }
    None
}

/// Feed data into an active SHA-256 computation.
///
/// # Panics
///
/// Panics (in debug builds) if `handle` is `None`.  In release builds the
/// call is a no-op for a `None` handle.
pub fn bootloader_sha256_data(handle: Sha256Handle, data: *const u8, len: usize) {
    let ctx = match handle {
        Some(p) => unsafe { &mut *p },
        None => return,
    };
    ctx.update(data, len);
}

/// Finalise a SHA-256 computation and write the 32-byte digest.
///
/// The context is freed (returned to the pool) after this call.
///
/// # Panics
///
/// Panics (in debug builds) if `handle` is `None`.  In release builds the
/// call is a no-op for a `None` handle.
pub fn bootloader_sha256_finish(handle: Sha256Handle, digest: *mut u8) {
    let ctx = match handle {
        Some(p) => unsafe { &mut *p },
        None => return,
    };
    ctx.finalize(digest);

    // Return the context to the pool
    let ctx_addr = ctx as *mut Sha256Context as usize;
    unsafe {
        for i in 0..POOL_SIZE {
            if (&mut CTX_POOL[i] as *mut Sha256Context as usize) == ctx_addr {
                CTX_USED[i] = false;
                break;
            }
        }
    }
}
