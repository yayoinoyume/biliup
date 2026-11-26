import { apiFetch, handleResponse } from './api-streamer'
import { RelayApiError, type RelayEnvelope } from './relay'
import { describeHttpStatus } from './relay-format'

const YOUTUBE_ACCOUNTS = '/v1/relay/youtube/accounts'
const JSON_HEADERS = { 'Content-Type': 'application/json' } as const
const GENERIC_HTTP_MESSAGE = /^(HTTP \d{3}|Unauthorized)$/

export type YoutubeAccountSourceType = 'browser' | 'cookies'

export interface YoutubeAccount {
  id: string
  label: string
  source_type: YoutubeAccountSourceType
  browser?: string | null
  profile?: string | null
  cookie_count: number
  updated_at: number
}

export interface YoutubeAccountsSnapshot {
  revision: number
  selected_id: string | null
  accounts: YoutubeAccount[]
}

export type YoutubeAccountSource =
  | { type: 'browser'; browser: string; profile?: string }
  | { type: 'cookies'; content: string }

export interface YoutubeAccountCreateRequest {
  label: string
  expected_revision: number
  source: YoutubeAccountSource
}

async function request<T>(path: string, init: RequestInit): Promise<T | null> {
  const response = await apiFetch(path, init)
  if (!response.ok) {
    const raw = await response.clone().text().catch(() => '')
    let shared: string | null = null
    try {
      await handleResponse(response)
    } catch (error) {
      if (error instanceof Error && !GENERIC_HTTP_MESSAGE.test(error.message)) shared = error.message
    }
    let message: string | null = null
    let code: string | null = null
    try {
      const parsed = JSON.parse(raw) as { message?: unknown; code?: unknown }
      if (typeof parsed.message === 'string' && parsed.message.trim()) message = parsed.message.trim()
      if (typeof parsed.code === 'string') code = parsed.code
    } catch {
      message = raw.trim() || null
    }
    throw new RelayApiError(
      message ?? shared ?? describeHttpStatus(response.status),
      response.status,
      code ?? `http_${response.status}`,
    )
  }
  const envelope = (await response.json()) as RelayEnvelope<T>
  if (!envelope.success) {
    throw new RelayApiError(envelope.message?.trim() || '转播服务拒绝了这次请求', response.status, 'refused')
  }
  return envelope.data ?? null
}

export async function youtubeAccounts(): Promise<YoutubeAccountsSnapshot> {
  const snapshot = await request<YoutubeAccountsSnapshot>(YOUTUBE_ACCOUNTS, { method: 'GET' })
  if (!snapshot) throw new RelayApiError('转播服务没有返回 YouTube 账号列表', 200, 'refused')
  return snapshot
}

export function youtubeAddAccount(
  input: YoutubeAccountCreateRequest,
): Promise<YoutubeAccountsSnapshot | null> {
  return request<YoutubeAccountsSnapshot>(YOUTUBE_ACCOUNTS, {
    method: 'POST',
    headers: JSON_HEADERS,
    body: JSON.stringify(input),
  })
}

export function youtubeDeleteAccount(
  id: string,
  expectedRevision: number,
): Promise<YoutubeAccountsSnapshot | null> {
  return request<YoutubeAccountsSnapshot>(`${YOUTUBE_ACCOUNTS}/${encodeURIComponent(id)}`, {
    method: 'DELETE',
    headers: JSON_HEADERS,
    body: JSON.stringify({ expected_revision: expectedRevision }),
  })
}

export function youtubeSelectAccount(
  id: string | null,
  expectedRevision: number,
): Promise<YoutubeAccountsSnapshot | null> {
  return request<YoutubeAccountsSnapshot>(`${YOUTUBE_ACCOUNTS}/selected`, {
    method: 'POST',
    headers: JSON_HEADERS,
    body: JSON.stringify({ id, expected_revision: expectedRevision }),
  })
}
