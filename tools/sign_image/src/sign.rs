use p256::ecdsa::{Signature, signature::Signer};
use sha2::{Sha256, Digest};
use std::path::Path;
use crate::sig_block::{SigBlock, image_content_len};

pub fn sign(key_path: &Path, in_path: &Path, out_path: &Path) -> anyhow::Result<()> {
    let signing_key = crate::keygen::load_signing_key(key_path)?;
    let image = std::fs::read(in_path)?;
    let content_len = image_content_len(&image);

    let digest = compute_digest(&image[..content_len]);

    let sig: Signature = signing_key.sign(&image[..content_len]);
    let r: [u8; 32] = sig.r().to_bytes().into();
    let s: [u8; 32] = sig.s().to_bytes().into();
    let mut signature = [0u8; 64];
    signature[..32].copy_from_slice(&r);
    signature[32..].copy_from_slice(&s);

    let point = signing_key.verifying_key().to_encoded_point(false);
    let raw = point.as_bytes();
    // raw is 0x04 || x (32) || y (32) = 65 bytes
    let mut pub_key = [0u8; 64];
    pub_key.copy_from_slice(&raw[1..]);

    let block = SigBlock { image_digest: digest, signature, pub_key };

    let mut out = image[..content_len].to_vec();
    out.extend_from_slice(&block.to_bytes());
    std::fs::write(out_path, &out)?;

    println!(
        "Signed image written to {} ({} + 4096 = {} bytes)",
        out_path.display(),
        content_len,
        out.len()
    );
    println!("Image digest: {}", hex::encode(digest));
    Ok(())
}

fn compute_digest(data: &[u8]) -> [u8; 32] {
    let mut h = Sha256::new();
    h.update(data);
    h.finalize().into()
}
