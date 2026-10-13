//! AES-256 ECB decryption/encryption, matching Unreal's FAES3 usage for
//! pak indices, pak entries and IOStore directory indices / container payloads.

use aes::cipher::{BlockDecrypt, BlockEncrypt, KeyInit};
use aes::Aes256;

#[derive(Clone)]
pub struct AesKey {
    cipher: Aes256,
    raw: [u8; 32],
}

impl std::fmt::Debug for AesKey {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "AesKey({})", hex::encode(&self.raw[..4]))
    }
}

impl AesKey {
    /// Build a key from a 64-char hex string or 32 raw bytes (UE-style "0x..." accepted).
    pub fn parse(text: &str) -> Option<Self> {
        let text = text.trim().trim_start_matches("0x").replace(['-', ' '], "");
        let bytes = hex::decode(&text).ok()?;
        if bytes.len() != 32 {
            return None;
        }
        let mut raw = [0u8; 32];
        raw.copy_from_slice(&bytes);
        Some(Self::from_bytes(raw))
    }

    pub fn from_bytes(raw: [u8; 32]) -> Self {
        Self {
            cipher: Aes256::new((&raw).into()),
            raw,
        }
    }

    pub fn raw(&self) -> &[u8; 32] {
        &self.raw
    }

    pub fn decrypt_ecb_in_place(&self, data: &mut [u8]) {
        for block in data.chunks_exact_mut(16) {
            self.cipher.decrypt_block(block.try_into().unwrap());
        }
    }

    pub fn encrypt_ecb_in_place(&self, data: &mut [u8]) {
        for block in data.chunks_exact_mut(16) {
            self.cipher.encrypt_block(block.try_into().unwrap());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrip() {
        let key = AesKey::from_bytes([7u8; 32]);
        let mut data = vec![42u8; 48];
        let orig = data.clone();
        key.encrypt_ecb_in_place(&mut data);
        assert_ne!(orig, data);
        key.decrypt_ecb_in_place(&mut data);
        assert_eq!(orig, data);
    }

    #[test]
    fn parse_hex() {
        let k = AesKey::parse("0x0000000000000000000000000000000000000000000000000000000000000001");
        assert!(k.is_some());
        assert!(AesKey::parse("zz").is_none());
    }
}
