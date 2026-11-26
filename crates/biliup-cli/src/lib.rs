pub mod cli;
pub mod downloader;
pub mod entry;
pub mod node_cli;
pub mod season_cli;
pub mod server;
pub mod tools;
pub mod upload_lock;
pub mod uploader;
pub mod web_user_cli;

// use crate::server::api::router::ApplicationController;
use crate::server::app::ApplicationController;
use crate::server::config::{Config, StreamerConfig};
use crate::server::core::download_manager::DownloadManager;
use crate::server::errors::{AppError, AppResult};
use crate::server::fleet::FleetOptions;
use crate::server::infrastructure::connection_pool::{ConnectionManager, ConnectionPool};
use crate::server::infrastructure::models::live_streamer::InsertLiveStreamer;
use crate::server::infrastructure::models::upload_streamer::{
    InsertUploadStreamer, UploadStreamer, is_noop_uploader,
};
use crate::server::infrastructure::repositories;
use crate::server::infrastructure::service_register::ServiceRegister;
use biliup_accounts::{AccountMeta, AccountProvider};
use clap::ValueEnum;
use error_stack::{Report, ResultExt, bail};
use futures::future::BoxFuture;
use std::net::{SocketAddr, ToSocketAddrs};
use std::path::{Path, PathBuf};
use std::sync::{Arc, OnceLock, RwLock};
use tracing_subscriber::{EnvFilter, Registry, reload};

// 定义 Handle 的类型别名，简化代码
// EnvFilter: 我们使用的过滤器类型
// Registry: 基础的 Subscriber 类型
pub type LogHandle = reload::Handle<EnvFilter, Registry>;

pub async fn run(
    addr: (&str, u16),
    auth: bool,
    log_handle: LogHandle,
    config_path: Option<PathBuf>,
) -> AppResult<()> {
    run_with_cookie(
        addr,
        auth,
        false,
        log_handle,
        config_path,
        PathBuf::from("cookies.json"),
    )
    .await
}

pub async fn run_with_cookie(
    addr: (&str, u16),
    auth: bool,
    secure_session_cookie: bool,
    log_handle: LogHandle,
    config_path: Option<PathBuf>,
    user_cookie: PathBuf,
) -> AppResult<()> {
    let listener = bind(addr, auth).await?;
    serve_on(
        listener,
        auth,
        secure_session_cookie,
        log_handle,
        config_path,
        user_cookie,
        FleetOptions::default(),
        None,
    )
    .await
}

/// Resolves and binds the Web server address, refusing an unauthenticated
/// non-loopback bind.
pub async fn bind(addr: (&str, u16), auth: bool) -> AppResult<tokio::net::TcpListener> {
    let addr = addr
        .to_socket_addrs()
        .change_context(AppError::Unknown)?
        .next()
        .ok_or_else(|| {
            Report::new(AppError::Custom(
                "bind address resolved to no sockets".into(),
            ))
        })?;
    validate_server_exposure(addr, auth)?;
    tokio::net::TcpListener::bind(addr)
        .await
        .change_context(AppError::Unknown)
        .attach_with(|| format!("could not listen on {addr}"))
}

/// Runs the Web server on an already-bound `listener`. With `shutdown`, the
/// server stops when that future completes and does not install its own
/// Ctrl+C / SIGTERM handlers, leaving signals to the embedding host.
#[allow(clippy::too_many_arguments)]
pub async fn serve_on(
    listener: tokio::net::TcpListener,
    auth: bool,
    secure_session_cookie: bool,
    log_handle: LogHandle,
    config_path: Option<PathBuf>,
    user_cookie: PathBuf,
    fleet: FleetOptions,
    shutdown: Option<BoxFuture<'static, ()>>,
) -> AppResult<()> {
    biliup_relay::install_crypto_provider();
    let addr = listener.local_addr().change_context(AppError::Unknown)?;
    validate_server_exposure(addr, auth)?;
    server::fleet::reject_config_file(config_path.as_deref())?;
    // WebSub 的回调监听要避开面板端口：转播底座启动前就得把真实端口登记进去。
    biliup_relay::plugins::youtube_websub::set_webui_port(addr.port());

    // let config = Arc::new(AppConfig::parse());

    tracing::info!(
        "environment loaded and configuration parsed, initializing Postgres connection and running migrations..."
    );
    let conn_pool = ConnectionManager::new_pool("data/data.sqlite3")
        .await
        .attach("could not initialize the database connection pool")?;
    // 先收尾上次异常退出留下的分段与场次，再开始监控录制
    if let Err(e) = server::workbench::recover(&conn_pool).await {
        tracing::warn!(error = %e, "切片工作台启动收尾失败，不影响录制与上传");
    }
    server::workbench::clips::remove_leftovers(std::path::Path::new(
        server::infrastructure::service_register::CLIPS_DIR,
    ));
    match server::workbench::clips::recover(&conn_pool, server::workbench::recorder::now_ms()).await
    {
        Ok(0) => {}
        Ok(n) => tracing::info!(clips = n, "上次没导出完的切片已记为失败，可以重试"),
        Err(e) => tracing::warn!(error = %e, "收尾没导出完的切片失败"),
    }
    match server::workbench::clips::recover_submits(
        &conn_pool,
        server::workbench::recorder::now_ms(),
    )
    .await
    {
        Ok(0) => {}
        Ok(n) => tracing::warn!(
            clips = n,
            "上次投稿到一半服务退出了，这些切片的投稿结果未知：再次发布前请先到 B 站稿件管理确认是否已投稿"
        ),
        Err(e) => tracing::warn!(error = %e, "标记投稿结果未知的切片失败"),
    }

    // 凭据密钥：`BILIUP_ACCOUNT_KEY_FILE` 优先，否则 `data/accounts.key`；统一落成绝对路径并设回
    // 环境变量，供后续 credential.rs 的读写（含扫码登录）找同一把密钥。
    let account_key = resolve_account_key();

    // 旧版账号是 `data/<mid>.json` 明文：逐账号迁成密文并把两张表里的引用改成新路径，
    // 迁不动只警告——账号读不了不该连累录制与上传启动。
    match repositories::migrate_plaintext_accounts(
        &conn_pool,
        Path::new("data"),
        Path::new("data/.backups"),
        &account_key,
        None,
    )
    .await
    {
        Ok(report) if report.failed > 0 => tracing::warn!(
            migrated = report.migrated,
            repointed = report.repointed,
            failed = report.failed,
            "有旧版明文 B 站账号没迁完，明文已保留，下次启动会重试"
        ),
        Ok(report) if report.migrated > 0 || report.repointed > 0 => tracing::info!(
            migrated = report.migrated,
            repointed = report.repointed,
            "旧版明文 B 站账号已迁移为加密文件"
        ),
        Ok(_) => {}
        Err(error) => tracing::warn!(%error, "旧版明文 B 站账号迁移失败，继续启动"),
    }

    if let Some(configuration) =
        repositories::register_bilibili_cookie(&conn_pool, &user_cookie).await?
    {
        tracing::info!(
            cookie_file = %configuration.value,
            "registered CLI Bilibili cookie file for Web UI"
        );
    }

    let loaded_config = if let Some(path) = config_path.as_deref() {
        let config = Config::load(path)?;
        tracing::info!(config = %path.display(), "loaded server config file");
        config
    } else {
        repositories::get_config(&conn_pool).await?
    };
    tools::set_configured_ffmpeg(loaded_config.ffmpeg_path.as_deref());
    tools::set_configured_render_fonts(loaded_config.render_fonts_dir.as_deref());
    tools::warn_legacy_danmaku_factory_path(loaded_config.danmaku_factory_path.as_deref());
    if cfg!(windows) {
        tracing::info!(
            create_no_window = biliup::tools::hides_console_windows(),
            "child processes get CREATE_NO_WINDOW only when this process has no console"
        );
    }

    let config = Arc::new(RwLock::new(loaded_config));
    let _sweeper = server::workbench::retention::spawn_sweeper(conn_pool.clone(), config.clone());
    server::auto_clip::runner::install(conn_pool.clone(), config.clone()).await;
    let download_manager = DownloadManager::new(
        config.read().unwrap().pool1_size,
        config.read().unwrap().pool2_size,
        conn_pool.clone(),
    );
    let service_register = ServiceRegister::new(
        conn_pool.clone(),
        config.clone(),
        download_manager,
        log_handle,
    )
    .await;

    let relay_ffmpeg_path = config
        .read()
        .unwrap()
        .ffmpeg_path
        .clone()
        .map(PathBuf::from);
    let relay_accounts = SqliteAccountProvider::load(&conn_pool).await;
    install_relay_accounts(Arc::clone(&relay_accounts), conn_pool.clone());
    if let Err(error) = service_register
        .relay
        .start(biliup_relay::RelayInitConfig {
            data_dir: PathBuf::from("data/relay"),
            key_file: None,
            ffmpeg_path: relay_ffmpeg_path,
            account_provider: Some(relay_accounts),
        })
        .await
    {
        tracing::warn!(%error, "转播底座启动失败，健康快照会停在 disabled");
    }

    if let Some(path) = config_path.as_deref() {
        import_config_streamers(path, &service_register).await?;
    } else {
        import_database_streamers(&service_register).await?;
    }

    let fleet = server::fleet::start(&fleet, &service_register).await?;

    tracing::info!("migrations successfully ran, initializing axum server...");
    ApplicationController::serve(
        listener,
        auth,
        secure_session_cookie,
        service_register,
        fleet,
        shutdown,
    )
    .await
    .attach("could not initialize application routes")?;
    Ok(())
}

/// 账号密钥：`BILIUP_ACCOUNT_KEY_FILE` 优先，否则 `data/accounts.key`。
///
/// 落成绝对路径后设回环境变量，`credential.rs` 的 `Cipher::from_env` 与扫码登录都读它；
/// 启动期调用，之后只读。
fn resolve_account_key() -> PathBuf {
    let configured = std::env::var_os(biliup_accounts::KEY_FILE_ENV).map(PathBuf::from);
    let fallback = PathBuf::from("data/accounts.key");
    let Ok(path) =
        std::path::absolute(configured.clone().unwrap_or_else(|| fallback.clone()))
    else {
        tracing::warn!("B 站凭据密钥路径无法解析为绝对路径，加密账号的读写会失败");
        return configured.unwrap_or(fallback);
    };
    if configured.is_none() {
        unsafe { std::env::set_var(biliup_accounts::KEY_FILE_ENV, &path) };
    }
    path
}

fn validate_server_exposure(addr: SocketAddr, auth: bool) -> AppResult<()> {
    if !addr.ip().is_loopback() && !auth {
        bail!(AppError::Custom(format!(
            "refusing to expose the unauthenticated Web API on {addr}; use a loopback bind address or enable --auth"
        )));
    }
    Ok(())
}

async fn import_config_streamers(path: &Path, service_register: &ServiceRegister) -> AppResult<()> {
    let streamers: Vec<_> = {
        let config = service_register.config.read().unwrap();
        config
            .streamers
            .iter()
            .flat_map(|(remark, streamer)| {
                let global_uploader = config.uploader.clone();
                streamer
                    .url
                    .iter()
                    .map(|url| {
                        (
                            remark.clone(),
                            url.clone(),
                            streamer.clone(),
                            global_uploader.clone(),
                        )
                    })
                    .collect::<Vec<_>>()
            })
            .collect()
    };

    let mut imported = 0usize;
    for (remark, url, streamer, global_uploader) in streamers {
        let upload_config =
            import_upload_config(&remark, &streamer, global_uploader, service_register).await?;
        let live_streamer = repositories::upsert_live_streamer_by_url(
            &service_register.pool,
            to_live_streamer_insert(
                &remark,
                &url,
                &streamer,
                upload_config.as_ref().map(|cfg| cfg.id),
            )?,
        )
        .await?;
        service_register
            .managers
            .add_room(service_register.worker(live_streamer.clone(), upload_config))
            .await
            .ok_or_else(|| {
                Report::new(AppError::Custom(format!(
                    "not supported url in {}: {}",
                    path.display(),
                    live_streamer.url
                )))
            })?;
        imported += 1;
    }
    tracing::info!(config = %path.display(), imported, "imported config streamers");
    Ok(())
}

async fn import_database_streamers(service_register: &ServiceRegister) -> AppResult<()> {
    let streamers = repositories::get_all_streamer(&service_register.pool).await?;
    let mut imported = 0usize;
    for live_streamer in streamers {
        let upload_config =
            repositories::get_upload_config(&service_register.pool, live_streamer.id).await?;

        service_register
            .managers
            .add_room(service_register.worker(live_streamer.clone(), upload_config))
            .await
            .ok_or_else(|| {
                Report::new(AppError::Custom(format!(
                    "not supported url in database: {}",
                    live_streamer.url
                )))
            })?;
        imported += 1;
    }
    tracing::info!(imported, "imported database streamers");
    Ok(())
}

async fn import_upload_config(
    remark: &str,
    streamer: &StreamerConfig,
    global_uploader: Option<String>,
    service_register: &ServiceRegister,
) -> AppResult<Option<UploadStreamer>> {
    let Some(payload) = to_upload_streamer_insert(remark, streamer, global_uploader)? else {
        return Ok(None);
    };

    repositories::upsert_upload_streamer_by_template_name(&service_register.pool, payload)
        .await
        .map(Some)
}

fn to_upload_streamer_insert(
    remark: &str,
    streamer: &StreamerConfig,
    global_uploader: Option<String>,
) -> AppResult<Option<InsertUploadStreamer>> {
    let uploader = streamer.uploader.clone().or(global_uploader);
    if is_noop_uploader(uploader.as_deref()) {
        return Ok(None);
    }

    if !has_upload_config(streamer) && uploader.is_none() {
        return Ok(None);
    }

    Ok(Some(InsertUploadStreamer {
        id: None,
        template_name: format!("config:{remark}"),
        title: streamer.title.clone(),
        tid: streamer
            .tid
            .map(|value| {
                u16::try_from(value).map_err(|_| {
                    Report::new(AppError::Custom(format!(
                        "streamer {remark} tid 超出 u16 范围: {value}"
                    )))
                })
            })
            .transpose()?,
        tid_v2: streamer.tid_v2,
        copyright: streamer.copyright,
        copyright_source: streamer.copyright_source.clone(),
        cover_path: streamer
            .cover_path
            .as_ref()
            .map(|path| path.to_string_lossy().into_owned()),
        description: streamer.description.clone(),
        dynamic: streamer.dynamic.clone(),
        dtime: streamer
            .dtime
            .map(|value| {
                u32::try_from(value).map_err(|_| {
                    Report::new(AppError::Custom(format!(
                        "streamer {remark} dtime 超出 u32 范围: {value}"
                    )))
                })
            })
            .transpose()?,
        dolby: streamer.dolby,
        hires: streamer.hires,
        charging_pay: streamer.charging_pay,
        no_reprint: streamer.no_reprint,
        uploader,
        user_cookie: streamer.user_cookie.clone(),
        tags: streamer.tags.clone().unwrap_or_default(),
        credits: streamer
            .credits
            .as_ref()
            .map(serde_json::to_value)
            .transpose()
            .change_context(AppError::Unknown)?,
        up_selection_reply: streamer.up_selection_reply,
        up_close_reply: streamer.up_close_reply,
        up_close_danmu: streamer.up_close_danmu,
        extra_fields: streamer.extra_fields.clone(),
        is_only_self: streamer.is_only_self,
    }))
}

fn has_upload_config(streamer: &StreamerConfig) -> bool {
    streamer.title.is_some()
        || streamer.tid.is_some()
        || streamer.tid_v2.is_some()
        || streamer.copyright.is_some()
        || streamer.copyright_source.is_some()
        || streamer.cover_path.is_some()
        || streamer.description.is_some()
        || streamer.credits.is_some()
        || streamer.dynamic.is_some()
        || streamer.dtime.is_some()
        || streamer.dolby.is_some()
        || streamer.hires.is_some()
        || streamer.charging_pay.is_some()
        || streamer.no_reprint.is_some()
        || streamer.up_selection_reply.is_some()
        || streamer.up_close_reply.is_some()
        || streamer.up_close_danmu.is_some()
        || streamer.is_only_self.is_some()
        || streamer.user_cookie.is_some()
        || streamer.tags.is_some()
        || streamer.extra_fields.is_some()
}

fn to_live_streamer_insert(
    remark: &str,
    url: &str,
    streamer: &StreamerConfig,
    upload_streamers_id: Option<i64>,
) -> AppResult<InsertLiveStreamer> {
    Ok(InsertLiveStreamer {
        url: url.to_string(),
        remark: remark.to_string(),
        filename_prefix: streamer.filename_prefix.clone(),
        time_range: streamer.time_range.clone(),
        upload_streamers_id,
        format: streamer.format.clone(),
        override_cfg: streamer
            .override_cfg
            .clone()
            .map(|cfg| serde_json::from_value(serde_json::Value::Object(cfg.into_iter().collect())))
            .transpose()
            .change_context(AppError::Unknown)?,
        preprocessor: streamer.preprocessor.clone(),
        segment_processor: streamer.segment_processor.clone(),
        downloaded_processor: streamer.downloaded_processor.clone(),
        postprocessor: streamer.postprocessor.clone(),
        opt_args: streamer
            .opt_args
            .as_ref()
            .map(|args| serde_json::Value::Array(args.iter().cloned().map(Into::into).collect())),
        excluded_keywords: streamer.excluded_keywords.as_ref().map(|keywords| {
            serde_json::Value::Array(keywords.iter().cloned().map(Into::into).collect())
        }),
    })
}

#[derive(Clone, PartialEq, Eq, PartialOrd, Ord, ValueEnum)]
pub enum UploadLine {
    Bldsa,
    Cnbldsa,
    Andsa,
    Atdsa,
    Bda2,
    Cnbd,
    Anbd,
    Atbd,
    Tx,
    Cntx,
    Antx,
    Attx,
    Txa,
    Alia,
    Estx,
    Akbd,
}

/// 宿主账号登记表的只读快照：`id` 就是 `configuration` 里 `bilibili-cookies` 的行号。
#[derive(Default)]
pub struct SqliteAccountProvider {
    accounts: RwLock<std::collections::BTreeMap<i64, StoredAccount>>,
}

struct StoredAccount {
    meta: AccountMeta,
    document: serde_json::Value,
}

impl SqliteAccountProvider {
    pub async fn load(pool: &ConnectionPool) -> Arc<Self> {
        let provider = Arc::new(Self::default());
        provider.reload(pool).await;
        provider
    }

    /// 重读登记表：解不开或读不了的账号只警告并跳过，不影响其余账号。
    pub async fn reload(&self, pool: &ConnectionPool) {
        let rows = sqlx::query_as::<_, (i64, String)>(
            "SELECT id, value FROM configuration WHERE key = 'bilibili-cookies' ORDER BY id",
        )
        .fetch_all(pool)
        .await;
        let rows = match rows {
            Ok(rows) => rows,
            Err(error) => {
                tracing::warn!(%error, "读不了 B 站账号登记表，转播账号选择沿用上一份快照");
                return;
            }
        };
        let mut accounts = std::collections::BTreeMap::new();
        for (id, path) in rows {
            let info = match biliup::uploader::credential::load_login_info(Path::new(&path)) {
                Ok(info) => info,
                Err(error) => {
                    tracing::warn!(id, path = %path, %error, "B 站账号读不了，跳过");
                    continue;
                }
            };
            let document = match serde_json::to_value(&info) {
                Ok(document) => document,
                Err(error) => {
                    tracing::warn!(id, %error, "B 站账号无法转成凭据文档，跳过");
                    continue;
                }
            };
            accounts.insert(
                id,
                StoredAccount {
                    meta: AccountMeta {
                        id,
                        mid: info.token_info.mid,
                        name: None,
                    },
                    document,
                },
            );
        }
        *self.write() = accounts;
    }

    fn read(&self) -> std::sync::RwLockReadGuard<'_, std::collections::BTreeMap<i64, StoredAccount>> {
        self.accounts
            .read()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    fn write(&self) -> std::sync::RwLockWriteGuard<'_, std::collections::BTreeMap<i64, StoredAccount>> {
        self.accounts
            .write()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }
}

impl AccountProvider for SqliteAccountProvider {
    fn list(&self) -> Vec<AccountMeta> {
        self.read()
            .values()
            .map(|account| account.meta.clone())
            .collect()
    }

    fn resolve(&self, id: i64) -> Option<serde_json::Value> {
        self.read()
            .get(&id)
            .map(|account| account.document.clone())
    }
}

static RELAY_ACCOUNTS: OnceLock<(Arc<SqliteAccountProvider>, ConnectionPool)> = OnceLock::new();

pub fn install_relay_accounts(provider: Arc<SqliteAccountProvider>, pool: ConnectionPool) {
    let _ = RELAY_ACCOUNTS.set((provider, pool));
}

/// 登记或删除账号后刷新快照，并让转播丢掉旧配置快照。
pub async fn refresh_relay_accounts() {
    if let Some((provider, pool)) = RELAY_ACCOUNTS.get() {
        provider.reload(pool).await;
        biliup_relay::accounts_changed();
    }
}

#[cfg(test)]
mod server_exposure_tests {
    use super::validate_server_exposure;
    use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr};

    #[test]
    fn unauthenticated_server_is_limited_to_loopback() {
        for ip in [
            IpAddr::V4(Ipv4Addr::LOCALHOST),
            IpAddr::V6(Ipv6Addr::LOCALHOST),
        ] {
            assert!(validate_server_exposure(SocketAddr::new(ip, 19159), false).is_ok());
        }
        assert!(
            validate_server_exposure(
                SocketAddr::new(IpAddr::V4(Ipv4Addr::UNSPECIFIED), 19159),
                false,
            )
            .is_err()
        );
    }

    /// The Web administrator is bootstrapped over the network on first visit, so
    /// an authenticated server must start on a non-loopback bind even when no
    /// administrator exists yet — a container or headless host has no way to
    /// reach a loopback-only bind to initialize one.
    #[test]
    fn authenticated_server_can_bind_non_loopback_before_bootstrap() {
        assert!(
            validate_server_exposure(
                SocketAddr::new(IpAddr::V4(Ipv4Addr::UNSPECIFIED), 19159),
                true,
            )
            .is_ok()
        );
    }
}

#[cfg(test)]
mod tid_v2_config_tests {
    use super::{has_upload_config, to_upload_streamer_insert};
    use crate::server::config::StreamerConfig;

    #[test]
    fn upload_insert_propagates_tid_v2() {
        let streamer = StreamerConfig {
            tid: Some(95),
            tid_v2: Some(2102),
            tags: Some(vec!["tag".into()]),
            ..Default::default()
        };
        assert!(has_upload_config(&streamer));
        let insert = to_upload_streamer_insert("demo", &streamer, None)
            .unwrap()
            .expect("should create insert");
        assert_eq!(insert.tid, Some(95));
        assert_eq!(insert.tid_v2, Some(2102));
    }

    #[test]
    fn upload_insert_tid_only_keeps_tid_v2_none() {
        let streamer = StreamerConfig {
            tid: Some(171),
            ..Default::default()
        };
        let insert = to_upload_streamer_insert("demo", &streamer, None)
            .unwrap()
            .expect("should create insert");
        assert_eq!(insert.tid, Some(171));
        assert!(insert.tid_v2.is_none());
    }
}
