'use client'

/**
 * 汇总指标与上一周期（同长度窗口）的环比。
 *
 * 后端 `/history/summary` 同时回 `kpis` 与 `prev_kpis`，两者形状一致，直接逐项比。
 * 可用率与成功率在契约里是 0..1 的比例（`query.rs:2122`），不是百分数。
 */

import type { RelayHistoryKpis } from '@/app/lib/relay'
import { formatDurationSec } from '@/app/lib/relay-format'
import relay from '@/app/ui/relay/relay.module.scss'
import styles from './analytics.module.scss'

const percent = (value: number | null) => (value === null ? '—' : `${(value * 100).toFixed(1)}%`)
const count = (value: number | null) => (value === null ? '—' : String(Math.round(value)))
const secs = (value: number | null) => (value === null ? '—' : formatDurationSec(value))

interface Metric {
  label: string
  value: (kpis: RelayHistoryKpis) => number | null
  format: (value: number | null) => string
  /** 涨是好事还是坏事；不填就只显示方向，不染色。 */
  goodWhenUp?: boolean
}

const METRICS: Metric[] = [
  { label: '直播时长', value: (k) => k.live_secs, format: secs, goodWhenUp: true },
  { label: '可用率', value: (k) => k.availability, format: percent, goodWhenUp: true },
  { label: '场次', value: (k) => k.sessions, format: count },
  { label: '失败', value: (k) => k.failures, format: count, goodWhenUp: false },
  { label: '成功率', value: (k) => k.success_rate, format: percent, goodWhenUp: true },
  { label: '平均恢复', value: (k) => k.recover_secs_avg, format: secs, goodWhenUp: false },
  { label: '峰值在线', value: (k) => k.online_peak, format: count },
  { label: '弹幕', value: (k) => k.danmu, format: count },
  { label: '涨粉', value: (k) => k.add_fans, format: count },
  { label: '通知', value: (k) => k.notify_sent, format: count, goodWhenUp: true },
]

/** 环比：上一周期为 0 或缺失时没有意义，返回 null 交给 UI 显示「—」。 */
function change(current: number | null, previous: number | null): number | null {
  if (current === null || previous === null || previous === 0) return null
  return (current - previous) / Math.abs(previous)
}

function Change({ ratio, goodWhenUp }: { ratio: number | null; goodWhenUp?: boolean }) {
  if (ratio === null) {
    return <span className={styles.deltaFlat}>环比 —</span>
  }
  const flat = Math.abs(ratio) < 0.0005
  const up = ratio > 0
  const tone = flat || goodWhenUp === undefined ? 'flat' : up === goodWhenUp ? 'up' : 'down'
  return (
    <span
      className={
        tone === 'up' ? styles.deltaUp : tone === 'down' ? styles.deltaDown : styles.deltaFlat
      }
    >
      {flat ? '环比持平' : `环比 ${up ? '▲' : '▼'} ${Math.abs(ratio * 100).toFixed(1)}%`}
    </span>
  )
}

export interface KpiProps {
  kpis: RelayHistoryKpis
  prev: RelayHistoryKpis
}

export default function Kpi({ kpis, prev }: KpiProps) {
  return (
    <div className={relay.statGrid}>
      {METRICS.map((metric) => (
        <div className={relay.stat} key={metric.label}>
          <span className={relay.statLabel}>{metric.label}</span>
          <span className={relay.statValue}>{metric.format(metric.value(kpis))}</span>
          <Change
            ratio={change(metric.value(kpis), metric.value(prev))}
            goodWhenUp={metric.goodWhenUp}
          />
        </div>
      ))}
    </div>
  )
}
