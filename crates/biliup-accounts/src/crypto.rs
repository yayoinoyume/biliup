use crate::AccountError;
use ring::aead;
use std::io;
use zeroize::Zeroize;

pub const KEY_FILE_ENV: &str = "BILIUP_ACCOUNT_KEY_FILE";

pub(crate) const MAGIC: &[u8; 7] = b"BUPACC1";
pub(crate) const NONCE_LEN: usize = 12;
pub(crate) const TAG_LEN: usize = 16;
pub(crate) const HEADER_LEN: usize = MAGIC.len() + NONCE_LEN;

const AAD: &[u8] = b"biliup-accounts/v1";

pub(crate) struct Key([u8; 32]);

impl Key {
    pub(crate) fn from_bytes(bytes: [u8; 32]) -> Self {
        Self(bytes)
    }

    fn cipher(&self) -> io::Result<aead::LessSafeKey> {
        aead::UnboundKey::new(&aead::AES_256_GCM, &self.0)
            .map(aead::LessSafeKey::new)
            .map_err(|_| AccountError::Damaged.into_io())
    }
}

impl Drop for Key {
    fn drop(&mut self) {
        self.0.zeroize();
    }
}

pub(crate) fn random<const N: usize>() -> io::Result<[u8; N]> {
    let mut bytes = [0; N];
    getrandom::fill(&mut bytes).map_err(|_| AccountError::Random.into_io())?;
    Ok(bytes)
}

pub(crate) fn hex(bytes: &[u8]) -> String {
    use std::fmt::Write;
    bytes.iter().fold(String::new(), |mut out, byte| {
        let _ = write!(out, "{byte:02x}");
        out
    })
}

pub(crate) fn unhex(text: &str) -> Option<[u8; 32]> {
    if text.len() != 64 || !text.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return None;
    }
    let mut out = [0u8; 32];
    for (index, slot) in out.iter_mut().enumerate() {
        *slot = u8::from_str_radix(&text[index * 2..index * 2 + 2], 16).ok()?;
    }
    Some(out)
}

pub(crate) fn seal(key: &Key, plaintext: &[u8]) -> io::Result<Vec<u8>> {
    let nonce = random::<NONCE_LEN>()?;
    let mut payload = plaintext.to_vec();
    key.cipher()?
        .seal_in_place_append_tag(
            aead::Nonce::assume_unique_for_key(nonce),
            aead::Aad::from(AAD),
            &mut payload,
        )
        .map_err(|_| AccountError::Damaged.into_io())?;
    let mut sealed = Vec::with_capacity(HEADER_LEN + payload.len());
    sealed.extend_from_slice(MAGIC);
    sealed.extend_from_slice(&nonce);
    sealed.extend_from_slice(&payload);
    payload.zeroize();
    Ok(sealed)
}

pub(crate) fn open(key: &Key, sealed: &[u8]) -> io::Result<Vec<u8>> {
    if sealed.len() < HEADER_LEN + TAG_LEN || &sealed[..MAGIC.len()] != MAGIC {
        return Err(AccountError::Damaged.into_io());
    }
    let nonce: [u8; NONCE_LEN] = sealed[MAGIC.len()..HEADER_LEN]
        .try_into()
        .map_err(|_| AccountError::Damaged.into_io())?;
    let mut payload = sealed[HEADER_LEN..].to_vec();
    let plain = key
        .cipher()?
        .open_in_place(
            aead::Nonce::assume_unique_for_key(nonce),
            aead::Aad::from(AAD),
            &mut payload,
        )
        .map_err(|_| AccountError::Damaged.into_io())?
        .to_vec();
    payload.zeroize();
    Ok(plain)
}
