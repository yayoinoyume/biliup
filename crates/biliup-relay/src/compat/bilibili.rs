//! 待 Phase 2 B 站插件迁入后替换。

use std::error::Error;

/// 待 phase2/plugins 迁入后替换：凭据续期要访问 B 站接口，Phase 1 不做网络调用。
pub async fn renew() -> Result<(), Box<dyn Error>> {
    Err("B 站凭据刷新待 Phase 2 迁入".into())
}
