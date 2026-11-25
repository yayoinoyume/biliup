use crate::AccountError;
use crate::crypto::{self, KEY_FILE_ENV, Key};
use serde_json::Value;
use std::ffi::OsStr;
use std::fs::{self, File, OpenOptions};
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};
use zeroize::{Zeroize, Zeroizing};

const KEY_TEXT_LEN: usize = 64;
const MAX_ACCOUNT_BYTES: u64 = 8 * 1024 * 1024;

pub struct AccountStore {
    data_dir: PathBuf,
    accounts: PathBuf,
    key: Key,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MigratedAccount {
    pub mid: u64,
    pub source: PathBuf,
    pub dest: PathBuf,
}

impl AccountStore {
    pub fn open(data_dir: impl AsRef<Path>, key_path: Option<&Path>) -> io::Result<Self> {
        let data_dir = data_dir.as_ref().to_path_buf();
        let accounts = data_dir.join("accounts");
        private_dir(&accounts)?;
        let key = load_key(&resolve_key_path(&data_dir, key_path)?)?;
        Ok(Self {
            data_dir,
            accounts,
            key,
        })
    }

    pub fn path_for(&self, mid: u64) -> PathBuf {
        self.accounts.join(format!("{mid}.json.enc"))
    }

    pub fn save(&self, mid: u64, value: &Value) -> io::Result<PathBuf> {
        let plaintext = Zeroizing::new(
            serde_json::to_vec(value).map_err(|error| AccountError::Serialize(error).into_io())?,
        );
        let sealed = crypto::seal(&self.key, &plaintext)?;
        let dest = self.path_for(mid);
        write_atomic(&dest, &sealed)?;
        Ok(dest)
    }

    pub fn load(&self, mid: u64) -> io::Result<Value> {
        let sealed = read_limited(&self.path_for(mid))?;
        let plaintext = Zeroizing::new(crypto::open(&self.key, &sealed)?);
        serde_json::from_slice(&plaintext).map_err(|error| AccountError::Serialize(error).into_io())
    }

    pub fn migrate_plaintext(&self, backup_root: &Path) -> io::Result<Vec<MigratedAccount>> {
        let mut sources: Vec<(PathBuf, PathBuf)> = Vec::new();
        for entry in fs::read_dir(&self.data_dir)? {
            let entry = entry?;
            let path = entry.path();
            if !entry.file_type()?.is_file() || path.extension() != Some(OsStr::new("json")) {
                continue;
            }
            sources.push((PathBuf::from(entry.file_name()), path));
        }
        sources.sort();

        let mut migrated = Vec::new();
        let mut backup: Option<PathBuf> = None;
        for (name, path) in sources {
            let bytes = read_limited(&path)?;
            let Ok(value) = serde_json::from_slice::<Value>(&bytes) else {
                continue;
            };
            let Some(mid) = value.pointer("/token_info/mid").and_then(Value::as_u64) else {
                continue;
            };
            let dest = self.path_for(mid);
            if dest.try_exists()? {
                continue;
            }
            self.save(mid, &value)?;
            let directory = match backup.clone() {
                Some(directory) => directory,
                None => {
                    let directory =
                        backup_root.join(format!("accounts-migration-{}", utc_stamp()?));
                    private_dir(&directory)?;
                    backup = Some(directory.clone());
                    directory
                }
            };
            let source = path;
            fs::rename(&source, directory.join(&name))?;
            migrated.push(MigratedAccount { mid, source, dest });
        }
        Ok(migrated)
    }
}

fn resolve_key_path(data_dir: &Path, key_path: Option<&Path>) -> io::Result<PathBuf> {
    if let Some(path) = key_path {
        return std::path::absolute(path);
    }
    if let Some(path) = std::env::var_os(KEY_FILE_ENV) {
        return std::path::absolute(PathBuf::from(path));
    }
    std::path::absolute(data_dir.join("accounts.key"))
}

fn load_key(path: &Path) -> io::Result<Key> {
    if !path.try_exists()? {
        let parent = path
            .parent()
            .ok_or_else(|| AccountError::InvalidKey.into_io())?;
        private_dir(parent)?;
        let mut raw = crypto::random::<32>()?;
        let encoded = Zeroizing::new(format!("{}\n", crypto::hex(&raw)));
        let key = Key::from_bytes(raw);
        raw.zeroize();
        write_new_private(path, encoded.as_bytes())?;
        return Ok(key);
    }

    let metadata = fs::symlink_metadata(path)?;
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
    let bytes = read_limited(path)?;
    let text = std::str::from_utf8(&bytes).map_err(|_| AccountError::InvalidKey.into_io())?;
    let trimmed = text.trim();
    if trimmed.len() != KEY_TEXT_LEN {
        return Err(AccountError::InvalidKey.into_io());
    }
    let raw = crypto::unhex(trimmed).ok_or_else(|| AccountError::InvalidKey.into_io())?;
    Ok(Key::from_bytes(raw))
}

fn private_dir(path: &Path) -> io::Result<()> {
    if path
        .symlink_metadata()
        .is_ok_and(|metadata| metadata.file_type().is_symlink())
    {
        return Err(AccountError::InvalidAccountFile.into_io());
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::{DirBuilderExt, PermissionsExt};
        fs::DirBuilder::new()
            .recursive(true)
            .mode(0o700)
            .create(path)?;
        fs::set_permissions(path, fs::Permissions::from_mode(0o700))?;
    }
    #[cfg(not(unix))]
    {
        fs::create_dir_all(path)?;
    }
    Ok(())
}

fn write_new_private(path: &Path, bytes: &[u8]) -> io::Result<()> {
    let mut options = OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let mut file = options.open(path)?;
    if let Err(error) = file.write_all(bytes).and_then(|()| file.sync_all()) {
        drop(file);
        let _ = fs::remove_file(path);
        return Err(error);
    }
    sync_parent(path)
}

fn write_atomic(dest: &Path, bytes: &[u8]) -> io::Result<()> {
    let parent = dest
        .parent()
        .ok_or_else(|| AccountError::InvalidAccountFile.into_io())?;
    let name = dest
        .file_name()
        .ok_or_else(|| AccountError::InvalidAccountFile.into_io())?;
    let tmp = parent.join(format!("{}.tmp", name.to_string_lossy()));

    let mut options = OpenOptions::new();
    options.write(true).create(true).truncate(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let mut file = options.open(&tmp)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&tmp, fs::Permissions::from_mode(0o600))?;
    }
    if let Err(error) = file.write_all(bytes).and_then(|()| file.sync_all()) {
        drop(file);
        let _ = fs::remove_file(&tmp);
        return Err(error);
    }
    drop(file);
    if let Err(error) = fs::rename(&tmp, dest) {
        let _ = fs::remove_file(&tmp);
        return Err(error);
    }
    sync_parent(dest)
}

fn sync_parent(path: &Path) -> io::Result<()> {
    #[cfg(unix)]
    if let Some(parent) = path.parent() {
        File::open(parent)?.sync_all()?;
    }
    Ok(())
}

fn read_limited(path: &Path) -> io::Result<Vec<u8>> {
    let mut bytes = Vec::new();
    File::open(path)?
        .take(MAX_ACCOUNT_BYTES + 1)
        .read_to_end(&mut bytes)?;
    if bytes.len() as u64 > MAX_ACCOUNT_BYTES {
        return Err(AccountError::InvalidAccountFile.into_io());
    }
    Ok(bytes)
}

fn utc_stamp() -> io::Result<u64> {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|elapsed| elapsed.as_secs())
        .map_err(|error| io::Error::other(error.to_string()))
}
