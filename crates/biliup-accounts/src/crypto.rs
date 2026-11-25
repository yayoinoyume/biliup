use crate::AccountError;
use ring::aead;
use std::fs;
use std::io::{self, Read};
use std::path::Path;
use zeroize::Zeroize;

pub const KEY_FILE_ENV: &str = "BILIUP_ACCOUNT_KEY_FILE";

const KEY_TEXT_LEN: usize = 64;
const KEY_FILE_MAX_BYTES: u64 = 4096;

pub(crate) const MAGIC: &[u8; 7] = b"BUPACC1";
pub(crate) const NONCE_LEN: usize = 12;
pub(crate) const TAG_LEN: usize = 16;
pub(crate) const HEADER_LEN: usize = MAGIC.len() + NONCE_LEN;

const AAD: &[u8] = b"biliup-accounts/v1";

pub struct Cipher {
    key: Key,
}

impl Cipher {
    pub fn from_key_file(path: &Path) -> io::Result<Self> {
        read_key(path).map(|key| Self { key })
    }

    pub fn from_env() -> io::Result<Option<Self>> {
        match std::env::var_os(KEY_FILE_ENV) {
            Some(path) => Ok(Some(Self::from_key_file(Path::new(&path))?)),
            None => Ok(None),
        }
    }

    pub fn seal(&self, plaintext: &[u8]) -> io::Result<Vec<u8>> {
        seal(&self.key, plaintext)
    }

    pub fn open(&self, ciphertext: &[u8]) -> io::Result<Vec<u8>> {
        open(&self.key, ciphertext)
    }

    pub(crate) fn from_key(key: Key) -> Self {
        Self { key }
    }

    pub(crate) fn new_random() -> io::Result<Self> {
        let raw = random::<32>()?;
        Ok(Self::from_key(Key::from_bytes(raw)))
    }

    pub(crate) fn key_text(&self) -> String {
        format!("{}\n", hex(&self.key.0))
    }
}

pub fn is_ciphertext(bytes: &[u8]) -> bool {
    bytes.starts_with(MAGIC)
}

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

fn read_key(path: &Path) -> io::Result<Key> {
    let metadata = match fs::symlink_metadata(path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == io::ErrorKind::NotFound => {
            return Err(AccountError::MissingKey.into_io());
        }
        Err(error) => return Err(error),
    };
    if !metadata.is_file() || metadata.file_type().is_symlink() {
        return Err(AccountError::InvalidKey.into_io());
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        if metadata.permissions().mode() & 0o077 != 0 {
            return Err(AccountError::InsecureKeyPermissions.into_io());
        }
    }
    let mut bytes = Vec::new();
    fs::File::open(path)?
        .take(KEY_FILE_MAX_BYTES + 1)
        .read_to_end(&mut bytes)?;
    if bytes.len() as u64 > KEY_FILE_MAX_BYTES {
        return Err(AccountError::InvalidKey.into_io());
    }
    let text = std::str::from_utf8(&bytes).map_err(|_| AccountError::InvalidKey.into_io())?;
    let trimmed = text.trim();
    if trimmed.len() != KEY_TEXT_LEN {
        return Err(AccountError::InvalidKey.into_io());
    }
    let raw = unhex(trimmed).ok_or_else(|| AccountError::InvalidKey.into_io())?;
    Ok(Key::from_bytes(raw))
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
