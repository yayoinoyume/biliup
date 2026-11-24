//! 加密存储：YouTube Cookie、播放器过滤词、整库备份导出与恢复。
//!
//! 所有同步存储调用都放进 spawn_blocking：SQLite 与文件 I/O 绝不占用 tokio worker。

use std::io;

use axum::Json;
use axum::extract::Json as JsonBody;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use base64::Engine as _;
use serde::{Deserialize, Serialize};
use serde_json::json;

use super::ApiResponse;

/// 恢复体的 base64 长度上限：早于解码与底层 128MiB 检查。
const MAX_RESTORE_BASE64_BYTES: usize = 48 * 1024 * 1024;
/// 播放器过滤词上限，与前端输入框一致。
const MAX_PLAYER_FILTER_BYTES: usize = 1024 * 1024;

fn storage_response<T: Serialize>(result: io::Result<T>, message: &str) -> Response {
    match result {
        Ok(data) => (
            StatusCode::OK,
            Json(ApiResponse {
                success: true,
                data: Some(data),
                message: Some(message.to_owned()),
            }),
        )
            .into_response(),
        Err(error) => {
            let status = match error.kind() {
                io::ErrorKind::WouldBlock | io::ErrorKind::AlreadyExists => StatusCode::CONFLICT,
                io::ErrorKind::InvalidInput | io::ErrorKind::InvalidData => StatusCode::BAD_REQUEST,
                _ => StatusCode::INTERNAL_SERVER_ERROR,
            };
            (
                status,
                Json(ApiResponse::<()> {
                    success: false,
                    data: None,
                    message: Some(error.to_string()),
                }),
            )
                .into_response()
        }
    }
}

pub async fn storage_status() -> Json<serde_json::Value> {
    let result =
        tokio::task::spawn_blocking(|| -> io::Result<serde_json::Value> {
            let store = crate::storage::global()?;
            Ok(json!({
                "ready": true,
                "schema": 1,
                "sqlite_version": rusqlite::version(),
                "configured": store.revision("config.json")?.is_some(),
            }))
        })
        .await;
    match result {
        Ok(Ok(value)) => Json(value),
        Ok(Err(error)) => Json(json!({"ready":false,"message":error.to_string()})),
        Err(_) => Json(json!({"ready":false,"message":"数据存储暂不可用"})),
    }
}

pub async fn youtube_cookie_status() -> Response {
    let status = tokio::task::spawn_blocking(crate::plugins::youtube_cookies::status).await;
    let result = match status {
        Ok(result) => result,
        Err(error) => Err(io::Error::other(error)),
    };
    storage_response(result, "")
}

#[derive(Deserialize)]
pub struct CookieImport {
    content: String,
    expected_revision: u64,
}

#[derive(Deserialize)]
pub struct Revision {
    expected_revision: u64,
}

pub async fn import_youtube_cookies(JsonBody(payload): JsonBody<CookieImport>) -> Response {
    storage_response(
        crate::plugins::youtube_cookies::import(payload.content, payload.expected_revision).await,
        "Cookie 已加密保存",
    )
}

pub async fn clear_youtube_cookies(JsonBody(payload): JsonBody<Revision>) -> Response {
    storage_response(
        crate::plugins::youtube_cookies::clear(payload.expected_revision).await,
        "Cookie 已清除",
    )
}

pub async fn get_player_filter() -> Response {
    let result = tokio::task::spawn_blocking(|| {
        let doc = crate::storage::global()?
            .read("invalid_words.txt")?
            .ok_or_else(|| io::Error::other("过滤词不可用"))?;
        Ok(json!({"content":doc.value,"revision":doc.revision}))
    })
    .await
    .map_err(io::Error::other)
    .and_then(|result| result);
    storage_response(result, "")
}

pub async fn save_player_filter(JsonBody(payload): JsonBody<CookieImport>) -> Response {
    if payload.content.len() > MAX_PLAYER_FILTER_BYTES || payload.content.contains('\0') {
        return storage_response::<()>(
            Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "过滤词内容无效或过大",
            )),
            "",
        );
    }
    let result = tokio::task::spawn_blocking(move || {
        crate::storage::global()?.compare_exchange(
            "invalid_words.txt",
            payload.expected_revision,
            serde_json::Value::String(payload.content),
        )
    })
    .await
    .map_err(io::Error::other)
    .and_then(|result| result);
    storage_response(result, "过滤词已保存")
}

#[derive(Deserialize)]
pub struct BackupRequest {
    password: String,
}

pub async fn export_storage_backup(JsonBody(payload): JsonBody<BackupRequest>) -> Response {
    let result = tokio::task::spawn_blocking(move || {
        crate::storage::global()?.export_backup(&payload.password)
    })
    .await
    .map_err(io::Error::other)
    .and_then(|result| result);
    match result {
        Ok(bytes) => (
            [
                (axum::http::header::CONTENT_TYPE, "application/octet-stream"),
                (
                    axum::http::header::CONTENT_DISPOSITION,
                    "attachment; filename=biliup.backup",
                ),
                (axum::http::header::CACHE_CONTROL, "no-store"),
            ],
            bytes,
        )
            .into_response(),
        Err(error) => storage_response::<()>(Err(error), ""),
    }
}

#[derive(Deserialize)]
pub struct RestoreRequest {
    password: String,
    backup_base64: String,
}

pub async fn restore_storage_backup(JsonBody(payload): JsonBody<RestoreRequest>) -> Response {
    let result = tokio::task::spawn_blocking(move || {
        if payload.backup_base64.len() > MAX_RESTORE_BASE64_BYTES {
            return Err(io::Error::new(io::ErrorKind::InvalidInput, "备份文件过大"));
        }
        let bytes = base64::engine::general_purpose::STANDARD
            .decode(payload.backup_base64)
            .map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, "备份格式无效"))?;
        crate::storage::global()?.restore_backup(&payload.password, &bytes)?;
        crate::webui::events::publish(crate::webui::events::CONFIG);
        crate::plugins::set_config_updated();
        crate::webui::state::request_status_refresh();
        Ok(())
    })
    .await
    .map_err(io::Error::other)
    .and_then(|result| result);
    storage_response(result, "备份已恢复")
}

/// 对等节点据此判断本机能承接哪些同步；单机不带围栏承诺。
#[allow(dead_code)] // 待 P2-05 挂载路由后拆除
pub(crate) async fn storage_sync_capabilities() -> Response {
    let mut capabilities = json!({"config_sync":2,"node_local_credentials":true});
    if let Some(fencing) =
        tokio::task::spawn_blocking(crate::cluster::membership_fencing_capability)
            .await
            .ok()
            .flatten()
    {
        capabilities["membership_fencing"] = fencing;
    }
    Json(capabilities).into_response()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn init_store(dir: &tempfile::TempDir) -> std::sync::Arc<crate::storage::Store> {
        crate::storage::init(&dir.path().join("data"), Some(&dir.path().join("key"))).unwrap()
    }

    async fn body_json(response: Response) -> serde_json::Value {
        let bytes = axum::body::to_bytes(response.into_body(), 1 << 24)
            .await
            .unwrap();
        serde_json::from_slice(&bytes).unwrap()
    }

    /// 断言失败时也要关掉全局句柄，否则它指向已删除的临时目录。
    struct StoreShutdown;

    impl Drop for StoreShutdown {
        fn drop(&mut self) {
            crate::storage::shutdown();
        }
    }

    #[tokio::test]
    async fn restore_rejects_oversized_payload_before_decoding() {
        let payload = RestoreRequest {
            password: "synthetic-backup-password".into(),
            backup_base64: "A".repeat(MAX_RESTORE_BASE64_BYTES + 1),
        };
        let response = restore_storage_backup(JsonBody(payload)).await;
        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
        let value = body_json(response).await;
        assert!(
            value["message"].as_str().unwrap().contains("备份文件过大"),
            "必须在解码前按长度拒绝：{value}"
        );
    }

    #[tokio::test]
    async fn backup_round_trips_with_the_right_password_and_rejects_a_wrong_one() {
        let _guard = crate::storage::lock_test_global();
        let dir = tempfile::tempdir().unwrap();
        let _store = init_store(&dir);
        let _shutdown = StoreShutdown;
        let password = "synthetic-backup-password";

        let export = export_storage_backup(JsonBody(BackupRequest {
            password: password.into(),
        }))
        .await;
        assert_eq!(export.status(), StatusCode::OK);
        assert_eq!(
            export.headers()[axum::http::header::CACHE_CONTROL],
            "no-store",
            "备份不得被任何一层缓存留存"
        );
        let bytes = axum::body::to_bytes(export.into_body(), 1 << 24)
            .await
            .unwrap();
        assert_eq!(&bytes[..8], b"BSTRBAK1");
        let encoded = base64::engine::general_purpose::STANDARD.encode(&bytes);

        let wrong = restore_storage_backup(JsonBody(RestoreRequest {
            password: "another-backup-password".into(),
            backup_base64: encoded.clone(),
        }))
        .await;
        assert_eq!(wrong.status(), StatusCode::BAD_REQUEST, "错误密码必须被拒绝");

        let mut events = crate::app_state::AppState::current().subscribe_events();
        let restored = restore_storage_backup(JsonBody(RestoreRequest {
            password: password.into(),
            backup_base64: encoded,
        }))
        .await;
        assert_eq!(restored.status(), StatusCode::OK);
        let mut seen = Vec::new();
        while let Ok(kind) = events.try_recv() {
            seen.push(kind);
        }
        assert!(
            seen.contains(&crate::webui::events::CONFIG),
            "恢复成功后必须广播配置变更：{seen:?}"
        );
    }

    #[tokio::test]
    async fn player_filter_rejects_oversized_nul_and_stale_revision() {
        let _guard = crate::storage::lock_test_global();
        let dir = tempfile::tempdir().unwrap();
        let _store = init_store(&dir);
        let _shutdown = StoreShutdown;

        let oversized = save_player_filter(JsonBody(CookieImport {
            content: "a".repeat(MAX_PLAYER_FILTER_BYTES + 1),
            expected_revision: 0,
        }))
        .await;
        assert_eq!(oversized.status(), StatusCode::BAD_REQUEST);

        let nul = save_player_filter(JsonBody(CookieImport {
            content: "a\0b".into(),
            expected_revision: 0,
        }))
        .await;
        assert_eq!(nul.status(), StatusCode::BAD_REQUEST, "NUL 字节必须被拒绝");

        let saved = save_player_filter(JsonBody(CookieImport {
            content: "屏蔽词".into(),
            expected_revision: 0,
        }))
        .await;
        assert_eq!(saved.status(), StatusCode::OK);

        let stale = save_player_filter(JsonBody(CookieImport {
            content: "另一批".into(),
            expected_revision: 0,
        }))
        .await;
        assert_eq!(stale.status(), StatusCode::CONFLICT, "过期版本号必须冲突");

        let value = body_json(get_player_filter().await).await;
        assert_eq!(value["data"]["content"], "屏蔽词");
        assert_eq!(value["data"]["revision"], 1);
    }

    #[tokio::test]
    async fn youtube_cookie_import_and_clear_reject_stale_revisions() {
        let _guard = crate::storage::lock_test_global();
        let dir = tempfile::tempdir().unwrap();
        let _store = init_store(&dir);
        let _shutdown = StoreShutdown;
        const JAR: &str = "# Netscape HTTP Cookie File\n#HttpOnly_.example.invalid\tTRUE\t/\tTRUE\t0\tsession\tsynthetic\n";

        let imported = import_youtube_cookies(JsonBody(CookieImport {
            content: JAR.into(),
            expected_revision: 0,
        }))
        .await;
        assert_eq!(imported.status(), StatusCode::OK);
        let value = body_json(imported).await;
        assert_eq!(value["data"]["count"], 1);
        assert_eq!(value["data"]["configured"], true);
        let revision = value["data"]["revision"].as_u64().unwrap();

        let stale_import = import_youtube_cookies(JsonBody(CookieImport {
            content: JAR.into(),
            expected_revision: 0,
        }))
        .await;
        assert_eq!(
            stale_import.status(),
            StatusCode::CONFLICT,
            "过期版本号导入必须冲突"
        );

        let cleared = clear_youtube_cookies(JsonBody(Revision {
            expected_revision: revision,
        }))
        .await;
        assert_eq!(cleared.status(), StatusCode::OK);
        let value = body_json(youtube_cookie_status().await).await;
        assert_eq!(value["data"]["configured"], false);

        let stale_clear = clear_youtube_cookies(JsonBody(Revision {
            expected_revision: revision,
        }))
        .await;
        assert_eq!(
            stale_clear.status(),
            StatusCode::CONFLICT,
            "过期版本号清除必须冲突"
        );
    }
}
