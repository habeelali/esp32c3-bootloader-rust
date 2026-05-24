use p256::ecdsa::{VerifyingKey, Signature, signature::Verifier};
use p256::elliptic_curve::sec1::EncodedPoint;
use sha2::{Sha256, Digest};
use std::path::Path;
use crate::sig_block::{SigBlock, BLOCK_SIZE, MAGIC, VERSION};

pub fn verify(key_path: &Path, in_path: &Path) -> anyhow::Result<()> {
    let signing_key = crate::keygen::load_signing_key(key_path)?;
    let verifying_key = signing_key.verifying_key();

    let image = std::fs::read(in_path)?;
    let block_raw = read_sig_block(&image)?;

    let block = SigBlock::from_bytes(block_raw)
        .ok_or_else(|| anyhow::anyhow!("Invalid signature block (bad magic, version, or CRC)"))?;

    let content_len = image.len() - BLOCK_SIZE;
    let digest: [u8; 32] = {
        let mut h = Sha256::new();
        h.update(&image[..content_len]);
        h.finalize().into()
    };

    if digest != block.image_digest {
        anyhow::bail!(
            "Image digest mismatch\n  computed: {}\n  in block: {}",
            hex::encode(digest),
            hex::encode(block.image_digest)
        );
    }

    // Reconstruct public key: prepend 0x04 (uncompressed point prefix)
    let mut point_bytes = [0u8; 65];
    point_bytes[0] = 0x04;
    point_bytes[1..].copy_from_slice(&block.pub_key);
    let encoded = EncodedPoint::<p256::NistP256>::from_bytes(&point_bytes)
        .map_err(|e| anyhow::anyhow!("Bad public key in block: {}", e))?;
    let block_vk = VerifyingKey::from_encoded_point(&encoded)
        .map_err(|e| anyhow::anyhow!("Bad public key in block: {}", e))?;

    if block_vk != *verifying_key {
        anyhow::bail!("Public key in signature block does not match provided key");
    }

    // r || s → Signature
    let sig = Signature::from_slice(&block.signature)
        .map_err(|e| anyhow::anyhow!("Bad signature encoding: {}", e))?;

    verifying_key
        .verify(&image[..content_len], &sig)
        .map_err(|_| anyhow::anyhow!("Signature verification FAILED"))?;

    println!("Signature OK");
    println!("Image digest: {}", hex::encode(digest));
    Ok(())
}

fn read_sig_block(image: &[u8]) -> anyhow::Result<&[u8; BLOCK_SIZE]> {
    if image.len() < BLOCK_SIZE {
        anyhow::bail!("Image too small to contain a signature block ({} bytes)", image.len());
    }
    let offset = image.len() - BLOCK_SIZE;
    if image[offset] != MAGIC || image[offset + 1] != VERSION {
        anyhow::bail!(
            "No valid signature block at image tail (expected magic={:#04x} ver={:#04x}, got {:#04x} {:#04x})",
            MAGIC, VERSION, image[offset], image[offset + 1]
        );
    }
    Ok(image[offset..].try_into().unwrap())
}
