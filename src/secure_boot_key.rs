// Compile-time P-256 public key used for Secure Boot image verification.
// Replace with the output of: sign-image keygen --out key.pem --out-rust src/secure_boot_key.rs
pub const PUB_KEY_X: [u8; 32] = [0u8; 32];
pub const PUB_KEY_Y: [u8; 32] = [0u8; 32];
