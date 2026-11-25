mod crypto;
mod store;

use std::io;

pub use crypto::{Cipher, KEY_FILE_ENV, is_ciphertext};
pub use store::{AccountStore, MigratedAccount};

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
