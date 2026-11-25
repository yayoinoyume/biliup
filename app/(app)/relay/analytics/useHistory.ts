'use client'

/**
 * 推流统计的数据层：五个历史端点、ETag 条件请求、并发闸与游标分页都在这里。
 *
 * 三条硬约束直接来自后端：
 * 1. `history.rs` 的 `QUERIES` 每节点只有两个查询槽位，第三个并发请求直接 429「统计查询繁忙」——
 *    所以所有历史请求都排队走 `historySlot`，同时最多两个；
 * 2. `/history/*` 回 `ETag`，带上 `If-None-Match` 命中 304 时**不能**用空结果覆盖旧数据；
 * 3. 合并后的响应超过 1MiB 回 413「统计结果过大」，页面据此提示用户缩小范围。
 *
 * 状态里带 `key`：换查询时不必在 effect 里同步清空状态（那会触发级联渲染），
 * 渲染时按 `entry.key === key` 派生「这份数据属于当前查询」，旧查询的结果自然不参与渲染。
 */

import { useCallback, useEffect, useMemo, useRef, useState } from 'react'
import {
  relayHistoryEvents,
  relayHistoryQueryString,
  relayHistorySessions,
  relayHistoryStats,
  relayHistoryStatus,
  relayHistorySummary,
  type RelayConditionalResult,
  type RelayHistoryEventsPage,
  type RelayHistoryQuery,
  type RelayHistorySessionsPage,
  type RelayHistoryStats,
  type RelayHistoryStatus,
  type RelayHistorySummary,
} from '@/app/lib/relay'

/** 与后端 `QUERIES` 的槽位数一致，多出来的请求只能排队。 */
const MAX_INFLIGHT = 2

let inflight = 0
const parked: (() => void)[] = []

/** 所有历史请求都必须经过这个闸：后端每节点只给两个并发统计查询。 */
export async function historySlot<T>(task: () => Promise<T>): Promise<T> {
  while (inflight >= MAX_INFLIGHT) {
    await new Promise<void>((resolve) => parked.push(resolve))
  }
  inflight += 1
  try {
    return await task()
  } finally {
    inflight -= 1
    parked.shift()?.()
  }
}

export type HistoryBucket = 'hour' | '6h' | 'day'
export type HistoryPlatform = 'YT' | 'TW' | 'NC'

/** 页面上的筛选条件：时间窗口、平台、频道与分桶粒度。 */
export interface HistoryWindow {
  from: number
  to: number
  bucket: HistoryBucket
  platform?: HistoryPlatform
  channel?: string
}

/** 图表后端只认这五个指标名（`crate::events::event::METRICS`）。 */
export const HISTORY_METRICS = ['bitrate', 'fps', 'speed', 'online', 'watched']

interface Entry<T> {
  key: string
  data: T | null
  error: unknown
  loading: boolean
}

/**
 * 带 ETag 的条件请求：304 时保留上一份数据，只更新 loading 与错误。
 * `key` 变化时渲染层会忽略旧数据，因此不需要在 effect 里同步清空。
 */
function useConditional<T>(
  key: string,
  load: (etag: string | null) => Promise<RelayConditionalResult<T>>,
): Entry<T> & { reload: () => void } {
  const [entry, setEntry] = useState<Entry<T>>({ key, data: null, error: null, loading: true })
  const etagRef = useRef<string | null>(null)
  const seqRef = useRef(0)
  const loadRef = useRef(load)

  useEffect(() => {
    loadRef.current = load
  })

  const reload = useCallback(() => {
    const seq = (seqRef.current += 1)
    void historySlot(() => loadRef.current(etagRef.current)).then(
      (result) => {
        if (seq !== seqRef.current) return
        if (result.etag) etagRef.current = result.etag
        if (result.notModified) {
          setEntry((prev) => ({
            key,
            data: prev.key === key ? prev.data : null,
            error: null,
            loading: false,
          }))
          return
        }
        setEntry({ key, data: result.data, error: null, loading: false })
      },
      (error: unknown) => {
        if (seq !== seqRef.current) return
        setEntry((prev) => ({
          key,
          data: prev.key === key ? prev.data : null,
          error,
          loading: false,
        }))
      },
    )
  }, [key])

  useEffect(() => {
    reload()
  }, [reload])

  const current = entry.key === key
  return {
    key,
    data: current ? entry.data : null,
    error: current ? entry.error : null,
    loading: !current || entry.loading,
    reload,
  }
}

/** 分页端点共有的形状。 */
interface Page {
  items: unknown[]
  next_cursor: string | null
}

interface PagedStore<P extends Page> {
  key: string
  items: P['items']
  cursor: string | null
  loading: boolean
  loadingMore: boolean
  error: unknown
}

export interface PagedHistory<P extends Page> {
  items: P['items']
  nextCursor: string | null
  loading: boolean
  loadingMore: boolean
  error: unknown
  reload: () => void
  loadMore: () => void
}

const EMPTY_ITEMS: unknown[] = []

/**
 * 游标分页列表：第一页参与 ETag（304 时保留已累积的行），
 * 之后的页只带 `cursor`——后端 `decode_cursor` 不认 ETag，两者不能混用。
 */
function usePagedHistory<P extends Page>(
  key: string,
  loadPage: (cursor: string | null, etag: string | null) => Promise<RelayConditionalResult<P>>,
): PagedHistory<P> {
  const [store, setStore] = useState<PagedStore<P>>({
    key,
    items: EMPTY_ITEMS as P['items'],
    cursor: null,
    loading: true,
    loadingMore: false,
    error: null,
  })
  const etagRef = useRef<string | null>(null)
  const seqRef = useRef(0)
  const loadRef = useRef(loadPage)

  useEffect(() => {
    loadRef.current = loadPage
  })

  const reload = useCallback(() => {
    const seq = (seqRef.current += 1)
    void historySlot(() => loadRef.current(null, etagRef.current)).then(
      (result) => {
        if (seq !== seqRef.current) return
        if (result.etag) etagRef.current = result.etag
        setStore((prev) => {
          const same = prev.key === key
          const items = result.notModified
            ? same
              ? prev.items
              : (EMPTY_ITEMS as P['items'])
            : result.data.items
          return {
            key,
            items,
            cursor: result.notModified ? (same ? prev.cursor : null) : result.data.next_cursor,
            loading: false,
            loadingMore: false,
            error: null,
          }
        })
      },
      (error: unknown) => {
        if (seq !== seqRef.current) return
        setStore((prev) =>
          prev.key === key
            ? { ...prev, loading: false, error }
            : { ...prev, key, items: EMPTY_ITEMS as P['items'], cursor: null, loading: false, error },
        )
      },
    )
  }, [key])

  const loadMore = useCallback(() => {
    if (store.key !== key || !store.cursor || store.loadingMore) return
    const cursor = store.cursor
    setStore((prev) => ({ ...prev, loadingMore: true }))
    void historySlot(() => loadRef.current(cursor, null)).then(
      (result) => {
        if (result.notModified) {
          setStore((prev) => ({ ...prev, loadingMore: false }))
          return
        }
        setStore((prev) => ({
          ...prev,
          items: [...prev.items, ...result.data.items],
          cursor: result.data.next_cursor,
          loadingMore: false,
          error: null,
        }))
      },
      (error: unknown) => {
        setStore((prev) => ({ ...prev, loadingMore: false, error }))
      },
    )
  }, [store, key])

  useEffect(() => {
    reload()
  }, [reload])

  const current = store.key === key
  return {
    items: current ? store.items : (EMPTY_ITEMS as P['items']),
    nextCursor: current ? store.cursor : null,
    loading: !current || store.loading,
    loadingMore: current && store.loadingMore,
    error: current ? store.error : null,
    reload,
    loadMore,
  }
}

/** 北京时间等本地时区的分钟偏移；后端按它把日界与分桶对齐到用户本地时间。 */
export function localTzOffsetMin(): number {
  return -new Date().getTimezoneOffset()
}

/** 把窗口拼成后端认识的查询对象（不含分页与指标）。 */
export function windowQuery(window: HistoryWindow): RelayHistoryQuery {
  return {
    from: window.from,
    to: window.to,
    tz_offset_min: localTzOffsetMin(),
    bucket: window.bucket,
    platform: window.platform,
    channel: window.channel,
  }
}

export interface HistoryData {
  status: RelayHistoryStatus | null
  summary: RelayHistorySummary | null
  stats: RelayHistoryStats | null
  statusError: unknown
  summaryError: unknown
  statsError: unknown
  loading: boolean
  reload: () => void
}

/**
 * 汇总、曲线与库状态三条线。
 * `calendar` 必须显式传 `1`，否则 `summary` 里的日历数组是空的（`query.rs` 的 `q.calendar` 分支）。
 */
export function useHistory(window: HistoryWindow, maxPoints: number): HistoryData {
  const base = useMemo(() => windowQuery(window), [window])
  const baseKey = relayHistoryQueryString(base)

  const summary = useConditional<RelayHistorySummary>(`summary|${baseKey}`, (etag) =>
    relayHistorySummary({ ...base, calendar: true }, etag),
  )
  const stats = useConditional<RelayHistoryStats>(`stats|${baseKey}|${maxPoints}`, (etag) =>
    relayHistoryStats({ ...base, metrics: HISTORY_METRICS, max_points: maxPoints }, etag),
  )
  const status = useConditional<RelayHistoryStatus>('status', (etag) => relayHistoryStatus(etag))

  const { reload: reloadSummary } = summary
  const { reload: reloadStats } = stats
  const { reload: reloadStatus } = status
  const reload = useCallback(() => {
    reloadStatus()
    reloadSummary()
    reloadStats()
  }, [reloadStatus, reloadSummary, reloadStats])

  return {
    status: status.data,
    summary: summary.data,
    stats: stats.data,
    statusError: status.error,
    summaryError: summary.error,
    statsError: stats.error,
    loading: status.loading || summary.loading || stats.loading,
    reload,
  }
}

/** 场次列表：游标分页 + ETag；`query` 里可以带 `from/to` 收窄到某一天。 */
export function useHistorySessions(query: RelayHistoryQuery): PagedHistory<RelayHistorySessionsPage> {
  const key = useMemo(() => `sessions|${relayHistoryQueryString(query)}`, [query])
  return usePagedHistory<RelayHistorySessionsPage>(key, (cursor, etag) =>
    relayHistorySessions({ ...query, ...(cursor ? { cursor } : {}) }, etag),
  )
}

/** 事件流：同上，`kinds`/`severities` 由后端按单数参数名解析。 */
export function useHistoryEvents(query: RelayHistoryQuery): PagedHistory<RelayHistoryEventsPage> {
  const key = useMemo(() => `events|${relayHistoryQueryString(query)}`, [query])
  return usePagedHistory<RelayHistoryEventsPage>(key, (cursor, etag) =>
    relayHistoryEvents({ ...query, ...(cursor ? { cursor } : {}) }, etag),
  )
}
