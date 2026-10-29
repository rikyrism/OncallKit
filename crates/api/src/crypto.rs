//! Secret handling: AES-256-GCM for integration credentials at rest, SHA-256 for tokens we only
//! ever need to compare (sessions, invitations, OAuth state).

use aes_gcm::{
    aead::{Aead, KeyInit},
    Aes256Gcm, Nonce,
};
use anyhow::{anyhow, bail, Context, Result};
use base64::{engine::general_purpose::STANDARD, engine::general_purpose::URL_SAFE_NO_PAD, Engine};
use rand::RngCore;
use sha2::{Digest, Sha256};

pub fn parse_key(encoded: &str) -> Result<[u8; 32]> {
    let bytes = STANDARD.decode(encoded.trim()).or_else(|_| URL_SAFE_NO_PAD.decode(encoded.trim())).context("ENCRYPTION_KEY must be base64")?;
    bytes.try_into().map_err(|_| anyhow!("ENCRYPTION_KEY must decode to exactly 32 bytes"))
}

/// A random URL-safe token with 256 bits of entropy.
pub fn random_token() -> String {
    let mut bytes = [0u8; 32];
    rand::thread_rng().fill_bytes(&mut bytes);
    URL_SAFE_NO_PAD.encode(bytes)
}

pub fn hash_token(token: &str) -> Vec<u8> {
    Sha256::digest(token.as_bytes()).to_vec()
}

pub struct SealedSecret {
    pub ciphertext: Vec<u8>,
    /// `None` means the value was stored without encryption (no key configured, development only).
    pub nonce: Option<Vec<u8>>,
}

#[derive(Clone)]
pub struct SecretBox {
    cipher: Option<Aes256Gcm>,
}

impl SecretBox {
    pub fn new(key: Option<[u8; 32]>) -> Self {
        SecretBox { cipher: key.map(|k| Aes256Gcm::new(&k.into())) }
    }

    pub fn is_encrypting(&self) -> bool {
        self.cipher.is_some()
    }

    pub fn seal(&self, plaintext: &str) -> Result<SealedSecret> {
        match &self.cipher {
            Some(cipher) => {
                let mut nonce = [0u8; 12];
                rand::thread_rng().fill_bytes(&mut nonce);
                let ciphertext = cipher.encrypt(&Nonce::from(nonce), plaintext.as_bytes()).map_err(|_| anyhow!("encryption failed"))?;
                Ok(SealedSecret { ciphertext, nonce: Some(nonce.to_vec()) })
            }
            None => Ok(SealedSecret { ciphertext: plaintext.as_bytes().to_vec(), nonce: None }),
        }
    }

    pub fn open(&self, ciphertext: &[u8], nonce: Option<&[u8]>) -> Result<String> {
        let bytes = match (nonce, &self.cipher) {
            (Some(nonce), Some(cipher)) => {
                let nonce: [u8; 12] = nonce.try_into().map_err(|_| anyhow!("stored nonce has the wrong length"))?;
                cipher.decrypt(&Nonce::from(nonce), ciphertext).map_err(|_| anyhow!("could not decrypt a stored secret; was ENCRYPTION_KEY changed?"))?
            }
            (Some(_), None) => bail!("a stored secret is encrypted but ENCRYPTION_KEY is not configured"),
            (None, _) => ciphertext.to_vec(),
        };
        String::from_utf8(bytes).context("stored secret is not UTF-8")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn seal_and_open_round_trip() {
        let sb = SecretBox::new(Some([3u8; 32]));
        let sealed = sb.seal("https://discord.com/api/webhooks/1/abc").unwrap();
        assert_ne!(sealed.ciphertext, b"https://discord.com/api/webhooks/1/abc");
        assert_eq!(sb.open(&sealed.ciphertext, sealed.nonce.as_deref()).unwrap(), "https://discord.com/api/webhooks/1/abc");
    }

    #[test]
    fn wrong_key_fails() {
        let sealed = SecretBox::new(Some([3u8; 32])).seal("secret").unwrap();
        assert!(SecretBox::new(Some([4u8; 32])).open(&sealed.ciphertext, sealed.nonce.as_deref()).is_err());
        assert!(SecretBox::new(None).open(&sealed.ciphertext, sealed.nonce.as_deref()).is_err());
    }

    #[test]
    fn parse_key_requires_32_bytes() {
        assert!(parse_key(&STANDARD.encode([1u8; 32])).is_ok());
        assert!(parse_key(&STANDARD.encode([1u8; 16])).is_err());
    }

    #[test]
    fn tokens_are_unique_and_hashes_stable() {
        assert_ne!(random_token(), random_token());
        assert_eq!(hash_token("a"), hash_token("a"));
    }
}
