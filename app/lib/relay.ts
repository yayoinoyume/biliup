/**
 * `/v1/relay/*` 的完整前端契约：类型与请求函数一一对上 Rust 端已挂载的路由。
 *
 * 路径与字段名逐个对照过这些源码，改任何一处都要回去核对，别凭记忆改：
 * - `crates/biliup-cli/src/server/api/relay.rs`（路由与方法）
 * - `crates/biliup-relay/src/webui/api/*.rs`（请求体与响应形状）
 * - `crates/biliup-relay/src/webui/state.rs`、`config.rs`（状态与配置读写模型）
 *
 * 响应形状分四类，调用方必须按类使用，不能用同一个泛型硬套：
 * 1. 裸对象：`/health`、`/deps/status`、`/logs`、`/channels`、`/areas`、`/banned-keywords`、`/storage`、`/setup-status` 等；
 * 2. 信封 `{ success, data?, message? }`：`/status`、`/config`、`/holodex/*` 等，**必须以 success 判定**，
 *    失败的 HTTP 200 也带着 `success: false`；
 * 3. ETag / 304：`/history/*`，可传 `If-None-Match`，结果过大回 413；
 * 4. 二进制与大请求：`/storage/backup`（blob）与 `/storage/restore`（base64 JSON）。
 *
 * 统一 401 跳登录、403 提示并刷新权限点：都复用 `api-streamer.ts` 的既有体系。
 */

import { apiFetch, handleResponse } from './api-streamer'
import { RELAY_MAX_RESTORE_BYTES, describeHttpStatus, fileToBase64 } from './relay-format'

/* ------------------------------------------------------------------ 错误 */

const STATUS_ERROR_CODES: Record<number, string> = {
  400: 'bad_request',
  401: 'unauthorized',
  403: 'forbidden',
  404: 'not_found',
  405: 'method_not_allowed',
  409: 'conflict',
  413: 'too_large',
  422: 'unprocessable',
  429: 'rate_limited',
  500: 'server_error',
  502: 'bad_gateway',
  503: 'unavailable',
  504: 'timeout',
}

/** 空 body 时 api-streamer 抛的泛化英文，不该盖过本地中文兜底。 */
const GENERIC_HTTP_MESSAGE = /^(HTTP \d{3}|Unauthorized)$/

/**
 * 转播接口的统一错误：`status` 是真实 HTTP 状态（信封拒绝时为 200），
 * `code` 是后端给的业务码，没有就按状态码推一个稳定值，`message` 一定是可直接展示的中文。
 */
export class RelayApiError extends Error {
  readonly status: number
  readonly code: string

  constructor(message: string, status: number, code: string) {
    super(message)
    this.name = 'RelayApiError'
    this.status = status
    this.code = code
  }
}

/** 判断错误是不是转播接口抛出的，供页面决定是否显示重试按钮。 */
export function isRelayApiError(error: unknown): error is RelayApiError {
  return error instanceof RelayApiError
}

function errorCodeFor(status: number): string {
  return STATUS_ERROR_CODES[status] ?? `http_${status}`
}

/** 后端错误体可能是 JSON 信封、纯文本（`(StatusCode, String)` 处理器）或空。 */
function readErrorBody(raw: string): { message: string | null; code: string | null } {
  const text = raw.trim()
  if (!text) return { message: null, code: null }
  try {
    const parsed: unknown = JSON.parse(text)
    if (parsed && typeof parsed === 'object') {
      const record = parsed as Record<string, unknown>
      const message = typeof record.message === 'string' ? record.message.trim() : ''
      const code = typeof record.code === 'string' ? record.code : null
      if (message) return { message, code }
      return { message: null, code }
    }
  } catch {
    // 纯文本错误体：原文就是给人看的提示
  }
  return { message: text, code: null }
}

async function relayRequest(
  path: string,
  init?: RequestInit,
  allowNotModified = false,
): Promise<Response> {
  const response = await apiFetch(path, init)
  if (allowNotModified && response.status === 304) return response
  if (response.ok) return response
  const raw = await response.clone().text().catch(() => '')
  let shared: string | null = null
  try {
    await handleResponse(response)
  } catch (error) {
    // 409/500 的空 body 会得到 'HTTP 409' 这类泛化文案，交给 describeHttpStatus 出中文
    if (error instanceof Error && !GENERIC_HTTP_MESSAGE.test(error.message)) shared = error.message
  }
  const body = readErrorBody(raw)
  throw new RelayApiError(
    body.message ?? shared ?? describeHttpStatus(response.status),
    response.status,
    body.code ?? errorCodeFor(response.status),
  )
}

const JSON_HEADERS = { 'Content-Type': 'application/json' } as const

async function relayJson<T>(path: string, init?: RequestInit): Promise<T> {
  const response = await relayRequest(path, init)
  return (await response.json()) as T
}

function relayGetJson<T>(path: string): Promise<T> {
  return relayJson<T>(path, { method: 'GET' })
}

function relaySendJson<T>(path: string, body: unknown, method: 'POST' | 'PUT'): Promise<T> {
  return relayJson<T>(path, {
    method,
    headers: JSON_HEADERS,
    body: JSON.stringify(body ?? {}),
  })
}

/* ------------------------------------------------------------ 信封解包 */

export interface RelayEnvelope<T> {
  success: boolean
  data?: T | null
  message?: string | null
}

const REFUSED_FALLBACK = '转播服务拒绝了这次请求'

function refused(message: string | null | undefined): never {
  throw new RelayApiError(message?.trim() || REFUSED_FALLBACK, 200, 'refused')
}

/** 必须有 data 的信封端点。 */
function unwrapData<T>(envelope: RelayEnvelope<T>): T {
  if (!envelope.success) refused(envelope.message)
  if (envelope.data === undefined || envelope.data === null) refused(envelope.message)
  return envelope.data
}

/** `Option<T>` 数据的信封端点：`null` 是合法答案，例如 `/setup/bili-room`。 */
function unwrapNullable<T>(envelope: RelayEnvelope<T | null>): T | null {
  if (!envelope.success) refused(envelope.message)
  return envelope.data ?? null
}

/** `ApiResponse<()>` 端点：只取服务端那句中文提示。 */
function unwrapMessage(envelope: RelayEnvelope<unknown>): string | null {
  if (!envelope.success) refused(envelope.message)
  return envelope.message?.trim() || null
}

/* ------------------------------------------------------------- 状态快照 */

export interface RelayHealthSnapshot {
  enabled: boolean
  state: string
  data_dir: string
  sqlite_driver: string
  storage_ready: boolean
}

export interface RelayDepsStatus {
  ffmpeg: boolean
  yt_dlp: boolean
  streamlink: boolean
  ready: boolean
}

export interface RelayLogsResponse {
  success: boolean
  logs: string
}

/** 与 `webui::state::NetworkStatus` 对应；空指标一律是 `null`。 */
export interface RelayNetworkStatus {
  ffmpeg_running: boolean
  stream_speed: number | null
  stream_cache_speed: number | null
  stream_bitrate_kbps: number | null
  stream_cache_bitrate_kbps: number | null
  stream_fps: number | null
  stream_frame: number | null
  stream_time_secs: number | null
  stream_cache_time_secs: number | null
  hls_cache_active: boolean
  stream_bitrate_history?: number[]
  stream_cache_bitrate_history?: number[]
}

export interface RelayBiliStatus {
  room: number
  is_live: boolean
  ffmpeg_running: boolean
  title: string
  area_id: number
  area_name: string
  online?: number | null
  live_start_ts?: number | null
  room_locked?: boolean
  lock_till?: number | null
  stream_quality: string | null
  stream_speed: number | null
  stream_cache_speed: number | null
  stream_bitrate_kbps: number | null
  stream_cache_bitrate_kbps: number | null
  stream_fps: number | null
  stream_frame: number | null
  stream_time_secs: number | null
  stream_cache_time_secs: number | null
  hls_cache_active: boolean
  stream_bitrate_history?: number[]
  stream_cache_bitrate_history?: number[]
  enable_danmaku_command: boolean
}

export interface RelayYtStatus {
  is_live: boolean
  enable_monitor: boolean
  warning_held?: boolean
  title: string | null
  topic: string | null
  channel_name: string
  channel_id: string
  quality: string
  area_id: number
  area_name: string
  crop_enabled: boolean
  ffmpeg_cache_enabled: boolean
  ffmpeg_cache_latency_secs: number
}

/** 与 webui::state::TwStatus 一致：没有 YouTube 的 topic，换成 game。 */
export interface RelayTwStatus {
  is_live: boolean
  enable_monitor: boolean
  warning_held?: boolean
  title: string | null
  game: string | null
  channel_name: string
  channel_id: string
  quality: string
  area_id: number
  area_name: string
  crop_enabled: boolean
  ffmpeg_cache_enabled: boolean
  ffmpeg_cache_latency_secs: number
}

export interface RelayNicoStatus {
  is_live: boolean
  enable_monitor: boolean
  warning_held?: boolean
  title: string | null
  channel_name: string
  channel_id: string
  live_id?: string | null
  scheduled_start?: string | null
  quality: string
  area_id: number
  area_name: string
  crop_enabled: boolean
  ffmpeg_cache_enabled: boolean
  ffmpeg_cache_latency_secs: number
}

export interface RelayPriorityChannelStatus {
  enabled: boolean
  auto_restart?: boolean
  warning_held?: boolean
  channel_name: string
  is_live: boolean
  platform: string | null
  title: string | null
  default_area: number
  default_area_name?: string | null
}

export interface RelayStatusData {
  bilibili: RelayBiliStatus
  youtube: RelayYtStatus | null
  twitch: RelayTwStatus | null
  niconico: RelayNicoStatus | null
  priority_channel: RelayPriorityChannelStatus | null
}

/* --------------------------------------------------------- 配置读写模型 */

export interface RelayCropConfig {
  width: number
  height: number
  x: number
  y: number
}

export interface RelayFfmpegCache {
  enabled: boolean
  latency_secs: number
}

export interface RelayClusterPeer {
  node_id: string
  name?: string
  api_url: string
  priority?: number
}

export interface RelayClusterHealthThresholds {
  max_failed_restarts?: number
  max_external_api_failures?: number
  external_api_failure_window_secs?: number
}

export interface RelayPublicStatusConfig {
  node_id?: string
  bind?: string
  port?: number
  holodex_refresh_secs?: number
  public_url?: string
}

export interface RelayClusterConfig {
  enabled?: boolean
  node_id?: string
  node_name?: string
  public_api_url?: string
  peers?: RelayClusterPeer[]
  priority?: number
  heartbeat_interval_secs?: number
  failover_timeout_secs?: number
  lease_ttl_secs?: number
  sync_monitored_channels?: boolean
  auto_failover?: boolean
  thresholds?: RelayClusterHealthThresholds
  public_status?: RelayPublicStatusConfig
}

export interface RelayHistoryRetention {
  raw_days: number
  rollup_days: number
  event_days: number
  event_rows: number
  max_mb: number
}

export interface RelayHistoryConfig {
  enabled: boolean
  sample_secs: number
  bili_sample_secs: number
  bili_stop_summary: boolean
  capture: boolean
  retention: RelayHistoryRetention
}

export interface RelayExemptChannel {
  platform: string
  channel_id: string
  name?: string
}

/** 设置页读模型：`config_response` 的逐字段投影，密钥只以掩码与 `*_configured` 出现。 */
export interface RelayConfigResponse {
  interval: number
  auto_cover: boolean
  show_priority_channel: boolean
  show_twitch: boolean
  show_niconico: boolean
  youtube_rss_enabled: boolean
  enable_anti_collision: boolean
  enable_lol_monitor: boolean
  lol_monitor_interval: number | null
  riot_api_key: string
  riot_api_key_mask: string
  riot_api_key_configured: boolean
  holodex_api_key: string
  holodex_api_key_mask: string
  holodex_api_key_configured: boolean
  holodex_jwt_configured: boolean
  holodex_skip_jwt_verify: boolean
  holodex_monitor_gate: boolean
  youtube_api_key: string
  youtube_api_key_mask: string
  youtube_api_key_configured: boolean
  youtube_websub_callback_url: string
  youtube_websub_port: number
  anti_collision_list: Record<string, number>
  enable_youtube_monitor: boolean
  enable_twitch_monitor: boolean
  auto_stop_after_source_end: boolean
  auto_stop_threshold_hours: number
  cluster: RelayClusterConfig
  history: RelayHistoryConfig
  bilibili: { room: number; enable_danmaku_command: boolean }
  youtube: {
    enable_monitor: boolean
    channel_name: string
    channel_id: string
    area_v2: number
    proxy: string
    proxy_configured: boolean
    cookies_file: string | null
    cookies_from_browser: string | null
    deno_path: string | null
    ffmpeg_cache: RelayFfmpegCache
  }
  twitch: {
    enable_monitor: boolean
    channel_name: string
    channel_id: string
    area_v2: number
    proxy_region: string
    proxy: string
    proxy_configured: boolean
    ffmpeg_cache: RelayFfmpegCache
  }
  niconico: {
    enable_monitor: boolean
    channel_name: string
    channel_id: string
    live_id: string
    area_v2: number
    quality: string
    user_session_configured: boolean
    user_session_mask: string
    session_check_enabled: boolean
    proxy: string
    proxy_configured: boolean
    ffmpeg_cache: RelayFfmpegCache
  }
  priority_channel: {
    enabled: boolean
    channel_name: string
    youtube_channel_id: string
    twitch_channel_id: string
    default_area: number
    auto_restart: boolean
  }
  warning_hold_exempt: RelayExemptChannel[]
  warning_hold_exempt_reasons: string[]
  warning_hold_exempt_all_low_risk: boolean
  warning_hold_exempt_channels: { name: string; platforms: { platform: string; channel_id: string }[] }[]
  /** 当前配置内容指纹：写回时放在 `expected_secret_revision` 里做三方校验。 */
  secret_revision: number
}

export interface RelayHistoryPatch {
  enabled?: boolean
  sample_secs?: number
  bili_sample_secs?: number
  bili_stop_summary?: boolean
  capture?: boolean
  retention?: Partial<RelayHistoryRetention>
}

/**
 * 写模型：全部字段可选，只发真正改过的键。
 *
 * - `expected`：被编辑键的原始值，用来撞出 409 EDIT_CONFLICT（缺键会回 400 EDIT_INVALID）。
 * - `expected_secret_revision`：改任何密钥/代理时的配置指纹，不匹配直接 409。
 * - `*_keep_password` + 空口令位：保留已保存的代理密码。
 */
export interface RelayUpdateConfigRequest {
  expected?: Record<string, unknown>
  expected_secret_revision?: number
  clear_riot_api_key?: boolean
  clear_holodex_api_key?: boolean
  clear_youtube_api_key?: boolean
  interval?: number
  auto_stop_after_source_end?: boolean
  auto_stop_threshold_hours?: number
  auto_cover?: boolean
  show_priority_channel?: boolean
  show_twitch?: boolean
  show_niconico?: boolean
  youtube_rss_enabled?: boolean
  enable_anti_collision?: boolean
  enable_lol_monitor?: boolean
  lol_monitor_interval?: number
  riot_api_key?: string
  holodex_api_key?: string
  holodex_jwt?: string
  holodex_skip_jwt_verify?: boolean
  holodex_monitor_gate?: boolean
  youtube_api_key?: string
  youtube_websub_callback_url?: string
  youtube_websub_port?: number
  twitch_proxy_region?: string
  twitch_proxy?: string
  twitch_proxy_keep_password?: boolean
  clear_twitch_proxy?: boolean
  youtube_proxy?: string
  youtube_proxy_keep_password?: boolean
  clear_youtube_proxy?: boolean
  youtube_deno_path?: string
  anti_collision_list?: Record<string, number>
  enable_danmaku_command?: boolean
  youtube_enable_monitor?: boolean
  twitch_enable_monitor?: boolean
  warning_hold_exempt?: RelayExemptChannel[]
  warning_hold_exempt_reasons?: string[]
  warning_hold_exempt_all_low_risk?: boolean
  youtube_cookies_from_browser?: string
  youtube_cookies_file?: string
  niconico_user_session?: string
  clear_niconico_user_session?: boolean
  niconico_session_check_enabled?: boolean
  niconico_proxy?: string
  niconico_proxy_keep_password?: boolean
  clear_niconico_proxy?: boolean
  cluster?: RelayClusterConfig
  history?: RelayHistoryPatch
}

export interface RelayUpdatePriorityChannelRequest {
  expected?: Record<string, unknown>
  enabled?: boolean
  channel_name?: string
  default_area?: number
  auto_restart?: boolean
}

/* ------------------------------------------------------- 频道与分区文档 */

/** `areas.json` 的一行；`serde(flatten)` 允许携带额外自定义键。 */
export interface RelayArea {
  id: number
  name: string
  title_keywords: string[]
  aliases: string[]
  [extra: string]: unknown
}

export interface RelayAreasDocument {
  areas: RelayArea[]
  banned_keywords?: string[]
  streaming_banned_keywords?: string[]
  [extra: string]: unknown
}

export interface RelayChannel {
  name: string
  niconico_name?: string | null
  aliases: string[]
  platforms: Record<string, string>
  riot_puuid?: string | null
  [extra: string]: unknown
}

export interface RelayChannelsDocument {
  channels: RelayChannel[]
  [extra: string]: unknown
}

export interface RelayBannedKeywords {
  danmaku_banned_keywords: string[]
  streaming_banned_keywords: string[]
}

export interface RelayUpdateBannedKeywordsRequest {
  expected?: Record<string, unknown>
  danmaku_banned_keywords?: string[]
  streaming_banned_keywords?: string[]
}

export interface RelayAddAreaRequest {
  id: number
  name: string
  title_keywords: string[]
  aliases: string[]
}

export interface RelayAddChannelRequest {
  name: string
  aliases: string[]
  platforms: Record<string, string>
  riot_puuid?: string | null
  niconico_name?: string | null
}

/* ----------------------------------------------------------- 控制端点 */

export interface RelayStartStreamRequest {
  platform?: 'YT' | 'TW' | 'NC'
}

export interface RelayStartStreamData {
  requires_face_auth?: boolean
  qr_url?: string
}

/**
 * 开播结果：后端失败也回 HTTP 200，人脸验证时还带着 `qr_url`，
 * 因此这里不抛错，交给页面决定是提示还是弹二维码。
 */
export type RelayStartStreamOutcome =
  | { ok: true; message: string | null }
  | { ok: false; message: string; faceAuth: { qrUrl: string | null } | null }

export interface RelayReleasePriorityWarningRequest {
  channel_name: string
  platform?: string
}

export interface RelayUpdateCoverRequest {
  /** 只接受应用缓存里的文件名：`cover.jpg` 或 `pic_for_crop.jpg`。 */
  image_path: 'cover.jpg' | 'pic_for_crop.jpg'
}

export interface RelayUpdateAreaRequest {
  area_id: number
}

export interface RelayUpdateTitleRequest {
  title: string
}

export interface RelayUpdateChannelRequest {
  platform: 'youtube' | 'twitch' | 'niconico'
  channel_id?: string
  channel_name?: string
  live_id?: string
  area_id?: number
  quality?: string
  riot_api_key?: string
  cookies_file?: string
  cookies_from_browser?: string
}

export interface RelayToggleMonitorRequest {
  enabled: boolean
}

export interface RelaySwitchStreamRequest {
  channel_id: string
  area_id?: number
  title?: string
  topic_id?: string
  status?: string
  platform?: string
  external_link?: string
  twitch_channel_id?: string
}

export interface RelayAdminStreamListItem {
  id: string
  title: string
  topic: string | null
  status: string
  start_scheduled: string | null
  start_actual: string | null
  available_at: string | null
  published_at: string | null
  live_viewers: number | null
  channel_id: string
  channel_name: string
  channel_photo: string | null
  suggested_area_id: number | null
  suggested_area_name: string | null
  is_placeholder: boolean
  placeholder_type: string | null
  link: string | null
  thumbnail: string | null
}

export interface RelayHolodexStreamsQuery {
  favorites?: boolean
  force?: boolean
}

/* --------------------------------------------------------------- 引导 */

export interface RelaySetupStatus {
  needs_setup: boolean
  missing_files: string[]
  setup_command: string
  storage_error: string | null
}

export interface RelaySetupArea {
  id: number
  name: string
  parent_name: string
}

export interface RelayHolodexFavoriteChannel {
  id: string
  name: string
}

export interface RelaySetupFavoritesRequest {
  api_key: string
  jwt: string
}

export interface RelaySetupConfigRequest {
  room: number
  auto_cover: boolean
  enable_danmaku_command: boolean
  interval: number
  anti_collision: boolean
  enable_lol_monitor: boolean
  youtube_enable_monitor?: boolean
  youtube_channel_name?: string
  youtube_channel_id?: string
  youtube_area_v2?: number
  youtube_quality?: string
  youtube_proxy?: string
  twitch_enable_monitor?: boolean
  twitch_channel_name?: string
  twitch_channel_id?: string
  twitch_area_v2?: number
  twitch_proxy_region?: string
  twitch_quality?: string
  twitch_proxy?: string
  niconico_enable_monitor?: boolean
  niconico_channel_name?: string
  niconico_channel_id?: string
  niconico_area_v2?: number
  selected_areas?: RelaySetupArea[]
  selected_youtube_channels?: RelayHolodexFavoriteChannel[]
  holodex_api_key?: string
  holodex_jwt?: string
  riot_api_key?: string
}

export interface RelayLoginStatus {
  logged_in: boolean
  message: string
}

export interface RelayQrCode {
  qr_url: string
  qr_image: string
  auth_code: string
}

export interface RelayPollLoginRequest {
  auth_code: string
}

export interface RelayPollLoginResult {
  status: 'success' | 'waiting' | 'expired' | 'error'
  message: string
}

/* ------------------------------------------------------------- 裁切 */

export type RelayCropPlatform = 'youtube' | 'twitch' | 'niconico'

export interface RelayUpdateCropRequest {
  platform: RelayCropPlatform
  enabled: boolean
  width?: number
  height?: number
  x?: number
  y?: number
}

export interface RelayUpdateFfmpegCacheRequest {
  platform: RelayCropPlatform
  enabled: boolean
  latency_secs?: number
}

/* ------------------------------------------------------------- 存储 */

export type RelayStorageStatus =
  | { ready: true; schema: number; sqlite_version: string; configured: boolean }
  | { ready: false; message: string }

export interface RelayYoutubeCookieStatus {
  configured: boolean
  count: number
  revision: number
  updated_at: number
}

export interface RelayPlayerFilter {
  content: string
  revision: number
}

export interface RelayPlayerFilterSaveRequest {
  content: string
  expected_revision: number
}

/* ------------------------------------------------------- Niconico 会话 */

export type RelayNiconicoSessionState =
  | 'unconfigured'
  | 'unchecked'
  | 'valid'
  | 'invalid'
  | 'unavailable'

export interface RelayNiconicoSessionStatus {
  state: RelayNiconicoSessionState
  checked_at: string | null
  message: string
}

/* --------------------------------------------------------- Holodex/YT */

export interface RelayHolodexAuthStatus {
  logged_in: boolean
  username?: string | null
  skip_jwt_verify?: boolean
  jwt_refreshed?: boolean
  expired?: boolean
  message?: string
}

export interface RelayYoutubeKeyStatusEntry {
  /** 密钥指纹，绝不含密钥本体。 */
  fingerprint: string
  used: number
  state: 'usable' | 'exhausted' | 'rejected'
}

export interface RelayYoutubeKeyStatus {
  configured: boolean
  keys?: RelayYoutubeKeyStatusEntry[]
  budget_per_key?: number
  remaining_fraction?: number
  resets_at?: string
  playlist?: unknown
  websub?: unknown
}

/* ---------------------------------------------------------- 统计历史 */

export interface RelayHistoryQuery {
  /** Unix 秒，闭开区间 `[from, to)` 的起点。 */
  from?: number
  /** Unix 秒；缺省取服务端当前时间。 */
  to?: number
  tz_offset_min?: number
  bucket?: 'hour' | '6h' | 'day'
  platform?: 'YT' | 'TW' | 'NC'
  channel?: string
  /** 会话 id，逗号分隔。 */
  id?: string
  limit?: number
  cursor?: string
  /** 事件类型，逗号分隔（后端参数名是单数 `kind`）。 */
  kinds?: string[]
  /** 事件级别，逗号分隔（后端参数名是单数 `severity`）。 */
  severities?: string[]
  subject?: string
  metrics?: string[]
  /** 图表用的会话 id，逗号分隔。 */
  session?: string
  max_points?: number
  /** 只有 `true` 会被后端当成 1。 */
  calendar?: boolean
  /** `local` / `cluster` 或已知成员 id。 */
  scope?: string
}

export interface RelayMissingNode {
  node_id: string
  reason: string
  detail: string | null
}

export interface RelayHistoryKpis {
  live_secs: number
  availability: number | null
  sessions: number
  failures: number
  recover_secs_avg: number | null
  success_rate: number | null
  notify_sent: number
  notify_failed: number
  watched: number | null
  watched_source: 'mixed' | 'sample' | 'summary' | null
  online_peak: number | null
  danmu: number | null
  add_fans: number | null
  new_fans_club: number | null
  max_online: number | null
}

export interface RelayHistorySummaryBucket {
  start: number
  live_secs: number
  sessions: number
  failures: number
  recover_secs_avg: number | null
  notify_sent: number
  notify_failed: number
}

export interface RelayHistorySummary {
  buckets: RelayHistorySummaryBucket[]
  kpis: RelayHistoryKpis
  prev_kpis: RelayHistoryKpis
  calendar: { day: string; live_secs: number; failures: number }[]
  reliability: {
    clean: number
    counted_failures: number
    planned: number
    reasons: { class: string; count: number }[]
  }
  per_node: {
    node_id: string
    live_secs: number
    failures: number
    down_secs: number | null
    node_observed_secs: number | null
    last_seen_at: number | null
  }[]
  coverage: {
    from: number | null
    per_node: Record<string, number | null>
    partial_before: Record<string, number>
  }
  node_segments: unknown[]
  node_segments_more: boolean
  missing_nodes: RelayMissingNode[]
  clamped_to_days?: number
  clamp_scope?: 'local'
  partial_nodes?: { node: string; clamped_to_days: number }[]
}

export interface RelayHistoryBiliItem {
  state: string
  watched: number | null
  watched_source?: string | null
  online_peak: number | null
  danmu: number | null
  add_fans: number | null
  new_fans_club: number | null
  max_online: number | null
}

export interface RelayHistorySessionItem {
  /** `node:eid` 形式的全局 id。 */
  id: string
  eid: string
  node: string
  platform: string
  channel_id: string
  channel_name: string
  title: string | null
  area_v2?: number | null
  started_at: number
  ended_at: number | null
  duration_secs: number
  end_reason: string | null
  runs: number
  restarts: number
  failures: number
  stuck: number
  /** `[start, end, reason]` 三元组。 */
  gaps: [number, number, string][]
  median_bitrate_kbps: number | null
  source_id?: string
  broadcast_id: string
  bili: RelayHistoryBiliItem | null
  end_cut_off?: boolean
  end_warning_stop?: boolean
  end_detail?: string
}

export interface RelayHistorySessionsPage {
  items: RelayHistorySessionItem[]
  next_cursor: string | null
  missing_nodes: RelayMissingNode[]
  more?: boolean
  clamped_to_days?: number
  clamp_scope?: 'local'
  partial_nodes?: { node: string; clamped_to_days: number }[]
}

export interface RelayHistoryNotifyAttempt {
  event_key: string
  dedup_key: string
  phase: string
  mode: string | null
  ok: boolean
  code: number | null
  trace_id: string | null
  test: boolean
  suppressed: number
  undelivered: number
}

export interface RelayHistoryEventItem {
  id: string
  eid: string
  at: number
  node: string
  kind: string
  session_id?: string
  title: string
  detail: string
  severity: string
  notify?: RelayHistoryNotifyAttempt
}

export interface RelayHistoryEventsPage {
  items: RelayHistoryEventItem[]
  next_cursor: string | null
  missing_nodes: RelayMissingNode[]
  more?: boolean
  clamped_to_days?: number
  clamp_scope?: 'local'
  partial_nodes?: { node: string; clamped_to_days: number }[]
}

export interface RelayHistoryStats {
  resolution: 'raw' | '1m'
  ts: number[]
  series: Record<string, { mean: (number | null)[]; min: (number | null)[]; max: (number | null)[] }>
  coverage?: { partial_before: number | null }
  missing_nodes: RelayMissingNode[]
}

export interface RelayHistoryStatus {
  node_id: string
  cluster_nodes: string[]
  enabled: boolean
  configured_enabled: boolean
  restart_required: boolean
  schema_version: number
  oldest_at: number | null
  recording_since: number | null
  db_bytes: number
  wal_bytes?: number
  space_pressure?: boolean
  partial_before?: number | null
  rows: Record<string, number>
  dropped: Record<string, number>
  writer: { state: string; last_batch_at: number | null; last_error: string | null }
  retention: RelayHistoryRetention
  recorder: unknown
  missing_nodes: RelayMissingNode[]
}

export interface RelayHistoryCleanReport {
  rows: number
  nodes: { node_id: string; rows: number }[]
  failed: { node_id: string; reason: string }[]
}

/** 304 与正常结果分开，页面才知道该保留旧数据还是换新数据。 */
export type RelayConditionalResult<T> =
  | { notModified: true; etag: string | null; data: null }
  | { notModified: false; etag: string | null; data: T }

/* ------------------------------------------------------------------ 路由 */

const RELAY = '/v1/relay'

/** 设置里的 Niconico 登录态路由：与 relay.rs:131 的字面量逐字节一致（斜杠分段）。 */
const RELAY_NICONICO_SESSION = `${RELAY}/niconico/session`
/** 设置里的 YouTube Cookie 路由：与 relay.rs:167 的字面量逐字节一致（斜杠分段）。 */
const RELAY_YOUTUBE_COOKIES = `${RELAY}/youtube/cookies`

/* ---------------------------------------------------------- 裸对象端点 */

/** GET /v1/relay/health —— 底座自检；仍需 relay.view（permissions.rs:456-486 的 RelayView 组）。 */
export function relayHealth(): Promise<RelayHealthSnapshot> {
  return relayGetJson<RelayHealthSnapshot>(`${RELAY}/health`)
}

/** GET /v1/relay/deps/status —— ffmpeg / yt-dlp / streamlink 是否可用。 */
export function relayDepsStatus(): Promise<RelayDepsStatus> {
  return relayGetJson<RelayDepsStatus>(`${RELAY}/deps/status`)
}

/** GET /v1/relay/logs —— 内存环形日志，`logs` 是整段纯文本。 */
export function relayLogs(): Promise<RelayLogsResponse> {
  return relayGetJson<RelayLogsResponse>(`${RELAY}/logs`)
}

/** GET /v1/relay/channels —— `channels.json` 原文。 */
export function relayChannels(): Promise<RelayChannelsDocument> {
  return relayGetJson<RelayChannelsDocument>(`${RELAY}/channels`)
}

/** GET /v1/relay/areas —— `areas.json` 原文（含禁用关键词）。 */
export function relayAreas(): Promise<RelayAreasDocument> {
  return relayGetJson<RelayAreasDocument>(`${RELAY}/areas`)
}

/** GET /v1/relay/banned-keywords —— 弹幕/推流两套禁用关键词。 */
export function relayBannedKeywords(): Promise<RelayBannedKeywords> {
  return relayGetJson<RelayBannedKeywords>(`${RELAY}/banned-keywords`)
}

/** GET /v1/relay/storage —— 加密存储是否就绪。 */
export function relayStorageStatus(): Promise<RelayStorageStatus> {
  return relayGetJson<RelayStorageStatus>(`${RELAY}/storage`)
}

/** GET /v1/relay/setup-status —— 引导页是否还需要走首次配置。 */
export function relaySetupStatus(): Promise<RelaySetupStatus> {
  return relayGetJson<RelaySetupStatus>(`${RELAY}/setup-status`)
}

/** GET /v1/relay/setup/login-status —— B 站账号是否已登录。 */
export function relaySetupLoginStatus(): Promise<RelayLoginStatus> {
  return relayGetJson<RelayLoginStatus>(`${RELAY}/setup/login-status`)
}

/* ------------------------------------------------------------ 信封端点 */

/** GET /v1/relay/version —— 转播底座版本号。 */
export async function relayVersion(): Promise<string> {
  const envelope = await relayGetJson<RelayEnvelope<{ version: string }>>(`${RELAY}/version`)
  return unwrapData(envelope).version
}

/**
 * GET /v1/relay/status —— 面板状态快照。
 *
 * 失败时也是 HTTP 200 + `success: false`（例如配置文件缺失），因此必须按 success 判定。
 */
export async function relayStatus(): Promise<RelayStatusData> {
  const envelope = await relayGetJson<RelayEnvelope<RelayStatusData>>(`${RELAY}/status`)
  return unwrapData(envelope)
}

/** GET /v1/relay/network-status —— 本机推流器实时指标。 */
export async function relayNetworkStatus(): Promise<RelayNetworkStatus> {
  const envelope = await relayGetJson<RelayEnvelope<RelayNetworkStatus>>(
    `${RELAY}/network-status`,
  )
  return unwrapData(envelope)
}

/** GET /v1/relay/config —— 设置页读模型（密钥只出掩码与 `*_configured`）。 */
export function relayGetConfig(): Promise<RelayConfigResponse> {
  return relayGetJson<RelayConfigResponse>(`${RELAY}/config`)
}

/**
 * POST /v1/relay/config —— 逐字段局部更新。
 *
 * 缺 `expected` 会回 400 EDIT_INVALID；`expected` 与现值不符回 409 EDIT_CONFLICT。
 */
export async function relayUpdateConfig(patch: RelayUpdateConfigRequest): Promise<string | null> {
  const envelope = await relaySendJson<RelayEnvelope<null>>(`${RELAY}/config`, patch, 'POST')
  return unwrapMessage(envelope)
}

/** POST /v1/relay/priority-channel —— 优先频道开关与默认分区。 */
export async function relayUpdatePriorityChannel(
  patch: RelayUpdatePriorityChannelRequest,
): Promise<string | null> {
  const envelope = await relaySendJson<RelayEnvelope<null>>(
    `${RELAY}/priority-channel`,
    patch,
    'POST',
  )
  return unwrapMessage(envelope)
}

/** POST /v1/relay/priority-channel/release-warning —— 手动解除优先频道的告警挂起。 */
export async function relayReleasePriorityWarning(
  payload: RelayReleasePriorityWarningRequest,
): Promise<unknown> {
  const envelope = await relaySendJson<RelayEnvelope<unknown>>(
    `${RELAY}/priority-channel/release-warning`,
    payload,
    'POST',
  )
  if (!envelope.success) refused(envelope.message)
  return envelope.data ?? null
}

/** POST /v1/relay/start —— 开播；失败可能带人脸验证二维码。 */
export async function relayStartStream(
  payload: RelayStartStreamRequest = {},
): Promise<RelayStartStreamOutcome> {
  const envelope = await relaySendJson<RelayEnvelope<RelayStartStreamData>>(
    `${RELAY}/start`,
    payload,
    'POST',
  )
  const message = envelope.message?.trim() || null
  if (envelope.success) return { ok: true, message }
  const data = envelope.data ?? null
  return {
    ok: false,
    message: message ?? '开播失败',
    faceAuth: data?.requires_face_auth
      ? { qrUrl: data.qr_url?.trim() || null }
      : null,
  }
}

/** POST /v1/relay/stop —— 停播。 */
export async function relayStopStream(): Promise<string | null> {
  const envelope = await relaySendJson<RelayEnvelope<null>>(`${RELAY}/stop`, {}, 'POST')
  return unwrapMessage(envelope)
}

/** POST /v1/relay/restart —— 停掉当前流并重新加载配置。 */
export async function relayRestartStream(): Promise<string | null> {
  const envelope = await relaySendJson<RelayEnvelope<null>>(`${RELAY}/restart`, {}, 'POST')
  return unwrapMessage(envelope)
}

/** POST /v1/relay/danmaku —— 发一条弹幕到直播间。 */
export async function relaySendDanmaku(message: string): Promise<string | null> {
  const envelope = await relaySendJson<RelayEnvelope<null>>(
    `${RELAY}/danmaku`,
    { message },
    'POST',
  )
  return unwrapMessage(envelope)
}

/** POST /v1/relay/cover —— 用应用缓存里的封面更新直播间封面。 */
export async function relayUpdateCover(payload: RelayUpdateCoverRequest): Promise<string | null> {
  const envelope = await relaySendJson<RelayEnvelope<null>>(`${RELAY}/cover`, payload, 'POST')
  return unwrapMessage(envelope)
}

/** POST /v1/relay/area —— 改直播间分区。 */
export async function relayUpdateArea(payload: RelayUpdateAreaRequest): Promise<string | null> {
  const envelope = await relaySendJson<RelayEnvelope<null>>(`${RELAY}/area`, payload, 'POST')
  return unwrapMessage(envelope)
}

/** POST /v1/relay/title —— 改直播间标题。 */
export async function relayUpdateTitle(payload: RelayUpdateTitleRequest): Promise<string | null> {
  const envelope = await relaySendJson<RelayEnvelope<null>>(`${RELAY}/title`, payload, 'POST')
  return unwrapMessage(envelope)
}

/** POST /v1/relay/channel —— 改某个平台的监听目标（频道、分区、画质、Cookie）。 */
export async function relayUpdateChannel(
  payload: RelayUpdateChannelRequest,
): Promise<string | null> {
  const envelope = await relaySendJson<RelayEnvelope<null>>(`${RELAY}/channel`, payload, 'POST')
  return unwrapMessage(envelope)
}

/** POST /v1/relay/channels/resolve-youtube —— 把 @handle / 网址解析成 UC 频道 id。 */
export async function relayResolveYoutubeChannel(input: string): Promise<string> {
  const envelope = await relaySendJson<RelayEnvelope<{ channel_id: string }>>(
    `${RELAY}/channels/resolve-youtube`,
    { input },
    'POST',
  )
  return unwrapData(envelope).channel_id
}

/** GET /v1/relay/areas/catalog —— B 站官方两级分区（父类 + 分区）。 */
export async function relayAreaCatalog(): Promise<RelaySetupArea[]> {
  const envelope = await relayGetJson<RelayEnvelope<RelaySetupArea[]>>(
    `${RELAY}/areas/catalog`,
  )
  return unwrapData(envelope)
}

/** POST /v1/relay/banned-keywords —— 改两套禁用关键词。 */
export async function relayUpdateBannedKeywords(
  payload: RelayUpdateBannedKeywordsRequest,
): Promise<string | null> {
  const envelope = await relaySendJson<RelayEnvelope<null>>(
    `${RELAY}/banned-keywords`,
    payload,
    'POST',
  )
  return unwrapMessage(envelope)
}

/** POST /v1/relay/setup/save-config —— 引导页一次性落盘。 */
export async function relaySaveSetupConfig(
  payload: RelaySetupConfigRequest,
): Promise<string | null> {
  const envelope = await relaySendJson<RelayEnvelope<null>>(
    `${RELAY}/setup/save-config`,
    payload,
    'POST',
  )
  return unwrapMessage(envelope)
}

/** POST /v1/relay/setup/holodex-favorites —— 预览 Holodex 收藏夹，未落盘。 */
export async function relayPreviewSetupFavorites(
  payload: RelaySetupFavoritesRequest,
): Promise<RelayHolodexFavoriteChannel[]> {
  const envelope = await relaySendJson<RelayEnvelope<RelayHolodexFavoriteChannel[]>>(
    `${RELAY}/setup/holodex-favorites`,
    payload,
    'POST',
  )
  return unwrapData(envelope)
}

/** GET /v1/relay/setup/qrcode —— B 站扫码登录二维码（本地渲染的 SVG data URL）。 */
export async function relaySetupQrCode(): Promise<RelayQrCode> {
  const envelope = await relayGetJson<RelayEnvelope<RelayQrCode>>(`${RELAY}/setup/qrcode`)
  return unwrapData(envelope)
}

/** POST /v1/relay/setup/poll-login —— 轮询扫码结果。 */
export async function relaySetupPollLogin(
  authCode: string,
): Promise<RelayPollLoginResult> {
  const envelope = await relaySendJson<RelayEnvelope<RelayPollLoginResult>>(
    `${RELAY}/setup/poll-login`,
    { auth_code: authCode } satisfies RelayPollLoginRequest,
    'POST',
  )
  return unwrapData(envelope)
}

/** GET /v1/relay/setup/bili-room —— 用已登录账号的 uid 反推房间号；`null` 表示仍需手填。 */
export async function relaySetupBiliRoom(): Promise<number | null> {
  const envelope = await relayGetJson<RelayEnvelope<number | null>>(
    `${RELAY}/setup/bili-room`,
  )
  return unwrapNullable(envelope)
}

/** POST /v1/relay/toggle-youtube-monitor —— 开关 YouTube 监听。 */
export async function relayToggleYoutubeMonitor(enabled: boolean): Promise<string | null> {
  const envelope = await relaySendJson<RelayEnvelope<null>>(
    `${RELAY}/toggle-youtube-monitor`,
    { enabled } satisfies RelayToggleMonitorRequest,
    'POST',
  )
  return unwrapMessage(envelope)
}

/** POST /v1/relay/toggle-twitch-monitor —— 开关 Twitch 监听。 */
export async function relayToggleTwitchMonitor(enabled: boolean): Promise<string | null> {
  const envelope = await relaySendJson<RelayEnvelope<null>>(
    `${RELAY}/toggle-twitch-monitor`,
    { enabled } satisfies RelayToggleMonitorRequest,
    'POST',
  )
  return unwrapMessage(envelope)
}

/** POST /v1/relay/toggle-niconico-monitor —— 开关 Niconico 监听。 */
export async function relayToggleNiconicoMonitor(enabled: boolean): Promise<string | null> {
  const envelope = await relaySendJson<RelayEnvelope<null>>(
    `${RELAY}/toggle-niconico-monitor`,
    { enabled } satisfies RelayToggleMonitorRequest,
    'POST',
  )
  return unwrapMessage(envelope)
}

/** POST /v1/relay/holodex/switch —— 把监听目标切到某条 Holodex 流。 */
export async function relaySwitchStream(payload: RelaySwitchStreamRequest): Promise<string | null> {
  const envelope = await relaySendJson<RelayEnvelope<null>>(
    `${RELAY}/holodex/switch`,
    payload,
    'POST',
  )
  return unwrapMessage(envelope)
}

/** GET /v1/relay/holodex/streams —— 管理端直播与预告列表。 */
export async function relayHolodexStreams(
  query: RelayHolodexStreamsQuery = {},
): Promise<{ source: string | null; items: RelayAdminStreamListItem[] }> {
  const search = new URLSearchParams()
  if (query.favorites) search.set('favorites', 'true')
  if (query.force) search.set('force', 'true')
  const queryString = search.toString()
  const suffix = queryString ? `?${queryString}` : ''
  const response = await relayGetJson<
    RelayEnvelope<RelayAdminStreamListItem[]> & { source?: string }
  >(`${RELAY}/holodex/streams${suffix}`)
  if (!response.success) refused(response.message)
  return { source: response.source ?? null, items: response.data ?? [] }
}

/** GET /v1/relay/holodex/auth/status —— Holodex 账号状态。 */
export async function relayHolodexAuthStatus(): Promise<RelayHolodexAuthStatus> {
  const envelope = await relayGetJson<RelayEnvelope<RelayHolodexAuthStatus>>(
    `${RELAY}/holodex/auth/status`,
  )
  return unwrapData(envelope)
}

/** GET /v1/relay/youtube/keys —— YouTube 密钥池与 WebSub 状态（只有指纹）。 */
export async function relayYoutubeKeys(): Promise<RelayYoutubeKeyStatus> {
  const envelope = await relayGetJson<RelayEnvelope<RelayYoutubeKeyStatus>>(
    `${RELAY}/youtube/keys`,
  )
  return unwrapData(envelope)
}

/** GET /v1/relay/refresh/youtube —— 立刻重取 YouTube 状态。 */
export async function relayRefreshYoutube(): Promise<string | null> {
  const envelope = await relayGetJson<RelayEnvelope<null>>(`${RELAY}/refresh/youtube`)
  return unwrapMessage(envelope)
}

/** GET /v1/relay/refresh/twitch —— 立刻重取 Twitch 状态。 */
export async function relayRefreshTwitch(): Promise<string | null> {
  const envelope = await relayGetJson<RelayEnvelope<null>>(`${RELAY}/refresh/twitch`)
  return unwrapMessage(envelope)
}

/** GET /v1/relay/refresh/niconico —— 立刻重取 Niconico 状态。 */
export async function relayRefreshNiconico(): Promise<string | null> {
  const envelope = await relayGetJson<RelayEnvelope<null>>(`${RELAY}/refresh/niconico`)
  return unwrapMessage(envelope)
}

/** GET /v1/relay/refresh/priority-channel —— 立刻重取优先频道状态。 */
export async function relayRefreshPriorityChannel(): Promise<string | null> {
  const envelope = await relayGetJson<RelayEnvelope<null>>(
    `${RELAY}/refresh/priority-channel`,
  )
  return unwrapMessage(envelope)
}

/** GET /v1/relay/niconico/session —— Niconico 登录态只读快照。 */
export async function relayNiconicoSessionStatus(): Promise<RelayNiconicoSessionStatus> {
  const envelope = await relayGetJson<RelayEnvelope<RelayNiconicoSessionStatus>>(
    RELAY_NICONICO_SESSION,
  )
  return unwrapData(envelope)
}

/** POST /v1/relay/niconico/session/check —— 真去请求一次 Niconico 校验会话。 */
export async function relayCheckNiconicoSession(): Promise<RelayNiconicoSessionStatus> {
  const envelope = await relaySendJson<RelayEnvelope<RelayNiconicoSessionStatus>>(
    `${RELAY_NICONICO_SESSION}/check`,
    {},
    'POST',
  )
  return unwrapData(envelope)
}

/* -------------------------------------------------------------- 裁切 */

/** GET /v1/relay/crop/{platform} —— 该平台的裁切配置；没配过是 `null`。 */
export async function relayGetCrop(platform: RelayCropPlatform): Promise<RelayCropConfig | null> {
  const envelope = await relayGetJson<RelayEnvelope<RelayCropConfig | null>>(
    `${RELAY}/crop/${platform}`,
  )
  return unwrapNullable(envelope)
}

/**
 * POST /v1/relay/crop/{platform}/capture —— 抓一帧用于框选裁切区域。
 *
 * 后端把 JPEG 的 data URL 放在 `message` 字段里（`data` 是 `null`），这里解出来。
 */
export async function relayCaptureCropFrame(
  platform: RelayCropPlatform,
  channelId?: string,
): Promise<string> {
  const search = new URLSearchParams()
  if (channelId?.trim()) search.set('channel_id', channelId.trim())
  const queryString = search.toString()
  const suffix = queryString ? `?${queryString}` : ''
  const envelope = await relaySendJson<RelayEnvelope<null>>(
    `${RELAY}/crop/capture/${platform}${suffix}`,
    {},
    'POST',
  )
  if (!envelope.success) refused(envelope.message)
  const dataUrl = envelope.message?.trim() ?? ''
  if (!dataUrl.startsWith('data:image/')) {
    throw new RelayApiError(dataUrl || '没有取到画面，请确认该频道正在直播', 200, 'no_frame')
  }
  return dataUrl
}

/** POST /v1/relay/crop/update —— 保存裁切开关与区域。 */
export async function relayUpdateCrop(payload: RelayUpdateCropRequest): Promise<string | null> {
  const envelope = await relaySendJson<RelayEnvelope<null>>(
    `${RELAY}/crop/update`,
    payload,
    'POST',
  )
  return unwrapMessage(envelope)
}

/** GET /v1/relay/ffmpeg-cache/{platform} —— 该平台的 HLS 缓存配置。 */
export async function relayGetFfmpegCache(platform: RelayCropPlatform): Promise<RelayFfmpegCache> {
  const envelope = await relayGetJson<RelayEnvelope<RelayFfmpegCache>>(
    `${RELAY}/ffmpeg-cache/${platform}`,
  )
  return unwrapData(envelope)
}

/** POST /v1/relay/ffmpeg-cache/update —— 保存 HLS 缓存开关与延迟（后端夹到 1–60 秒）。 */
export async function relayUpdateFfmpegCache(
  payload: RelayUpdateFfmpegCacheRequest,
): Promise<string | null> {
  const envelope = await relaySendJson<RelayEnvelope<null>>(
    `${RELAY}/ffmpeg-cache/update`,
    payload,
    'POST',
  )
  return unwrapMessage(envelope)
}

/* -------------------------------------------------------------- 存储 */

/** GET /v1/relay/youtube/cookies —— YouTube Cookie 是否已配置（不含内容）。 */
export async function relayYoutubeCookieStatus(): Promise<RelayYoutubeCookieStatus> {
  const envelope = await relayGetJson<RelayEnvelope<RelayYoutubeCookieStatus>>(
    RELAY_YOUTUBE_COOKIES,
  )
  return unwrapData(envelope)
}

/** POST /v1/relay/youtube/cookies —— 导入 Netscape 格式 Cookie 文本。 */
export async function relayImportYoutubeCookies(
  content: string,
  expectedRevision: number,
): Promise<RelayYoutubeCookieStatus> {
  const envelope = await relaySendJson<RelayEnvelope<RelayYoutubeCookieStatus>>(
    RELAY_YOUTUBE_COOKIES,
    { content, expected_revision: expectedRevision },
    'POST',
  )
  return unwrapData(envelope)
}

/** DELETE /v1/relay/youtube/cookies —— 清除已保存的 Cookie。 */
export async function relayClearYoutubeCookies(
  expectedRevision: number,
): Promise<RelayYoutubeCookieStatus> {
  const response = await relayRequest(RELAY_YOUTUBE_COOKIES, {
    method: 'DELETE',
    headers: JSON_HEADERS,
    body: JSON.stringify({ expected_revision: expectedRevision }),
  })
  const envelope = (await response.json()) as RelayEnvelope<RelayYoutubeCookieStatus>
  return unwrapData(envelope)
}

/** GET /v1/relay/player-filter —— 播放器过滤词与它的修订号。 */
export async function relayGetPlayerFilter(): Promise<RelayPlayerFilter> {
  const envelope = await relayGetJson<RelayEnvelope<RelayPlayerFilter>>(
    `${RELAY}/player-filter`,
  )
  return unwrapData(envelope)
}

/** POST /v1/relay/player-filter —— 保存过滤词；修订号不符回 409。 */
export async function relaySavePlayerFilter(
  payload: RelayPlayerFilterSaveRequest,
): Promise<string | null> {
  const envelope = await relaySendJson<RelayEnvelope<null>>(
    `${RELAY}/player-filter`,
    payload,
    'POST',
  )
  return unwrapMessage(envelope)
}

/**
 * POST /v1/relay/storage/backup —— 导出加密整库备份。
 *
 * 成功时响应体是二进制，因此这条不走 JSON 解包；失败时仍是 JSON 信封。
 */
export async function relayStorageBackup(
  passphrase: string,
): Promise<{ blob: Blob; filename: string }> {
  const response = await relayRequest(`${RELAY}/storage/backup`, {
    method: 'POST',
    headers: JSON_HEADERS,
    body: JSON.stringify({ password: passphrase }),
  })
  const disposition = response.headers.get('Content-Disposition') ?? ''
  const match = /filename="?([^";]+)"?/i.exec(disposition)
  return {
    blob: await response.blob(),
    filename: match?.[1]?.trim() || 'biliup.backup',
  }
}

/**
 * POST /v1/relay/storage/restore —— 用加密备份覆盖整库。
 *
 * 前端先卡 32MiB 原始大小，再分块转 base64（后端限 48MiB base64）。
 */
export async function relayStorageRestore(passphrase: string, file: Blob): Promise<string | null> {
  if (file.size > RELAY_MAX_RESTORE_BYTES) {
    throw new RelayApiError('备份文件超过 32MiB 上限，请确认选对了文件', 413, 'too_large')
  }
  const backupBase64 = await fileToBase64(file, RELAY_MAX_RESTORE_BYTES)
  const envelope = await relaySendJson<RelayEnvelope<null>>(
    `${RELAY}/storage/restore`,
    { password: passphrase, backup_base64: backupBase64 },
    'POST',
  )
  return unwrapMessage(envelope)
}

/* -------------------------------------------------------------- 名单 */

/** GET /v1/relay/manage/areas —— 分区文档（含关键词与禁用词）。 */
export async function relayManageAreas(): Promise<RelayAreasDocument> {
  const envelope = await relayGetJson<RelayEnvelope<RelayAreasDocument>>(
    `${RELAY}/manage/areas`,
  )
  return unwrapData(envelope)
}

/** POST /v1/relay/manage/areas —— 新增分区；id 重复会回 `success: false`。 */
export async function relayAddArea(payload: RelayAddAreaRequest): Promise<string | null> {
  const envelope = await relaySendJson<RelayEnvelope<null>>(
    `${RELAY}/manage/areas`,
    payload,
    'POST',
  )
  return unwrapMessage(envelope)
}

/** PUT /v1/relay/manage/areas —— 按 id 更新分区。 */
export async function relayUpdateAreaManage(payload: RelayAddAreaRequest): Promise<string | null> {
  const envelope = await relaySendJson<RelayEnvelope<null>>(
    `${RELAY}/manage/areas`,
    payload,
    'PUT',
  )
  return unwrapMessage(envelope)
}

/** DELETE /v1/relay/manage/areas/{id} —— 按 id 删除分区。 */
export async function relayDeleteArea(id: number): Promise<string | null> {
  const response = await relayRequest(`${RELAY}/manage/areas/${encodeURIComponent(id)}`, {
    method: 'DELETE',
  })
  const envelope = (await response.json()) as RelayEnvelope<null>
  return unwrapMessage(envelope)
}

/** GET /v1/relay/manage/channels —— 频道文档。 */
export async function relayManageChannels(): Promise<RelayChannelsDocument> {
  const envelope = await relayGetJson<RelayEnvelope<RelayChannelsDocument>>(
    `${RELAY}/manage/channels`,
  )
  return unwrapData(envelope)
}

/** POST /v1/relay/manage/channels —— 新增频道；YouTube 输入会先被解析成 UC id。 */
export async function relayAddChannel(payload: RelayAddChannelRequest): Promise<string | null> {
  const envelope = await relaySendJson<RelayEnvelope<null>>(
    `${RELAY}/manage/channels`,
    payload,
    'POST',
  )
  return unwrapMessage(envelope)
}

/** PUT /v1/relay/manage/channels —— 按 name 更新频道。 */
export async function relayUpdateChannelManage(
  payload: RelayAddChannelRequest,
): Promise<string | null> {
  const envelope = await relaySendJson<RelayEnvelope<null>>(
    `${RELAY}/manage/channels`,
    payload,
    'PUT',
  )
  return unwrapMessage(envelope)
}

/** DELETE /v1/relay/manage/channels/{name} —— 按 name 删除频道。 */
export async function relayDeleteChannel(name: string): Promise<string | null> {
  const response = await relayRequest(
    `${RELAY}/manage/channels/${encodeURIComponent(name)}`,
    { method: 'DELETE' },
  )
  const envelope = (await response.json()) as RelayEnvelope<null>
  return unwrapMessage(envelope)
}

/* ---------------------------------------------------------- 统计历史 */

/** 把查询对象拼成后端认识的那串参数（单复数按 `history/query.rs` 的解析器来）。 */
export function relayHistoryQueryString(query: RelayHistoryQuery): string {
  const search = new URLSearchParams()
  const number = (key: string, value: number | undefined) => {
    if (typeof value === 'number' && Number.isFinite(value)) search.set(key, String(value))
  }
  const text = (key: string, value: string | undefined) => {
    const trimmed = value?.trim()
    if (trimmed) search.set(key, trimmed)
  }
  number('from', query.from)
  number('to', query.to)
  number('tz_offset_min', query.tz_offset_min)
  number('limit', query.limit)
  number('max_points', query.max_points)
  text('bucket', query.bucket)
  text('platform', query.platform)
  text('channel', query.channel)
  text('id', query.id)
  text('cursor', query.cursor)
  text('subject', query.subject)
  text('session', query.session)
  text('scope', query.scope)
  if (query.kinds?.length) search.set('kind', query.kinds.join(','))
  if (query.severities?.length) search.set('severity', query.severities.join(','))
  if (query.metrics?.length) search.set('metrics', query.metrics.join(','))
  if (query.calendar) search.set('calendar', '1')
  return search.toString()
}

function relayConditionalHeaders(ifNoneMatch?: string | null): HeadersInit {
  return ifNoneMatch ? { 'If-None-Match': ifNoneMatch } : {}
}

/** 304 与实体分开返回，`etag` 每次都取回，页面直接拿去下次条件请求。 */
async function relayConditional<T>(
  path: string,
  ifNoneMatch?: string | null,
): Promise<RelayConditionalResult<T>> {
  const response = await relayRequest(
    path,
    { method: 'GET', headers: relayConditionalHeaders(ifNoneMatch) },
    true,
  )
  const etag = response.headers.get('ETag')
  if (response.status === 304) return { notModified: true, etag, data: null }
  const envelope = (await response.json()) as RelayEnvelope<T>
  if (!envelope.success) refused(envelope.message)
  if (envelope.data === undefined || envelope.data === null) refused(envelope.message)
  return { notModified: false, etag, data: envelope.data }
}

/** GET /v1/relay/history/status —— 本节点统计库健康状况（支持 304）。 */
export function relayHistoryStatus(
  ifNoneMatch?: string | null,
): Promise<RelayConditionalResult<RelayHistoryStatus>> {
  return relayConditional<RelayHistoryStatus>(`${RELAY}/history/status`, ifNoneMatch)
}

/** GET /v1/relay/history/summary —— KPI、日历与可靠性汇总（支持 304）。 */
export function relayHistorySummary(
  query: RelayHistoryQuery,
  ifNoneMatch?: string | null,
): Promise<RelayConditionalResult<RelayHistorySummary>> {
  return relayConditional<RelayHistorySummary>(
    `${RELAY}/history/summary?${relayHistoryQueryString(query)}`,
    ifNoneMatch,
  )
}

/** GET /v1/relay/history/sessions —— 场次列表，游标分页（支持 304）。 */
export function relayHistorySessions(
  query: RelayHistoryQuery,
  ifNoneMatch?: string | null,
): Promise<RelayConditionalResult<RelayHistorySessionsPage>> {
  return relayConditional<RelayHistorySessionsPage>(
    `${RELAY}/history/sessions?${relayHistoryQueryString(query)}`,
    ifNoneMatch,
  )
}

/** GET /v1/relay/history/events —— 事件流，游标分页（支持 304）。 */
export function relayHistoryEvents(
  query: RelayHistoryQuery,
  ifNoneMatch?: string | null,
): Promise<RelayConditionalResult<RelayHistoryEventsPage>> {
  return relayConditional<RelayHistoryEventsPage>(
    `${RELAY}/history/events?${relayHistoryQueryString(query)}`,
    ifNoneMatch,
  )
}

/** GET /v1/relay/history/stats —— 图表时间序列（支持 304，结果过大回 413）。 */
export function relayHistoryStats(
  query: RelayHistoryQuery,
  ifNoneMatch?: string | null,
): Promise<RelayConditionalResult<RelayHistoryStats>> {
  return relayConditional<RelayHistoryStats>(
    `${RELAY}/history/stats?${relayHistoryQueryString(query)}`,
    ifNoneMatch,
  )
}

/** POST /v1/relay/history/clean —— 清掉本节点已记录的统计行。 */
export async function relayHistoryClean(): Promise<RelayHistoryCleanReport> {
  const envelope = await relaySendJson<RelayEnvelope<RelayHistoryCleanReport>>(
    `${RELAY}/history/clean`,
    {},
    'POST',
  )
  return unwrapData(envelope)
}
