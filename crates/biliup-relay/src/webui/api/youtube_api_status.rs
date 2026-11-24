//! YouTube Data API 与发现状态：只读汇总，密钥只以指纹出现。

use axum::Json;
use serde_json::json;

use crate::config::load_config;

/// 设置页的密钥池、uploads 轮询与 WebSub 状态；没有配置密钥时只回 configured=false。
pub async fn get_status() -> Json<serde_json::Value> {
    let cfg = match load_config().await {
        Ok(c) => c,
        Err(e) => {
            return Json(json!({
                "success": false,
                "message": format!("配置读取失败：{}", e)
            }));
        }
    };
    let keys = cfg.youtube_api_keys();
    if keys.is_empty() {
        return Json(json!({ "success": true, "data": { "configured": false } }));
    }
    let mut data = crate::plugins::youtube_data::key_pool_status(&keys);
    data["configured"] = json!(true);
    data["playlist"] = json!(crate::plugins::youtube_discovery::playlist_status());
    data["websub"] = json!(crate::plugins::youtube_websub::status());
    Json(json!({ "success": true, "data": data }))
}
