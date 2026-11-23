use super::crypto::{random, Key};
use sha2::{Digest, Sha256};
use std::fs::{self, File, OpenOptions};
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};

const KEY_FILE_ENV: &str = "BILISTREAM_KEY_FILE";

pub(super) fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

/// 默认密钥文件：按 XDG/LOCALAPPDATA 与数据目录摘要派生，永远落在 `data` 之外。
pub(super) fn default_key_file(data: &Path) -> io::Result<PathBuf> {
    if let Some(path) = std::env::var_os(KEY_FILE_ENV) {
        return std::path::absolute(path);
    }
    let root = if cfg!(windows) {
        std::env::var_os("LOCALAPPDATA").map(PathBuf::from)
    } else {
        std::env::var_os("XDG_CONFIG_HOME")
            .map(PathBuf::from)
            .or_else(|| std::env::var_os("HOME").map(|p| PathBuf::from(p).join(".config")))
    }
    .ok_or_else(|| io::Error::other("请设置 BILISTREAM_KEY_FILE 指定独立密钥文件"))?;
    let id = hex(&Sha256::digest(data.to_string_lossy().as_bytes()));
    Ok(root
        .join("bilistream")
        .join("keys")
        .join(format!("{id}.key")))
}

pub fn private_dir(path: &Path) -> io::Result<()> {
    if path
        .symlink_metadata()
        .is_ok_and(|m| m.file_type().is_symlink())
    {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "私有数据目录不能是符号链接",
        ));
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
    #[cfg(windows)]
    {
        fs::create_dir_all(path)?;
        let user =
            std::env::var("USERNAME").map_err(|_| io::Error::other("无法确认 Windows 运行账号"))?;
        let mut cmd = std::process::Command::new("icacls");
        crate::plugins::subprocess::configure_no_window(&mut cmd);
        let output = cmd
            .arg(path)
            .args(["/inheritance:r", "/grant:r", &format!("{user}:(OI)(CI)F")])
            .output()?;
        if !output.status.success() {
            return Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                "无法限制私有数据目录权限",
            ));
        }
    }
    Ok(())
}

pub fn create_private(path: &Path) -> io::Result<File> {
    let mut options = OpenOptions::new();
    options.write(true).read(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    options.open(path)
}

pub fn write_private(path: &Path, bytes: &[u8]) -> io::Result<()> {
    let mut file = create_private(path)?;
    if let Err(error) = file.write_all(bytes).and_then(|_| file.sync_all()) {
        drop(file);
        let _ = fs::remove_file(path);
        return Err(error);
    }
    // The verified recovery/key must survive a crash before plaintext retires.
    #[cfg(unix)]
    if let Some(parent) = path.parent() {
        File::open(parent)?.sync_all()?;
    }
    Ok(())
}

pub(super) fn load_key(path: &Path, existing: bool) -> io::Result<Key> {
    if !path.try_exists()? {
        if existing {
            return Err(io::Error::new(
                io::ErrorKind::NotFound,
                "数据库密钥缺失，请恢复原密钥或导入加密备份",
            ));
        }
        let parent = path
            .parent()
            .ok_or_else(|| io::Error::other("密钥路径无效"))?;
        private_dir(parent)?;
        let key = Key(random()?);
        let encoded = protect(&key.0)?;
        write_private(path, &encoded)?;
        return Ok(key);
    }
    let metadata = path.symlink_metadata()?;
    if !metadata.is_file() || metadata.file_type().is_symlink() || metadata.len() > 16 * 1024 {
        return Err(io::Error::new(io::ErrorKind::InvalidData, "密钥文件无效"));
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        if metadata.mode() & 0o077 != 0 || metadata.uid() != unsafe { libc::geteuid() } {
            return Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                "密钥权限不安全，需要仅运行账号可读写（600）",
            ));
        }
    }
    let mut bytes = Vec::new();
    File::open(path)?.take(16 * 1024).read_to_end(&mut bytes)?;
    let plain = unprotect(&bytes)?;
    Ok(Key(plain.as_slice().try_into().map_err(|_| {
        io::Error::new(io::ErrorKind::InvalidData, "密钥长度无效")
    })?))
}

// Windows DPAPI 保护待 Phase 2 补齐；当前各平台都只依赖 600/ACL 权限。
fn protect(bytes: &[u8]) -> io::Result<Vec<u8>> {
    Ok(bytes.to_vec())
}

fn unprotect(bytes: &[u8]) -> io::Result<Vec<u8>> {
    Ok(bytes.to_vec())
}
