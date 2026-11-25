'use client'

import type { RelayConfigResponse } from '@/app/lib/relay'
import { RELAY_MASK_CHAR } from '@/app/lib/relay-format'

export type RelayConfigFormKey =
  | 'interval'
  | 'auto_stop_after_source_end'
  | 'auto_stop_threshold_hours'
  | 'show_priority_channel'
  | 'show_twitch'
  | 'show_niconico'
  | 'youtube_rss_enabled'
  | 'auto_cover'
  | 'enable_anti_collision'
  | 'enable_danmaku_command'
  | 'holodex_monitor_gate'
  | 'enable_lol_monitor'
  | 'lol_monitor_interval'
  | 'riot_api_key'
  | 'clear_riot_api_key'
  | 'holodex_api_key'
  | 'clear_holodex_api_key'
  | 'youtube_api_key'
  | 'clear_youtube_api_key'
  | 'youtube_websub_callback_url'
  | 'youtube_websub_port'
  | 'anti_collision_list'
  | 'youtube_proxy'
  | 'clear_youtube_proxy'
  | 'twitch_proxy'
  | 'clear_twitch_proxy'
  | 'twitch_proxy_region'
  | 'youtube_cookies_from_browser'
  | 'youtube_cookies_file'
  | 'youtube_deno_path'
  | 'niconico_user_session'
  | 'clear_niconico_user_session'
  | 'niconico_session_check_enabled'
  | 'niconico_proxy'
  | 'clear_niconico_proxy'
  | 'warning_hold_exempt'
  | 'warning_hold_exempt_reasons'
  | 'warning_hold_exempt_all_low_risk'

const trimmed = (value: string | null | undefined) => (value ?? '').trim()

export function configFormValue(data: RelayConfigResponse, key: RelayConfigFormKey): unknown {
  switch (key) {
    case 'interval':
      return data.interval
    case 'auto_stop_after_source_end':
      return data.auto_stop_after_source_end
    case 'auto_stop_threshold_hours':
      return data.auto_stop_threshold_hours
    case 'show_priority_channel':
      return data.show_priority_channel
    case 'show_twitch':
      return data.show_twitch
    case 'show_niconico':
      return data.show_niconico
    case 'youtube_rss_enabled':
      return data.youtube_rss_enabled
    case 'auto_cover':
      return data.auto_cover
    case 'enable_anti_collision':
      return data.enable_anti_collision
    case 'enable_danmaku_command':
      return data.bilibili.enable_danmaku_command
    case 'holodex_monitor_gate':
      return data.holodex_monitor_gate
    case 'enable_lol_monitor':
      return data.enable_lol_monitor
    case 'lol_monitor_interval':
      return data.lol_monitor_interval ?? 1
    case 'riot_api_key':
    case 'holodex_api_key':
    case 'youtube_api_key':
    case 'niconico_user_session':
    case 'youtube_proxy':
    case 'twitch_proxy':
    case 'niconico_proxy':
      return ''
    case 'clear_riot_api_key':
    case 'clear_holodex_api_key':
    case 'clear_youtube_api_key':
    case 'clear_niconico_user_session':
    case 'clear_youtube_proxy':
    case 'clear_twitch_proxy':
    case 'clear_niconico_proxy':
      return false
    case 'youtube_websub_callback_url':
      return trimmed(data.youtube_websub_callback_url)
    case 'youtube_websub_port':
      return data.youtube_websub_port
    case 'anti_collision_list':
      return data.anti_collision_list
    case 'twitch_proxy_region':
      return data.twitch.proxy_region
    case 'youtube_cookies_from_browser':
      return trimmed(data.youtube.cookies_from_browser)
    case 'youtube_cookies_file':
      return trimmed(data.youtube.cookies_file)
    case 'youtube_deno_path':
      return trimmed(data.youtube.deno_path)
    case 'niconico_session_check_enabled':
      return data.niconico.session_check_enabled
    case 'warning_hold_exempt':
      return data.warning_hold_exempt
    case 'warning_hold_exempt_reasons':
      return data.warning_hold_exempt_reasons
    case 'warning_hold_exempt_all_low_risk':
      return data.warning_hold_exempt_all_low_risk
  }
}

export function expectedConfigValues(
  data: RelayConfigResponse,
  keys: RelayConfigFormKey[],
): Record<string, unknown> {
  const expected: Record<string, unknown> = {}
  for (const key of keys) {
    expected[key] = configFormValue(data, key)
  }
  return expected
}

export interface ProxyPasswordSpan {
  start: number
  end: number
}

export function proxyPasswordSpan(value: string): ProxyPasswordSpan | null {
  const scheme = value.indexOf('://')
  const start = scheme >= 0 ? scheme + 3 : 0
  const authority = value.slice(start).split(/[/?#]/)[0] ?? ''
  const at = authority.lastIndexOf('@')
  if (at < 0) return null
  const colon = authority.slice(0, at).indexOf(':')
  if (colon < 0) return null
  return { start: start + colon + 1, end: start + at }
}

export function proxyHasPasswordMask(value: string): boolean {
  const span = proxyPasswordSpan(value)
  if (!span) return false
  return value.slice(span.start, span.end).includes(RELAY_MASK_CHAR)
}

export function proxyHasPassword(value: string): boolean {
  const span = proxyPasswordSpan(value)
  if (!span) return false
  const secret = value.slice(span.start, span.end)
  return secret.length > 0 && !secret.includes(RELAY_MASK_CHAR)
}

export type ProxyPatchResult =
  | { status: 'unchanged' }
  | { status: 'invalid'; message: string }
  | { status: 'patch'; clear: boolean; value?: string; keep: boolean }

export function buildProxyPatch(
  input: string | null | undefined,
  display: string | null | undefined,
  configured: boolean,
): ProxyPatchResult {
  const text = (input ?? '').trim()
  const shown = (display ?? '').trim()
  if (text === shown) return { status: 'unchanged' }
  if (!text) return { status: 'patch', clear: true, keep: false }
  const span = proxyPasswordSpan(text)
  if (span && text.slice(span.start, span.end).includes(RELAY_MASK_CHAR)) {
    return {
      status: 'invalid',
      message: '代理密码只以掩码显示：请把密码位清空后保存，后端会沿用已保存的密码',
    }
  }
  const keep = configured && proxyHasPasswordMask(shown) && span !== null && !proxyHasPassword(text)
  return { status: 'patch', clear: false, value: text, keep }
}
