'use client'

/**
 * `/v1/relay/events` 的 SSE 订阅：服务端状态一变，页面立刻重取，不必轮询。
 *
 * 事件名逐字对上 `crates/biliup-relay/src/ui_events.rs`：五个分类事件加一个 `refresh`
 * （订阅端落后太多时服务端发 `data: all`，等价于“全部重取”）。
 *
 * 重连策略：`EventSource` 自带的自动重连没有退避，撤权后还会一直敲门，
 * 因此这里在出错时主动关掉、按指数退避重建，并顺手 `revalidateMe()`：
 * 会话失效会跳登录页，被降权会按新的权限点收起页面（同 `danmaku-feed.ts`）。
 */

import { useEffect, useRef } from 'react'
import { API_BASE, revalidateMe } from './api-streamer'

/** 服务端会发的全部事件名。 */
export const RELAY_EVENT_KINDS = [
  'status',
  'cluster',
  'config',
  'holodex',
  'history',
  'refresh',
] as const

export type RelayEventKind = (typeof RELAY_EVENT_KINDS)[number]

export interface RelayEvent {
  kind: RelayEventKind
  /** 收到事件时的 Unix 毫秒（本地时钟，仅用于节流与调试）。 */
  at: number
}

export type RelayConnectionState = 'connecting' | 'open' | 'reconnecting' | 'offline'

export interface RelayEventsOptions {
  /** 关掉就断开连接；默认开启。 */
  enabled?: boolean
  /** 同一窗口内多类事件合并成一次回调的最短间隔，默认 400ms。 */
  intervalMs?: number
  onEvent: (event: RelayEvent) => void
  onConnectionChange?: (state: RelayConnectionState) => void
}

/** 退避上限：再慢就不像实时面板了。 */
const MAX_BACKOFF_MS = 30_000
const BASE_BACKOFF_MS = 1_000

/** 事件名 → 中文，供日志页与调试条展示。 */
export const RELAY_EVENT_LABELS: Record<RelayEventKind, string> = {
  status: '直播状态',
  cluster: '集群拓扑',
  config: '配置变更',
  holodex: '直播与预告',
  history: '统计历史',
  refresh: '全量刷新',
}

function isRelayEventKind(value: string): value is RelayEventKind {
  return (RELAY_EVENT_KINDS as readonly string[]).includes(value)
}

function backoffFor(attempt: number): number {
  const grown = BASE_BACKOFF_MS * 2 ** Math.min(attempt, 5)
  return Math.min(grown, MAX_BACKOFF_MS)
}

/**
 * 订阅转播面板事件流。
 *
 * 回调通过 ref 转发，所以调用方不必为了稳定的箭头函数去 `useCallback`；
 * 连接只随 `enabled` 与 `intervalMs` 变化重建。
 */
export function useRelayEvents({
  enabled = true,
  intervalMs = 400,
  onEvent,
  onConnectionChange,
}: RelayEventsOptions): void {
  const eventRef = useRef(onEvent)
  const stateRef = useRef(onConnectionChange)

  // 回调放进 ref 而不是依赖数组：调用方每帧新建箭头函数也不该重连。
  // 只在 effect 里写 ref，渲染期间不碰 ref（react-hooks/refs）。
  useEffect(() => {
    eventRef.current = onEvent
    stateRef.current = onConnectionChange
  }, [onEvent, onConnectionChange])

  useEffect(() => {
    if (!enabled || typeof window === 'undefined') {
      stateRef.current?.('offline')
      return
    }

    let source: EventSource | null = null
    let retryTimer: number | null = null
    let flushTimer: number | null = null
    let attempt = 0
    let disposed = false
    const pending = new Set<RelayEventKind>()

    const flush = () => {
      flushTimer = null
      if (disposed || pending.size === 0) return
      const kinds = Array.from(pending)
      pending.clear()
      const at = Date.now()
      for (const kind of kinds) eventRef.current({ kind, at })
    }

    const queue = (kind: RelayEventKind) => {
      pending.add(kind)
      if (flushTimer !== null) return
      flushTimer = window.setTimeout(flush, intervalMs)
    }

    const scheduleRetry = () => {
      if (disposed) return
      attempt += 1
      const delay = backoffFor(attempt - 1)
      stateRef.current?.('reconnecting')
      retryTimer = window.setTimeout(() => {
        retryTimer = null
        connect()
      }, delay)
    }

    const connect = () => {
      if (disposed) return
      stateRef.current?.(attempt === 0 ? 'connecting' : 'reconnecting')
      const next = new EventSource(`${API_BASE}/v1/relay/events`, { withCredentials: true })
      source = next
      next.onopen = () => {
        if (disposed) return
        attempt = 0
        stateRef.current?.('open')
      }
      for (const kind of RELAY_EVENT_KINDS) {
        next.addEventListener(kind, (event) => {
          if (!isRelayEventKind(kind)) return
          const payload = (event as MessageEvent<string>).data
          // 落后补发是全量刷新；其余事件的 data 只是 changed，不值得解析。
          if (kind === 'refresh' && payload && payload !== 'all') return
          queue(kind)
        })
      }
      // 主动关掉以接管重连节奏：自带重连不会退避，撤权后会一直敲门。
      next.onerror = () => {
        next.close()
        if (source === next) source = null
        if (disposed) return
        revalidateMe()
        scheduleRetry()
      }
    }

    connect()

    return () => {
      disposed = true
      if (retryTimer !== null) window.clearTimeout(retryTimer)
      if (flushTimer !== null) window.clearTimeout(flushTimer)
      source?.close()
      source = null
    }
  }, [enabled, intervalMs])
}
