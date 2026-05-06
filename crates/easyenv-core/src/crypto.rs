use aes_gcm::{
    Aes256Gcm, Nonce,
    aead::{Aead, KeyInit},
};
use anyhow::{Context, Result, anyhow, bail};
use zeroize::Zeroizing;

const MAGIC: &[u8; 4] = b"EENV";
const VERSION: u8 = 1;
const NONCE_LEN: usize = 12;
const KEY_LEN: usize = 32;

pub fn generate_key() -> Result<Zeroizing<Vec<u8>>> {
    let mut key = Zeroizing::new(vec![0_u8; KEY_LEN]);
    getrandom::fill(key.as_mut_slice())
        .map_err(|error| anyhow!("failed to generate master key: {error}"))?;
    Ok(key)
}

pub fn encrypt(key: &[u8], plaintext: &[u8]) -> Result<Vec<u8>> {
    if key.len() != KEY_LEN {
        bail!("master key must be 32 bytes")
    }

    let cipher = Aes256Gcm::new_from_slice(key).context("failed to initialize cipher")?;
    let mut nonce_bytes = [0_u8; NONCE_LEN];
    getrandom::fill(&mut nonce_bytes)
        .map_err(|error| anyhow!("failed to generate nonce: {error}"))?;
    let nonce = Nonce::from_slice(&nonce_bytes);
    let ciphertext = cipher
        .encrypt(nonce, plaintext)
        .map_err(|_| anyhow!("failed to encrypt value"))?;

    let mut blob = Vec::with_capacity(MAGIC.len() + 1 + NONCE_LEN + ciphertext.len());
    blob.extend_from_slice(MAGIC);
    blob.push(VERSION);
    blob.extend_from_slice(&nonce_bytes);
    blob.extend_from_slice(&ciphertext);
    Ok(blob)
}

pub fn decrypt(key: &[u8], blob: &[u8]) -> Result<Zeroizing<Vec<u8>>> {
    if key.len() != KEY_LEN {
        bail!("master key must be 32 bytes")
    }

    if blob.len() < MAGIC.len() + 1 + NONCE_LEN {
        bail!("encrypted payload is truncated")
    }

    if &blob[..MAGIC.len()] != MAGIC {
        bail!("encrypted payload header is invalid")
    }

    if blob[MAGIC.len()] != VERSION {
        bail!("unsupported encrypted payload version")
    }

    let nonce_start = MAGIC.len() + 1;
    let nonce_end = nonce_start + NONCE_LEN;
    let nonce = Nonce::from_slice(&blob[nonce_start..nonce_end]);
    let ciphertext = &blob[nonce_end..];
    let cipher = Aes256Gcm::new_from_slice(key).context("failed to initialize cipher")?;
    let plaintext = cipher
        .decrypt(nonce, ciphertext)
        .map_err(|_| anyhow!("failed to decrypt value"))?;
    Ok(Zeroizing::new(plaintext))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrips_payloads() {
        let key = generate_key().unwrap();
        let encrypted = encrypt(key.as_ref(), b"super-secret").unwrap();
        let decrypted = decrypt(key.as_ref(), &encrypted).unwrap();
        assert_eq!(decrypted.as_slice(), b"super-secret");
    }
}
