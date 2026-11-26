mod crypto;
mod store;

use std::io;

pub use crypto::{CIPHERTEXT_PREFIX, Cipher, KEY_FILE_ENV, is_ciphertext};
pub use zeroize::Zeroizing;
pub use store::{AccountStore, MigratedAccount};

/// 账号中心里可被其他模块选中的一个账号：`id` 是宿主的登记行号。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AccountMeta {
    pub id: i64,
    pub mid: u64,
    pub name: Option<String>,
}

/// 宿主注入的只读账号视图：其他模块按 `id` 取凭据，自己不去碰宿主的存储。
pub trait AccountProvider: Send + Sync {
    fn list(&self) -> Vec<AccountMeta>;
    fn resolve(&self, id: i64) -> Option<serde_json::Value>;
}

#[derive(Debug, thiserror::Error)]
pub enum AccountError {
    #[error("无法生成安全随机数据")]
    Random,
    #[error("密钥文件无效：需要 64 位十六进制密钥")]
    InvalidKey,
    #[error("密钥权限不安全，需要仅运行账号可读写（600）")]
    InsecureKeyPermissions,
    #[error("密钥不匹配或加密数据已损坏")]
    Damaged,
    #[error("密钥文件缺失，请恢复 accounts.key")]
    MissingKey,
    #[error("账号文件无效")]
    InvalidAccountFile,
    #[error("凭据序列化失败")]
    Serialize(#[from] serde_json::Error),
}

impl AccountError {
    pub(crate) fn into_io(self) -> io::Error {
        let kind = match &self {
            Self::MissingKey => io::ErrorKind::NotFound,
            Self::InsecureKeyPermissions => io::ErrorKind::PermissionDenied,
            Self::Random => io::ErrorKind::Other,
            Self::InvalidKey | Self::Damaged | Self::InvalidAccountFile | Self::Serialize(_) => {
                io::ErrorKind::InvalidData
            }
        };
        io::Error::new(kind, self)
    }
}
