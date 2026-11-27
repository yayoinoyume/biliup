/**
 * 转播前端共用的格式化与安全阀：纯函数，不碰网络、不碰 React。
 *
 * 时间一律显式区分单位，调用方不许自己乘除 1000：
 * - `formatUnixSec` 只收 Unix 秒（后端 `*_at`、`*_ts` 字段）。
 * - `formatIsoDateTime` 只收 RFC3339 / ISO 字符串（后端 `scheduled_start`、`start_scheduled` 等）。
 * - `formatDurationSec` 只收秒数时长。
 */

/** 备份恢复体的原始文件上限，与后端 48MiB base64 上限对齐前的前端闸门。 */
export const RELAY_MAX_RESTORE_BYTES = 32 * 1024 * 1024

/** 后端把凭据掩码成同长度的一串点；任何只由它组成的输入都等于“没输入新值”。 */
export const RELAY_MASK_CHAR = '•'

const EMPTY_TEXT = '—'

const pad2 = (value: number) => String(Math.trunc(value)).padStart(2, '0')

const isFiniteNumber = (value: number | null | undefined): value is number =>
  typeof value === 'number' && Number.isFinite(value)

/**
 * Unix 秒 → `YYYY-MM-DD HH:mm:ss`（浏览器本地时区）。
 *
 * 不猜毫秒：传毫秒会得到遥远的将来时间，调用方应按字段来源选对函数。
 */
export function formatUnixSec(value: number | null | undefined): string {
  if (!isFiniteNumber(value)) return EMPTY_TEXT
  const date = new Date(value * 1000)
  if (Number.isNaN(date.getTime())) return EMPTY_TEXT
  return `${date.getFullYear()}-${pad2(date.getMonth() + 1)}-${pad2(date.getDate())} ${pad2(
    date.getHours(),
  )}:${pad2(date.getMinutes())}:${pad2(date.getSeconds())}`
}

/** RFC3339 / ISO 时间串 → `YYYY-MM-DD HH:mm:ss`；解析不了时原样返回，便于排查。 */
export function formatIsoDateTime(value: string | null | undefined): string {
  const text = value?.trim() ?? ''
  if (!text) return EMPTY_TEXT
  const date = new Date(text)
  if (Number.isNaN(date.getTime())) return text
  return `${date.getFullYear()}-${pad2(date.getMonth() + 1)}-${pad2(date.getDate())} ${pad2(
    date.getHours(),
  )}:${pad2(date.getMinutes())}:${pad2(date.getSeconds())}`
}

/** 秒数时长 → `m:ss` 或 `h:mm:ss`；负数与非法值按未知处理。 */
export function formatDurationSec(value: number | null | undefined): string {
  if (!isFiniteNumber(value) || value < 0) return EMPTY_TEXT
  const total = Math.floor(value)
  const hours = Math.floor(total / 3600)
  const minutes = Math.floor((total % 3600) / 60)
  const seconds = total % 60
  if (hours > 0) return `${hours}:${pad2(minutes)}:${pad2(seconds)}`
  return `${minutes}:${pad2(seconds)}`
}

/** 字节数 → 便于阅读的单位，只用于提示文案。 */
export function formatBytes(value: number | null | undefined): string {
  if (!isFiniteNumber(value) || value < 0) return EMPTY_TEXT
  const units = ['B', 'KiB', 'MiB', 'GiB', 'TiB']
  let size = value
  let unit = 0
  while (size >= 1024 && unit < units.length - 1) {
    size /= 1024
    unit += 1
  }
  return `${unit === 0 ? size.toFixed(0) : size.toFixed(1)} ${units[unit]}`
}

/** 输入框里的内容是否只剩掩码点与空白——是则视同“留空”。 */
export function isMaskedOrBlank(input: string | null | undefined): boolean {
  const text = input ?? ''
  return Array.from(text).every(
    (char) => char === RELAY_MASK_CHAR || char === ' ' || char === '\n' || char === '\t',
  )
}

/**
 * 凭据字段的写入语义：已配置且用户没输入新值时必须发 `keep_*: true` 并回传空口令位，
 * 否则后端会按 EDIT_INVALID / 400 拒绝（见 `config.rs` 的 `resolve_proxy_input`）。
 */
export function relaySecretPatch(
  input: string | null | undefined,
  configured: boolean,
): { secret?: string; keep: boolean } {
  const text = input ?? ''
  if (isMaskedOrBlank(text)) return configured ? { keep: true } : { keep: false }
  return { secret: text, keep: false }
}

const HTTP_STATUS_TEXT: Record<number, string> = {
  400: '请求内容不合法，请检查后重试',
  401: '登录已失效，请重新登录',
  403: '当前账号没有这项权限',
  404: '没有找到这项内容',
  405: '请求方式不被支持',
  409: '配置已被其他操作修改，请重新加载后再保存',
  412: '还没有完成首次设置，请先打开首次设置。',
  413: '提交的数据过大，请缩小范围或减少记录数',
  422: '提交的内容无法处理，请检查填写内容',
  429: '操作过于频繁，请稍后再试',
  500: '转播服务内部错误，请查看实时日志',
  502: '转播服务暂时不可达',
  503: '转播服务正在启动或重启，请稍后重试',
  504: '转播服务响应超时',
}

/** 后端回了空响应体时的中文兜底文案；未知状态码给出可读的通用说法。 */
export function describeHttpStatus(status: number | null | undefined): string {
  if (!isFiniteNumber(status)) return '请求失败，请稍后重试'
  return HTTP_STATUS_TEXT[status] ?? `请求失败（HTTP ${status}）`
}

/**
 * `File`/`Blob` → 标准 base64（不带 data URL 前缀），分块拼接二进制串。
 *
 * 一次性 `String.fromCharCode(...bytes)` 在几十 MiB 上会打爆调用栈，
 * 因此按 32KiB 分片累加。超过上限直接抛错，不发请求。
 */
export async function fileToBase64(
  file: Blob,
  maxBytes: number = RELAY_MAX_RESTORE_BYTES,
): Promise<string> {
  if (file.size > maxBytes) {
    throw new Error(`文件 ${formatBytes(file.size)} 超过 ${formatBytes(maxBytes)} 上限`)
  }
  const bytes = new Uint8Array(await file.arrayBuffer())
  const chunk = 0x8000
  let binary = ''
  for (let start = 0; start < bytes.length; start += chunk) {
    binary += String.fromCharCode(...bytes.subarray(start, start + chunk))
  }
  return btoa(binary)
}

/** 标准 base64 → 字节；与 `fileToBase64` 配对，供离线预览或校验使用。 */
export function base64ToBytes(value: string): Uint8Array {
  const binary = atob(value)
  const bytes = new Uint8Array(binary.length)
  for (let index = 0; index < binary.length; index += 1) {
    bytes[index] = binary.charCodeAt(index)
  }
  return bytes
}
